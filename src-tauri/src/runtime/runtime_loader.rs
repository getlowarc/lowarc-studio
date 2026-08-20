// Mirrors Bootstrap's runtime_loader.rs — same discipline as Contracts/IModuleLoader.cs, applied
// here too: which runtime a set of modules needs is not the caller's business, it's a registered,
// ordered slot, asked in turn until one says yes. ProcessLoader and NativeLoader are equal
// implementations; nothing in this crate is privileged over the other.
//
// Real, deliberate difference from Bootstrap's version: `run()` here returns instead of exiting
// the process. Bootstrap IS the run — when it ends, the process is supposed to end. The IDE is
// not the run — it has to stay alive afterward, ready for the next one, and a user has to be able
// to hit Stop mid-run rather than only ever waiting for a module to end itself. So `RunContext`
// carries a stop flag either side can set (a module's own request, or an external Stop command),
// and every loader's `run` is expected to return once that flag is observed, not exit anything.

use crate::runtime::manifest::ModuleInfo;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Everything a loader might need to run a set of modules. `stop_flag` is the one thing every
/// loader MUST honor promptly — it's how a user's Stop click actually reaches a running module,
/// and it's a single flag for the whole run, not per-module, since stopping is a whole-run action.
pub struct RunContext<'a> {
    pub source_code: &'a str,
    pub source_path: &'a Path,
    pub target_fps: u32,
    pub settings: Value,
    pub stop_flag: Arc<AtomicBool>,
    pub log: Arc<dyn Fn(LogLevel, &str) + Send + Sync>,
}

pub trait RuntimeLoader: Send + Sync {
    /// Stable id, for error messages and logging — mirrors IModuleLoader.Id.
    fn id(&self) -> &'static str;
    /// Asked in registration order; the first `true` owns the whole run.
    fn can_handle(&self, modules: &[ModuleInfo]) -> bool;
    /// Runs the modules until `ctx.stop_flag` is observed or every module ends on its own.
    /// Returns once the run is over — never exits the process, unlike Bootstrap's version.
    fn run(&self, modules: Vec<ModuleInfo>, ctx: &RunContext) -> Result<(), String>;
}

pub fn default_loaders() -> Vec<Box<dyn RuntimeLoader>> {
    vec![Box::new(crate::runtime::process_module::ProcessLoader), Box::new(crate::runtime::native_module::NativeLoader)]
}
