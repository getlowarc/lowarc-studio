// Monaco's own web workers (language services for json/css/html/ts, plus the generic editor
// worker) are spun up via a Worker(blob-url) that immediately importScripts() the real worker
// file with a path relative to *this* document — Chromium resolves that relative path against the
// creating document's URL, not the blob's, so a bare relative path here is enough; no need to
// hardcode an absolute origin. Requires the host's CSP to allow `worker-src 'self' blob:`.
const WORKER_BY_LABEL = {
  json: "vs/language/json/json.worker.js",
  css: "vs/language/css/css.worker.js",
  scss: "vs/language/css/css.worker.js",
  less: "vs/language/css/css.worker.js",
  html: "vs/language/html/html.worker.js",
  handlebars: "vs/language/html/html.worker.js",
  razor: "vs/language/html/html.worker.js",
  typescript: "vs/language/typescript/ts.worker.js",
  javascript: "vs/language/typescript/ts.worker.js",
};

self.MonacoEnvironment = {
  getWorkerUrl(_moduleId, label) {
    const target = WORKER_BY_LABEL[label] || "vs/editor/editor.worker.js";
    return URL.createObjectURL(new Blob([`importScripts(${JSON.stringify(target)});`], { type: "text/javascript" }));
  },
};

// Broad but not exhaustive — anything Monaco can highlight is fair game to add here later. ".uc"
// (LowArc's own language) is registered with no tokenizer yet, further down, purely so it's
// labelled correctly instead of silently falling back to plaintext; a real Monarch grammar for it
// is a separate, later task.
const LANGUAGE_BY_EXT = {
  ".js": "javascript", ".jsx": "javascript", ".mjs": "javascript", ".cjs": "javascript",
  ".ts": "typescript", ".tsx": "typescript",
  ".json": "json", ".jsonc": "json",
  ".html": "html", ".htm": "html",
  ".css": "css", ".scss": "scss", ".less": "less",
  ".md": "markdown", ".markdown": "markdown",
  ".py": "python",
  ".rs": "rust",
  ".go": "go",
  ".java": "java",
  ".c": "c", ".h": "c",
  ".cpp": "cpp", ".hpp": "cpp", ".cc": "cpp",
  ".cs": "csharp",
  ".php": "php",
  ".rb": "ruby",
  ".sh": "shell", ".bash": "shell",
  ".yml": "yaml", ".yaml": "yaml",
  ".xml": "xml",
  ".sql": "sql",
  ".lua": "lua",
  ".swift": "swift",
  ".kt": "kotlin",
  ".dart": "dart",
  ".ps1": "powershell",
  ".bat": "bat",
  ".ini": "ini",
  ".uc": "uc",
  ".txt": "plaintext",
};

function languageForPath(path) {
  const dot = path.lastIndexOf(".");
  const ext = dot === -1 ? "" : path.slice(dot).toLowerCase();
  return LANGUAGE_BY_EXT[ext] || "plaintext";
}

require.config({ paths: { vs: "vs" } });

// Plugin settings (Settings > Plugins > Monaco, see plugin.json's `settings` declaration) are
// always stored/returned as plain strings — Settings.plugin_settings is a
// HashMap<String, HashMap<String, String>> with no per-field type on the Rust side — so parsing
// and defaulting each one is this plugin's own job, same as every other plugin that reads its own
// settings this way. Fetched in parallel with the (much slower) editor.main module load itself,
// not after it, so this never adds to the real bottleneck.
function boolSetting(settings, key, fallback) {
  if (settings[key] === "true") return true;
  if (settings[key] === "false") return false;
  return fallback;
}
function numSetting(settings, key, fallback) {
  const n = Number(settings[key]);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

Promise.all([new Promise((resolve) => require(["vs/editor/editor.main"], resolve)), window.lowarc.getSettings()]).then(([, settings]) => {
  settings = settings || {};
  monaco.languages.register({ id: "uc" });
  monaco.editor.setTheme("vs-dark");

  const editor = monaco.editor.create(document.getElementById("container"), {
    value: "",
    language: "plaintext",
    automaticLayout: true,
    minimap: { enabled: boolSetting(settings, "minimap", true) },
    lineNumbers: boolSetting(settings, "lineNumbers", true) ? "on" : "off",
    wordWrap: boolSetting(settings, "wordWrap", false) ? "on" : "off",
    fontSize: numSetting(settings, "fontSize", 14),
    tabSize: numSetting(settings, "tabSize", 4),
    insertSpaces: boolSetting(settings, "insertSpaces", true),
    // Monaco's own right-click menu is a second, separate context-menu system living outside the
    // app's own (every other plugin either has none or, like file explorer, builds its own via
    // window.lowarc.showMenu() into the host's un-clippable floating-menu) — and its default
    // "Command Palette" entry is one of the two ways Monaco's native palette overlay was reachable
    // even after the keybindings below were overridden, since a context-menu click calls the
    // action directly rather than going through keybinding dispatch.
    contextmenu: false,
  });

  // This instance can now outlive any one file — it's mounted once per (editor group, Monaco) pair
  // and stays alive as long as ANY file that group opened through Monaco is still open, the same
  // "one iframe, many documents" shape the Terminal plugin already uses for multiple terminal tabs
  // (see its own file header). path -> {model, viewState, savedVersionId}. Nothing here is read
  // off the URL any more — every file this instance shows arrives entirely through
  // lowarc:openFile/activateFile/closeFile messages over its lifetime, matching the host's own
  // contract in plugin_assets.rs.
  const docs = new Map();
  let activePath = null;
  let saving = false;
  // A doc's own model.setValue()/createModel() fires onDidChangeContent, which would otherwise
  // report a brand-new file as dirty against its own starting content — this guard is what keeps
  // "just opened" from reading as "already edited."
  let loading = false;

  // The baseline to compare against for dirty-tracking — Monaco's own "back to saved" detection
  // (undoing past every edit) rather than a plain string-equality diff, so redo/undo round-trips
  // back to a clean state clear the dirty flag exactly the way VS Code's own editor does. Kept
  // per-doc (in docs, not one shared variable) since each open file needs its own baseline.
  function updateDirty(path) {
    if (loading) return;
    const doc = docs.get(path);
    if (!doc) return;
    const dirty = doc.model.getAlternativeVersionId() !== doc.savedVersionId;
    window.lowarc.markDirty(path, dirty);
  }

  // onDidChangeMarkers is a static event on monaco.editor (fires for marker changes on any model
  // in the process, not just the active one) rather than something scoped to one model — only a
  // language with real diagnostics wired up reports anything here (json's schema validation,
  // typescript/javascript's checker); a Monarch-tokenizer-only language (most of LANGUAGE_BY_EXT)
  // never fires this at all, so "red for errors" is honest about only covering what Monaco can
  // actually validate, not a promise of universal linting. Only the ACTIVE file's tab can show
  // this anyway (markErrors needs a path, and there's only one file visibly focused at a time), so
  // a marker change on a backgrounded file's model is silently ignored until it's activated again.
  monaco.editor.onDidChangeMarkers((uris) => {
    if (!activePath) return;
    const doc = docs.get(activePath);
    if (!doc) return;
    if (!uris.some((uri) => uri.toString() === doc.model.uri.toString())) return;
    const hasErrors = monaco.editor.getModelMarkers({ resource: doc.model.uri }).some((m) => m.severity === monaco.MarkerSeverity.Error);
    window.lowarc.markErrors(activePath, hasErrors);
  });

  // A path already in `docs` means it's already open here — nothing to do (lowarc:activateFile is
  // the separate "make this one visible" step; re-sending lowarc:openFile for an already-open path
  // is a normal no-op, not an error, since the host doesn't track per-instance state itself).
  window.lowarc.on("lowarc:openFile", (payload) => {
    if (!payload || typeof payload.path !== "string" || docs.has(payload.path)) return;
    const model = monaco.editor.createModel(payload.contents, languageForPath(payload.path), monaco.Uri.file(payload.path));
    const doc = { model, viewState: null, savedVersionId: model.getAlternativeVersionId() };
    docs.set(payload.path, doc);
    model.onDidChangeContent(() => updateDirty(payload.path));
  });

  window.lowarc.on("lowarc:activateFile", (payload) => {
    const doc = payload && docs.get(payload.path);
    if (!doc) return;
    // Saving/restoring view state (scroll position, cursor, folds) around the swap is what makes
    // switching tabs feel like coming back to the same place — the model itself already carries
    // undo history for free, but a model has no notion of "where the viewport was."
    if (activePath && docs.has(activePath)) {
      docs.get(activePath).viewState = editor.saveViewState();
    }
    activePath = payload.path;
    loading = true;
    editor.setModel(doc.model);
    loading = false;
    if (doc.viewState) editor.restoreViewState(doc.viewState);
    editor.focus();
  });

  window.lowarc.on("lowarc:closeFile", (payload) => {
    const doc = payload && docs.get(payload.path);
    if (!doc) return;
    docs.delete(payload.path);
    if (activePath === payload.path) activePath = null;
    doc.model.dispose();
  });

  // The host's one HOST-initiated request (see requestPluginContent() in editor.html, used when
  // moving a file to the other editor group) — replies with this instance's actual current text
  // for that path, unsaved edits included, rather than making the host re-read the file from disk
  // and silently discard them.
  window.lowarc.on("lowarc:getContent", (payload) => {
    if (!payload) return;
    const doc = docs.get(payload.path);
    window.parent.postMessage({ type: "hostRequestReply", replyId: payload.replyId, content: doc ? doc.model.getValue() : null }, "*");
  });

  // A plugin that isn't this file's own viewer asking for a specific line's text to change (the
  // Outline plugin, editing a value it parsed out). editor.executeEdits(), not model.applyEdits()
  // or model.setValue() — this is the one that actually integrates with the editor's own
  // undo-redo controller the same way a real keystroke does; a bare model-level edit still lands
  // in the model's own undo stack, but doesn't reliably wire up to Ctrl+Z the way a genuine editor
  // operation does (confirmed missing live: an Outline edit couldn't be undone at all before this
  // was executeEdits). Only valid while this file is the one actually showing in the editor —
  // executeEdits acts on whatever model is CURRENTLY SET, so editing a backgrounded file through
  // it would silently corrupt whichever OTHER file happens to be on screen; falls back to a plain
  // model edit for that case; genuinely rare in practice, since Outline only ever shows the active
  // file's own symbols to begin with.
  window.lowarc.on("lowarc:applyLineEdit", (payload) => {
    if (!payload || typeof payload.line !== "number" || typeof payload.text !== "string") return;
    const doc = docs.get(payload.path);
    if (!doc) return;
    const line = payload.line;
    if (line < 1 || line > doc.model.getLineCount()) return;
    const range = new monaco.Range(line, 1, line, doc.model.getLineMaxColumn(line));
    if (payload.path === activePath) {
      editor.executeEdits("outline", [{ range, text: payload.text }]);
    } else {
      doc.model.applyEdits([{ range, text: payload.text }]);
    }
  });

  async function save() {
    if (saving || !activePath) return;
    const path = activePath;
    const doc = docs.get(path);
    if (!doc) return;
    saving = true;
    try {
      await window.lowarc.saveFile(path, doc.model.getValue());
      doc.savedVersionId = doc.model.getAlternativeVersionId();
      updateDirty(path);
    } catch (err) {
      // The host already surfaces a toast for a failed write — nothing useful for this iframe
      // to additionally show, but the dirty flag must NOT be cleared on a failed save.
    } finally {
      saving = false;
    }
  }

  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, save);

  // The host's File > Save menu item / Ctrl+S (see editor.html) — a click happens in the host
  // document, not this iframe, so it can't reach the keybinding above; this is the same save()
  // reached a different way.
  window.lowarc.on("lowarc:requestSave", save);

  // The host's Edit menu (Undo/Redo/Cut/Copy/Find/Replace — everything except Paste) and any
  // matching host-level shortcut — editor.trigger(), not editor.getAction(id).run() (what
  // lowarc:runCommand below uses), because Undo/Redo aren't registered Actions, only Commands;
  // trigger() dispatches to either kind, so one handler covers all of them.
  window.lowarc.on("lowarc:editorCommand", (payload) => {
    if (!payload || typeof payload.command !== "string") return;
    editor.trigger("host-menu", payload.command, null);
  });

  // Paste specifically: the host already read the OS clipboard itself (this iframe's own
  // navigator.clipboard.readText() is what's actually blocked — see editor.html's paste handler
  // for the full reasoning) and hands over plain text to drop in at the current selection.
  // executeEdits(), not insertText/applyEdits, for the same undo-integration reason every other
  // real edit in this file uses it.
  window.lowarc.on("lowarc:insertText", (payload) => {
    if (!payload || typeof payload.text !== "string" || !activePath) return;
    editor.executeEdits("host-paste", [{ range: editor.getSelection(), text: payload.text }]);
    editor.focus();
  });

  // Monaco ships its own command palette (F1 / Ctrl+Shift+P), a second, separate "list of every
  // action" surface competing with the host's own — reported directly by the person building this
  // app as something they want living in ONE place, not two. Feeding Monaco's real action list into
  // the host's palette (rather than a hand-picked subset) via window.lowarc.setCommands() gets
  // everything Monaco can actually do into the same list as everything else; overriding these two
  // keybindings to no-ops (same trick already used above for Ctrl+S) removes Monaco's own overlay
  // so there's no second competing surface left to open.
  // Overriding just the F1/Ctrl+Shift+P keybindings (still done below, as further insurance)
  // wasn't enough on its own — Monaco's own keybinding for its built-in quickCommand action
  // apparently still won that resolution. Neutering the action's own .run() is the one place
  // every trigger path (keybinding dispatch, a context-menu click, anything calling
  // editor.getAction(id).run() directly) actually goes through, so this is the real kill switch.
  const quickCommandAction = editor.getAction("editor.action.quickCommand");
  if (quickCommandAction) quickCommandAction.run = async () => {};

  // Runs once per editor instance, on creation — now genuinely once per (editor group, Monaco)
  // pair rather than once per opened file, since this instance persists across every file it
  // shows instead of being recreated per open. Monaco's own action list doesn't depend on which
  // file is active anyway, so re-sending it on every file open was always redundant (the host
  // replaces this plugin's previous registration rather than accumulating duplicates — see
  // setDynamicPluginCommands in editor.html); this just also removes the redundancy itself,
  // rather than merely being harmless underneath it.
  function refreshCommands() {
    const commands = editor.getSupportedActions()
      .filter((a) => a.label && a.id !== "editor.action.quickCommand")
      .map((a) => ({ id: a.id, label: a.label }));
    window.lowarc.setCommands(commands);
  }
  window.lowarc.on("lowarc:runCommand", (payload) => {
    const action = payload && editor.getAction(payload.commandId);
    if (action) action.run();
  });
  editor.addCommand(monaco.KeyCode.F1, () => {});
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyP, () => {});
  refreshCommands();
});
