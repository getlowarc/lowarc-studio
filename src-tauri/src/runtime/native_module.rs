// Port of Bootstrap's native_module.rs — same ABI a native-kind module exports:
//   void lowarc_module_start(const char* settings_json, void (*request_stop)(void)); // required
//   void lowarc_module_frame(double delta_seconds);                                  // optional
//   void lowarc_module_stop(void);                                                   // optional
// Deliberately IDENTICAL signature to Bootstrap's, not just similar — a module author writes one
// `lowarc_module_start`, and it works unchanged whether it's loaded by an export (Bootstrap) or a
// dev-run (this crate). The one adaptation is internal, not ABI-visible: `request_stop()` takes
// no arguments (matching Bootstrap's ABI), so it can't carry a pointer back to which run's stop
// flag to set — CURRENT_RUN_STOP holds it instead, set at the start of each run and cleared at
// the end. That's safe here because only one dev-run happens at a time; Bootstrap never needed
// this at all since it only ever runs once per process.

use serde::Deserialize;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::dylib::Library;
use crate::runtime::manifest::ModuleInfo;
use crate::runtime::runtime_loader::{LogLevel, RunContext, RuntimeLoader};

pub const DESCRIPTOR_NAME: &str = "native.json";

fn current_run_stop() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static CURRENT_RUN_STOP: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    CURRENT_RUN_STOP.get_or_init(|| Mutex::new(None))
}

extern "C" fn request_stop() {
    if let Some(flag) = current_run_stop().lock().unwrap().as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NativeDescriptor {
    pub library: String,
}

impl NativeDescriptor {
    fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join(DESCRIPTOR_NAME)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

fn platform_library_file_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{base}.dylib")
    } else {
        format!("lib{base}.so")
    }
}

type RequestStopFn = extern "C" fn();
type StartFn = extern "C" fn(*const c_char, RequestStopFn);
type FrameFn = extern "C" fn(f64);
type StopFn = extern "C" fn();

struct NativeModule {
    name: String,
    frame_fn: Option<FrameFn>,
    stop_fn: Option<StopFn>,
    _lib: Library,
}

impl NativeModule {
    fn load(folder: &Path, desc: &NativeDescriptor, name: String, settings: &str) -> Result<Self, String> {
        let file_name = platform_library_file_name(&desc.library);
        let lib = Library::open(&folder.join(&file_name))?;

        let start: StartFn = unsafe { std::mem::transmute(lib.symbol("lowarc_module_start")?) };
        let frame_fn = lib.symbol("lowarc_module_frame").ok().map(|p| unsafe { std::mem::transmute::<_, FrameFn>(p) });
        let stop_fn = lib.symbol("lowarc_module_stop").ok().map(|p| unsafe { std::mem::transmute::<_, StopFn>(p) });

        let settings_c = CString::new(settings).unwrap_or_else(|_| CString::new("").unwrap());
        start(settings_c.as_ptr(), request_stop);

        Ok(Self { name, frame_fn, stop_fn, _lib: lib })
    }

    fn frame(&self, delta_seconds: f64) {
        if let Some(f) = self.frame_fn {
            f(delta_seconds);
        }
    }

    fn stop(&self) {
        if let Some(f) = self.stop_fn {
            f();
        }
    }
}

/// Every module runs in-process via a loaded shared library — no VM, no child process. Only
/// matches a set where EVERY module has native.json, same reasoning as ProcessLoader.
pub struct NativeLoader;

impl RuntimeLoader for NativeLoader {
    fn id(&self) -> &'static str {
        "native"
    }

    fn can_handle(&self, modules: &[ModuleInfo]) -> bool {
        !modules.is_empty() && modules.iter().all(|i| i.folder.join(DESCRIPTOR_NAME).exists())
    }

    fn run(&self, modules: Vec<ModuleInfo>, ctx: &RunContext) -> Result<(), String> {
        *current_run_stop().lock().unwrap() = Some(ctx.stop_flag.clone());

        let load_order = crate::runtime::manifest::order_by_requires(modules);
        let settings = ctx.settings.to_string();

        let mut loaded = Vec::new();
        for info in &load_order {
            let Some(desc) = NativeDescriptor::read(&info.folder) else {
                (ctx.log)(LogLevel::Error, &format!("Module folder \"{}\" has no readable native.json — skipped.", info.folder.display()));
                continue;
            };
            match NativeModule::load(&info.folder, &desc, info.manifest.name.clone(), &settings) {
                Ok(m) => loaded.push(m),
                Err(e) => (ctx.log)(LogLevel::Error, &format!("Module \"{}\" failed to load — skipped. {e}", info.manifest.name)),
            }
        }

        let load_order_rank: std::collections::HashMap<&str, i32> =
            load_order.iter().map(|i| (i.manifest.name.as_str(), i.manifest.load_order)).collect();
        loaded.sort_by_key(|m| load_order_rank.get(m.name.as_str()).copied().unwrap_or(i32::MAX));

        if loaded.is_empty() {
            *current_run_stop().lock().unwrap() = None;
            return Err("Every native module failed to load.".into());
        }

        crate::runtime::driver::run(ctx.target_fps, &ctx.stop_flag, |delta| {
            for m in &loaded {
                m.frame(delta);
            }
        });

        for m in &loaded {
            m.stop();
        }
        *current_run_stop().lock().unwrap() = None;
        Ok(())
    }
}
