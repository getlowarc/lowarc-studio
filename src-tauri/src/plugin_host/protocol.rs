// The plugin<->host wire protocol: one JSON object per line over stdin/stdout, same shape
// runtime/process_module.rs already proved for modules, different vocabulary. Deliberately a
// SEPARATE protocol, not a shared one — modules and plugins are independent systems (Nolan's
// call: a module built with knowledge of a specific plugin would be fragile the moment a project
// doesn't have that plugin installed), so nothing here reuses process_module's message shapes,
// even where they'd look similar.
//
// v1 scope, deliberately narrow: prove a plugin process is genuinely isolated (a crash there
// can't take the editor down) and can tell the host something happened (a panel registration).
// How a registered panel's actual UI content gets rendered into the webview is NOT decided here —
// that's a real, separate design question, not solved by proving the process/message mechanics.
//
// Messages:
//   host -> plugin:  {"phase":"start","settings":{...}}   {"phase":"stop"}
//   plugin -> host:  {"ok":bool,"error"?:string}                          (replies)
//                    {"log":{"severity":..,"message":..}}                 (notification, any time)
//                    {"registerPanel":{"id":..,"title":..,"location":..}} (notification, any time)

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::runtime_loader::LogLevel;

pub const DESCRIPTOR_NAME: &str = "plugin.json";

type LogFn = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;
type RegisterFn = Arc<dyn Fn(PanelRegistration) + Send + Sync>;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PluginDescriptor {
    pub command: String,
    pub args: Vec<String>,
    #[serde(rename = "timeoutMs", default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    10000
}

impl PluginDescriptor {
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join(DESCRIPTOR_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

#[derive(Debug, Clone)]
pub struct PanelRegistration {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    pub location: String,
}

pub struct PluginProcess {
    pub id: String,
    timeout: Duration,
    stdin: Mutex<ChildStdin>,
    replies: Receiver<Value>,
    child: Mutex<Child>,
    dead: Arc<AtomicBool>,
}

impl PluginProcess {
    pub fn spawn(folder: &Path, desc: &PluginDescriptor, id: String, log: LogFn, on_register: RegisterFn) -> std::io::Result<Self> {
        let local = folder.join(&desc.command);
        let exe = if local.is_file() { local } else { std::path::PathBuf::from(&desc.command) };

        let mut cmd = Command::new(exe);
        cmd.args(&desc.args)
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let dead = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Value>();

        spawn_stdout_reader(stdout, tx, dead.clone(), log.clone(), on_register, id.clone());
        spawn_stderr_reader(stderr, log, id.clone());

        Ok(Self { id, timeout: Duration::from_millis(desc.timeout_ms.max(1)), stdin: Mutex::new(stdin), replies: rx, child: Mutex::new(child), dead })
    }

    fn request(&self, message: Value) -> Value {
        if self.dead.load(Ordering::SeqCst) {
            return json!({"ok": false, "error": "plugin process ended"});
        }
        let mut line = serde_json::to_string(&message).unwrap_or_default();
        line.push('\n');
        {
            let mut stdin = self.stdin.lock().unwrap();
            if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                self.dead.store(true, Ordering::SeqCst);
                return json!({"ok": false, "error": "failed to write to plugin process"});
            }
        }
        match self.replies.recv_timeout(self.timeout) {
            Ok(reply) => reply,
            Err(_) => {
                self.dead.store(true, Ordering::SeqCst);
                json!({"ok": false, "error": "plugin did not respond in time"})
            }
        }
    }

    fn ok(reply: &Value) -> bool {
        !matches!(reply.get("ok"), Some(Value::Bool(false)))
    }

    pub fn start(&self, settings: &Value) -> bool {
        Self::ok(&self.request(json!({"phase": "start", "settings": settings})))
    }

    /// A dead plugin process — crashed, hung, or misbehaving — never reaches the host beyond
    /// this returning false. It cannot take the editor down with it; that's the whole point of
    /// running it as its own process instead of dlopen'd into this one.
    pub fn is_alive(&self) -> bool {
        !self.dead.load(Ordering::SeqCst)
    }

    pub fn stop_and_kill(&self) {
        if !self.dead.load(Ordering::SeqCst) {
            let _ = self.request(json!({"phase": "stop"}));
        }
        self.kill();
    }

    fn kill(&self) {
        self.dead.store(true, Ordering::SeqCst);
        let mut child = self.child.lock().unwrap();
        for _ in 0..25 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn_stdout_reader(
    stdout: std::process::ChildStdout,
    replies: mpsc::Sender<Value>,
    dead: Arc<AtomicBool>,
    log: LogFn,
    on_register: RegisterFn,
    plugin_id: String,
) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                log(LogLevel::Warn, &format!("[{plugin_id}] unparseable line on stdout: {line}"));
                continue;
            };
            let Some(obj) = value.as_object() else { continue };

            if let Some(l) = obj.get("log").and_then(|l| l.as_object()) {
                let severity = l.get("severity").and_then(|s| s.as_str()).unwrap_or("info");
                let message = l.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let level = match severity.to_ascii_lowercase().as_str() {
                    "error" => LogLevel::Error,
                    "warn" | "warning" => LogLevel::Warn,
                    _ => LogLevel::Info,
                };
                log(level, &format!("[{plugin_id}] {message}"));
                continue;
            }
            if let Some(panel) = obj.get("registerPanel").and_then(|p| p.as_object()) {
                on_register(PanelRegistration {
                    plugin_id: plugin_id.clone(),
                    id: panel.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    title: panel.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    location: panel.get("location").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                });
                continue;
            }
            let _ = replies.send(value);
        }
        dead.store(true, Ordering::SeqCst);
    });
}

fn spawn_stderr_reader(stderr: std::process::ChildStderr, log: LogFn, plugin_id: String) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let truncated: String = line.chars().take(300).collect();
            log(LogLevel::Error, &format!("[{plugin_id}] {truncated}"));
        }
    });
}
