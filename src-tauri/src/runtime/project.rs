// Resolves a project's module PRESET (project.json — "here's what I need", not the modules
// themselves) against the global module store. Modules live globally (Nolan's call: reuses the
// existing toggle system's shape, avoids on-disk duplication) and a version/identity conflict
// among the modules a project actually needs is a hard error, not something silently resolved:
// the run doesn't happen at all until the user resolves it themselves.

use crate::runtime::manifest::{Dependency, Manifest, ModuleInfo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ProjectPreset {
    pub requires: Vec<Dependency>,
    /// Path to the project's entry file, relative to the project's own root. Empty by default:
    /// create_project never sets one (there's no project-configuration UI to set one from yet;
    /// this field exists so dev-run has something to read once that UI does). A blank entry is
    /// checked for on the frontend before start_dev_run is even called, so this stays a plain
    /// String rather than an Option: "not set" and "empty string" are the same thing here.
    pub entry: String,
}

impl ProjectPreset {
    // A missing project.json is treated as an empty preset (no requires, no entry set), not an
    // error: this app works as a plain editor over any folder, LowArc project or not, and
    // project.json only matters once something actually needs a project-run feature (module
    // resolution, an entry file). One that exists but fails to PARSE is still a real error,
    // though: that's a genuine problem with a file the user (or a previous run) actually wrote,
    // not an absence to quietly paper over.
    pub fn load(project_dir: &Path) -> Result<Self, String> {
        let path = project_dir.join("project.json");
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("project.json not found or unreadable: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("project.json is invalid: {e}"))
    }

    pub fn save(&self, project_dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(project_dir.join("project.json"), text).map_err(|e| e.to_string())
    }
}

/// Resolves the transitive closure of modules a project needs: its preset, plus whatever each
/// resolved module requires. Every error is collected and returned together rather than just the
/// first, so a user sees the whole picture instead of fixing one conflict only to hit the next.
///
/// Resolution walks by id; the ranges are then checked in one pass at the end, by check_versions.
pub fn resolve(preset: &ProjectPreset, modules_dir: &Path) -> Result<Vec<ModuleInfo>, Vec<String>> {
    let by_id = scan_store(modules_dir);

    let mut errors = Vec::new();
    // Two separate trackers, not one: resolved_ids guards against reprocessing a module that's
    // already been successfully resolved (and, same as before, against infinite requeueing on a
    // dependency cycle among successfully-installed modules — nothing here ever re-expands a
    // resolved module's own requires a second time). errored_ids only dedupes the ERROR MESSAGE
    // for a given id. It does NOT prevent reprocessing the way resolved_ids does, since a missing
    // id referenced optionally by one module and mandatorily by another has to still surface as an
    // error the moment ANY mandatory reference to it shows up, however many optional references to
    // the same still-missing id came first.
    let mut resolved_ids: HashSet<String> = HashSet::new();
    let mut errored_ids: HashSet<String> = HashSet::new();
    let mut resolved: Vec<ModuleInfo> = Vec::new();
    // (id, optional): a project's own top-level requires is always mandatory (see Dependency's
    // own doc comment on why); only a MODULE's own manifest.requires can mark one optional.
    let mut queue: VecDeque<(String, bool)> = preset.requires.iter().map(|d| (d.store_id().to_string(), false)).collect();

    while let Some((id, optional)) = queue.pop_front() {
        if id.is_empty() || resolved_ids.contains(&id) {
            continue;
        }

        match by_id.get(&id) {
            None if optional => continue, // safe to be absent. See Dependency::optional
            None => {
                if errored_ids.insert(id.clone()) {
                    errors.push(format!("Module \"{id}\" is required but not installed."));
                }
            }
            Some(folders) if folders.len() > 1 => {
                if errored_ids.insert(id.clone()) {
                    let paths = folders.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
                    errors.push(format!(
                        "Module \"{id}\" is claimed by more than one installed module: {paths}. \
                         Remove or rename one before this project can run."
                    ));
                }
            }
            Some(folders) => {
                let folder = folders[0].clone();
                let manifest = match Manifest::read(&folder) {
                    Ok(m) => m,
                    Err(why) => {
                        if errored_ids.insert(id.clone()) {
                            errors.push(format!("Module \"{id}\" has an unreadable manifest. {why}"));
                        }
                        continue;
                    }
                };
                resolved_ids.insert(id);

                // Two ways a manifest can encode the same fact twice and mean neither clearly.
                // Both are the author's mistake, not the user's, so they say which module and
                // which entry rather than just failing the run.
                if manifest.is_contract() && folder.join("process.json").exists() {
                    errors.push(format!(
                        "Module \"{}\" is declared kind \"contract\" but ships a process.json. A contract \
                         defines a vocabulary and never runs, so that process would be silently ignored. \
                         Drop one of the two.",
                        manifest.id
                    ));
                }
                if let Some(kind) = manifest.invalid_kind() {
                    errors.push(format!(
                        "Module \"{}\" declares kind \"{kind}\", which this version of LowArc does not know. \
                         The only kind is \"contract\"; leave it out for an ordinary module.",
                        manifest.id
                    ));
                }
                for dep in manifest.requires.iter().filter(|d| d.names_both()) {
                    errors.push(format!(
                        "Module \"{}\" requires both a module \"{}\" and a contract \"{}\" in one entry. \
                         Those mean different things; split them into two entries.",
                        manifest.id, dep.id, dep.contract
                    ));
                }

                for dep in &manifest.requires {
                    // store_id(), not id: a contract requirement names the contract MODULE, which is
                    // what gets installed and version-checked. Which modules provide it is a
                    // separate question answered at run time, and deliberately not resolved here:
                    // pulling in every installed provider would silently add modules the project
                    // never asked for.
                    if !dep.store_id().is_empty() {
                        queue.push_back((dep.store_id().to_string(), dep.optional));
                    }
                }
                resolved.push(ModuleInfo { folder, manifest });
            }
        }
    }

    errors.extend(check_versions(preset, &resolved));

    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(errors)
    }
}

/// Every requirement's range against the version of whatever resolved to satisfy it.
///
/// A second pass rather than part of the walk above, because the walk visits each id ONCE: a
/// module wanted by three others at three different ranges would only ever have the first checked.
/// Here every demand is visible, so all three are, and a module that satisfies nobody says so
/// once per unsatisfied requirer rather than once in total.
fn check_versions(preset: &ProjectPreset, resolved: &[ModuleInfo]) -> Vec<String> {
    let by_id: HashMap<&str, &ModuleInfo> = resolved.iter().map(|i| (i.manifest.id.as_str(), i)).collect();
    let mut errors = Vec::new();

    // (who is asking, what they asked for). The project itself asks under its own name, so an
    // error can say whether the demand came from a module or from project.json.
    let demands = preset.requires.iter().map(|d| ("this project", d)).chain(
        resolved.iter().flat_map(|i| i.manifest.requires.iter().map(move |d| (i.manifest.id.as_str(), d))),
    );

    for (who, dep) in demands {
        let range = match dep.range() {
            Ok(r) => r,
            Err(why) => {
                errors.push(format!("{who} requires \"{}\", but {why}", dep.store_id()));
                continue;
            }
        };
        // Nothing resolved under this id means it was optional and absent, which the walk above
        // already decided is fine. A range cannot object to something that is not there.
        let Some(target) = by_id.get(dep.store_id()) else { continue };
        if range == semver::VersionReq::STAR {
            continue;
        }
        match target.manifest.declared_version() {
            Some(have) if range.matches(&have) => {}
            Some(have) => errors.push(format!(
                "{who} requires \"{}\" {}, but the installed one is {have}.",
                dep.store_id(),
                dep.version
            )),
            None => errors.push(format!(
                "{who} requires \"{}\" {}, but that module declares no usable version, so nothing can be checked against it.",
                dep.store_id(),
                dep.version
            )),
        }
    }
    errors
}

/// folder-name -> manifest.json read from every immediate subdirectory of the global store,
/// grouped by declared id (a `Vec` per id, so a duplicate is visible rather than silently
/// overwritten by whichever folder happened to be scanned last). pub(crate) so installs.rs can
/// reuse it for the "list everything installed, regardless of what any one project needs" view.
pub(crate) fn scan_store(modules_dir: &Path) -> HashMap<String, Vec<PathBuf>> {
    let mut by_id: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let Ok(entries) = std::fs::read_dir(modules_dir) else {
        return by_id; // no store yet: every requirement will report as missing, correctly
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        if let Ok(manifest) = Manifest::read(&folder) {
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
            format!(r#"{{"id":"{id}","name":"{id}","priority":100,"requires":[{}]}}"#, requires_json.join(",")),
        )
        .unwrap();
    }

    /// Writes a manifest verbatim, for the malformed cases write_module can't express.
    fn write_raw(modules_dir: &Path, folder_name: &str, manifest: &str, with_process: bool) {
        let dir = modules_dir.join(folder_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        if with_process {
            std::fs::write(dir.join("process.json"), r#"{"command":"noop"}"#).unwrap();
        }
    }

    #[test]
    fn a_contract_that_also_ships_a_process_is_rejected_rather_than_half_ignored() {
        let modules_dir = temp_dir("resolve_contract_with_process");
        write_raw(&modules_dir, "c", r#"{"id":"a-contract","kind":"contract","name":"C","requires":[]}"#, true);

        let preset = ProjectPreset { requires: vec![Dependency { id: "a-contract".into(), version: "*".into(), ..Dependency::default() }], ..Default::default() };
        let errors = resolve(&preset, &modules_dir).expect_err("a contract with a process is a contradiction");

        assert!(errors.iter().any(|e| e.contains("process.json")), "error should name the problem, got {errors:?}");
    }

    #[test]
    fn a_requirement_naming_both_a_module_and_a_contract_is_rejected() {
        let modules_dir = temp_dir("resolve_names_both");
        write_raw(
            &modules_dir,
            "m",
            r#"{"id":"confused","name":"Confused","requires":[{"id":"some-module","contract":"some-contract","version":"*"}]}"#,
            false,
        );

        let preset = ProjectPreset { requires: vec![Dependency { id: "confused".into(), version: "*".into(), ..Dependency::default() }], ..Default::default() };
        let errors = resolve(&preset, &modules_dir).expect_err("one entry cannot mean both");

        assert!(
            errors.iter().any(|e| e.contains("some-module") && e.contains("some-contract")),
            "error should name both halves so the author can split them, got {errors:?}"
        );
    }

    /// A module with a declared version and arbitrary requirement entries.
    fn write_versioned(modules_dir: &Path, id: &str, version: &str, requires: &[(&str, &str)]) {
        let dir = modules_dir.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let reqs: Vec<String> = requires.iter().map(|(dep, range)| format!(r#"{{"id":"{dep}","version":"{range}"}}"#)).collect();
        std::fs::write(
            dir.join("manifest.json"),
            format!(r#"{{"id":"{id}","name":"{id}","version":"{version}","requires":[{}]}}"#, reqs.join(",")),
        )
        .unwrap();
    }

    fn preset_for(id: &str, range: &str) -> ProjectPreset {
        ProjectPreset { requires: vec![Dependency { id: id.into(), version: range.into(), ..Dependency::default() }], ..Default::default() }
    }

    #[test]
    fn a_range_that_the_installed_version_satisfies_resolves() {
        let modules_dir = temp_dir("ver_ok");
        write_versioned(&modules_dir, "lib", "1.4.2", &[]);
        assert!(resolve(&preset_for("lib", "^1"), &modules_dir).is_ok());
        assert!(resolve(&preset_for("lib", ">=1.2, <2"), &modules_dir).is_ok());
    }

    #[test]
    fn a_range_the_installed_version_misses_is_an_error_naming_both() {
        let modules_dir = temp_dir("ver_bad");
        write_versioned(&modules_dir, "lib", "2.0.0", &[]);

        let errors = resolve(&preset_for("lib", "^1"), &modules_dir).expect_err("2.0.0 does not satisfy ^1");
        let joined = errors.join(" ");
        assert!(joined.contains("^1") && joined.contains("2.0.0"), "say what was wanted and what is there, got {errors:?}");
    }

    #[test]
    fn star_still_accepts_a_module_that_declares_no_version() {
        // 29 of the requirements in this repo say "*", and most fixture manifests declare no
        // version at all. Enforcement must not turn that into an error.
        let modules_dir = temp_dir("ver_star");
        write_module(&modules_dir, "m", "mod-m", &[]);
        assert!(resolve(&preset_for("mod-m", "*"), &modules_dir).is_ok());
        assert!(resolve(&preset_for("mod-m", ""), &modules_dir).is_ok(), "an empty range means the same as *");
    }

    #[test]
    fn a_real_range_against_a_module_with_no_version_is_an_error() {
        let modules_dir = temp_dir("ver_none");
        write_module(&modules_dir, "m", "mod-m", &[]);

        let errors = resolve(&preset_for("mod-m", "^1"), &modules_dir).expect_err("nothing to check ^1 against");
        assert!(errors.iter().any(|e| e.contains("declares no usable version")), "got {errors:?}");
    }

    #[test]
    fn every_requirer_of_one_module_is_checked_not_just_the_first() {
        // The whole reason this is a second pass. The resolve walk visits "lib" once, so a
        // single-pass check would only ever have seen whichever requirer reached it first.
        let modules_dir = temp_dir("ver_many");
        write_versioned(&modules_dir, "lib", "1.0.0", &[]);
        write_versioned(&modules_dir, "happy", "1.0.0", &[("lib", "^1")]);
        write_versioned(&modules_dir, "unhappy", "1.0.0", &[("lib", "^3")]);

        let preset = ProjectPreset {
            requires: vec![
                Dependency { id: "happy".into(), version: "*".into(), ..Dependency::default() },
                Dependency { id: "unhappy".into(), version: "*".into(), ..Dependency::default() },
            ],
            ..Default::default()
        };
        let errors = resolve(&preset, &modules_dir).expect_err("unhappy wants ^3 and gets 1.0.0");
        assert!(errors.iter().any(|e| e.contains("unhappy") && e.contains("^3")), "got {errors:?}");
    }

    #[test]
    fn a_range_that_is_not_a_range_is_reported_rather_than_ignored() {
        let modules_dir = temp_dir("ver_garbage");
        write_versioned(&modules_dir, "lib", "1.0.0", &[]);

        let errors = resolve(&preset_for("lib", "newest please"), &modules_dir).expect_err("not a range");
        assert!(errors.iter().any(|e| e.contains("is not a version range")), "got {errors:?}");
    }

    #[test]
    fn resolves_direct_and_transitive_requires() {
        let modules_dir = temp_dir("resolve_ok");
        write_module(&modules_dir, "a", "mod-a", &["mod-b"]);
        write_module(&modules_dir, "b", "mod-b", &[]);

        let preset = ProjectPreset { requires: vec![Dependency { id: "mod-a".into(), version: "*".into(), ..Dependency::default() }], ..Default::default() };
        let resolved = resolve(&preset, &modules_dir).expect("expected a successful resolution");

        let ids: Vec<&str> = resolved.iter().map(|m| m.manifest.id.as_str()).collect();
        assert!(ids.contains(&"mod-a"), "direct requirement should be resolved");
        assert!(ids.contains(&"mod-b"), "transitive requirement should be pulled in too");
    }

    #[test]
    fn missing_module_is_a_visible_error_not_a_silent_skip() {
        let modules_dir = temp_dir("resolve_missing");
        let preset = ProjectPreset { requires: vec![Dependency { id: "does-not-exist".into(), version: "*".into(), ..Dependency::default() }], ..Default::default() };

        let errors = resolve(&preset, &modules_dir).expect_err("a missing module must fail resolution");
        assert!(errors.iter().any(|e| e.contains("does-not-exist")), "error should name the missing module: {errors:?}");
    }

    #[test]
    fn duplicate_id_in_the_store_is_a_conflict_not_a_silent_pick() {
        let modules_dir = temp_dir("resolve_conflict");
        write_module(&modules_dir, "a-old", "mod-a", &[]);
        write_module(&modules_dir, "a-new", "mod-a", &[]);

        let preset = ProjectPreset { requires: vec![Dependency { id: "mod-a".into(), version: "*".into(), ..Dependency::default() }], ..Default::default() };
        let errors = resolve(&preset, &modules_dir).expect_err("two modules claiming one id must fail, not silently pick one");
        assert!(errors.iter().any(|e| e.contains("more than one")), "error should explain the conflict: {errors:?}");
    }

    fn write_module_with_dep(modules_dir: &Path, folder_name: &str, id: &str, dep_id: &str, optional: bool) {
        let dir = modules_dir.join(folder_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            format!(r#"{{"id":"{id}","name":"{id}","priority":100,"requires":[{{"id":"{dep_id}","version":"*","optional":{optional}}}]}}"#),
        )
        .unwrap();
    }

    #[test]
    fn an_optional_missing_dependency_is_silently_left_out_not_an_error() {
        let modules_dir = temp_dir("resolve_optional_missing");
        write_module_with_dep(&modules_dir, "a", "mod-a", "does-not-exist", true);

        let preset = ProjectPreset { requires: vec![Dependency { id: "mod-a".into(), version: "*".into(), ..Dependency::default() }], ..Default::default() };
        let resolved = resolve(&preset, &modules_dir).expect("an optional-and-missing dependency should not fail resolution");

        let ids: Vec<&str> = resolved.iter().map(|m| m.manifest.id.as_str()).collect();
        assert_eq!(ids, vec!["mod-a"], "the module that declared it should still resolve, the missing optional dep should just be absent");
    }

    #[test]
    fn a_mandatory_reference_still_errors_even_after_an_optional_one_saw_the_same_missing_id() {
        // mod-a asks for "missing" optionally; mod-b asks for the SAME id mandatorily. Whichever
        // gets queued first must not let the other's requirement go unnoticed.
        let modules_dir = temp_dir("resolve_optional_then_mandatory");
        write_module_with_dep(&modules_dir, "a", "mod-a", "missing", true);
        write_module_with_dep(&modules_dir, "b", "mod-b", "missing", false);

        let preset = ProjectPreset {
            requires: vec![
                Dependency { id: "mod-a".into(), version: "*".into(), ..Dependency::default() },
                Dependency { id: "mod-b".into(), version: "*".into(), ..Dependency::default() },
            ],
            ..Default::default()
        };
        let errors = resolve(&preset, &modules_dir).expect_err("mod-b's mandatory requirement must still fail resolution");
        assert!(errors.iter().any(|e| e.contains("missing")), "error should name the missing module: {errors:?}");
        assert_eq!(errors.len(), 1, "the same missing id should only be reported once, not once per referencing module: {errors:?}");
    }
}
