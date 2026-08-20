// Shared module-description types — same shape lowarc/Bootstrap already uses, and the same shape
// project.json's preset list reuses (Dependency = {id, version}), so a project's "requires" and a
// module's "requires" are literally the same schema at two different levels.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct Dependency {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(rename = "loadOrder")]
    pub load_order: i32,
    pub requires: Vec<Dependency>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self { id: String::new(), name: "Unnamed Module".into(), load_order: 100, requires: Vec::new() }
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

/// Post-order DFS over Requires, seeded by LoadOrder — identical to Bootstrap's
/// launch_config::order_by_requires: a module loads after everything it requires, tolerant of
/// cycles and missing ids.
pub fn order_by_requires(mut infos: Vec<ModuleInfo>) -> Vec<ModuleInfo> {
    infos.sort_by_key(|i| i.manifest.load_order);

    let by_id: std::collections::HashMap<String, usize> = infos
        .iter()
        .enumerate()
        .filter(|(_, i)| !i.manifest.id.is_empty())
        .map(|(idx, i)| (i.manifest.id.clone(), idx))
        .collect();

    let mut seen = vec![false; infos.len()];
    let mut order = Vec::with_capacity(infos.len());

    fn visit(
        idx: usize,
        infos: &[ModuleInfo],
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

    for idx in 0..infos.len() {
        visit(idx, &infos, &by_id, &mut seen, &mut order);
    }

    let mut slots: Vec<Option<ModuleInfo>> = infos.into_iter().map(Some).collect();
    order.into_iter().map(|i| slots[i].take().unwrap()).collect()
}
