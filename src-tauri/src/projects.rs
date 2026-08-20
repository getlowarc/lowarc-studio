// Recent-projects tracking and project creation/opening. The recents file is deliberately plain
// text, one path per line — a path list needs nothing more than that, and a missing path shows up
// as `exists: false` for the frontend to render as a visible error, not silently dropped from the
// list (Nolan's call, mirrored from the same rule the old IDE never had).

use crate::app_paths::AppPaths;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub exists: bool,
}

pub fn list_recent() -> Vec<RecentProject> {
    list_recent_from(&AppPaths::recent_projects_file())
}

fn list_recent_from(recents_file: &Path) -> Vec<RecentProject> {
    let text = std::fs::read_to_string(recents_file).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|path_str| {
            let path = PathBuf::from(path_str);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path_str.to_string());
            let exists = path.join("project.json").is_file();
            RecentProject { path: path_str.to_string(), name, exists }
        })
        .collect()
}

fn add_recent(recents_file: &Path, path: &Path) -> std::io::Result<()> {
    let canon = path.to_string_lossy().to_string();
    let mut entries: Vec<String> = list_recent_from(recents_file).into_iter().map(|r| r.path).filter(|p| p != &canon).collect();
    entries.insert(0, canon);
    entries.truncate(20); // a recents list that grows forever stops being "recent"

    if let Some(parent) = recents_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut contents = entries.join("\n");
    if !entries.is_empty() {
        contents.push('\n');
    }
    std::fs::write(recents_file, contents)
}

/// Creates `<parent_dir>/<name>/` with an empty project.json preset and a uc/ folder, and adds
/// it to recents. The preset starts with no requires — the user picks modules for it afterward,
/// this just gets a real, openable project on disk.
pub fn create_project(parent_dir: &Path, name: &str) -> Result<PathBuf, String> {
    create_project_in(&AppPaths::recent_projects_file(), parent_dir, name)
}

fn create_project_in(recents_file: &Path, parent_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Project name can't be empty.".into());
    }
    let project_dir = parent_dir.join(name);
    if project_dir.exists() {
        return Err(format!("{} already exists.", project_dir.display()));
    }

    std::fs::create_dir_all(project_dir.join("uc")).map_err(|e| e.to_string())?;
    std::fs::write(project_dir.join("project.json"), "{\"requires\":[]}\n").map_err(|e| e.to_string())?;
    add_recent(recents_file, &project_dir).map_err(|e| e.to_string())?;
    Ok(project_dir)
}

/// Validates the folder is actually a project (has project.json) before adding it to recents —
/// the same "throw a visible error, don't guess" rule as everywhere else, not a silent no-op.
pub fn open_project(path: &Path) -> Result<(), String> {
    open_project_in(&AppPaths::recent_projects_file(), path)
}

fn open_project_in(recents_file: &Path, path: &Path) -> Result<(), String> {
    if !path.join("project.json").is_file() {
        return Err(format!("{} is not a LowArc Studio project — no project.json found.", path.display()));
    }
    add_recent(recents_file, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests go through create_project_in/open_project_in (not the public create_project/
    // open_project) so each test writes to its own temp recents file instead of the real
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
        let recents_file = base.join("recent.txt");
        let project_dir = create_project_in(&recents_file, &base, "my-game").expect("create_project should succeed");
        assert!(project_dir.join("project.json").is_file());
        assert!(project_dir.join("uc").is_dir());

        // open_project must accept exactly what create_project just made, with no fixup needed.
        open_project_in(&recents_file, &project_dir).expect("a freshly created project should be immediately openable");

        let recents = list_recent_from(&recents_file);
        assert!(recents.iter().any(|r| r.path == project_dir.to_string_lossy()), "expected the new project in recents");
    }

    #[test]
    fn open_project_rejects_a_folder_with_no_project_json() {
        let base = temp_dir("not_a_project");
        let recents_file = base.join("recent.txt");
        let err = open_project_in(&recents_file, &base).expect_err("a folder with no project.json must be rejected");
        assert!(err.contains("project.json"));
    }

    #[test]
    fn create_project_refuses_to_overwrite_an_existing_folder() {
        let base = temp_dir("no_overwrite_parent");
        let recents_file = base.join("recent.txt");
        create_project_in(&recents_file, &base, "dup").unwrap();
        let err = create_project_in(&recents_file, &base, "dup").expect_err("creating the same project twice must fail");
        assert!(err.contains("already exists"));
    }
}
