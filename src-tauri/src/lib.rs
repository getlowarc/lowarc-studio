mod app_paths;
mod dylib;
pub mod plugin_host;
mod projects;
pub mod runtime;
mod settings;

use app_paths::AppPaths;
use plugin_host::protocol::PluginProcess;
use projects::RecentProject;
use runtime::runtime_loader::LogLevel;
use settings::Settings;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// The one active dev-run's stop flag, if any. A second start_dev_run while one is already
/// running is refused rather than silently replacing it — mirrors "throw a visible error, let the
/// user decide" rather than guessing what they meant.
#[derive(Default)]
struct RunState(Mutex<Option<Arc<AtomicBool>>>);

/// Plugins started once at launch and kept alive for the app's whole session — wrapped in a
/// Mutex (not stored bare) because PluginProcess holds an mpsc::Receiver, which is Send but not
/// Sync; the Mutex supplies the synchronization Tauri's shared state needs regardless.
#[derive(Default)]
struct PluginState(Mutex<Vec<PluginProcess>>);

fn log_level_str(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
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

    let entry = PathBuf::from(entry_file);
    let project = PathBuf::from(project_dir);
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
fn get_settings() -> Settings {
    settings::load()
}

#[tauri::command]
fn save_settings(settings: Settings) -> Result<(), String> {
    settings::save(&settings)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .manage(RunState::default())
    .manage(PluginState::default())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![
      start_dev_run,
      stop_dev_run,
      list_recent_projects,
      create_project,
      open_project,
      get_settings,
      save_settings
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

      let log_handle = app.handle().clone();
      let log: Arc<dyn Fn(LogLevel, &str) + Send + Sync> = Arc::new(move |level, message| {
        let _ = log_handle.emit("plugin-log", serde_json::json!({"level": log_level_str(level), "message": message}));
      });
      let register_handle = app.handle().clone();
      let on_register: Arc<dyn Fn(plugin_host::protocol::PanelRegistration) + Send + Sync> = Arc::new(move |p| {
        let _ = register_handle.emit(
          "plugin-panel-registered",
          serde_json::json!({"pluginId": p.plugin_id, "id": p.id, "title": p.title, "location": p.location}),
        );
      });

      let started = plugin_host::start_all(&AppPaths::plugins(), log, on_register);
      *app.state::<PluginState>().0.lock().unwrap() = started;

      Ok(())
    })
    .on_window_event(|window, event| {
      // Plugins are their own processes, so nothing kills them just because the window closes —
      // stop them explicitly, same as a dev-run's own stop_and_kill, rather than leaving orphans.
      if let tauri::WindowEvent::Destroyed = event {
        let plugins = window.state::<PluginState>();
        for p in plugins.0.lock().unwrap().iter() {
          p.stop_and_kill();
        }
      }
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
