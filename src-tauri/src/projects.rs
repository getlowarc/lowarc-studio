// Recent-projects tracking and project creation/opening. The recents file is JSON, one entry per
// project (path and pinned), since a bare path-per-line list has nowhere to put per-entry state.
// A missing path shows up as `exists: false` for the frontend to render as a visible error, rather
// than being silently dropped from the list.

use crate::app_paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What's persisted to disk — just enough to reconstruct a RecentProject at read time (name/exists
/// are always computed live, never stored, so a renamed or deleted project shows up correctly).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    path: String,
    #[serde(default)]
    pinned: bool,
}

/// What the frontend actually gets back.
#[derive(Debug, Serialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub exists: bool,
    pub pinned: bool,
}

const RECENTS_CAP: usize = 20; // a recents list that grows forever stops being "recent"

fn read_entries(recents_file: &Path) -> Vec<StoredEntry> {
    let text = std::fs::read_to_string(recents_file).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_entries(recents_file: &Path, entries: &[StoredEntry]) -> std::io::Result<()> {
    if let Some(parent) = recents_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(entries).unwrap_or_else(|_| "[]".to_string());
    std::fs::write(recents_file, contents)
}

fn to_recent_project(entry: &StoredEntry) -> RecentProject {
    let path = PathBuf::from(&entry.path);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| entry.path.clone());
    // Whether the FOLDER is still there, not whether it happens to have a project.json — a
    // project-less folder (see open_project_in below) is just as openable as one with a preset,
    // so gating "exists" on project.json specifically would show every one of those as the
    // "missing" (grayed out, unclickable) state in the recents list despite being perfectly fine.
    let exists = path.is_dir();
    RecentProject { path: entry.path.clone(), name, exists, pinned: entry.pinned }
}

pub fn list_recent() -> Vec<RecentProject> {
    list_recent_from(&AppPaths::recent_projects_file())
}

fn list_recent_from(recents_file: &Path) -> Vec<RecentProject> {
    read_entries(recents_file).iter().map(to_recent_project).collect()
}

fn add_recent(recents_file: &Path, path: &Path) -> std::io::Result<()> {
    let canon = path.to_string_lossy().to_string();
    let mut entries = read_entries(recents_file);

    // Reopening an already-pinned project must not un-pin it.
    let was_pinned = entries.iter().find(|e| e.path == canon).map(|e| e.pinned).unwrap_or(false);
    entries.retain(|e| e.path != canon);
    entries.insert(0, StoredEntry { path: canon, pinned: was_pinned });

    // Cap unpinned entries only — pinned entries never get evicted regardless of how old they are.
    let unpinned_count = entries.iter().filter(|e| !e.pinned).count();
    if unpinned_count > RECENTS_CAP {
        let mut to_drop = unpinned_count - RECENTS_CAP;
        for i in (0..entries.len()).rev() {
            if to_drop == 0 {
                break;
            }
            if !entries[i].pinned {
                entries.remove(i);
                to_drop -= 1;
            }
        }
    }

    write_entries(recents_file, &entries)
}

/// Sets (or clears) a recent project's pinned flag. Errors if the path isn't in the recents list —
/// same "visible error, not a silent no-op" rule as everywhere else here.
pub fn set_recent_pinned(path: &Path, pinned: bool) -> Result<(), String> {
    set_recent_pinned_in(&AppPaths::recent_projects_file(), path, pinned)
}

fn set_recent_pinned_in(recents_file: &Path, path: &Path, pinned: bool) -> Result<(), String> {
    let canon = path.to_string_lossy().to_string();
    let mut entries = read_entries(recents_file);
    let entry = entries.iter_mut().find(|e| e.path == canon).ok_or_else(|| format!("{} isn't in recents.", path.display()))?;
    entry.pinned = pinned;
    write_entries(recents_file, &entries).map_err(|e| e.to_string())
}

/// Removes a project from recents entirely (the Delete action) — pinned or not.
pub fn remove_recent(path: &Path) -> Result<(), String> {
    remove_recent_in(&AppPaths::recent_projects_file(), path)
}

fn remove_recent_in(recents_file: &Path, path: &Path) -> Result<(), String> {
    let canon = path.to_string_lossy().to_string();
    let mut entries = read_entries(recents_file);
    let before = entries.len();
    entries.retain(|e| e.path != canon);
    if entries.len() == before {
        return Err(format!("{} isn't in recents.", path.display()));
    }
    write_entries(recents_file, &entries).map_err(|e| e.to_string())
}

/// Creates `<parent_dir>/<name>/` with an empty project.json preset, and adds it to recents.
/// The preset starts with no requires — the user picks modules for it afterward, this just
/// gets a real, openable project on disk.
pub fn create_project(parent_dir: &Path, name: &str) -> Result<PathBuf, String> {
    create_project_in(&AppPaths::recent_projects_file(), parent_dir, name)
}

fn create_project_in(recents_file: &Path, parent_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Project name can't be empty.".into());
    }
    // Same reasoning as export::sanitize_name and AppPaths::is_valid_component_id's other
    // callers: name is about to be joined onto parent_dir, so it can't be allowed to contain a
    // path separator or a bare "."/".." — otherwise a typed name could land the new project
    // folder somewhere other than inside parent_dir. Rejected outright rather than silently
    // stripped (unlike sanitize_name's auto-derived export folder name) since this is a user
    // directly naming their own project in a dialog that already has a place to show the error.
    if !AppPaths::is_valid_component_id(name) {
        return Err("Project name can't contain a path separator, or be \".\" or \"..\".".into());
    }
    let project_dir = parent_dir.join(name);
    if project_dir.exists() {
        return Err(format!("{} already exists.", project_dir.display()));
    }

    std::fs::create_dir_all(&project_dir).map_err(|e| e.to_string())?;
    std::fs::write(project_dir.join("project.json"), "{\"requires\":[]}\n").map_err(|e| e.to_string())?;
    add_recent(recents_file, &project_dir).map_err(|e| e.to_string())?;
    Ok(project_dir)
}

/// Validates the folder actually exists before adding it to recents — the same "throw a visible
/// error, don't guess" rule as everywhere else, not a silent no-op. project.json is deliberately
/// NOT required here: this app works as a plain editor over any folder, LowArc project or not —
/// project.json only matters once something actually needs project-run features (see
/// ProjectPreset::load treating a missing one as an empty preset, and start_dev_run/
/// set_project_entry, which create it on demand the first time Run actually needs one).
pub fn open_project(path: &Path) -> Result<(), String> {
    open_project_in(&AppPaths::recent_projects_file(), path)
}

fn open_project_in(recents_file: &Path, path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err(format!("{} is not a folder.", path.display()));
    }
    add_recent(recents_file, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests go through create_project_in/open_project_in/etc. (not the public wrappers) so
    // each test writes to its own temp recents file instead of the real
    // AppPaths::recent_projects_file() — that file is the dev app's actual recents list (in a
    // source checkout, user_data() is the repo root), so a test suite writing through the public
    // API would leave fake project entries in a real running LowArc Studio window. Each test uses
    // its own uniquely-named temp dir so they don't collide when run in parallel.

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_projects_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_project_produces_an_openable_project() {
        let base = temp_dir("create_parent");
        let recents_file = base.join("recent.json");
        let project_dir = create_project_in(&recents_file, &base, "my-game").expect("create_project should succeed");
        assert!(project_dir.join("project.json").is_file());

        // open_project must accept exactly what create_project just made, with no fixup needed.
        open_project_in(&recents_file, &project_dir).expect("a freshly created project should be immediately openable");

        let recents = list_recent_from(&recents_file);
        assert!(recents.iter().any(|r| r.path == project_dir.to_string_lossy()), "expected the new project in recents");
    }

    #[test]
    fn open_project_accepts_a_plain_folder_with_no_project_json() {
        let base = temp_dir("plain_folder");
        let recents_file = base.join("recent.json");
        open_project_in(&recents_file, &base).expect("a plain folder with no project.json should still be openable — LowArc project or not");

        let recents = list_recent_from(&recents_file);
        let entry = recents.iter().find(|r| r.path == base.to_string_lossy()).expect("expected the folder in recents");
        assert!(entry.exists, "a real folder should report exists:true even without a project.json");
    }

    #[test]
    fn open_project_rejects_a_path_that_does_not_exist() {
        let base = temp_dir("open_missing_parent");
        let recents_file = base.join("recent.json");
        let missing = base.join("nowhere");
        let err = open_project_in(&recents_file, &missing).expect_err("a path that isn't a real folder must be rejected");
        assert!(err.contains("not a folder"));
    }

    #[test]
    fn create_project_refuses_to_overwrite_an_existing_folder() {
        let base = temp_dir("no_overwrite_parent");
        let recents_file = base.join("recent.json");
        create_project_in(&recents_file, &base, "dup").unwrap();
        let err = create_project_in(&recents_file, &base, "dup").expect_err("creating the same project twice must fail");
        assert!(err.contains("already exists"));
    }

    #[test]
    fn create_project_rejects_a_name_that_would_escape_parent_dir() {
        let base = temp_dir("no_traversal_parent");
        let recents_file = base.join("recent.json");
        for bad_name in ["..", ".", "../elsewhere", "sub/dir"] {
            let err = create_project_in(&recents_file, &base, bad_name).expect_err(&format!("{bad_name:?} should be rejected"));
            assert!(err.contains("path separator"), "got: {err}");
        }
        assert!(!base.parent().unwrap().join("elsewhere").exists(), "must not have created anything outside base");
    }

    #[test]
    fn pinning_survives_truncation_past_the_cap() {
        let base = temp_dir("pin_survives_cap");
        let recents_file = base.join("recent.json");

        let first = create_project_in(&recents_file, &base, "first").unwrap();
        set_recent_pinned_in(&recents_file, &first, true).expect("pin should succeed");

        for i in 0..RECENTS_CAP + 5 {
            create_project_in(&recents_file, &base, &format!("filler-{i}")).unwrap();
        }

        let recents = list_recent_from(&recents_file);
        assert!(recents.iter().any(|r| r.path == first.to_string_lossy() && r.pinned), "pinned entry must survive truncation");
        let unpinned_count = recents.iter().filter(|r| !r.pinned).count();
        assert_eq!(unpinned_count, RECENTS_CAP, "unpinned entries should still be capped");
    }

    #[test]
    fn remove_recent_deletes_an_entry() {
        let base = temp_dir("remove_recent");
        let recents_file = base.join("recent.json");
        let project_dir = create_project_in(&recents_file, &base, "removable").unwrap();

        remove_recent_in(&recents_file, &project_dir).expect("remove should succeed");
        let recents = list_recent_from(&recents_file);
        assert!(!recents.iter().any(|r| r.path == project_dir.to_string_lossy()));

        let err = remove_recent_in(&recents_file, &project_dir).expect_err("removing twice should error");
        assert!(err.contains("isn't in recents"));
    }
}
