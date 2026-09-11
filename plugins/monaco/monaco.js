// Monaco's own web workers (language services for json/css/html/ts, plus the generic editor
// worker) are spun up via a Worker(blob-url) that immediately importScripts() the real worker
// file with a path relative to *this* document: Chromium resolves that relative path against the
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

// Broad but not exhaustive: anything Monaco can highlight is fair game to add here later. Only
// languages Monaco actually ships belong here; registering an id Monaco has no tokenizer for buys
// nothing over the plaintext fallback it would get anyway.
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
  ".txt": "plaintext",
};

function languageForPath(path) {
  const dot = path.lastIndexOf(".");
  const ext = dot === -1 ? "" : path.slice(dot).toLowerCase();
  return LANGUAGE_BY_EXT[ext] || "plaintext";
}

require.config({ paths: { vs: "vs" } });

// Plugin settings (Settings > Plugins > Monaco, see plugin.json's `settings` declaration) are
// always stored/returned as plain strings: Settings.plugin_settings is a
// HashMap<String, HashMap<String, String>> with no per-field type on the Rust side, so parsing
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

// Monaco does NOT read the app's CSS variables; it owns its own theme registry. Its theming splits
// in two, and only one half is ours:
//
//   rules   54 syntax token scopes: colours the WORDS
//   colors  431 ids: colours the FURNITURE
//
// `rules` stays empty, permanently. The line: if it would still make sense with the editor empty it
// belongs to LowArc, and if it only means something because there is code on screen it belongs to
// Monaco. Keywords and squiggles are Monaco's; a background, gutter, cursor and find box are not.
// Remapping syntax would also read worse, since the palette has four chromatic hues against
// syntax's eight-plus roles, and would put danger-red on keywords.
//
// So this inherits whichever built-in matches the app's ground, which brings all 54 rules with it,
// and overrides about 30 furniture colours. Anything unlisted falls back, so the map goes stale
// rather than broken as Monaco adds ids.
function luminanceOfHex(hex) {
  const m = /^#?([0-9a-f]{6})$/i.exec((hex || "").trim());
  if (!m) return 0;
  const n = parseInt(m[1], 16);
  return (0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
}

function applyEditorTheme() {
  const css = getComputedStyle(document.documentElement);
  const token = (name, fallback) => {
    const value = (css.getPropertyValue(name) || "").trim();
    // Monaco rejects anything that isn't #rrggbb/#rrggbbaa and throws out the whole theme with it.
    return /^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(value) ? value : fallback;
  };

  // The app's tokens are opaque, but half of these colours are WASHES laid over text: a solid
  // selection would bury whatever it highlights. Monaco takes #rrggbbaa, so the alpha is synthesised
  // here rather than being something every theme has to define.
  const alpha = (hex, aa) => hex.slice(0, 7) + aa;

  const bg = token("--bg", "#1e1e1e");
  const bgRaised = token("--bg-raised", "#252526");
  const bgHover = token("--bg-hover", "#2a2d2e");
  const border = token("--border", "#3c3c3c");
  const fg = token("--fg", "#d4d4d4");
  const fgDim = token("--fg-dim", "#8a8a8a");
  const cyan = token("--cyan", "#00ffff");

  const base = luminanceOfHex(bg) > 0.5 ? "vs" : "vs-dark";
  try {
    monaco.editor.defineTheme("lowarc", {
      base,
      inherit: true,
      rules: [], // see the note above. Syntax is Monaco's, deliberately
      colors: {
        // The page itself.
        "editor.background": bg,
        "editor.foreground": fg,
        "editorGutter.background": bg,
        "minimap.background": bg,

        // Margin furniture: present and meaningful with no code at all.
        "editorLineNumber.foreground": fgDim,
        "editorLineNumber.activeForeground": fg,
        "editorCursor.foreground": cyan,
        "editorWhitespace.foreground": alpha(fgDim, "59"),
        "editorIndentGuide.background1": border,
        "editorIndentGuide.activeBackground1": fgDim,
        "editorOverviewRuler.border": border,

        // Washes. The alphas differ by intent: a selection has to stay readable through it, a find
        // match should shout slightly louder, and the current-line tint should be barely there.
        "editor.selectionBackground": alpha(cyan, "40"),
        "editor.inactiveSelectionBackground": alpha(cyan, "24"),
        "editor.selectionHighlightBackground": alpha(cyan, "24"),
        "editor.findMatchBackground": alpha(cyan, "59"),
        "editor.findMatchHighlightBackground": alpha(cyan, "33"),
        "editor.lineHighlightBackground": alpha(fg, "0d"),

        // Panels Monaco floats over the editor: these are dialogs, and should look like the app's
        // dialogs rather than like VS Code's.
        "editorWidget.background": bgRaised,
        "editorWidget.foreground": fg,
        "editorWidget.border": border,
        "editorHoverWidget.background": bgRaised,
        "editorHoverWidget.foreground": fg,
        "editorHoverWidget.border": border,
        "editorSuggestWidget.background": bgRaised,
        "editorSuggestWidget.foreground": fg,
        "editorSuggestWidget.border": border,
        "editorSuggestWidget.selectedBackground": bgHover,
        "input.background": bg,
        "input.foreground": fg,
        "input.border": border,
        "focusBorder": cyan,

        // Scrollbar: a wash again, so the code under it stays legible while dragging.
        "scrollbarSlider.background": alpha(fgDim, "40"),
        "scrollbarSlider.hoverBackground": alpha(fgDim, "66"),
        "scrollbarSlider.activeBackground": alpha(fgDim, "99"),
      },
    });
    monaco.editor.setTheme("lowarc");
  } catch (err) {
    // A malformed custom theme must not leave the editor unstyled: fall back to the built-in that
    // matches the app's ground, which is still better than staying on the wrong one.
    monaco.editor.setTheme(base);
  }
}

Promise.all([new Promise((resolve) => require(["vs/editor/editor.main"], resolve)), window.lowarc.getSettings()]).then(([, settings]) => {
  settings = settings || {};
  applyEditorTheme();

  // The host pushes new values into this iframe when the IDE's theme changes (see the harness in
  // plugin_assets.rs); by the time this fires they are already on documentElement, so re-reading is
  // all that's needed. Without it the editor keeps its startup theme until the panel is reopened.
  window.lowarc.on("lowarc:theme", () => applyEditorTheme());

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
    // window.lowarc.showMenu() into the host's un-clippable floating-menu), and its default
    // "Command Palette" entry is one of the two ways Monaco's native palette overlay was reachable
    // even after the keybindings below were overridden, since a context-menu click calls the
    // action directly rather than going through keybinding dispatch.
    contextmenu: false,

    // Everything below is VS Code furniture that Monaco turns on by default and that nothing in
    // this app ever feeds. Each one is either a control with no provider behind it (so it can only
    // ever render empty), or a second surface competing with one LowArc already owns. Measured
    // against the vendored build rather than assumed: all of these read as enabled out of the box.
    //
    // Sticky scroll is the most visible: a floating breadcrumb bar pinned over the top lines,
    // duplicating what the Outline panel is already for, while eating editor rows to do it.
    stickyScroll: { enabled: false },
    // Code lens and the lightbulb only ever appear when a language service contributes actions.
    // Nothing here does, so they are dead affordances: the lightbulb in particular shifts the
    // gutter around when it thinks it might show.
    codeLens: false,
    lightbulb: { enabled: "off" },
    inlayHints: { enabled: "off" },
    // Ghost-text completion, with its own hover toolbar and its own "snooze" commands. There is no
    // inline completion provider, and adding one is a plugin's business, not the editor's.
    inlineSuggest: { enabled: false },
    // The swatch beside a hex colour opens Monaco's own colour picker: a second, differently
    // styled colour UI inside an app whose Appearance page already has one.
    colorDecorators: false,
    // Ctrl+click on a URL calls window.open, which this sandbox has no allow-popups for: the
    // control renders, offers the hand cursor, and silently does nothing.
    links: false,
    // Dropping a file is the host's gesture. It decides which group and which viewer opens it.
    // Monaco's own drop handling would insert the path as text instead.
    dropIntoEditor: { enabled: false },
    // A hairline down the right edge that belongs to VS Code's chrome, not this panel's.
    overviewRulerBorder: false,
  });

  // Live counterpart to the getSettings() read above. See set_plugin_setting/
  // plugin-setting-changed in lib.rs and its relay to lowarc:settingsChanged in split-view.js.
  // Every option this plugin reads from settings is a plain editor.updateOptions() field, so there's
  // no per-doc/per-model state to touch: one shared call applies to whichever file is showing.
  window.lowarc.on("lowarc:settingsChanged", ({ key, value }) => {
    const one = { [key]: value };
    switch (key) {
      case "minimap":
        editor.updateOptions({ minimap: { enabled: boolSetting(one, key, true) } });
        break;
      case "lineNumbers":
        editor.updateOptions({ lineNumbers: boolSetting(one, key, true) ? "on" : "off" });
        break;
      case "wordWrap":
        editor.updateOptions({ wordWrap: boolSetting(one, key, false) ? "on" : "off" });
        break;
      case "fontSize":
        editor.updateOptions({ fontSize: numSetting(one, key, 14) });
        break;
      case "tabSize":
        editor.updateOptions({ tabSize: numSetting(one, key, 4) });
        break;
      case "insertSpaces":
        editor.updateOptions({ insertSpaces: boolSetting(one, key, true) });
        break;
    }
  });

  // This instance can now outlive any one file: it's mounted once per (editor group, Monaco) pair
  // and stays alive as long as ANY file that group opened through Monaco is still open, the same
  // "one iframe, many documents" shape the Terminal plugin already uses for multiple terminal tabs
  // (see its own file header). path -> {model, viewState, savedVersionId}. Nothing here is read
  // off the URL any more: every file this instance shows arrives entirely through
  // lowarc:openFile/activateFile/closeFile messages over its lifetime, matching the host's own
  // contract in plugin_assets.rs.
  const docs = new Map();
  let activePath = null;
  let saving = false;
  // A doc's own model.setValue()/createModel() fires onDidChangeContent, which would otherwise
  // report a brand-new file as dirty against its own starting content: this guard is what keeps
  // "just opened" from reading as "already edited."
  let loading = false;

  // The baseline to compare against for dirty-tracking: Monaco's own "back to saved" detection
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
  // in the process, not just the active one) rather than something scoped to one model: only a
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

  // A path already in `docs` means it's already open here: nothing to do (lowarc:activateFile is
  // the separate "make this one visible" step; re-sending lowarc:openFile for an already-open path
  // is a normal no-op, not an error, since the host doesn't track per-instance state itself).
  window.lowarc.on("lowarc:openFile", (payload) => {
    if (!payload || typeof payload.path !== "string" || docs.has(payload.path)) return;
    const model = monaco.editor.createModel(payload.contents, languageForPath(payload.path), monaco.Uri.file(payload.path));
    const doc = { model, viewState: null, savedVersionId: model.getAlternativeVersionId() };
    docs.set(payload.path, doc);
    model.onDidChangeContent(() => updateDirty(payload.path));
  });

  // The host's own signal for "this path's disk content just changed out from under you" (see
  // refreshOpenFile() in tabs-inspector.js — the Draft Tool's Revert writing straight to disk is
  // what motivated this) — deliberately a SEPARATE event from lowarc:openFile, which is a no-op
  // for an already-open path by design (see that handler's own comment). setValue(), not a fresh
  // model: keeps this the SAME model instance (still attached to the editor if it's the active
  // file, still the same object everything else here references): just replaces its content and
  // resets the dirty baseline to match, the same "this is now the clean, saved state" treatment a
  // real save gets. Undo history resets along with it, on purpose: there's no meaningful "undo"
  // back to an in-editor state that disk has since genuinely diverged from.
  window.lowarc.on("lowarc:refreshFile", (payload) => {
    if (!payload || typeof payload.path !== "string") return;
    const doc = docs.get(payload.path);
    if (!doc) return; // not open here
    loading = true;
    doc.model.setValue(payload.contents);
    loading = false;
    doc.savedVersionId = doc.model.getAlternativeVersionId();
    updateDirty(payload.path);
  });

  window.lowarc.on("lowarc:activateFile", (payload) => {
    const doc = payload && docs.get(payload.path);
    if (!doc) return;
    // Saving/restoring view state (scroll position, cursor, folds) around the swap is what makes
    // switching tabs feel like coming back to the same place: the model itself already carries
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
  // moving a file to the other editor group): replies with this instance's actual current text
  // for that path, unsaved edits included, rather than making the host re-read the file from disk
  // and silently discard them.
  window.lowarc.on("lowarc:getContent", (payload) => {
    if (!payload) return;
    const doc = docs.get(payload.path);
    window.parent.postMessage({ type: "hostRequestReply", replyId: payload.replyId, content: doc ? doc.model.getValue() : null }, "*");
  });

  // A plugin that is not this file's viewer asking for one line's text to change. executeEdits(),
  // not applyEdits() or setValue(): only executeEdits integrates with the undo-redo controller the
  // way a keystroke does, so without it the edit cannot be undone.
  //
  // It acts on whatever model is CURRENTLY SET, so using it on a backgrounded file would corrupt
  // whichever file is on screen. That case falls back to a plain model edit; it is rare, since
  // Outline only shows the active file's symbols.
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
      // The host already surfaces a toast for a failed write: nothing useful for this iframe
      // to additionally show, but the dirty flag must NOT be cleared on a failed save.
    } finally {
      saving = false;
    }
  }

  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, save);

  // The host's File > Save menu item / Ctrl+S (see editor.html): a click happens in the host
  // document, not this iframe, so it can't reach the keybinding above; this is the same save()
  // reached a different way.
  window.lowarc.on("lowarc:requestSave", save);

  // The host's Edit menu (Undo/Redo/Cut/Copy/Find/Replace — everything except Paste) and any
  // matching host-level shortcut: editor.trigger(), not editor.getAction(id).run() (what
  // lowarc:runCommand below uses), because Undo/Redo aren't registered Actions, only Commands;
  // trigger() dispatches to either kind, so one handler covers all of them.
  window.lowarc.on("lowarc:editorCommand", (payload) => {
    if (!payload || typeof payload.command !== "string") return;
    editor.trigger("host-menu", payload.command, null);
  });

  // Paste specifically: the host already read the OS clipboard itself (this iframe's own
  // navigator.clipboard.readText() is what's actually blocked; see editor.html's paste handler
  // for the full reasoning) and hands over plain text to drop in at the current selection.
  // executeEdits(), not insertText/applyEdits, for the same undo-integration reason every other
  // real edit in this file uses it.
  window.lowarc.on("lowarc:insertText", (payload) => {
    if (!payload || typeof payload.text !== "string" || !activePath) return;
    editor.executeEdits("host-paste", [{ range: editor.getSelection(), text: payload.text }]);
    editor.focus();
  });

  // Monaco ships its own command palette on F1 and Ctrl+Shift+P, a second list-of-every-action
  // surface competing with the host's. Its real action list goes into the host's palette via
  // setCommands() instead, so there is one list rather than two.
  //
  // Overriding the keybindings alone is not enough, since Monaco's own binding for quickCommand
  // still wins that resolution. Neutering the action's .run() is the one place every trigger path
  // goes through, so that is the actual kill switch; the keybinding overrides below are insurance.
  const quickCommandAction = editor.getAction("editor.action.quickCommand");
  if (quickCommandAction) quickCommandAction.run = async () => {};

  // Runs once per editor instance. Monaco's action list does not depend on which file is active,
  // so this never needs re-sending per open.
  //
  // Monaco reports 127 labelled actions and the palette is a list a person reads, so the ones that
  // cannot work here are dropped. toggleHighContrast is the one that matters: it swaps Monaco onto
  // a theme of its own and wipes applyEditorTheme()'s mapping with no way back short of reopening
  // the file. fontZoom* moves the font size behind the app's own Editor setting. The rest have
  // nothing behind them at all.
  //
  // A list rather than a pattern, so anything Monaco adds in a future version shows up instead of
  // being swallowed by an over-broad rule.
  const DEAD_COMMANDS = new Set([
    "editor.action.quickCommand", // the host owns the palette; see the kill switch above
    "editor.action.toggleHighContrast",
    "editor.action.fontZoomIn",
    "editor.action.fontZoomOut",
    "editor.action.fontZoomReset",
    "editor.action.showContextMenu",
    "editor.action.inlineSuggest.trigger",
    "editor.action.inlineSuggest.toggleShowCollapsed",
    "editor.action.openLink",
    "editor.action.pasteAs",
    "editor.action.pasteAsText",
    "editor.action.marker.nextInFiles",
    "editor.action.marker.prevInFiles",
    "editor.action.debugEditorGpuRenderer",
    "editor.action.forceRetokenize",
    "editor.action.inspectTokens",
  ]);

  function refreshCommands() {
    const commands = editor.getSupportedActions()
      .filter((a) => a.label && !DEAD_COMMANDS.has(a.id))
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
