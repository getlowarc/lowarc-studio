// Where the Bootstrap executable that export_folder copies into every export actually comes
// from. Deliberately its own module, separate from mod.rs's staging logic, because this part is a
// known temporary bridge, not a permanent design: lowarc-studio and lowarc (which owns Bootstrap)
// are two separate repos today, so the only way to get a Bootstrap binary right now is to build it
// from the sibling checkout on disk. That only works on a machine that actually has `lowarc` next
// to `lowarc-studio` — fine for development, not something a real end user's installed copy of
// this app could ever do. Revisit (vendor Bootstrap's source, or ship a prebuilt binary per
// platform) before this is anything other than an internal dev tool.

use crate::app_paths::AppPaths;
use std::path::PathBuf;

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
pub fn build_bootstrap(log: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let repo = sibling_lowarc_repo().ok_or(
        "Could not find a `lowarc` checkout next to this one to build the runtime from. \
         This export path only works on a dev machine with both repos checked out side by side.",
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

    let exe_name = if cfg!(windows) { "lowarc-bootstrap.exe" } else { "lowarc-bootstrap" };
    let built = bootstrap_dir.join("target").join("release").join(exe_name);
    if !built.is_file() {
        return Err(format!("Expected the built runtime at {} but it isn't there.", built.display()));
    }
    Ok(built)
}
