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
    // Asks the host to show THIS plugin's own inspector contribution (a plugin.json panel with
    // location: "inspector") right now, opening the Inspector panel if it's closed, and passing
    // context straight through as a lowarc:inspectorContext emit to whatever's now showing.
    // Fire-and-forget — the caller has nothing to wait on, same as openFile. A plugin with no
    // inspector contribution declared just gets silently ignored by the host, same "nothing to do"
    // shape as showMenu/openPopup targeting something that doesn't exist. This exists because the
    // Inspector, unlike the sidebar or console, has no permanent icon/tab of its own for a person
    // to click — something IN a plugin (a node getting clicked, e.g.) has to be able to ask for it
    // instead.
    //
    // onlyIfOpen (default false): when true, this is a silent no-op unless the Inspector panel is
    // ALREADY open — never forces it open, never switches its content if it was closed. For a
    // continuous interaction that touches a node repeatedly (dragging it around, say) that already
    // opened the Inspector once on its own — a background drag shouldn't be able to yank the panel
    // open or hijack whatever it was already showing.
    openInspector(context, onlyIfOpen) {
      window.parent.postMessage({ type: "host", action: "openInspector", context: context ?? null, onlyIfOpen: Boolean(onlyIfOpen) }, "*");
    },
    // Sends event/payload to every OTHER currently-mounted iframe belonging to THIS SAME plugin —
    // never the caller itself, and never a different plugin's iframe. Fire-and-forget. Exists for
    // a plugin with more than one simultaneous iframe that need to coordinate (e.g. Node Graph's
    // Inspector drawer editing a node that a separate canvas iframe actually owns and renders) —
    // there was previously no way for two iframes of the same plugin to talk to each other at all,
    // only host-to-plugin and plugin-to-host.
    broadcastToSelf(event, payload) {
      window.parent.postMessage({ type: "host", action: "broadcastToSelf", event, payload: payload ?? null }, "*");
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
    // never needs to parse it, only use it as an opaque map key). Namespaced under `session`
    // (2026-09-01) rather than sitting flat as startSession/sendSession/stopSession — this is a
    // generic mechanism any "session": true plugin could use, not something tied to the file-
    // lifecycle/host-chrome calls that make up most of this object, and grouping it makes that
    // boundary visible instead of just implied by a shared name prefix. All three are fire-and-
    // forget: start's session doesn't exist yet to reply through, send's replies (if any) arrive
    // separately as lowarc:sessionOutput emits tagged with that same sessionId, and stop has
    // nothing to report back beyond the process simply no longer running.
    session: {
      start(sessionId, shell) {
        window.parent.postMessage({ type: "host", action: "startSession", sessionId, shell: shell || null }, "*");
      },
      send(sessionId, message) {
        window.parent.postMessage({ type: "host", action: "sendSession", sessionId, message }, "*");
      },
      stop(sessionId) {
        window.parent.postMessage({ type: "host", action: "stopSession", sessionId }, "*");
      },
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
    // Run-debugging primitives — generic, not scoped to any one plugin (any plugin could build a
    // run monitor, not just the first-party Debugger one). Namespaced under `debug` (2026-09-01),
    // same reasoning as `session` above: a distinct, generic mini-API, not another file/host-
    // chrome call sitting flat among them. Fire-and-forget, same reasoning as requestClose/
    // markDirty: a failure (e.g. "no run is active") has nothing for the caller itself to branch
    // on, so the host just surfaces it as its own toast. setBreakpoints always sends the WHOLE
    // list — same "frontend always resends everything" convention plugin settings/commands
    // already use, one fewer state-sync mechanism to get wrong.
    debug: {
      pause() {
        window.parent.postMessage({ type: "host", action: "pauseRun" }, "*");
      },
      resume() {
        window.parent.postMessage({ type: "host", action: "resumeRun" }, "*");
      },
      step(count) {
        window.parent.postMessage({ type: "host", action: "stepRun", count: count || 1 }, "*");
      },
      setBreakpoints(breakpoints) {
        window.parent.postMessage({ type: "host", action: "setBreakpoints", breakpoints: breakpoints || [] }, "*");
      },
    },
    // A plain DOM utility, not a host round-trip like everything else here — wires up any
    // ".numeric-input" markup (see __lowarc-primitives.css) with real, working steppers. This is
    // the ENTIRE reason that CSS class is safe to expose to plugins at all: a native
    // type="number" input's spin arrows can't be restyled to match the app (hence the drawn
    // ".numeric-steppers" buttons instead), and CSS alone would just be buttons that visibly do
    // nothing on click. Same clamp/read-attributes behavior as the host's own initNumericInputs()
    // in primitives.js — kept as a separate copy on purpose, same as everything else in this file:
    // this is the plugin-facing contract, and it shouldn't secretly depend on a host-only script a
    // plugin can never load. Dispatches BOTH "input" and "change" so a caller can commit with a
    // plain input.addEventListener("change", ...), the exact same pattern already used for every
    // other field type — no special-casing just because this one has stepper buttons too.
    initNumericInputs(root) {
      (root || document).querySelectorAll(".numeric-input").forEach((el) => {
        if (el.dataset.numericInit) return;
        el.dataset.numericInit = "true";
        const input = el.querySelector("input");
        const up = el.querySelector(".stepper-up");
        const down = el.querySelector(".stepper-down");
        const min = input.hasAttribute("min") ? Number(input.min) : -Infinity;
        const max = input.hasAttribute("max") ? Number(input.max) : Infinity;
        const step = input.hasAttribute("step") ? Number(input.step) : 1;
        const clamp = (n) => Math.min(max, Math.max(min, n));
        const bump = (delta) => {
          input.value = clamp((Number(input.value) || 0) + delta);
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.dispatchEvent(new Event("change", { bubbles: true }));
        };
        if (up) up.addEventListener("click", () => bump(step));
        if (down) down.addEventListener("click", () => bump(-step));
        input.addEventListener("blur", () => {
          if (input.value === "") return;
          input.value = clamp(Number(input.value) || 0);
        });
      });
    },
    // Same data-tooltip="..." convention as the host's own initTooltips() in primitives.js — wires
    // up any element carrying that attribute (set once in markup, or any time via
    // el.dataset.tooltip = "..."), a small delayed popup on hover, one shared ".tooltip-popup"
    // element per document. NOT a byte-for-byte port, though, unlike most of this file's other
    // pairs: the host's version only ever prefers a side and falls back once, which is fine in the
    // full app window but was a real, shipped bug the first time a plugin tried it in its own
    // narrow iframe — a "fixed" element here can only ever paint inside THIS iframe's own
    // viewport, so a tooltip wider than the panel would just get cut off no matter which side it
    // preferred. This version clamps fully inside window.innerWidth/innerHeight instead of only
    // choosing a side, and pairs with ".tooltip-popup"'s max-width/wrapping in
    // __lowarc-primitives.css for the same reason.
    initTooltips(root) {
      let tooltipEl = document.querySelector(".tooltip-popup");
      if (!tooltipEl) {
        tooltipEl = document.createElement("div");
        tooltipEl.className = "tooltip-popup";
        document.body.appendChild(tooltipEl);
      }
      let showTimer = null;
      (root || document).querySelectorAll("[data-tooltip]").forEach((el) => {
        if (el.dataset.tooltipInit) return;
        el.dataset.tooltipInit = "true";

        el.addEventListener("mouseenter", () => {
          clearTimeout(showTimer);
          showTimer = setTimeout(() => {
            tooltipEl.textContent = el.dataset.tooltip;
            tooltipEl.classList.add("is-visible");
            const rect = el.getBoundingClientRect();
            const tipRect = tooltipEl.getBoundingClientRect();
            const fitsRight = rect.right + 8 + tipRect.width <= window.innerWidth;
            const left = fitsRight ? rect.right + 8 : rect.left - tipRect.width - 8;
            tooltipEl.style.left = `${Math.min(Math.max(4, left), Math.max(4, window.innerWidth - tipRect.width - 4))}px`;
            tooltipEl.style.top = `${Math.min(Math.max(4, rect.top + rect.height / 2 - tipRect.height / 2), Math.max(4, window.innerHeight - tipRect.height - 4))}px`;
          }, 400);
        });
        el.addEventListener("mouseleave", () => {
          clearTimeout(showTimer);
          tooltipEl.classList.remove("is-visible");
        });
        el.addEventListener("click", () => {
          clearTimeout(showTimer);
          tooltipEl.classList.remove("is-visible");
        });
      });
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
    if rel_path.is_empty() {
        return None;
    }
    // plugin_id has to name exactly one folder directly under plugins() — reject anything that
    // could shift plugin_root itself outside that directory (a bare "..", an embedded path
    // separator, or a "." component) before it's ever joined onto a real path. Without this, a
    // plugin_id of ".." would make plugin_root canonicalize to plugins()'s own parent, and the
    // starts_with(plugin_root) check below would then accept any rel_path reachable from there —
    // the check has to hold on plugin_id itself, not just on the eventual resolved path.
    if !AppPaths::is_valid_component_id(plugin_id) {
        return None;
    }
    let plugins_root = AppPaths::plugins().canonicalize().ok()?;
    let plugin_root = plugins_root.join(plugin_id).canonicalize().ok()?;
    if !plugin_root.starts_with(&plugins_root) {
        return None;
    }
    let resolved = plugin_root.join(rel_path).canonicalize().ok()?;
    // The load-bearing check: canonicalize resolves ".." components for real, so this catches a
    // request trying to climb out of the plugin's own folder regardless of how it's spelled.
    if !resolved.starts_with(&plugin_root) {
        return None;
    }
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_asset_path_rejects_a_traversal_via_plugin_id() {
        // Regression test for a real path-traversal bug: plugin_id used to be joined onto
        // plugins() and canonicalized BEFORE being checked, so a plugin_id of ".." shifted the
        // confinement boundary itself to plugins()'s own parent — letting any rel_path reachable
        // from there (e.g. settings.json, one directory up from plugins/ in a dev checkout) through
        // the starts_with(plugin_root) check below it. Both files genuinely exist on disk here.
        assert!(resolve_asset_path("..", "settings.json").is_none());
        assert!(resolve_asset_path("..", "recent.json").is_none());
    }

    #[test]
    fn resolve_asset_path_rejects_other_traversal_shapes_in_plugin_id() {
        assert!(resolve_asset_path(".", "settings.json").is_none());
        assert!(resolve_asset_path("foo/../..", "settings.json").is_none());
        assert!(resolve_asset_path("foo\\..\\..", "settings.json").is_none());
    }

    #[test]
    fn resolve_asset_path_still_resolves_a_real_plugin_asset() {
        // Sanity check the fix didn't break the legitimate case.
        let plugins_dir = AppPaths::plugins();
        let some_plugin = std::fs::read_dir(&plugins_dir)
            .expect("plugins/ should exist in a dev checkout")
            .filter_map(|e| e.ok())
            .find(|e| e.path().is_dir())
            .expect("at least one plugin folder should exist");
        let plugin_id = some_plugin.file_name().to_string_lossy().into_owned();
        assert!(resolve_asset_path(&plugin_id, "plugin.json").is_some());
    }
}
