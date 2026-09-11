// The exported, standalone runtime: this binary IS what "Export" produces (export::export_folder
// copies it in, renamed to the project's own name, right alongside launch.json/modules/source).
// Reads launch.json from its own directory and runs exactly what it names, through the same
// RuntimeLoader machinery (ProcessLoader/NativeLoader) the IDE's own dev-run uses. See
// runtime::run_from_launch_dir's own comment. Genuinely the same engine, not a reimplementation of
// it: this is a different, minimal front door onto the exact same runtime module, with no IDE, no
// window, no UI of its own at all: read launch.json, run it, exit.
//
// No OS signal handling (Ctrl+C, window-close) yet: a module's own requestStop is the only way a
// run currently ends early. Worth adding before this is more than a first pass.

use lowarc_studio_lib::runtime::{
    self,
    runtime_loader::{DebugHooks, LogFn},
};
use parking_lot::Mutex;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn main() {
    let exe = std::env::current_exe().unwrap_or_else(|e| fatal(&format!("could not resolve the current executable: {e}")));
    let dir = exe.parent().unwrap_or_else(|| fatal("the current executable has no parent directory")).to_path_buf();

    // Read once here just to decide whether a diagnostics.log file is wanted before the log
    // callback exists to hand to run_from_launch_dir, which reads launch.json again itself. A
    // second read of one small JSON file is a cheap, simple price for not threading a pre-parsed
    // LaunchConfig through that function's signature just for this one field.
    let diagnostics_log = runtime::LaunchConfig::read(&dir).map(|l| l.diagnostics_log).unwrap_or(false);
    let diag_file: Option<Mutex<std::fs::File>> = if diagnostics_log { std::fs::File::create(dir.join("diagnostics.log")).ok().map(Mutex::new) } else { None };

    let log: LogFn = Arc::new(move |level, message| {
        let line = format!("[{level:?}] {message}");
        println!("{line}");
        if let Some(file) = &diag_file {
            let mut f = file.lock();
            let _ = writeln!(f, "{line}");
        }
    });

    let stop_flag = Arc::new(AtomicBool::new(false));

    // No debugger in an export. See run_from_launch_dir's own comment on why this is a parameter
    // now rather than something it decides for itself.
    if let Err(errors) = runtime::run_from_launch_dir(&dir, stop_flag, log, DebugHooks::disabled()) {
        for e in &errors {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }
}
