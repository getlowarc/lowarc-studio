// Proves bin/node_graph_runtime.rs (the first real "consuming module" for a .lan node graph — see
// that file's own header comment for the design) actually walks a real graph correctly, not just
// that it type-checks. Observing what it published each tick reuses the debugger's own mechanism
// (a frame-count breakpoint's on_frame callback carries every module's raw reply, publish field
// included) rather than inventing a second way to peek at internal state from outside.

use lowarc_studio_lib::runtime;
use lowarc_studio_lib::runtime::runtime_loader::{Breakpoint, DebugHooks, FrameTrace, LogFn};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_ngr_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const LINEAR_GRAPH: &str = r#"{
  "lowarcNodeGraph": 1,
  "nodes": [
    {"id": "n1", "label": "Start Scene", "x": 0, "y": 0, "files": ["scenes/start.txt"], "hasInput": false, "hasOutput": true},
    {"id": "n2", "label": "Middle Scene", "x": 100, "y": 0, "files": ["scenes/middle.txt"], "hasInput": true, "hasOutput": true},
    {"id": "n3", "label": "End Scene", "x": 200, "y": 0, "files": ["scenes/end.txt"], "hasInput": true, "hasOutput": false}
  ],
  "connections": [
    {"from": "n1", "to": "n2"},
    {"from": "n2", "to": "n3"}
  ]
}"#;

fn write_interpreter_module(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("node-graph-runtime");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"id":"node-graph-runtime","name":"Node Graph Runtime","loadOrder":1,"requires":[{"id":"input","version":"*","optional":true}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("process.json"), r#"{"command":"node_graph_runtime","args":[],"wantsFrames":true}"#).unwrap();
    let built_exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_node_graph_runtime"));
    let file_name = if cfg!(windows) { "node_graph_runtime.exe" } else { "node_graph_runtime" };
    std::fs::copy(&built_exe, dir.join(file_name)).unwrap();
}

// Publishes {"advanceTo": "n2"} starting on its second frame — this module's own loadOrder (2,
// after node-graph-runtime's 1) doesn't actually matter for ordering here; node-graph-runtime
// REQUIRES "input", and that's what puts it after input in the requires-DFS regardless.
fn write_input_module(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("input");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"input","name":"Fake Input","loadOrder":2,"requires":[]}"#).unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("module.ps1"),
        r#"
$frameCount = 0
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    switch ($msg.phase) {
        "compile" { $reply = @{ ok = $true } }
        "start" { $reply = @{ ok = $true } }
        "frame" {
            $frameCount++
            if ($frameCount -ge 2) {
                $reply = @{ ok = $true; publish = @{ advanceTo = "n2" } }
            } else {
                $reply = @{ ok = $true }
            }
        }
        "stop" { $reply = @{ ok = $true } }
        default { $reply = @{ ok = $false; error = "unknown phase" } }
    }
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress -Depth 5)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();
}

fn write_project(project_dir: &std::path::Path, requires_json: &str) -> std::path::PathBuf {
    std::fs::write(project_dir.join("project.json"), format!(r#"{{"requires":{requires_json}}}"#)).unwrap();
    let entry = project_dir.join("graph.lan");
    std::fs::write(&entry, LINEAR_GRAPH).unwrap();
    entry
}

fn noop_logger() -> LogFn {
    Arc::new(|_level, _msg| {})
}

/// Runs to a frame-count breakpoint, returns that frame's captured trace, then stops the run:
/// same pattern runtime_end_to_end.rs's own breakpoint test already uses.
fn run_to_frame_and_capture(entry: &std::path::Path, project_dir: &std::path::Path, modules_dir: &std::path::Path, frame_count: u64) -> FrameTrace {
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
        breakpoints: Arc::new(Mutex::new(vec![Breakpoint::FrameCount { count: frame_count }])),
        on_frame,
    };

    let entry = entry.to_path_buf();
    let project_dir = project_dir.to_path_buf();
    let modules_dir = modules_dir.to_path_buf();
    let stop_flag_for_thread = stop_flag.clone();
    let handle =
        std::thread::spawn(move || runtime::start_run(&entry, &project_dir, &modules_dir, 200, serde_json::json!({}), stop_flag_for_thread, noop_logger(), debug));

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while last_trace.lock().is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for frame {frame_count}");
        std::thread::sleep(Duration::from_millis(20));
    }

    let trace = last_trace.lock().clone().unwrap();
    stop_flag.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    trace
}

fn published_by(trace: &FrameTrace, module_id: &str) -> serde_json::Value {
    trace
        .modules
        .iter()
        .find(|m| m.id == module_id)
        .and_then(|m| m.reply.get("publish"))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[test]
fn the_interpreter_starts_on_the_marked_start_node_and_stays_there_without_an_input_module() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("no_input_modules");
    write_interpreter_module(&modules_dir);
    // Deliberately no "input" module — node-graph-runtime's own manifest requires it, but a
    // project not providing one is a normal, supported case (see that module's own header
    // comment): it should just sit on its start node forever, not error.

    let project_dir = temp_dir("no_input_project");
    let entry = write_project(&project_dir, r#"[{"id":"node-graph-runtime","version":"*"}]"#);

    let trace = run_to_frame_and_capture(&entry, &project_dir, &modules_dir, 2);
    let published = published_by(&trace, "node-graph-runtime");
    assert_eq!(published["activeNodeId"], "n1", "should start on n1 (the only node marked hasInput:false) and stay there, got {published:?}");
    assert_eq!(published["activeNodeLabel"], "Start Scene");
    assert_eq!(published["activeNodeFiles"], serde_json::json!(["scenes/start.txt"]));
    assert_eq!(published["choices"], serde_json::json!(["n2"]), "n1 has exactly one outgoing connection");
}

#[test]
fn the_interpreter_advances_when_an_input_module_tells_it_to() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("with_input_modules");
    write_interpreter_module(&modules_dir);
    write_input_module(&modules_dir);

    let project_dir = temp_dir("with_input_project");
    let entry = write_project(&project_dir, r#"[{"id":"node-graph-runtime","version":"*"},{"id":"input","version":"*"}]"#);

    // input starts publishing advanceTo:"n2" on ITS OWN second frame, and node-graph-runtime
    // requires input, so by the time this same tick reaches node-graph-runtime, it already sees
    // that fresh value (the requires-ordering guarantee spawn_and_run's own comment describes),
    // not one tick stale. Frame 2 is therefore already enough to prove it, not just frame 3+.
    let trace = run_to_frame_and_capture(&entry, &project_dir, &modules_dir, 2);
    let published = published_by(&trace, "node-graph-runtime");
    assert_eq!(published["activeNodeId"], "n2", "should have advanced to n2 the same tick input published advanceTo, got {published:?}");
    assert_eq!(published["activeNodeLabel"], "Middle Scene");
    assert_eq!(published["choices"], serde_json::json!(["n3"]));
}
