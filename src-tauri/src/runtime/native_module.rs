// A native-kind module exports:
//   void lowarc_module_start(const char* settings_json, void (*request_stop)(void)); // required
//   void lowarc_module_frame(double delta_seconds);                                  // optional
//   void lowarc_module_stop(void);                                                   // optional
// Bootstrap (the exported, standalone app) dlopens this directly into its own disposable
// process — fine there, since that process only ever runs once and exists for exactly this. This
// IDE is not disposable: it stays alive across many dev-runs and holds unsaved work, so it can't
// take on arbitrary native code (which by definition can crash, corrupt memory, or do literally
// anything a native process can do) in its own address space the same way. Instead this loader
// spawns one native_module_host helper process (src-tauri/src/bin/native_module_host.rs) per
// native-kind module — that helper is the thing that actually dlopens the library and calls the
// ABI above — and drives it through process_module's exact same wire protocol a real process.json
// module speaks (see spawn_and_run). A crash in a native module now only takes down its own
// helper process, never LowArc Studio itself.

use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::runtime::manifest::ModuleInfo;
use crate::runtime::process_module::{spawn_and_run, ProcessDescriptor};
use crate::runtime::runtime_loader::{LogLevel, RunContext, RuntimeLoader};

pub const DESCRIPTOR_NAME: &str = "native.json";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NativeDescriptor {
    pub library: String,
}

impl NativeDescriptor {
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join(DESCRIPTOR_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

pub fn platform_library_file_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{base}.dylib")
    } else {
        format!("lib{base}.so")
    }
}

/// The isolation helper always ships in the same directory as this app's own executable — it's
/// built by the same Cargo package (src/bin auto-discovery), never something a user installs or a
/// module provides, so there's no search path here, just "next to whatever exe is running now."
fn native_module_host_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("could not resolve the current executable: {e}"))?;
    let dir = exe.parent().ok_or("the current executable has no parent directory")?;
    let name = if cfg!(windows) { "native_module_host.exe" } else { "native_module_host" };
    Ok(dir.join(name))
}

/// Every module gets its own isolated native_module_host process — not one process per run, but
/// one PER MODULE — so a crash in one native module can't take a sibling native module down with
/// it either, the same isolation guarantee ProcessLoader already gives process.json modules.
/// Matches a set where EVERY module has native.json, same reasoning as ProcessLoader.
pub struct NativeLoader;

impl RuntimeLoader for NativeLoader {
    fn id(&self) -> &'static str {
        "native"
    }

    fn can_handle(&self, modules: &[ModuleInfo]) -> bool {
        !modules.is_empty() && modules.iter().all(|i| i.folder.join(DESCRIPTOR_NAME).exists())
    }

    fn run(&self, modules: Vec<ModuleInfo>, ctx: &RunContext) -> Result<(), String> {
        let load_order = crate::runtime::manifest::order_by_requires(modules);
        let host = native_module_host_path()?;

        let mut descriptors = Vec::new();
        for info in &load_order {
            if NativeDescriptor::read(&info.folder).is_none() {
                (ctx.log)(LogLevel::Error, &format!("Module folder \"{}\" has no readable native.json — skipped.", info.folder.display()));
                continue;
            }
            let desc = ProcessDescriptor {
                command: host.to_string_lossy().into_owned(),
                args: vec![info.folder.to_string_lossy().into_owned()],
                wants_frames: true,
                timeout_ms: 10000,
            };
            descriptors.push((info, desc));
        }

        spawn_and_run(descriptors, ctx)
    }
}
