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

/// Every plugin's entry HTML loads this before its own script. Defence in depth on top of the real
/// boundary, which is capabilities/default.json granting this origin nothing.
///
/// On Windows, wry injects Tauri's IPC bridge into every frame including a sandboxed iframe,
/// regardless of for_main_frame_only; only the macOS and Linux backends honour that flag. So
/// window.__TAURI__ does exist here briefly, and this deletes it as early as content under our
/// control can.
pub const HARNESS_JS: &str = r#"(function () {
  try { delete window.__TAURI__; } catch (e) {}
  try { delete window.__TAURI_INTERNALS__; } catch (e) {}

  // No system right-click menu anywhere a plugin doesn't build its own: a plugin that wants a
  // context menu calls showMenu() below, whose own listener calls preventDefault() itself before
  // this ever runs, so that path is unaffected. Anything without one just gets no menu at all.
  window.addEventListener("contextmenu", (e) => e.preventDefault());

  // A click inside a sandboxed iframe never bubbles to the host document, so the host's
  // click-outside handlers never see it and an open menu stays stuck open. Focus is not enough on
  // its own: it fires only on the transition, so clicking back into an already-focused iframe
  // moves no focus. A pointerdown always happens. Capture phase, so a plugin stopping propagation
  // in its own content cannot disable the host's menus.
  window.addEventListener("pointerdown", () => {
    try { window.parent.postMessage({ type: "pointerdown" }, "*"); } catch (e) {}
  }, true);

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
    } else if (data.type === "theme") {
      // Handled here rather than dispatched to on() listeners: every plugin that links
      // __lowarc.css wants this, and none of them should have to write code to stay in step with
      // the IDE's theme. __lowarc-theme.css already themed this document at load; this is only for
      // a theme CHANGED while the plugin is open, which a sandboxed iframe cannot otherwise notice
      // without being reloaded, and reloading would throw away whatever the plugin was showing.
      // Custom properties only, so a malformed payload can't set arbitrary styles.
      const vars = data.vars && typeof data.vars === "object" ? data.vars : {};
      for (const [name, value] of Object.entries(vars)) {
        if (typeof name === "string" && name.startsWith("--") && typeof value === "string") {
          document.documentElement.style.setProperty(name, value);
        }
      }
      // Applied above AND announced here, because a plugin whose colors don't all come from CSS has
      // real work to do. Monaco owns its own theme system and would otherwise keep rendering the
      // editor surface in whatever theme it was told about at startup, however the page around it
      // restyles. Listeners run after the properties are set, so on("lowarc:theme") can just read
      // the ones it needs off documentElement.
      (listeners.get("lowarc:theme") || []).forEach((handler) => handler(vars));
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
    // A multi-document viewer plugin manages several open files inside ONE mounted iframe, which is
    // what lets its view state (scroll, cursor, folds) survive a tab switch without the host
    // knowing anything about that state. Driven by three emits a plugin listens for via on():
    //   lowarc:openFile     {path, contents} — create whatever internal state this file needs.
    //                        Not necessarily the one to show; activateFile is that step.
    //   lowarc:activateFile {path} — switch to showing this already-opened path.
    //   lowarc:closeFile    {path} — dispose whatever was created for it.
    //
    // The host can also ask for a path's current, possibly unsaved, content, which is the one
    // HOST-initiated request that needs a reply:
    //   lowarc:getContent   {path, replyId} — reply with
    //     window.parent.postMessage({type: "hostRequestReply", replyId, content}, "*")
    //     where content is the current text, or null for a path this plugin has nothing for.
    //
    // markDirty and requestClose talk straight to the host's tab-bar UI rather than through the
    // plugin's backend, since there is nothing for a backend to decide. `path` is required because
    // one iframe can hold several files at once.
    markDirty(path, dirty) {
      window.parent.postMessage({ type: "host", action: "markDirty", path, dirty: Boolean(dirty) }, "*");
    },
    // Same shape and reasoning as markDirty: a viewer with its own diagnostics (Monaco's marker
    // list, today) tells the host whenever that set of errors becomes empty/non-empty, not what
    // the errors actually are; the host only needs a boolean to decorate a tab or file row with.
    markErrors(path, hasErrors) {
      window.parent.postMessage({ type: "host", action: "markErrors", path, hasErrors: Boolean(hasErrors) }, "*");
    },
    // Registers (replacing any previous set from this same plugin) this plugin's own Command
    // Palette entries: the dynamic counterpart to plugin.json's static `commands` array (see
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
    // Lets a plugin that is not the file's viewer (Outline, editing a value it parsed out) push a
    // change into whatever is showing that file. The host forwards it as lowarc:applyLineEdit and
    // the viewer applies it as a real edit, so undo and dirty-tracking behave as if typed. `line`
    // is 1-based and `text` replaces that whole line. Fire-and-forget: a path that is not open
    // does nothing rather than erroring.
    editFile(path, line, text) {
      window.parent.postMessage({ type: "host", action: "editFile", path, line, text }, "*");
    },
    // Any plugin can raise a toast/notification through the host's own system. See showToast()
    // in primitives.js, which is the SAME pipe host chrome's own errors/confirmations already go
    // through, not a separate plugin-only notification channel. Fire-and-forget: a plugin has
    // nothing to wait on here, same reasoning as markDirty/requestClose. variant is "info" |
    // "warning" | "success" | "error"; the host tags the resulting history entry with this
    // plugin's id so a person browsing the bell's history can tell where it came from.
    notify(variant, message) {
      window.parent.postMessage({ type: "host", action: "notify", variant, message }, "*");
    },
    // Opens (or activates, if already open) a file in the host's own tab bar: the open-files
    // list is core IDE state, not something any one plugin owns, so a file explorer (or anything
    // else that wants to open something) just asks the host to add to it rather than managing
    // its own separate notion of "what's open". Same direct-to-host channel as markDirty/
    // requestClose, for the same reason: nothing for this plugin's own backend to decide here.
    // opts.openInSplit: true opens (or moves an already-open file) into the second editor group,
    // opening the split first if it isn't already. See the file explorer's "Open in Split View".
    openFile(path, opts) {
      window.parent.postMessage({ type: "host", action: "openFile", path, openInSplit: Boolean(opts && opts.openInSplit) }, "*");
    },
    // Tells the host a path's on-disk content just changed out from under any editor that has it
    // open — e.g. the Draft Tool's Revert writing straight to disk, bypassing the editor entirely.
    // A no-op if that path isn't currently open anywhere. Not addressed to any one plugin, same
    // reasoning as openFile above: the open-files list (and re-reading a path's real content) is
    // core IDE state, not something a sidebar panel manages itself.
    refreshFile(path) {
      window.parent.postMessage({ type: "host", action: "refreshFile", path }, "*");
    },
    // Pushes a { "<path>": {added, removed} } map into the status bar's per-active-file diff
    // display: the File Explorer's Draft Tool is the only caller today, whenever its own
    // diffCounts changes (a capture, a revert, a commit). Fire-and-forget, same reasoning as
    // markDirty: nothing for the caller to wait on, and the host re-renders reactively off
    // whichever file is actually active right now, not off this call's own timing.
    setDiffStatus(diffCounts) {
      window.parent.postMessage({ type: "host", action: "setDiffStatus", diffCounts: diffCounts || {} }, "*");
    },
    // Generic "let the user pick a file for me to open": a plugin has no filesystem access of
    // its own to browse with, so this asks the host to show its real native file picker instead
    // (options passed straight through to Tauri's dialog.open(), e.g. {filters: [{name, extensions}]}).
    // Resolves to the picked absolute path, or null if cancelled: never rejects, same reasoning
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
    // Generic "let the user pick where a brand-new file goes, then write it for me": the
    // complement to pickOpenFile, for a plugin that wants a "New…" button of its own without ever
    // needing raw filesystem write access itself (opts.contents is what actually gets written,
    // "" if omitted). Resolves to the new file's absolute path, or null if the save dialog was
    // cancelled: never rejects.
    createFile(opts) {
      return new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, { resolve, reject: () => resolve(null) });
        window.parent.postMessage({ type: "host", action: "createFile", id, opts: opts || {} }, "*");
      });
    },
    // Shows this plugin's inspector contribution, opening the Inspector panel if closed and
    // passing context through as a lowarc:inspectorContext emit. Fire-and-forget; a plugin with no
    // inspector contribution is silently ignored. It exists because the Inspector has no permanent
    // icon of its own, so something inside a plugin has to ask for it.
    //
    // onlyIfOpen (default false): a silent no-op unless the Inspector is ALREADY open. For a
    // repeated interaction like dragging a node, so a background drag cannot yank the panel open
    // or hijack what it was showing.
    openInspector(context, onlyIfOpen) {
      window.parent.postMessage({ type: "host", action: "openInspector", context: context ?? null, onlyIfOpen: Boolean(onlyIfOpen) }, "*");
    },
    // Sends event/payload to every OTHER currently-mounted iframe belonging to THIS SAME plugin —
    // never the caller itself, and never a different plugin's iframe. Fire-and-forget. Exists for
    // a plugin with more than one simultaneous iframe that need to coordinate (e.g. Node Graph's
    // Inspector drawer editing a node that a separate canvas iframe owns and renders). Everything
    // else in this harness is host-to-plugin or plugin-to-host; this is the one plugin-to-itself
    // path.
    broadcastToSelf(event, payload) {
      window.parent.postMessage({ type: "host", action: "broadcastToSelf", event, payload: payload ?? null }, "*");
    },
    // Fire-and-forget, same reasoning as markDirty/requestClose/openFile: a file explorer (or
    // anything else that mutates the filesystem) tells the host a path is gone or moved so the
    // host can flag that path's own tab as missing, if it happens to be open. Nothing for the
    // caller to wait on; the host doesn't own a reply for these the way saveFile needs one.
    notifyPathDeleted(path) {
      window.parent.postMessage({ type: "host", action: "notifyPathDeleted", path }, "*");
    },
    notifyPathRenamed(oldPath, newPath) {
      window.parent.postMessage({ type: "host", action: "notifyPathRenamed", oldPath, newPath }, "*");
    },
    // A "session": true plugin manages its own instances: each open terminal tab is its own
    // sessionId, chosen by the plugin and opaque to the host. Namespaced rather than flat because
    // it is generic to any session plugin, unlike the file-lifecycle calls around it. All three are
    // fire-and-forget: start has no session to reply through yet, send's replies arrive separately
    // as lowarc:sessionOutput emits tagged with the same sessionId, and stop has nothing to report.
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
    // Unlike markDirty/requestClose/openFile, this one needs a real answer: a write can fail
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
    // Renders a menu on the real screen, outside this iframe's box, which a position:fixed element
    // here cannot escape. x and y are this document's own coordinates; the host translates them,
    // since only it knows where this iframe sits. items is [{label, value, disabled}], with
    // {divider: true} for a divider. Resolves to the chosen value, or null if dismissed.
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
    // Run-debugging primitives, generic rather than scoped to the first-party Debugger. Namespaced
    // for the same reason `session` is. Fire-and-forget: a failure like "no run is active" has
    // nothing for the caller to branch on, so the host surfaces it as a toast. setBreakpoints
    // always sends the WHOLE list, the same resend-everything convention settings and commands
    // use, which is one fewer state-sync mechanism to get wrong.
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
    // A plain DOM utility rather than a host round-trip: wires ".numeric-input" markup up with
    // working steppers. Without it the class would be buttons that do nothing on click, since a
    // native type="number" input's spin arrows cannot be restyled to match the app. A separate copy
    // of the host's initNumericInputs() on purpose, because this is the plugin-facing contract and
    // must not depend on a host script a plugin can never load. Dispatches both "input" and
    // "change", so a caller commits the same way it would for any other field.
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
    // Same data-tooltip convention as the host's initTooltips(), but NOT a byte-for-byte port. The
    // host's version prefers a side and falls back once, which is wrong in a narrow iframe: a fixed
    // element here can only paint inside THIS iframe's viewport, so a tooltip wider than the panel
    // gets cut off whichever side it picks. This clamps fully inside window.innerWidth and
    // innerHeight, and pairs with ".tooltip-popup"'s max-width and wrapping.
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
/// rules): the literal same file every host page (editor.html, settings.html, etc.) already
/// links, via `include_str!` so this can never drift from it. Served at `__lowarc.css`.
///
/// Opt-in: a plugin links it itself, nothing forces it. The `:root` block here is the DEFAULT
/// palette only. The resolved theme comes from `__lowarc-theme.css` (see plugin_asset_server.rs),
/// linked after this one. That covers initial load; a theme changed while a plugin is open arrives
/// over the harness's `lowarc:theme` message, since the host cannot make a sandboxed iframe
/// re-fetch a stylesheet without reloading it.
pub const SHARED_STYLE_CSS: &str = include_str!("../../src/style.css");

/// The host's genuinely reusable component styles — buttons, text/numeric inputs, checkboxes, a
/// progress bar, setting rows. Split out of primitives.css so it could be exposed here, via
/// `include_str!` so it is one real file rather than a copy. Served at `__lowarc-primitives.css`,
/// opt-in like `__lowarc.css`. Deliberately NOT the larger primitives.css, which is full of
/// host-chrome classes assuming the host's DOM and JS: a plugin could not reuse them without
/// reimplementing that JS. Depends on `__lowarc.css`'s tokens, so link both, that one first.
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
/// for this specifically) with its own small standalone behavior. See dropdown-shared.js's own
/// header for why this isn't just reusing the host's fuller-featured one. Served at
/// __lowarc-dropdown.js; exposes one function, `createLowarcDropdown(options, value, onChange)`.
pub const SHARED_DROPDOWN_JS: &str = include_str!("../../src/dropdown-shared.js");

/// Compact number formatting ("1.4K", "10K", "1M") built on Intl.NumberFormat — see
/// format-shared.js's own header. Served at __lowarc-format.js; exposes one function,
/// `lowarcFormatCompact(n)`.
pub const SHARED_FORMAT_JS: &str = include_str!("../../src/format-shared.js");

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
