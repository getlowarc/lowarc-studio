mod app_paths;
mod dylib;
mod installs;
pub mod plugin_host;
mod plugin_asset_server;
mod plugin_protocol;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// The one active dev-run's stop flag, if any. A second start_dev_run while one is already
/// running is refused rather than silently replacing it — mirrors "throw a visible error, let the
/// user decide" rather than guessing what they meant.
#[derive(Default)]
struct RunState(Mutex<Option<Arc<AtomicBool>>>);

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
fn start_dev_run(app: AppHandle, state: State<'_, RunState>, entry_file: String, project_dir: String) -> Result<(), String> {
    let stop_flag = {
        let mut guard = state.0.lock().unwrap();
        if guard.is_some() {
            return Err("A run is already active — stop it before starting another.".into());
        }
        let flag = Arc::new(AtomicBool::new(false));
        *guard = Some(flag.clone());
        flag
    };

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

    let target_fps = settings::load().dev_run_target_fps;

    let done_handle = app.clone();
    std::thread::spawn(move || {
        // Per-module settings (the 4th arg) aren't sourced from anywhere real yet — that's config
        // handed to game modules at start, a separate concept from Studio's own settings.json.
        let result = runtime::start_run(&entry, &project, &modules, target_fps, serde_json::json!({}), stop_flag, log);

        *done_handle.state::<RunState>().0.lock().unwrap() = None;
        let payload = match &result {
            Ok(()) => serde_json::json!({"ok": true}),
            Err(errors) => serde_json::json!({"ok": false, "errors": errors}),
        };
        let _ = done_handle.emit("dev-run-ended", payload);
    });

    Ok(())
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

/// Reads an open file's contents so the host can push them into whichever viewer plugin claimed
/// that extension — a viewer's own sandboxed iframe has no filesystem access of its own, on
/// purpose, so this has to happen host-side rather than the plugin reading the file itself.
#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("Could not read {path}: {e}"))
}

/// The other half of read_text_file — invoked host-side in response to a viewer plugin's
/// `window.lowarc.saveFile(path, contents)`, same reasoning as read: a sandboxed viewer iframe
/// has no filesystem access of its own.
#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("Could not save {path}: {e}"))
}

/// Lets editor.html (the host page, not a plugin's own iframe) read one of a plugin's static
/// assets — e.g. a rail icon — as text, over IPC rather than a network request of any kind.
/// Not strictly required now that plugin assets are served over real HTTP (fetch() would work
/// too), but kept as-is: it already works, and rail icons need the raw SVG text to sanitize and
/// inline, not a URL to fetch.
#[tauri::command]
fn read_plugin_asset(plugin_id: String, rel_path: String) -> Result<String, String> {
    let resolved = plugin_protocol::resolve_asset_path(&plugin_id, &rel_path)
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
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        }
        None => Err("No run is active.".into()),
    }
}

#[tauri::command]
fn list_installed_modules() -> Vec<ModuleListItem> {
    let disabled: HashSet<String> = settings::load().disabled_modules.into_iter().collect();
    installs::list_modules(&AppPaths::modules(), &disabled)
}

#[tauri::command]
fn install_module(source_dir: String) -> Result<String, String> {
    installs::install_module(&AppPaths::modules(), &PathBuf::from(source_dir))
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

/// Relays a call from a plugin's own webview content (see plugin_protocol.rs's injected harness
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
/// process. `shell` is Terminal-specific (a per-instance shell override, from its "..." menu) but
/// kept generic here as "the one extra launch arg a session might want" rather than hardcoding
/// Terminal's name into the host — falls back to Settings.terminalShell, then to the plugin's own
/// platform default, when not given.
#[tauri::command]
fn start_plugin_session(app: AppHandle, id: String, session_id: String, shell: Option<String>, sessions: State<plugin_session::SessionRegistry>) -> Result<(), String> {
    if settings::load().disabled_plugins.iter().any(|p| p == &id) {
        return Err(format!("Plugin \"{id}\" is disabled."));
    }
    let folder = AppPaths::plugins().join(&id);
    let mut desc = plugin_host::protocol::PluginDescriptor::read(&folder).ok_or_else(|| format!("No readable plugin.json for \"{id}\"."))?;
    if let Some(shell) = shell.or_else(|| settings::load().terminal_shell) {
        desc.args.push(shell);
    }
    sessions.start(&app, &folder, &desc, &id, &session_id)
}

/// Fire-and-forget write to a running session's stdin — see window.lowarc.sendSession() in
/// plugin_protocol.rs. Whatever the session has to say back arrives separately, as a
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
fn install_plugin(source_dir: String) -> Result<String, String> {
    installs::install_plugin(&AppPaths::plugins(), &PathBuf::from(source_dir))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  // Started before the builder even runs, so the port is already known by the time editor.html's
  // first plugin_asset_port() call can possibly happen.
  plugin_asset_server::start();

  // window-state must be registered here, before .run() creates the config-declared "main"
  // window, not inside .setup() — its on_window_ready hook only fires for windows created after
  // the plugin is registered, and by the time setup() runs, "main" already exists.
  let builder = tauri::Builder::default().manage(RunState::default()).manage(plugin_session::SessionRegistry::default());
  #[cfg(desktop)]
  let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());

  builder
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![
      start_dev_run,
      stop_dev_run,
      get_project_preset,
      set_project_entry,
      entry_file_exists,
      read_text_file,
      write_text_file,
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
