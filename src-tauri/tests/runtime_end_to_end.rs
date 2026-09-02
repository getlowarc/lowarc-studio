// Exercises the real thing: a project.json preset, resolved against a real global module store,
// run through the actual runtime::start_run — a real PowerShell process module spawned, ticked a
// few real frames, and stopped, both by its own request and (in the second test) by an external
// Stop. Not a unit test of one function — the same standard as lowarc/Bootstrap's smoke tests.

use lowarc_studio_lib::runtime;
use lowarc_studio_lib::runtime::runtime_loader::{Breakpoint, DebugHooks, FrameTrace, LogFn};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fixture_module(modules_dir: &std::path::Path, frames_before_stop: u32) {
    let dir = modules_dir.join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"id":"echo","name":"Echo Module","loadOrder":1,"requires":[]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("module.ps1"),
        format!(
            r#"
$frameCount = 0
while ($line = [Console]::In.ReadLine()) {{
    if ([string]::IsNullOrWhiteSpace($line)) {{ continue }}
    $msg = $line | ConvertFrom-Json
    switch ($msg.phase) {{
        "compile" {{ $reply = @{{ ok = $true }} }}
        "start" {{
            $log = @{{ log = @{{ severity = "info"; message = "hello from the studio runtime" }} }} | ConvertTo-Json -Compress
            [Console]::Out.WriteLine($log); [Console]::Out.Flush()
            $reply = @{{ ok = $true }}
        }}
        "frame" {{
            $frameCount++
            if ($frameCount -ge {frames_before_stop}) {{
                $stop = @{{ requestStop = $true }} | ConvertTo-Json -Compress
                [Console]::Out.WriteLine($stop); [Console]::Out.Flush()
            }}
            $reply = @{{ ok = $true }}
        }}
        "stop" {{ $reply = @{{ ok = $true }} }}
        default {{ $reply = @{{ ok = $false; error = "unknown phase" }} }}
    }}
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") {{ break }}
}}
"#
        ),
    )
    .unwrap();
}

fn collecting_logger() -> (LogFn, Arc<Mutex<Vec<String>>>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let sink = messages.clone();
    let log: LogFn = Arc::new(move |_level, msg| {
        sink.lock().push(msg.to_string());
    });
    (log, messages)
}

#[test]
fn runs_a_process_module_end_to_end_via_its_own_request_stop() {
    if !cfg!(windows) {
        return; // fixture is a PowerShell script — matches how Bootstrap's own tests are scoped
    }

    let modules_dir = temp_dir("self_stop_modules");
    write_fixture_module(&modules_dir, 3);

    let project_dir = temp_dir("self_stop_project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"echo","version":"*"}]}"#).unwrap();

    let uc_dir = project_dir.join("uc");
    std::fs::create_dir_all(&uc_dir).unwrap();
    let entry = uc_dir.join("main.txt");
    std::fs::write(&entry, "hello").unwrap();

    let (log, messages) = collecting_logger();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log, runtime::runtime_loader::DebugHooks::disabled());

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    let messages = messages.lock();
    assert!(
        messages.iter().any(|m| m.contains("hello from the studio runtime")),
        "expected the module's own log line to reach the caller, got {messages:?}"
    );
}

#[test]
fn an_external_stop_flag_ends_a_run_that_never_asks_to_stop_itself() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("external_stop_modules");
    // A huge frame count means it would never request its own stop within the test's timeout —
    // proving this run only ends because the external flag was set, not because the module asked.
    write_fixture_module(&modules_dir, 1_000_000);

    let project_dir = temp_dir("external_stop_project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"echo","version":"*"}]}"#).unwrap();
    let uc_dir = project_dir.join("uc");
    std::fs::create_dir_all(&uc_dir).unwrap();
    let entry = uc_dir.join("main.txt");
    std::fs::write(&entry, "hello").unwrap();

    let (log, _messages) = collecting_logger();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_for_timer = stop_flag.clone();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        stop_flag_for_timer.store(true, Ordering::SeqCst);
    });

    let started = std::time::Instant::now();
    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log, runtime::runtime_loader::DebugHooks::disabled());

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    assert!(started.elapsed() < Duration::from_secs(10), "run should have ended promptly once the external flag was set");
}

#[test]
fn pausing_then_stepping_advances_exactly_one_frame_at_a_time() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("pause_step_modules");
    write_fixture_module(&modules_dir, 1_000_000); // never self-stops within the test window

    let project_dir = temp_dir("pause_step_project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"echo","version":"*"}]}"#).unwrap();
    let uc_dir = project_dir.join("uc");
    std::fs::create_dir_all(&uc_dir).unwrap();
    let entry = uc_dir.join("main.txt");
    std::fs::write(&entry, "hello").unwrap();

    let (log, _messages) = collecting_logger();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let step_request = Arc::new(AtomicU32::new(0));
    let frames_seen = Arc::new(AtomicU32::new(0));

    let frames_seen_cb = frames_seen.clone();
    let on_frame: Arc<dyn Fn(FrameTrace) + Send + Sync> = Arc::new(move |_trace| {
        frames_seen_cb.fetch_add(1, Ordering::SeqCst);
    });

    let debug = DebugHooks { pause_flag: pause_flag.clone(), step_request: step_request.clone(), breakpoints: Arc::new(Mutex::new(Vec::new())), on_frame };

    let stop_flag_for_thread = stop_flag.clone();
    // A high target_fps means the loop is always ready to tick the instant it's allowed to, so the
    // test doesn't have to line itself up against a real frame interval.
    let handle = std::thread::spawn(move || runtime::start_run(&entry, &project_dir, &modules_dir, 200, serde_json::json!({}), stop_flag_for_thread, log, debug));

    std::thread::sleep(Duration::from_millis(300)); // let the module actually start up before pausing it
    pause_flag.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300)); // let any already-in-flight tick land and the loop settle into idle
    let baseline = frames_seen.load(Ordering::SeqCst);

    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(frames_seen.load(Ordering::SeqCst), baseline, "no new frame should appear while paused with no step requested");

    step_request.store(1, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(frames_seen.load(Ordering::SeqCst), baseline + 1, "exactly one new frame should appear after a single step");

    let after_step = frames_seen.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(frames_seen.load(Ordering::SeqCst), after_step, "run should re-pause after consuming its one requested step, not keep advancing");

    stop_flag.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
}

#[test]
fn a_frame_count_breakpoint_pauses_the_run_automatically() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("bp_frame_count_modules");
    write_fixture_module(&modules_dir, 1_000_000);

    let project_dir = temp_dir("bp_frame_count_project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"echo","version":"*"}]}"#).unwrap();
    let uc_dir = project_dir.join("uc");
    std::fs::create_dir_all(&uc_dir).unwrap();
    let entry = uc_dir.join("main.txt");
    std::fs::write(&entry, "hello").unwrap();

    let (log, _messages) = collecting_logger();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let breakpoints = Arc::new(Mutex::new(vec![Breakpoint::FrameCount { count: 3 }]));
    let last_trace = Arc::new(Mutex::new(None));

    let last_trace_cb = last_trace.clone();
    let on_frame: Arc<dyn Fn(FrameTrace) + Send + Sync> = Arc::new(move |trace| {
        *last_trace_cb.lock() = Some(trace);
    });

    let debug = DebugHooks { pause_flag: pause_flag.clone(), step_request: Arc::new(AtomicU32::new(0)), breakpoints, on_frame };

    let stop_flag_for_thread = stop_flag.clone();
    let handle = std::thread::spawn(move || runtime::start_run(&entry, &project_dir, &modules_dir, 200, serde_json::json!({}), stop_flag_for_thread, log, debug));

    // Nothing else gates this run, so if the breakpoint didn't fire it would be well past frame 3
    // within this window — generous to absorb a slow PowerShell process startup, not because
    // reaching frame 3 itself is slow.
    std::thread::sleep(Duration::from_millis(1000));

    assert!(pause_flag.load(Ordering::SeqCst), "the run should have paused itself on hitting the frame-count breakpoint");
    let trace = last_trace.lock().clone().expect("expected a frame trace to accompany the breakpoint hit");
    assert_eq!(trace.frame_index, 3);
    assert!(
        matches!(trace.triggered, Some(Breakpoint::FrameCount { count: 3 })),
        "expected the FrameCount breakpoint to be reported as the trigger, got {:?}",
        trace.triggered
    );

    // Confirm it's genuinely stopped, not just paused-and-still-ticking.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(last_trace.lock().as_ref().unwrap().frame_index, 3, "should stay paused at frame 3, not keep advancing");

    stop_flag.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
}
