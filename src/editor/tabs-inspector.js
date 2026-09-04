      // ---------- Tab bar / open files (one or two editor groups — see toggleSplit below) ----------
      // openFiles: absolute path -> { title, dirty, missing, hasErrors, pluginId, iframe, groupId }.
      // ONE iframe per (editor group, viewer plugin) pair, not one per open file — a "multi-document
      // viewer" plugin (Monaco, today) manages several open files inside that one instance itself
      // (see the lowarc:openFile/activateFile/closeFile/getContent contract in plugin_assets.rs),
      // the same shape a "session": true plugin like Terminal already uses for multiple terminal
      // tabs inside one iframe — the host never needs to know a plugin has "instances," only which
      // path is currently active. Per-file scroll position/undo history survive a tab switch
      // because the PLUGIN itself saves/restores view state around each activateFile, the same way
      // VS Code's own editor does — not because the DOM node never went away.
      // Unlike VS Code, a file isn't shared across groups here — there's no way to share a model
      // between two separate sandboxed iframes (two different JS realms) to split — so a path can
      // only be open in ONE group at a time; opening an already-open file just activates it
      // wherever it already is, and moving it to the other group asks the source instance for its
      // current content (see requestPluginContent below) rather than re-reading the file from disk,
      // so unsaved edits survive the move.
      const openFiles = new Map();
      // iframe.contentWindow -> Set<path> this iframe is currently responsible for. A plugin's own
      // markDirty/markErrors/requestClose/saveFile calls all carry an explicit path now (see
      // plugin_assets.rs) — this Set is what validates that path actually belongs to the iframe
      // claiming it, the same trust role a single Map<Window, path> played before multiple files
      // could share one iframe.
      const windowToFilePaths = new Map();
      // "${groupId}:${pluginId}" -> the one iframe currently mounted for that plugin in that group,
      // if any — what openFile()/moveFileToGroup() check before deciding whether to reuse an
      // already-mounted multi-document viewer or create a fresh one.
      const groupViewerIframes = new Map();
      function viewerIframeKey(groupId, pluginId) {
        return `${groupId}:${pluginId}`;
      }
      const groupActiveFilePath = { 0: null, 1: null };
      let activeGroupId = 0;

      // The reverse of call()/saveFile()'s own request/reply plumbing (see plugin_assets.rs) — this
      // is the one place the HOST initiates a request a plugin must reply to, used by
      // moveFileToGroup() to pull a file's current (possibly unsaved) content out of its source
      // instance before mounting it in the other group. A plugin replies by posting
      // {type: "hostRequestReply", replyId, content} — matched in the message listener below.
      // A misbehaving (or just slow/stuck) plugin never replying would otherwise hang whatever
      // was awaiting it forever — moveFileToGroup's own move, or now the Outline panel's refresh.
      // Resolving null on timeout instead of leaving that stuck is the whole reason for it; both
      // callers already treat a null/falsy content result as "fall back to reading from disk."
      const REQUEST_CONTENT_TIMEOUT_MS = 2000;
      let nextHostRequestId = 1;
      const hostPendingRequests = new Map();
      function requestPluginContent(iframe, path) {
        return new Promise((resolve) => {
          const id = nextHostRequestId++;
          let settled = false;
          const finish = (value) => {
            if (settled) return;
            settled = true;
            hostPendingRequests.delete(id);
            resolve(value);
          };
          hostPendingRequests.set(id, finish);
          iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:getContent", payload: { path, replyId: id } }, "*");
          setTimeout(() => finish(null), REQUEST_CONTENT_TIMEOUT_MS);
        });
      }

      function groupEls(id) {
        return {
          tabBar: document.getElementById(`tab-bar-tabs-${id}`),
          viewport: document.getElementById(`center-viewport-${id}`),
          empty: document.getElementById(`center-viewport-empty-${id}`),
        };
      }

      // Tracks which group new files open into and which group's tab bar gets the "active" shade
      // (see .editor-group.is-active-group in the CSS above) — updated on any real interaction
      // with a group: clicking one of its tabs (via the per-group tab-change listener below),
      // clicking anywhere else in its chrome (this listener), or a viewer iframe inside it
      // receiving focus (see the iframe "focus" listener in openFile()).
      function setActiveGroup(id) {
        if (activeGroupId === id) return;
        activeGroupId = id;
        document.getElementById("editor-group-0").classList.toggle("is-active-group", id === 0);
        document.getElementById("editor-group-1").classList.toggle("is-active-group", id === 1);
        updateInspectorForActiveFile();
      }
      for (const id of [0, 1]) {
        document.getElementById(`editor-group-${id}`).addEventListener("mousedown", () => setActiveGroup(id));
      }

      // ---------- Inspector: context-based, empty until claimed ----------
      // Unlike the sidebar/console, the Inspector has no permanent icon or tab of its own for a
      // person to click — nothing mounts here and nothing is visible until some plugin actually
      // asks for it (window.lowarc.openInspector(), see the "openInspector" host-action handler
      // below), which is exactly what showSlotTab's own lazy-mount-on-first-show already does for
      // the sidebar and console — this just reuses that instead of the old single-slot, eager-
      // mount-at-startup path Inspector used to have to itself. Multiple plugins can register an
      // inspector contribution; activeInspectorKey is just "whichever one is currently showing",
      // the same role activeConsoleTabKey already plays for the console.
      let activeInspectorKey = null;

      // onlyIfOpen: skip entirely (don't switch content, don't open the panel) unless the panel is
      // already open — see window.lowarc.openInspector()'s own comment in plugin_assets.rs for why.
      function showInspector(contributionId, context, onlyIfOpen) {
        if (onlyIfOpen && !PANELS.right.open) return;
        const contribution = getSlot("inspector").find((c) => c.id === contributionId);
        if (!contribution) return;
        activeInspectorKey = contributionId;
        showSlotTab("inspector", "right-panel-body", contributionId);
        if (!onlyIfOpen) setPanelOpen("right", true);
        const entry = pluginPanels.get(contributionId);
        if (entry && entry.iframe && context !== undefined) {
          entry.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:inspectorContext", payload: context }, "*");
        }
      }

      // "The inspector's iframe" is just whichever contribution is currently showing, if any — no
      // installed inspector plugin at all, or one that's simply never been claimed yet, is a
      // legitimate, common state, not an error.
      function inspectorIframe() {
        if (!activeInspectorKey) return null;
        const entry = pluginPanels.get(activeInspectorKey);
        return entry ? entry.iframe : null;
      }

      // Tells whatever's in the Inspector slot (Outline, today) what the currently-focused file
      // actually is — null when there isn't one, or its viewer is a binary one (an image has
      // nothing for a symbols outline to extract). Live content, not a disk read: asks the owning
      // viewer instance directly (same requestPluginContent() moveFileToGroup uses), since an
      // outline that only ever reflected the last save would silently drift from what's actually
      // on screen the moment someone starts editing — falls back to a disk read only if that
      // times out or the viewer doesn't implement lowarc:getContent at all. Deliberately NOT wired
      // to every keystroke — only file focus changes and successful saves trigger this, matching
      // the scope this was actually built to.
      async function updateInspectorForActiveFile() {
        const iframe = inspectorIframe();
        if (!iframe) return;
        const path = groupActiveFilePath[activeGroupId];
        const file = path ? openFiles.get(path) : null;
        const viewer = path ? viewersByExtension.get(fileExtension(path)) : null;
        if (!file || !viewer || viewer.viewer.binary) {
          iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:activeFile", payload: null }, "*");
          return;
        }
        const live = file.iframe ? await requestPluginContent(file.iframe, path) : null;
        const contents = live ?? (await invoke("read_text_file", { path }).catch(() => null));
        iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:activeFile", payload: contents == null ? null : { path, contents } }, "*");
      }

      // Pushed to every currently-mounted plugin iframe (sidebar/inspector panels *and* open-file
      // viewers, in either group) whenever anything about openFiles changes — the file explorer is
      // the reason this exists (so it can decorate its own tree rows with the same dirty/error
      // status the tab bar shows), but it's not addressed to any one plugin, since anything
      // sidebar-hosted might reasonably want it later. Sent as a full snapshot rather than a
      // delta: simpler, and cheap at the scale of "files a person has open in one project."
      function broadcastFileStatus() {
        const status = {};
        for (const [path, file] of openFiles) {
          status[path] = { dirty: file.dirty, missing: file.missing, hasErrors: file.hasErrors };
        }
        const payload = { type: "emit", event: "lowarc:fileStatus", payload: status };
        for (const entry of pluginPanels.values()) {
          if (entry.iframe) entry.iframe.contentWindow.postMessage(payload, "*");
        }
        // A Set, not a direct iteration over openFiles — several open files can now share one
        // iframe (see groupViewerIframes above), and that instance only needs this once, not once
        // per file it happens to have open.
        const notified = new Set();
        for (const file of openFiles.values()) {
          if (notified.has(file.iframe)) continue;
          notified.add(file.iframe);
          file.iframe.contentWindow.postMessage(payload, "*");
        }
      }

      function fileTitle(path) {
        return path.split(/[\\/]/).pop() || path;
      }

      function fileExtension(path) {
        const name = fileTitle(path);
        const dot = name.lastIndexOf(".");
        return dot === -1 ? "" : name.slice(dot).toLowerCase();
      }

      // #tab-bar-tabs-0/#tab-bar-tabs-1 are each their own "center-0"/"center-1" slot (see
      // contribute()/getSlot() in primitives.js) — a file's contribution is registered in
      // openFile() and removed in closeFile()/setSplitOpen(), both alongside (not instead of) the
      // matching openFiles.set()/delete(), since openFiles stays the authoritative store for the
      // MUTABLE per-file state (dirty/missing/hasErrors) a contribution deliberately doesn't carry
      // — decorate() below reads that live, straight from openFiles, on every render. groupId
      // selects which group's tab bar (and slot) to render; renderTabStrip is what actually builds
      // the tab elements, close buttons included (contribution.closeable:true is what triggers
      // those — see openFile()'s contribute() call).
      // Right-click on one open file's own tab — value handling shares closeGroupFiles/
      // closeSavedFiles with the tab-BAR's own context menu below (right-click on the empty strip
      // itself), since "close everything but this one"/"close everything already saved" mean the
      // same thing from either trigger.
      function showFileTabContextMenu(path, groupId, x, y) {
        openMenuOverlay(
          [
            { label: "Close", value: "close" },
            { label: "Close Others", value: "close-others" },
            { label: "Close Saved", value: "close-saved" },
            { label: "Close All", value: "close-all" },
            { divider: true },
            { label: "Copy Path", value: "copy-path" },
          ],
          { x, y }
        ).then((value) => {
          if (value === "close") closeFile(path);
          else if (value === "close-others") closeGroupFiles(groupId, path);
          else if (value === "close-saved") closeSavedFiles(groupId);
          else if (value === "close-all") closeGroupFiles(groupId);
          else if (value === "copy-path") {
            window.__TAURI__.clipboardManager.writeText(path).catch(() => showToast({ variant: "error", message: "Couldn't copy the path." }));
          }
        });
      }

      // Right-click on the tab bar's own empty space (not a specific tab) — no per-file actions,
      // just the group-wide ones, plus the same Split Editor toggle the View menu already has, for
      // discoverability right where a split would actually appear.
      function showTabBarContextMenu(groupId, x, y) {
        const hasFiles = Array.from(openFiles.values()).some((f) => f.groupId === groupId);
        openMenuOverlay(
          [
            { label: "Close All", value: "close-all", disabled: !hasFiles },
            { label: "Close Saved", value: "close-saved", disabled: !hasFiles },
            { divider: true },
            { label: "Split Editor", value: "split", checked: splitOpen },
          ],
          { x, y }
        ).then((value) => {
          if (value === "close-all") closeGroupFiles(groupId);
          else if (value === "close-saved") closeSavedFiles(groupId);
          else if (value === "split") toggleSplit();
        });
      }

      function renderTabBar(groupId) {
        const container = groupEls(groupId).tabBar;
        renderTabStrip(container, `center-${groupId}`, {
          itemClass: "file-tab",
          iconBased: false,
          emptyText: "No files open",
          decorate: (contribution, tab) => {
            const file = openFiles.get(contribution.id);
            if (!file) return;
            tab.title = contribution.id;
            const isActiveFile = contribution.id === groupActiveFilePath[groupId];
            tab.classList.toggle("is-active", isActiveFile);
            tab.setAttribute("aria-selected", String(isActiveFile));
            tab.classList.add(file.missing ? "status-missing" : file.dirty ? "status-dirty" : "status-clean");
            // Same per-language glyph the file explorer shows (window.lowarcIconClass, vendored
            // vendor/seti-icons/) — inserted first so it sits before the label, same reading order
            // as the explorer's own rows. Guarded the same way explorer.js guards it: if the icon
            // assets somehow aren't loaded, the tab just has no icon rather than an error.
            if (typeof window.lowarcIconClass === "function") {
              const icon = document.createElement("span");
              icon.className = "file-tab-icon file-icon " + window.lowarcIconClass(file.title);
              tab.insertBefore(icon, tab.firstChild);
            }
            // Stopped from bubbling to the tab bar's own contextmenu listener below — without
            // this, right-clicking a tab would show BOTH menus' worth of confusion (the container
            // listener firing right after this one resolves).
            tab.addEventListener("contextmenu", (e) => {
              e.preventDefault();
              e.stopPropagation();
              showFileTabContextMenu(contribution.id, groupId, e.clientX, e.clientY);
            });
          },
          onActivate: (contribution) => {
            groupActiveFilePath[groupId] = contribution.id;
            setActiveGroup(groupId);
            showActiveFile(groupId);
          },
          onClose: (contribution) => closeFile(contribution.id),
        });
      }

      for (const groupId of [0, 1]) {
        groupEls(groupId).tabBar.addEventListener("contextmenu", (e) => {
          e.preventDefault();
          showTabBarContextMenu(groupId, e.clientX, e.clientY);
        });
      }

      // iframe -> Promise<void>, resolved once that iframe is actually ready to receive messages
      // (its own script has run far enough to register a lowarc:openFile listener). A second file
      // opened into an already-mounted-but-still-loading instance has to wait on this before
      // posting — without it, a message sent between iframe creation and the listener existing is
      // simply gone (postMessage doesn't queue for a future listener), the same failure shape the
      // very first ensurePluginMounted() bug this session turned out to be.
      const viewerIframeReady = new Map();

      // Gets-or-creates the one iframe for (groupId, viewer.pluginId), pushes this path's content
      // into it, and resolves once it's genuinely ready — either immediately (an already-mounted
      // instance already has a load-complete script running) or after a freshly-created iframe's
      // own load event fires. Doesn't touch openFiles/contribute/groupActiveFilePath/rendering —
      // callers (openFile, moveFileToGroup) own that, since what happens around a mount differs
      // between "brand new file" and "moved from the other group."
      function mountFileInGroup(path, contents, viewer, groupId) {
        const key = viewerIframeKey(groupId, viewer.pluginId);
        const existingIframe = groupViewerIframes.get(key);

        if (existingIframe) {
          return viewerIframeReady.get(existingIframe).then(() => {
            windowToFilePaths.get(existingIframe.contentWindow).add(path);
            existingIframe.contentWindow.postMessage({ type: "emit", event: "lowarc:openFile", payload: { path, contents } }, "*");
            return existingIframe;
          });
        }

        return (async () => {
          const iframe = document.createElement("iframe");
          iframe.className = "plugin-panel-frame";
          iframe.dataset.pluginId = viewer.pluginId; // lets a command look up this plugin's own mounted iframe by id, see runCommand()
          iframe.setAttribute("sandbox", "allow-scripts");
          // No per-file path in the URL anymore — this instance can outlive any one file, so which
          // file(s) it's showing arrives entirely through lowarc:openFile/activateFile/closeFile
          // messages over its lifetime, not something baked into how it was loaded.
          iframe.src = await pluginAssetUrl(viewer.pluginId, viewer.viewer.entry);
          // A click inside a sandboxed iframe never bubbles to the parent document, so the
          // mousedown-based group-activation listener above can't see it — this is the
          // supplementary signal for "the user is now working in this group" once its content
          // actually takes focus (Monaco, e.g., calls focus() on the editor surface). Also closes
          // any open menu/dropdown/floating-menu for the same reason — see closeAllOverlays().
          iframe.addEventListener("focus", () => {
            setActiveGroup(groupId);
            closeAllOverlays();
          });
          groupEls(groupId).viewport.appendChild(iframe);
          windowToFilePaths.set(iframe.contentWindow, new Set([path]));
          windowToPlugin.set(iframe.contentWindow, viewer.pluginId);
          groupViewerIframes.set(key, iframe);

          const ready = new Promise((resolve) => {
            // The viewer's own iframe has no filesystem access (sandboxed, connect-src 'none') —
            // the host already read the file, so it pushes the content in once the iframe's own
            // script has had a chance to call lowarc.on("lowarc:openFile", ...) and start listening.
            iframe.addEventListener("load", () => {
              iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:openFile", payload: { path, contents } }, "*");
              broadcastFileStatus();
              resolve();
            });
          });
          viewerIframeReady.set(iframe, ready);
          return ready.then(() => iframe);
        })();
      }

      // Tears down whatever a plugin instance was tracking for one path — disposing (e.g. Monaco's
      // model), not necessarily removing the iframe itself, since other open files may still be
      // relying on it. Only once nothing else references it does the instance actually go away.
      // Callers run this BEFORE removing their own openFiles entry for `path` (closeFile) or before
      // reassigning it to the new iframe (moveFileToGroup) — excluding `path` itself from the
      // "still used" check is what keeps that from always finding itself and never actually
      // tearing the instance down.
      function unmountFileFromGroup(path, iframe, groupId, pluginId) {
        if (!iframe) return;
        iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:closeFile", payload: { path } }, "*");
        const paths = windowToFilePaths.get(iframe.contentWindow);
        if (paths) paths.delete(path);
        const stillUsed = Array.from(openFiles.entries()).some(([p, f]) => p !== path && f.iframe === iframe);
        if (stillUsed) return;
        windowToFilePaths.delete(iframe.contentWindow);
        windowToPlugin.delete(iframe.contentWindow);
        groupViewerIframes.delete(viewerIframeKey(groupId, pluginId));
        viewerIframeReady.delete(iframe);
        iframe.remove();
      }

      function showActiveFile(groupId) {
        const { viewport, empty } = groupEls(groupId);
        viewport.querySelectorAll(".plugin-panel-frame").forEach((f) => f.classList.remove("is-active"));

        const path = groupActiveFilePath[groupId];
        const file = path ? openFiles.get(path) : null;
        empty.style.display = file ? "none" : "flex";
        if (file && file.iframe) {
          file.iframe.classList.add("is-active");
          file.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:activateFile", payload: { path } }, "*");
        }
        // Only actually matters for the group the user is currently focused on —
        // updateInspectorForActiveFile re-derives from activeGroupId itself, so calling it here
        // even when `groupId` is the OTHER (unfocused) group's own file switch is harmless, just
        // a no-op recompute of the same answer as before.
        if (groupId === activeGroupId) updateInspectorForActiveFile();
      }

      // Only for switching TO an already-rendered tab (a re-open, or right after openFile() has
      // just rendered a new one) — clicks the real DOM element so it goes through the exact same
      // primitive-delegated path a user's own click would, rather than a second way to activate.
      function activateFile(path) {
        const file = openFiles.get(path);
        if (!file) return;
        const tab = document.querySelector(`#tab-bar-tabs-${file.groupId} [data-tab-value="${CSS.escape(path)}"]`);
        if (tab) tab.click();
      }

      // Closing isn't a primitive-mediated user click on a tab — it's host logic deciding what
      // the next active tab should be, so this sets groupActiveFilePath directly rather than
      // simulating a click on whatever's chosen. Async because a dirty file gates on the popup
      // below — every caller (the close button, requestClose) already treats this as fire-and-
      // forget, so nothing needed to change at the call sites for that.
      async function closeFile(path) {
        const file = openFiles.get(path);
        if (!file) return;

        if (file.dirty) {
          const confirmed = await showPopup("confirm-close-dirty", { title: file.title });
          if (!confirmed) return;
          // The confirm popup is itself async — the file could have been closed some other way
          // (e.g. its own plugin calling requestClose after a successful save) while it was open.
          if (!openFiles.has(path)) return;
        }

        const groupId = file.groupId;
        unmountFileFromGroup(path, file.iframe, groupId, file.pluginId);
        openFiles.delete(path);
        removeContribution(`center-${groupId}`, path);

        if (groupActiveFilePath[groupId] === path) {
          const remaining = Array.from(openFiles.entries()).filter(([, f]) => f.groupId === groupId).map(([p]) => p);
          groupActiveFilePath[groupId] = remaining.length ? remaining[remaining.length - 1] : null;
        }

        renderTabBar(groupId);
        showActiveFile(groupId);
        broadcastFileStatus();
      }

      // Shared by the tab/tab-bar context menus' "Close Others"/"Close All" — one file at a time,
      // through the same closeFile() every other close path uses (dirty confirmation included),
      // rather than a bulk short-circuit that would skip it. exceptPath omitted closes everything
      // in the group; passed, it's the one tab that survives ("Close Others").
      async function closeGroupFiles(groupId, exceptPath) {
        const paths = Array.from(openFiles.entries())
          .filter(([p, f]) => f.groupId === groupId && p !== exceptPath)
          .map(([p]) => p);
        for (const p of paths) await closeFile(p);
      }

      async function closeSavedFiles(groupId) {
        const paths = Array.from(openFiles.entries())
          .filter(([, f]) => f.groupId === groupId && !f.dirty)
          .map(([p]) => p);
        for (const p of paths) await closeFile(p);
      }

      // Moves an ALREADY-open file into a different group. Used to reparent the same iframe node
      // outright when one instance = one file; now that an instance can hold several files at
      // once, moving just ONE of them can't simply drag the whole iframe along (that would move
      // every OTHER file sharing it too) — instead this asks the source instance for the file's
      // current content (possibly with unsaved edits re-reading from disk would silently lose),
      // unmounts it there, and mounts that same content into whatever instance already exists (or
      // gets created) in the target group. Unlike setSplitOpen's bulk merge (which deliberately
      // leaves whichever tab was already active in the destination group alone), this is always a
      // direct response to the user acting on this ONE file — dragging its tab across, or "Open in
      // Split View" — so it always becomes the active tab in its new group.
      async function moveFileToGroup(path, targetGroupId) {
        const file = openFiles.get(path);
        if (!file || file.groupId === targetGroupId) return;
        const fromGroupId = file.groupId;
        const viewer = viewersByExtension.get(fileExtension(path));
        if (!viewer) return; // the plugin that opened this got uninstalled out from under it

        // A binary viewer (images, video) is read-only — no unsaved-edit concept to preserve, so
        // re-reading from disk is exactly as correct as asking the plugin and means a binary
        // viewer never has to implement lowarc:getContent at all, unlike a real editable one.
        const contents = viewer.viewer.binary
          ? await invoke("read_binary_file", { path }).catch(() => "")
          : file.iframe
            ? await requestPluginContent(file.iframe, path)
            : "";

        removeContribution(`center-${fromGroupId}`, path);
        unmountFileFromGroup(path, file.iframe, fromGroupId, file.pluginId);

        try {
          file.iframe = await mountFileInGroup(path, contents ?? "", viewer, targetGroupId);
        } catch (err) {
          // The old iframe (and its center-${fromGroupId} contribution) is already gone above —
          // closing the file outright, rather than leaving it half-moved with a stale iframe
          // reference and no tab in either group, is the least surprising outcome of a failed
          // remount, same reasoning as openFile's own catch around this same call.
          showToast({ variant: "error", message: String(err) });
          openFiles.delete(path);
          renderTabBar(fromGroupId);
          return;
        }
        file.groupId = targetGroupId;
        contribute(`center-${targetGroupId}`, { id: path, sourceType: "plugin", pluginId: file.pluginId, label: file.title, closeable: true });

        if (groupActiveFilePath[fromGroupId] === path) {
          const remaining = Array.from(openFiles.entries()).filter(([, f]) => f.groupId === fromGroupId).map(([p]) => p);
          groupActiveFilePath[fromGroupId] = remaining.length ? remaining[remaining.length - 1] : null;
        }
        groupActiveFilePath[targetGroupId] = path;

        renderTabBar(fromGroupId);
        renderTabBar(targetGroupId);
        showActiveFile(fromGroupId);
        showActiveFile(targetGroupId);
        setActiveGroup(targetGroupId);
        broadcastFileStatus();
      }

      // opts.openInSplit: true forces this file into group 1, opening the split first if it isn't
      // already — the file-explorer's "Open in Split View" context-menu item (see window.lowarc.
      // openFile()'s new second argument in plugin_assets.rs) and, later, a cross-group tab drag
      // land here too, moving an already-open file rather than re-opening it.
      async function openFile(path, opts = {}) {
        const targetGroupId = opts.openInSplit ? 1 : activeGroupId;
        if (opts.openInSplit && !splitOpen) setSplitOpen(true);

        const existing = openFiles.get(path);
        if (existing) {
          if (existing.groupId !== targetGroupId && opts.openInSplit) {
            await moveFileToGroup(path, targetGroupId);
          } else {
            setActiveGroup(existing.groupId);
            activateFile(path);
          }
          return;
        }

        const ext = fileExtension(path);
        const viewer = viewersByExtension.get(ext);
        if (!viewer) {
          showToast({ variant: "error", message: `No installed plugin can open "${fileTitle(path)}"${ext ? ` (${ext})` : ""}.` });
          return;
        }

        let contents;
        try {
          // A viewer whose plugin.json marks itself "binary": true (images, video) gets its
          // content read+base64'd by read_binary_file instead — read_text_file would force a
          // UTF-8 decode on bytes that were never text, corrupting them.
          contents = await invoke(viewer.viewer.binary ? "read_binary_file" : "read_text_file", { path });
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }

        // Opens into whichever group was last interacted with, unless opts.openInSplit forced it —
        // there's no OTHER per-open group picker, matching the toolbar-level (not per-tab) split
        // button this pairs with.
        const groupId = targetGroupId;
        let iframe;
        try {
          iframe = await mountFileInGroup(path, contents, viewer, groupId);
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }

        openFiles.set(path, { title: fileTitle(path), dirty: false, missing: false, hasErrors: false, pluginId: viewer.pluginId, iframe, groupId });
        contribute(`center-${groupId}`, { id: path, sourceType: "plugin", pluginId: viewer.pluginId, label: fileTitle(path), closeable: true });
        renderTabBar(groupId);
        activateFile(path);
      }

      // window.lowarc.refreshFile()'s host half — some plugin (the Draft Tool's Revert, so far)
      // just changed `path`'s content on disk directly, bypassing whatever editor has it open. A
      // no-op if it isn't currently open anywhere (nothing to refresh). Unlike openFile(), this
      // pushes a DIFFERENT event — lowarc:openFile is a deliberate no-op for an already-open path
      // (see monaco.js's own comment on that), so an already-open file needs its own "yes, really,
      // replace what you're showing" signal; lowarc:refreshFile is that signal.
      async function refreshOpenFile(path) {
        const file = openFiles.get(path);
        if (!file) return;
        try {
          const contents = await invoke("read_text_file", { path });
          file.dirty = false;
          file.missing = false;
          file.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:refreshFile", payload: { path, contents } }, "*");
        } catch (err) {
          // Revert can also delete a path outright (one that didn't exist before the Draft
          // touched it) — same "missing" state a file deleted out from under an open tab any
          // other way already gets, not a hard error.
          file.missing = true;
        }
        renderTabBar(file.groupId);
        broadcastFileStatus();
        if (path === groupActiveFilePath[activeGroupId]) updateInspectorForActiveFile();
      }

      document.getElementById("open-file-item").addEventListener("click", async () => {
        const picked = await openDialog({ title: "Open File", defaultPath: projectPath });
        if (picked) openFile(picked);
      });

