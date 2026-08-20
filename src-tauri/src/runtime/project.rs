// Resolves a project's module PRESET (project.json — "here's what I need", not the modules
// themselves) against the global module store. Modules live globally (Nolan's call: reuses the
// existing toggle system's shape, avoids on-disk duplication) and a version/identity conflict
// among the modules a project actually needs is a hard error, not something silently resolved —
// the run doesn't happen at all until the user resolves it themselves.

use crate::runtime::manifest::{Dependency, Manifest, ModuleInfo};
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProjectPreset {
    pub requires: Vec<Dependency>,
}

impl ProjectPreset {
    pub fn load(project_dir: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(project_dir.join("project.json"))
            .map_err(|e| format!("project.json not found or unreadable: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("project.json is invalid: {e}"))
    }
}

/// Resolves the transitive closure of modules a project needs — its own preset, plus whatever
/// each resolved module itself Requires, exactly like the existing C# ModuleRegistry's dependency
/// closure. Every error found (missing module, or more than one global module claiming the same
/// id) is collected and returned together, not just the first — so the user sees the whole
/// picture at once and can actually decide, rather than fixing one conflict only to hit the next.
///
/// NOTE: matches by id only right now. `Dependency.version` (">=1.0.0", "=2.0.0", etc.) is not
/// yet checked for satisfaction — that needs real semver range logic (the `semver` crate, not a
/// hand-rolled comparator) and is a deliberate, acknowledged gap, not an oversight.
pub fn resolve(preset: &ProjectPreset, modules_dir: &Path) -> Result<Vec<ModuleInfo>, Vec<String>> {
    let by_id = scan_store(modules_dir);

    let mut errors = Vec::new();
    let mut resolved_ids: HashSet<String> = HashSet::new();
    let mut resolved: Vec<ModuleInfo> = Vec::new();
    let mut queue: VecDeque<String> = preset.requires.iter().map(|d| d.id.clone()).collect();

    while let Some(id) = queue.pop_front() {
        if id.is_empty() || !resolved_ids.insert(id.clone()) {
            continue; // already resolved (or already errored) — dependency cycles are fine
        }

        match by_id.get(&id) {
            None => errors.push(format!("Module \"{id}\" is required but not installed.")),
            Some(folders) if folders.len() > 1 => {
                let paths = folders.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
                errors.push(format!(
                    "Module \"{id}\" is claimed by more than one installed module: {paths}. \
                     Remove or rename one before this project can run."
                ));
            }
            Some(folders) => {
                let folder = folders[0].clone();
                let Some(manifest) = Manifest::read(&folder) else {
                    errors.push(format!("Module \"{id}\" at {} has an unreadable manifest.json.", folder.display()));
                    continue;
                };
                for dep in &manifest.requires {
                    if !dep.id.is_empty() {
                        queue.push_back(dep.id.clone());
                    }
                }
                resolved.push(ModuleInfo { folder, manifest });
            }
        }
    }

    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(errors)
    }
}

/// folder-name -> manifest.json read from every immediate subdirectory of the global store,
/// grouped by declared id (a `Vec` per id, so a duplicate is visible rather than silently
/// overwritten by whichever folder happened to be scanned last).
fn scan_store(modules_dir: &Path) -> HashMap<String, Vec<PathBuf>> {
    let mut by_id: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let Ok(entries) = std::fs::read_dir(modules_dir) else {
        return by_id; // no store yet — every requirement will report as missing, correctly
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        if let Some(manifest) = Manifest::read(&folder) {
            if !manifest.id.is_empty() {
                by_id.entry(manifest.id).or_default().push(folder);
            }
        }
    }
    by_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_module(modules_dir: &Path, folder_name: &str, id: &str, requires: &[&str]) {
        let dir = modules_dir.join(folder_name);
        std::fs::create_dir_all(&dir).unwrap();
        let requires_json: Vec<String> = requires.iter().map(|r| format!(r#"{{"id":"{r}","version":"*"}}"#)).collect();
        std::fs::write(
            dir.join("manifest.json"),
            format!(r#"{{"id":"{id}","name":"{id}","loadOrder":100,"requires":[{}]}}"#, requires_json.join(",")),
        )
        .unwrap();
    }

    #[test]
    fn resolves_direct_and_transitive_requires() {
        let modules_dir = temp_dir("resolve_ok");
        write_module(&modules_dir, "a", "mod-a", &["mod-b"]);
        write_module(&modules_dir, "b", "mod-b", &[]);

        let preset = ProjectPreset { requires: vec![Dependency { id: "mod-a".into(), version: "*".into() }] };
        let resolved = resolve(&preset, &modules_dir).expect("expected a successful resolution");

        let ids: Vec<&str> = resolved.iter().map(|m| m.manifest.id.as_str()).collect();
        assert!(ids.contains(&"mod-a"), "direct requirement should be resolved");
        assert!(ids.contains(&"mod-b"), "transitive requirement should be pulled in too");
    }

    #[test]
    fn missing_module_is_a_visible_error_not_a_silent_skip() {
        let modules_dir = temp_dir("resolve_missing");
        let preset = ProjectPreset { requires: vec![Dependency { id: "does-not-exist".into(), version: "*".into() }] };

        let errors = resolve(&preset, &modules_dir).expect_err("a missing module must fail resolution");
        assert!(errors.iter().any(|e| e.contains("does-not-exist")), "error should name the missing module: {errors:?}");
    }

    #[test]
    fn duplicate_id_in_the_store_is_a_conflict_not_a_silent_pick() {
        let modules_dir = temp_dir("resolve_conflict");
        write_module(&modules_dir, "a-old", "mod-a", &[]);
        write_module(&modules_dir, "a-new", "mod-a", &[]);

        let preset = ProjectPreset { requires: vec![Dependency { id: "mod-a".into(), version: "*".into() }] };
        let errors = resolve(&preset, &modules_dir).expect_err("two modules claiming one id must fail, not silently pick one");
        assert!(errors.iter().any(|e| e.contains("more than one")), "error should explain the conflict: {errors:?}");
    }
}
