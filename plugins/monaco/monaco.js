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

require(["vs/editor/editor.main"], () => {
  monaco.languages.register({ id: "uc" });
  monaco.editor.setTheme("vs-dark");

  const editor = monaco.editor.create(document.getElementById("container"), {
    value: "",
    language: "plaintext",
    automaticLayout: true,
    minimap: { enabled: true },
  });

  // The path this instance is showing — read synchronously off the iframe's own URL fragment
  // (see the file-explorer plugin's explorer.js for the same "fragment, not a race-prone
  // postMessage" reasoning), NOT the fragment's own contents: the actual file text always
  // arrives afterward via the lowarc:openFile emit, once the host has read it and this iframe
  // has finished loading — a sandboxed viewer iframe has no filesystem access of its own.
  const path = new URLSearchParams(window.location.hash.replace(/^#/, "")).get("path");

  // The baseline to compare against for dirty-tracking — Monaco's own "back to saved" detection
  // (undoing past every edit) rather than a plain string-equality diff, so redo/undo round-trips
  // back to a clean state clear the dirty flag exactly the way VS Code's own editor does.
  let savedVersionId = null;
  let saving = false;
  // model.setValue() below fires onDidChangeContent synchronously, before savedVersionId can be
  // set to match — without this guard, loading a file would immediately (and wrongly) report
  // itself dirty, since the change fires against the *old* (empty) baseline a line too early.
  let loading = false;

  function updateDirty() {
    if (loading) return;
    const dirty = editor.getModel().getAlternativeVersionId() !== savedVersionId;
    window.lowarc.markDirty(dirty);
  }

  // onDidChangeMarkers is a static event on monaco.editor (fires for marker changes on any model
  // in the process, not just this one) rather than something scoped to our own model — only a
  // language with real diagnostics wired up reports anything here (json's schema validation,
  // typescript/javascript's checker); a Monarch-tokenizer-only language (most of LANGUAGE_BY_EXT)
  // never fires this at all, so "red for errors" is honest about only covering what Monaco can
  // actually validate, not a promise of universal linting.
  monaco.editor.onDidChangeMarkers((uris) => {
    const model = editor.getModel();
    if (!uris.some((uri) => uri.toString() === model.uri.toString())) return;
    const hasErrors = monaco.editor.getModelMarkers({ resource: model.uri }).some((m) => m.severity === monaco.MarkerSeverity.Error);
    window.lowarc.markErrors(hasErrors);
  });

  window.lowarc.on("lowarc:openFile", (payload) => {
    if (!payload || payload.path !== path) return;
    const model = editor.getModel();
    loading = true;
    monaco.editor.setModelLanguage(model, languageForPath(path));
    model.setValue(payload.contents);
    loading = false;
    savedVersionId = model.getAlternativeVersionId();
    editor.focus();
  });

  editor.getModel().onDidChangeContent(updateDirty);

  async function save() {
    if (saving || !path) return;
    saving = true;
    try {
      await window.lowarc.saveFile(path, editor.getValue());
      savedVersionId = editor.getModel().getAlternativeVersionId();
      updateDirty();
    } catch (err) {
      // The host already surfaces a toast for a failed write — nothing useful for this iframe
      // to additionally show, but the dirty flag must NOT be cleared on a failed save.
    } finally {
      saving = false;
    }
  }

  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, save);
});
