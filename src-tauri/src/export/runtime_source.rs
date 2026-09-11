// Where the runtime executable export_folder copies into every export comes from.
// bin/lowarc_runtime.rs is a binary target of this crate's own workspace, so Cargo builds it as a
// normal side effect of building lowarc-studio. Nothing has to be fetched from anywhere else, and
// export works on any machine that can build this repo.
//
// Same "next to the running exe, or runtime_helpers() as the installed-copy fallback" shape as
// runtime::native_module::native_module_host_path(). See that function's own comment for why an
// existence check rather than a dev/installed branch is what makes this resolve correctly no
// matter which of those it's actually running as.

use crate::app_paths::AppPaths;
use std::path::PathBuf;

fn runtime_exe_name() -> &'static str {
    if cfg!(windows) {
        "lowarc_runtime.exe"
    } else {
        "lowarc_runtime"
    }
}

pub fn resolve_runtime() -> Result<PathBuf, String> {
    let name = runtime_exe_name();
    let exe = std::env::current_exe().map_err(|e| format!("could not resolve the current executable: {e}"))?;
    let dir = exe.parent().ok_or("the current executable has no parent directory")?;
    let next_to_exe = dir.join(name);
    if next_to_exe.is_file() {
        return Ok(next_to_exe);
    }
    let bundled = AppPaths::runtime_helpers().join(name);
    if bundled.is_file() {
        return Ok(bundled);
    }
    Err(format!(
        "Could not find the runtime executable ({name}) — expected it next to this app or in {}. \
         This should never happen in a real build; see prepare-bundle.ps1.",
        AppPaths::runtime_helpers().display()
    ))
}
