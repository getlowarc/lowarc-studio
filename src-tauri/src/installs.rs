// List/install/remove for the two things a user installs into LowArc Studio: modules (game-side,
// see runtime::project) and plugins (editor-side, see plugin_host). Kept separate from both of
// those — they stay focused on resolving a project's requires and on starting/talking to running
// plugin processes respectively — this is purely the "what's on disk, and manage it" concern
// behind the Modules/Plugins pages. "Disabled" is state this module doesn't own (it's persisted in
// Settings) — every function here just takes the current disabled-id set as a parameter, same
// dependency-injection-for-testability shape as everywhere else in this codebase.

use crate::plugin_host::protocol::{Contributes, PluginDescriptor, PluginSettingField};
use crate::runtime::manifest::Manifest;
use crate::runtime::project;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleListItem {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub load_order: i32,
    pub requires: Vec<String>,
    pub folder: String,
    pub disabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginListItem {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub command: String,
    pub folder: String,
    pub disabled: bool,
    /// True for a "session": true plugin (Terminal) — tells editor.html to start its backend once
    /// and keep it running (see plugin_session.rs) instead of the default invoke-per-call model.
    pub session: bool,
    /// What this plugin declares it provides — see plugin_host::protocol::Contributes. The
    /// editor shell builds rail icons/console tabs from this directly. Plugins are invoked per
    /// call, not kept running, so there's no separate "is it actually running" status any more —
    /// `disabled` is the only state that matters.
    pub contributes: Contributes,
    /// This plugin's own declared settings schema, if any — see PluginSettingField. Rendering the
    /// actual fields (and reading/writing their current values) is the Settings page's job, not
    /// this list's; this is just "does this plugin have any, and what do they look like."
    pub settings: Vec<PluginSettingField>,
}

/// Every installed module, unconditionally — unlike project::resolve, which only pulls in what one
/// project's requires actually needs. A duplicate id shows up as two separate rows (same
/// reasoning as resolve's "conflict, not a silent pick" rule) rather than being deduped away.
pub fn list_modules(modules_dir: &Path, disabled: &HashSet<String>) -> Vec<ModuleListItem> {
    let mut items: Vec<ModuleListItem> = project::scan_store(modules_dir)
        .into_values()
        .flatten()
        .filter_map(|folder| {
            let manifest = Manifest::read(&folder)?;
            Some(ModuleListItem {
                disabled: disabled.contains(&manifest.id),
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                description: manifest.description,
                load_order: manifest.load_order,
                requires: manifest.requires.into_iter().map(|d| d.id).collect(),
                folder: folder.display().to_string(),
            })
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

/// Every installed plugin, read from disk — nothing to start or stop any more, plugins are
/// invoked fresh per call (see plugin_host::protocol::invoke).
pub fn list_plugins(plugins_dir: &Path, disabled: &HashSet<String>) -> Vec<PluginListItem> {
    let mut items = Vec::new();
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return items;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let Some(desc) = PluginDescriptor::read(&folder) else {
            continue;
        };
        let id = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        items.push(PluginListItem {
            disabled: disabled.contains(&id),
            name: desc.name.clone().unwrap_or_else(|| id.clone()),
            id,
            version: desc.version,
            description: desc.description,
            command: desc.command,
            folder: folder.display().to_string(),
            session: desc.session,
            contributes: desc.contributes,
            settings: desc.settings,
        });
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

/// Validates `source_dir` has a readable manifest.json with a non-empty id, refuses if that id is
/// already installed, then copies the folder in. Returns the installed module's id. `on_progress`
/// is called with (bytes copied so far, total bytes) as the copy proceeds — a module folder is
/// usually small enough this never even needs to be seen, but nothing enforces that, so this
/// doesn't assume it.
pub fn install_module(modules_dir: &Path, source_dir: &Path, on_progress: &mut dyn FnMut(u64, u64)) -> Result<String, String> {
    let manifest = Manifest::read(source_dir).ok_or_else(|| format!("{} has no readable manifest.json.", source_dir.display()))?;
    if manifest.id.is_empty() {
        return Err("That module's manifest.json has no id.".to_string());
    }
    if list_modules(modules_dir, &HashSet::new()).iter().any(|m| m.id == manifest.id) {
        return Err(format!("A module with id \"{}\" is already installed.", manifest.id));
    }

    let folder_name = source_dir.file_name().ok_or_else(|| "That's not a valid folder.".to_string())?;
    let dest = modules_dir.join(folder_name);
    if dest.exists() {
        return Err(format!("{} already exists in the modules folder.", dest.display()));
    }

    copy_dir_recursive_with_progress(source_dir, &dest, on_progress).map_err(|e| e.to_string())?;
    Ok(manifest.id)
}

/// Validates `source_dir` has a readable plugin.json, refuses if a plugin folder with the same
/// name is already installed (a plugin's id IS its folder name — see protocol.rs), then copies it
/// in. Returns the installed plugin's id. `on_progress`: see install_module's own note — this is
/// the one that actually matters in practice, since a real plugin (Monaco's vendored ~24MB, say)
/// is nowhere near instant to copy.
pub fn install_plugin(plugins_dir: &Path, source_dir: &Path, on_progress: &mut dyn FnMut(u64, u64)) -> Result<String, String> {
    PluginDescriptor::read(source_dir).ok_or_else(|| format!("{} has no readable plugin.json.", source_dir.display()))?;

    let folder_name = source_dir.file_name().ok_or_else(|| "That's not a valid folder.".to_string())?;
    let id = folder_name.to_string_lossy().into_owned();
    let dest = plugins_dir.join(folder_name);
    if dest.exists() {
        return Err(format!("A plugin folder named \"{id}\" is already installed."));
    }

    copy_dir_recursive_with_progress(source_dir, &dest, on_progress).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn remove_module(modules_dir: &Path, id: &str) -> Result<(), String> {
    let item = list_modules(modules_dir, &HashSet::new())
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("No installed module with id \"{id}\"."))?;
    std::fs::remove_dir_all(&item.folder).map_err(|e| e.to_string())
}

pub fn remove_plugin(plugins_dir: &Path, id: &str) -> Result<(), String> {
    let folder = plugins_dir.join(id);
    if !folder.is_dir() {
        return Err(format!("No installed plugin with id \"{id}\"."));
    }
    std::fs::remove_dir_all(&folder).map_err(|e| e.to_string())
}

pub(crate) fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    copy_dir_recursive_with_progress(from, to, &mut |_, _| {})
}

/// Same copy as copy_dir_recursive, but calls `on_progress(bytes_done, total_bytes)` after every
/// file. Byte-based, not file-count-based — a folder can be one huge file or hundreds of tiny
/// ones, and only bytes give a fraction that actually tracks how much work is left. total_bytes is
/// computed once up front; nothing else should be writing into `from` mid-install, so a size
/// changing out from under this isn't a case worth handling.
pub(crate) fn copy_dir_recursive_with_progress(from: &Path, to: &Path, on_progress: &mut dyn FnMut(u64, u64)) -> std::io::Result<()> {
    let total = dir_size(from)?;
    let mut done = 0u64;
    copy_dir_recursive_inner(from, to, total, &mut done, on_progress)
}

fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        total += if entry.file_type()?.is_dir() { dir_size(&entry.path())? } else { entry.metadata()?.len() };
    }
    Ok(total)
}

fn copy_dir_recursive_inner(from: &Path, to: &Path, total: u64, done: &mut u64, on_progress: &mut dyn FnMut(u64, u64)) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive_inner(&src, &dst, total, done, on_progress)?;
        } else {
            std::fs::copy(&src, &dst)?;
            *done += entry.metadata()?.len();
            on_progress(*done, total);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_installs_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_source_module(root: &Path, name: &str, id: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), format!(r#"{{"id":"{id}","name":"{id}","version":"1.0.0","description":"A test module."}}"#)).unwrap();
        std::fs::write(dir.join("extra.txt"), "payload").unwrap();
        dir
    }

    fn write_source_plugin(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.json"), r#"{"command":"run.exe","name":"Test Plugin"}"#).unwrap();
        dir
    }

    #[test]
    fn install_module_succeeds_and_shows_up_in_the_list() {
        let root = temp_dir("install_module_ok");
        let modules_dir = root.join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();
        let source = write_source_module(&root, "source", "test-mod");

        let id = install_module(&modules_dir, &source, &mut |_, _| {}).expect("install should succeed");
        assert_eq!(id, "test-mod");
        assert!(modules_dir.join("source").join("extra.txt").is_file(), "payload files must be copied too, not just the manifest");

        let list = list_modules(&modules_dir, &HashSet::new());
        assert!(list.iter().any(|m| m.id == "test-mod" && m.version.as_deref() == Some("1.0.0")));
    }

    #[test]
    fn install_module_rejects_a_folder_with_no_manifest() {
        let root = temp_dir("install_module_no_manifest");
        let modules_dir = root.join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();
        let source = root.join("empty");
        std::fs::create_dir_all(&source).unwrap();

        let err = install_module(&modules_dir, &source, &mut |_, _| {}).expect_err("a folder with no manifest.json must be rejected");
        assert!(err.contains("manifest.json"));
    }

    #[test]
    fn install_module_rejects_an_id_collision() {
        let root = temp_dir("install_module_collision");
        let modules_dir = root.join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();
        let source_a = write_source_module(&root, "source-a", "dup-mod");
        let source_b = write_source_module(&root, "source-b", "dup-mod");

        install_module(&modules_dir, &source_a, &mut |_, _| {}).unwrap();
        let err = install_module(&modules_dir, &source_b, &mut |_, _| {}).expect_err("installing the same id twice must fail");
        assert!(err.contains("already installed"));
    }

    #[test]
    fn remove_module_deletes_it() {
        let root = temp_dir("remove_module");
        let modules_dir = root.join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();
        let source = write_source_module(&root, "source", "removable-mod");
        install_module(&modules_dir, &source, &mut |_, _| {}).unwrap();

        remove_module(&modules_dir, "removable-mod").expect("remove should succeed");
        assert!(list_modules(&modules_dir, &HashSet::new()).is_empty());

        let err = remove_module(&modules_dir, "removable-mod").expect_err("removing twice should error");
        assert!(err.contains("No installed module"));
    }

    #[test]
    fn list_modules_marks_disabled_ids() {
        let root = temp_dir("list_disabled");
        let modules_dir = root.join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();
        let source = write_source_module(&root, "source", "toggle-mod");
        install_module(&modules_dir, &source, &mut |_, _| {}).unwrap();

        let disabled: HashSet<String> = ["toggle-mod".to_string()].into_iter().collect();
        let list = list_modules(&modules_dir, &disabled);
        assert!(list.iter().find(|m| m.id == "toggle-mod").unwrap().disabled);
    }

    #[test]
    fn install_plugin_succeeds_and_shows_up_in_the_list() {
        let root = temp_dir("install_plugin_ok");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let source = write_source_plugin(&root, "my-plugin");

        let id = install_plugin(&plugins_dir, &source, &mut |_, _| {}).expect("install should succeed");
        assert_eq!(id, "my-plugin");

        let list = list_plugins(&plugins_dir, &HashSet::new());
        assert!(list.iter().any(|p| p.id == "my-plugin" && p.name == "Test Plugin"));
    }

    #[test]
    fn install_plugin_rejects_a_folder_name_collision() {
        let root = temp_dir("install_plugin_collision");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let source = write_source_plugin(&root, "dup-plugin");

        install_plugin(&plugins_dir, &source, &mut |_, _| {}).unwrap();
        let err = install_plugin(&plugins_dir, &source, &mut |_, _| {}).expect_err("installing the same folder name twice must fail");
        assert!(err.contains("already installed"));
    }

    #[test]
    fn remove_plugin_deletes_it() {
        let root = temp_dir("remove_plugin");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let source = write_source_plugin(&root, "removable-plugin");
        install_plugin(&plugins_dir, &source, &mut |_, _| {}).unwrap();

        remove_plugin(&plugins_dir, "removable-plugin").expect("remove should succeed");
        assert!(list_plugins(&plugins_dir, &HashSet::new()).is_empty());
    }
}
