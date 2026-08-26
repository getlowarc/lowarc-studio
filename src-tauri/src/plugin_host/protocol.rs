// The plugin<->host wire protocol: one JSON line in, one JSON line out, over a process spawned
// fresh for exactly one call. Deliberately NOT a shared/persistent connection — Nolan's call: a
// plugin is a finished product you invoke to do one thing, not a long-running service the host
// has to keep synchronized with live state. This also happens to be a straight upgrade over the
// v1/v2 persistent-process design it replaced: no start/stop lifecycle, no request-id correlation
// (that existed specifically so concurrent calls could share one connection — under one-process-
// per-call, concurrent calls just get concurrent separate processes, nothing to correlate), no
// "is this plugin currently running" state to track anywhere. A crashing plugin still can't take
// the editor down with it — every call is already its own isolated process, no isolation lost.
//
// Messages:
//   host -> plugin (stdin, exactly one line):
//     {"method":"listDir","params":{...}}
//   plugin -> host (stdout, zero or more log lines, THEN exactly one final line):
//     {"log":{"severity":"info"|"warn"|"error","message":".."}}   (any number, before the reply)
//     {"ok":bool,"result"?:..,"error"?:string,"emit"?:{"event":..,"payload":..}}   (the reply — the
//       first non-log line the host sees is what's read as the reply; the process is killed
//       immediately after, whether or not it was already planning to exit on its own)
//
// "emit" riding along on the reply (instead of a plugin pushing it unprompted at some arbitrary
// later time, which nothing persists long enough to do any more) lets a plugin still tell its own
// panel "something changed" as a side effect of a call — still invoke-triggered, not truly live,
// so it fits the model. Contributes (panels/consoleTabs/viewers) stays the sole, purely static
// source of what a plugin provides — there's no running process left to "confirm" a registration
// live the way v2's registerPanel notification did, so that whole mechanism is gone with it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::runtime::runtime_loader::LogLevel;

pub const DESCRIPTOR_NAME: &str = "plugin.json";

pub type LogFn = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PluginDescriptor {
    pub command: String,
    pub args: Vec<String>,
    #[serde(rename = "timeoutMs", default = "default_timeout")]
    pub timeout_ms: u64,
    /// Purely descriptive — the plugin's real identity everywhere else (logs, PluginProcess.id) is
    /// still its folder name, not this. Shown in the Plugins manage page.
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    /// True for a plugin whose backend can't be invoke-per-call — Terminal, so far, and the only
    /// thing this flag changes: the host spawns `command` once (see plugin_session.rs) instead of
    /// fresh per call, and keeps it running until explicitly stopped or the app exits. Everything
    /// else (contributes, path-traversal rules, etc.) is identical either way; the wire protocol
    /// on that persistent process's stdin/stdout is also JSON-lines, just streamed instead of
    /// one-shot — see plugin_session.rs for the exact shape.
    #[serde(default)]
    pub session: bool,
    #[serde(default)]
    pub contributes: Contributes,
}

/// What a plugin declares up front, read fresh on every call (cheap — plugin.json is tiny) — the
/// only source of truth for what a plugin provides, now that there's no running process left to
/// confirm anything live.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Contributes {
    pub panels: Vec<PanelContribution>,
    pub console_tabs: Vec<ConsoleTabContribution>,
    pub viewers: Vec<ViewerContribution>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelContribution {
    pub id: String,
    pub title: String,
    /// "sidebar" | "inspector" — console isn't a value here since it can hold multiple tabs at
    /// once (see `console_tabs` below), and there's no "viewport" location any more — an open
    /// file's viewer is picked by extension via the sibling `viewers` list instead, since the
    /// viewport now holds one iframe per open file rather than a single permanent plugin.
    pub location: String,
    /// Path within the plugin's own folder, e.g. "index.html" — served over loopback HTTP, see
    /// plugin_asset_server.rs.
    pub entry: String,
    /// Path to an svg within the plugin's folder. Only meaningful for `location: "sidebar"` — the
    /// inspector location is single-slot, nothing to pick between with an icon.
    pub rail_icon: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsoleTabContribution {
    pub id: String,
    pub title: String,
    pub entry: String,
}

/// A plugin that can render an open file's contents in the tab bar's viewport. Matched by
/// extension against the file being opened — first plugin to register a given extension wins it,
/// the same "first wins, no silent override" rule the single-slot panel locations use, since two
/// viewers silently fighting over one file type would be worse than an honest "already taken".
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewerContribution {
    pub id: String,
    pub title: String,
    /// Path within the plugin's own folder, e.g. "index.html" — served over loopback HTTP, see
    /// plugin_asset_server.rs.
    pub entry: String,
    /// Lowercase, dot-included ("`.uc`", "`.png`") — matched case-insensitively against the open
    /// file's own extension.
    pub extensions: Vec<String>,
}

fn default_timeout() -> u64 {
    10000
}

impl PluginDescriptor {
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join(DESCRIPTOR_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// Resolves a plugin's `command` to an actual file: prefers a same-named file in the plugin's own
/// folder, also trying the platform's native executable extension so a single plugin.json entry
/// (e.g. "file_explorer_backend", no extension) resolves correctly whether the shipped binary is
/// `file_explorer_backend.exe` (Windows) or the extension-less build everywhere else — falling
/// back to a bare PATH lookup (e.g. "powershell", "bash") if no local file matches either way.
/// Shared between invoke() below and plugin_session.rs, since both need the exact same rule.
pub fn resolve_command(folder: &Path, command: &str) -> PathBuf {
    let local = folder.join(command);
    if local.is_file() {
        return local;
    }
    if cfg!(windows) {
        let with_exe = folder.join(format!("{command}.exe"));
        if with_exe.is_file() {
            return with_exe;
        }
    }
    PathBuf::from(command)
}

/// Spawns `desc.command` fresh in `folder`, writes exactly one `{"method":..,"params":..}` line to
/// its stdin, and returns whatever it replies with — or a synthetic `{"ok":false,"error":..}` if
/// it fails to launch, doesn't reply within `desc.timeout_ms`, or replies with something that
/// isn't valid JSON. The process is killed immediately once a reply is in hand (or the timeout
/// fires) regardless of whether it was already finishing up on its own — a one-shot invocation has
/// nothing left to do for it once it's answered.
pub fn invoke(folder: &Path, desc: &PluginDescriptor, plugin_id: &str, method: &str, params: &Value, log: &LogFn) -> Value {
    let exe = resolve_command(folder, &desc.command);

    let mut cmd = Command::new(exe);
    cmd.args(&desc.args)
        .current_dir(folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": format!("failed to launch plugin: {e}")}),
    };

    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let request = json!({"method": method, "params": params});
        let mut line = serde_json::to_string(&request).unwrap_or_default();
        line.push('\n');
        if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return json!({"ok": false, "error": "failed to write to plugin process"});
        }
        // Dropping stdin here closes it — the EOF signal a well-behaved one-shot plugin reads
        // until, rather than something that has to loop waiting for a second message that will
        // never come.
    }

    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel::<Value>();
    let reader_log = log.clone();
    let reader_plugin_id = plugin_id.to_string();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                reader_log(LogLevel::Warn, &format!("[{reader_plugin_id}] unparseable line on stdout: {line}"));
                continue;
            };
            if let Some(l) = value.get("log").and_then(|l| l.as_object()) {
                let severity = l.get("severity").and_then(|s| s.as_str()).unwrap_or("info");
                let message = l.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let level = match severity.to_ascii_lowercase().as_str() {
                    "error" => LogLevel::Error,
                    "warn" | "warning" => LogLevel::Warn,
                    _ => LogLevel::Info,
                };
                reader_log(level, &format!("[{reader_plugin_id}] {message}"));
                continue;
            }
            // First non-log line is the reply — this invocation is done regardless of whether
            // the process itself has actually exited yet.
            let _ = tx.send(value);
            return;
        }
        // Stdout closed (EOF) without ever sending a reply — the caller's recv_timeout will time
        // out and report it, nothing more to do here.
    });

    let timeout = Duration::from_millis(desc.timeout_ms.max(1));
    let reply = rx.recv_timeout(timeout).ok();

    let _ = child.kill();
    let mut stderr_text = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        let _ = stderr.read_to_string(&mut stderr_text);
    }
    let _ = child.wait();

    match reply {
        Some(value) => value,
        None => {
            let detail: String = stderr_text.trim().chars().take(300).collect();
            let suffix = if detail.is_empty() { String::new() } else { format!(" ({detail})") };
            json!({"ok": false, "error": format!("plugin did not respond in time{suffix}")})
        }
    }
}
