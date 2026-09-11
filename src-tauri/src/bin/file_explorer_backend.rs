// The file-explorer plugin's backend: a standalone binary, invoked fresh per call, same wire
// protocol as any other plugin (see plugin_host/protocol.rs): one JSON line in on stdin
// (`{"method":..,"params":..}`), one JSON line out on stdout (`{"ok":bool,"result"?:..,
// "error"?:string}`). Deliberately self-contained — no dependency on lowarc_studio_lib — a real
// third-party plugin author has no access to the host's internals either, and this binary is
// meant to double as the reference example for "here's what a compiled plugin backend looks
// like."
//
// Every call carries "root" (the project directory) alongside its own arguments and gets
// canonicalize-checked against it before touching disk: same reasoning as plugin_assets.rs's
// path-traversal check for served assets: never trust a path without confirming it's still inside
// the boundary it's supposed to be confined to, regardless of who's asking. The Draft Tool methods
// below (openDraft/captureBaseline/diffStatus/revertAll/commit) are the one exception: their
// paths come from the host's own already-validated lowarc:beforeSave broadcast, not user input
// into this plugin, so they don't need a second root check of their own.
//
// ---------- Draft Tool: a lightweight, disk-based backward snapshot for the CURRENT session ----
// Bolted onto the file explorer (not a separate plugin — Nolan: "it will just be bolted on to the
// normal functionality") since browsing files and reviewing what changed in them are the same
// surface. Never stops a save from happening — it just watches for one (via the host's generic
// lowarc:beforeSave, see write_text_file in lib.rs) and, the first time a tracked path changes
// after a Draft opens, keeps that path's previous content in this binary's own scratch storage.
// Revert restores every captured path; Commit just forgets them (current disk content was already
// correct). Storage is entirely this binary's own concern — the host doesn't know the path, just
// wipes it once on every launch (see lib.rs's setup()) so "session-only" is a real guarantee
// rather than "OS temp-dir cleanup eventually happens." One folder per open Draft:
//   <temp_dir>/lowarc-offshoot/<draft_id>/manifest.json   — { "<real path>": {"index":0,"existed":true} }
//   <temp_dir>/lowarc-offshoot/<draft_id>/snapshots/<index>.txt: that path's original content

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn main() {
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        return; // EOF before any request — nothing to do
    }

    let reply = match serde_json::from_str::<Value>(line.trim()) {
        Ok(request) => {
            let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            handle(method, &params)
        }
        Err(e) => json!({"ok": false, "error": format!("invalid request: {e}")}),
    };

    println!("{}", serde_json::to_string(&reply).unwrap_or_else(|_| r#"{"ok":false,"error":"failed to encode reply"}"#.to_string()));
}

fn handle(method: &str, params: &Value) -> Value {
    let Some(root) = params.get("root").and_then(|v| v.as_str()) else {
        return json!({"ok": false, "error": "missing \"root\""});
    };
    let root = Path::new(root);

    let result = match method {
        "listDir" => str_param(params, "path").and_then(|path| list_dir(root, path)),
        "createFile" => str_param(params, "path").and_then(|path| create_entry(root, path, false)),
        "createFolder" => str_param(params, "path").and_then(|path| create_entry(root, path, true)),
        "rename" => (|| {
            let path = str_param(params, "path")?;
            let new_name = str_param(params, "newName")?;
            rename_entry(root, path, new_name)
        })(),
        "move" => (|| {
            let source = str_param(params, "sourcePath")?;
            let dest_dir = str_param(params, "destDir")?;
            move_entry(root, source, dest_dir)
        })(),
        "copy" => (|| {
            let source = str_param(params, "sourcePath")?;
            let dest_dir = str_param(params, "destDir")?;
            copy_entry(root, source, dest_dir)
        })(),
        "deletePath" => str_param(params, "path").and_then(|path| delete_entry(root, path)),
        "countTree" => count_tree(root),
        "openDraft" => (|| {
            let draft_id = str_param(params, "draftId")?;
            let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("Untitled Draft");
            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("");
            open_draft(draft_id, label, description)
        })(),
        "getActiveDraft" => get_active_draft(),
        "captureBaseline" => (|| {
            let draft_id = str_param(params, "draftId")?;
            let path = str_param(params, "path")?;
            let previous_content = params.get("previousContent").and_then(|v| v.as_str());
            capture_baseline(draft_id, path, previous_content)
        })(),
        "diffStatus" => str_param(params, "draftId").and_then(diff_status),
        "revertAll" => str_param(params, "draftId").and_then(revert_all),
        "commit" => str_param(params, "draftId").and_then(commit),
        _ => Err(format!("unknown method \"{method}\"")),
    };

    match result {
        Ok(value) => json!({"ok": true, "result": value}),
        Err(error) => json!({"ok": false, "error": error}),
    }
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params.get(key).and_then(|v| v.as_str()).ok_or_else(|| format!("missing \"{key}\""))
}

/// A path that must already exist, confirmed to be inside `root` via canonicalize (not a plain
/// prefix check — that's the part that actually resolves ".."/symlinks/case differences rather
/// than just string-comparing them away).
fn validate_existing(root: &Path, path: &str) -> Result<PathBuf, String> {
    let canonical_root = root.canonicalize().map_err(|e| format!("invalid root: {e}"))?;
    let canonical = Path::new(path).canonicalize().map_err(|e| format!("{path} does not exist: {e}"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!("{path} is outside the project root."));
    }
    Ok(canonical)
}

/// A path that must NOT exist yet (a create/rename/move target) — canonicalizes the parent
/// instead, since canonicalize itself requires the path to already be on disk.
fn validate_new(root: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    let file_name = candidate.file_name().ok_or_else(|| format!("{path} has no file name."))?;
    let parent = candidate.parent().ok_or_else(|| format!("{path} has no parent directory."))?;

    let canonical_root = root.canonicalize().map_err(|e| format!("invalid root: {e}"))?;
    let canonical_parent = parent.canonicalize().map_err(|e| format!("{} does not exist: {e}", parent.display()))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(format!("{path} is outside the project root."));
    }

    let full = canonical_parent.join(file_name);
    if full.exists() {
        return Err(format!("{} already exists.", full.display()));
    }
    Ok(full)
}

fn list_dir(root: &Path, path: &str) -> Result<Value, String> {
    let dir = validate_existing(root, path)?;
    if !dir.is_dir() {
        return Err(format!("{path} is not a directory."));
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("could not read {path}: {e}"))? {
        let entry = entry.map_err(|e| format!("could not read an entry in {path}: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(json!({"name": name, "isDir": is_dir}));
    }
    Ok(Value::Array(entries))
}

fn create_entry(root: &Path, path: &str, is_dir: bool) -> Result<Value, String> {
    let target = validate_new(root, path)?;
    let outcome = if is_dir { std::fs::create_dir(&target) } else { std::fs::write(&target, []) };
    outcome.map_err(|e| format!("could not create {}: {e}", target.display()))?;
    Ok(Value::Null)
}

fn rename_entry(root: &Path, path: &str, new_name: &str) -> Result<Value, String> {
    let source = validate_existing(root, path)?;
    let parent = source.parent().ok_or_else(|| format!("{path} has no parent directory."))?;
    let dest = parent.join(new_name);
    if dest.exists() {
        return Err(format!("{} already exists.", dest.display()));
    }
    std::fs::rename(&source, &dest).map_err(|e| format!("could not rename {}: {e}", source.display()))?;
    Ok(json!({"path": dest.display().to_string()}))
}

fn move_entry(root: &Path, source_path: &str, dest_dir: &str) -> Result<Value, String> {
    let source = validate_existing(root, source_path)?;
    let dest_dir = validate_existing(root, dest_dir)?;
    if !dest_dir.is_dir() {
        return Err(format!("{} is not a directory.", dest_dir.display()));
    }
    let file_name = source.file_name().ok_or_else(|| format!("{source_path} has no file name."))?;
    let dest = dest_dir.join(file_name);
    if dest.exists() {
        return Err(format!("{} already exists.", dest.display()));
    }
    std::fs::rename(&source, &dest).map_err(|e| format!("could not move {}: {e}", source.display()))?;
    Ok(json!({"path": dest.display().to_string()}))
}

fn copy_entry(root: &Path, source_path: &str, dest_dir: &str) -> Result<Value, String> {
    let source = validate_existing(root, source_path)?;
    let dest_dir = validate_existing(root, dest_dir)?;
    if !dest_dir.is_dir() {
        return Err(format!("{} is not a directory.", dest_dir.display()));
    }
    let file_name = source.file_name().ok_or_else(|| format!("{source_path} has no file name."))?;
    let dest = dest_dir.join(file_name);
    if dest.exists() {
        return Err(format!("{} already exists.", dest.display()));
    }

    if source.is_dir() {
        copy_dir_recursive(&source, &dest).map_err(|e| format!("could not copy {}: {e}", source.display()))?;
    } else {
        std::fs::copy(&source, &dest).map_err(|e| format!("could not copy {}: {e}", source.display()))?;
    }
    Ok(json!({"path": dest.display().to_string()}))
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

fn delete_entry(root: &Path, path: &str) -> Result<Value, String> {
    let target = validate_existing(root, path)?;
    let outcome = if target.is_dir() { std::fs::remove_dir_all(&target) } else { std::fs::remove_file(&target) };
    outcome.map_err(|e| format!("could not delete {}: {e}", target.display()))?;
    Ok(Value::Null)
}

/// Whole-project totals for the footer — folders/files/bytes under `root`, recursively. Symlinks
/// are skipped rather than followed (DirEntry::file_type() reports a symlink's own type, which is
/// neither is_dir() nor is_file(), so they simply don't match either branch below) — walking into
/// one could cycle back on itself, and a symlink's "real" size belongs to whatever it points at,
/// not to this project.
fn count_tree(root: &Path) -> Result<Value, String> {
    let mut totals = Totals::default();
    walk_count(root, &mut totals).map_err(|e| format!("could not walk {}: {e}", root.display()))?;
    Ok(json!({"files": totals.files, "folders": totals.folders, "bytes": totals.bytes}))
}

#[derive(Default)]
struct Totals {
    files: u64,
    folders: u64,
    bytes: u64,
}

fn walk_count(dir: &Path, totals: &mut Totals) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            totals.folders += 1;
            walk_count(&entry.path(), totals)?;
        } else if file_type.is_file() {
            totals.files += 1;
            totals.bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(())
}

// ---------- Draft Tool ----------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct DraftManifestEntry {
    index: usize,
    /// False for a path that didn't exist before this Draft's first-seen change to it — revert
    /// deletes the file in that case, rather than overwriting it with empty content.
    existed: bool,
}

type DraftManifest = HashMap<String, DraftManifestEntry>;

/// Every Draft's own scratch storage — this binary's sole concern, the host only ever wipes the
/// whole thing wholesale on launch (see lib.rs). Not configurable, not read from `params`: a fixed,
/// well-known location under the OS temp dir.
fn draft_scratch_root() -> PathBuf {
    std::env::temp_dir().join("lowarc-offshoot")
}

fn draft_dir(draft_id: &str) -> PathBuf {
    draft_scratch_root().join(draft_id)
}

fn draft_manifest_path(draft_id: &str) -> PathBuf {
    draft_dir(draft_id).join("manifest.json")
}

fn draft_snapshot_path(draft_id: &str, index: usize) -> PathBuf {
    draft_dir(draft_id).join("snapshots").join(format!("{index}.txt"))
}

fn read_draft_manifest(draft_id: &str) -> DraftManifest {
    std::fs::read_to_string(draft_manifest_path(draft_id)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn write_draft_manifest(draft_id: &str, manifest: &DraftManifest) -> Result<(), String> {
    let text = serde_json::to_string(manifest).map_err(|e| e.to_string())?;
    std::fs::write(draft_manifest_path(draft_id), text).map_err(|e| format!("could not save Draft state: {e}"))
}

/// Which Draft (if any) is currently open: a single well-known file, not per-draft, since this
/// UI only ever has one open at a time. Exists entirely so a page refresh (which wipes every bit
/// of the plugin iframe's own JS state, but neither this backend process nor its disk storage) has
/// something to ask "was a Draft actually open?" on reload. See getActiveDraft below. Restarting
/// the whole app still forgets it, same as everything else under scratch_root(): lib.rs's setup()
/// wipes that entire folder wholesale on every launch.
#[derive(Debug, Serialize, Deserialize)]
struct ActiveDraft {
    #[serde(rename = "draftId")]
    draft_id: String,
    label: String,
    description: String,
}

fn active_draft_path() -> PathBuf {
    draft_scratch_root().join("active.json")
}

/// Idempotent — opening the same draft_id twice (a double-click, a retry) just leaves any already-
/// captured baselines in place rather than losing them (though it DOES refresh the remembered
/// label/description, in case those were edited on a retry).
fn open_draft(draft_id: &str, label: &str, description: &str) -> Result<Value, String> {
    std::fs::create_dir_all(draft_dir(draft_id).join("snapshots")).map_err(|e| format!("could not open Draft: {e}"))?;
    if !draft_manifest_path(draft_id).is_file() {
        write_draft_manifest(draft_id, &DraftManifest::new())?;
    }
    let active = ActiveDraft { draft_id: draft_id.to_string(), label: label.to_string(), description: description.to_string() };
    // Best-effort: a failure to write the "resume after refresh" pointer shouldn't fail opening
    // the Draft itself, it just means a refresh won't be able to rediscover it.
    let _ = serde_json::to_string(&active).map(|text| std::fs::write(active_draft_path(), text));
    Ok(Value::Null)
}

/// The frontend's own "did I lose track of an open Draft?" check, called once on page load —
/// covers exactly the gap a webview refresh leaves (see ActiveDraft's own doc comment). None (not
/// an error) whenever nothing's tracked, which is the ordinary case outside a mid-Draft refresh.
fn get_active_draft() -> Result<Value, String> {
    let Ok(text) = std::fs::read_to_string(active_draft_path()) else {
        return Ok(Value::Null);
    };
    match serde_json::from_str::<ActiveDraft>(&text) {
        Ok(active) => Ok(json!({"draftId": active.draft_id, "label": active.label, "description": active.description})),
        Err(_) => Ok(Value::Null),
    }
}

/// First-write-wins per path: a second (or third, ...) save of the same file after its baseline
/// is already captured is a no-op here, which is exactly the point: the ORIGINAL content is what
/// Revert needs to restore, not whatever the file looked like a moment before the most recent save.
fn capture_baseline(draft_id: &str, path: &str, previous_content: Option<&str>) -> Result<Value, String> {
    let mut manifest = read_draft_manifest(draft_id);
    if manifest.contains_key(path) {
        return Ok(Value::Null);
    }

    let index = manifest.len();
    let existed = previous_content.is_some();
    std::fs::write(draft_snapshot_path(draft_id, index), previous_content.unwrap_or("")).map_err(|e| format!("could not capture {path}: {e}"))?;
    manifest.insert(path.to_string(), DraftManifestEntry { index, existed });
    write_draft_manifest(draft_id, &manifest)?;
    Ok(Value::Null)
}

/// Re-reads every tracked path's CURRENT disk content fresh on every call (never cached), same
/// "the backend is the source of truth, re-fetched on demand" convention list_dir/count_tree above
/// already follow. A path that's been deleted since its baseline was captured reads as empty
/// current content (a full removal shows as every original line removed), rather than erroring the
/// whole status call over one missing file.
fn diff_status(draft_id: &str) -> Result<Value, String> {
    let manifest = read_draft_manifest(draft_id);
    let mut status = serde_json::Map::new();
    for (path, entry) in &manifest {
        let baseline = if entry.existed { std::fs::read_to_string(draft_snapshot_path(draft_id, entry.index)).unwrap_or_default() } else { String::new() };
        let current = std::fs::read_to_string(path).unwrap_or_default();
        let (added, removed) = count_line_changes(&baseline, &current);
        status.insert(path.clone(), json!({"added": added, "removed": removed}));
    }
    Ok(Value::Object(status))
}

fn count_line_changes(baseline: &str, current: &str) -> (usize, usize) {
    let diff = similar::TextDiff::from_lines(baseline, current);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// Restores every tracked path to its captured baseline (deleting a path that didn't exist before
/// the Draft touched it, rather than leaving it behind), then discards the Draft's own storage.
fn revert_all(draft_id: &str) -> Result<Value, String> {
    let manifest = read_draft_manifest(draft_id);
    for (path, entry) in &manifest {
        if entry.existed {
            let baseline = std::fs::read_to_string(draft_snapshot_path(draft_id, entry.index)).map_err(|e| format!("could not read the original content of {path}: {e}"))?;
            std::fs::write(path, baseline).map_err(|e| format!("could not restore {path}: {e}"))?;
        } else {
            // Best-effort: if it's already gone (the user deleted it themselves), nothing to undo.
            let _ = std::fs::remove_file(path);
        }
    }
    discard_draft(draft_id)
}

/// Current disk content is already what it should be. Commit's only job is forgetting the
/// baselines so they stop being tracked.
fn commit(draft_id: &str) -> Result<Value, String> {
    discard_draft(draft_id)
}

fn discard_draft(draft_id: &str) -> Result<Value, String> {
    let _ = std::fs::remove_dir_all(draft_dir(draft_id));
    // Only clear the "resume after refresh" pointer if it was actually pointing at THIS draft —
    // defensive more than load-bearing (only one Draft is ever open at a time in this UI today),
    // but a stale unrelated pointer shouldn't be silently erased just because some other draft_id
    // got cleaned up.
    if let Ok(text) = std::fs::read_to_string(active_draft_path()) {
        if serde_json::from_str::<ActiveDraft>(&text).ok().map(|a| a.draft_id).as_deref() == Some(draft_id) {
            let _ = std::fs::remove_file(active_draft_path());
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    // open_draft/commit/revert_all all touch the ONE shared active_draft_path() now (not
    // anything keyed by draft_id). Cargo test runs tests in parallel by default, so without this
    // every test calling any of the three would otherwise stomp on another's active-pointer
    // expectations, which shows up as intermittent failures.
    // parking_lot, not std::sync: a std Mutex poisons permanently on a panicking test, which
    // would otherwise cascade an unrelated assertion failure into every other test sharing this
    // lock; parking_lot has no poisoning, so one failure stays exactly that.
    static ACTIVE_DRAFT_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_file_explorer_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn call(root: &Path, method: &str, mut params: Value) -> Value {
        params.as_object_mut().unwrap().insert("root".to_string(), json!(root.to_string_lossy()));
        handle(method, &params)
    }

    fn ok(reply: &Value) -> bool {
        reply.get("ok").and_then(|v| v.as_bool()) == Some(true)
    }

    #[test]
    fn list_dir_returns_immediate_children_only() {
        let root = temp_dir("list");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("nested.txt"), "").unwrap();
        std::fs::write(root.join("top.txt"), "").unwrap();

        let reply = call(&root, "listDir", json!({"path": root.to_string_lossy()}));
        assert!(ok(&reply), "{reply:?}");
        let entries = reply["result"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "should list only immediate children, not nested.txt");
        assert!(entries.iter().any(|e| e["name"] == "sub" && e["isDir"] == true));
        assert!(entries.iter().any(|e| e["name"] == "top.txt" && e["isDir"] == false));
    }

    #[test]
    fn create_file_and_folder_then_reject_a_second_attempt() {
        let root = temp_dir("create");
        let file_path = root.join("note.txt");
        let ok_reply = call(&root, "createFile", json!({"path": file_path.to_string_lossy()}));
        assert!(ok(&ok_reply), "{ok_reply:?}");
        assert!(file_path.is_file());

        let dup_reply = call(&root, "createFile", json!({"path": file_path.to_string_lossy()}));
        assert!(!ok(&dup_reply), "creating the same file twice must fail, not silently overwrite");

        let dir_path = root.join("sub");
        let dir_reply = call(&root, "createFolder", json!({"path": dir_path.to_string_lossy()}));
        assert!(ok(&dir_reply), "{dir_reply:?}");
        assert!(dir_path.is_dir());
    }

    #[test]
    fn rename_moves_within_the_same_directory_and_rejects_a_collision() {
        let root = temp_dir("rename");
        let original = root.join("a.txt");
        std::fs::write(&original, "content").unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();

        let reply = call(&root, "rename", json!({"path": original.to_string_lossy(), "newName": "renamed.txt"}));
        assert!(ok(&reply), "{reply:?}");
        assert!(root.join("renamed.txt").is_file());
        assert!(!original.exists());

        let collision = call(&root, "rename", json!({"path": root.join("renamed.txt").to_string_lossy(), "newName": "b.txt"}));
        assert!(!ok(&collision), "renaming onto an existing file must fail, not overwrite it");
    }

    #[test]
    fn move_relocates_into_a_destination_directory() {
        let root = temp_dir("move");
        let source = root.join("file.txt");
        std::fs::write(&source, "content").unwrap();
        let dest_dir = root.join("dest");
        std::fs::create_dir_all(&dest_dir).unwrap();

        let reply = call(&root, "move", json!({"sourcePath": source.to_string_lossy(), "destDir": dest_dir.to_string_lossy()}));
        assert!(ok(&reply), "{reply:?}");
        assert!(dest_dir.join("file.txt").is_file());
        assert!(!source.exists());
    }

    #[test]
    fn copy_duplicates_a_folder_recursively_and_leaves_the_source_intact() {
        let root = temp_dir("copy");
        let source_dir = root.join("src");
        std::fs::create_dir_all(source_dir.join("nested")).unwrap();
        std::fs::write(source_dir.join("a.txt"), "a").unwrap();
        std::fs::write(source_dir.join("nested").join("b.txt"), "b").unwrap();
        let dest_dir = root.join("dest");
        std::fs::create_dir_all(&dest_dir).unwrap();

        let reply = call(&root, "copy", json!({"sourcePath": source_dir.to_string_lossy(), "destDir": dest_dir.to_string_lossy()}));
        assert!(ok(&reply), "{reply:?}");
        assert!(dest_dir.join("src").join("a.txt").is_file());
        assert!(dest_dir.join("src").join("nested").join("b.txt").is_file());
        assert!(source_dir.exists(), "copy must leave the source in place, unlike move");
    }

    #[test]
    fn delete_removes_a_file_and_recursively_removes_a_folder() {
        let root = temp_dir("delete");
        let file = root.join("gone.txt");
        std::fs::write(&file, "").unwrap();
        let dir = root.join("gone_dir");
        std::fs::create_dir_all(dir.join("nested")).unwrap();

        assert!(ok(&call(&root, "deletePath", json!({"path": file.to_string_lossy()}))));
        assert!(!file.exists());

        assert!(ok(&call(&root, "deletePath", json!({"path": dir.to_string_lossy()}))));
        assert!(!dir.exists());
    }

    #[test]
    fn every_operation_rejects_a_path_outside_the_root() {
        let root = temp_dir("outside_root");
        let outside = temp_dir("outside_target");
        let outside_file = outside.join("elsewhere.txt");
        std::fs::write(&outside_file, "").unwrap();

        let list_reply = call(&root, "listDir", json!({"path": outside.to_string_lossy()}));
        assert!(!ok(&list_reply), "listDir must refuse a directory outside the project root");

        let delete_reply = call(&root, "deletePath", json!({"path": outside_file.to_string_lossy()}));
        assert!(!ok(&delete_reply), "deletePath must refuse a file outside the project root");
        assert!(outside_file.exists(), "the outside file must survive an attempted delete that should have been rejected");
    }

    #[test]
    fn count_tree_totals_files_folders_and_bytes_recursively() {
        let root = temp_dir("count");
        std::fs::write(root.join("top.txt"), "abcde").unwrap(); // 5 bytes
        std::fs::create_dir_all(root.join("sub").join("nested")).unwrap();
        std::fs::write(root.join("sub").join("mid.txt"), "ab").unwrap(); // 2 bytes
        std::fs::write(root.join("sub").join("nested").join("deep.txt"), "a").unwrap(); // 1 byte

        let reply = call(&root, "countTree", json!({}));
        assert!(ok(&reply), "{reply:?}");
        assert_eq!(reply["result"]["files"], 3);
        assert_eq!(reply["result"]["folders"], 2, "sub and sub/nested, not root itself");
        assert_eq!(reply["result"]["bytes"], 8);
    }

    #[test]
    fn unknown_method_is_a_visible_error() {
        let root = temp_dir("unknown_method");
        let reply = call(&root, "doSomethingUnsupported", json!({}));
        assert!(!ok(&reply));
        assert!(reply["error"].as_str().unwrap().contains("unknown method"));
    }

    // ---------- Draft Tool ----------
    // Every test gets its own draft_id (not its own scratch root — that's a fixed, real OS temp
    // path this binary always uses) so parallel `cargo test` runs never collide with each other.

    fn unique_draft_id(name: &str) -> String {
        format!("test_{name}_{}", std::process::id())
    }

    fn cleanup_draft(draft_id: &str) {
        let _ = std::fs::remove_dir_all(draft_dir(draft_id));
    }

    #[test]
    fn open_draft_is_idempotent_and_starts_with_an_empty_manifest() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("open");
        cleanup_draft(&draft_id);

        assert!(open_draft(&draft_id, "Test Draft", "").is_ok());
        assert!(read_draft_manifest(&draft_id).is_empty());
        assert!(open_draft(&draft_id, "Test Draft", "").is_ok(), "opening the same draft twice must not error");

        cleanup_draft(&draft_id);
    }

    #[test]
    fn capture_baseline_is_first_write_wins() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("first_write_wins");
        cleanup_draft(&draft_id);
        open_draft(&draft_id, "Test Draft", "").unwrap();

        capture_baseline(&draft_id, "C:/fake/a.txt", Some("original")).unwrap();
        capture_baseline(&draft_id, "C:/fake/a.txt", Some("edited once")).unwrap();

        let manifest = read_draft_manifest(&draft_id);
        let entry = manifest.get("C:/fake/a.txt").unwrap();
        assert!(entry.existed);
        let stored = std::fs::read_to_string(draft_snapshot_path(&draft_id, entry.index)).unwrap();
        assert_eq!(stored, "original");

        cleanup_draft(&draft_id);
    }

    #[test]
    fn capture_baseline_records_a_brand_new_file_as_not_existed() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("new_file");
        cleanup_draft(&draft_id);
        open_draft(&draft_id, "Test Draft", "").unwrap();

        capture_baseline(&draft_id, "C:/fake/new.txt", None).unwrap();
        let manifest = read_draft_manifest(&draft_id);
        assert!(!manifest.get("C:/fake/new.txt").unwrap().existed);

        cleanup_draft(&draft_id);
    }

    #[test]
    fn diff_status_counts_added_and_removed_lines() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("diff_status");
        cleanup_draft(&draft_id);
        open_draft(&draft_id, "Test Draft", "").unwrap();

        let root = temp_dir("diff_status_file");
        let file = root.join("diff_status.txt");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        capture_baseline(&draft_id, &file.to_string_lossy(), Some("one\ntwo\nthree\n")).unwrap();
        std::fs::write(&file, "one\ntwo\nfour\nfive\n").unwrap();

        let reply = diff_status(&draft_id).unwrap();
        let counts = &reply[file.to_string_lossy().as_ref()];
        assert_eq!(counts["removed"], 1, "\"three\" was removed");
        assert_eq!(counts["added"], 2, "\"four\" and \"five\" were added");

        cleanup_draft(&draft_id);
    }

    #[test]
    fn revert_all_restores_original_content_and_deletes_a_newly_created_file() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("revert");
        cleanup_draft(&draft_id);
        open_draft(&draft_id, "Test Draft", "").unwrap();

        let root = temp_dir("revert_files");
        let existing = root.join("revert_existing.txt");
        std::fs::write(&existing, "original").unwrap();
        capture_baseline(&draft_id, &existing.to_string_lossy(), Some("original")).unwrap();
        std::fs::write(&existing, "edited").unwrap();

        let created = root.join("revert_created.txt");
        std::fs::write(&created, "brand new").unwrap();
        capture_baseline(&draft_id, &created.to_string_lossy(), None).unwrap();

        assert!(revert_all(&draft_id).is_ok());
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "original");
        assert!(!created.exists(), "a file that didn't exist before the Draft must be deleted, not left empty");
        assert!(!draft_dir(&draft_id).exists(), "revert must discard the Draft's own storage when it's done");
    }

    #[test]
    fn commit_leaves_current_content_untouched_and_clears_storage() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("commit");
        cleanup_draft(&draft_id);
        open_draft(&draft_id, "Test Draft", "").unwrap();

        let root = temp_dir("commit_file");
        let file = root.join("commit.txt");
        std::fs::write(&file, "original").unwrap();
        capture_baseline(&draft_id, &file.to_string_lossy(), Some("original")).unwrap();
        std::fs::write(&file, "edited, kept").unwrap();

        assert!(commit(&draft_id).is_ok());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited, kept");
        assert!(!draft_dir(&draft_id).exists());
    }

    #[test]
    fn get_active_draft_reflects_the_currently_open_draft() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let draft_id = unique_draft_id("active_reflects");
        cleanup_draft(&draft_id);

        open_draft(&draft_id, "My Feature", "Trying something out").unwrap();
        let active = get_active_draft().unwrap();
        assert_eq!(active["draftId"], draft_id);
        assert_eq!(active["label"], "My Feature");
        assert_eq!(active["description"], "Trying something out");

        commit(&draft_id).unwrap();
    }

    #[test]
    fn get_active_draft_is_cleared_by_commit_and_by_revert() {
        let _guard = ACTIVE_DRAFT_LOCK.lock();
        let committed_id = unique_draft_id("active_cleared_commit");
        cleanup_draft(&committed_id);
        open_draft(&committed_id, "Commit Me", "").unwrap();
        assert_eq!(get_active_draft().unwrap()["draftId"], committed_id);
        commit(&committed_id).unwrap();
        assert!(get_active_draft().unwrap().is_null(), "commit must clear the active-draft pointer");

        let reverted_id = unique_draft_id("active_cleared_revert");
        cleanup_draft(&reverted_id);
        open_draft(&reverted_id, "Revert Me", "").unwrap();
        assert_eq!(get_active_draft().unwrap()["draftId"], reverted_id);
        revert_all(&reverted_id).unwrap();
        assert!(get_active_draft().unwrap().is_null(), "revert must clear the active-draft pointer");
    }
}
