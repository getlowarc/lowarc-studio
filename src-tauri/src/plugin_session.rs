// Manages long-lived processes for "session": true plugins (Terminal, so far) — the counterpart
// to plugin_host::protocol::invoke's spawn-fresh-per-call model. A session is spawned once (on
// demand, see start()) and stays running, with its stdout read continuously on a background
// thread and re-fired as a "plugin-emit" Tauri event (lowarc:sessionOutput) — the exact same
// relay-into-iframe path invoke_plugin's own "emit" field already rides, in editor.html — rather
// than inventing a second delivery mechanism. The wire format on that stdout is entirely the
// plugin's own business (see terminal_backend.rs for Terminal's), not something this file
// interprets; it only knows how to read JSON lines and forward them.
//
// Keyed by an opaque session_id (frontend-generated, e.g. a UUID), not by plugin_id — one plugin
// can own several concurrent sessions (Terminal's multi-instance support: each open terminal tab
// is its own session_id under the same "terminal" plugin). Each Session remembers its own
// plugin_id purely so the output relay knows which iframe(s) to reach — a plugin's own frontend is
// responsible for further routing an incoming lowarc:sessionOutput by the session_id riding along
// in its payload to whichever internal terminal instance it belongs to.

use crate::plugin_host::protocol::PluginDescriptor;
use crate::runtime::child_process::{resolve_command, spawn_piped};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

struct Session {
    child: Child,
    stdin: ChildStdin,
}

#[derive(Default)]
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Session>>,
}

impl SessionRegistry {
    /// Idempotent — a second start() for a session_id that's already running is a no-op, not a
    /// second process (guards against a double-click on "new terminal" racing itself, say).
    pub fn start(&self, app: &AppHandle, folder: &Path, desc: &PluginDescriptor, plugin_id: &str, session_id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.contains_key(session_id) {
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
                // Tag every message with the session it came from — the plugin's own frontend can
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

        sessions.insert(session_id.to_string(), Session { child, stdin });
        Ok(())
    }

    /// Fire-and-forget, matching window.lowarc.sendSession() on the caller's side — there's no
    /// reply to a raw stdin write, only whatever the session eventually emits as output.
    pub fn send(&self, session_id: &str, message: &Value) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).ok_or_else(|| format!("no running session \"{session_id}\""))?;
        let mut line = serde_json::to_string(message).map_err(|e| e.to_string())?;
        line.push('\n');
        session.stdin.write_all(line.as_bytes()).and_then(|_| session.stdin.flush()).map_err(|e| format!("failed to write to session: {e}"))
    }

    pub fn stop(&self, session_id: &str) {
        if let Some(mut session) = self.sessions.lock().unwrap().remove(session_id) {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }

    /// Called once, on app exit — without this, every session process (a real shell) would be
    /// orphaned rather than closed when the window closes.
    pub fn stop_all(&self) {
        for (_, mut session) in self.sessions.lock().unwrap().drain() {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }
}
