// Manages long-lived processes for "session": true plugins (Terminal, so far) — the counterpart
// to plugin_host::protocol::invoke's spawn-fresh-per-call model. A session is spawned once (on
// demand, see start()) and stays running, with its stdout read continuously on a background
// thread and re-fired as a "plugin-emit" Tauri event (lowarc:sessionOutput) — the exact same
// relay-into-iframe path invoke_plugin's own "emit" field already rides, in editor.html — rather
// than inventing a second delivery mechanism. The wire format on that stdout is entirely the
// plugin's own business (see terminal_backend.rs for Terminal's), not something this file
// interprets; it only knows how to read JSON lines and forward them.
//
// Keyed by an opaque session_id (frontend-generated, e.g. a UUID), not by plugin_id: one plugin
// can own several concurrent sessions (Terminal's multi-instance support: each open terminal tab
// is its own session_id under the same "terminal" plugin). A plugin's own frontend is responsible
// for further routing an incoming lowarc:sessionOutput by the session_id riding along in its
// payload to whichever internal terminal instance it belongs to.
//
// plugin_id IS stored on Session (below), and send()/stop() both take the caller's plugin_id and
// verify it against the session's actual owner before touching anything. Without this, any
// session-mode plugin's iframe could write into or kill ANY other plugin's live session just by
// somehow getting hold of its session_id (guessed, logged, leaked some other way): the wire
// protocol on a session's stdin is otherwise unauthenticated by design (it's meant for that
// session's own owner only), so this ownership check is the one thing standing between "a plugin
// that doesn't follow the rules" and actually reaching another plugin's process. The caller's own
// plugin_id always comes from the host's real, unforgeable per-iframe identity (see
// windowToPlugin in split-view.js), never from anything the requesting message itself claims.

use crate::plugin_host::protocol::PluginDescriptor;
use crate::runtime::child_process::{resolve_command, spawn_piped};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use parking_lot::Mutex;
use std::process::{Child, ChildStdin};
use tauri::{AppHandle, Emitter};

struct Session {
    child: Child,
    stdin: ChildStdin,
    plugin_id: String,
}

#[derive(Default)]
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Session>>,
}

impl SessionRegistry {
    /// Idempotent: a second start() for a session_id that's already running (and already owned by
    /// this same plugin_id) is a no-op, not a second process (guards against a double-click on "new
    /// terminal" racing itself, say). A second start() for a session_id already owned by a
    /// DIFFERENT plugin_id is rejected outright rather than silently succeeding as a no-op: a
    /// caller has no way to distinguish "my own session, already running" from "someone else's
    /// session, so I'll pretend I started it" if this returned Ok(()) either way.
    pub fn start(&self, app: &AppHandle, folder: &Path, desc: &PluginDescriptor, plugin_id: &str, session_id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock();
        if let Some(existing) = sessions.get(session_id) {
            if existing.plugin_id != plugin_id {
                return Err(format!("session \"{session_id}\" is already owned by another plugin"));
            }
            return Ok(());
        }

        let exe = resolve_command(folder, &desc.command);
        let mut child = spawn_piped(exe, &desc.args, folder).map_err(|e| format!("failed to start \"{plugin_id}\": {e}"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let reader_app = app.clone();
        let reader_plugin_id = plugin_id.to_string();
        let reader_session_id = session_id.to_string();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(mut payload) = serde_json::from_str::<Value>(&line) else { continue };
                // Tag every message with the session it came from: the plugin's own frontend can
                // own several instances (several open terminal tabs) sharing this one relay path,
                // and has no other way to tell them apart on the receiving end.
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("sessionId".to_string(), Value::String(reader_session_id.clone()));
                }
                let _ = reader_app.emit(
                    "plugin-emit",
                    serde_json::json!({"pluginId": reader_plugin_id, "event": "lowarc:sessionOutput", "payload": payload}),
                );
            }
        });

        sessions.insert(session_id.to_string(), Session { child, stdin, plugin_id: plugin_id.to_string() });
        Ok(())
    }

    /// Fire-and-forget, matching window.lowarc.session.send() on the caller's side — there's no
    /// reply to a raw stdin write, only whatever the session eventually emits as output. Errors if
    /// session_id doesn't exist OR belongs to a different plugin_id: the two are reported the same
    /// way ("no running session"), since a plugin has no legitimate reason to distinguish "that
    /// session doesn't exist" from "that session isn't yours" and telling it which would just be
    /// free reconnaissance for a plugin trying to probe for other plugins' live session ids.
    pub fn send(&self, plugin_id: &str, session_id: &str, message: &Value) -> Result<(), String> {
        let mut sessions = self.sessions.lock();
        let session = sessions
            .get_mut(session_id)
            .filter(|s| s.plugin_id == plugin_id)
            .ok_or_else(|| format!("no running session \"{session_id}\""))?;
        let mut line = serde_json::to_string(message).map_err(|e| e.to_string())?;
        line.push('\n');
        session.stdin.write_all(line.as_bytes()).and_then(|_| session.stdin.flush()).map_err(|e| format!("failed to write to session: {e}"))
    }

    /// Same ownership check as send() — stopping a session_id that exists but belongs to a
    /// different plugin_id is a silent no-op, matching how stopping a session_id that doesn't exist
    /// at all was already handled (this function never returned an error to begin with).
    pub fn stop(&self, plugin_id: &str, session_id: &str) {
        let mut sessions = self.sessions.lock();
        let owned = sessions.get(session_id).is_some_and(|s| s.plugin_id == plugin_id);
        if !owned {
            return;
        }
        if let Some(mut session) = sessions.remove(session_id) {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }

    /// Called once, on app exit — without this, every session process (a real shell) would be
    /// orphaned rather than closed when the window closes.
    pub fn stop_all(&self) {
        for (_, mut session) in self.sessions.lock().drain() {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Bypasses start() (which needs a real AppHandle just to spawn the output-relay thread — not
    // available in a plain unit test) and inserts a Session directly, since these tests are only
    // about send()/stop()'s ownership check, not the spawn/relay machinery start() itself owns.
    // The process still has to be real (a live stdin/Child), not faked, since send() genuinely
    // writes to it.
    fn spawn_stub_session() -> (Child, ChildStdin) {
        let mut child = spawn_piped(
            PathBuf::from("powershell"),
            &["-NoProfile".to_string(), "-Command".to_string(), "[Console]::In.ReadToEnd() | Out-Null".to_string()],
            &std::env::temp_dir(),
        )
        .expect("failed to spawn stub process for test");
        let stdin = child.stdin.take().expect("piped stdin");
        (child, stdin)
    }

    fn insert_owned_by(registry: &SessionRegistry, session_id: &str, plugin_id: &str) {
        let (child, stdin) = spawn_stub_session();
        registry.sessions.lock().insert(session_id.to_string(), Session { child, stdin, plugin_id: plugin_id.to_string() });
    }

    #[test]
    fn send_rejects_a_session_owned_by_a_different_plugin() {
        let registry = SessionRegistry::default();
        insert_owned_by(&registry, "sess-1", "terminal");

        let err = registry.send("some-other-plugin", "sess-1", &Value::String("hi".into())).unwrap_err();
        assert!(err.contains("no running session"), "got: {err}");

        // The rightful owner is unaffected by the rejected attempt.
        registry.send("terminal", "sess-1", &Value::String("hi".into())).expect("owner should still be able to send");
        registry.stop("terminal", "sess-1");
    }

    #[test]
    fn stop_is_a_no_op_for_a_session_owned_by_a_different_plugin() {
        let registry = SessionRegistry::default();
        insert_owned_by(&registry, "sess-2", "terminal");

        registry.stop("some-other-plugin", "sess-2");
        // Still alive: the impostor's stop() didn't touch it.
        registry.send("terminal", "sess-2", &Value::String("hi".into())).expect("session should still be running");

        registry.stop("terminal", "sess-2");
        let err = registry.send("terminal", "sess-2", &Value::String("hi".into())).unwrap_err();
        assert!(err.contains("no running session"), "got: {err}");
    }

    #[test]
    fn send_and_stop_are_safe_for_a_session_id_that_never_existed() {
        let registry = SessionRegistry::default();
        let err = registry.send("terminal", "never-existed", &Value::String("hi".into())).unwrap_err();
        assert!(err.contains("no running session"), "got: {err}");
        registry.stop("terminal", "never-existed"); // must not panic
    }
}
