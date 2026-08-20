// Exercises the real thing: a project.json preset, resolved against a real global module store,
// run through the actual runtime::start_run — a real PowerShell process module spawned, ticked a
// few real frames, and stopped, both by its own request and (in the second test) by an external
// Stop. Not a unit test of one function — the same standard as lowarc/Bootstrap's smoke tests.

use lowarc_studio_lib::runtime;
use lowarc_studio_lib::runtime::runtime_loader::LogLevel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

fn collecting_logger() -> (Arc<dyn Fn(LogLevel, &str) + Send + Sync>, Arc<Mutex<Vec<String>>>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let sink = messages.clone();
    let log: Arc<dyn Fn(LogLevel, &str) + Send + Sync> = Arc::new(move |_level, msg| {
        sink.lock().unwrap().push(msg.to_string());
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

    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log);

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    let messages = messages.lock().unwrap();
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
    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log);

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    assert!(started.elapsed() < Duration::from_secs(10), "run should have ended promptly once the external flag was set");
}
