// Proves the actual point of running plugins as isolated processes, not just that messages
// flow: a well-behaved plugin registers a panel and stops cleanly, AND a plugin that crashes
// mid-session doesn't hang or take the test process down — the host just observes it died and
// moves on. If the second test didn't hold, isolation would be theater.

use lowarc_studio_lib::plugin_host;
use lowarc_studio_lib::plugin_host::protocol::PanelRegistration;
use lowarc_studio_lib::runtime::runtime_loader::LogLevel;
use std::sync::{Arc, Mutex};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_plugin_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn collectors() -> (
    Arc<dyn Fn(LogLevel, &str) + Send + Sync>,
    Arc<Mutex<Vec<String>>>,
    Arc<dyn Fn(PanelRegistration) + Send + Sync>,
    Arc<Mutex<Vec<PanelRegistration>>>,
) {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let log_sink = logs.clone();
    let log: Arc<dyn Fn(LogLevel, &str) + Send + Sync> = Arc::new(move |_lvl, msg| log_sink.lock().unwrap().push(msg.to_string()));

    let panels = Arc::new(Mutex::new(Vec::new()));
    let panel_sink = panels.clone();
    let on_register: Arc<dyn Fn(PanelRegistration) + Send + Sync> = Arc::new(move |p| panel_sink.lock().unwrap().push(p));

    (log, logs, on_register, panels)
}

#[test]
fn a_well_behaved_plugin_registers_a_panel_and_stops_cleanly() {
    if !cfg!(windows) {
        return;
    }

    let plugins_dir = temp_dir("well_behaved");
    let dir = plugins_dir.join("hello-panel");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","plugin.ps1"]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("plugin.ps1"),
        r#"
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    if ($msg.phase -eq "start") {
        $reg = @{ registerPanel = @{ id = "hello"; title = "Hello Panel"; location = "left" } } | ConvertTo-Json -Compress
        [Console]::Out.WriteLine($reg); [Console]::Out.Flush()
        [Console]::Out.WriteLine('{"ok":true}'); [Console]::Out.Flush()
    } elseif ($msg.phase -eq "stop") {
        [Console]::Out.WriteLine('{"ok":true}'); [Console]::Out.Flush()
        break
    }
}
"#,
    )
    .unwrap();

    let (log, _logs, on_register, panels) = collectors();
    let started = plugin_host::start_all(&plugins_dir, log, on_register);

    assert_eq!(started.len(), 1, "expected exactly one plugin to have started");
    std::thread::sleep(std::time::Duration::from_millis(200)); // let the async registration notification arrive

    let panels = panels.lock().unwrap();
    assert_eq!(panels.len(), 1, "expected the plugin's panel registration to reach the host");
    assert_eq!(panels[0].id, "hello");
    assert_eq!(panels[0].location, "left");

    for p in &started {
        assert!(p.is_alive());
        p.stop_and_kill();
    }
}

#[test]
fn a_crashing_plugin_cannot_take_the_host_down() {
    if !cfg!(windows) {
        return;
    }

    let plugins_dir = temp_dir("crashing");
    let dir = plugins_dir.join("bad-plugin");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","plugin.ps1"],"timeoutMs":2000}"#,
    )
    .unwrap();
    // Replies to "start" successfully, then exits without ever handling anything else — the
    // process just dies, exactly like a real crash would look to the host.
    std::fs::write(dir.join("plugin.ps1"), r#"[Console]::Out.WriteLine('{"ok":true}'); [Console]::Out.Flush(); exit 1"#).unwrap();

    let (log, _logs, on_register, _panels) = collectors();
    let started = plugin_host::start_all(&plugins_dir, log, on_register);
    assert_eq!(started.len(), 1);

    // Give the process time to actually exit, then confirm the host correctly sees it as dead —
    // and, critically, that reaching this line at all means the test process itself is still
    // alive and responsive, not hung waiting on a process that will never answer again.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!started[0].is_alive(), "the host should have noticed the plugin process ended");

    // stop_and_kill on an already-dead plugin must not hang either.
    let stop_started = std::time::Instant::now();
    started[0].stop_and_kill();
    assert!(stop_started.elapsed() < std::time::Duration::from_secs(5));
}
