// Folder-mode export: stages a project's resolved modules + source + a launch.json into a fresh,
// non-colliding subfolder of a chosen output directory, alongside a copy of the runtime
// executable (bin/lowarc_runtime.rs; see runtime_source.rs for how that gets located). No
// packaging trickery: no zip, no appended payload; just a flat, self-contained folder
// lowarc_runtime reads straight from its own directory. Single-file/bundle packaging is a
// separate, later addition. It wraps this same staged folder differently per OS, it doesn't
// replace the staging this file does.
//
// Deliberately excludes: which modules to include isn't a user choice here. export_folder reuses
// runtime::project::resolve() exactly as dev-run does, which already resolves the project's own
// transitive `requires` closure: a module merely installed (or enabled) globally but not actually
// required never enters that set. There is no "lean" toggle because there's no scenario where
// bundling something the project doesn't need would ever be wanted; this is just what resolving
// correctly already gets you, not an optimization layered on top.

pub mod runtime_source;

use crate::installs::copy_dir_recursive;
use crate::runtime::manifest::ModuleInfo;
use crate::runtime::project::{self, ProjectPreset};
use crate::runtime::LaunchConfig;
use std::path::{Path, PathBuf};

pub struct ExportOptions {
    pub project_dir: PathBuf,
    pub modules_dir: PathBuf,
    pub output_dir: PathBuf,
    pub name: String,
    pub diagnostics_log: bool,
    pub target_fps: u32,
}

/// Stages `options` into a fresh subfolder of `options.output_dir`, using `runtime_exe` as the
/// runtime to copy in (locating that executable is a separate concern; see runtime_source.rs).
/// Returns the path to the exported folder on success.
pub fn export_folder(options: &ExportOptions, runtime_exe: &Path, log: &dyn Fn(&str)) -> Result<PathBuf, Vec<String>> {
    let preset = ProjectPreset::load(&options.project_dir).map_err(|e| vec![e])?;
    if preset.entry.trim().is_empty() {
        return Err(vec!["This project has no entry file set — set one before exporting.".into()]);
    }

    log("Resolving modules...");
    let resolved = project::resolve(&preset, &options.modules_dir)?;

    let target_dir = non_colliding_dir(&options.output_dir, &sanitize_name(&options.name));
    std::fs::create_dir_all(&target_dir).map_err(|e| vec![format!("Could not create {}: {e}", target_dir.display())])?;

    log("Copying project source...");
    copy_project_source(&options.project_dir, &target_dir)
        .map_err(|e| vec![format!("Could not copy the project's source: {e}")])?;

    log("Copying modules...");
    let mut module_rel_paths = Vec::with_capacity(resolved.len());
    let modules_dest_root = target_dir.join("modules");
    let mut needs_native_host = false;
    for info in &resolved {
        let folder_name = module_dest_folder_name(info);
        let dest = modules_dest_root.join(&folder_name);
        copy_dir_recursive(&info.folder, &dest).map_err(|e| vec![format!("Could not copy module \"{}\": {e}", info.manifest.name)])?;
        module_rel_paths.push(format!("modules/{folder_name}"));
        needs_native_host = needs_native_host || info.folder.join("native.json").is_file();
    }

    log("Copying the runtime...");
    let exe_dest = target_dir.join(runtime_dest_file_name(&options.name));
    std::fs::copy(runtime_exe, &exe_dest).map_err(|e| vec![format!("Could not copy the runtime: {e}")])?;

    // native_module_host is what actually dlopens a native-kind module (see runtime::
    // native_module (isolation, one disposable helper process per module)), only worth shipping
    // alongside an export that has at least one such module, not dead weight in every export.
    // native_module_host_path()'s own "next to the running exe" resolution is exactly why THIS
    // exe's own directory is where it has to land.
    if needs_native_host {
        let host_src = crate::runtime::native_module::native_module_host_path().map_err(|e| vec![e])?;
        let host_name = if cfg!(windows) { "native_module_host.exe" } else { "native_module_host" };
        std::fs::copy(&host_src, target_dir.join(host_name)).map_err(|e| vec![format!("Could not copy native_module_host: {e}")])?;
    }

    log("Writing launch.json...");
    let launch = LaunchConfig {
        target_fps: options.target_fps,
        source: preset.entry,
        modules: module_rel_paths,
        diagnostics_log: options.diagnostics_log,
        settings: serde_json::json!({}),
    };
    let launch_text = serde_json::to_string_pretty(&launch).map_err(|e| vec![e.to_string()])?;
    std::fs::write(target_dir.join("launch.json"), launch_text).map_err(|e| vec![format!("Could not write launch.json: {e}")])?;

    Ok(target_dir)
}

/// Copies everything in `project_dir` into `target_dir` except project.json: that file is this
/// IDE's own bookkeeping (module preset, entry path), not something the exported app should ship
/// or lowarc_runtime would ever read. Preserves the project's relative layout exactly, so
/// `preset.entry` (relative to the project root) resolves identically relative to `target_dir`.
fn copy_project_source(project_dir: &Path, target_dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(project_dir)? {
        let entry = entry?;
        if entry.file_name() == "project.json" {
            continue;
        }
        let src = entry.path();
        let dst = target_dir.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// The store folder's own name, not the module's id — matches how the store is actually laid out
/// (scan_store groups by id, but a folder can be named anything) and keeps launch.json's `modules`
/// entries readable, same spirit as the id being for identity and the folder name being for
/// on-disk location.
fn module_dest_folder_name(info: &ModuleInfo) -> String {
    info.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| info.manifest.id.clone())
}

fn runtime_dest_file_name(app_name: &str) -> String {
    let base = sanitize_name(app_name);
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base
    }
}

/// Strips characters that are illegal (or awkward) in a file/folder name on some platform, so one
/// name works everywhere rather than needing per-OS validation later. Falls back to a fixed name
/// rather than an empty string: an export always needs *some* name on disk.
fn sanitize_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c)).collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() { "ExportedApp".to_string() } else { trimmed.to_string() }
}

/// `base`, or `base (1)`, `base (2)`, ... — whichever doesn't already exist under `parent`. Never
/// deletes or clears anything already there; this is the entire safety story for the output
/// folder, not a guard bolted on beside a delete step.
fn non_colliding_dir(parent: &Path, base: &str) -> PathBuf {
    let candidate = parent.join(base);
    if !candidate.exists() {
        return candidate;
    }
    for n in 1.. {
        let candidate = parent.join(format!("{base} ({n})"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_export_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitize_name_strips_illegal_characters() {
        assert_eq!(sanitize_name("My:Game?"), "MyGame");
    }

    #[test]
    fn sanitize_name_falls_back_when_nothing_is_left() {
        assert_eq!(sanitize_name("???"), "ExportedApp");
    }

    #[test]
    fn non_colliding_dir_picks_the_first_free_name() {
        let parent = temp_dir("collide");
        std::fs::create_dir_all(parent.join("App")).unwrap();
        std::fs::create_dir_all(parent.join("App (1)")).unwrap();
        assert_eq!(non_colliding_dir(&parent, "App"), parent.join("App (2)"));
    }

    #[test]
    fn non_colliding_dir_never_touches_an_existing_folder() {
        let parent = temp_dir("preserve");
        let existing = parent.join("App");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::write(existing.join("keepme.txt"), b"do not delete me").unwrap();

        let chosen = non_colliding_dir(&parent, "App");
        assert_ne!(chosen, existing, "must not choose the folder that's already there");
        assert!(existing.join("keepme.txt").exists(), "must not have touched the existing folder's contents");
    }

    #[test]
    fn export_folder_stages_project_modules_and_launch_json() {
        let modules_dir = temp_dir("modules");
        let module_dir = modules_dir.join("echo-module");
        std::fs::create_dir_all(&module_dir).unwrap();
        std::fs::write(module_dir.join("manifest.json"), r#"{"id":"echo","name":"Echo","loadOrder":1,"requires":[]}"#).unwrap();
        // Deliberately no native.json: this fixture isn't testing native_module_host bundling
        // (that has no test seam of its own yet; see export_folder's own comment on it), and
        // native_module_host_path()'s real filesystem lookups would make this flaky depending on
        // what happens to already be on disk.
        std::fs::write(module_dir.join("process.json"), r#"{"command":"echo","args":[]}"#).unwrap();

        let project_dir = temp_dir("project");
        std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"echo","version":"*"}],"entry":"src/main.txt"}"#).unwrap();
        let src_dir = project_dir.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("main.txt"), "hello world").unwrap();

        let output_dir = temp_dir("output");
        let runtime_stub = temp_dir("runtime_stub").join("runtime_stub.bin");
        std::fs::write(&runtime_stub, b"pretend this is a real executable").unwrap();

        let options = ExportOptions {
            project_dir: project_dir.clone(),
            modules_dir: modules_dir.clone(),
            output_dir: output_dir.clone(),
            name: "My:Game".into(),
            diagnostics_log: true,
            target_fps: 60,
        };

        let logged = std::cell::RefCell::new(Vec::new());
        let target = export_folder(&options, &runtime_stub, &|msg| logged.borrow_mut().push(msg.to_string())).expect("export should succeed");

        assert_eq!(target, output_dir.join("MyGame"));
        assert!(target.join("src/main.txt").exists(), "project source should be copied preserving its relative layout");
        assert!(!target.join("project.json").exists(), "project.json is IDE bookkeeping, not export output");
        assert!(target.join("modules/echo-module/manifest.json").exists(), "resolved module should be copied in");

        let launch: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(target.join("launch.json")).unwrap()).unwrap();
        assert_eq!(launch["source"], "src/main.txt");
        assert_eq!(launch["modules"][0], "modules/echo-module");
        assert_eq!(launch["diagnosticsLog"], true);
        assert!(!logged.borrow().is_empty(), "export_folder should report progress through the log callback");
    }

    #[test]
    fn export_folder_refuses_a_project_with_no_entry_set() {
        let project_dir = temp_dir("no_entry");
        std::fs::write(project_dir.join("project.json"), r#"{"requires":[]}"#).unwrap();

        let options = ExportOptions {
            project_dir,
            modules_dir: temp_dir("no_entry_modules"),
            output_dir: temp_dir("no_entry_output"),
            name: "App".into(),
            diagnostics_log: false,
            target_fps: 60,
        };

        let runtime_stub = temp_dir("no_entry_runtime").join("stub.bin");
        std::fs::write(&runtime_stub, b"stub").unwrap();

        let errors = export_folder(&options, &runtime_stub, &|_| {}).expect_err("no entry set should be a visible error");
        assert!(errors.iter().any(|e| e.contains("entry")));
    }
}
