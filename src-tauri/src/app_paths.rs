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
    /// Where a native binary this app needs at runtime, but that isn't part of any one plugin,
    /// lives for an INSTALLED copy — native_module_host (see runtime/native_module.rs) and
    /// lowarc_runtime, the exported/standalone engine binary (see export/runtime_source.rs).
    /// Both are genuine binary targets of this same Cargo workspace (src/bin auto-discovery), so
    /// a SOURCE checkout doesn't need this at all — they resolve next to the running exe instead,
    /// since Cargo already puts every one of this workspace's binaries in the same target/
    /// directory as a normal side effect of building it. An installed copy has no such guarantee
    /// (Tauri doesn't bundle a sibling binary just because it happened to exist in the same build
    /// output directory), so both resolvers fall back to here, populated by
    /// ensure_installed_copy_resources() from this app's own bundled resources on first run — see
    /// prepare-bundle.ps1 for how they get into that bundle. Distinct from plugins() since neither
    /// of these is a plugin or has a plugin.json of its own.
    pub fn runtime_helpers() -> PathBuf {
        Self::user_data().join("runtime-helpers")
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
    /// — plugin binaries genuinely not existing yet is a normal state, not a failure. Also skips a
    /// plugin folder with no plugin.json in it: on a fresh clone that's never true (plugin.json is
    /// tracked source, checked out already — only the compiled binary is ever missing), so this
    /// never blocks the intended fresh-clone-just-works case. But if a user has actually removed
    /// one of these plugins (folder deleted, whether through the app's own Remove button or by
    /// hand), an empty/missing folder is exactly what that looks like — resurrecting just the
    /// binary into it on the next launch would silently undo a deliberate removal.
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

    /// The installed-copy counterpart to ensure_builtin_plugin_binaries() above — that one only
    /// runs for a source checkout (dev_root().is_some(), the opposite guard from this one). A
    /// fresh install's plugins()/runtime_helpers() start out completely empty: unlike a source
    /// checkout, where user_data() IS the repo root, so plugins() already IS the real plugins/
    /// folder with everything already in it, an installed copy's writable per-user data location
    /// has no relationship to where the installer actually put anything. `resource_dir` is where
    /// Tauri's own bundled resources (this app's `bundle.resources`, populated at build time by
    /// prepare-bundle.ps1 — see its own comment for the full bundling story) actually live; this
    /// copies the built-in plugins and native_module_host out of there and into the writable
    /// locations the rest of the app already expects to find them in, exactly once.
    ///
    /// First-run only, not a sync: skips a plugin folder that already has a plugin.json, the same
    /// "don't resurrect something the user deliberately removed" reasoning as
    /// ensure_builtin_plugin_binaries(). Deliberately does NOT handle "this installed copy was
    /// upgraded to a newer version, refresh what's already there" — that needs real update
    /// infrastructure to do safely (a user's own edits inside a plugin folder shouldn't be
    /// silently clobbered by an update), which is a separate, larger piece of work, not something
    /// to fake here.
    pub fn ensure_installed_copy_resources(resource_dir: &Path) -> std::io::Result<()> {
        if Self::dev_root().is_some() {
            return Ok(());
        }
        unpack_installed_resources(resource_dir, &Self::plugins(), &Self::runtime_helpers())
    }
}

/// The real logic behind ensure_installed_copy_resources(), minus its dev_root() guard — split out
/// the same way needs_copy() was, so this is directly testable with real temp directories instead
/// of needing to fake AppPaths' own dev-checkout detection (which would always say "yes, dev" when
/// tests run from this actual repo, making the guarded version untestable here).
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

        // Force a real, filesystem-visible mtime gap — some filesystems only have 1-2s resolution,
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
