// Port of Bootstrap's process_module.rs: the SAME wire protocol as
// lowarc/Contracts/ProcessModuleLoader.cs (one JSON object per line over stdin/stdout;
// compile/start/frame/stop phases; {"log":...}/{"requestStop":true} notifications), so an
// existing process-kind module works unmodified whether it's run by an export or by this IDE.
//
// One addition to that protocol: a "start" reply may carry `"degraded": "<reason>"` next to its ok.
// See ProcessModule::start for why. Purely additive — a module that never sends it is unaffected,
// which is the same compatibility rule the update manifest follows (see docs/updating.md).
// The only real differences from Bootstrap's copy: logging goes through a caller-supplied
// callback instead of a Diag file, and {"requestStop":true} sets the run's shared stop flag
// instead of a process-wide global: this crate can run more than one session in its lifetime.
//
// Genuinely new versus Bootstrap: inter-module communication. Every "frame" request now carries
// a `"shared"` object, and every "frame" reply MAY carry a `"publish"` object. See
// spawn_and_run's own comment below for the full design. The short version: a module publishes
// under its own id AND under every contract it declares it provides, into a namespace only modules
// that actually `requires` one of those can see, and the existing requires-ordering already
// guarantees a producer's frame runs before a consumer's in the same tick. Native modules get this
// for free too, since native_module_host.rs speaks this exact same JSON wire protocol and routes
// through this same spawn_and_run, translating "shared"/"publish" across the C ABI on its own side
// (see that file's PublishFn).

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
    /// The manifest's display name: for humans reading logs (`[{name}] ...`), never for matching
    /// anything a caller configured. See `id` below for the field breakpoints/traces actually key
    /// on; the two are deliberately kept separate rather than reusing one field for both jobs.
    pub name: String,
    /// The manifest's stable id: the same string a project's project.json "requires" names this
    /// module by, and the only sensible thing for a breakpoint's "module" field or a FrameTrace to
    /// key on. A display name is decorative and not even guaranteed unique; the id is what a user
    /// actually knows and controls.
    pub id: String,
    /// What this module's manifest.json requires, as the keys those requirements resolve to: a
    /// module id for a plain requirement, a contract id for a contract one (see Dependency's
    /// store_id). Either way it is a key in the shared map, which is what lets the filter below
    /// treat both kinds identically.
    ///
    /// The same list already decides load order (see manifest::order_by_requires), doing double
    /// duty as the shared-state visibility rule: this module's frame() only ever sees published
    /// state for keys in this list, never something it never declared depending on. No separate
    /// permission concept to introduce; requiring something already means "I depend on it."
    requires: Vec<String>,
    /// Contracts this module declares it provides. Whatever it publishes is mirrored under each of
    /// these alongside its own id, which is what lets a consumer read a ROLE without knowing which
    /// module happens to be filling it. Empty for the overwhelming majority of modules.
    provides: Vec<String>,
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
        provides: Vec<String>,
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
            provides,
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
    /// log alike, without failing the module. Degrading is the intended behavior, being silent
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

    /// Returns the raw request/reply pair and how long the round-trip took: `spawn_and_run` needs
    /// all three to build this tick's `FrameModuleTrace` and to evaluate ModuleError/JsonMatch
    /// breakpoints against the reply. A module that doesn't want frames, or is already dead,
    /// contributes nothing to the trace rather than a fabricated empty one.
    ///
    /// `all_shared` is the FULL run-wide published-state map (module id -> whatever it last
    /// published); this filters it down to just the entries this module actually `requires`
    /// before it ever reaches the wire, so a module's own request payload only ever contains
    /// state it declared a dependency on. See this file's own header comment for why.
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
                // Evaluated right here rather than back in the frame loop: a module can log at
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

/// Spawns one ProcessModule per `(info, descriptor)` pair and drives them through
/// compile, start, the frame loop and stop, in requires-order. That ordering must come from
/// manifest::requires_rank rather than raw load_order: two modules can declare any loadOrder
/// regardless of what they require, and only a requires-DFS guarantees a producer runs before its
/// consumers, which the shared/publish design depends on for correctness.
///
/// Shared by ProcessLoader and NativeLoader, the latter bridging native modules through the
/// native_module_host helper with a synthetic descriptor. The compile phase is a no-op for a
/// native module, so this lifecycle never needs to know which kind it is driving.
///
/// `shared` is one run-wide map of key to whatever was last published under it, persisting frame to
/// frame. Each module's frame() reads only the keys it `requires` and may publish under its own id
/// and under every contract it provides. Since `started` is in requires-order, a consumer sees its
/// dependency's output from THIS tick rather than one tick stale, with no double-buffering needed.
/// `requires` doing double duty as the read permission means there is no second, parallel concept
/// to keep in sync with it.
pub fn spawn_and_run(descriptors: Vec<(&ModuleInfo, ProcessDescriptor)>, ctx: &RunContext) -> Result<(), String> {
    let mut spawned: Vec<ProcessModule> = Vec::new();
    for (info, desc) in &descriptors {
        match ProcessModule::spawn(
            &info.folder,
            desc,
            info.manifest.name.clone(),
            info.manifest.id.clone(),
            // store_id(), not id: a contract requirement names the contract, and the contract id is
            // exactly the key its gathered array lives under in shared, so the existing filter
            // below needs no special case for contracts at all.
            info.manifest.requires.iter().map(|d| d.store_id().to_string()).collect(),
            info.manifest.provides.iter().map(|p| p.contract.clone()).collect(),
            ctx.log.clone(),
            ctx.stop_flag.clone(),
            ctx.debug.breakpoints.clone(),
            ctx.debug.pause_flag.clone(),
        ) {
            Ok(m) => spawned.push(m),
            Err(e) => (ctx.log)(LogLevel::Error, &format!("Module \"{}\" failed to start its process — skipped. {e}", info.manifest.name)),
        }
    }

    // Requires-order, not just raw loadOrder: the same DFS manifest::order_by_requires uses,
    // exposed here as requires_rank() since this function only ever borrows its ModuleInfos (it
    // doesn't own descriptors, so it can't consume-and-reorder the way order_by_requires does).
    // This is what makes the "a consumer's frame() runs after its dependency's" guarantee spelled
    // out in this function's own header comment actually hold: a plain loadOrder-only sort here
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

    // A ModuleStart breakpoint pauses before the very first frame: setting pause_flag here, right
    // after start and before driver::run's loop ever begins, is all that's needed: the loop checks
    // the flag before its first tick the same as any other, so "paused from frame zero" falls out
    // of the existing gate for free rather than needing a special case.
    for m in &started {
        if runtime_loader::check_module_start_breakpoint(&ctx.debug.breakpoints, &m.id).is_some() {
            ctx.debug.pause_flag.store(true, Ordering::SeqCst);
            (ctx.log)(LogLevel::Info, &format!("Breakpoint hit: module \"{}\" started — pausing before the first frame.", m.name));
        }
    }

    // Run-wide, not per-tick. See spawn_and_run's own comment above for the full design. Lives
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
                mirror_onto_contracts(&mut shared, &m.id, &m.provides);
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
        // breakpoint that just fired (pause_flag only just became true above) alike: one gate,
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

/// Every module runs as its own child process, driven by the default paced loop: no CLR
/// touched. Only matches a set where EVERY module has process.json, same reasoning as Bootstrap's
/// ProcessLoader: a run mixing in a native-kind module has to go through NativeLoader instead.
pub struct ProcessLoader;

/// Mirrors a module's published state onto every contract it provides, so a consumer can read a
/// ROLE without knowing which module fills it. An alias beside the per-module-id entry, not a
/// replacement.
///
/// A contract holds an ARRAY, one entry per provider, tagged with the id it came from. Not a map
/// keyed by provider id, because serde_json's map is sorted rather than insertion-ordered, which
/// would silently make a gathered draw list's z-order alphabetical. Entries append on first publish
/// and update in place after, so the array is in run order, which already guarantees a provider ran
/// before its consumers this tick.
fn mirror_onto_contracts(shared: &mut serde_json::Map<String, Value>, id: &str, provides: &[String]) {
    if provides.is_empty() {
        return;
    }
    let Some(state) = shared.get(id).cloned() else { return };
    for contract in provides {
        let list = shared.entry(contract.clone()).or_insert_with(|| Value::Array(Vec::new()));
        let Some(entries) = list.as_array_mut() else { continue };
        let mut entry = state.clone();
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("from".into(), Value::String(id.to_string()));
        }
        match entries.iter().position(|e| e.get("from").and_then(|f| f.as_str()) == Some(id)) {
            Some(i) => entries[i] = entry,
            None => entries.push(entry),
        }
    }
}

impl RuntimeLoader for ProcessLoader {
    fn id(&self) -> &'static str {
        "process"
    }

    /// Contract modules are excluded from the question entirely rather than counted as modules this
    /// loader can't handle: a contract defines a vocabulary and has no process.json by design, so
    /// counting it here would make one installed contract answer "no loader recognises this
    /// project" for a project that is otherwise perfectly ordinary.
    fn can_handle(&self, modules: &[ModuleInfo]) -> bool {
        let runnable: Vec<&ModuleInfo> = modules.iter().filter(|i| !i.manifest.is_contract()).collect();
        !runnable.is_empty() && runnable.iter().all(|i| i.folder.join(DESCRIPTOR_NAME).exists())
    }

    fn run(&self, modules: Vec<ModuleInfo>, ctx: &RunContext) -> Result<(), String> {
        // Ranked BEFORE contracts are dropped, so a consumer still lands after everything providing
        // what it requires: the contract entries themselves just never become processes.
        let load_order = order_by_requires(modules);

        let mut descriptors = Vec::new();
        for info in &load_order {
            if info.manifest.is_contract() {
                continue; // a definition, not something to run. See Manifest::kind
            }
            match ProcessDescriptor::read(&info.folder) {
                Some(desc) => descriptors.push((info, desc)),
                None => (ctx.log)(LogLevel::Error, &format!("Module folder \"{}\" has no readable process.json — skipped.", info.folder.display())),
            }
        }

        spawn_and_run(descriptors, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(shared: &mut serde_json::Map<String, Value>, id: &str, provides: &[&str], state: Value) {
        let entry = shared.entry(id.to_string()).or_insert_with(|| Value::Object(Default::default()));
        entry.as_object_mut().unwrap().extend(state.as_object().unwrap().clone());
        let provides: Vec<String> = provides.iter().map(|s| s.to_string()).collect();
        mirror_onto_contracts(shared, id, &provides);
    }

    #[test]
    fn a_contract_gathers_every_provider_in_the_order_they_published() {
        let mut shared = serde_json::Map::new();
        publish(&mut shared, "world", &["draw-commands"], json!({"draw": ["floor"]}));
        publish(&mut shared, "overlay", &["draw-commands"], json!({"draw": ["fps"]}));

        let gathered = shared["draw-commands"].as_array().expect("a contract key is an array");
        let order: Vec<&str> = gathered.iter().map(|e| e["from"].as_str().unwrap()).collect();
        // Order is the whole point: draw order is list order, so publish order is z-order. A map
        // keyed by provider id would have sorted these alphabetically and put the overlay under
        // the floor.
        assert_eq!(order, vec!["world", "overlay"]);
        assert_eq!(gathered[0]["draw"], json!(["floor"]));
    }

    #[test]
    fn a_provider_publishing_again_updates_its_own_entry_rather_than_appending_a_second() {
        let mut shared = serde_json::Map::new();
        publish(&mut shared, "world", &["draw-commands"], json!({"draw": ["frame one"]}));
        publish(&mut shared, "world", &["draw-commands"], json!({"draw": ["frame two"]}));

        let gathered = shared["draw-commands"].as_array().unwrap();
        assert_eq!(gathered.len(), 1, "a provider has one entry, however many frames it publishes");
        assert_eq!(gathered[0]["draw"], json!(["frame two"]));
    }

    #[test]
    fn publishing_under_a_contract_never_replaces_the_per_module_entry() {
        // Both addressing modes have to keep working: anything depending on one specific module
        // still reads it by id, exactly as it did before contracts existed.
        let mut shared = serde_json::Map::new();
        publish(&mut shared, "vector-canvas", &["input-state"], json!({"width": 800}));

        assert_eq!(shared["vector-canvas"]["width"], json!(800), "the module's own id must still address it");
        assert_eq!(shared["input-state"][0]["width"], json!(800), "and the contract aliases the same state");
    }

    #[test]
    fn one_module_can_answer_several_contracts_at_once() {
        let mut shared = serde_json::Map::new();
        publish(&mut shared, "everything", &["draw-commands", "audio-cues"], json!({"draw": [], "play": []}));

        assert_eq!(shared["draw-commands"][0]["from"], json!("everything"));
        assert_eq!(shared["audio-cues"][0]["from"], json!("everything"));
    }

    #[test]
    fn a_module_providing_nothing_adds_no_contract_keys() {
        let mut shared = serde_json::Map::new();
        publish(&mut shared, "quiet", &[], json!({"anything": 1}));
        assert_eq!(shared.len(), 1, "only its own id, no stray keys: {shared:?}");
    }
}
