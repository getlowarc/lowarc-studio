// The actual dlopen for exactly one native-kind module: deliberately its own disposable process,
// spawned by runtime::native_module::NativeLoader, one per module. This is where the isolation
// lives: whatever the module's native code does (crash, corrupt its own memory, spin forever) can
// only take this process down, never the long-lived LowArc Studio process that spawned it.
//
// Speaks the exact same wire protocol as a process.json module (runtime::process_module) — one
// JSON object per line on stdin/stdout, compile/start/frame/stop phases, {"log":...}/
// {"requestStop":true} notifications, and now "shared"/"publish" for inter-module communication
// too (see process_module.rs's own header comment for the full design), so NativeLoader can
// drive it through process_module's existing spawn/compile/start/frame-loop/stop lifecycle
// unchanged, same as a real process module. "compile" is a no-op here (native code is already
// compiled); replied to immediately so the shared lifecycle doesn't need to know which kind of
// module it's talking to.
//
// "shared" crosses the C ABI the same way "settings" already does at start(): serialized to a
// JSON string, handed across as a plain `*const c_char`. "publish" goes the other way via a
// callback (PublishFn), the same shape request_stop already is: the module calls it zero or more
// times during frame() with a JSON *object* string, and whatever it passed gets merged (by key,
// last call wins on a collision) into PUBLISH_BUFFER, which this process reads back out and sends
// as this frame's reply once frame_fn returns. A callback rather than a return value sidesteps any
// question of who owns/frees a string handed back across the FFI boundary: nothing has to.

use lowarc_studio_lib::dylib::Library;
use lowarc_studio_lib::runtime::native_module::{platform_library_file_name, NativeDescriptor};
use serde_json::{json, Map, Value};
use std::ffi::CString;
use std::io::{self, BufRead, Write};
use std::os::raw::c_char;
use std::path::PathBuf;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

type RequestStopFn = extern "C" fn();
type PublishFn = extern "C" fn(*const c_char);
type StartFn = extern "C" fn(*const c_char, RequestStopFn);
type FrameFn = extern "C" fn(f64, *const c_char, PublishFn);
type StopFn = extern "C" fn();

/// Serializes stdout writes: the module's own request_stop() callback can fire from a thread the
/// module created itself, concurrently with this process's own main-loop reply to a frame/stop
/// message. Without this, two writers could interleave mid-line and hand the host an unparseable
/// (or worse, wrongly-parseable) line.
fn stdout_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// What this frame's publish() calls have accumulated so far: cleared right before each frame()
/// call, read back out (and included in the reply) right after it returns. A module's own
/// request_stop() callback can already fire from a thread it created itself (see stdout_lock's
/// comment); publish() is documented as frame()-only (called synchronously, from the same thread
/// frame() itself runs on) specifically so this doesn't need the same cross-thread story: a plain
/// Mutex is enough, not because publish() couldn't race but because a well-behaved module never
/// gives it the chance to.
fn publish_buffer() -> &'static Mutex<Map<String, Value>> {
    static BUFFER: OnceLock<Mutex<Map<String, Value>>> = OnceLock::new();
    BUFFER.get_or_init(|| Mutex::new(Map::new()))
}

/// The module calls this any number of times during frame() with a JSON *object* string: anything
/// else (unparseable, or valid JSON that isn't an object) is silently dropped rather than killing
/// the module's whole frame over one malformed publish call.
extern "C" fn publish(json_str: *const c_char) {
    if json_str.is_null() {
        return;
    }
    let Ok(s) = (unsafe { std::ffi::CStr::from_ptr(json_str) }).to_str() else { return };
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(s) else { return };
    publish_buffer().lock().extend(obj);
}

fn write_line(value: &Value) {
    let _guard = stdout_lock().lock();
    let mut out = io::stdout();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn reply(ok: bool, mut extra: Map<String, Value>) {
    extra.insert("ok".into(), Value::Bool(ok));
    write_line(&Value::Object(extra));
}

extern "C" fn request_stop() {
    write_line(&json!({"requestStop": true}));
}

fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn main() {
    let folder = match std::env::args().nth(1) {
        Some(f) => PathBuf::from(f),
        None => fatal("usage: native_module_host <module-folder>"),
    };

    let desc = match NativeDescriptor::read(&folder) {
        Some(d) => d,
        None => fatal(&format!("no readable native.json in {}", folder.display())),
    };

    let file_name = platform_library_file_name(&desc.library);
    let lib = match Library::open(&folder.join(&file_name)) {
        Ok(l) => l,
        Err(e) => fatal(&format!("failed to open {file_name}: {e}")),
    };

    let start: StartFn = match lib.symbol("lowarc_module_start") {
        Ok(p) => unsafe { std::mem::transmute::<*mut std::ffi::c_void, StartFn>(p) },
        Err(e) => fatal(&format!("module is missing lowarc_module_start: {e}")),
    };
    let frame_fn: Option<FrameFn> = lib.symbol("lowarc_module_frame").ok().map(|p| unsafe { std::mem::transmute(p) });
    let stop_fn: Option<StopFn> = lib.symbol("lowarc_module_stop").ok().map(|p| unsafe { std::mem::transmute(p) });
    let started = AtomicBool::new(false);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let phase = msg.get("phase").and_then(|p| p.as_str()).unwrap_or("");

        match phase {
            "compile" => reply(true, json!({"items": []}).as_object().cloned().unwrap()),
            "start" => {
                let settings = msg.get("settings").cloned().unwrap_or(Value::Null).to_string();
                let settings_c = CString::new(settings).unwrap_or_else(|_| CString::new("").unwrap());
                start(settings_c.as_ptr(), request_stop);
                started.store(true, Ordering::SeqCst);
                reply(true, Map::new());
            }
            "frame" => {
                let mut extra = Map::new();
                if started.load(Ordering::SeqCst) {
                    if let Some(f) = frame_fn {
                        let delta = msg.get("delta").and_then(|d| d.as_f64()).unwrap_or(0.0);
                        let shared = msg.get("shared").cloned().unwrap_or_else(|| Value::Object(Map::new())).to_string();
                        let shared_c = CString::new(shared).unwrap_or_else(|_| CString::new("{}").unwrap());
                        publish_buffer().lock().clear();
                        f(delta, shared_c.as_ptr(), publish);
                        let published = std::mem::take(&mut *publish_buffer().lock());
                        if !published.is_empty() {
                            extra.insert("publish".into(), Value::Object(published));
                        }
                    }
                }
                reply(true, extra);
            }
            "stop" => {
                if started.load(Ordering::SeqCst) {
                    if let Some(f) = stop_fn {
                        f();
                    }
                }
                reply(true, Map::new());
                break;
            }
            other => reply(false, json!({"error": format!("unknown phase \"{other}\"")}).as_object().cloned().unwrap()),
        }
    }
}
