// The engine. Two entry points share everything below RunContext (the loader trait,
// ProcessLoader, NativeLoader, the driver): start_run() resolves a project's module PRESET
// against the GLOBAL module store (project.rs): what the IDE's own dev-run needs, since nothing
// has resolved anything yet at that point. run_from_launch_dir() instead reads an ALREADY-resolved
// launch.json (what export::export_folder wrote; see its own module comment) and runs exactly
// the modules it names, in the folder they were staged into: what bin/lowarc_runtime.rs (the
// exported, standalone runtime) needs, since export already did the resolving once, at export
// time, and re-resolving against a "global store" that doesn't exist in a shipped folder would be
// both wrong and impossible.

pub mod child_process;
pub mod driver;
pub mod manifest;
pub mod native_module;
pub mod process_module;
pub mod project;
pub mod runtime_loader;

use manifest::{Manifest, ModuleInfo};
use runtime_loader::{DebugHooks, LogFn, RunContext};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// A field nothing reads is almost always a misspelling of one that is read, and the run that
/// follows looks healthy while quietly missing whatever that field was meant to declare. Warned
/// rather than refused: the same shape is what a manifest written for a newer LowArc looks like
/// from here, and refusing to run those would make every module release a breaking one.
fn warn_about_unknown_fields(modules: &[ModuleInfo], log: &LogFn) {
    for info in modules {
        let unknown = info.manifest.unknown_fields();
        if unknown.is_empty() {
            continue;
        }
        log(
            runtime_loader::LogLevel::Warn,
            &format!(
                "Module \"{}\" has manifest fields nothing reads: {}. Check the spelling, or ignore this if they are meant for a newer LowArc.",
                info.manifest.id,
                unknown.join(", ")
            ),
        );
    }
}

/// Whether every module providing a contract speaks a version the modules consuming it accept.
///
/// resolve() already checked each contract requirement against the CONTRACT's own version. This is
/// the other pairing, and the one that decides whether the data actually makes sense: a consumer
/// asking for `draw-commands ^1` handed the output of a provider speaking `2.0.0` gets a list of
/// ops in a vocabulary it does not know.
///
/// A warning, not a refusal, and the entry is still gathered. `shared` holds one array per
/// contract that every consumer reads, so dropping an entry would take it from consumers that were
/// perfectly happy with it. Each entry carries its `version` (see mirror_onto_contracts) so a
/// consumer that cares can filter; this says out loud that it might need to.
fn warn_about_contract_versions(modules: &[ModuleInfo], log: &LogFn) {
    for consumer in modules {
        for dep in consumer.manifest.requires.iter().filter(|d| d.names_contract()) {
            let Ok(range) = dep.range() else { continue }; // resolve() already reported this
            if range == semver::VersionReq::STAR {
                continue;
            }
            for provider in modules {
                for provision in provider.manifest.provides.iter().filter(|p| p.contract == dep.contract) {
                    let speaks = semver::Version::parse(provision.version.trim()).ok();
                    let happy = speaks.as_ref().map(|v| range.matches(v)).unwrap_or(false);
                    if happy {
                        continue;
                    }
                    log(
                        runtime_loader::LogLevel::Warn,
                        &format!(
                            "\"{}\" provides {} {}, but \"{}\" asked for {}. Its entry is still gathered; \"{}\" may not understand it.",
                            provider.manifest.id,
                            dep.contract,
                            if speaks.is_some() { provision.version.clone() } else { format!("\"{}\", which is not a version,", provision.version) },
                            consumer.manifest.id,
                            dep.version,
                            consumer.manifest.id
                        ),
                    );
                }
            }
        }
    }
}

/// Resolves `project_dir`'s module preset against `modules_dir`, picks the runtime loader that
/// can handle the resolved set, and runs `entry_file` through it. Blocking: call this on its own
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
    log: LogFn,
    debug: DebugHooks,
) -> Result<(), Vec<String>> {
    let preset = project::ProjectPreset::load(project_dir).map_err(|e| vec![e])?;
    let modules = project::resolve(&preset, modules_dir)?;

    warn_about_unknown_fields(&modules, &log);
    warn_about_contract_versions(&modules, &log);

    let source_code = std::fs::read_to_string(entry_file)
        .map_err(|e| vec![format!("Could not read {}: {e}", entry_file.display())])?;

    let loaders = runtime_loader::default_loaders();
    let loader = loaders.iter().find(|l| l.can_handle(&modules)).ok_or_else(|| {
        vec![format!(
            "No runtime loader recognises this project's modules (tried: {}).",
            loaders.iter().map(|l| l.id()).collect::<Vec<_>>().join(", ")
        )]
    })?;

    let ctx = RunContext { source_code: &source_code, source_path: entry_file, target_fps, settings, stop_flag, log, debug };
    loader.run(modules, &ctx).map_err(|e| vec![e])
}

/// launch.json's own shape: the one file bin/lowarc_runtime.rs (the exported, standalone
/// runtime) reads, and the only thing export::export_folder writes describing HOW to run what it
/// staged. Shared here (not a private struct in export/mod.rs, not a raw json!() macro either) so
/// the writer and the one real reader can't drift out of sync on a key name. `modules` is already
/// the fully-resolved list (export-time output, not a preset to re-resolve): paths relative to
/// this same file's own directory, same convention `source` already uses.
#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchConfig {
    pub target_fps: u32,
    pub source: String,
    pub modules: Vec<String>,
    pub diagnostics_log: bool,
    pub settings: Value,
}

impl LaunchConfig {
    pub fn read(dir: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(dir.join("launch.json")).map_err(|e| format!("Could not read launch.json: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("launch.json is invalid: {e}"))
    }
}

/// The exported-app counterpart to start_run(). `dir` is the folder launch.json and everything it
/// names lives in, taken as a PathBuf rather than "wherever the exe is" so this stays testable
/// without a built binary.
///
/// `debug` is a parameter because the two callers want opposite things: lowarc_runtime.rs passes
/// DebugHooks::disabled(), dev_run_host.rs passes real hooks wired to Studio's control protocol.
/// That one parameter is what lets both binaries share this machinery rather than duplicate it.
pub fn run_from_launch_dir(dir: &Path, stop_flag: Arc<AtomicBool>, log: LogFn, debug: DebugHooks) -> Result<(), Vec<String>> {
    let launch = LaunchConfig::read(dir).map_err(|e| vec![e])?;

    let entry_file = dir.join(&launch.source);
    let source_code = std::fs::read_to_string(&entry_file).map_err(|e| vec![format!("Could not read {}: {e}", entry_file.display())])?;

    let mut infos = Vec::with_capacity(launch.modules.len());
    for rel in &launch.modules {
        let folder = dir.join(rel);
        let manifest = Manifest::read(&folder).map_err(|e| vec![e])?;
        infos.push(ModuleInfo { folder, manifest });
    }
    let modules = manifest::in_run_order(infos);
    warn_about_unknown_fields(&modules, &log);
    warn_about_contract_versions(&modules, &log);

    let loaders = runtime_loader::default_loaders();
    let loader = loaders.iter().find(|l| l.can_handle(&modules)).ok_or_else(|| {
        vec![format!(
            "No runtime loader recognises this project's modules (tried: {}).",
            loaders.iter().map(|l| l.id()).collect::<Vec<_>>().join(", ")
        )]
    })?;

    let ctx = RunContext {
        source_code: &source_code,
        source_path: &entry_file,
        target_fps: launch.target_fps,
        settings: launch.settings,
        stop_flag,
        log,
        debug,
    };
    loader.run(modules, &ctx).map_err(|e| vec![e])
}
