// Where LowArc Studio's own files live — the Rust equivalent of lowarc/Core/AppPaths.cs, same
// two-root split for the same reason: a source checkout keeps everything inside the repo (so dev
// testing doesn't scatter state into a hidden per-user folder), an installed copy uses the
// platform's real application-data location instead.
//
// Dev-checkout detection mirrors AppPaths.FindDevRoot exactly: walk up from wherever the running
// exe actually is, looking for a marker file that exists in a source checkout but never ships in
// a built app. AppPaths used the solution file (lowarc.slnx); this uses rust-toolchain.toml — it
// exists at this repo's root for the same "pin the toolchain" reason described in that file, and
// like the .slnx, it's dev-tooling that a Tauri build output never carries.

use std::path::{Path, PathBuf};

const APP_FOLDER_NAME: &str = "LowArcStudio";
const DEV_MARKER: &str = "rust-toolchain.toml";

pub struct AppPaths;

impl AppPaths {
    /// The folder the running executable is in. Treat as read-only once installed.
    pub fn install() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The repo root when running from a source checkout (found by walking up from `install()`
    /// looking for rust-toolchain.toml), or None for an installed copy — never assumed from
    /// directory depth, same reasoning as AppPaths: an installed copy has no source tree above it.
    pub fn dev_root() -> Option<PathBuf> {
        Self::dev_root_from(&Self::install())
    }

    fn dev_root_from(start: &Path) -> Option<PathBuf> {
        let mut dir = Some(start.to_path_buf());
        for _ in 0..8 {
            let d = dir.as_ref()?;
            if d.join(DEV_MARKER).is_file() {
                return Some(d.clone());
            }
            dir = d.parent().map(Path::to_path_buf);
        }
        None
    }

    /// The writable root. A source checkout keeps everything in the repo; an installed app uses
    /// the platform's application-data location.
    pub fn user_data() -> PathBuf {
        Self::user_data_for(Self::dev_root().as_deref())
    }

    /// Split out from `user_data()` so the installed-copy path is directly testable without
    /// needing to fake the filesystem walk — same reason AppPaths exposes UserDataFor(devRoot).
    pub fn user_data_for(dev_root: Option<&Path>) -> PathBuf {
        match dev_root {
            Some(root) => root.to_path_buf(),
            None => platform_app_data_root().join(APP_FOLDER_NAME),
        }
    }

    pub fn modules() -> PathBuf {
        Self::user_data().join("modules")
    }
    pub fn plugins() -> PathBuf {
        Self::user_data().join("plugins")
    }
    pub fn settings_file() -> PathBuf {
        Self::user_data().join("settings.json")
    }
    /// A JSON array of `{ path, pinned }` entries — see projects.rs for why this isn't a bare
    /// path-per-line list (pinning needs somewhere to put per-entry state).
    pub fn recent_projects_file() -> PathBuf {
        Self::user_data().join("recent.json")
    }
    /// One JSON file per user-saved custom color theme, named after the preset. Built-in Light/
    /// Dark aren't here — they ship baked into the frontend, since every install always has them.
    pub fn themes() -> PathBuf {
        Self::user_data().join("themes")
    }

    pub fn ensure_directories() -> std::io::Result<()> {
        std::fs::create_dir_all(Self::modules())?;
        std::fs::create_dir_all(Self::plugins())?;
        std::fs::create_dir_all(Self::themes())?;
        Ok(())
    }

    /// A source checkout's own plugin backend binaries (file_explorer_backend.exe,
    /// terminal_backend.exe) are build OUTPUT, not source — Cargo already compiles them as a side
    /// effect of building this same workspace (src/bin/*.rs is picked up automatically, no [[bin]]
    /// needed in Cargo.toml), landing them right next to this very executable. All that was missing
    /// was copying them into the plugin folder the app actually loads plugins from — this does that,
    /// so a fresh clone works with nothing beyond `cargo build`/`cargo run`, no separate manual step
    /// it would have no way to know about. Only runs for a source build; an installed copy gets its
    /// plugins a different way (bundled resources / the normal install flow), not this. Skips a
    /// binary that isn't built yet (e.g. `cargo test`'s own exe lives elsewhere) rather than erroring
    /// — plugin binaries genuinely not existing yet is a normal state, not a failure.
    pub fn ensure_builtin_plugin_binaries() -> std::io::Result<()> {
        if Self::dev_root().is_none() {
            return Ok(());
        }
        for (plugin_dir, bin_name) in BUILTIN_PLUGIN_BACKENDS {
            let file_name = format!("{bin_name}{}", std::env::consts::EXE_SUFFIX);
            let src = Self::install().join(&file_name);
            if !src.is_file() {
                continue;
            }
            let dest_dir = Self::plugins().join(plugin_dir);
            std::fs::create_dir_all(&dest_dir)?;
            let dest = dest_dir.join(&file_name);
            if needs_copy(&src, &dest)? {
                std::fs::copy(&src, &dest)?;
            }
        }
        Ok(())
    }
}

const BUILTIN_PLUGIN_BACKENDS: &[(&str, &str)] = &[("file-explorer", "file_explorer_backend"), ("terminal", "terminal_backend")];

/// True if `dest` doesn't exist yet, or `src` was modified more recently than it — split out from
/// ensure_builtin_plugin_binaries() so it's directly testable with real temp files rather than
/// needing to fake AppPaths' own exe-location walk.
fn needs_copy(src: &Path, dest: &Path) -> std::io::Result<bool> {
    let src_modified = std::fs::metadata(src)?.modified()?;
    match std::fs::metadata(dest).and_then(|m| m.modified()) {
        Ok(dest_modified) => Ok(src_modified > dest_modified),
        Err(_) => Ok(true), // dest missing (or its mtime unreadable) — copy unconditionally
    }
}

#[cfg(windows)]
fn platform_app_data_root() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "macos")]
fn platform_app_data_root() -> PathBuf {
    home_dir().join("Library").join("Application Support")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_app_data_root() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    home_dir().join(".local").join("share")
}

// Deliberately not std::env::home_dir() — it's a long-standing footgun on Windows (wrong/ambiguous
// results in some environment configurations) and was left deprecated-but-not-removed for
// backwards compatibility rather than fixed. Reading HOME directly, only where this function is
// actually called (macOS/Linux), sidesteps that entirely.
#[cfg(unix)]
fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_build_keeps_everything_in_the_repo() {
        // The tests run out of the actual repo, so dev_root() should find rust-toolchain.toml a
        // few levels up from wherever `cargo test` put the test binary.
        let dev_root = AppPaths::dev_root();
        assert!(dev_root.is_some(), "expected to find rust-toolchain.toml walking up from the test binary");
        let dev_root = dev_root.unwrap();
        assert!(dev_root.join(DEV_MARKER).is_file());

        let user_data = AppPaths::user_data_for(Some(&dev_root));
        assert_eq!(user_data, dev_root, "a source build's UserData should be the repo root itself");
    }

    #[test]
    fn installed_uses_a_per_user_writable_location_distinct_from_dev_root() {
        let installed = AppPaths::user_data_for(None);
        assert!(installed.ends_with(APP_FOLDER_NAME));
        assert!(installed.is_absolute());

        if let Some(dev_root) = AppPaths::dev_root() {
            assert_ne!(installed, dev_root, "installed UserData must never collide with a dev checkout's own root");
        }
    }

    #[test]
    fn needs_copy_is_true_when_dest_is_missing_or_older() {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_needs_copy_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        let dest = dir.join("dest.bin");

        std::fs::write(&src, b"v1").unwrap();
        assert!(needs_copy(&src, &dest).unwrap(), "dest doesn't exist yet");

        std::fs::write(&dest, b"v1").unwrap();
        assert!(!needs_copy(&src, &dest).unwrap(), "dest is already at least as new as src");

        // Force a real, filesystem-visible mtime gap — some filesystems only have 1-2s resolution,
        // so a same-tick write here wouldn't reliably register as "newer" otherwise.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&src, b"v2").unwrap();
        assert!(needs_copy(&src, &dest).unwrap(), "src was modified after dest");
    }
}
