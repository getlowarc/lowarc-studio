// Port of Bootstrap's process_module.rs — the SAME wire protocol as
// lowarc/Contracts/ProcessModuleLoader.cs (one JSON object per line over stdin/stdout;
// compile/start/frame/stop phases; {"log":...}/{"requestStop":true} notifications), so an
// existing process-kind module works unmodified whether it's run by an export or by this IDE.
// The only real differences from Bootstrap's copy: logging goes through a caller-supplied
// callback instead of a Diag file, and {"requestStop":true} sets the run's shared stop flag
// instead of a process-wide global — this crate can run more than one session in its lifetime.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::child_process::{parse_log_severity, resolve_command, spawn_piped, STDERR_LOG_TRUNCATE_CHARS};
use crate::runtime::manifest::{order_by_requires, ModuleInfo};
use crate::runtime::runtime_loader::{LogLevel, RunContext, RuntimeLoader};

pub const DESCRIPTOR_NAME: &str = "process.json";

type LogFn = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProcessDescriptor {
    pub command: String,
    pub args: Vec<String>,
    #[serde(rename = "wantsFrames")]
    pub wants_frames: bool,
    #[serde(rename = "timeoutMs", default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    10000
}

impl ProcessDescriptor {
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join(DESCRIPTOR_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

pub struct ProcessModule {
    pub name: String,
    pub wants_frames: bool,
    timeout: Duration,
    stdin: Mutex<ChildStdin>,
    replies: Receiver<Value>,
    child: Mutex<Child>,
    dead: Arc<AtomicBool>,
    log: LogFn,
    compile_items: Value,
}

impl ProcessModule {
    pub fn spawn(folder: &Path, desc: &ProcessDescriptor, name: String, log: LogFn, stop_flag: Arc<AtomicBool>) -> std::io::Result<Self> {
        let exe = resolve_command(folder, &desc.command);
        let mut child = spawn_piped(exe, &desc.args, folder)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let dead = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Value>();

        spawn_stdout_reader(stdout, tx, dead.clone(), log.clone(), name.clone(), stop_flag);
        spawn_stderr_reader(stderr, log.clone(), name.clone());

        Ok(Self {
            name,
            wants_frames: desc.wants_frames,
            timeout: Duration::from_millis(desc.timeout_ms.max(1)),
            stdin: Mutex::new(stdin),
            replies: rx,
            child: Mutex::new(child),
            dead,
            log,
            compile_items: Value::Array(vec![]),
        })
    }

    fn request(&self, message: Value) -> Value {
        if self.dead.load(Ordering::SeqCst) {
            return json!({"ok": false, "error": "module process ended"});
        }
        let mut line = serde_json::to_string(&message).unwrap_or_default();
        line.push('\n');
        {
            let mut stdin = self.stdin.lock().unwrap();
            if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                self.dead.store(true, Ordering::SeqCst);
                return json!({"ok": false, "error": "failed to write to module process"});
            }
        }
        match self.replies.recv_timeout(self.timeout) {
            Ok(reply) => reply,
            Err(_) => {
                self.dead.store(true, Ordering::SeqCst);
                json!({"ok": false, "error": "module did not respond in time"})
            }
        }
    }

    fn ok(reply: &Value) -> bool {
        !matches!(reply.get("ok"), Some(Value::Bool(false)))
    }

    pub fn compile(&mut self, source_code: &str, source_path: &str) -> bool {
        let reply = self.request(json!({"phase": "compile", "sourceCode": source_code, "sourcePath": source_path}));
        if !Self::ok(&reply) {
            (self.log)(LogLevel::Error, &format!(
                "Module \"{}\" failed to compile — skipped. {}",
                self.name, reply.get("error").and_then(|e| e.as_str()).unwrap_or("")
            ));
            return false;
        }
        self.compile_items = reply.get("items").cloned().unwrap_or(Value::Array(vec![]));
        true
    }

    pub fn start(&self, settings: &Value) -> bool {
        let reply = self.request(json!({"phase": "start", "settings": settings, "items": self.compile_items}));
        if !Self::ok(&reply) {
            (self.log)(LogLevel::Error, &format!(
                "[{}] OnStart threw: {}", self.name, reply.get("error").and_then(|e| e.as_str()).unwrap_or("")
            ));
        }
        Self::ok(&reply)
    }

    pub fn frame(&self, delta_seconds: f64) {
        if !self.wants_frames || self.dead.load(Ordering::SeqCst) {
            return;
        }
        let reply = self.request(json!({"phase": "frame", "delta": delta_seconds}));
        if !Self::ok(&reply) {
            (self.log)(LogLevel::Error, &format!(
                "[{}] OnFrame threw: {}", self.name, reply.get("error").and_then(|e| e.as_str()).unwrap_or("")
            ));
        }
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
    name: String,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                log(LogLevel::Warn, &format!("[{name}] unparseable line on stdout: {line}"));
                continue;
            };
            let Some(obj) = value.as_object() else { continue };

            if let Some(l) = obj.get("log").and_then(|l| l.as_object()) {
                let severity = l.get("severity").and_then(|s| s.as_str()).unwrap_or("info");
                let message = l.get("message").and_then(|m| m.as_str()).unwrap_or("");
                log(parse_log_severity(severity), &format!("[{name}] {message}"));
                continue;
            }
            if obj.get("requestStop").and_then(|r| r.as_bool()) == Some(true) {
                stop_flag.store(true, Ordering::SeqCst);
                continue;
            }
            let _ = replies.send(value);
        }
        dead.store(true, Ordering::SeqCst);
    });
}

fn spawn_stderr_reader(stderr: std::process::ChildStderr, log: LogFn, name: String) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let truncated: String = line.chars().take(STDERR_LOG_TRUNCATE_CHARS).collect();
            log(LogLevel::Error, &format!("[{name}] {truncated}"));
        }
    });
}

/// Spawns one ProcessModule per `(info, descriptor)` pair, then drives them through the standard
/// compile→start→frame-loop→stop lifecycle, sorted by each module's own manifest.load_order.
/// Shared by ProcessLoader (real process.json modules) and NativeLoader (native.json modules
/// bridged through the native_module_host helper process — see that file for why: it needs the
/// exact same spawn/start/frame/stop shape a real process module gets, just with a synthetic
/// descriptor pointing at the helper instead of one read from disk). The "compile" phase is a
/// harmless no-op for a native-bridging module — native_module_host answers it immediately with
/// no work to do — so this lifecycle doesn't need to know which kind of module it's driving.
pub fn spawn_and_run(descriptors: Vec<(&ModuleInfo, ProcessDescriptor)>, ctx: &RunContext) -> Result<(), String> {
    let mut spawned: Vec<ProcessModule> = Vec::new();
    for (info, desc) in &descriptors {
        match ProcessModule::spawn(&info.folder, desc, info.manifest.name.clone(), ctx.log.clone(), ctx.stop_flag.clone()) {
            Ok(m) => spawned.push(m),
            Err(e) => (ctx.log)(LogLevel::Error, &format!("Module \"{}\" failed to start its process — skipped. {e}", info.manifest.name)),
        }
    }

    let load_order_rank: std::collections::HashMap<&str, i32> =
        descriptors.iter().map(|(i, _)| (i.manifest.name.as_str(), i.manifest.load_order)).collect();
    spawned.sort_by_key(|m| load_order_rank.get(m.name.as_str()).copied().unwrap_or(i32::MAX));

    if spawned.is_empty() {
        return Err("No runnable modules — nothing to run.".into());
    }

    let mut started: Vec<ProcessModule> = Vec::new();
    for mut m in spawned {
        if m.compile(ctx.source_code, &ctx.source_path.to_string_lossy()) {
            started.push(m);
        }
    }

    started.retain(|m| m.start(&ctx.settings));
    if started.is_empty() {
        return Err("Every module failed to start.".into());
    }

    crate::runtime::driver::run(ctx.target_fps, &ctx.stop_flag, |delta| {
        for m in &started {
            m.frame(delta);
        }
    });

    for m in &started {
        m.stop_and_kill();
    }
    Ok(())
}

/// Every module runs as its own child process, driven by the default paced loop — no CLR
/// touched. Only matches a set where EVERY module has process.json, same reasoning as Bootstrap's
/// ProcessLoader: a run mixing in a native-kind module has to go through NativeLoader instead.
pub struct ProcessLoader;

impl RuntimeLoader for ProcessLoader {
    fn id(&self) -> &'static str {
        "process"
    }

    fn can_handle(&self, modules: &[ModuleInfo]) -> bool {
        !modules.is_empty() && modules.iter().all(|i| i.folder.join(DESCRIPTOR_NAME).exists())
    }

    fn run(&self, modules: Vec<ModuleInfo>, ctx: &RunContext) -> Result<(), String> {
        let load_order = order_by_requires(modules);

        let mut descriptors = Vec::new();
        for info in &load_order {
            match ProcessDescriptor::read(&info.folder) {
                Some(desc) => descriptors.push((info, desc)),
                None => (ctx.log)(LogLevel::Error, &format!("Module folder \"{}\" has no readable process.json — skipped.", info.folder.display())),
            }
        }

        spawn_and_run(descriptors, ctx)
    }
}
