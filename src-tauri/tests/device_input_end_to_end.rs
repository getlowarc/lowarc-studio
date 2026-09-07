// Proves bin/device_input_runtime.rs actually runs as a real process module through the real runtime and
// publishes real hardware state — not just that it type-checks in isolation. Asserts on SHAPE
// (the right fields, the right JSON types) rather than exact values, since actual keyboard/mouse/
// gamepad state isn't something a test can control or predict; a manual smoke test (piping the
// wire protocol straight into the built exe) is what actually confirmed live values come back —
// see this module's own header comment for that story. Same debugger-frame-trace observation
// technique as node_graph_runtime_end_to_end.rs.

use lowarc_studio_lib::runtime;
use lowarc_studio_lib::runtime::runtime_loader::{Breakpoint, DebugHooks, FrameTrace, LogFn};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_input_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_input_module(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("device-input");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"device-input","name":"Device Input","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(dir.join("process.json"), r#"{"command":"device_input_runtime","args":[],"wantsFrames":true}"#).unwrap();
    let built_exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_device_input_runtime"));
    let file_name = if cfg!(windows) { "device_input_runtime.exe" } else { "device_input_runtime" };
    std::fs::copy(&built_exe, dir.join(file_name)).unwrap();
}

fn noop_logger() -> LogFn {
    Arc::new(|_level, _msg| {})
}

#[test]
fn the_input_module_publishes_real_keyboard_mouse_and_gamepad_shape() {
    let modules_dir = temp_dir("modules");
    write_input_module(&modules_dir);

    let project_dir = temp_dir("project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"device-input","version":"*"}]}"#).unwrap();
    let entry = project_dir.join("main.txt");
    std::fs::write(&entry, "unused by this module").unwrap();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let last_trace: Arc<Mutex<Option<FrameTrace>>> = Arc::new(Mutex::new(None));

    let last_trace_cb = last_trace.clone();
    let on_frame: Arc<dyn Fn(FrameTrace) + Send + Sync> = Arc::new(move |trace| {
        *last_trace_cb.lock() = Some(trace);
    });
    let debug = DebugHooks {
        pause_flag: pause_flag.clone(),
        step_request: Arc::new(AtomicU32::new(0)),
        breakpoints: Arc::new(Mutex::new(vec![Breakpoint::FrameCount { count: 1 }])),
        on_frame,
    };

    let entry_c = entry.clone();
    let project_dir_c = project_dir.clone();
    let modules_dir_c = modules_dir.clone();
    let stop_flag_for_thread = stop_flag.clone();
    let handle = std::thread::spawn(move || {
        runtime::start_run(&entry_c, &project_dir_c, &modules_dir_c, 30, serde_json::json!({}), stop_flag_for_thread, noop_logger(), debug)
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while last_trace.lock().is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the first frame");
        std::thread::sleep(Duration::from_millis(20));
    }
    let trace = last_trace.lock().clone().unwrap();

    stop_flag.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");

    let published = trace.modules.iter().find(|m| m.id == "device-input").and_then(|m| m.reply.get("publish")).cloned().expect("input module should have published something");

    assert!(published["keyboard"]["keysDown"].is_array(), "keyboard.keysDown should be an array, got {published:?}");
    assert!(published["mouse"]["x"].is_i64() || published["mouse"]["x"].is_u64(), "mouse.x should be a real integer, got {published:?}");
    assert!(published["mouse"]["y"].is_i64() || published["mouse"]["y"].is_u64(), "mouse.y should be a real integer, got {published:?}");
    assert!(published["mouse"]["buttonsDown"].is_array(), "mouse.buttonsDown should be an array, got {published:?}");
    assert!(published["gamepads"].is_array(), "gamepads should be an array (empty is fine — none may be connected), got {published:?}");
}
