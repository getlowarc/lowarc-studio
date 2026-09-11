// Drives the real dev_run_host binary the way Studio does — spawn it on a launch.json, read its
// stdout, write commands to its stdin, and proves the protocol both directions actually works.
//
// This is the load-bearing safety property of moving dev-run out of Studio's process: if the
// control channel silently didn't work, a run would be unstoppable and undebuggable from the IDE
// with nothing to show for it. Same standard as the other *_end_to_end.rs tests here: spawn real
// processes, assert on real output, no mocking of the wire.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_devrunhost_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Every fixture below sets a generous timeoutMs. The 10s default is tuned for a real module, not
// for starting a PowerShell interpreter: this file runs three of them in parallel, and where that
// costs 0.36s on a developer machine it has taken 12s+ on a CI runner. A module timeout exists to
// catch one that has WEDGED, and 30s still does that while tolerating a slow, contended start.

/// Built by the same `cargo test` invocation that builds this test (both are targets of this one
/// package), so it always lands beside the test executable's own directory's parent: the standard
/// layout for a bin target's output.
fn dev_run_host_bin() -> PathBuf {
    let mut dir = std::env::current_exe().expect("test executable path");
    dir.pop(); // the deps/ folder the test binary itself lives in
    dir.pop();
    dir.join(if cfg!(windows) { "dev_run_host.exe" } else { "dev_run_host" })
}

/// Logs one line per frame and never stops on its own, so the run can only ever end because the
/// test told it to, which is exactly what's being verified.
fn write_ticking_module(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"ticker","name":"Ticker","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true,"timeoutMs":30000}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("module.ps1"),
        r#"
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    if ($msg.phase -eq "frame") {
        [Console]::Out.WriteLine((@{ log = @{ severity = "info"; message = "tick" } } | ConvertTo-Json -Compress))
    }
    [Console]::Out.WriteLine((@{ ok = $true } | ConvertTo-Json -Compress)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();
}

/// Same as write_ticking_module, but its "start" reply carries the degraded marker. Still ticks
/// afterwards, which is the half of the contract that would be easy to break: degrading must not
/// be treated as failing.
fn write_degraded_module(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"ticker","name":"Ticker","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true,"timeoutMs":30000}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("module.ps1"),
        r#"
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    if ($msg.phase -eq "frame") {
        [Console]::Out.WriteLine((@{ log = @{ severity = "info"; message = "tick" } } | ConvertTo-Json -Compress))
    }
    if ($msg.phase -eq "start") {
        [Console]::Out.WriteLine((@{ ok = $true; degraded = "no widget frobnicator present" } | ConvertTo-Json -Compress))
    } else {
        [Console]::Out.WriteLine((@{ ok = $true } | ConvertTo-Json -Compress))
    }
    [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();
}

/// Absolute paths in `modules`/`source`, exactly as lib.rs's write_dev_run_launch builds them —
/// run_from_launch_dir joins each against the launch dir, and joining an absolute path yields it
/// unchanged, which is what lets dev-run reuse the export-shaped LaunchConfig with no staging.
fn write_launch(run_dir: &Path, module_dir: &Path, entry: &Path) {
    let launch = serde_json::json!({
        "targetFps": 60,
        "source": entry.to_string_lossy(),
        "modules": [module_dir.to_string_lossy()],
        "diagnosticsLog": false,
        "settings": {},
    });
    std::fs::write(run_dir.join("launch.json"), serde_json::to_string_pretty(&launch).unwrap()).unwrap();
}

#[test]
fn studio_can_drive_a_run_in_its_own_process_and_stop_it() {
    let root = temp_dir("stop");
    let module_dir = root.join("ticker");
    write_ticking_module(&module_dir);
    let entry = root.join("main.txt");
    std::fs::write(&entry, "// entry").unwrap();
    write_launch(&root, &module_dir, &entry);

    let host = dev_run_host_bin();
    assert!(host.is_file(), "expected {} to have been built alongside this test", host.display());

    let mut child = std::process::Command::new(&host)
        .arg(&root)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("dev_run_host should start");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));

    let mut saw_log = false;
    let mut ended: Option<Value> = None;
    let mut sent_stop = false;

    for line in stdout.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };

        if value.get("log").is_some() {
            saw_log = true;
            // The module never stops itself — proving the stop COMMAND is what ends this run, not
            // the module reaching some natural end of its own.
            if !sent_stop {
                sent_stop = true;
                writeln!(stdin, "{}", serde_json::json!({"cmd": "stop"})).unwrap();
                stdin.flush().unwrap();
            }
        } else if let Some(payload) = value.get("ended") {
            ended = Some(payload.clone());
        }
    }

    let _ = child.wait();

    assert!(saw_log, "the run should have relayed the module's own log lines to stdout");
    let ended = ended.expect("the host must always announce that the run ended");
    assert_eq!(ended.get("ok").and_then(Value::as_bool), Some(true), "a stopped run is a clean end, not a failure: {ended}");
}

#[test]
fn a_frame_count_breakpoint_set_over_the_wire_actually_pauses_and_reports_a_trace() {
    let root = temp_dir("breakpoint");
    let module_dir = root.join("ticker");
    write_ticking_module(&module_dir);
    let entry = root.join("main.txt");
    std::fs::write(&entry, "// entry").unwrap();
    write_launch(&root, &module_dir, &entry);

    let mut child = std::process::Command::new(dev_run_host_bin())
        .arg(&root)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("dev_run_host should start");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));

    // Sent the same way Studio sends it at spawn, if this doesn't cross the process boundary, no
    // frame trace can ever appear, since a freely-running loop never emits one.
    writeln!(stdin, "{}", serde_json::json!({"cmd": "setBreakpoints", "breakpoints": [{"kind": "frameCount", "count": 2}]})).unwrap();
    stdin.flush().unwrap();

    let mut trace: Option<Value> = None;
    // Same reasoning as the degraded test's own `seen`: a missing frame trace is otherwise reported
    // with no hint as to whether the module started at all.
    let mut seen: Vec<String> = Vec::new();
    for line in stdout.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        if seen.len() < 40 {
            seen.push(line.chars().take(160).collect());
        }
        if let Some(frame) = value.get("frame") {
            trace = Some(frame.clone());
            writeln!(stdin, "{}", serde_json::json!({"cmd": "stop"})).unwrap();
            stdin.flush().unwrap();
        } else if value.get("ended").is_some() {
            break;
        }
    }

    let _ = child.wait();

    let trace = trace.unwrap_or_else(|| {
        panic!("a frameCount breakpoint should have produced a frame trace. Lines seen were: {seen:#?}")
    });
    assert_eq!(trace.get("frameIndex").and_then(Value::as_u64), Some(2), "the trace should be for the frame the breakpoint named: {trace}");
    assert!(!trace.get("triggered").unwrap_or(&Value::Null).is_null(), "a breakpoint-caused pause must say what triggered it: {trace}");
}

#[test]
fn a_module_that_starts_degraded_says_so_and_keeps_running() {
    // The gap this closes: a module missing something it needed (an audio device, a display) used
    // to run to completion doing nothing, indistinguishable from one that had nothing to do. Both
    // halves matter: the reason has to surface, AND the module has to carry on, since degrading is
    // the intended behaviour rather than a failure.
    let root = temp_dir("degraded");
    let module_dir = root.join("ticker");
    write_degraded_module(&module_dir);
    let entry = root.join("main.txt");
    std::fs::write(&entry, "// entry").unwrap();
    write_launch(&root, &module_dir, &entry);

    let mut child = std::process::Command::new(dev_run_host_bin())
        .arg(&root)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("dev_run_host should start");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));

    let mut degraded_line: Option<String> = None;
    let mut ticked_after = false;
    let mut sent_stop = false;
    // Kept so a failure can say what DID arrive. Without it the panic is just "expected line
    // missing", which on a machine you cannot reproduce on costs a whole CI round trip to learn
    // nothing: the same gap that made the canvas's own failure opaque.
    let mut seen: Vec<String> = Vec::new();

    for line in stdout.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(message) = value.pointer("/log/message").and_then(Value::as_str) else { continue };
        if seen.len() < 40 {
            seen.push(message.to_string());
        }

        if message.contains("started degraded") {
            degraded_line = Some(message.to_string());
        } else if message.contains("tick") {
            // A frame after the degraded start is the proof it wasn't dropped from the run.
            if degraded_line.is_some() {
                ticked_after = true;
            }
            if !sent_stop {
                sent_stop = true;
                writeln!(stdin, "{}", serde_json::json!({"cmd": "stop"})).unwrap();
                stdin.flush().unwrap();
            }
        }
    }

    let _ = child.wait();

    let degraded = degraded_line
        .unwrap_or_else(|| panic!("a degraded start must be reported, not swallowed. Log lines seen were: {seen:#?}"));
    assert!(
        degraded.contains("no widget frobnicator present"),
        "the report should carry the module's own reason, not a generic one: {degraded}"
    );
    assert!(ticked_after, "a degraded module must keep running — degrading is not failing. Saw: {seen:#?}");
}
