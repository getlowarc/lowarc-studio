// User-saved custom color themes. The built-in Light and Dark presets aren't here — they're
// baked into the frontend (theme.js) since every install always has them; this module only
// handles presets a user creates themselves in the Appearance page, each one a JSON file under
// AppPaths::themes().
//
// ThemeColors intentionally mirrors the exact custom-property list in style.css's :root (minus
// --accent, which is just an alias for --cyan and never stored on its own). A fixed struct rather
// than a free-form map means a typo'd token name fails to compile instead of silently doing
// nothing in the browser.

use crate::app_paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeColors {
    pub bg: String,
    pub bg_raised: String,
    pub bg_hover: String,
    pub border: String,
    pub fg: String,
    pub fg_dim: String,
    pub cyan: String,
    pub cyan_dark: String,
    pub cyan_dim: String,
    pub cyan_ink: String,
    pub yellow: String,
    pub yellow_dark: String,
    pub yellow_dim: String,
    pub yellow_ink: String,
    pub danger: String,
    pub success: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemePreset {
    pub name: String,
    pub colors: ThemeColors,
}

fn slug(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

pub fn list_presets() -> Vec<ThemePreset> {
    list_presets_in(&AppPaths::themes())
}

fn list_presets_in(dir: &Path) -> Vec<ThemePreset> {
    let mut presets: Vec<ThemePreset> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<ThemePreset>(&text).ok())
        .collect();
    presets.sort_by_key(|p| p.name.to_lowercase());
    presets
}

/// Creating and overwriting both go through here: a preset is identified by its (slugified) name,
/// so saving again under the same name is how a user updates one, not a separate operation.
pub fn save_preset(preset: &ThemePreset) -> Result<(), String> {
    save_preset_in(&AppPaths::themes(), preset)
}

fn save_preset_in(dir: &Path, preset: &ThemePreset) -> Result<(), String> {
    let name = preset.name.trim();
    if name.is_empty() {
        return Err("Preset name can't be empty.".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", slug(name)));
    let text = serde_json::to_string_pretty(preset).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

pub fn delete_preset(name: &str) -> Result<(), String> {
    delete_preset_in(&AppPaths::themes(), name)
}

fn delete_preset_in(dir: &Path, name: &str) -> Result<(), String> {
    let path = preset_path(dir, name);
    if !path.is_file() {
        return Err(format!("No saved theme named \"{name}\".", name = name));
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

fn preset_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.json", slug(name)))
}

// ---------- Theme resolution for plugins (see plugin_asset_server.rs's __lowarc-theme.css) ----------
// theme.js does this same resolution client-side for the host's own pages, but it calls Tauri
// commands (get_settings, list_theme_presets) a sandboxed plugin has no access to: this is the
// server-side equivalent, computed fresh per request so a freshly-mounted/reloaded plugin panel
// always reflects whatever's currently active. Values below are copied verbatim from theme.js's own
// DARK_THEME/LIGHT_THEME constants — kept as functions, not `const`, since ThemeColors' fields are
// owned Strings, not const-evaluable &'static str.

fn dark_theme() -> ThemeColors {
    ThemeColors {
        bg: "#1e1e1e".into(),
        bg_raised: "#252526".into(),
        bg_hover: "#2a2d2e".into(),
        border: "#3c3c3c".into(),
        fg: "#d4d4d4".into(),
        fg_dim: "#8a8a8a".into(),
        cyan: "#00ffff".into(),
        cyan_dark: "#0096c8".into(),
        cyan_dim: "#06272c".into(),
        cyan_ink: "#04191b".into(),
        yellow: "#ffff00".into(),
        yellow_dark: "#e8960a".into(),
        yellow_dim: "#332b00".into(),
        yellow_ink: "#1a1600".into(),
        danger: "#ff3b30".into(),
        success: "#00e676".into(),
    }
}

fn light_theme() -> ThemeColors {
    ThemeColors {
        bg: "#f5f5f7".into(),
        bg_raised: "#ffffff".into(),
        bg_hover: "#ececee".into(),
        border: "#dcdce0".into(),
        fg: "#1c1c1e".into(),
        fg_dim: "#6c6c70".into(),
        cyan: "#0097a8".into(),
        cyan_dark: "#00707d".into(),
        cyan_dim: "#e3f6f8".into(),
        cyan_ink: "#ffffff".into(),
        yellow: "#a87900".into(),
        yellow_dark: "#7a5800".into(),
        yellow_dim: "#fbf0d9".into(),
        yellow_ink: "#ffffff".into(),
        danger: "#ff3b30".into(),
        success: "#00e676".into(),
    }
}

enum Resolved {
    /// "system" (or a themeMode pointing at a since-deleted preset, matching theme.js's own
    /// fallback for that case) — reacts live to the OS's own light/dark preference via a CSS media
    /// query, so no Rust-side "what does the OS currently prefer" lookup is needed at all, unlike
    /// theme.js's own JS-side matchMedia listener.
    FollowSystem,
    Fixed(Box<ThemeColors>),
}

fn resolve(theme_mode: &str) -> Resolved {
    match theme_mode {
        "system" => Resolved::FollowSystem,
        "light" => Resolved::Fixed(Box::new(light_theme())),
        "dark" => Resolved::Fixed(Box::new(dark_theme())),
        name => match list_presets().into_iter().find(|p| p.name == name) {
            Some(preset) => Resolved::Fixed(Box::new(preset.colors)),
            None => Resolved::FollowSystem,
        },
    }
}

fn root_block(colors: &ThemeColors) -> String {
    format!(
        ":root {{ --bg:{}; --bg-raised:{}; --bg-hover:{}; --border:{}; --fg:{}; --fg-dim:{}; --cyan:{}; --cyan-dark:{}; --cyan-dim:{}; --cyan-ink:{}; --yellow:{}; --yellow-dark:{}; --yellow-dim:{}; --yellow-ink:{}; --danger:{}; --success:{}; --accent: var(--cyan); }}",
        colors.bg,
        colors.bg_raised,
        colors.bg_hover,
        colors.border,
        colors.fg,
        colors.fg_dim,
        colors.cyan,
        colors.cyan_dark,
        colors.cyan_dim,
        colors.cyan_ink,
        colors.yellow,
        colors.yellow_dark,
        colors.yellow_dim,
        colors.yellow_ink,
        colors.danger,
        colors.success,
    )
}

/// The CSS text for `__lowarc-theme.css`. `Resolved::FollowSystem` emits a dark `:root` block plus
/// a `@media (prefers-color-scheme: light)` override — both rules have identical specificity, so
/// the media block wins whenever its condition is true purely from coming later in source order,
/// exactly the override a plugin needs with zero JS of its own.
pub fn resolved_css(theme_mode: &str) -> String {
    match resolve(theme_mode) {
        Resolved::Fixed(colors) => root_block(&colors),
        Resolved::FollowSystem => {
            format!("{}\n@media (prefers-color-scheme: light) {{ {} }}", root_block(&dark_theme()), root_block(&light_theme()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_theme_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_colors() -> ThemeColors {
        ThemeColors {
            bg: "#111111".into(),
            bg_raised: "#181818".into(),
            bg_hover: "#1e1e1e".into(),
            border: "#2a2a2a".into(),
            fg: "#eeeeee".into(),
            fg_dim: "#999999".into(),
            cyan: "#00ffff".into(),
            cyan_dark: "#0096c8".into(),
            cyan_dim: "#06272c".into(),
            cyan_ink: "#04191b".into(),
            yellow: "#ffff00".into(),
            yellow_dark: "#e8960a".into(),
            yellow_dim: "#332b00".into(),
            yellow_ink: "#1a1600".into(),
            danger: "#ff3b30".into(),
            success: "#00e676".into(),
        }
    }

    #[test]
    fn round_trips_through_save_and_list() {
        let dir = temp_dir("round_trip");
        let preset = ThemePreset { name: "Midnight".to_string(), colors: sample_colors() };
        save_preset_in(&dir, &preset).unwrap();

        let listed = list_presets_in(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], preset);
    }

    #[test]
    fn saving_the_same_name_again_overwrites_rather_than_duplicating() {
        let dir = temp_dir("overwrite");
        let mut preset = ThemePreset { name: "Midnight".to_string(), colors: sample_colors() };
        save_preset_in(&dir, &preset).unwrap();

        preset.colors.bg = "#000000".to_string();
        save_preset_in(&dir, &preset).unwrap();

        let listed = list_presets_in(&dir);
        assert_eq!(listed.len(), 1, "same name should overwrite, not add a second file");
        assert_eq!(listed[0].colors.bg, "#000000");
    }

    #[test]
    fn delete_removes_it_from_the_list() {
        let dir = temp_dir("delete");
        let preset = ThemePreset { name: "Temp Theme".to_string(), colors: sample_colors() };
        save_preset_in(&dir, &preset).unwrap();
        assert_eq!(list_presets_in(&dir).len(), 1);

        delete_preset_in(&dir, "Temp Theme").unwrap();
        assert_eq!(list_presets_in(&dir).len(), 0);
    }

    #[test]
    fn deleting_a_name_that_was_never_saved_is_a_visible_error() {
        let dir = temp_dir("delete_missing");
        let err = delete_preset_in(&dir, "Nope").expect_err("deleting a nonexistent preset must fail");
        assert!(err.contains("Nope"));
    }

    #[test]
    fn empty_name_is_rejected() {
        let dir = temp_dir("empty_name");
        let preset = ThemePreset { name: "   ".to_string(), colors: sample_colors() };
        let err = save_preset_in(&dir, &preset).expect_err("a blank preset name must be rejected");
        assert!(err.contains("empty"));
    }

    #[test]
    fn dark_and_light_modes_resolve_to_one_fixed_root_block_each() {
        let dark = resolved_css("dark");
        assert!(dark.contains("--bg:#1e1e1e;"), "expected dark's own bg, got {dark}");
        assert!(!dark.contains("@media"), "an explicit mode shouldn't emit a media-query fallback");

        let light = resolved_css("light");
        assert!(light.contains("--bg:#f5f5f7;"), "expected light's own bg, got {light}");
        assert!(!light.contains("@media"));
    }

    #[test]
    fn system_mode_emits_a_dark_default_plus_a_light_media_override() {
        let css = resolved_css("system");
        assert!(css.contains("--bg:#1e1e1e;"), "dark should still be the unconditional default");
        assert!(css.contains("@media (prefers-color-scheme: light)"));
        assert!(css.contains("--bg:#f5f5f7;"), "light values should appear inside the override");
        // The override has to come AFTER the plain block for the cascade to actually favor it when
        // the media condition is true: same specificity either way, so source order decides.
        assert!(css.find("@media").unwrap() > css.find(":root").unwrap());
    }

    #[test]
    fn a_theme_mode_naming_no_real_preset_falls_back_to_system() {
        // Doesn't touch the real themes directory — "definitely-not-a-real-preset-name" can't
        // collide with anything list_presets() might actually find on this machine, so this stays
        // deterministic without needing an injectable themes dir just for this one case.
        assert_eq!(resolved_css("definitely-not-a-real-preset-name"), resolved_css("system"));
    }
}
