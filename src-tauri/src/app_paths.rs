// Where LowArc Studio's own files live: the Rust equivalent of lowarc/Core/AppPaths.cs, same
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
    /// looking for rust-toolchain.toml), or None for an installed copy: never assumed from
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
    /// needing to fake the filesystem walk: same reason AppPaths exposes UserDataFor(devRoot).
    pub fn user_data_for(dev_root: Option<&Path>) -> PathBuf {
        match dev_root {
            Some(root) => root.to_path_buf(),
            None => platform_app_data_root().join(APP_FOLDER_NAME),
        }
    }

    /// LOWARC_MODULES_DIR overrides where modules are resolved from. Added for bin/lowarc.rs's own
    /// end-to-end tests, which have to build a store of their own — resolving against the real one
    /// would make them depend on whatever the developer happens to have installed, and pass or fail
    /// per machine. It is genuinely useful beyond that (running a project against a different module
    /// set without disturbing the installed one), which is why it is a documented override rather
    /// than a test-only backdoor. Deliberately NOT applied to plugins()/settings_file(): those are
    /// Studio's own state, and nothing has asked to relocate them.
    pub fn modules() -> PathBuf {
        match std::env::var_os("LOWARC_MODULES_DIR") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => Self::user_data().join("modules"),
        }
    }
    pub fn plugins() -> PathBuf {
        Self::user_data().join("plugins")
    }
    pub fn settings_file() -> PathBuf {
        Self::user_data().join("settings.json")
    }
    /// A JSON array of `{ path, pinned }` entries. See projects.rs for why this isn't a bare
    /// path-per-line list (pinning needs somewhere to put per-entry state).
    pub fn recent_projects_file() -> PathBuf {
        Self::user_data().join("recent.json")
    }
    /// One JSON file per user-saved custom color theme, named after the preset. Built-in Light/
    /// Dark aren't here — they ship baked into the frontend, since every install always has them.
    pub fn themes() -> PathBuf {
        Self::user_data().join("themes")
    }
    /// Where native binaries this app needs at runtime, but that belong to no plugin, live for an
    /// INSTALLED copy: native_module_host and lowarc_runtime. A source checkout does not need this,
    /// since both are binary targets of this workspace and Cargo puts them next to the running exe.
    /// An installed copy has no such guarantee, so both resolvers fall back here, populated by
    /// ensure_installed_copy_resources() on first run. Distinct from plugins(), since neither is a
    /// plugin or has a plugin.json.
    pub fn runtime_helpers() -> PathBuf {
        Self::user_data().join("runtime-helpers")
    }

    pub fn ensure_directories() -> std::io::Result<()> {
        std::fs::create_dir_all(Self::modules())?;
        std::fs::create_dir_all(Self::plugins())?;
        std::fs::create_dir_all(Self::themes())?;
        Ok(())
    }

    /// A source checkout's plugin backend binaries are build OUTPUT, not source: Cargo compiles
    /// them from src/bin/*.rs as a side effect of building this workspace, next to this executable.
    /// This copies them into the plugin folder the app loads from, so a fresh clone works with
    /// nothing beyond `cargo build`. Source builds only; an installed copy gets its plugins from
    /// bundled resources instead.
    ///
    /// Skips a binary that is not built yet rather than erroring, since that is a normal state.
    /// Also skips a plugin folder with no plugin.json: on a fresh clone that never happens, because
    /// plugin.json is tracked source and only the binary is ever missing. An empty folder means the
    /// user removed that plugin, and putting the binary back would undo a deliberate removal.
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
            if !dest_dir.join("plugin.json").is_file() {
                continue;
            }
            let dest = dest_dir.join(&file_name);
            if needs_copy(&src, &dest)? {
                std::fs::copy(&src, &dest)?;
            }
        }
        Ok(())
    }

    /// The installed-copy counterpart to ensure_builtin_plugin_binaries(), which runs only for a
    /// source checkout. An installed copy's plugins() and runtime_helpers() start empty, since its
    /// writable per-user location has no relationship to where the installer put anything.
    /// `resource_dir` is where Tauri's bundled resources live; this copies the built-in plugins and
    /// native_module_host out of there into the writable locations the app expects, once.
    ///
    /// First run only, not a sync: skips a plugin folder that already has a plugin.json, so a
    /// deliberate removal is not undone. Refreshing an upgraded install is deliberately out of
    /// scope, since doing it safely without clobbering a user's own edits needs real update
    /// infrastructure.
    pub fn ensure_installed_copy_resources(resource_dir: &Path) -> std::io::Result<()> {
        if Self::dev_root().is_some() {
            return Ok(());
        }
        unpack_installed_resources(resource_dir, &Self::plugins(), &Self::runtime_helpers())
    }

    /// True for a name that's safe to join as a single path component onto some other directory
    /// without checking anything further — rejects an empty string, a bare "." or "..", or
    /// anything containing a path separator, any of which could otherwise turn `dir.join(name)`
    /// into a path outside `dir`. Named for its original and most common use (a plugin/module id
    /// is always meant to name exactly one folder directly under plugins()/modules() — see
    /// plugin_host::protocol::PluginDescriptor's own note that a plugin's real identity is its
    /// folder name), but the same check applies anywhere a single path component is about to be
    /// joined onto a directory (projects::create_project's own project name, say) — every such
    /// spot should check it first, same reasoning as plugin_assets::resolve_asset_path's identical
    /// check on the plugin_id half of an asset request.
    pub fn is_valid_component_id(id: &str) -> bool {
        !id.is_empty() && id != "." && id != ".." && !id.contains('/') && !id.contains('\\')
    }
}

/// The logic behind ensure_installed_copy_resources() without its dev_root() guard, so it is
/// testable against real temp directories. The guarded version is not: dev-checkout detection
/// always says "dev" when tests run from this repo.
fn unpack_installed_resources(resource_dir: &Path, plugins_dest: &Path, helpers_dest: &Path) -> std::io::Result<()> {
    let src_plugins = resource_dir.join("plugins");
    if src_plugins.is_dir() {
        for entry in std::fs::read_dir(&src_plugins)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dest = plugins_dest.join(entry.file_name());
            if dest.join("plugin.json").is_file() {
                continue; // already there — either a prior first-run, or a deliberate removal
            }
            copy_dir_all(&entry.path(), &dest)?;
        }
    }

    let src_helpers = resource_dir.join("runtime-helpers");
    if src_helpers.is_dir() {
        std::fs::create_dir_all(helpers_dest)?;
        for entry in std::fs::read_dir(&src_helpers)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let dest = helpers_dest.join(entry.file_name());
            if !dest.is_file() {
                std::fs::copy(entry.path(), &dest)?;
            }
        }
    }

    Ok(())
}

/// std has no recursive directory copy — needed here for the plugins/ tree (Monaco's vendored
/// bundle alone is hundreds of files across nested language/asset folders).
fn copy_dir_all(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
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

        // Force a real, filesystem-visible mtime gap. Some filesystems only have 1-2s resolution,
        // so a same-tick write here wouldn't reliably register as "newer" otherwise.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&src, b"v2").unwrap();
        assert!(needs_copy(&src, &dest).unwrap(), "src was modified after dest");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_app_paths_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_dir_all_reproduces_a_nested_tree() {
        let root = temp_dir("copy_dir_all");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("top.txt"), b"top").unwrap();
        std::fs::write(src.join("nested").join("deep.txt"), b"deep").unwrap();

        let dest = root.join("dest");
        copy_dir_all(&src, &dest).unwrap();

        assert_eq!(std::fs::read(dest.join("top.txt")).unwrap(), b"top");
        assert_eq!(std::fs::read(dest.join("nested").join("deep.txt")).unwrap(), b"deep");
    }

    #[test]
    fn unpack_installed_resources_copies_plugins_and_helpers_once() {
        let root = temp_dir("unpack_resources");
        let resource_dir = root.join("resources");
        let plugins_dest = root.join("data").join("plugins");
        let helpers_dest = root.join("data").join("runtime-helpers");

        // A bundled resource tree: one plugin folder, one helper binary.
        let src_plugin = resource_dir.join("plugins").join("file-explorer");
        std::fs::create_dir_all(&src_plugin).unwrap();
        std::fs::write(src_plugin.join("plugin.json"), b"{}").unwrap();
        std::fs::write(src_plugin.join("file_explorer_backend.exe"), b"binary").unwrap();
        let src_helpers = resource_dir.join("runtime-helpers");
        std::fs::create_dir_all(&src_helpers).unwrap();
        std::fs::write(src_helpers.join("native_module_host.exe"), b"host").unwrap();

        unpack_installed_resources(&resource_dir, &plugins_dest, &helpers_dest).unwrap();

        assert!(plugins_dest.join("file-explorer").join("plugin.json").is_file());
        assert_eq!(
            std::fs::read(plugins_dest.join("file-explorer").join("file_explorer_backend.exe")).unwrap(),
            b"binary"
        );
        assert_eq!(std::fs::read(helpers_dest.join("native_module_host.exe")).unwrap(), b"host");

        // A plugin folder the "user" already has (their own edit, or a deliberate removal that
        // left it present-but-empty) must NOT be overwritten by a second unpack.
        std::fs::write(plugins_dest.join("file-explorer").join("plugin.json"), b"edited").unwrap();
        unpack_installed_resources(&resource_dir, &plugins_dest, &helpers_dest).unwrap();
        assert_eq!(std::fs::read(plugins_dest.join("file-explorer").join("plugin.json")).unwrap(), b"edited");
    }
}
