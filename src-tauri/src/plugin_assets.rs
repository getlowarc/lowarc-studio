// Shared building blocks for serving an installed plugin's static UI assets (see
// plugin_host::protocol::PanelContribution). The actual transport is plugin_asset_server.rs — a
// loopback HTTP server, not a Tauri custom URI scheme. A custom scheme (`plugin://...`) was tried
// first and works fine for top-level navigation, but on Windows/WebView2 a sub-frame (iframe)
// navigation to a custom scheme silently never reaches the registered handler at all — confirmed
// directly (zero invocations logged, "provisional headers only" in devtools, reproduced even for
// a hardcoded response with no filesystem access) and matches a known, still-unresolved Tauri
// limitation (tauri-apps/tauri discussions #10868). A real `http://127.0.0.1:<port>` origin has
// none of that custom-scheme baggage, so panels are hosted there instead.
//
// This file owns the two things every caller of a plugin's assets needs to agree on:
//   1. Path-traversal protection — canonicalizing the resolved path and checking it's still under
//      the plugin's own folder before ever reading it.
//   2. The harness script and CSP every response should carry.

use crate::app_paths::AppPaths;
use std::path::PathBuf;

/// Every plugin's entry HTML is expected to load this first, before its own script. It exists for
/// exactly one reason: defense in depth on top of the real security boundary (capabilities/
/// default.json granting nothing to this origin). Verified directly against the installed wry
/// source that on Windows specifically, Tauri's IPC bridge script is injected into every frame —
/// including a sandboxed iframe — regardless of the for_main_frame_only flag (wry's own comment:
/// "Windows: scripts are always added to subframes regardless of the for_main_frame_only option";
/// only the macOS/Linux backends actually honor it). So window.__TAURI__ genuinely exists here for
/// a moment no matter what — this deletes it as early as content under our control possibly can,
/// before any plugin-authored script gets a chance to touch it.
pub const HARNESS_JS: &str = r#"(function () {
  try { delete window.__TAURI__; } catch (e) {}
  try { delete window.__TAURI_INTERNALS__; } catch (e) {}

  // No system right-click menu anywhere a plugin doesn't build its own — a plugin that wants a
  // context menu calls showMenu() below, whose own listener calls preventDefault() itself before
  // this ever runs, so that path is unaffected. Anything without one just gets no menu at all.
  window.addEventListener("contextmenu", (e) => e.preventDefault());

  let nextId = 1;
  const pending = new Map();
  const listeners = new Map();

  window.addEventListener("message", (event) => {
    const data = event.data;
    if (!data || typeof data !== "object") return;
    if (data.type === "reply" && pending.has(data.id)) {
      const waiter = pending.get(data.id);
      pending.delete(data.id);
      if (data.ok) waiter.resolve(data.result);
      else waiter.reject(new Error(data.error || "plugin call failed"));
    } else if (data.type === "emit") {
      (listeners.get(data.event) || []).forEach((handler) => handler(data.payload));
    }
  });

  window.lowarc = {
    call(method, params) {
      return new Promise((resolve, reject) => {
        const id = nextId++;
        pending.set(id, { resolve, reject });
        window.parent.postMessage({ type: "call", id, method, params: params || null }, "*");
      });
    },
    on(event, handler) {
      if (!listeners.has(event)) listeners.set(event, []);
      listeners.get(event).push(handler);
    },
    // A multi-document viewer plugin (Monaco is the first) manages several open files inside ONE
    // mounted iframe instead of getting a fresh iframe per file — cheaper, and it's what lets a
    // widget's own view state (scroll/cursor/folds) survive a tab switch the same way a real
    // editor's undo history already does, without the host needing to know anything about that
    // internal state. The host drives this with three emits a plugin listens for via on(), not new
    // methods here — there was nothing to add to the call surface, only new events to handle:
    //   lowarc:openFile   {path, contents} — a file was opened; create whatever internal state
    //                      this file needs (e.g. a model) if it doesn't exist yet. Not necessarily
    //                      the one to show — activateFile is the separate "make this visible" step.
    //   lowarc:activateFile {path} — switch to showing this already-opened path.
    //   lowarc:closeFile  {path} — dispose whatever was created for this path; it won't be
    //                      referenced again unless a fresh lowarc:openFile arrives for it later.
    // The host can also ask for a path's current (possibly unsaved) content — e.g. when moving a
    // file to the other editor group, where re-reading from disk would silently drop unsaved
    // edits. This is the one case where a HOST-initiated request needs a reply, the reverse of
    // every other request/reply pair in this file — there's no dedicated method for it since it's
    // not something a plugin ever calls, only receives:
    //   lowarc:getContent {path, replyId} — reply with
    //     window.parent.postMessage({type: "hostRequestReply", replyId, content}, "*")
    //     (content: the current text, or null if this plugin has nothing for that path).
    // markDirty/requestClose talk straight to the host's own tab-bar UI, not through this
    // plugin's backend process the way call()/on() do — there's nothing for a backend to decide
    // here, it's just "update my tab", so routing it through a process round-trip would be pure
    // overhead. A viewer plugin (see tab-bar/open-files) is the only kind of plugin these mean
    // anything to; anyone else calling them is just poking a host tab that doesn't exist for them.
    // `path` is required, not implicit — one viewer iframe can now be responsible for several open
    // files at once (see lowarc:openFile/activateFile/closeFile below), so there's no longer a
    // single unambiguous file this call could only be about.
    markDirty(path, dirty) {
      window.parent.postMessage({ type: "host", action: "markDirty", path, dirty: Boolean(dirty) }, "*");
    },
    // Same shape and reasoning as markDirty — a viewer with its own diagnostics (Monaco's marker
    // list, today) tells the host whenever that set of errors becomes empty/non-empty, not what
    // the errors actually are; the host only needs a boolean to decorate a tab or file row with.
    markErrors(path, hasErrors) {
      window.parent.postMessage({ type: "host", action: "markErrors", path, hasErrors: Boolean(hasErrors) }, "*");
    },
    // Registers (replacing any previous set from this same plugin) this plugin's own Command
    // Palette entries — the dynamic counterpart to plugin.json's static `commands` array (see
    // PluginCommand in protocol.rs). For a plugin whose available commands can't be known ahead of
    // time at manifest-authoring time (Monaco's own built-in editor actions, which vary by what's
    // actually registered at runtime) this is how it tells the host what to list instead. commands
    // is [{id, label, hint}], same shape as the static ones; running one posts the same
    // lowarc:runCommand emit either way. Fire-and-forget, same reasoning as markDirty.
    setCommands(commands) {
      window.parent.postMessage({ type: "host", action: "setCommands", commands: commands || [] }, "*");
    },
    requestClose(path) {
      window.parent.postMessage({ type: "host", action: "requestClose", path }, "*");
    },
    // Lets a plugin that isn't itself the file's own viewer (Outline, editing a value it parsed
    // out of the file) push a change into whatever IS currently showing that file — the host
    // forwards it as a lowarc:applyLineEdit emit to that path's owning viewer instance, which
    // applies it as a real edit (Monaco: model.applyEdits, not setValue — preserves undo history
    // and fires the exact same dirty-tracking a person's own keystroke would). `line` is 1-based,
    // `text` replaces that entire line's content. Fire-and-forget, same reasoning as markDirty —
    // nothing for the caller to wait on; if the path isn't open or has no viewer, this silently
    // does nothing rather than erroring, the same as every other host-owned action here.
    editFile(path, line, text) {
      window.parent.postMessage({ type: "host", action: "editFile", path, line, text }, "*");
    },
    // Any plugin can raise a toast/notification through the host's own system — see showToast()
    // in primitives.js, which is the SAME pipe host chrome's own errors/confirmations already go
    // through, not a separate plugin-only notification channel. Fire-and-forget: a plugin has
    // nothing to wait on here, same reasoning as markDirty/requestClose. variant is "info" |
    // "warning" | "success" | "error"; the host tags the resulting history entry with this
    // plugin's id so a person browsing the bell's history can tell where it came from.
    notify(variant, message) {
      window.parent.postMessage({ type: "host", action: "notify", variant, message }, "*");
    },
    // Opens (or activates, if already open) a file in the host's own tab bar — the open-files
    // list is core IDE state, not something any one plugin owns, so a file explorer (or anything
    // else that wants to open something) just asks the host to add to it rather than managing
    // its own separate notion of "what's open". Same direct-to-host channel as markDirty/
    // requestClose, for the same reason: nothing for this plugin's own backend to decide here.
    // opts.openInSplit: true opens (or moves an already-open file) into the second editor group,
    // opening the split first if it isn't already — see the file explorer's "Open in Split View".
    openFile(path, opts) {
      window.parent.postMessage({ type: "host", action: "openFile", path, openInSplit: Boolean(opts && opts.openInSplit) }, "*");
    },
    // Generic "let the user pick a file for me to open" — a plugin has no filesystem access of
    // its own to browse with, so this asks the host to show its real native file picker instead
    // (options passed straight through to Tauri's dialog.open(), e.g. {filters: [{name, extensions}]}).
    // Resolves to the picked absolute path, or null if cancelled — never rejects, same reasoning
    // showMenu() gives for its own reject-into-null. Opening the result is a separate step
    // (openFile() above) rather than automatic, since a caller might want the path itself for
    // something else (e.g. remembering it, or reading it back through call()).
    pickOpenFile(opts) {
      return new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, { resolve, reject: () => resolve(null) });
        window.parent.postMessage({ type: "host", action: "pickOpenFile", id, opts: opts || {} }, "*");
      });
    },
    // Generic "let the user pick where a brand-new file goes, then write it for me" — the
    // complement to pickOpenFile, for a plugin that wants a "New…" button of its own without ever
    // needing raw filesystem write access itself (opts.contents is what actually gets written,
    // "" if omitted). Resolves to the new file's absolute path, or null if the save dialog was
    // cancelled — never rejects.
    createFile(opts) {
      return new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, { resolve, reject: () => resolve(null) });
        window.parent.postMessage({ type: "host", action: "createFile", id, opts: opts || {} }, "*");
      });
    },
    // Fire-and-forget, same reasoning as markDirty/requestClose/openFile — a file explorer (or
    // anything else that mutates the filesystem) tells the host a path is gone or moved so the
    // host can flag that path's own tab as missing, if it happens to be open. Nothing for the
    // caller to wait on; the host doesn't own a reply for these the way saveFile needs one.
    notifyPathDeleted(path) {
      window.parent.postMessage({ type: "host", action: "notifyPathDeleted", path }, "*");
    },
    notifyPathRenamed(oldPath, newPath) {
      window.parent.postMessage({ type: "host", action: "notifyPathRenamed", oldPath, newPath }, "*");
    },
    // A "session": true plugin's own UI (Terminal, so far) manages its own instances — each open
    // terminal tab is its own sessionId, chosen by the plugin itself (a UUID is fine; the host
    // never needs to parse it, only use it as an opaque map key). All three are fire-and-forget:
    // startSession's session doesn't exist yet to reply through, sendSession's replies (if any)
    // arrive separately as lowarc:sessionOutput emits tagged with that same sessionId, and
    // stopSession has nothing to report back beyond the process simply no longer running.
    startSession(sessionId, shell) {
      window.parent.postMessage({ type: "host", action: "startSession", sessionId, shell: shell || null }, "*");
    },
    sendSession(sessionId, message) {
      window.parent.postMessage({ type: "host", action: "sendSession", sessionId, message }, "*");
    },
    stopSession(sessionId) {
      window.parent.postMessage({ type: "host", action: "stopSession", sessionId }, "*");
    },
    // Unlike markDirty/requestClose/openFile, this one needs a real answer — a write can fail
    // (disk full, permissions, the file having been deleted from under it), and the caller needs
    // to know before it clears its own "unsaved changes" state. Reuses the exact same id/pending/
    // "reply" plumbing as call() rather than inventing a second request/response mechanism; the
    // host just has to remember to reply with that shape instead of only firing an event.
    saveFile(path, contents) {
      return new Promise((resolve, reject) => {
        const id = nextId++;
        pending.set(id, { resolve, reject });
        window.parent.postMessage({ type: "host", action: "saveFile", id, path, contents }, "*");
      });
    },
    // Asks the host to render a context/action menu on the real screen, outside this iframe's own
    // (sandboxed, position:fixed-can't-escape) box — see the .floating-menu primitive in
    // primitives.css for why this exists at all. x/y are this document's own coordinates (e.g. a
    // right-click's clientX/clientY) — the host translates them into screen space itself, since it
    // knows where this iframe sits in its own layout and this iframe doesn't. items is
    // [{label, value, disabled}] (a divider is {divider: true}); resolves to the chosen item's
    // value, or null if the menu was dismissed without a choice. Same reply plumbing as saveFile(),
    // since the caller genuinely needs to know what was picked.
    showMenu(items, x, y) {
      return new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, { resolve, reject: () => resolve(null) });
        window.parent.postMessage({ type: "host", action: "showMenu", id, items, x, y }, "*");
      });
    },
    // Asks the host to open a popup from its shared Popup stack (see showPopup() in editor.html) —
    // id must name a popup some contribution already registered; target is passed straight through
    // to that popup's own mount(). Resolves to whatever that popup closed with, or null if it was
    // dismissed (Escape, backdrop, the X) or no such popup exists. Same reply plumbing as showMenu.
    openPopup(id, target) {
      return new Promise((resolve) => {
        const id_ = nextId++;
        pending.set(id_, { resolve, reject: () => resolve(null) });
        window.parent.postMessage({ type: "host", action: "openPopup", id: id_, popupId: id, target }, "*");
      });
    },
    // This plugin's own currently-saved values for whatever it declared in its own plugin.json's
    // `settings` (see PluginSettingField) — {key: value}, empty object if it hasn't declared any or
    // none are saved yet. Read-only from here on purpose: values are set through the Settings page,
    // not by a plugin writing its own config, so there's no setSettings() to go with this.
    getSettings() {
      return new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, { resolve, reject: () => resolve({}) });
        window.parent.postMessage({ type: "host", action: "getSettings", id }, "*");
      });
    },
    // Debugger primitives — generic, not scoped to any one plugin (any plugin could build a run
    // monitor, not just the first-party Debugger one). Fire-and-forget, same reasoning as
    // requestClose/markDirty: a failure (e.g. "no run is active") has nothing for the caller
    // itself to branch on, so the host just surfaces it as its own toast. setBreakpoints always
    // sends the WHOLE list — same "frontend always resends everything" convention plugin
    // settings/commands already use, one fewer state-sync mechanism to get wrong.
    pauseRun() {
      window.parent.postMessage({ type: "host", action: "pauseRun" }, "*");
    },
    resumeRun() {
      window.parent.postMessage({ type: "host", action: "resumeRun" }, "*");
    },
    stepRun(count) {
      window.parent.postMessage({ type: "host", action: "stepRun", count: count || 1 }, "*");
    },
    setBreakpoints(breakpoints) {
      window.parent.postMessage({ type: "host", action: "setBreakpoints", breakpoints: breakpoints || [] }, "*");
    },
  };
})();
"#;

// worker-src covers Monaco's language-service web workers, which it spins up via a Worker(blob
// URL) that immediately importScripts() the real (same-origin) worker file — 'blob:' for the
// Worker constructor call itself, 'self' for the importScripts target. Without worker-src, that
// falls back to default-src 'none' and silently breaks every worker Monaco tries to create.
// font-src covers Monaco's own icon font (fold arrows, error/warning glyphs, etc.) — it's a
// `data:` URI embedded directly in editor.main.css, not a separate file, so without explicit
// permission here it also falls back to default-src 'none' and silently renders as fallback
// tofu/square glyphs instead of the real icons.
// media-src covers <video>/<audio> src (the media-viewer plugin's own data: URIs) — CSP treats
// this as a distinct resource type from img-src, so without it a <video> falls back to
// default-src 'none' the same way an unlisted font or worker would.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; media-src 'self' data:; worker-src 'self' blob:; connect-src 'none'";

/// The host's own base stylesheet (root color-token variables, plus reset/scrollbar/titlebar
/// rules) — the literal same file every host page (editor.html, settings.html, etc.) already
/// links, via `include_str!` so this can never drift from it. Served at `__lowarc.css`.
///
/// A plugin opts in entirely on its own — nothing forces this on any plugin, first-party or not —
/// by adding `<link rel="stylesheet" href="__lowarc.css">` to its own HTML, same convention as
/// `<script src="__lowarc.js">`. Not theme-live: this bakes in style.css's own `:root` values
/// (the app's default dark palette), not whatever theme.js resolves Settings.themeMode to at
/// runtime — theme.js itself can't run inside a plugin (it calls Tauri commands a sandboxed plugin
/// has no access to), so a plugin using this always renders in the default palette regardless of
/// whether the IDE itself is currently on Light or a custom preset. A real fix for that would mean
/// serving a theme-resolved stylesheet dynamically (a new Rust-side route mirroring theme.js's own
/// resolution) — a deliberate, acknowledged scope cut, not an oversight.
pub const SHARED_STYLE_CSS: &str = include_str!("../../src/style.css");

/// The host's genuinely reusable component styles — buttons, text/numeric inputs, checkboxes, a
/// progress bar, setting rows — split out of primitives.css specifically so it could be exposed
/// here (see primitives-shared.css's own header for the full reasoning); `include_str!` again, one
/// real file, never a copy. Served at `__lowarc-primitives.css`, same opt-in-only convention as
/// `__lowarc.css` above — a plugin author links it, nothing forces it. Deliberately NOT the much
/// larger primitives.css: that file is packed with host-chrome-specific classes (`.rail`,
/// `.tab-bar`, `.manage-list`, dropdowns/toasts/popups/floating-menus) that assume the host's own
/// DOM structure and JS-driven interaction — a plugin can't meaningfully reuse any of that without
/// also reimplementing the JS behind it, so none of it is exposed. Depends on
/// `__lowarc.css`'s tokens; a plugin using this should link both, `__lowarc.css` first.
pub const SHARED_PRIMITIVES_CSS: &str = include_str!("../../src/primitives-shared.css");

/// File-type icons — vendored from vscode's built-in "Seti" icon theme (see
/// src/vendor/seti-icons/SETI_LICENSE), same include_str!/include_bytes! vendoring as everything
/// else here. Three pieces, all opt-in the same way as __lowarc.css: the font itself
/// (__lowarc-icons.woff), the generated stylesheet mapping each icon id to its glyph + color
/// (__lowarc-icons.css, depends on the font), and a small resolver script exposing
/// `window.lowarcIconClass(filename)` (__lowarc-icons.js) so a plugin doesn't have to reimplement
/// vscode's own fileNames-then-longest-extension matching order itself.
pub const SHARED_ICONS_CSS: &str = include_str!("../../src/vendor/seti-icons/icons.css");
pub const SHARED_ICONS_JS: &str = include_str!("../../src/vendor/seti-icons/icons.js");
pub const SHARED_ICONS_WOFF: &[u8] = include_bytes!("../../src/vendor/seti-icons/seti.woff");

/// A single-select dropdown matching the real IDE look (the CSS moved to primitives-shared.css
/// for this specifically) with its own small standalone behavior — see dropdown-shared.js's own
/// header for why this isn't just reusing the host's fuller-featured one. Served at
/// __lowarc-dropdown.js; exposes one function, `createLowarcDropdown(options, value, onChange)`.
pub const SHARED_DROPDOWN_JS: &str = include_str!("../../src/dropdown-shared.js");

/// Resolves `<plugin_id>/<rel_path>` to a real file, refusing anything that canonicalizes outside
/// that plugin's own folder — shared between plugin_asset_server.rs and `read_plugin_asset`
/// (lib.rs), since both need the exact same path-traversal protection and there's no reason for
/// two copies of security-critical logic to drift apart.
pub fn resolve_asset_path(plugin_id: &str, rel_path: &str) -> Option<PathBuf> {
    if plugin_id.is_empty() || rel_path.is_empty() {
        return None;
    }
    let plugin_root = AppPaths::plugins().join(plugin_id).canonicalize().ok()?;
    let resolved = plugin_root.join(rel_path).canonicalize().ok()?;
    // The load-bearing check: canonicalize resolves ".." components for real, so this catches a
    // request trying to climb out of the plugin's own folder regardless of how it's spelled.
    if !resolved.starts_with(&plugin_root) {
        return None;
    }
    Some(resolved)
}
