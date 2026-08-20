// Scans a plugins directory (built-in and user-installed both land in the same shape — see the
// structure decision: built-ins are seeded here on first run, not architecturally special) and
// starts every plugin.json'd folder found, each as its own isolated process.

pub mod protocol;

use protocol::{PanelRegistration, PluginDescriptor, PluginProcess};
use crate::runtime::runtime_loader::LogLevel;
use std::path::Path;
use std::sync::Arc;

type LogFn = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;
type RegisterFn = Arc<dyn Fn(PanelRegistration) + Send + Sync>;

/// Spawns and starts every plugin found directly under `plugins_dir` (one subfolder per plugin,
/// each containing plugin.json). A plugin that fails to spawn or start is logged and skipped —
/// it never blocks the rest of the plugins, same "one bad actor doesn't take down anything else"
/// reasoning as running each one in its own process at all.
pub fn start_all(plugins_dir: &Path, log: LogFn, on_register: RegisterFn) -> Vec<PluginProcess> {
    let mut started = Vec::new();
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return started; // no plugins directory yet — nothing to start, not an error
    };

    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let Some(desc) = PluginDescriptor::read(&folder) else {
            continue; // not a plugin folder (no plugin.json) — silently not our business
        };
        let id = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        match PluginProcess::spawn(&folder, &desc, id.clone(), log.clone(), on_register.clone()) {
            Ok(plugin) => {
                if plugin.start(&serde_json::json!({})) {
                    started.push(plugin);
                } else {
                    log(LogLevel::Error, &format!("Plugin \"{id}\" failed to start — skipped."));
                    plugin.stop_and_kill();
                }
            }
            Err(e) => log(LogLevel::Error, &format!("Plugin \"{id}\" failed to launch — skipped. {e}")),
        }
    }

    started
}
