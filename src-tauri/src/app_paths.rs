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
    // No caller yet — the settings system and recent-projects tracking aren't built. Defined here
    // anyway, same reasoning as AppPaths.cs listing every path up front: this module's whole job
    // is being the one place that knows where things live, not growing new path logic wherever a
    // future feature happens to need one.
    #[allow(dead_code)]
    pub fn settings_file() -> PathBuf {
        Self::user_data().join("settings.json")
    }
    /// One project path per line — see the structure decision: a path list needs nothing more
    /// than that, JSON would be pure overhead.
    #[allow(dead_code)]
    pub fn recent_projects_file() -> PathBuf {
        Self::user_data().join("recent.txt")
    }

    pub fn ensure_directories() -> std::io::Result<()> {
        std::fs::create_dir_all(Self::modules())?;
        std::fs::create_dir_all(Self::plugins())?;
        Ok(())
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
}
