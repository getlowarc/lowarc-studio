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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// A pause condition, evaluated purely against the wire-protocol traffic every module already
/// produces (log lines, frame requests/replies) — deliberately nothing here assumes any
/// module-internal concept like a source line or a call stack, since a module can be anything (a
/// compiled binary, a script, a native library) and the engine has no way to know which, or care.
/// `module: None` where present means "any module", not "no module".
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Breakpoint {
    ModuleStart { module: String },
    ModuleError { module: Option<String> },
    LogLevel { level: LogLevel, module: Option<String> },
    FrameCount { count: u64 },
    /// Pauses when `module`'s frame reply has the value at `path` (a JSON Pointer, e.g.
    /// "/state/hp") equal to `equals`. The one genuinely conditional kind — inspects whatever a
    /// module chooses to put in its own frame reply, without the engine needing to understand
    /// what that data means. `path`/`equals` are meaningless for a module that never puts
    /// anything interesting in its reply — that's an honest limit of a module-agnostic design,
    /// not a bug.
    JsonMatch { module: String, path: String, equals: Value },
}

/// One module's request/reply for one frame, captured for the debugger — see
/// `process_module::ProcessModule::frame`, the one place this is actually produced. `id` is the
/// manifest's stable id (what a project's own project.json names this module by), deliberately
/// NOT its decorative display `name` — a breakpoint's `module` field has to match something the
/// user actually knows and controls, and only the id qualifies (a display name can even collide
/// between modules; the id can't).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameModuleTrace {
    pub id: String,
    pub request: Value,
    pub reply: Value,
    pub duration_ms: f64,
}

/// Sent to the frontend once per tick that's actually worth showing a human — never on every tick
/// of a free-running loop (that would flood the IPC channel with JSON nobody's watching). See
/// `process_module::spawn_and_run`'s tick closure for exactly when that is: any tick that fires
/// while `DebugHooks::pause_flag` is true, which covers both a manual step and a breakpoint that
/// just fired mid-tick — one push mechanism doing both jobs, not two.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameTrace {
    pub frame_index: u64,
    pub delta_seconds: f64,
    pub modules: Vec<FrameModuleTrace>,
    /// Only set on the tick where a breakpoint condition newly became true — a step the user
    /// asked for has no "triggered" breakpoint, it's just the frame they requested.
    pub triggered: Option<Breakpoint>,
}

/// The whole debugger feature surface, bundled into one field on `RunContext` rather than four
/// flat ones — they only ever travel together (`lib.rs` constructs all four per run, and both
/// `driver.rs` and `process_module.rs` need the full set), so bundling avoids `RunContext`'s field
/// list ballooning for what's really one cohesive feature.
#[derive(Clone)]
pub struct DebugHooks {
    /// The frame loop blocks between ticks while this is true — checked once per would-be tick in
    /// `driver::run`, not per-module, since pausing (like stopping) is a whole-run action.
    pub pause_flag: Arc<AtomicBool>,
    /// While paused, the driver lets exactly this many more ticks through before re-blocking, then
    /// decrements it — set via a step command, consumed by the driver as it ticks. Zero while
    /// running freely or fully paused with nothing requested yet.
    pub step_request: Arc<AtomicU32>,
    /// Replaced wholesale by the frontend's set-breakpoints call (mirrors "always resend the full
    /// set" rather than incremental add/remove — one fewer state-sync mechanism to get wrong).
    /// Lives independently of any one run (see `lib.rs`'s `BreakpointState`) so breakpoints
    /// configured before a run starts are still honored from the very first frame, and survive
    /// across separate runs the same way a real debugger's breakpoints do.
    pub breakpoints: Arc<Mutex<Vec<Breakpoint>>>,
    /// Fired per FrameTrace — see FrameTrace's own doc comment for exactly when.
    pub on_frame: Arc<dyn Fn(FrameTrace) + Send + Sync>,
}

impl DebugHooks {
    /// A no-op bundle for a caller that doesn't want the debugger at all — an export build, or a
    /// test exercising the run loop itself rather than the debugger. Never pauses, so `on_frame`
    /// is never actually called; still needs to be a real callable, not `None`, since spawn_and_run
    /// calls it unconditionally when `pause_flag` happens to be true.
    pub fn disabled() -> Self {
        Self {
            pause_flag: Arc::new(AtomicBool::new(false)),
            step_request: Arc::new(AtomicU32::new(0)),
            breakpoints: Arc::new(Mutex::new(Vec::new())),
            on_frame: Arc::new(|_| {}),
        }
    }
}

/// Checks a just-produced (module, reply) pair against ModuleError/JsonMatch breakpoints — the
/// two kinds with a per-module frame reply to inspect. FrameCount/ModuleStart/LogLevel are
/// evaluated at their own natural point instead (see spawn_and_run and spawn_stdout_reader), since
/// none of those three have a frame reply at all.
pub fn check_frame_breakpoints(breakpoints: &Mutex<Vec<Breakpoint>>, module: &str, reply: &Value) -> Option<Breakpoint> {
    let list = breakpoints.lock().unwrap();
    list.iter()
        .find(|bp| match bp {
            Breakpoint::ModuleError { module: target } => {
                reply.get("ok").and_then(Value::as_bool) == Some(false) && target.as_deref().map_or(true, |t| t == module)
            }
            Breakpoint::JsonMatch { module: target, path, equals } => target == module && reply.pointer(path) == Some(equals),
            _ => false,
        })
        .cloned()
}

pub fn check_frame_count_breakpoint(breakpoints: &Mutex<Vec<Breakpoint>>, frame_index: u64) -> Option<Breakpoint> {
    let list = breakpoints.lock().unwrap();
    list.iter().find(|bp| matches!(bp, Breakpoint::FrameCount { count } if *count == frame_index)).cloned()
}

pub fn check_module_start_breakpoint(breakpoints: &Mutex<Vec<Breakpoint>>, module: &str) -> Option<Breakpoint> {
    let list = breakpoints.lock().unwrap();
    list.iter().find(|bp| matches!(bp, Breakpoint::ModuleStart { module: target } if target == module)).cloned()
}

pub fn check_log_level_breakpoint(breakpoints: &Mutex<Vec<Breakpoint>>, level: LogLevel, module: &str) -> Option<Breakpoint> {
    let list = breakpoints.lock().unwrap();
    list.iter()
        .find(|bp| matches!(bp, Breakpoint::LogLevel { level: target_level, module: target_module } if *target_level == level && target_module.as_deref().map_or(true, |m| m == module)))
        .cloned()
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
    pub debug: DebugHooks,
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
