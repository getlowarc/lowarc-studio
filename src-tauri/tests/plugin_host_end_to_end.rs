// Proves the actual point of invoking plugins per call rather than keeping a process alive: a
// well-behaved plugin replies (and any log line it sends before its reply is captured), a plugin
// that never replies times out instead of hanging the host, a plugin that exits early reports an
// error promptly, and concurrent invocations of the same plugin stay correctly isolated from each
// other: trivially true now, since each call gets its own process, but worth confirming directly
// rather than by luck.

use lowarc_studio_lib::plugin_host::protocol::{self, LogFn, PluginDescriptor};
use serde_json::Value;
use std::sync::{Arc, Mutex};

fn no_op_log() -> LogFn {
    Arc::new(|_level, _msg| {})
}

fn collecting_log() -> (LogFn, Arc<Mutex<Vec<String>>>) {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let sink = logs.clone();
    let log: LogFn = Arc::new(move |_level, msg| sink.lock().unwrap().push(msg.to_string()));
    (log, logs)
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_plugin_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_well_behaved_plugin_replies_and_its_log_line_is_captured() {
    if !cfg!(windows) {
        return;
    }

    let plugins_dir = temp_dir("well_behaved");
    let dir = plugins_dir.join("hello");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","plugin.ps1"]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("plugin.ps1"),
        r#"
$line = [Console]::In.ReadLine()
$msg = $line | ConvertFrom-Json
$log = @{ log = @{ severity = "info"; message = "handling $($msg.method)" } } | ConvertTo-Json -Compress
[Console]::Out.WriteLine($log); [Console]::Out.Flush()
$reply = @{ ok = $true; result = "reply-for-$($msg.method)" } | ConvertTo-Json -Compress
[Console]::Out.WriteLine($reply); [Console]::Out.Flush()
"#,
    )
    .unwrap();

    let desc = PluginDescriptor::read(&dir).expect("plugin.json should be readable");
    let (log, logs) = collecting_log();
    let reply = protocol::invoke(&dir, &desc, "hello", "ping", &Value::Null, &log);

    assert_eq!(reply.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(reply.get("result").and_then(|v| v.as_str()), Some("reply-for-ping"));
    assert!(
        logs.lock().unwrap().iter().any(|m| m.contains("handling ping")),
        "the log line sent before the reply should have reached the log callback"
    );
}

#[test]
fn a_plugin_that_never_replies_times_out_instead_of_hanging() {
    if !cfg!(windows) {
        return;
    }

    let plugins_dir = temp_dir("hangs");
    let dir = plugins_dir.join("slow");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","plugin.ps1"],"timeoutMs":300}"#,
    )
    .unwrap();
    // Reads the request, then just sits there: never writes a reply. The host has to notice on
    // its own via the timeout, not by anything the plugin says.
    std::fs::write(dir.join("plugin.ps1"), "$line = [Console]::In.ReadLine()\nStart-Sleep -Seconds 30\n").unwrap();

    let desc = PluginDescriptor::read(&dir).expect("plugin.json should be readable");
    let started = std::time::Instant::now();
    let reply = protocol::invoke(&dir, &desc, "slow", "ping", &Value::Null, &no_op_log());

    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "invoke() must return once its 300ms timeout fires, not hang for the full 30s the plugin sleeps"
    );
    assert_eq!(reply.get("ok").and_then(|v| v.as_bool()), Some(false));
}

#[test]
fn a_plugin_that_exits_without_replying_reports_an_error_not_a_hang() {
    if !cfg!(windows) {
        return;
    }

    let plugins_dir = temp_dir("crashes");
    let dir = plugins_dir.join("bad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","plugin.ps1"],"timeoutMs":2000}"#,
    )
    .unwrap();
    std::fs::write(dir.join("plugin.ps1"), "$line = [Console]::In.ReadLine()\nexit 1\n").unwrap();

    let desc = PluginDescriptor::read(&dir).expect("plugin.json should be readable");
    let started = std::time::Instant::now();
    let reply = protocol::invoke(&dir, &desc, "bad", "ping", &Value::Null, &no_op_log());

    assert_eq!(reply.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "an early exit should be noticed (stdout EOF) and reported quickly, not wait out the full 2s timeout"
    );
}

#[test]
fn concurrent_invocations_of_the_same_plugin_stay_isolated() {
    if !cfg!(windows) {
        return;
    }

    let plugins_dir = temp_dir("concurrent");
    let dir = plugins_dir.join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","plugin.ps1"]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("plugin.ps1"),
        r#"
$line = [Console]::In.ReadLine()
$msg = $line | ConvertFrom-Json
$reply = @{ ok = $true; result = "reply-for-$($msg.method)" } | ConvertTo-Json -Compress
[Console]::Out.WriteLine($reply); [Console]::Out.Flush()
"#,
    )
    .unwrap();

    let mut handles = Vec::new();
    for i in 0..8 {
        let dir = dir.clone();
        handles.push(std::thread::spawn(move || {
            let desc = PluginDescriptor::read(&dir).expect("plugin.json should be readable");
            let method = format!("method-{i}");
            let reply = protocol::invoke(&dir, &desc, "echo", &method, &Value::Null, &no_op_log());
            (method, reply)
        }));
    }

    for handle in handles {
        let (method, reply) = handle.join().unwrap();
        assert_eq!(
            reply.get("result").and_then(|r| r.as_str()),
            Some(format!("reply-for-{method}").as_str()),
            "each concurrent invocation — its own process, no shared connection — must get its own matching reply"
        );
    }
}
