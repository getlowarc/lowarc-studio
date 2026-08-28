mod app_paths;
pub mod dylib;
pub mod export;
mod installs;
pub mod plugin_host;
mod plugin_asset_server;
mod plugin_assets;
mod plugin_session;
mod projects;
pub mod runtime;
mod settings;
mod theme;

use app_paths::AppPaths;
use installs::{ModuleListItem, PluginListItem};
use projects::RecentProject;
use runtime::runtime_loader::LogLevel;
use settings::Settings;
use theme::ThemePreset;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// The debug-control handles for the one active dev-run, if any. A second start_dev_run while one
/// is already running is refused rather than silently replacing it — mirrors "throw a visible
/// error, let the user decide" rather than guessing what they meant.
struct ActiveRun {
    stop_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    step_request: Arc<AtomicU32>,
}

#[derive(Default)]
struct RunState(Mutex<Option<ActiveRun>>);

/// Breakpoints deliberately live independently of any one run, not inside RunState/ActiveRun —
/// configuring them before Start is pressed has to actually take effect from frame zero (a
/// ModuleStart breakpoint is meaningless if it can only be set after the module already started),
/// and a real debugger's breakpoints persisting across separate runs (like VS Code's do) is the
/// expected behavior, not an accident. `set_breakpoints` works with or without an active run;
/// `start_dev_run` just clones this same Arc into the new run's DebugHooks rather than starting
/// from an empty list every time.
#[derive(Default, Clone)]
struct BreakpointState(Arc<Mutex<Vec<runtime::runtime_loader::Breakpoint>>>);

/// Whether an export is currently running — no stop flag, unlike RunState: nothing about a
/// cargo-build-then-copy-files export is safely cancellable mid-step, so this only guards against
/// a second export starting while one's already in flight.
#[derive(Default)]
struct ExportState(Mutex<bool>);

fn log_level_str(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

/// The log callback every plugin_host::protocol::invoke call gets — plugins are invoked fresh per
/// call now (no persistent process to wire this up once for at launch), so this is built fresh
/// per call too rather than stored anywhere.
fn plugin_log_fn(app: &AppHandle) -> plugin_host::protocol::LogFn {
    let handle = app.clone();
    Arc::new(move |level, message| {
        let _ = handle.emit("plugin-log", serde_json::json!({"level": log_level_str(level), "message": message}));
    })
}

#[tauri::command]
fn start_dev_run(
    app: AppHandle,
    state: State<'_, RunState>,
    breakpoint_state: State<'_, BreakpointState>,
    entry_file: String,
    project_dir: String,
) -> Result<(), String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let step_request = Arc::new(AtomicU32::new(0));

    {
        let mut guard = state.0.lock().unwrap();
        if guard.is_some() {
            return Err("A run is already active — stop it before starting another.".into());
        }
        *guard = Some(ActiveRun { stop_flag: stop_flag.clone(), pause_flag: pause_flag.clone(), step_request: step_request.clone() });
    }

    // entry_file is stored (and passed in here) relative to the project root — see
    // ProjectPreset::entry's doc comment — so it has to be joined before it's an actually
    // readable path, rather than assumed to already be one.
    let project = PathBuf::from(project_dir);
    let entry = project.join(&entry_file);
    let modules = AppPaths::modules();

    let log_handle = app.clone();
    let log: Arc<dyn Fn(LogLevel, &str) + Send + Sync> = Arc::new(move |level, message| {
        let _ = log_handle.emit("dev-run-log", serde_json::json!({"level": log_level_str(level), "message": message}));
    });

    let frame_handle = app.clone();
    let on_frame: Arc<dyn Fn(runtime::runtime_loader::FrameTrace) + Send + Sync> = Arc::new(move |trace| {
        let _ = frame_handle.emit("dev-run-frame", trace);
    });

    let target_fps = settings::load().dev_run_target_fps;
    let debug = runtime::runtime_loader::DebugHooks { pause_flag, step_request, breakpoints: breakpoint_state.0.clone(), on_frame };

    let done_handle = app.clone();
    std::thread::spawn(move || {
        // Per-module settings (the 4th arg) aren't sourced from anywhere real yet — that's config
        // handed to game modules at start, a separate concept from Studio's own settings.json.
        let result = runtime::start_run(&entry, &project, &modules, target_fps, serde_json::json!({}), stop_flag, log, debug);

        *done_handle.state::<RunState>().0.lock().unwrap() = None;
        let payload = match &result {
            Ok(()) => serde_json::json!({"ok": true}),
            Err(errors) => serde_json::json!({"ok": false, "errors": errors}),
        };
        let _ = done_handle.emit("dev-run-ended", payload);
    });

    Ok(())
}

/// Pausing also zeroes any in-flight step request — otherwise a step queued right before a manual
/// pause (unlikely from the UI, since Step is normally only enabled while already paused, but not
/// impossible to race) would let one more tick slip through right after this call returns.
#[tauri::command]
fn pause_dev_run(state: State<'_, RunState>) -> Result<(), String> {
    match state.0.lock().unwrap().as_ref() {
        Some(active) => {
            active.step_request.store(0, Ordering::SeqCst);
            active.pause_flag.store(true, Ordering::SeqCst);
            Ok(())
        }
        None => Err("No run is active.".into()),
    }
}

#[tauri::command]
fn resume_dev_run(state: State<'_, RunState>) -> Result<(), String> {
    match state.0.lock().unwrap().as_ref() {
        Some(active) => {
            active.step_request.store(0, Ordering::SeqCst);
            active.pause_flag.store(false, Ordering::SeqCst);
            Ok(())
        }
        None => Err("No run is active.".into()),
    }
}

/// Only valid while already paused — stepping a freely-running loop has no well-defined meaning
/// (step to where, exactly, if it's already advancing on its own?), so this refuses rather than
/// silently pausing-then-stepping on the caller's behalf.
#[tauri::command]
fn step_dev_run(state: State<'_, RunState>, count: Option<u32>) -> Result<(), String> {
    match state.0.lock().unwrap().as_ref() {
        Some(active) => {
            if !active.pause_flag.load(Ordering::SeqCst) {
                return Err("Pause the run before stepping.".into());
            }
            active.step_request.fetch_add(count.unwrap_or(1).max(1), Ordering::SeqCst);
            Ok(())
        }
        None => Err("No run is active.".into()),
    }
}

/// Works with or without an active run — see BreakpointState's own doc comment for why. Replaces
/// the whole set rather than adding/removing one at a time, same "frontend always resends
/// everything" convention plugin settings/commands already use.
#[tauri::command]
fn set_breakpoints(state: State<'_, BreakpointState>, breakpoints: Vec<runtime::runtime_loader::Breakpoint>) {
    *state.0.lock().unwrap() = breakpoints;
}

#[tauri::command]
fn get_project_preset(project_dir: String) -> Result<runtime::project::ProjectPreset, String> {
    runtime::project::ProjectPreset::load(&PathBuf::from(project_dir))
}

/// Sets a project's entry file from an absolute path (as returned by the native file-picker
/// dialog) and returns the project-relative path that actually got saved. The entry must live
/// inside the project directory — canonicalize (not a plain strip_prefix) so a case or `..`
/// difference between the two paths doesn't produce a false "outside the project" rejection.
#[tauri::command]
fn set_project_entry(project_dir: String, absolute_entry_path: String) -> Result<String, String> {
    let project_dir = PathBuf::from(project_dir);
    let canonical_project = project_dir.canonicalize().map_err(|e| format!("Invalid project directory: {e}"))?;
    let canonical_entry = PathBuf::from(&absolute_entry_path)
        .canonicalize()
        .map_err(|e| format!("Invalid entry file: {e}"))?;

    let relative = canonical_entry
        .strip_prefix(&canonical_project)
        .map_err(|_| "The entry file must be inside the project folder.".to_string())?;
    let relative = relative.to_string_lossy().into_owned();

    let mut preset = runtime::project::ProjectPreset::load(&project_dir).unwrap_or_default();
    preset.entry = relative.clone();
    preset.save(&project_dir)?;
    Ok(relative)
}

#[tauri::command]
fn entry_file_exists(project_dir: String, entry: String) -> bool {
    PathBuf::from(project_dir).join(entry).is_file()
}

/// A sanity backstop, not a full capability boundary — these two commands are reachable by any
/// code that can invoke a Tauri command, and this app's editor legitimately opens/saves files
/// anywhere the user has picked via a native dialog, so there's no "must be inside X" allowlist to
/// enforce without breaking real usage. "Is this specific plugin allowed to touch this specific
/// file" still lives where it has to — editor.html's own windowToFilePath map, checked before a
/// plugin's saveFile request ever reaches invoke() at all — since only the host knows which iframe
/// asked and what path it was actually opened for; Rust has no visibility into that session state
/// and isn't trying to fake it. What THIS catches: an empty path, a relative one (every real
/// caller already has an absolute one — a file explorer entry or a native dialog result, never a
/// bare relative string), and a parent directory that doesn't genuinely resolve to what it claims
/// (canonicalizing it collapses any `..`/symlink misdirection in the directory portion, then the
/// file name — which doesn't need to exist yet, for a save of new content — is rejoined as-is).
fn validate_file_path(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("No file path given.".into());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("\"{}\" isn't an absolute path.", path.display()));
    }
    let parent = path.parent().ok_or_else(|| format!("\"{}\" has no parent directory.", path.display()))?;
    let canonical_parent = parent.canonicalize().map_err(|e| format!("\"{}\": {e}", parent.display()))?;
    let file_name = path.file_name().ok_or_else(|| format!("\"{}\" has no file name.", path.display()))?;
    Ok(canonical_parent.join(file_name))
}

/// Reads an open file's contents so the host can push them into whichever viewer plugin claimed
/// that extension — a viewer's own sandboxed iframe has no filesystem access of its own, on
/// purpose, so this has to happen host-side rather than the plugin reading the file itself.
#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    let path = validate_file_path(&path)?;
    std::fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))
}

/// The other half of read_text_file — invoked host-side in response to a viewer plugin's
/// `window.lowarc.saveFile(path, contents)`, same reasoning as read: a sandboxed viewer iframe
/// has no filesystem access of its own.
#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    let path = validate_file_path(&path)?;
    std::fs::write(&path, contents).map_err(|e| format!("Could not save {}: {e}", path.display()))
}

/// read_text_file's counterpart for a viewer whose plugin.json marks its `viewers` entry
/// `"binary": true` (images, video — anything read_to_string would corrupt by forcing a UTF-8
/// decode on bytes that were never text). Base64, not a raw byte array — directly usable as a
/// data: URI on the plugin side (`data:${mime};base64,${content}`) with no further decoding, and
/// far more compact over postMessage/JSON than Tauri's default array-of-numbers serialization for
/// Vec<u8> would be.
#[tauri::command]
fn read_binary_file(path: String) -> Result<String, String> {
    let path = validate_file_path(&path)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    Ok(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes))
}

#[cfg(test)]
mod file_path_tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_file_path_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_an_absolute_path_whose_parent_exists() {
        let dir = temp_dir("ok");
        let path = dir.join("file.txt");
        let resolved = validate_file_path(&path.to_string_lossy()).expect("a real absolute path should be accepted");
        assert_eq!(resolved.file_name().unwrap(), "file.txt");
    }

    #[test]
    fn rejects_an_empty_path() {
        assert!(validate_file_path("").is_err());
        assert!(validate_file_path("   ").is_err());
    }

    #[test]
    fn rejects_a_relative_path() {
        assert!(validate_file_path("src/main.rs").is_err());
    }

    #[test]
    fn rejects_a_path_whose_parent_does_not_exist() {
        let dir = temp_dir("missing_parent");
        let path = dir.join("does-not-exist-at-all").join("file.txt");
        assert!(validate_file_path(&path.to_string_lossy()).is_err());
    }
}

/// Lets editor.html (the host page, not a plugin's own iframe) read one of a plugin's static
/// assets — e.g. a rail icon — as text, over IPC rather than a network request of any kind.
/// Not strictly required now that plugin assets are served over real HTTP (fetch() would work
/// too), but kept as-is: it already works, and rail icons need the raw SVG text to sanitize and
/// inline, not a URL to fetch.
#[tauri::command]
fn read_plugin_asset(plugin_id: String, rel_path: String) -> Result<String, String> {
    let resolved = plugin_assets::resolve_asset_path(&plugin_id, &rel_path)
        .ok_or_else(|| format!("No such plugin asset: \"{plugin_id}/{rel_path}\"."))?;
    std::fs::read_to_string(&resolved).map_err(|e| format!("Could not read {}: {e}", resolved.display()))
}

/// The port editor.html should use to build plugin panel/viewer iframe URLs as
/// `http://127.0.0.1:<port>/<plugin_id>/<rel_path>` — see plugin_asset_server.rs.
#[tauri::command]
fn plugin_asset_port() -> u16 {
    plugin_asset_server::port()
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn set_project_entry_saves_relative_path_and_returns_it() {
        let project_dir = temp_dir("entry_ok");
        std::fs::write(project_dir.join("project.json"), "{\"requires\":[]}\n").unwrap();
        std::fs::create_dir_all(project_dir.join("src")).unwrap();
        let entry_path = project_dir.join("src").join("main.uc");
        std::fs::write(&entry_path, "// entry").unwrap();

        let relative = set_project_entry(project_dir.to_string_lossy().into_owned(), entry_path.to_string_lossy().into_owned())
            .expect("an entry file inside the project should be accepted");
        assert_eq!(relative, PathBuf::from("src").join("main.uc").to_string_lossy());

        let preset = runtime::project::ProjectPreset::load(&project_dir).unwrap();
        assert_eq!(preset.entry, relative, "the saved project.json should carry the same relative path back out");
    }

    #[test]
    fn set_project_entry_rejects_a_file_outside_the_project() {
        let project_dir = temp_dir("entry_outside_project");
        std::fs::write(project_dir.join("project.json"), "{\"requires\":[]}\n").unwrap();
        let outside_dir = temp_dir("entry_outside_target");
        let outside_file = outside_dir.join("elsewhere.uc");
        std::fs::write(&outside_file, "// not in the project").unwrap();

        let err = set_project_entry(project_dir.to_string_lossy().into_owned(), outside_file.to_string_lossy().into_owned())
            .expect_err("a file outside the project directory must be rejected");
        assert!(err.contains("must be inside"), "error should explain why: {err}");
    }

    #[test]
    fn entry_file_exists_reflects_disk_state() {
        let project_dir = temp_dir("entry_exists");
        let entry_path = project_dir.join("main.uc");
        std::fs::write(&entry_path, "// entry").unwrap();

        assert!(entry_file_exists(project_dir.to_string_lossy().into_owned(), "main.uc".to_string()));

        std::fs::remove_file(&entry_path).unwrap();
        assert!(!entry_file_exists(project_dir.to_string_lossy().into_owned(), "main.uc".to_string()), "a deleted entry should report as missing, not stale-true");
    }
}

#[tauri::command]
fn list_recent_projects() -> Vec<RecentProject> {
    projects::list_recent()
}

#[tauri::command]
fn create_project(parent_dir: String, name: String) -> Result<String, String> {
    projects::create_project(&PathBuf::from(parent_dir), &name).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_project(path: String) -> Result<(), String> {
    projects::open_project(&PathBuf::from(path))
}

#[tauri::command]
fn set_recent_pinned(path: String, pinned: bool) -> Result<(), String> {
    projects::set_recent_pinned(&PathBuf::from(path), pinned)
}

#[tauri::command]
fn remove_recent_project(path: String) -> Result<(), String> {
    projects::remove_recent(&PathBuf::from(path))
}

#[tauri::command]
fn get_settings() -> Settings {
    settings::load()
}

#[tauri::command]
fn save_settings(settings: Settings) -> Result<(), String> {
    settings::save(&settings)
}

#[tauri::command]
fn list_theme_presets() -> Vec<ThemePreset> {
    theme::list_presets()
}

#[tauri::command]
fn save_theme_preset(preset: ThemePreset) -> Result<(), String> {
    theme::save_preset(&preset)
}

#[tauri::command]
fn delete_theme_preset(name: String) -> Result<(), String> {
    theme::delete_preset(&name)
}

#[tauri::command]
fn stop_dev_run(state: State<'_, RunState>) -> Result<(), String> {
    match state.0.lock().unwrap().as_ref() {
        Some(active) => {
            active.stop_flag.store(true, Ordering::SeqCst);
            Ok(())
        }
        None => Err("No run is active.".into()),
    }
}

/// Folder-mode export only for now — see export::export_folder's own header for why a folder is a
/// real, complete output shape rather than a stopgap. Runs on its own thread (the runtime build
/// step alone can take real time) and reports progress the same way dev-run does: `export-log`
/// events while it runs, one `export-ended` event when it's done either way.
#[tauri::command]
fn start_export(app: AppHandle, state: State<'_, ExportState>, project_dir: String, output_dir: String, name: String, diagnostics_log: bool) -> Result<(), String> {
    {
        let mut guard = state.0.lock().unwrap();
        if *guard {
            return Err("An export is already in progress — wait for it to finish before starting another.".into());
        }
        *guard = true;
    }

    // Each of build_bootstrap's and export_folder's own log(...) calls is one fixed, known stage
    // announced in a fixed order — 1 from build_bootstrap, 5 from export_folder today — so a plain
    // running count against that fixed total is a real (if coarse) progress fraction, not a guess.
    // Keep EXPORT_TOTAL_STEPS in sync if either function's own count of log(...) calls changes.
    const EXPORT_TOTAL_STEPS: u32 = 6;
    let step = Arc::new(AtomicU32::new(0));
    let log_handle = app.clone();
    let log: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |message: &str| {
        let current = step.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = log_handle.emit("export-log", serde_json::json!({"message": message, "step": current, "total": EXPORT_TOTAL_STEPS}));
    });

    let options = export::ExportOptions {
        project_dir: PathBuf::from(project_dir),
        modules_dir: AppPaths::modules(),
        output_dir: PathBuf::from(output_dir),
        name,
        diagnostics_log,
        target_fps: settings::load().dev_run_target_fps,
    };

    let done_handle = app.clone();
    std::thread::spawn(move || {
        let result = export::bootstrap_source::build_bootstrap(&*log)
            .map_err(|e| vec![e])
            .and_then(|bootstrap_exe| export::export_folder(&options, &bootstrap_exe, &*log));

        *done_handle.state::<ExportState>().0.lock().unwrap() = false;
        let payload = match &result {
            Ok(path) => serde_json::json!({"ok": true, "path": path.to_string_lossy()}),
            Err(errors) => serde_json::json!({"ok": false, "errors": errors}),
        };
        let _ = done_handle.emit("export-ended", payload);
    });

    Ok(())
}

#[tauri::command]
fn list_installed_modules() -> Vec<ModuleListItem> {
    let disabled: HashSet<String> = settings::load().disabled_modules.into_iter().collect();
    installs::list_modules(&AppPaths::modules(), &disabled)
}

#[tauri::command]
fn install_module(app: AppHandle, source_dir: String) -> Result<String, String> {
    installs::install_module(&AppPaths::modules(), &PathBuf::from(source_dir), &mut |done, total| {
        let _ = app.emit("install-progress", serde_json::json!({"kind": "module", "bytesDone": done, "totalBytes": total}));
    })
}

#[tauri::command]
fn remove_module(id: String) -> Result<(), String> {
    installs::remove_module(&AppPaths::modules(), &id)
}

#[tauri::command]
fn set_module_enabled(id: String, enabled: bool) -> Result<(), String> {
    let mut settings = settings::load();
    settings.disabled_modules.retain(|m| m != &id);
    if !enabled {
        settings.disabled_modules.push(id);
    }
    settings::save(&settings)
}

#[tauri::command]
fn list_installed_plugins() -> Vec<PluginListItem> {
    let disabled: HashSet<String> = settings::load().disabled_plugins.into_iter().collect();
    installs::list_plugins(&AppPaths::plugins(), &disabled)
}

/// Relays a call from a plugin's own webview content (see plugin_assets.rs's injected harness
/// and editor.html's message-relay listener) into a fresh invocation of that plugin's backend,
/// and surfaces its reply. This is the only path a plugin's UI has back to its own backend — it
/// never gets a real Tauri capability of its own. A reply carrying an "emit" field gets relayed on
/// to the plugin's own mounted panels the same way a live process's unprompted push used to — the
/// one piece of that mechanism that survives invoke-per-call, since it rides along on a reply
/// instead of needing anything to persist between calls.
#[tauri::command]
fn invoke_plugin(app: AppHandle, id: String, method: String, params: serde_json::Value) -> Result<serde_json::Value, String> {
    if settings::load().disabled_plugins.iter().any(|p| p == &id) {
        return Err(format!("Plugin \"{id}\" is disabled."));
    }

    let folder = AppPaths::plugins().join(&id);
    let desc = plugin_host::protocol::PluginDescriptor::read(&folder).ok_or_else(|| format!("No readable plugin.json for \"{id}\"."))?;
    let log = plugin_log_fn(&app);
    let reply = plugin_host::protocol::invoke(&folder, &desc, &id, &method, &params, &log);

    if let Some(emit) = reply.get("emit").and_then(|e| e.as_object()) {
        let event = emit.get("event").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let payload = emit.get("payload").cloned().unwrap_or(serde_json::Value::Null);
        let _ = app.emit("plugin-emit", serde_json::json!({"pluginId": id, "event": event, "payload": payload}));
    }

    match reply.get("ok") {
        Some(serde_json::Value::Bool(false)) => Err(reply.get("error").and_then(|e| e.as_str()).unwrap_or("plugin call failed").to_string()),
        _ => Ok(reply.get("result").cloned().unwrap_or(serde_json::Value::Null)),
    }
}

/// Starts one instance of a "session": true plugin's backend (see plugin_session.rs) — idempotent
/// per session_id (a frontend-generated id, e.g. a UUID; Terminal mints one per open terminal
/// tab), so re-requesting an already-running session_id is a harmless no-op rather than a second
/// process. `shell` is one extra launch arg a session might want (Terminal's per-instance override,
/// from its "..." menu) — genuinely generic, not Terminal-specific: whatever the frontend passes is
/// pushed verbatim, with no Rust-side knowledge of what it means or which plugin asked for it. A
/// plugin that wants a persisted default (like Terminal's own shell setting) reads it itself via
/// get_plugin_settings/window.lowarc.getSettings() and passes the resolved value here — this
/// command no longer reaches into Settings on any plugin's behalf.
#[tauri::command]
fn start_plugin_session(app: AppHandle, id: String, session_id: String, shell: Option<String>, sessions: State<plugin_session::SessionRegistry>) -> Result<(), String> {
    if settings::load().disabled_plugins.iter().any(|p| p == &id) {
        return Err(format!("Plugin \"{id}\" is disabled."));
    }
    let folder = AppPaths::plugins().join(&id);
    let mut desc = plugin_host::protocol::PluginDescriptor::read(&folder).ok_or_else(|| format!("No readable plugin.json for \"{id}\"."))?;
    if let Some(shell) = shell {
        desc.args.push(shell);
    }
    sessions.start(&app, &folder, &desc, &id, &session_id)
}

/// Fire-and-forget write to a running session's stdin — see window.lowarc.sendSession() in
/// plugin_assets.rs. Whatever the session has to say back arrives separately, as a
/// lowarc:sessionOutput emit (plugin_session.rs), not as this call's return value.
#[tauri::command]
fn send_to_plugin_session(session_id: String, message: serde_json::Value, sessions: State<plugin_session::SessionRegistry>) -> Result<(), String> {
    sessions.send(&session_id, &message)
}

#[tauri::command]
fn stop_plugin_session(session_id: String, sessions: State<plugin_session::SessionRegistry>) {
    sessions.stop(&session_id);
}

#[tauri::command]
fn install_plugin(app: AppHandle, source_dir: String) -> Result<String, String> {
    installs::install_plugin(&AppPaths::plugins(), &PathBuf::from(source_dir), &mut |done, total| {
        let _ = app.emit("install-progress", serde_json::json!({"kind": "plugin", "bytesDone": done, "totalBytes": total}));
    })
}

#[tauri::command]
fn remove_plugin(id: String) -> Result<(), String> {
    installs::remove_plugin(&AppPaths::plugins(), &id)
}

#[tauri::command]
fn set_plugin_enabled(id: String, enabled: bool) -> Result<(), String> {
    let mut settings = settings::load();
    settings.disabled_plugins.retain(|p| p != &id);
    if !enabled {
        settings.disabled_plugins.push(id);
    }
    settings::save(&settings)
}

/// A plugin's own currently-saved values for whatever it declared in plugin.json's `settings` —
/// see window.lowarc.getSettings() in plugin_assets.rs, the harness call this backs. Empty map for
/// a plugin with no saved values yet (including one that's never declared any settings at all);
/// the schema itself lives in plugin.json, not here, so there's nothing to validate a key against
/// on this side — a plugin only ever asks for its own values back, never another plugin's.
#[tauri::command]
fn get_plugin_settings(id: String) -> std::collections::HashMap<String, String> {
    settings::load().plugin_settings.get(&id).cloned().unwrap_or_default()
}

/// Sets one (id, key) -> value in Settings.plugin_settings, leaving every other plugin's — and
/// this plugin's own other fields' — values untouched. Called from the Settings UI, not from a
/// plugin itself (a plugin only ever reads its own settings, never writes them for itself).
#[tauri::command]
fn set_plugin_setting(id: String, key: String, value: String) -> Result<(), String> {
    let mut settings = settings::load();
    settings.plugin_settings.entry(id).or_default().insert(key, value);
    settings::save(&settings)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  // Started before the builder even runs, so the port is already known by the time editor.html's
  // first plugin_asset_port() call can possibly happen.
  plugin_asset_server::start();

  // window-state must be registered here, before .run() creates the config-declared "main"
  // window, not inside .setup() — its on_window_ready hook only fires for windows created after
  // the plugin is registered, and by the time setup() runs, "main" already exists.
  let builder = tauri::Builder::default()
    .manage(RunState::default())
    .manage(BreakpointState::default())
    .manage(ExportState::default())
    .manage(plugin_session::SessionRegistry::default());
  #[cfg(desktop)]
  let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());

  builder
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_clipboard_manager::init())
    .invoke_handler(tauri::generate_handler![
      start_dev_run,
      stop_dev_run,
      pause_dev_run,
      resume_dev_run,
      step_dev_run,
      set_breakpoints,
      start_export,
      get_project_preset,
      set_project_entry,
      entry_file_exists,
      read_text_file,
      write_text_file,
      read_binary_file,
      read_plugin_asset,
      plugin_asset_port,
      list_recent_projects,
      create_project,
      open_project,
      set_recent_pinned,
      remove_recent_project,
      get_settings,
      save_settings,
      list_theme_presets,
      save_theme_preset,
      delete_theme_preset,
      list_installed_modules,
      install_module,
      remove_module,
      set_module_enabled,
      list_installed_plugins,
      install_plugin,
      remove_plugin,
      set_plugin_enabled,
      get_plugin_settings,
      set_plugin_setting,
      invoke_plugin,
      start_plugin_session,
      send_to_plugin_session,
      stop_plugin_session
    ])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }

      AppPaths::ensure_directories()?;
      // Non-fatal on purpose, unlike ensure_directories() above — a failed copy here (e.g. the
      // binary is locked by another running instance) should still let the app start with
      // whatever plugin binary was already in place, not crash outright; the affected plugin
      // just surfaces its own "couldn't start" error later, the same as any other missing/broken
      // plugin already does, rather than taking the whole app down over it.
      if let Err(err) = AppPaths::ensure_builtin_plugin_binaries() {
        log::warn!("couldn't refresh a built-in plugin's backend binary: {err}");
      }
      // The installed-copy counterpart to the dev-only copy above — no-ops entirely for a source
      // checkout (AppPaths::dev_root().is_some()), same as ensure_builtin_plugin_binaries() does
      // in reverse. resource_dir() can itself fail on some platforms/configurations; that's not
      // fatal either, for the same "don't crash the whole app over an asset problem" reasoning —
      // whatever plugin/helper ends up missing surfaces its own error later instead.
      if let Ok(resource_dir) = app.path().resource_dir() {
        if let Err(err) = AppPaths::ensure_installed_copy_resources(&resource_dir) {
          log::warn!("couldn't unpack this install's bundled plugin/runtime resources: {err}");
        }
      }

      Ok(())
    })
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(|app_handle, event| {
      // Without this, a session-mode plugin's process (a real shell, for Terminal) would be
      // orphaned rather than closed when the app quits — invoke-per-call plugins have no
      // equivalent problem since nothing outlives a single call in the first place.
      if let tauri::RunEvent::Exit = event {
        app_handle.state::<plugin_session::SessionRegistry>().stop_all();
      }
    });
}
