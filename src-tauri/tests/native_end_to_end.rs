// Same standard as runtime_end_to_end.rs, for NativeLoader instead of ProcessLoader: builds a
// real tiny Rust cdylib, resolves it through a real project.json + global module store, and runs
// it through the actual runtime::start_run — proving the dlopen/ABI/stop-flag wiring for real,
// not just that it type-checks. This is genuinely new code versus Bootstrap's version (the
// current-run stop-flag handling is different), so it earns its own proof rather than inheriting
// Bootstrap's.

use lowarc_studio_lib::runtime;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_native_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn runs_a_native_module_end_to_end_via_dlopen() {
    let fixture_crate = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/native-fixture-src");
    let build = std::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&fixture_crate)
        .output()
        .expect("failed to invoke cargo for the native fixture");
    assert!(build.status.success(), "fixture build failed: {}", String::from_utf8_lossy(&build.stderr));

    let built_dll = fixture_crate.join("target/release/lowarc_studio_native_fixture.dll");
    assert!(built_dll.exists(), "expected the fixture cdylib at {}", built_dll.display());

    let modules_dir = temp_dir("modules");
    let module_dir = modules_dir.join("native-echo");
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(module_dir.join("manifest.json"), r#"{"id":"native-echo","name":"Native Echo","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(module_dir.join("native.json"), r#"{"library":"lowarc_studio_native_fixture"}"#).unwrap();
    std::fs::copy(&built_dll, module_dir.join("lowarc_studio_native_fixture.dll")).unwrap();

    let project_dir = temp_dir("project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"native-echo","version":"*"}]}"#).unwrap();
    let uc_dir = project_dir.join("uc");
    std::fs::create_dir_all(&uc_dir).unwrap();
    let entry = uc_dir.join("main.txt");
    std::fs::write(&entry, "hello").unwrap();

    let trace_path = temp_dir("trace").join("trace.txt");
    std::env::set_var("LOWARC_STUDIO_TEST_TRACE", &trace_path);

    let log: Arc<dyn Fn(runtime::runtime_loader::LogLevel, &str) + Send + Sync> = {
        let messages = Arc::new(Mutex::new(Vec::<String>::new()));
        Arc::new(move |_level, msg| messages.lock().unwrap().push(msg.to_string()))
    };
    let stop_flag = Arc::new(AtomicBool::new(false));

    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log);
    std::env::remove_var("LOWARC_STUDIO_TEST_TRACE");

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
    assert!(trace.contains("started"), "expected lowarc_module_start to have run, trace was: {trace:?}");
    assert!(trace.contains("stopped"), "expected lowarc_module_stop to have run, trace was: {trace:?}");
}
