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
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::runtime::child_process::{parse_log_severity, resolve_command, spawn_piped, STDERR_LOG_TRUNCATE_CHARS};
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
    /// Purely descriptive, shown in the Plugins manage page's detail header.
    pub website: Option<String>,
    /// Path to an SVG/PNG within this plugin's own folder — a general "this is the plugin" icon,
    /// distinct from a panel's own `rail_icon` (sidebar-slot-specific, plugins.rs's Contributes).
    /// Read the same way (see loadRailIconSvg in plugin-hosting.js); absent falls back to a
    /// generic placeholder on the frontend.
    pub icon: Option<String>,
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
    /// Config fields this plugin wants exposed generically in Settings, rendered off this schema
    /// rather than the app hand-coding a UI control per plugin. Optional — most plugins won't
    /// declare any. Values themselves live in Settings.plugin_settings, keyed by plugin id then by
    /// each field's own `key`; a plugin reads its current values back via window.lowarc.getSettings()
    /// (see plugin_assets.rs). No mechanism forces a plugin to use this over reading its own
    /// plugin-specific config some other way — it's the generic option, not a requirement.
    #[serde(default)]
    pub settings: Vec<PluginSettingField>,
    /// Commands this plugin wants exposed in the app's Command Palette — the plugin-declared half
    /// of that registry; host-owned menu items are the other half, auto-discovered from the menu
    /// bar's own DOM rather than declared anywhere (see buildHostMenuCommands() in editor.html) —
    /// a sandboxed plugin has no equivalent to introspect, so it has to say so itself. Selecting one
    /// posts a `lowarc:runCommand` emit (payload `{commandId}`) into this plugin's own iframe if
    /// it's currently mounted; if it isn't, the palette can't run it yet (no auto-mount today) and
    /// says so rather than silently doing nothing.
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginCommand {
    /// Passed back verbatim in the `lowarc:runCommand` emit's payload — this plugin's own business
    /// to interpret, the host never looks inside it.
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginSettingField {
    /// This field's key within Settings.plugin_settings[pluginId] — stable identity, not shown to
    /// the user (label is).
    pub key: String,
    pub label: String,
    /// "text" | "checkbox" | "select" — anything else falls back to "text" on the frontend (see
    /// settings.html's renderer), so an unrecognized value degrades gracefully rather than hiding
    /// the field entirely.
    #[serde(rename = "type", default = "default_setting_field_type")]
    pub field_type: String,
    /// Only meaningful for "select" — ignored otherwise.
    #[serde(default)]
    pub options: Vec<PluginSettingOption>,
    pub hint: Option<String>,
    pub placeholder: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PluginSettingOption {
    pub value: String,
    pub label: String,
}

fn default_setting_field_type() -> String {
    "text".to_string()
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
    /// False (the default — every existing viewer, Monaco included, wants this) reads the file as
    /// UTF-8 text via read_text_file and pushes it as a plain string, same as always. True reads
    /// it via read_binary_file instead and pushes it as a base64 string — for anything that isn't
    /// text (images, video), where decoding as UTF-8 would corrupt the bytes. See lowarc:openFile's
    /// payload shape in plugin_assets.rs for exactly which field carries which.
    #[serde(default)]
    pub binary: bool,
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

/// Spawns `desc.command` fresh in `folder`, writes exactly one `{"method":..,"params":..}` line to
/// its stdin, and returns whatever it replies with — or a synthetic `{"ok":false,"error":..}` if
/// it fails to launch, doesn't reply within `desc.timeout_ms`, or replies with something that
/// isn't valid JSON. The process is killed immediately once a reply is in hand (or the timeout
/// fires) regardless of whether it was already finishing up on its own — a one-shot invocation has
/// nothing left to do for it once it's answered. Command resolution and the spawn itself
/// (resolve_command/spawn_piped) are shared with plugin_session.rs and this crate's own
/// runtime::process_module, not reimplemented here — see runtime::child_process.
pub fn invoke(folder: &Path, desc: &PluginDescriptor, plugin_id: &str, method: &str, params: &Value, log: &LogFn) -> Value {
    let exe = resolve_command(folder, &desc.command);

    let mut child = match spawn_piped(exe, &desc.args, folder) {
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
                reader_log(parse_log_severity(severity), &format!("[{reader_plugin_id}] {message}"));
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
            let detail: String = stderr_text.trim().chars().take(STDERR_LOG_TRUNCATE_CHARS).collect();
            let suffix = if detail.is_empty() { String::new() } else { format!(" ({detail})") };
            json!({"ok": false, "error": format!("plugin did not respond in time{suffix}")})
        }
    }
}
