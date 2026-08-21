// Studio's own app-level preferences — distinct from the per-module `settings` Value threaded
// through runtime/ (that's opaque config handed to game modules at dev-run start, not this).
// One field so far: the dev-run target FPS, previously hardcoded to 60 in lib.rs.

use crate::app_paths::AppPaths;
use serde::{Deserialize, Serialize};
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
}

fn default_target_fps() -> u32 {
    60
}

fn default_theme_mode() -> String {
    "system".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings { dev_run_target_fps: default_target_fps(), theme_mode: default_theme_mode() }
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
        save_to(&path, &Settings { dev_run_target_fps: 30, theme_mode: "light".to_string() }).unwrap();
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
}
