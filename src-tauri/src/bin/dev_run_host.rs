// The IDE's dev-run, as its own process. Studio spawns this, hands it a scratch directory holding
// a launch.json it just wrote, and drives it from outside over stdin/stdout: one JSON object per
// line, the same shape runtime::process_module already uses one level down for an individual
// module.
//
// Why this is a separate process: user code must never be able to take Studio down. Run on a
// thread inside lowarc-studio.exe, the loader and driver machinery, and anything loaded alongside
// it, would sit in the address space holding the user's unsaved editor work. Out here, a crash
// ends the run being debugged and nothing else, structurally rather than by the good behaviour of
// whatever modules happen to be loaded.
//
// Separate from bin/lowarc_runtime.rs on purpose, even though both ultimately call
// runtime::run_from_launch_dir. That binary IS a shipped export — "no IDE, no window, no UI of its
// own at all", and has no debugger by design. This one is a Studio-only debug harness with the
// opposite needs. They share the run machinery through run_from_launch_dir's `debug` parameter
// rather than by one binary growing a mode flag for the other's lifecycle.
//
// Protocol:
//   stdout (pushes, unprompted — mirrors process_module::spawn_stdout_reader's own shape)
//     {"log":{"severity":"info"|"warn"|"error","message":"..."}}
//     {"frame": <FrameTrace>}
//     {"ended":{"ok":true}} | {"ended":{"ok":false,"errors":["..."]}}
//   stdin (commands, fire-and-forget, no reply correlation)
//     {"cmd":"pause"} {"cmd":"resume"} {"cmd":"step","count":n} {"cmd":"stop"}
//     {"cmd":"setBreakpoints","breakpoints":[...]}
//
// FrameTrace/Breakpoint/LogLevel go over the wire as their own serde impls: the same types the
// engine already produces and the frontend already consumes, not a parallel set to keep in sync.

use lowarc_studio_lib::runtime::{
    self,
    runtime_loader::{Breakpoint, DebugHooks, LogFn, LogLevel},
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// One writer for every line this process emits. Stdout is shared between the engine's log
/// callback, the frame-trace callback, and the final ended line, all of which can fire from
/// different threads. Interleaved half-lines would be unparseable on Studio's side.
fn stdout_lock() -> &'static Mutex<std::io::Stdout> {
    static OUT: std::sync::OnceLock<Mutex<std::io::Stdout>> = std::sync::OnceLock::new();
    OUT.get_or_init(|| Mutex::new(std::io::stdout()))
}

fn write_line(value: &Value) {
    let mut out = stdout_lock().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn log_level_str(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

fn fatal(message: &str) -> ! {
    // Reported as a normal ended-with-errors line rather than only on stderr, so Studio surfaces it
    // through the same path every other run failure already takes.
    write_line(&json!({"ended": {"ok": false, "errors": [message]}}));
    std::process::exit(1);
}

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => fatal("dev_run_host needs the run directory as its first argument."),
    };

    // The same primitives Studio's own RunState/BreakpointState held while the run lived in-process
    // — local to this process now, driven by the stdin commands below instead of by direct Tauri
    // state access.
    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let step_request = Arc::new(AtomicU32::new(0));
    let breakpoints: Arc<Mutex<Vec<Breakpoint>>> = Arc::new(Mutex::new(Vec::new()));

    spawn_command_reader(stop_flag.clone(), pause_flag.clone(), step_request.clone(), breakpoints.clone());

    let log: LogFn = Arc::new(|level, message| {
        write_line(&json!({"log": {"severity": log_level_str(level), "message": message}}));
    });

    let debug = DebugHooks {
        pause_flag,
        step_request,
        breakpoints,
        on_frame: Arc::new(|trace| write_line(&json!({"frame": trace}))),
    };

    // Called straight from main, NOT on a spawned thread: this process's true OS main thread has
    // to be the one running the frame loop. Nothing needs that today, but a module that opens a
    // real window will: platform windowing (macOS especially) requires its event loop on the main
    // thread, and burying the engine under a thread::spawn here would break that non-obviously
    // later. The stdin reader above is the background thread; this stays foreground.
    let result = runtime::run_from_launch_dir(&dir, stop_flag, log, debug);

    let payload = match &result {
        Ok(()) => json!({"ok": true}),
        Err(errors) => json!({"ok": false, "errors": errors}),
    };
    write_line(&json!({"ended": payload}));
}

/// Applies Studio's commands to the run's own primitives as they arrive. Its own thread because
/// reading stdin blocks, and the frame loop on main can't wait on it. An unparseable or unknown
/// line is ignored rather than fatal: a control-channel hiccup shouldn't kill a running project.
fn spawn_command_reader(
    stop_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    step_request: Arc<AtomicU32>,
    breakpoints: Arc<Mutex<Vec<Breakpoint>>>,
) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(cmd) = value.get("cmd").and_then(|c| c.as_str()) else { continue };

            match cmd {
                // Pausing zeroes any in-flight step for the same reason Studio's own pause_dev_run
                // did when it owned these atomics directly: a step queued right before a pause would
                // otherwise let one more tick slip through immediately after.
                "pause" => {
                    step_request.store(0, Ordering::SeqCst);
                    pause_flag.store(true, Ordering::SeqCst);
                }
                "resume" => {
                    step_request.store(0, Ordering::SeqCst);
                    pause_flag.store(false, Ordering::SeqCst);
                }
                "step" => {
                    let count = value.get("count").and_then(Value::as_u64).unwrap_or(1).max(1);
                    step_request.fetch_add(count as u32, Ordering::SeqCst);
                }
                "stop" => stop_flag.store(true, Ordering::SeqCst),
                // Replaced wholesale, matching the "frontend always resends the full set" convention
                // set_breakpoints already used when this list was shared with Studio by Arc.
                "setBreakpoints" => {
                    if let Some(list) = value.get("breakpoints") {
                        if let Ok(parsed) = serde_json::from_value::<Vec<Breakpoint>>(list.clone()) {
                            *breakpoints.lock() = parsed;
                        }
                    }
                }
                _ => {}
            }
        }
    });
}
