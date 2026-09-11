      // ---------- Transport controls (Run/Pause/Stop) ----------
      // Real now: Run calls start_dev_run, Stop calls stop_dev_run, Pause toggles pause_dev_run/
      // resume_dev_run, and dev-run-log/dev-run-ended/dev-run-frame (see lib.rs) feed the console's
      // Run tab and any plugin listening for lowarc:devRunFrame/lowarc:devRunEnded (the Debugger
      // plugin, so far; see broadcastToPlugins below). isRunning is authoritative from
      // dev-run-ended, not just set optimistically on click: a run can end on its own, not only
      // via Stop. isPaused IS set optimistically on a successful pause/resume call, since unlike
      // starting a run there's no meaningfully different "it didn't actually happen" outcome to
      // wait for: the call either fails outright (already caught) or the flag flips.
      let isRunning = false;
      let isPaused = false;

      function updateTransportButtons() {
        const runBtn = document.getElementById("run-btn");
        const pauseBtn = document.getElementById("pause-btn");
        const stopBtn = document.getElementById("stop-btn");

        runBtn.classList.toggle("btn-main", !isRunning);
        runBtn.classList.toggle("btn-ghost", isRunning);
        runBtn.disabled = isRunning;

        pauseBtn.classList.toggle("btn-warning", isRunning);
        pauseBtn.classList.toggle("btn-ghost", !isRunning);
        pauseBtn.disabled = !isRunning;

        stopBtn.classList.toggle("btn-danger", isRunning);
        stopBtn.classList.toggle("btn-ghost", !isRunning);
        stopBtn.disabled = !isRunning;

        // The Run menu's own items mirror the toolbar's disabled state exactly: "Run" is only
        // meaningful when nothing's running, "Pause"/"Stop"/"Restart" only once something is.
        document.getElementById("run-menu-run").disabled = isRunning;
        document.getElementById("run-menu-pause").disabled = !isRunning;
        document.getElementById("run-menu-stop").disabled = !isRunning;
        document.getElementById("run-menu-restart").disabled = !isRunning;
      }

      // The Run menu's "Edit Run Config…": the one place a project's entry file and required
      // modules are edited. Resolves true if it saved.
      function editRunConfig() {
        return showPopup("run-config", { projectPath });
      }

      // A project-relative entry path that is actually on disk, or null. "Never set" and "set but
      // since moved or deleted" are deliberately one case: from the user's side both mean the same
      // thing, and both are fixed the same way.
      async function usableEntry(preset) {
        const entry = preset.entry && preset.entry.trim() ? preset.entry.trim() : null;
        if (!entry) return null;
        return (await invoke("entry_file_exists", { projectDir: projectPath, entry })) ? entry : null;
      }

      // Run just runs the saved config, and deliberately never opens a file picker mid-click: a
      // Run button that sets configuration as a side effect is a surprise, and picking one file
      // would not make a project runnable anyway, since a project with no modules resolves to
      // nothing. An unrunnable config opens the editor instead,
      // and runs on the spot if that edit fixed it.
      async function startRun() {
        if (isRunning) return;

        let preset;
        try {
          preset = await invoke("get_project_preset", { projectDir: projectPath });
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }

        let entry = await usableEntry(preset);
        if (!entry) {
          showToast({
            variant: "error",
            message: preset.entry ? `Entry file "${preset.entry}" is missing — check the run config.` : "This project has no entry file set.",
          });
          if (!(await editRunConfig())) return; // cancelled: nothing changed, so nothing to retry
          try {
            preset = await invoke("get_project_preset", { projectDir: projectPath });
          } catch (err) {
            showToast({ variant: "error", message: String(err) });
            return;
          }
          entry = await usableEntry(preset);
          if (!entry) return; // saved, but still without a usable entry
        }

        try {
          await invoke("start_dev_run", { entryFile: entry, projectDir: projectPath });
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }

        isRunning = true;
        isPaused = false;
        updateTransportButtons();
        appendConsoleLine("info", `Run started (entry: ${entry}).`);
        activateConsoleTab("__run");
        setPanelOpen("console", true);
        broadcastRunState();
      }

      function stopRun() {
        invoke("stop_dev_run").catch((err) => showToast({ variant: "error", message: String(err) }));
      }

      // Stop, then start again once the backend has actually confirmed the run is over (the
      // dev-run-ended event, the same one the permanent listener below reacts to): not just
      // fire-and-forget stop_dev_run immediately followed by start_dev_run, since the backend can't
      // usefully begin a new run while it still considers the old one active.
      async function restartRun() {
        if (!isRunning) return;
        await window.__TAURI__.event.once("dev-run-ended", () => startRun());
        stopRun();
      }

      // The single place isPaused actually changes and gets announced: called after EITHER the
      // toolbar's own Pause button or a plugin's pauseRun()/resumeRun() successfully flips the
      // backend flag, so a plugin's own displayed state (the Debug plugin, so far) stays accurate
      // no matter which control was actually used. `log` is false for a plugin-triggered change
      // only to avoid double-logging when the plugin itself might want to say something more
      // specific than a generic line. Today nothing does, but the option costs nothing to keep.
      function setRunPaused(paused, { log = true } = {}) {
        isPaused = paused;
        if (log) appendConsoleLine("info", paused ? "Run paused." : "Run resumed.");
        broadcastRunState();
      }

      function togglePause() {
        if (!isRunning) return;
        invoke(isPaused ? "resume_dev_run" : "pause_dev_run")
          .then(() => setRunPaused(!isPaused))
          .catch((err) => showToast({ variant: "error", message: String(err) }));
      }

      // Pushed to every mounted plugin any time isRunning/isPaused actually changes (run start,
      // either Pause control, run end). See broadcastToPlugins below. Lets a plugin's own status
      // display (e.g. the Debug plugin's toolbar text) stay correct even when it had nothing to do
      // with causing the change itself, the same reasoning lowarc:fileStatus already follows for
      // dirty/error state.
      function broadcastRunState() {
        broadcastToPlugins("lowarc:devRunState", { running: isRunning, paused: isPaused });
      }

      document.getElementById("run-btn").addEventListener("click", startRun);
      document.getElementById("stop-btn").addEventListener("click", stopRun);
      document.getElementById("pause-btn").addEventListener("click", togglePause);

      // Broadcasts one event into every currently-mounted plugin iframe: same shape as
      // broadcastFileStatus(), just generalized into its own helper since dev-run-frame/
      // dev-run-ended both need the exact same "every mounted panel, not just one" relay.
      function broadcastToPlugins(event, payload) {
        const message = { type: "emit", event, payload };
        for (const entry of pluginPanels.values()) {
          if (entry.iframe) entry.iframe.contentWindow.postMessage(message, "*");
        }
      }

      window.__TAURI__.event.listen("dev-run-log", (event) => {
        appendConsoleLine(event.payload.level, event.payload.message);
      });

      // Never fired during a normal free-running loop (see spawn_and_run's on_frame gate in
      // process_module.rs) (only while paused, stepping, or right as a breakpoint fires), so this
      // is safe to relay unthrottled; it's already rare by construction, not something that needs
      // debouncing here too.
      window.__TAURI__.event.listen("dev-run-frame", (event) => {
        broadcastToPlugins("lowarc:devRunFrame", event.payload);
      });

      window.__TAURI__.event.listen("dev-run-ended", (event) => {
        isRunning = false;
        isPaused = false;
        updateTransportButtons();
        broadcastToPlugins("lowarc:devRunEnded", event.payload);
        broadcastRunState();
        if (event.payload.ok) {
          appendConsoleLine("info", "Run ended.");
        } else {
          (event.payload.errors || []).forEach((e) => appendConsoleLine("error", e));
          showToast({ variant: "error", message: "Run failed — see the Run tab." });
        }
      });

      // Plugin log lines share the same Run tab: one place to look for anything the app or
      // an installed plugin has to say, rather than needing per-plugin console real estate for it.
      window.__TAURI__.event.listen("plugin-log", (event) => {
        appendConsoleLine(event.payload.level, event.payload.message);
      });

      updateTransportButtons();

      // ---------- Unified panel state ----------
      // One place tracking each resizable panel's open/closed flag and its remembered size, so
      // the close (X) button, the header toggle icon, and dragging the divider past its minimum
      // all stay in sync instead of three separate ad-hoc mechanisms. Persisted globally in
      // settings.json's editorPanels (loaded below, saved on every toggle/close and on drag-end —
      // never on every drag tick, see initDividerDrag's onUp).

      const shell = document.getElementById("shell");

      // min/closeAt are shared across all three panels on purpose: different thresholds per
      // panel read as visually inconsistent (a sidebar and an inspector at different minimum
      // widths looks like a mistake, not a design choice).
      const PANELS = {
        sidebar: { open: false, size: 240, min: 160, closeAt: 50, max: 560, cssVar: "--sidebar-width", dataAttr: "sidebarOpen" },
        right: { open: false, size: 260, min: 160, closeAt: 50, max: 560, cssVar: "--right-width", dataAttr: "rightOpen" },
        console: { open: false, size: 220, min: 160, closeAt: 50, max: 640, cssVar: "--console-height", dataAttr: "consoleOpen" },
      };

      // Split-editor state lives outside PANELS on purpose. It isn't a grid-track toggle (no
      // cssVar/dataAttr, doesn't collapse to 0 on close), it's a flex ratio *inside* the "center"
      // grid area. See the Tab bar/open files section below for the group machinery this drives.
      let splitOpen = false;
      let splitRatio = 0.5;

      // Lives here, next to the two variables it reads, rather than in split-view.js with the rest
      // of the split machinery, because loadPanelLayout() below calls it after an await, and
      // split-view.js is the LAST script editor.html loads. That only ever worked because a real
      // Tauri invoke() takes longer than the four remaining script tags take to execute; anything
      // that made get_settings resolve promptly (a cache, a synchronous path) would have turned it
      // into a ReferenceError at startup. split-view.js loads later and can still call it.
      function applySplitRatio() {
        const group0 = document.getElementById("editor-group-0");
        group0.style.flex = splitOpen ? `0 0 ${splitRatio * 100}%` : "";
      }

      function applyPanel(key) {
        const p = PANELS[key];
        shell.style.setProperty(p.cssVar, `${p.open ? p.size : 0}px`);
        shell.dataset[p.dataAttr] = p.open ? "true" : "false";
      }

      // Read-modify-write: every save* below goes through this rather than spreading the cached
      // loadedSettings. set_plugin_enabled, install_* and remove_* all write Settings from a fresh
      // load() on the Rust side, and nothing pushes that back into this cache, so a later save
      // spreading a stale snapshot silently clobbers whatever they just wrote. Re-fetching here
      // costs one IPC round-trip per save, on user-initiated actions rather than a hot path, and
      // closes the whole class of bug instead of patching each caller that races.
      let loadedSettings = null;
      let restoredSidebarActiveKey = null;

      // buildPatch is either a plain object (merged as-is) or a function receiving the just-
      // fetched fresh settings and returning the patch to merge: the function form is for a
      // caller that needs to merge against a sub-object (tabOrder) rather than replace it wholesale,
      // so two different containers' saves in quick succession don't stomp on each other either.
      async function updateSettings(buildPatch) {
        const fresh = await invoke("get_settings");
        const patch = typeof buildPatch === "function" ? buildPatch(fresh) : buildPatch;
        const settings = { ...fresh, ...patch };
        await invoke("save_settings", { settings });
        loadedSettings = settings;
        return settings;
      }

      async function savePanelLayout() {
        try {
          await updateSettings({
            editorPanels: {
              sidebarOpen: PANELS.sidebar.open,
              sidebarSize: PANELS.sidebar.size,
              sidebarActiveKey: document.querySelector("#rail-tabs .sidebar-tab.is-active")?.dataset.tabValue || null,
              rightOpen: PANELS.right.open,
              rightSize: PANELS.right.size,
              consoleOpen: PANELS.console.open,
              consoleSize: PANELS.console.size,
              splitOpen,
              splitRatio,
            },
          });
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
        }
      }

      // ---------- Reorderable tab/rail strip order (Settings.tabOrder) ----------
      // containerId is the strip's own DOM id ("rail-tabs", "tab-bar-tabs-0", "console-tabs").
      async function saveTabOrder(containerId, order) {
        try {
          await updateSettings((fresh) => ({ tabOrder: { ...fresh.tabOrder, [containerId]: order } }));
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
        }
      }

      // Applies a saved order to whatever's already in the DOM: called once per renderTabStrip()
      // call (see there), whether that strip is built once and never re-rendered (the rail, the
      // console tabs) or rebuilt on every change (the file tab bars, whose contribution order
      // otherwise reverts to plain open-order (natural registry insertion order) on the very
      // next status change after a drag). anchorEl, if given, is a trailing non-item child items
      // must stay before (the console's tabs-spacer, ahead of its own "+"/"..." controls):
      // container.insertBefore(el, null) already behaves like appendChild, so omitting it for the
      // rail/file-tab-bar case (nothing trails there) needs no special case.
      function applySavedOrder(container, itemSelector, keyAttr, anchorEl) {
        const order = loadedSettings?.tabOrder?.[container.id];
        if (!order || !order.length) return;
        const byKey = new Map(Array.from(container.querySelectorAll(itemSelector)).map((el) => [el.dataset[keyAttr], el]));
        for (const key of order) {
          const el = byKey.get(key);
          if (el) container.insertBefore(el, anchorEl || null);
        }
      }

      function setPanelOpen(key, open) {
        PANELS[key].open = open;
        applyPanel(key);
        syncToggleUI();
        savePanelLayout();
      }

      function togglePanel(key) {
        setPanelOpen(key, !PANELS[key].open);
      }

      function syncToggleUI() {
        document.getElementById("toggle-console").classList.toggle("is-active", PANELS.console.open);
        document.getElementById("toggle-right").classList.toggle("is-active", PANELS.right.open);
      }

      document.getElementById("console-close").addEventListener("click", () => setPanelOpen("console", false));
      document.getElementById("console-clear-run").addEventListener("click", () => {
        document.getElementById("console-run-panel").querySelectorAll(".line").forEach((el) => el.remove());
      });
      document.getElementById("right-panel-close").addEventListener("click", () => setPanelOpen("right", false));
      document.getElementById("toggle-console").addEventListener("click", () => togglePanel("console"));
      document.getElementById("toggle-right").addEventListener("click", () => togglePanel("right"));

      async function loadPanelLayout() {
        try {
          loadedSettings = await invoke("get_settings");
          const ep = loadedSettings.editorPanels;
          if (ep) {
            PANELS.sidebar.open = ep.sidebarOpen;
            PANELS.sidebar.size = ep.sidebarSize;
            restoredSidebarActiveKey = ep.sidebarActiveKey || null;
            PANELS.right.open = ep.rightOpen;
            PANELS.right.size = ep.rightSize;
            PANELS.console.open = ep.consoleOpen;
            PANELS.console.size = ep.consoleSize;
            splitOpen = Boolean(ep.splitOpen);
            splitRatio = typeof ep.splitRatio === "number" ? ep.splitRatio : 0.5;
          }
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
        }
        applyPanel("sidebar");
        applyPanel("right");
        applyPanel("console");
        syncToggleUI();
        // Split has no mounted files to restore (open files never persist across restart, split
        // or not; see the Tab bar/open files section), just the layout shell itself: whether the
        // second group is visible, and at what width.
        document.getElementById("center-panel").classList.toggle("is-split", splitOpen);
        document.getElementById("toggle-split").classList.toggle("is-active", splitOpen);
        applySplitRatio();
      }

      loadPanelLayout();

