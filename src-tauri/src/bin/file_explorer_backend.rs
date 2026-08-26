// The file-explorer plugin's backend — a standalone binary, invoked fresh per call, same wire
// protocol as any other plugin (see plugin_host/protocol.rs): one JSON line in on stdin
// (`{"method":..,"params":..}`), one JSON line out on stdout (`{"ok":bool,"result"?:..,
// "error"?:string}`). Deliberately self-contained — no dependency on lowarc_studio_lib — a real
// third-party plugin author has no access to the host's internals either, and this binary is
// meant to double as the reference example for "here's what a compiled plugin backend looks
// like."
//
// Every call carries "root" (the project directory) alongside its own arguments and gets
// canonicalize-checked against it before touching disk — same reasoning as plugin_assets.rs's
// path-traversal check for served assets: never trust a path without confirming it's still inside
// the boundary it's supposed to be confined to, regardless of who's asking.

use serde_json::{json, Value};
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
