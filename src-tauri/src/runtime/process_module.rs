// Port of Bootstrap's process_module.rs — the SAME wire protocol as
// lowarc/Contracts/ProcessModuleLoader.cs (one JSON object per line over stdin/stdout;
// compile/start/frame/stop phases; {"log":...}/{"requestStop":true} notifications), so an
// existing process-kind module works unmodified whether it's run by an export or by this IDE.
//
// One addition to that protocol: a "start" reply may carry `"degraded": "<reason>"` next to its ok.
// See ProcessModule::start for why. Purely additive — a module that never sends it is unaffected,
// which is the same compatibility rule the update manifest follows (see docs/updating.md).
// The only real differences from Bootstrap's copy: logging goes through a caller-supplied
// callback instead of a Diag file, and {"requestStop":true} sets the run's shared stop flag
// instead of a process-wide global — this crate can run more than one session in its lifetime.
//
// Genuinely new versus Bootstrap: inter-module communication. Every "frame" request now carries
// a `"shared"` object, and every "frame" reply MAY carry a `"publish"` object — see
// spawn_and_run's own comment below for the full design (the short version: a module publishes
// under its own id, into a namespace only modules that actually `requires` it can see, and the
// existing requires-ordering already guarantees a producer's frame runs before a consumer's in
// the same tick). Native modules get this for free too — native_module_host.rs speaks this exact
// same JSON wire protocol, translating "shared"/"publish" across the C ABI on its own side (see
// that file's PublishFn).

use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::runtime::child_process::{parse_log_severity, resolve_command, spawn_piped, STDERR_LOG_TRUNCATE_CHARS};
use crate::runtime::manifest::{order_by_requires, ModuleInfo};
use crate::runtime::runtime_loader::{self, Breakpoint, FrameModuleTrace, FrameTrace, LogFn, LogLevel, RunContext, RuntimeLoader};

pub const DESCRIPTOR_NAME: &str = "process.json";

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
    /// The manifest's display name — for humans reading logs (`[{name}] ...`), never for matching
    /// anything a caller configured. See `id` below for the field breakpoints/traces actually key
    /// on; the two are deliberately kept separate rather than reusing one field for both jobs.
    pub name: String,
    /// The manifest's stable id — the same string a project's project.json "requires" names this
    /// module by, and the only sensible thing for a breakpoint's "module" field or a FrameTrace to
    /// key on. A display name is decorative and not even guaranteed unique; the id is what a user
    /// actually knows and controls.
    pub id: String,
    /// This module's own manifest.json requires, as plain ids — the same list that already
    /// decides load order (see manifest::order_by_requires) doing double duty as the shared-state
    /// visibility rule: this module's frame() only ever sees OTHER modules' published state for
    /// ids in this list, never a module it never declared depending on. No separate permission
    /// concept to introduce; requiring something already means "I depend on it existing."
    requires: Vec<String>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        folder: &Path,
        desc: &ProcessDescriptor,
        name: String,
        id: String,
        requires: Vec<String>,
        log: LogFn,
        stop_flag: Arc<AtomicBool>,
        breakpoints: Arc<Mutex<Vec<Breakpoint>>>,
        pause_flag: Arc<AtomicBool>,
    ) -> std::io::Result<Self> {
        let exe = resolve_command(folder, &desc.command);
        let mut child = spawn_piped(exe, &desc.args, folder)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let dead = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Value>();

        spawn_stdout_reader(stdout, tx, dead.clone(), log.clone(), name.clone(), id.clone(), stop_flag, breakpoints, pause_flag);
        spawn_stderr_reader(stderr, log.clone(), name.clone());

        Ok(Self {
            name,
            id,
            requires,
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
            let mut stdin = self.stdin.lock();
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

    /// A "start" reply may carry `"degraded": "<reason>"` alongside its ok — "I started, and I will
    /// keep answering, but something I needed isn't here." That is a genuinely different state from
    /// both success and failure, and it had no way to be expressed: a module with no audio device or
    /// no display would run to completion doing nothing, indistinguishable from one that simply had
    /// nothing to do. Reported at Warn so it reaches the dev-run console and an export's diagnostics
    /// log alike, without failing the module — degrading is the intended behavior, being silent
    /// about it was not.
    pub fn start(&self, settings: &Value) -> bool {
        let reply = self.request(json!({"phase": "start", "settings": settings, "items": self.compile_items}));
        if !Self::ok(&reply) {
            (self.log)(LogLevel::Error, &format!(
                "[{}] OnStart threw: {}", self.name, reply.get("error").and_then(|e| e.as_str()).unwrap_or("")
            ));
            return false;
        }
        if let Some(reason) = reply.get("degraded").and_then(|d| d.as_str()).filter(|r| !r.trim().is_empty()) {
            (self.log)(LogLevel::Warn, &format!("[{}] started degraded — {reason}", self.name));
        }
        true
    }

    /// Returns the raw request/reply pair and how long the round-trip took — `spawn_and_run` needs
    /// all three to build this tick's `FrameModuleTrace` and to evaluate ModuleError/JsonMatch
    /// breakpoints against the reply. A module that doesn't want frames, or is already dead,
    /// contributes nothing to the trace rather than a fabricated empty one.
    ///
    /// `all_shared` is the FULL run-wide published-state map (module id -> whatever it last
    /// published); this filters it down to just the entries this module actually `requires`
    /// before it ever reaches the wire, so a module's own request payload only ever contains
    /// state it declared a dependency on — see this file's own header comment for why.
    pub fn frame(&self, delta_seconds: f64, all_shared: &serde_json::Map<String, Value>) -> Option<(Value, Value, f64)> {
        if !self.wants_frames || self.dead.load(Ordering::SeqCst) {
            return None;
        }
        let shared: serde_json::Map<String, Value> =
            self.requires.iter().filter_map(|dep_id| all_shared.get(dep_id).map(|v| (dep_id.clone(), v.clone()))).collect();
        let request = json!({"phase": "frame", "delta": delta_seconds, "shared": shared});
        let started = Instant::now();
        let reply = self.request(request.clone());
        let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        if !Self::ok(&reply) {
            (self.log)(LogLevel::Error, &format!(
                "[{}] OnFrame threw: {}", self.name, reply.get("error").and_then(|e| e.as_str()).unwrap_or("")
            ));
        }
        Some((request, reply, duration_ms))
    }

    pub fn stop_and_kill(&self) {
        if !self.dead.load(Ordering::SeqCst) {
            let _ = self.request(json!({"phase": "stop"}));
        }
        self.kill();
    }

    fn kill(&self) {
        self.dead.store(true, Ordering::SeqCst);
        let mut child = self.child.lock();
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

#[allow(clippy::too_many_arguments)]
fn spawn_stdout_reader(
    stdout: std::process::ChildStdout,
    replies: mpsc::Sender<Value>,
    dead: Arc<AtomicBool>,
    log: LogFn,
    name: String,
    id: String,
    stop_flag: Arc<AtomicBool>,
    breakpoints: Arc<Mutex<Vec<Breakpoint>>>,
    pause_flag: Arc<AtomicBool>,
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
                let level = parse_log_severity(severity);
                let message = l.get("message").and_then(|m| m.as_str()).unwrap_or("");
                log(level, &format!("[{name}] {message}"));
                // Evaluated right here rather than back in the frame loop — a module can log at
                // any time, not just from inside a frame reply, so this is the one place a
                // LogLevel breakpoint's triggering data actually exists. No matching FrameTrace
                // accompanies this kind of pause (there's no frame to attach it to); the log line
                // just above already says what happened.
                if runtime_loader::check_log_level_breakpoint(&breakpoints, level, &id).is_some() {
                    pause_flag.store(true, Ordering::SeqCst);
                    log(LogLevel::Info, &format!("Breakpoint hit: [{name}] logged at {level:?} severity — pausing."));
                }
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
/// compile→start→frame-loop→stop lifecycle, sorted into requires-order (see manifest::
/// requires_rank — NOT just each module's own raw manifest.load_order; two modules can declare any
/// loadOrder numbers regardless of what they actually require, so only a real requires-DFS
/// guarantees a producer runs before its consumers, which the shared/publish design below depends
/// on for correctness, not just tidiness).
///
/// Shared by ProcessLoader (real process.json modules) and NativeLoader (native.json modules
/// bridged through the native_module_host helper process — see that file for why: it needs the
/// exact same spawn/start/frame/stop shape a real process module gets, just with a synthetic
/// descriptor pointing at the helper instead of one read from disk). The "compile" phase is a
/// harmless no-op for a native-bridging module — native_module_host answers it immediately with
/// no work to do — so this lifecycle doesn't need to know which kind of module it's driving.
///
/// Inter-module communication, in full: `shared` below is one run-wide map, module id -> whatever
/// that module last published (persists frame to frame; a key nobody's touched yet this tick just
/// keeps its previous value). Every module's frame() call reads its own filtered view of it (only
/// the ids it `requires` — see ProcessModule::frame) and may return new entries to publish under
/// its OWN id, merged into `shared` the moment that module's frame() call returns. Because `started`
/// is sorted by requires_rank (see this function's own header above — a module always runs after
/// everything it requires), a consumer's frame() this same tick sees its dependency's output from
/// THIS tick, not one tick stale — no separate synchronization or double-buffering needed, it falls
/// out of the existing ordering for free. No new manifest field either: requires already means "I
/// depend on this existing", so it doing double duty as the communication permission is the least
/// arbitrary reading of it, not a second, parallel concept to keep in sync with the first.
pub fn spawn_and_run(descriptors: Vec<(&ModuleInfo, ProcessDescriptor)>, ctx: &RunContext) -> Result<(), String> {
    let mut spawned: Vec<ProcessModule> = Vec::new();
    for (info, desc) in &descriptors {
        match ProcessModule::spawn(
            &info.folder,
            desc,
            info.manifest.name.clone(),
            info.manifest.id.clone(),
            info.manifest.requires.iter().map(|d| d.id.clone()).collect(),
            ctx.log.clone(),
            ctx.stop_flag.clone(),
            ctx.debug.breakpoints.clone(),
            ctx.debug.pause_flag.clone(),
        ) {
            Ok(m) => spawned.push(m),
            Err(e) => (ctx.log)(LogLevel::Error, &format!("Module \"{}\" failed to start its process — skipped. {e}", info.manifest.name)),
        }
    }

    // Requires-order, not just raw loadOrder — the same DFS manifest::order_by_requires uses,
    // exposed here as requires_rank() since this function only ever borrows its ModuleInfos (it
    // doesn't own descriptors, so it can't consume-and-reorder the way order_by_requires does).
    // This is what makes the "a consumer's frame() runs after its dependency's" guarantee spelled
    // out in this function's own header comment actually hold — a plain loadOrder-only sort here
    // would NOT have guaranteed it (two modules can declare any loadOrder numbers regardless of
    // what they actually require), so run_from_launch_dir (which already called order_by_requires
    // itself) and start_run (which resolves modules via project::resolve — plain BFS, not
    // requires-ordered at all) would have disagreed on something this basic.
    let module_infos: Vec<&ModuleInfo> = descriptors.iter().map(|(i, _)| *i).collect();
    let requires_rank = crate::runtime::manifest::requires_rank(&module_infos);
    spawned.sort_by_key(|m| requires_rank.get(&m.id).copied().unwrap_or(usize::MAX));

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

    // A ModuleStart breakpoint pauses before the very first frame — setting pause_flag here, right
    // after start and before driver::run's loop ever begins, is all that's needed: the loop checks
    // the flag before its first tick the same as any other, so "paused from frame zero" falls out
    // of the existing gate for free rather than needing a special case.
    for m in &started {
        if runtime_loader::check_module_start_breakpoint(&ctx.debug.breakpoints, &m.id).is_some() {
            ctx.debug.pause_flag.store(true, Ordering::SeqCst);
            (ctx.log)(LogLevel::Info, &format!("Breakpoint hit: module \"{}\" started — pausing before the first frame.", m.name));
        }
    }

    // Run-wide, not per-tick — see spawn_and_run's own comment above for the full design. Lives
    // right here (not behind an Arc<Mutex<_>>) since driver::run's tick closure is the only thing
    // that ever touches it, on this one thread, never concurrently with anything else.
    let mut shared: serde_json::Map<String, Value> = serde_json::Map::new();

    let mut frame_index: u64 = 0;
    crate::runtime::driver::run(ctx.target_fps, &ctx.stop_flag, &ctx.debug.pause_flag, &ctx.debug.step_request, |delta| {
        frame_index += 1;
        let mut modules_trace = Vec::with_capacity(started.len());
        let mut triggered: Option<Breakpoint> = None;

        for m in &started {
            let Some((request, reply, duration_ms)) = m.frame(delta, &shared) else { continue };
            if let Some(publish) = reply.get("publish").and_then(|p| p.as_object()) {
                shared.entry(m.id.clone()).or_insert_with(|| Value::Object(Default::default())).as_object_mut().unwrap().extend(publish.clone());
            }
            if triggered.is_none() {
                triggered = runtime_loader::check_frame_breakpoints(&ctx.debug.breakpoints, &m.id, &reply);
            }
            modules_trace.push(FrameModuleTrace { id: m.id.clone(), request, reply, duration_ms });
        }

        if triggered.is_none() {
            triggered = runtime_loader::check_frame_count_breakpoint(&ctx.debug.breakpoints, frame_index);
        }
        if let Some(bp) = &triggered {
            ctx.debug.pause_flag.store(true, Ordering::SeqCst);
            (ctx.log)(LogLevel::Info, &format!("Breakpoint hit: {bp:?} — pausing."));
        }

        // Fires for a manual step (pause_flag was already true going into this tick) and for a
        // breakpoint that just fired (pause_flag only just became true above) alike — one gate,
        // not two separate mechanisms for what's the same "show the user this tick" need. Stays
        // silent for every tick of a normal free-running loop, where pause_flag is false
        // throughout.
        if ctx.debug.pause_flag.load(Ordering::SeqCst) {
            (ctx.debug.on_frame)(FrameTrace { frame_index, delta_seconds: delta, modules: modules_trace, triggered });
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
