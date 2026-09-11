// Shared module-description types: same shape lowarc/Bootstrap already uses, and the same shape
// project.json's preset list reuses (Dependency = {id, version}), so a project's "requires" and a
// module's "requires" are literally the same schema at two different levels.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct Dependency {
    /// The module this depends on, by manifest id. `"module"` is accepted as a spelling of the
    /// same field so a manifest can say which KIND of thing it means without the reader having to
    /// infer it — `{"module": "vector-canvas"}` and `{"id": "vector-canvas"}` are identical, and
    /// the explicit spelling is the one to prefer in anything newly written.
    #[serde(alias = "module")]
    pub id: String,
    /// A CONTRACT this depends on, instead of a specific module — "I read whatever provides this,"
    /// where a plain id means "I depend on this exact implementation." Contracts are themselves
    /// modules (kind: "contract"), so this resolves through the same store and the same version
    /// check; what differs is that the consumer then reads the published state of every module
    /// that PROVIDES it, not the contract's own (a contract publishes nothing — it has no process).
    /// Mutually exclusive with `id` in practice; `id` wins if a manifest somehow sets both.
    pub contract: String,
    pub version: String,
    /// False (the default): project::resolve() fails the run if this is not installed. True: a
    /// module that cooperates with another IF present, reading its published state, but has no
    /// structural need for it, so resolve() leaves it out rather than erroring.
    ///
    /// Only meaningful on a MODULE's requires. A project's own top-level requires is always
    /// mandatory, since listing something there already means the author wants it.
    pub optional: bool,
}

impl Dependency {
    /// What this entry actually names in the module store. A contract requirement resolves to the
    /// contract module itself (that's what gets version-checked and installed); which modules end
    /// up PROVIDING it is a separate, runtime question. See `providers_of` below.
    pub fn store_id(&self) -> &str {
        if self.id.is_empty() {
            &self.contract
        } else {
            &self.id
        }
    }

    pub fn is_contract(&self) -> bool {
        self.id.is_empty() && !self.contract.is_empty()
    }
}

/// One contract a module declares it speaks. Being listed here is the ONLY thing that makes a
/// module eligible to fill a role, which is the whole point of the change this type exists for.
/// Before it, the only way to fill a role was to be NAMED the role (a module publishing under its
/// own id meant `shared.director.draw` required a module whose id was literally "director"), so
/// there could only ever be one of anything, and a module could not take a job without renaming
/// itself into it.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct Provision {
    pub contract: String,
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
    /// Contracts this module speaks. See Provision.
    pub provides: Vec<Provision>,
    /// `"contract"` marks a manifest that DEFINES a vocabulary rather than implementing anything.
    /// It resolves and version-checks like any other module. Contracts are modules, which is what
    /// lets them install and distribute through machinery that already exists, but it has no
    /// process, so the runtime never spawns it and it never publishes anything of its own. Any
    /// other value (or none) means an ordinary module.
    pub kind: Option<String>,
    /// Purely descriptive — shown in the Modules manage page, never read by resolve()'s
    /// dependency-closure logic (Dependency.version is the thing that's actually checked, and
    /// isn't even satisfied yet; see resolve()'s own note on that gap).
    pub version: Option<String>,
    pub description: Option<String>,
    /// Purely descriptive, shown in the Modules manage page's detail header: no marketplace
    /// exists yet to link out to, but a locally-authored module can still point at its own repo.
    pub website: Option<String>,
    /// Path to an SVG/PNG within this module's own folder, read the same way a plugin's rail icon
    /// is (see plugin_assets/loadRailIconSvg): a missing/absent icon falls back to a generic
    /// placeholder on the frontend rather than this field being required.
    pub icon: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Unnamed Module".into(),
            load_order: 100,
            requires: Vec::new(),
            provides: Vec::new(),
            kind: None,
            version: None,
            description: None,
            website: None,
            icon: None,
        }
    }
}

impl Manifest {
    pub fn read(folder: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(folder.join("manifest.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// A definition rather than an implementation — resolved and version-checked, never run.
    pub fn is_contract(&self) -> bool {
        self.kind.as_deref() == Some("contract")
    }

    pub fn provides_contract(&self, contract: &str) -> bool {
        self.provides.iter().any(|p| p.contract == contract)
    }
}

/// Which of `infos` provide `contract`, in the order given — callers pass an already-ranked list,
/// so the result is in run order, which is what makes a gathered contract's array meaningful
/// (draw order is list order, so run order is z-order).
pub fn providers_of<'a>(infos: &[&'a ModuleInfo], contract: &str) -> Vec<&'a ModuleInfo> {
    infos.iter().copied().filter(|i| i.manifest.provides_contract(contract)).collect()
}

#[derive(Debug)]
pub struct ModuleInfo {
    pub folder: PathBuf,
    pub manifest: Manifest,
}

/// Post-order DFS over Requires, seeded by LoadOrder, as each id's RANK in that walk (0 = runs
/// first) rather than a reordered Vec: for a caller that only has borrowed ModuleInfos to work
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
            if req.store_id().is_empty() {
                continue;
            }
            // A contract requirement has to order this module after everything that PROVIDES the
            // contract, not merely after the contract's own definition: the definition has no
            // process and publishes nothing, so ordering against it would guarantee nothing. This
            // is what keeps the shared/publish rule true for contracts: a consumer's frame sees
            // this tick's output from every provider, not last tick's.
            if req.is_contract() {
                for (dep_idx, info) in infos.iter().enumerate() {
                    if dep_idx != idx && info.manifest.provides_contract(&req.contract) {
                        visit(dep_idx, infos, by_id, seen, order);
                    }
                }
            }
            if let Some(&dep_idx) = by_id.get(req.store_id()) {
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
        // consumer's loadOrder (1) is LOWER than producer's (2): a naive loadOrder-only sort
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

    fn contract_consumer(id: &str, load_order: i32, contract: &str) -> ModuleInfo {
        ModuleInfo {
            folder: PathBuf::from(id),
            manifest: Manifest {
                id: id.to_string(),
                load_order,
                requires: vec![Dependency { contract: contract.to_string(), ..Dependency::default() }],
                ..Manifest::default()
            },
        }
    }

    fn contract_provider(id: &str, load_order: i32, contract: &str) -> ModuleInfo {
        ModuleInfo {
            folder: PathBuf::from(id),
            manifest: Manifest {
                id: id.to_string(),
                load_order,
                provides: vec![Provision { contract: contract.to_string(), ..Provision::default() }],
                ..Manifest::default()
            },
        }
    }

    #[test]
    fn a_contract_consumer_runs_after_everything_that_provides_it() {
        // The provider is not named after the contract and the consumer never names the provider:
        // that indirection is the entire point, and it's exactly what a naive id-only walk misses.
        // Load order is set against the desired result so only the contract link can produce it.
        let infos = vec![contract_consumer("canvas", 1, "draw-commands"), contract_provider("some-director", 2, "draw-commands")];
        let ordered = order_by_requires(infos);
        let ids: Vec<&str> = ordered.iter().map(|i| i.manifest.id.as_str()).collect();
        assert_eq!(ids, vec!["some-director", "canvas"], "a provider must run before its consumer, or shared state is a tick stale");
    }

    #[test]
    fn every_provider_of_a_contract_runs_before_the_consumer_not_just_one() {
        let infos = vec![
            contract_consumer("canvas", 1, "draw-commands"),
            contract_provider("world", 2, "draw-commands"),
            contract_provider("overlay", 3, "draw-commands"),
        ];
        let ordered = order_by_requires(infos);
        let ids: Vec<&str> = ordered.iter().map(|i| i.manifest.id.as_str()).collect();
        assert_eq!(ids.last(), Some(&"canvas"), "the consumer gathers from all of them, so it goes last: {ids:?}");
    }

    #[test]
    fn ordering_against_a_contract_nothing_provides_is_not_an_error() {
        // The optional case, and the normal one for a project that hasn't written its director yet:
        // the canvas still runs, and simply draws nothing.
        let infos = vec![contract_consumer("canvas", 1, "draw-commands")];
        let ordered = order_by_requires(infos);
        assert_eq!(ordered.len(), 1);
    }

    #[test]
    fn a_requirement_spells_out_which_kind_it_is() {
        let module: Dependency = serde_json::from_str(r#"{"module": "vector-canvas"}"#).unwrap();
        let bare: Dependency = serde_json::from_str(r#"{"id": "vector-canvas"}"#).unwrap();
        let contract: Dependency = serde_json::from_str(r#"{"contract": "draw-commands"}"#).unwrap();

        // "module" is a spelling of "id", so every manifest written before contracts existed keeps
        // resolving to exactly what it always did.
        assert_eq!(module.store_id(), "vector-canvas");
        assert_eq!(bare.store_id(), "vector-canvas");
        assert!(!module.is_contract() && !bare.is_contract());

        // A contract resolves through the same store (contracts are modules), but is read from
        // the gathered key rather than from one module's own published state.
        assert_eq!(contract.store_id(), "draw-commands");
        assert!(contract.is_contract());
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
