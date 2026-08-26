// The dev-run engine. Same three-piece shape as lowarc/Bootstrap (RuntimeLoader trait,
// ProcessLoader, NativeLoader) plus one thing Bootstrap never needed: project.rs, which resolves
// a project's module PRESET against the GLOBAL module store — Bootstrap gets its module list
// already resolved and staged by the C# Exporter; here, nothing has done that yet, so this crate
// does it itself.

pub mod child_process;
pub mod driver;
pub mod manifest;
pub mod native_module;
pub mod process_module;
pub mod project;
pub mod runtime_loader;

use runtime_loader::{LogLevel, RunContext};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Resolves `project_dir`'s module preset against `modules_dir`, picks the runtime loader that
/// can handle the resolved set, and runs `entry_file` through it. Blocking — call this on its own
/// thread, not the Tauri main thread. Returns once the run ends, for any reason: the loader's own
/// completion, `stop_flag` being set (a module's own request, or an external Stop), or an error.
#[allow(clippy::too_many_arguments)]
pub fn start_run(
    entry_file: &Path,
    project_dir: &Path,
    modules_dir: &Path,
    target_fps: u32,
    settings: Value,
    stop_flag: Arc<AtomicBool>,
    log: Arc<dyn Fn(LogLevel, &str) + Send + Sync>,
) -> Result<(), Vec<String>> {
    let preset = project::ProjectPreset::load(project_dir).map_err(|e| vec![e])?;
    let modules = project::resolve(&preset, modules_dir)?;

    let source_code = std::fs::read_to_string(entry_file)
        .map_err(|e| vec![format!("Could not read {}: {e}", entry_file.display())])?;

    let loaders = runtime_loader::default_loaders();
    let loader = loaders.iter().find(|l| l.can_handle(&modules)).ok_or_else(|| {
        vec![format!(
            "No runtime loader recognises this project's modules (tried: {}).",
            loaders.iter().map(|l| l.id()).collect::<Vec<_>>().join(", ")
        )]
    })?;

    let ctx = RunContext { source_code: &source_code, source_path: entry_file, target_fps, settings, stop_flag, log };
    loader.run(modules, &ctx).map_err(|e| vec![e])
}
