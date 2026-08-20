mod dylib;
pub mod runtime;

use runtime::runtime_loader::LogLevel;
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

#[tauri::command]
fn start_dev_run(
    app: AppHandle,
    state: State<'_, RunState>,
    entry_file: String,
    project_dir: String,
    modules_dir: String,
) -> Result<(), String> {
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
    let modules = PathBuf::from(modules_dir);

    let log_handle = app.clone();
    let log: Arc<dyn Fn(LogLevel, &str) + Send + Sync> = Arc::new(move |level, message| {
        let _ = log_handle.emit("dev-run-log", serde_json::json!({"level": log_level_str(level), "message": message}));
    });

    let done_handle = app.clone();
    std::thread::spawn(move || {
        // Real settings/target-fps sourcing doesn't exist yet — the IDE's own settings system
        // hasn't been built. 60fps + empty settings is a deliberate placeholder, not a silent gap.
        let result = runtime::start_run(&entry, &project, &modules, 60, serde_json::json!({}), stop_flag, log);

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
    .invoke_handler(tauri::generate_handler![start_dev_run, stop_dev_run])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
