      // ---------- File menu ----------
      document.getElementById("back-to-start").addEventListener("click", () => {
        window.location.href = "index.html";
      });

      document.getElementById("file-modules-item").addEventListener("click", () => {
        showPopup("modules", { url: "modules.html" });
      });

      document.getElementById("file-plugins-item").addEventListener("click", () => {
        showPopup("plugins", { url: "plugins.html" });
      });

      document.getElementById("file-export-item").addEventListener("click", () => {
        showPopup("export", { url: "export.html?project=" + encodeURIComponent(projectPath) });
      });

      document.getElementById("exit-item").addEventListener("click", () => {
        window.__TAURI__.window.getCurrentWindow().close();
      });

      document.getElementById("open-project-item").addEventListener("click", async () => {
        const path = await openDialog({ directory: true, title: "Open Folder" });
        if (!path) return;
        try {
          await invoke("open_project", { path });
          openEditor(path);
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
        }
      });

      document.getElementById("new-project-item").addEventListener("click", async () => {
        const parentDir = await openDialog({ directory: true, title: "Choose a folder for the new project" });
        if (!parentDir) return;
        const path = await showPopup("new-project", { parentDir });
        if (path) openEditor(path);
      });

      // ---------- File menu: Save / Save As ----------
      // Both route through the active file's own viewer iframe rather than the host reading/
      // writing disk directly: the iframe holds whatever's actually in its buffer (unsaved edits
      // included), the same reasoning moveFileToGroup() already relies on via
      // requestPluginContent(). A viewer with nothing to save (no open file, or a read-only viewer
      // like Media Viewer that never implements lowarc:getContent/requestSave) just does nothing.
      // Save is a silent no-op, Save As surfaces the same "couldn't read content" toast
      // requestPluginContent's null timeout already produces for other callers.
      function activeFileEntry() {
        return openFiles.get(groupActiveFilePath[activeGroupId]) || null;
      }

      document.getElementById("file-save-item").addEventListener("click", () => {
        const file = activeFileEntry();
        if (!file || !file.iframe) return;
        file.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:requestSave" }, "*");
      });

      document.getElementById("file-save-as-item").addEventListener("click", async () => {
        const path = groupActiveFilePath[activeGroupId];
        const file = path ? openFiles.get(path) : null;
        if (!file || !file.iframe) return;
        const content = await requestPluginContent(file.iframe, path);
        if (content === null) {
          showToast({ variant: "error", message: "Couldn't read this file's current content to save it." });
          return;
        }
        const target = await saveDialog({ defaultPath: path });
        if (!target) return;
        try {
          await invoke("write_text_file", { path: target, contents: content });
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }
        openFile(target);
      });

      // ---------- Edit menu ----------
      // Routed to whichever viewer iframe is showing the active file via a small generic "run this
      // editor command" event (lowarc:editorCommand), not the existing lowarc:runCommand — Undo/
      // Redo aren't registered Monaco Actions (only genuine Actions show up via editor.getAction(),
      // see refreshCommands() in monaco.js), so they need editor.trigger() instead, which reaches a
      // command OR an action either way. A viewer that doesn't understand a given command id just
      // ignores the event: nothing to route to for a read-only viewer, same graceful no-op as any
      // other unhandled event in this contract.
      function sendEditorCommand(command) {
        const file = activeFileEntry();
        if (!file || !file.iframe) return;
        file.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:editorCommand", payload: { command } }, "*");
      }

      document.getElementById("edit-undo-item").addEventListener("click", () => sendEditorCommand("undo"));
      document.getElementById("edit-redo-item").addEventListener("click", () => sendEditorCommand("redo"));
      document.getElementById("edit-cut-item").addEventListener("click", () => sendEditorCommand("editor.action.clipboardCutAction"));
      document.getElementById("edit-copy-item").addEventListener("click", () => sendEditorCommand("editor.action.clipboardCopyAction"));
      document.getElementById("edit-find-item").addEventListener("click", () => sendEditorCommand("actions.find"));
      document.getElementById("edit-replace-item").addEventListener("click", () => sendEditorCommand("editor.action.startFindReplaceAction"));

      // Paste does not go through sendEditorCommand like the other six. That action would read via
      // the sandboxed iframe's own navigator.clipboard, and Chromium denies clipboard READ to an
      // opaque origin; writeText, which Cut and Copy use, is not restricted the same way. The host
      // document has no such restriction, so it reads the OS clipboard through Tauri and hands the
      // text to the active viewer: the same host-holds-the-privilege split applyLineEdit uses.
      document.getElementById("edit-paste-item").addEventListener("click", async () => {
        const file = activeFileEntry();
        if (!file || !file.iframe) return;
        let text;
        try {
          text = await window.__TAURI__.clipboardManager.readText();
        } catch (err) {
          showToast({ variant: "error", message: "Couldn't read the clipboard." });
          return;
        }
        if (typeof text !== "string" || !text) return;
        file.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:insertText", payload: { text } }, "*");
      });

      // ---------- View menu ----------
      document.getElementById("view-toggle-sidebar").addEventListener("click", () => togglePanel("sidebar"));
      document.getElementById("view-toggle-console").addEventListener("click", () => togglePanel("console"));
      document.getElementById("view-toggle-right").addEventListener("click", () => togglePanel("right"));

      document.getElementById("view-appearance-item").addEventListener("click", () => {
        showPopup("settings", { url: "settings.html?tab=appearance" });
      });

      document.getElementById("view-reload-item").addEventListener("click", () => {
        window.location.reload();
      });

      document.getElementById("view-fullscreen-item").addEventListener("click", async () => {
        const appWindow = window.__TAURI__.window.getCurrentWindow();
        const isFullscreen = await appWindow.isFullscreen();
        await appWindow.setFullscreen(!isFullscreen);
      });

      // Focusing the input is enough to open it. See initSearchbar's own focus listener
      // (primitives.js), which renders the browsable categories the instant it's non-empty.
      function openCommandPalette() {
        commandCenterInput.focus();
      }

      document.getElementById("view-command-palette-item").addEventListener("click", openCommandPalette);

      // ---------- Run menu ----------
      // Wrapped in arrow functions (not bare references) on purpose: startRun/togglePause/stopRun/
      // restartRun/pickAndSetEntry are all defined in panels.js, which loads AFTER this file: a
      // bare reference here would need to resolve immediately, at registration time, and throw.
      // Wrapping defers the lookup to click time, by which every script has already loaded (the
      // same reason this line never had to think about it back when this was all one script:
      // function declarations are hoisted across an entire single script regardless of textual
      // order, but hoisting does NOT cross separate <script> tags — splitting the file is what
      // turned this from "always fine" into "fine only if deferred").
      document.getElementById("run-menu-run").addEventListener("click", () => startRun());
      document.getElementById("run-menu-pause").addEventListener("click", () => togglePause());
      document.getElementById("run-menu-stop").addEventListener("click", () => stopRun());
      document.getElementById("run-menu-restart").addEventListener("click", () => restartRun());
      document.getElementById("run-menu-config").addEventListener("click", () => editRunConfig());

      document.getElementById("dev-run-settings-item").addEventListener("click", () => {
        showPopup("settings", { url: "settings.html" });
      });

      // ---------- Help menu ----------
      document.getElementById("help-docs-item").addEventListener("click", () => {
        openUrl("https://lowarc.com");
      });

      document.getElementById("help-shortcuts-item").addEventListener("click", () => {
        showPopup("shortcuts");
      });

      document.getElementById("help-report-issue-item").addEventListener("click", () => {
        openUrl("https://github.com/NolanLT/lowarc-studio/issues");
      });

      // version::label() rather than __TAURI__.app.getVersion(), which returns the number without
      // the major's name. Both ultimately read Cargo.toml, so there is nothing to keep in sync.
      async function getAppVersionLabel() {
        try {
          return `Version ${await invoke("app_version")}`;
        } catch (err) {
          return "";
        }
      }

      document.getElementById("help-about-item").addEventListener("click", async () => {
        showPopup("about", { version: await getAppVersionLabel() });
      });

      // The logo has no click behavior of its own: this is purely what initTooltips() (below)
      // reads on hover. Starts as just the name (data-tooltip is already set in the markup) and
      // grows a version line once getAppVersionLabel() actually resolves; tooltip content is read
      // live at hover time, not cached at init, so updating the attribute after initTooltips() has
      // already run is enough: no re-init needed.
      getAppVersionLabel().then((version) => {
        const logo = document.getElementById("app-logo-icon");
        if (version) logo.dataset.tooltip = `LowArc Studio — ${version}`;
      });

      // ---------- Global keyboard shortcuts ----------
      // Deliberately small. Ctrl+S, Ctrl+Shift+S and Ctrl+Shift+P are host-level so they fire while
      // some other panel has focus, not only inside the editor. Not extended to Undo, Cut, Find and
      // friends, which Monaco already binds inside its own iframe.
      //
      // Like the Escape handler above, these only fire while the HOST document has focus: a
      // sandboxed iframe's keydown never reaches here, so they do nothing while you are typing in
      // one. See the "shortcuts" popup for the fuller caveat.
      document.addEventListener("keydown", (e) => {
        const mod = e.ctrlKey || e.metaKey;
        if (!mod || e.altKey) return;
        const key = e.key.toLowerCase();
        if (key === "s") {
          e.preventDefault();
          document.getElementById(e.shiftKey ? "file-save-as-item" : "file-save-item").click();
        } else if (key === "p" && e.shiftKey) {
          e.preventDefault();
          openCommandPalette();
        }
      });

      initTooltips();
      initWindowControls(() => confirmAppClose());
      initTabs();

      // Drag-to-reorder. See initReorderable() in primitives.js. Only these four strips opt in
      // (data-reorderable, set on each one's own markup); everything else (the top menu bar, a
      // standalone page's view tabs) is untouched. Safe to call before any of their items exist:
      // the listeners are delegated on the container itself, not attached per-item.
      initReorderable(document.getElementById("rail-tabs"), { axis: "y", onReorder: (order) => saveTabOrder("rail-tabs", order) });
      // Each group names the OTHER as its crossContainer, so a file tab can be dragged freely
      // between them. See initReorderable's crossContainer/onMoveAcross in primitives.js.
      // moveFileToGroup isn't defined yet at this point in the script, but neither closure below
      // calls it until an actual cross-drag completes, by which point it is (function declarations
      // are hoisted, and this only ever fires from later user interaction, never synchronously here).
      initReorderable(document.getElementById("tab-bar-tabs-0"), {
        axis: "x",
        crossContainer: document.getElementById("tab-bar-tabs-1"),
        crossZone: document.getElementById("editor-group-1"),
        onReorder: (order) => saveTabOrder("tab-bar-tabs-0", order),
        onMoveAcross: (path) => moveFileToGroup(path, 1),
      });
      initReorderable(document.getElementById("tab-bar-tabs-1"), {
        axis: "x",
        crossContainer: document.getElementById("tab-bar-tabs-0"),
        crossZone: document.getElementById("editor-group-0"),
        onReorder: (order) => saveTabOrder("tab-bar-tabs-1", order),
        onMoveAcross: (path) => moveFileToGroup(path, 0),
      });
      initReorderable(document.getElementById("console-tabs"), { axis: "x", onReorder: (order) => saveTabOrder("console-tabs", order) });

