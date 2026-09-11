// Proves the "shared"/"publish" wire-protocol extension (see process_module.rs's own header
// comment for the design) actually works between two real process modules, not just that it
// type-checks: same standard as runtime_end_to_end.rs's other smoke tests.

use lowarc_studio_lib::runtime;
use lowarc_studio_lib::runtime::runtime_loader::LogFn;
use parking_lot::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_imc_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Publishes {"value": <frame count>} every frame, forever — relies on the run being stopped
// externally (by the consumer requesting stop, or the test's own stop_flag), same as
// runtime_end_to_end.rs's own "never self-stops" fixtures.
fn write_producer(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("producer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"producer","name":"Producer","loadOrder":1,"requires":[]}"#).unwrap();
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
            $reply = @{ ok = $true; publish = @{ value = $frameCount } }
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

// requires producer — logs whatever it sees at shared.producer.value each frame, and requests
// stop once that value reaches 3. The log line is the actual proof: it can only ever contain a
// real number here if the value genuinely round-tripped from the producer's own publish, through
// the host's shared map, back down to this module's own frame request.
fn write_consumer(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("consumer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"id":"consumer","name":"Consumer","loadOrder":2,"requires":[{"id":"producer","version":"*"}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("module.ps1"),
        r#"
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    switch ($msg.phase) {
        "compile" { $reply = @{ ok = $true } }
        "start" { $reply = @{ ok = $true } }
        "frame" {
            $seen = $msg.shared.producer.value
            if ($seen) {
                $log = @{ log = @{ severity = "info"; message = "consumer saw producer.value=$seen" } } | ConvertTo-Json -Compress
                [Console]::Out.WriteLine($log); [Console]::Out.Flush()
                if ($seen -ge 3) {
                    $stop = @{ requestStop = $true } | ConvertTo-Json -Compress
                    [Console]::Out.WriteLine($stop); [Console]::Out.Flush()
                }
            }
            $reply = @{ ok = $true }
        }
        "stop" { $reply = @{ ok = $true } }
        default { $reply = @{ ok = $false; error = "unknown phase" } }
    }
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();
}

// Does NOT require producer — dumps the raw shared object it received into its own log every
// frame, so the test can assert producer's namespace never appears in it. Requests its own stop
// after a few frames so the test doesn't depend on anything external to end it.
fn write_bystander(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("bystander");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"bystander","name":"Bystander","loadOrder":3,"requires":[]}"#).unwrap();
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
            $sharedJson = $msg.shared | ConvertTo-Json -Compress
            $log = @{ log = @{ severity = "info"; message = "bystander frame $frameCount shared=$sharedJson" } } | ConvertTo-Json -Compress
            [Console]::Out.WriteLine($log); [Console]::Out.Flush()
            if ($frameCount -ge 3) {
                $stop = @{ requestStop = $true } | ConvertTo-Json -Compress
                [Console]::Out.WriteLine($stop); [Console]::Out.Flush()
            }
            $reply = @{ ok = $true }
        }
        "stop" { $reply = @{ ok = $true } }
        default { $reply = @{ ok = $false; error = "unknown phase" } }
    }
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();
}

fn write_project(project_dir: &std::path::Path, requires_json: &str) -> std::path::PathBuf {
    std::fs::write(project_dir.join("project.json"), format!(r#"{{"requires":{requires_json}}}"#)).unwrap();
    let uc_dir = project_dir.join("uc");
    std::fs::create_dir_all(&uc_dir).unwrap();
    let entry = uc_dir.join("main.txt");
    std::fs::write(&entry, "hello").unwrap();
    entry
}

fn collecting_logger() -> (LogFn, Arc<Mutex<Vec<String>>>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let sink = messages.clone();
    let log: LogFn = Arc::new(move |_level, msg| sink.lock().push(msg.to_string()));
    (log, messages)
}

#[test]
fn a_module_sees_its_dependency_publish_within_the_same_frame() {
    if !cfg!(windows) {
        return; // fixtures are PowerShell scripts — matches how the other e2e tests are scoped
    }

    let modules_dir = temp_dir("producer_consumer_modules");
    write_producer(&modules_dir);
    write_consumer(&modules_dir);

    let project_dir = temp_dir("producer_consumer_project");
    let entry = write_project(&project_dir, r#"[{"id":"producer","version":"*"},{"id":"consumer","version":"*"}]"#);

    let (log, messages) = collecting_logger();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log, runtime::runtime_loader::DebugHooks::disabled());

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    let messages = messages.lock();
    assert!(
        messages.iter().any(|m| m.contains("consumer saw producer.value=1")),
        "expected the consumer to see the producer's very first published frame, got {messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("consumer saw producer.value=3")),
        "expected the consumer to keep seeing fresh values across frames, got {messages:?}"
    );
}

#[test]
fn a_module_only_sees_shared_state_from_modules_it_requires() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("producer_bystander_modules");
    write_producer(&modules_dir);
    write_bystander(&modules_dir);

    let project_dir = temp_dir("producer_bystander_project");
    let entry = write_project(&project_dir, r#"[{"id":"producer","version":"*"},{"id":"bystander","version":"*"}]"#);

    let (log, messages) = collecting_logger();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let result = runtime::start_run(&entry, &project_dir, &modules_dir, 30, serde_json::json!({}), stop_flag, log, runtime::runtime_loader::DebugHooks::disabled());

    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
    let messages = messages.lock();
    assert!(messages.iter().any(|m| m.contains("bystander frame")), "expected the bystander to have logged at least one frame, got {messages:?}");
    assert!(
        !messages.iter().any(|m| m.contains("bystander") && m.contains("producer")),
        "bystander never declared requiring producer, so its shared view should never contain producer's namespace at all, got {messages:?}"
    );
}
