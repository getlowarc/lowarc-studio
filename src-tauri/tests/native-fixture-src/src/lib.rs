// Records what happened via a trace file named by an env var, since this runs loaded into the
// TEST process (not spawned separately) — same technique the process-module test fixtures use,
// just here to prove the trace survives being called from inside NativeLoader's dlopen'd code.

use std::cell::Cell;
use std::os::raw::c_char;

fn trace(line: &str) {
    if let Ok(path) = std::env::var("LOWARC_STUDIO_TEST_TRACE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

// native_module_host drives exactly one module from a single thread (frame() is never called
// concurrently with itself), so plain thread_locals are enough state here — no synchronization
// needed. REQUEST_STOP holds the callback handed to start() so frame() can call it once it's seen
// enough frames; there's no other point in this module's lifecycle that gets it.
thread_local! {
    static FRAME_COUNT: Cell<u32> = Cell::new(0);
    static REQUEST_STOP: Cell<Option<extern "C" fn()>> = Cell::new(None);
}

#[no_mangle]
pub extern "C" fn lowarc_module_start(_settings_json: *const c_char, request_stop: extern "C" fn()) {
    trace("started");
    REQUEST_STOP.with(|r| r.set(Some(request_stop)));
}

/// Proves the inter-module wire-protocol extension's native-ABI side for real (see
/// native_module_host.rs's own header comment for the design): reads the shared-state JSON string
/// it was handed (this module requires nothing, so it should always be the empty object) and
/// calls `publish` with one of its own — a crash or hang here would mean the CString/callback
/// plumbing on the host side is actually broken, not just that it type-checks. Stops itself after
/// a couple of frames rather than running forever.
#[no_mangle]
pub extern "C" fn lowarc_module_frame(_delta_seconds: f64, shared_json: *const c_char, publish: extern "C" fn(*const c_char)) {
    let shared = if shared_json.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(shared_json) }.to_string_lossy().into_owned()
    };
    let count = FRAME_COUNT.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n
    });
    trace(&format!("frame {count} shared={shared}"));

    if let Ok(published) = std::ffi::CString::new(format!(r#"{{"frameCount":{count}}}"#)) {
        publish(published.as_ptr());
    }

    if count >= 2 {
        REQUEST_STOP.with(|r| {
            if let Some(f) = r.get() {
                f();
            }
        });
    }
}

#[no_mangle]
pub extern "C" fn lowarc_module_stop() {
    trace("stopped");
}
