// Records what happened via a trace file named by an env var, since this runs loaded into the
// TEST process (not spawned separately) — same technique the process-module test fixtures use,
// just here to prove the trace survives being called from inside NativeLoader's dlopen'd code.

use std::os::raw::c_char;

fn trace(line: &str) {
    if let Ok(path) = std::env::var("LOWARC_STUDIO_TEST_TRACE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[no_mangle]
pub extern "C" fn lowarc_module_start(_settings_json: *const c_char, request_stop: extern "C" fn()) {
    trace("started");
    request_stop();
}

#[no_mangle]
pub extern "C" fn lowarc_module_stop() {
    trace("stopped");
}
