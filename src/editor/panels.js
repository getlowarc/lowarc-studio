      // ---------- Transport controls (Run/Pause/Stop) ----------
      // Real now: Run calls start_dev_run, Stop calls stop_dev_run, Pause toggles pause_dev_run/
      // resume_dev_run, and dev-run-log/dev-run-ended/dev-run-frame (see lib.rs) feed the console's
      // Run tab and any plugin listening for lowarc:devRunFrame/lowarc:devRunEnded (the Debugger
      // plugin, so far — see broadcastToPlugins below). isRunning is authoritative from
      // dev-run-ended, not just set optimistically on click — a run can end on its own, not only
      // via Stop. isPaused IS set optimistically on a successful pause/resume call, since unlike
      // starting a run there's no meaningfully different "it didn't actually happen" outcome to
      // wait for — the call either fails outright (already caught) or the flag flips.
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

        // The Run menu's own items mirror the toolbar's disabled state exactly — "Run" is only
        // meaningful when nothing's running, "Pause"/"Stop"/"Restart" only once something is.
        document.getElementById("run-menu-run").disabled = isRunning;
        document.getElementById("run-menu-pause").disabled = !isRunning;
        document.getElementById("run-menu-stop").disabled = !isRunning;
        document.getElementById("run-menu-restart").disabled = !isRunning;
      }

      // Opens the native file picker for the project's entry file and persists whatever gets
      // picked. Shared by the menu item (change it any time) and ensureEntrySet (prompt only when
      // Run actually needs one) — both just need "ask, save, tell the user", nothing else differs.
      async function pickAndSetEntry() {
        const picked = await openDialog({ title: "Choose the project's entry file", defaultPath: projectPath });
        if (!picked) return null;
        try {
          const entry = await invoke("set_project_entry", { projectDir: projectPath, absoluteEntryPath: picked });
          showToast({ variant: "success", message: `Entry file set to ${entry}.` });
          return entry;
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return null;
        }
      }

      // Returns a project-relative entry path that's actually there on disk, prompting the picker
      // if none is set yet or if the one that was set has since moved/been deleted — same dialog
      // either way, since from the user's side "never set" and "no longer valid" look identical.
      async function ensureEntrySet(preset) {
        let entry = preset.entry && preset.entry.trim() ? preset.entry.trim() : null;
        if (entry && !(await invoke("entry_file_exists", { projectDir: projectPath, entry }))) {
          entry = null;
        }
        return entry || pickAndSetEntry();
      }

      async function startRun() {
        if (isRunning) return;

        let preset;
        try {
          preset = await invoke("get_project_preset", { projectDir: projectPath });
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }

        const entry = await ensureEntrySet(preset);
        if (!entry) return; // picker was cancelled, or setting it failed (already toasted)

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
      // dev-run-ended event, the same one the permanent listener below reacts to) — not just
      // fire-and-forget stop_dev_run immediately followed by start_dev_run, since the backend can't
      // usefully begin a new run while it still considers the old one active.
      async function restartRun() {
        if (!isRunning) return;
        await window.__TAURI__.event.once("dev-run-ended", () => startRun());
        stopRun();
      }

      // The single place isPaused actually changes and gets announced — called after EITHER the
      // toolbar's own Pause button or a plugin's pauseRun()/resumeRun() successfully flips the
      // backend flag, so a plugin's own displayed state (the Debug plugin, so far) stays accurate
      // no matter which control was actually used. `log` is false for a plugin-triggered change
      // only to avoid double-logging when the plugin itself might want to say something more
      // specific than a generic line — today nothing does, but the option costs nothing to keep.
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
      // either Pause control, run end) — see broadcastToPlugins below. Lets a plugin's own status
      // display (e.g. the Debug plugin's toolbar text) stay correct even when it had nothing to do
      // with causing the change itself, the same reasoning lowarc:fileStatus already follows for
      // dirty/error state.
      function broadcastRunState() {
        broadcastToPlugins("lowarc:devRunState", { running: isRunning, paused: isPaused });
      }

      document.getElementById("run-btn").addEventListener("click", startRun);
      document.getElementById("stop-btn").addEventListener("click", stopRun);
      document.getElementById("pause-btn").addEventListener("click", togglePause);

      // Broadcasts one event into every currently-mounted plugin iframe — same shape as
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
      // process_module.rs) — only while paused, stepping, or right as a breakpoint fires — so this
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

      // Plugin log lines share the same Run tab — one place to look for anything the app or
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

      // min/closeAt are shared across all three panels on purpose — different thresholds per
      // panel read as visually inconsistent (a sidebar and an inspector at different minimum
      // widths looks like a mistake, not a design choice).
      const PANELS = {
        sidebar: { open: false, size: 240, min: 160, closeAt: 50, max: 560, cssVar: "--sidebar-width", dataAttr: "sidebarOpen" },
        right: { open: false, size: 260, min: 160, closeAt: 50, max: 560, cssVar: "--right-width", dataAttr: "rightOpen" },
        console: { open: false, size: 220, min: 160, closeAt: 50, max: 640, cssVar: "--console-height", dataAttr: "consoleOpen" },
      };

      // Split-editor state lives outside PANELS on purpose — it isn't a grid-track toggle (no
      // cssVar/dataAttr, doesn't collapse to 0 on close), it's a flex ratio *inside* the "center"
      // grid area. See the Tab bar/open files section below for the group machinery this drives.
      let splitOpen = false;
      let splitRatio = 0.5;

      function applyPanel(key) {
        const p = PANELS[key];
        shell.style.setProperty(p.cssVar, `${p.open ? p.size : 0}px`);
        shell.dataset[p.dataAttr] = p.open ? "true" : "false";
      }

      // Read-modify-write helper — every save* function below goes through this rather than
      // spreading the cached loadedSettings directly. Real bug hit and fixed live: set_plugin_
      // enabled/set_module_enabled/install_*/remove_* all write Settings straight from a *fresh*
      // settings::load() on the Rust side, without loadedSettings (a plain JS variable, never
      // pushed to from those other commands) ever finding out — so a later save*() spreading the
      // stale cache would silently clobber whatever one of those had just written. Concretely:
      // toggling Terminal's Enabled checkbox back on in the plugin manager, then later dragging a
      // rail icon (triggering saveTabOrder), re-disabled Terminal — the drag's own save spread a
      // loadedSettings snapshot captured before the enable toggle. Always re-fetching here instead
      // of trusting the cache costs one extra IPC round-trip per save (infrequent, user-initiated
      // actions, not a hot path) and closes the whole class of bug at once rather than patching
      // each caller that happens to race with one of those other commands.
      let loadedSettings = null;
      let restoredSidebarActiveKey = null;

      // buildPatch is either a plain object (merged as-is) or a function receiving the just-
      // fetched fresh settings and returning the patch to merge — the function form is for a
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

      // Applies a saved order to whatever's already in the DOM — called once per renderTabStrip()
      // call (see there), whether that strip is built once and never re-rendered (the rail, the
      // console tabs) or rebuilt on every change (the file tab bars, whose contribution order
      // otherwise reverts to plain open-order — natural registry insertion order — on the very
      // next status change after a drag). anchorEl, if given, is a trailing non-item child items
      // must stay before (the console's tabs-spacer, ahead of its own "+"/"..." controls) —
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
        // or not — see the Tab bar/open files section), just the layout shell itself: whether the
        // second group is visible, and at what width.
        document.getElementById("center-panel").classList.toggle("is-split", splitOpen);
        document.getElementById("toggle-split").classList.toggle("is-active", splitOpen);
        applySplitRatio();
      }

      loadPanelLayout();

