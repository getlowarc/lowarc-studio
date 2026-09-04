// Shared module-description types — same shape lowarc/Bootstrap already uses, and the same shape
// project.json's preset list reuses (Dependency = {id, version}), so a project's "requires" and a
// module's "requires" are literally the same schema at two different levels.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct Dependency {
    pub id: String,
    pub version: String,
    /// False (the default) means what it always has: project::resolve() fails the whole run if
    /// this id isn't installed. True means the opposite — a module that cooperates with another
    /// IF it happens to be present (reads its published state, see runtime::process_module's
    /// shared/publish design) but has no structural need for it to exist at all; resolve() simply
    /// leaves it out rather than erroring. Only meaningful on a MODULE's own manifest.requires —
    /// a project's own top-level requires (ProjectPreset.requires) is always treated as mandatory
    /// regardless of this field, since a project author listing something there already means
    /// "I want this," there's no reason for them to list something they don't.
    pub optional: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(rename = "loadOrder")]
    pub load_order: i32,
    pub requires: Vec<Dependency>,
    /// Purely descriptive — shown in the Modules manage page, never read by resolve()'s
    /// dependency-closure logic (Dependency.version is the thing that's actually checked, and
    /// isn't even satisfied yet — see resolve()'s own note on that gap).
    pub version: Option<String>,
    pub description: Option<String>,
    /// Purely descriptive, shown in the Modules manage page's detail header — no marketplace
    /// exists yet to link out to, but a locally-authored module can still point at its own repo.
    pub website: Option<String>,
    /// Path to an SVG/PNG within this module's own folder, read the same way a plugin's rail icon
    /// is (see plugin_assets/loadRailIconSvg) — a missing/absent icon falls back to a generic
    /// placeholder on the frontend rather than this field being required.
    pub icon: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self { id: String::new(), name: "Unnamed Module".into(), load_order: 100, requires: Vec::new(), version: None, description: None, website: None, icon: None }
    }
}

impl Manifest {
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join("manifest.json")).ok()?;
        serde_json::from_str(&text).ok()
    }
}

#[derive(Debug)]
pub struct ModuleInfo {
    pub folder: PathBuf,
    pub manifest: Manifest,
}

/// Post-order DFS over Requires, seeded by LoadOrder, as each id's RANK in that walk (0 = runs
/// first) rather than a reordered Vec — for a caller that only has borrowed ModuleInfos to work
/// with (process_module::spawn_and_run, which doesn't own the list it was handed, so it can't
/// consume-and-rebuild it the way order_by_requires below does; takes `&[&ModuleInfo]`, not
/// `&[ModuleInfo]`, specifically so that caller can pass borrowed references straight through).
/// Both share this exact same walk; order_by_requires is just the by-value convenience on top of
/// it for a caller that does own its Vec. Identical to Bootstrap's launch_config::order_by_requires:
/// a module loads after everything it requires, tolerant of cycles and missing ids.
pub fn requires_rank(infos: &[&ModuleInfo]) -> std::collections::HashMap<String, usize> {
    let mut load_order_seed: Vec<usize> = (0..infos.len()).collect();
    load_order_seed.sort_by_key(|&i| infos[i].manifest.load_order);

    let by_id: std::collections::HashMap<String, usize> = load_order_seed
        .iter()
        .filter(|&&i| !infos[i].manifest.id.is_empty())
        .map(|&i| (infos[i].manifest.id.clone(), i))
        .collect();

    let mut seen = vec![false; infos.len()];
    let mut order = Vec::with_capacity(infos.len());

    fn visit(
        idx: usize,
        infos: &[&ModuleInfo],
        by_id: &std::collections::HashMap<String, usize>,
        seen: &mut [bool],
        order: &mut Vec<usize>,
    ) {
        if seen[idx] {
            return;
        }
        seen[idx] = true;
        for req in &infos[idx].manifest.requires {
            if req.id.is_empty() {
                continue;
            }
            if let Some(&dep_idx) = by_id.get(&req.id) {
                if dep_idx != idx {
                    visit(dep_idx, infos, by_id, seen, order);
                }
            }
        }
        order.push(idx);
    }

    for &idx in &load_order_seed {
        visit(idx, infos, &by_id, &mut seen, &mut order);
    }

    order.into_iter().enumerate().map(|(rank, idx)| (infos[idx].manifest.id.clone(), rank)).collect()
}

/// By-value convenience over requires_rank, for a caller that owns its Vec and wants it physically
/// reordered rather than just ranked.
pub fn order_by_requires(mut infos: Vec<ModuleInfo>) -> Vec<ModuleInfo> {
    let refs: Vec<&ModuleInfo> = infos.iter().collect();
    let rank = requires_rank(&refs);
    infos.sort_by_key(|i| rank.get(&i.manifest.id).copied().unwrap_or(usize::MAX));
    infos
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, load_order: i32, requires: &[&str]) -> ModuleInfo {
        ModuleInfo {
            folder: PathBuf::from(id),
            manifest: Manifest {
                id: id.to_string(),
                load_order,
                requires: requires.iter().map(|r| Dependency { id: r.to_string(), ..Dependency::default() }).collect(),
                ..Manifest::default()
            },
        }
    }

    #[test]
    fn order_by_requires_puts_a_dependency_before_its_dependent_even_against_load_order() {
        // consumer's loadOrder (1) is LOWER than producer's (2) — a naive loadOrder-only sort
        // would put consumer first, which is exactly the bug this function exists to not have:
        // spawn_and_run's whole shared/publish guarantee depends on this being requires-order, not
        // just declaration order.
        let infos = vec![info("consumer", 1, &["producer"]), info("producer", 2, &[])];
        let ordered = order_by_requires(infos);
        let ids: Vec<&str> = ordered.iter().map(|i| i.manifest.id.as_str()).collect();
        assert_eq!(ids, vec!["producer", "consumer"]);
    }

    #[test]
    fn order_by_requires_is_tolerant_of_a_cycle_and_a_missing_id() {
        let infos = vec![info("a", 1, &["b"]), info("b", 2, &["a", "missing"])];
        let ordered = order_by_requires(infos);
        assert_eq!(ordered.len(), 2, "a cycle or a dangling requires id should never drop or duplicate a module");
    }

    #[test]
    fn requires_rank_agrees_with_order_by_requires() {
        let a = info("consumer", 1, &["producer"]);
        let b = info("producer", 2, &[]);
        let refs = vec![&a, &b];
        let rank = requires_rank(&refs);
        assert!(rank["producer"] < rank["consumer"], "producer should rank before consumer, got {rank:?}");
    }
}
