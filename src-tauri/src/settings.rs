// Studio's own app-level preferences — distinct from the per-module `settings` Value threaded
// through runtime/ (that's opaque config handed to game modules at dev-run start, not this).
// One field so far: the dev-run target FPS, previously hardcoded to 60 in lib.rs.

use crate::app_paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_target_fps")]
    pub dev_run_target_fps: u32,
    /// "system" | "light" | "dark" | the name of a saved custom theme (see theme.rs). Not an enum
    /// on the Rust side — the frontend is the one place that needs to interpret this value (resolve
    /// "system" against the OS, look up a custom name in the saved presets), so Rust just carries
    /// it through opaquely.
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// The editor's sidebar/inspector/console open-vs-closed + size, global across every project
    /// (not per-project) — restored on open so the shell looks the same as it did at last close.
    #[serde(default)]
    pub editor_panels: PanelLayout,
    /// Ids of installed modules/plugins that are disabled without being uninstalled. A module's id
    /// comes from its manifest.json; a plugin's id is its folder name (see installs.rs). Presence
    /// in these lists is the only place "disabled" exists — install_all/scan_store etc. don't know
    /// about it, callers cross-reference it themselves (see plugin_host::start_all's `disabled`
    /// param, and lib.rs's list_installed_* commands).
    #[serde(default)]
    pub disabled_modules: Vec<String>,
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
    /// Values for whatever config fields a plugin has declared in its own plugin.json's `settings`
    /// (see plugin_host::protocol::PluginSettingField) — keyed by plugin id, then by that plugin's
    /// own field key. The generic mechanism any plugin can opt into; superseded the old one-off
    /// `terminal_shell` field, which was this exact same idea (a persisted per-user override for
    /// one plugin's launch behavior) hand-coded for a single plugin instead of expressed through a
    /// schema every plugin can use. Plain strings, not typed values — a checkbox field's value is
    /// "true"/"false", matching how per-module settings (runtime/'s own `settings: HashMap<String,
    /// String>`) already keep this simple rather than modeling a real type system for it.
    #[serde(default)]
    pub plugin_settings: HashMap<String, HashMap<String, String>>,
    /// User-dragged item order for the handful of tab/rail strips that opt into reordering (see
    /// editor.html's initReorderable — only a strip explicitly marked data-reorderable ever writes
    /// here). Keyed by the strip's own DOM id ("rail-tabs", "tab-bar-tabs-0", "console-tabs", ...),
    /// valued by the ordered list of that strip's own item identifiers (a rail/console tab's
    /// panelKey, a file tab's absolute path). A strip missing from this map, or an id present in
    /// the map but no longer present in the strip (an uninstalled plugin, a closed file), just
    /// falls back to natural order — this only ever overrides once a real drag has happened for
    /// that specific strip.
    #[serde(default)]
    pub tab_order: HashMap<String, Vec<String>>,
    /// One consistent per-region persisted shape for the Base system's slot registry (see
    /// primitives.js's contribute()/getSlot()) — open/size/active-key/drag-order in one struct,
    /// keyed by region id ("sidebar", "inspector", "console", "center", ...). Not yet consumed by
    /// any region; editor_panels/tab_order stay authoritative until each region actually migrates
    /// onto the registry, at which point this replaces both for that region.
    #[serde(default)]
    pub regions: HashMap<String, RegionState>,
    /// Ids hidden from one Base-system strip's own display, right-click-toggled per item (see
    /// editor.html's openSlotVisibilityMenu) — keyed by slot name ("sidebar", "console"), valued by
    /// the ids hidden in it. Purely a display filter: a hidden contribution is otherwise completely
    /// unaffected (still enabled, still reachable through the Command Palette, its panel/tab just
    /// doesn't show a button in that one strip). Scoped to sidebar/console only for now, not every
    /// slot the registry knows about — Nolan: "Just the Console and Rail. Nowhere else for now."
    #[serde(default)]
    pub hidden_slot_items: HashMap<String, Vec<String>>,
}

fn default_target_fps() -> u32 {
    60
}

fn default_theme_mode() -> String {
    "system".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dev_run_target_fps: default_target_fps(),
            theme_mode: default_theme_mode(),
            editor_panels: PanelLayout::default(),
            disabled_modules: Vec::new(),
            disabled_plugins: Vec::new(),
            plugin_settings: HashMap::new(),
            tab_order: HashMap::new(),
            regions: HashMap::new(),
            hidden_slot_items: HashMap::new(),
        }
    }
}

/// A region's persisted chrome state under the Base system: whether it's open, its user-resized
/// size (None for a region with no resizable size), which contribution was last active, and the
/// user's own drag-order for its contributions. Mirrors what PanelLayout + tab_order together
/// express today, once per region instead of split across two differently-shaped structures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionState {
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub active_key: Option<String>,
    #[serde(default)]
    pub order: Vec<String>,
}

/// Mirrors editor.html's PANELS state: one open flag + one size per resizable panel. Sizes default
/// to what the shell used before persistence existed; open defaults to false for all three since
/// there's nothing plugin-supplied to show in any of them on a fresh install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelLayout {
    #[serde(default)]
    pub sidebar_open: bool,
    #[serde(default = "default_sidebar_size")]
    pub sidebar_size: f64,
    /// Which rail tab (panelKey(pluginId, panelId), see editor.html) was active when this layout
    /// was last saved — None if none was, or if it belonged to a plugin no longer installed. Lets
    /// a restart reopen the same sidebar panel instead of just an empty column with sidebar_open
    /// stale-true and nothing active behind it.
    #[serde(default)]
    pub sidebar_active_key: Option<String>,
    #[serde(default)]
    pub right_open: bool,
    #[serde(default = "default_right_size")]
    pub right_size: f64,
    #[serde(default)]
    pub console_open: bool,
    #[serde(default = "default_console_size")]
    pub console_size: f64,
    /// Whether the center panel's second editor group (see editor.html's toggleSplit) was open,
    /// and how much of the panel's width the first group took (0.0-1.0). Only the shell layout
    /// persists — which files were open in either group does not, matching every other panel here
    /// (and the main tab bar itself): only chrome visibility survives a restart, not content.
    #[serde(default)]
    pub split_open: bool,
    #[serde(default = "default_split_ratio")]
    pub split_ratio: f64,
}

fn default_sidebar_size() -> f64 {
    240.0
}
fn default_right_size() -> f64 {
    260.0
}
fn default_console_size() -> f64 {
    220.0
}
fn default_split_ratio() -> f64 {
    0.5
}

impl Default for PanelLayout {
    fn default() -> Self {
        PanelLayout {
            sidebar_open: false,
            sidebar_size: default_sidebar_size(),
            sidebar_active_key: None,
            right_open: false,
            right_size: default_right_size(),
            console_open: false,
            console_size: default_console_size(),
            split_open: false,
            split_ratio: default_split_ratio(),
        }
    }
}

pub fn load() -> Settings {
    load_from(&AppPaths::settings_file())
}

fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&AppPaths::settings_file(), settings)
}

fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_settings_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let settings = load_from(&temp_file("missing"));
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn round_trips_through_save_and_load() {
        let path = temp_file("roundtrip");
        save_to(&path, &Settings { dev_run_target_fps: 30, theme_mode: "light".to_string(), ..Default::default() }).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.dev_run_target_fps, 30);
        assert_eq!(loaded.theme_mode, "light");
    }

    #[test]
    fn a_file_missing_newer_fields_still_loads_with_defaults_for_them() {
        // Guards the #[serde(default)] on dev_run_target_fps: an older settings.json (or a
        // hand-edited empty object) must not fail to load just because a field was added later.
        let path = temp_file("partial");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(load_from(&path), Settings::default());
    }

    #[test]
    fn editor_panel_layout_round_trips() {
        let path = temp_file("panel_layout");
        let settings = Settings {
            editor_panels: PanelLayout {
                sidebar_open: true,
                sidebar_size: 300.0,
                sidebar_active_key: Some("terminal:main".to_string()),
                right_open: false,
                right_size: 260.0,
                console_open: true,
                console_size: 180.0,
                split_open: true,
                split_ratio: 0.35,
            },
            ..Settings::default()
        };
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.editor_panels, settings.editor_panels);
    }

    #[test]
    fn tab_order_round_trips_and_defaults_empty() {
        assert!(Settings::default().tab_order.is_empty());

        let path = temp_file("tab_order");
        let mut settings = Settings::default();
        settings.tab_order.insert("rail-tabs".to_string(), vec!["file-explorer::main".to_string(), "__plugin-manager".to_string()]);
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.tab_order, settings.tab_order);
    }

    #[test]
    fn plugin_settings_round_trip_and_default_empty() {
        assert!(Settings::default().plugin_settings.is_empty());

        let path = temp_file("plugin_settings");
        let mut settings = Settings::default();
        settings.plugin_settings.insert("terminal".to_string(), HashMap::from([("shell".to_string(), "pwsh".to_string())]));
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.plugin_settings, settings.plugin_settings);
    }

    #[test]
    fn hidden_slot_items_round_trip_and_default_empty() {
        assert!(Settings::default().hidden_slot_items.is_empty());

        let path = temp_file("hidden_slot_items");
        let mut settings = Settings::default();
        settings.hidden_slot_items.insert("sidebar".to_string(), vec!["file-explorer::main".to_string()]);
        settings.hidden_slot_items.insert("console".to_string(), vec!["terminal::main".to_string()]);
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.hidden_slot_items, settings.hidden_slot_items);
    }

    #[test]
    fn regions_round_trip_and_default_empty() {
        assert!(Settings::default().regions.is_empty());

        let path = temp_file("regions");
        let mut settings = Settings::default();
        settings.regions.insert(
            "sidebar".to_string(),
            RegionState { open: true, size: Some(300.0), active_key: Some("file-explorer::main".to_string()), order: vec!["file-explorer::main".to_string(), "__plugin-manager".to_string()] },
        );
        settings.regions.insert("popups".to_string(), RegionState { open: false, size: None, active_key: None, order: vec![] });
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.regions, settings.regions);
    }
}
