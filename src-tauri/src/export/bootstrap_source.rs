// Where the Bootstrap executable that export_folder copies into every export actually comes
// from. Deliberately its own module, separate from mod.rs's staging logic.
//
// A real installed copy never reaches past resolve_bootstrap()'s first check: prepare-bundle.ps1
// now builds Bootstrap from the sibling `lowarc` checkout ONCE, at package time, and stages it
// into runtime-helpers/ alongside native_module_host.exe — the exact same generic bundle/unpack
// path that binary already used (see app_paths.rs's ensure_installed_copy_resources /
// unpack_installed_resources, which needed no changes at all for this: it already copies every
// file under runtime-helpers/, not just the one name it originally shipped with). That's what
// actually closes the gap this module used to describe as "not something a real end user's
// installed copy of this app could ever do" — the sibling-checkout dependency still exists, it's
// just moved to the machine that builds a release, not to every user's own export click.
//
// The sibling-checkout build below still runs, but only as a dev-checkout fallback for whenever
// runtime_helpers() doesn't have a bundled copy yet (a plain `cargo build`/`cargo run` dev loop,
// which never goes through prepare-bundle.ps1 at all) — genuinely useful during active development
// on Bootstrap itself, not dead code kept "just in case."

use crate::app_paths::AppPaths;
use std::path::PathBuf;

fn bootstrap_exe_name() -> &'static str {
    if cfg!(windows) {
        "lowarc-bootstrap.exe"
    } else {
        "lowarc-bootstrap"
    }
}

/// `lowarc-studio`'s own dev-root sibling — both repos sit under the same parent folder on the
/// machine this was built on. None for an installed (non-source) build, same as AppPaths::dev_root
/// itself; there's nothing to walk up from once there's no source checkout at all.
fn sibling_lowarc_repo() -> Option<PathBuf> {
    let studio_root = AppPaths::dev_root()?;
    let sibling = studio_root.parent()?.join("lowarc");
    if sibling.join("Bootstrap").join("Cargo.toml").is_file() {
        Some(sibling)
    } else {
        None
    }
}

/// Builds Bootstrap in release mode from the sibling checkout and returns the path to the
/// resulting executable. Rebuilds every call rather than caching — cargo's own incremental build
/// already makes a no-op rebuild fast, and caching a "is it still current" check here would just
/// be re-implementing what cargo already does correctly.
///
/// Kept pub (not folded away as a resolve_bootstrap() implementation detail) specifically so
/// folder_export_end_to_end.rs can still exercise THIS exact path directly — it's proving the
/// sibling-checkout build genuinely works end to end, which resolve_bootstrap() alone couldn't
/// guarantee testing if a bundled binary happened to already exist in runtime_helpers() on the
/// test machine (e.g. left over from an earlier `cargo tauri build`).
pub fn build_bootstrap(log: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let repo = sibling_lowarc_repo().ok_or(
        "Could not find a bundled runtime, and no `lowarc` checkout next to this one to build it \
         from either. A real installed copy should never hit this — see prepare-bundle.ps1.",
    )?;
    let bootstrap_dir = repo.join("Bootstrap");

    log("Building the runtime (this can take a moment)...");
    let output = std::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&bootstrap_dir)
        .output()
        .map_err(|e| format!("failed to run cargo: {e}"))?;

    if !output.status.success() {
        return Err(format!("Building the runtime failed:\n{}", String::from_utf8_lossy(&output.stderr)));
    }

    let built = bootstrap_dir.join("target").join("release").join(bootstrap_exe_name());
    if !built.is_file() {
        return Err(format!("Expected the built runtime at {} but it isn't there.", built.display()));
    }
    Ok(built)
}

/// The one entry point mod.rs actually calls. Prefers a bundled binary (what every real installed
/// copy has, and what a packaged dev build has too, if prepare-bundle.ps1 already ran) — only
/// falls back to building fresh from a sibling checkout for a plain, unpackaged dev loop that
/// doesn't have one yet. Logs exactly once either way (a fast "found it" note here, or
/// build_bootstrap's own "building..." note) — start_export's own EXPORT_TOTAL_STEPS in lib.rs
/// counts on this stage contributing exactly one log(...) call, same as before this existed.
pub fn resolve_bootstrap(log: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let bundled = AppPaths::runtime_helpers().join(bootstrap_exe_name());
    if bundled.is_file() {
        log("Using the bundled runtime.");
        return Ok(bundled);
    }
    build_bootstrap(log)
}
