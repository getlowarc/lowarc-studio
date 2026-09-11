      // ---------- Split editor ----------
      // applySplitRatio() is deliberately not here but in panels.js, beside the splitOpen/splitRatio
      // state it reads — see its own comment there for why its position is load-order-sensitive.

      async function setSplitOpen(open) {
        if (splitOpen === open) return;
        splitOpen = open;
        document.getElementById("center-panel").classList.toggle("is-split", open);
        document.getElementById("toggle-split").classList.toggle("is-active", open);
        applySplitRatio();

        if (!open) {
          // Closing the split is a layout change, not "discard everything in the second group" —
          // move its files back into group 0 (via moveFileToGroup, so each one's current content
          // — possibly unsaved — survives the move the same way a single dragged-across tab does)
          // rather than closing them. moveFileToGroup already does its own per-file
          // renderTabBar/showActiveFile/setActiveGroup; group 1's own tab bar just needs one final
          // pass afterward since every one of its tabs is gone by then.
          const toMove = Array.from(openFiles.keys()).filter((p) => openFiles.get(p).groupId === 1);
          for (const path of toMove) {
            await moveFileToGroup(path, 0);
          }
          groupActiveFilePath[1] = null;
          renderTabBar(1);
        }

        savePanelLayout();
      }

      function toggleSplit() {
        setSplitOpen(!splitOpen);
      }

      document.getElementById("toggle-split").addEventListener("click", toggleSplit);
      document.getElementById("view-toggle-split").addEventListener("click", toggleSplit);

      // Same Pointer Capture reasoning as initDividerDrag below — this divider sits directly
      // beside a group that can hold a sandboxed iframe (Monaco, most often), and a plain
      // document-level mousemove would stop responding the instant the drag crosses into one.
      function initSplitDividerDrag() {
        const el = document.getElementById("editor-group-divider");
        const group0 = document.getElementById("editor-group-0");
        const centerPanel = document.getElementById("center-panel");
        const minPx = 160;

        el.addEventListener("pointerdown", (e) => {
          if (!splitOpen) return;
          e.preventDefault();
          el.setPointerCapture(e.pointerId);
          const startX = e.clientX;
          const totalWidth = centerPanel.getBoundingClientRect().width;
          const startWidth = group0.getBoundingClientRect().width;

          el.classList.add("is-dragging");
          document.body.style.cursor = "col-resize";
          document.body.style.userSelect = "none";

          const onMove = (moveEvent) => {
            const delta = moveEvent.clientX - startX;
            const rawWidth = Math.max(minPx, Math.min(totalWidth - minPx, startWidth + delta));
            splitRatio = rawWidth / totalWidth;
            group0.style.flex = `0 0 ${splitRatio * 100}%`;
          };

          const onUp = (upEvent) => {
            el.releasePointerCapture(upEvent.pointerId);
            el.classList.remove("is-dragging");
            document.body.style.cursor = "";
            document.body.style.userSelect = "";
            el.removeEventListener("pointermove", onMove);
            el.removeEventListener("pointerup", onUp);
            el.removeEventListener("pointercancel", onUp);
            savePanelLayout();
          };

          el.addEventListener("pointermove", onMove);
          el.addEventListener("pointerup", onUp);
          // pointercancel (lost capture — a system dialog, alt-tab, touch/pen interruption) gets
          // the exact same cleanup as pointerup: the ratio just stays wherever the drag left it,
          // same as a normal release, nothing to revert.
          el.addEventListener("pointercancel", onUp);
        });
      }
      initSplitDividerDrag();

      // Relays a plugin calling lowarc.call(method, params) into that plugin's process via
      // invoke_plugin, and the reply back. Trust comes from windowToPlugin, keyed by event.source,
      // which a message cannot lie about, never from anything the payload claims. A second branch
      // below handles "host" messages the same way, routed to a tab instead of a process.
      //
      // The origin check below compares against the literal string "null", NOT against the asset
      // server's http://127.0.0.1:<port> origin. sandbox="allow-scripts" without allow-same-origin
      // forces the frame into an opaque origin, and every browser reports that as "null" on
      // postMessage whatever URL served the content. Comparing against the real origin would
      // reject every plugin message. This is defence in depth only; the identity check is
      // windowToPlugin below, keyed by the unforgeable event.source.
      window.addEventListener("message", async (event) => {
        const data = event.data;
        if (!data || typeof data !== "object") return;
        if (event.origin !== "null") return;

        // Every plugin iframe reports its own pointerdowns (see the harness in plugin_assets.rs
        // for why focus alone wasn't enough) purely so the host's overlays can dismiss on a click
        // that lands inside one. No windowToPlugin check: the origin check above already proves it
        // came from a sandboxed frame, and "close the menus" is not an authority worth gating —
        // the worst a forged one can do is close a menu the user was about to click.
        if (data.type === "pointerdown") {
          closeAllOverlays();
          return;
        }

        // A plugin replying to a HOST-initiated request — the only such request today is
        // requestPluginContent() (see moveFileToGroup) — not one of the usual plugin-initiated
        // "call"/"host" messages, so it gets its own type rather than overloading "reply" (which
        // means something different: a plugin's OWN outgoing call() awaiting an answer).
        if (data.type === "hostRequestReply") {
          const resolve = hostPendingRequests.get(data.replyId);
          if (resolve) {
            hostPendingRequests.delete(data.replyId);
            resolve(data.content);
          }
          return;
        }

        if (data.type === "host") {
          // openFile isn't "about" a file that's already open the way markDirty/requestClose
          // are, so it doesn't route through windowToFilePaths — any tracked plugin iframe (a
          // sidebar panel like the file explorer, not just a viewer) can ask the host to add a
          // path to the open-files list. windowToPlugin is still the trust check: only a real,
          // currently-mounted plugin iframe can trigger it, never an arbitrary postMessage.
          if (data.action === "openFile") {
            if (windowToPlugin.get(event.source) && typeof data.path === "string" && data.path) {
              openFile(data.path, { openInSplit: Boolean(data.openInSplit) });
            }
            return;
          }

          // window.lowarc.refreshFile() — same trust shape as openFile above (any mounted plugin,
          // not scoped to the file's own viewer via windowToFilePaths, since the CALLER here is
          // never the viewer itself — it's whatever changed the file out from under it).
          if (data.action === "refreshFile") {
            if (windowToPlugin.get(event.source) && typeof data.path === "string" && data.path) {
              refreshOpenFile(data.path);
            }
            return;
          }

          // window.lowarc.setDiffStatus() — same trust shape as refreshFile above.
          if (data.action === "setDiffStatus") {
            if (windowToPlugin.get(event.source)) {
              setDraftDiffStatus(data.diffCounts && typeof data.diffCounts === "object" ? data.diffCounts : {});
            }
            return;
          }

          // window.lowarc.pickOpenFile()/createFile() — a plugin has no filesystem access of its
          // own to browse or write with, so these relay to the host's real native file dialogs
          // (openDialog/saveDialog, the same ones the File menu itself uses) rather than giving a
          // plugin raw FS access. Generic, like showMenu/getSettings: any mounted plugin can call
          // either, not just the one that motivated adding them (Node Graph's own sidebar panel).
          if (data.action === "pickOpenFile") {
            if (!windowToPlugin.get(event.source)) return;
            openDialog(data.opts || {})
              .then((path) => event.source.postMessage({ type: "reply", id: data.id, ok: true, result: path || null }, "*"))
              .catch((err) => event.source.postMessage({ type: "reply", id: data.id, ok: false, error: String(err) }, "*"));
            return;
          }
          if (data.action === "createFile") {
            if (!windowToPlugin.get(event.source)) return;
            saveDialog(data.opts || {})
              .then(async (path) => {
                if (!path) {
                  event.source.postMessage({ type: "reply", id: data.id, ok: true, result: null }, "*");
                  return;
                }
                await invoke("write_text_file", { path, contents: (data.opts && data.opts.contents) || "" });
                event.source.postMessage({ type: "reply", id: data.id, ok: true, result: path }, "*");
              })
              .catch((err) => event.source.postMessage({ type: "reply", id: data.id, ok: false, error: String(err) }, "*"));
            return;
          }

          // window.lowarc.openInspector() — finds the CALLING plugin's own inspector contribution
          // (never an arbitrary one named by the message, so a plugin can only ever show its own
          // content, never someone else's) and claims the slot with it. A plugin with no inspector
          // contribution at all is a silent no-op, same shape as showMenu/openPopup targeting
          // something that doesn't exist.
          if (data.action === "openInspector") {
            const pluginId = windowToPlugin.get(event.source);
            if (!pluginId) return;
            const contribution = getSlot("inspector").find((c) => c.pluginId === pluginId);
            if (contribution) showInspector(contribution.id, data.context, Boolean(data.onlyIfOpen));
            return;
          }

          // window.lowarc.broadcastToSelf() — relays to every OTHER mounted iframe of the SAME
          // plugin (never the sender, never a different plugin's iframe). iframe.plugin-panel-frame
          // is the one class every plugin-hosted iframe carries regardless of role (panel, viewer,
          // console tab — see iframeForWindow's own comment above), so this is the same enumeration
          // that already works for translating a plugin's showMenu() coordinates.
          if (data.action === "broadcastToSelf") {
            const pluginId = windowToPlugin.get(event.source);
            if (!pluginId) return;
            const message = { type: "emit", event: data.event, payload: data.payload };
            document.querySelectorAll("iframe.plugin-panel-frame").forEach((iframe) => {
              if (iframe.contentWindow === event.source) return;
              if (windowToPlugin.get(iframe.contentWindow) === pluginId) {
                iframe.contentWindow.postMessage(message, "*");
              }
            });
            return;
          }

          // window.lowarc.editFile() — a plugin that isn't the file's own viewer (Outline) asking
          // whatever IS to apply an edit. Same trust shape as openFile: any tracked plugin iframe
          // can ask, but only for a path that's actually open right now, forwarded to that path's
          // real owning instance rather than trusted blindly.
          if (data.action === "editFile") {
            if (
              windowToPlugin.get(event.source) &&
              typeof data.path === "string" &&
              typeof data.line === "number" &&
              typeof data.text === "string"
            ) {
              const file = openFiles.get(data.path);
              if (file && file.iframe) {
                file.iframe.contentWindow.postMessage(
                  { type: "emit", event: "lowarc:applyLineEdit", payload: { path: data.path, line: data.line, text: data.text } },
                  "*",
                );
              }
            }
            return;
          }

          // window.lowarc.notify() — routes into the exact same showToast() every host-chrome
          // error/confirmation already goes through (see primitives.js's toast/notification-
          // history section), tagged with which plugin it came from so the bell's history can
          // attribute it. Not routed through windowToFilePaths/windowToPlugin's file-specific
          // lookups — any tracked plugin iframe can raise one, not just a file viewer.
          if (data.action === "notify") {
            const pluginId = windowToPlugin.get(event.source);
            if (pluginId && typeof data.message === "string" && data.message) {
              const variant = ["info", "warning", "success", "error"].includes(data.variant) ? data.variant : "info";
              showToast({ variant, message: data.message, source: pluginId });
            }
            return;
          }

          // Same trust/routing shape as openFile — these come from whatever plugin performed the
          // filesystem operation (the file explorer, today), not from the affected file's own
          // viewer iframe, so they're keyed by windowToPlugin and an explicit path lookup rather
          // than windowToFilePaths. Only exact-path matches are handled: a folder rename/move/
          // delete doesn't currently cascade to mark every open file underneath it — a deliberate
          // scope cut, not an oversight.
          if (data.action === "notifyPathDeleted") {
            if (windowToPlugin.get(event.source) && typeof data.path === "string") {
              const file = openFiles.get(data.path);
              if (file) {
                file.missing = true;
                renderTabBar(file.groupId);
                broadcastFileStatus();
              }
            }
            return;
          }
          if (data.action === "notifyPathRenamed") {
            if (windowToPlugin.get(event.source) && typeof data.oldPath === "string") {
              const file = openFiles.get(data.oldPath);
              if (file) {
                file.missing = true;
                renderTabBar(file.groupId);
                broadcastFileStatus();
              }
            }
            return;
          }
          // All three routed by windowToPlugin, not windowToFilePaths — a session-mode plugin's UI
          // (a console-tab panel, not a file viewer) manages its own instances, each identified by
          // a sessionId it minted itself; the host just needs to know which plugin's files/backend
          // that id belongs to, the same trust boundary as everything else here.
          if (data.action === "startSession") {
            const pluginId = windowToPlugin.get(event.source);
            if (pluginId && typeof data.sessionId === "string") {
              invoke("start_plugin_session", { id: pluginId, sessionId: data.sessionId, shell: data.shell ?? null }).catch((err) => {
                showToast({ variant: "error", message: String(err) });
              });
            }
            return;
          }
          if (data.action === "sendSession") {
            const pluginId = windowToPlugin.get(event.source);
            if (pluginId && typeof data.sessionId === "string") {
              invoke("send_to_plugin_session", { id: pluginId, sessionId: data.sessionId, message: data.message ?? null }).catch((err) => {
                showToast({ variant: "error", message: String(err) });
              });
            }
            return;
          }
          if (data.action === "stopSession") {
            const pluginId = windowToPlugin.get(event.source);
            if (pluginId && typeof data.sessionId === "string") {
              invoke("stop_plugin_session", { id: pluginId, sessionId: data.sessionId });
            }
            return;
          }
          // window.lowarc.showMenu() — see plugin_assets.rs's harness. data.x/data.y are in the
          // calling iframe's OWN document coordinates (typically a right-click's clientX/clientY);
          // this iframe could be mounted in either editor group's viewport, a sidebar/inspector
          // panel, or a console tab, so its screen position isn't fixed — iframeForWindow() finds
          // the actual element and its current getBoundingClientRect() gives the real offset to
          // add. Trust is windowToPlugin, same as every other action here: only a real, currently-
          // mounted plugin iframe can trigger this.
          if (data.action === "showMenu") {
            if (windowToPlugin.get(event.source) && Array.isArray(data.items)) {
              const iframeEl = iframeForWindow(event.source);
              const rect = iframeEl ? iframeEl.getBoundingClientRect() : { left: 0, top: 0 };
              const anchor = { x: rect.left + (Number(data.x) || 0), y: rect.top + (Number(data.y) || 0) };
              openMenuOverlay(data.items, anchor).then((value) => {
                event.source.postMessage({ type: "reply", id: data.id, ok: true, result: value }, "*");
              });
            }
            return;
          }
          // window.lowarc.openPopup() — see plugin_assets.rs's harness. Unlike showMenu, a popup
          // is always centered on the real window rather than anchored to the calling iframe, so
          // there's no position to translate here. popupId must name a popup some contribution
          // already registered via contribute("popups", ...) — no plugin declares one yet, so this
          // is proven the same way notify() was: real, but unexercised until one has a reason to.
          // Same trust (windowToPlugin) and reply-routing (event.source, never broadcast) as every
          // other action here — the result goes back to the exact iframe that asked.
          if (data.action === "openPopup") {
            if (windowToPlugin.get(event.source) && typeof data.popupId === "string") {
              showPopup(data.popupId, data.target)
                .then((result) => event.source.postMessage({ type: "reply", id: data.id, ok: true, result }, "*"))
                .catch((err) => event.source.postMessage({ type: "reply", id: data.id, ok: false, error: String(err) }, "*"));
            }
            return;
          }
          // window.lowarc.getSettings() — see plugin_assets.rs's harness. windowToPlugin is the
          // trust boundary: a plugin only ever gets ITS OWN saved settings back, since pluginId
          // here comes from the map this iframe was mounted under, never from the message itself.
          if (data.action === "getSettings") {
            const pluginId = windowToPlugin.get(event.source);
            if (pluginId) {
              invoke("get_plugin_settings", { id: pluginId })
                .then((result) => event.source.postMessage({ type: "reply", id: data.id, ok: true, result }, "*"))
                .catch((err) => event.source.postMessage({ type: "reply", id: data.id, ok: false, error: String(err) }, "*"));
            }
            return;
          }
          // window.lowarc.debug.pause/resume/step/setBreakpoints() — see plugin_assets.rs's
          // harness. Generic, like every other action here: any mounted plugin can call these, not
          // just the first-party Debugger one. Fire-and-forget; a failure (most commonly "no run
          // is active") surfaces as the host's own toast, same as session.start's error handling.
          if (data.action === "pauseRun") {
            if (windowToPlugin.get(event.source)) {
              invoke("pause_dev_run").then(() => setRunPaused(true)).catch((err) => showToast({ variant: "error", message: String(err) }));
            }
            return;
          }
          if (data.action === "resumeRun") {
            if (windowToPlugin.get(event.source)) {
              invoke("resume_dev_run").then(() => setRunPaused(false)).catch((err) => showToast({ variant: "error", message: String(err) }));
            }
            return;
          }
          if (data.action === "stepRun") {
            if (windowToPlugin.get(event.source)) {
              invoke("step_dev_run", { count: Number(data.count) || 1 }).catch((err) => showToast({ variant: "error", message: String(err) }));
            }
            return;
          }
          if (data.action === "setBreakpoints") {
            if (windowToPlugin.get(event.source) && Array.isArray(data.breakpoints)) {
              invoke("set_breakpoints", { breakpoints: data.breakpoints }).catch((err) => showToast({ variant: "error", message: String(err) }));
            }
            return;
          }
          // window.lowarc.setCommands() — see plugin_assets.rs's harness.
          if (data.action === "setCommands") {
            const pluginId = windowToPlugin.get(event.source);
            if (pluginId && Array.isArray(data.commands)) {
              setDynamicPluginCommands(pluginId, data.commands);
            } else {
              showToast({ variant: "error", message: `Palette: setCommands rejected (pluginId=${pluginId}, isArray=${Array.isArray(data.commands)})` });
            }
            return;
          }

          // Every action below is "about" one specific open file — markDirty/markErrors/
          // requestClose/saveFile all carry an explicit path now (see plugin_assets.rs), since one
          // iframe can be responsible for several at once. windowToFilePaths is what validates a
          // plugin can only claim a path the host itself actually told it to open — not proof of
          // WHICH file (there's no single one anymore), just that this iframe is allowed to speak
          // for that path at all.
          const claimedPaths = windowToFilePaths.get(event.source);
          if (!claimedPaths || typeof data.path !== "string" || !claimedPaths.has(data.path)) return;
          const path = data.path;
          if (data.action === "markDirty") {
            const file = openFiles.get(path);
            if (file) {
              file.dirty = Boolean(data.dirty);
              renderTabBar(file.groupId);
              broadcastFileStatus();
            }
          } else if (data.action === "markErrors") {
            const file = openFiles.get(path);
            if (file) {
              file.hasErrors = Boolean(data.hasErrors);
              broadcastFileStatus();
            }
          } else if (data.action === "requestClose") {
            closeFile(path);
          } else if (data.action === "saveFile") {
            // Unlike markDirty/requestClose, this one owes the caller a reply (see saveFile()'s
            // own comment in __lowarc.js) — a write can fail, and the plugin needs to know before
            // it clears its own dirty state.
            try {
              await invoke("write_text_file", { path, contents: String(data.contents ?? "") });
              const file = openFiles.get(path);
              if (file) {
                // A successful write also clears "missing" — saving recreates the file at this
                // exact path if it had been deleted, which is a legitimate recovery, not a no-op.
                file.dirty = false;
                file.missing = false;
                renderTabBar(file.groupId);
                broadcastFileStatus();
                if (path === groupActiveFilePath[activeGroupId]) updateInspectorForActiveFile();
              }
              event.source.postMessage({ type: "reply", id: data.id, ok: true }, "*");
            } catch (err) {
              showToast({ variant: "error", message: String(err) });
              event.source.postMessage({ type: "reply", id: data.id, ok: false, error: String(err) }, "*");
            }
          }
          return;
        }

        const pluginId = windowToPlugin.get(event.source);
        if (!pluginId || data.type !== "call") return;

        let reply;
        try {
          const result = await invoke("invoke_plugin", { id: pluginId, method: data.method, params: data.params ?? null });
          reply = { type: "reply", id: data.id, ok: true, result };
        } catch (err) {
          reply = { type: "reply", id: data.id, ok: false, error: String(err) };
        }
        event.source.postMessage(reply, "*");
      });

      // Plugins are invoked per call now, not kept running (see plugin_host/protocol.rs) — a
      // backend cannot push something unprompted at an arbitrary later time, since nothing
      // persists between calls. It CAN ride an "emit" along on a call's own reply
      // though (invoke_plugin in lib.rs re-fires it as this event), so a plugin can tell its own
      // other panels/viewers "something changed" as a side effect of whatever it was just asked
      // to do — still call-triggered, just not truly live.
      window.__TAURI__.event.listen("plugin-emit", (event) => {
        const { pluginId, event: eventName, payload } = event.payload;
        for (const entry of pluginPanels.values()) {
          if (entry.pluginId === pluginId && entry.iframe) {
            entry.iframe.contentWindow.postMessage({ type: "emit", event: eventName, payload }, "*");
          }
        }
        for (const file of openFiles.values()) {
          if (file.pluginId === pluginId && file.iframe) {
            file.iframe.contentWindow.postMessage({ type: "emit", event: eventName, payload }, "*");
          }
        }
      });

      // The theme just changed in THIS document (theme.js announces every apply). A plugin runs in
      // a sandboxed iframe: it got the resolved theme from __lowarc-theme.css when it loaded, but it
      // has no way to notice a later change, and reloading it to pick one up would throw away
      // whatever it was showing — an editor's unsaved buffer, a terminal's scrollback. So the
      // resolved values are pushed straight in and applied as custom properties.
      //
      // Read off documentElement's inline style rather than a token list, because that IS what
      // applyThemeColors wrote — including derived values like --btn-hover-brightness that aren't
      // colors at all and would be forgotten by anything maintaining its own list.
      window.addEventListener("lowarc-theme-applied", () => {
        const style = document.documentElement.style;
        const vars = {};
        for (const name of style) {
          if (name.startsWith("--")) vars[name] = style.getPropertyValue(name);
        }
        const message = { type: "theme", vars };

        const notified = new Set();
        const push = (iframe) => {
          if (!iframe || notified.has(iframe)) return;
          notified.add(iframe);
          iframe.contentWindow.postMessage(message, "*");
        };
        for (const entry of pluginPanels.values()) push(entry.iframe);
        for (const file of openFiles.values()) push(file.iframe);
      });

      // A setting was changed in settings.html (a separate window/document, so it can't ride
      // plugin-emit — that one only fires as a side effect of invoke_plugin, and no plugin call
      // happens here) — see set_plugin_setting in lib.rs. Relayed the same way plugin-emit relays
      // a plugin's own emit: filtered to that plugin's own mounted panels/files only, so a plugin
      // doesn't need to know its own id to tell whether an incoming settingsChanged is for it.
      window.__TAURI__.event.listen("plugin-setting-changed", (event) => {
        const { id: pluginId, key, value } = event.payload;
        const payload = { type: "emit", event: "lowarc:settingsChanged", payload: { key, value } };
        for (const entry of pluginPanels.values()) {
          if (entry.pluginId === pluginId && entry.iframe) {
            entry.iframe.contentWindow.postMessage(payload, "*");
          }
        }
        const notified = new Set();
        for (const file of openFiles.values()) {
          if (file.pluginId === pluginId && file.iframe && !notified.has(file.iframe)) {
            notified.add(file.iframe);
            file.iframe.contentWindow.postMessage(payload, "*");
          }
        }
      });

      // The generic "a save is about to overwrite this file" hook (see write_text_file in lib.rs)
      // — broadcast to every mounted plugin iframe, same shape as broadcastFileStatus()
      // (tabs-inspector.js), just Rust-originated instead of client-state-originated. Host code has
      // no notion of which plugin (if any) cares — Offshoot is the only one listening today, but
      // nothing here says so.
      window.__TAURI__.event.listen("file-about-to-save", (event) => {
        const payload = { type: "emit", event: "lowarc:beforeSave", payload: event.payload };
        for (const entry of pluginPanels.values()) {
          if (entry.iframe) entry.iframe.contentWindow.postMessage(payload, "*");
        }
        const notified = new Set();
        for (const file of openFiles.values()) {
          if (notified.has(file.iframe)) continue;
          notified.add(file.iframe);
          file.iframe.contentWindow.postMessage(payload, "*");
        }
      });

      renderTabBar(0);
      renderTabBar(1);
      loadPluginContributions();

      // ---------- Drag-to-resize, with VS Code's two-stage close ----------
      // Dragging past the minimum doesn't close the panel right away — it sticks at the minimum
      // size (a dead zone) until the drag reaches a second, closer-to-the-edge threshold
      // (closeAt), only then does it actually collapse. Below closeAt, the panel goes visually
      // closed (0) but stays "live" — dragging back out past closeAt reopens it pinned at the
      // minimum, no need to release and start a new drag. While fully closed, the remembered size
      // is left untouched, so reopening later (via the header icon or the rail) restores the size
      // from before the drag, not 0 or whatever the mouse last happened to be at.

      // Pointer Capture, not plain mousedown/mousemove/mouseup on document — every panel this
      // resizes sits right next to a sandboxed iframe (the file explorer, Monaco, Terminal...),
      // and a plain document-level mousemove listener stops receiving events the instant the
      // cursor crosses into one, since the iframe has its own separate document/event target.
      // Confirmed live: dragging past an iframe's edge would just silently stop responding,
      // making a panel impossible to shrink whenever the mouse happened to cross one — a
      // pointerdown that setPointerCapture()s on the divider itself keeps *all* subsequent
      // pointer events routed to that element regardless of what's visually underneath the
      // cursor, iframe or not, which is exactly the guarantee this needs.
      function initDividerDrag(el, key, axis, invert, sizeEl) {
        const p = PANELS[key];

        el.addEventListener("pointerdown", (e) => {
          e.preventDefault();
          el.setPointerCapture(e.pointerId);
          const startPos = axis === "x" ? e.clientX : e.clientY;
          const startRect = sizeEl.getBoundingClientRect();
          const startSize = p.open ? (axis === "x" ? startRect.width : startRect.height) : 0;

          el.classList.add("is-dragging");
          document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
          document.body.style.userSelect = "none";

          const onMove = (moveEvent) => {
            const pos = axis === "x" ? moveEvent.clientX : moveEvent.clientY;
            const delta = (pos - startPos) * (invert ? -1 : 1);
            const raw = Math.max(0, Math.min(p.max, startSize + delta));

            if (raw < p.closeAt) {
              p.open = false;
            } else if (raw < p.min) {
              p.open = true;
              p.size = p.min;
            } else {
              p.open = true;
              p.size = raw;
            }
            applyPanel(key);
            syncToggleUI();
          };

          const onUp = (upEvent) => {
            el.releasePointerCapture(upEvent.pointerId);
            el.classList.remove("is-dragging");
            document.body.style.cursor = "";
            document.body.style.userSelect = "";
            el.removeEventListener("pointermove", onMove);
            el.removeEventListener("pointerup", onUp);
            el.removeEventListener("pointercancel", onUp);
            savePanelLayout();
          };

          el.addEventListener("pointermove", onMove);
          el.addEventListener("pointerup", onUp);
          // Same reasoning as initSplitDividerDrag's own pointercancel handling above.
          el.addEventListener("pointercancel", onUp);
        });
      }

      initDividerDrag(document.querySelector(".divider-left"), "sidebar", "x", false, document.querySelector(".left-panel"));
      initDividerDrag(document.querySelector(".divider-right"), "right", "x", true, document.querySelector(".right-panel"));
      initDividerDrag(document.querySelector(".divider-console"), "console", "y", true, document.querySelector(".console-panel"));
