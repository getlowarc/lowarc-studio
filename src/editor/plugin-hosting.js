      // ---------- Plugin UI hosting ----------
      // A plugin declares its panels statically (PluginDescriptor.contributes, read at scan time
      // by list_installed_plugins) — the shell builds rail icons/console tabs straight from that,
      // eagerly, whether or not the plugin has actually finished starting yet. Panel *content*
      // (the iframe) is mounted lazily, the first time it's actually shown — the plugin process is
      // already started at app launch (setup() in lib.rs) by the time anyone could click a rail
      // icon, so there's no real need to gate on the live registerPanel confirmation as well; that
      // event is still listened for below, just for visibility/logging rather than as a gate.
      //
      // pluginPanels: "pluginId::panelId" -> { pluginId, panel: {id,title,entry,...}, iframe }
      // windowToPlugin: an iframe's contentWindow -> pluginId, so the message-relay listener below
      // can trust *which* plugin a postMessage actually came from instead of the message's own
      // (self-reported, therefore untrustworthy) claim. Open-file viewer iframes (see the
      // tab-bar/open-files block further down) register in this same map — one trust mechanism,
      // shared by both kinds of plugin-hosted iframe.
      const pluginPanels = new Map();
      const windowToPlugin = new Map();

      function panelKey(pluginId, panelId) {
        return `${pluginId}::${panelId}`;
      }

      function mountPanelIframe(key, container) {
        const entry = pluginPanels.get(key);
        if (!entry) return null;
        if (!entry.iframe) {
          const iframe = document.createElement("iframe");
          iframe.className = "plugin-panel-frame";
          iframe.dataset.pluginId = entry.pluginId; // lets a command look up this plugin's own mounted iframe by id, see runCommand()
          iframe.setAttribute("sandbox", "allow-scripts");
          container.appendChild(iframe);
          entry.iframe = iframe;
          windowToPlugin.set(iframe.contentWindow, entry.pluginId);
          // Session-mode plugins (Terminal, so far — plugin.json's "session": true) start their
          // own sessions on demand (window.lowarc.session.start(), see plugin_session.rs) — one
          // per instance it wants running, not one the host auto-starts at mount time. Terminal
          // requests its first instance itself, the moment its own script runs, the exact same
          // way it requests every instance after that.
          // A panel has no filesystem access of its own, so if it needs to know the project root
          // (a file explorer does; most panels won't care) it's a URL fragment, not a postMessage
          // pushed after load (synchronous and readable the instant the plugin's own script
          // starts — no push-arrival-timing race to get wrong). Src is assigned once the
          // (cached, near-instant) plugin asset port is known — callers here get the iframe
          // element back synchronously to toggle classes on; they don't need the navigation
          // itself to have started yet.
          pluginAssetUrl(entry.pluginId, entry.panel.entry, `project=${encodeURIComponent(projectPath)}`)
            .then((url) => {
              iframe.src = url;
            })
            .catch(reportError);
          // So a freshly-mounted panel (the file explorer, most importantly) gets the current
          // dirty/error/missing snapshot right away instead of waiting for the next change.
          iframe.addEventListener("load", () => broadcastFileStatus());
          // A click landing inside this panel never bubbles up to this document, so any open
          // menu or dropdown would otherwise stay stuck open. The listener that actually covers
          // that is the harness reporting its own pointerdowns (see plugin_assets.rs and the
          // "pointerdown" branch in split-view.js), which fires whatever the focus state already
          // was. This one stays because it costs nothing and still fires first on the common path.
          iframe.addEventListener("focus", () => closeAllOverlays());
        }
        return entry.iframe;
      }

      // Generalizes what showSidebarPanel used to be into something any tab-strip region can use
      // (console joins in Phase 4) — shows contribution `id` from `slot` inside the element with id
      // `containerId`, mounting it (once, ever — cached below, regardless of source) the first time
      // it's actually shown. Host and plugin content are treated identically here: every
      // contribution gets the same reused per-tab wrapper (.host-panel-frame's existing show/hide
      // convention — flex column, 100%/100%, toggled via .is-active — not a new class just for
      // this), and mount(el) is free to build whatever it wants inside it, including delegating
      // straight to mountPanelIframe() the way a plugin adapter's mount() does; a plain iframe as
      // that wrapper's sole flex child fills it exactly the same as a host panel's own toolbar+list
      // markup does today.
      const slotTabMounted = new Map(); // "slot::id" -> the wrapper element already mounted for it

      function showSlotTab(slot, containerId, id) {
        const container = document.getElementById(containerId);
        container.classList.add("hosts-plugin");
        // Only the per-tab WRAPPER's own is-active is cleared here — never a mounted plugin
        // iframe's own .plugin-panel-frame.is-active nested inside one, which mount() sets exactly
        // once and never touches again (see the wrapper comment above). Clearing it here too would
        // un-set it permanently, since nothing would ever re-add it on a later show (mount() only
        // runs the first time) — the panel would come back empty on the second visit.
        container.querySelectorAll(".host-panel-frame").forEach((f) => f.classList.remove("is-active"));

        const contribution = getSlot(slot).find((c) => c.id === id);
        if (!contribution) return;

        const mountKey = `${slot}::${id}`;
        let el = slotTabMounted.get(mountKey);
        if (!el) {
          el = document.createElement("div");
          el.className = "host-panel-frame";
          container.appendChild(el);
          try {
            contribution.mount(el);
          } catch (err) {
            const errorEl = document.createElement("div");
            errorEl.style.color = "var(--danger)";
            errorEl.style.padding = "12px";
            errorEl.textContent = `This panel failed to load: ${err}`;
            el.appendChild(errorEl);
            showToast({ variant: "error", message: `"${id}" failed to render: ${err}`, source: contribution.pluginId || null });
          }
          slotTabMounted.set(mountKey, el);
        }
        el.classList.add("is-active");
        if (contribution.onShow) contribution.onShow();
      }

      // ---------- In-editor managers (host sidebar panels: __module-manager, __plugin-manager) ---
      // A quality-of-life alternative to the standalone Modules/Plugins pages (see file-modules-item/
      // file-plugins-item below) that never leaves the editor — deliberately separate surfaces, not
      // one replacing the other (Nolan: "those will be separate things"). Both wrap the exact same
      // Tauri commands the standalone pages already use — nothing new on the backend, just a
      // narrower-sidebar-shaped front end (single-column accordion instead of those pages' wide
      // two-pane list+detail layout, which needs more width than the sidebar's 240px default has).
      // One factory instead of two near-duplicate blocks, since a module manager and a plugin
      // manager are the same shape apart from which commands they call and which extra fields a
      // row shows — config.extraFields(item) is the only part that actually differs between them.
      function createManagerPanel(config) {
        const CHEVRON_SVG = '<svg viewBox="0 0 10 10" fill="none"><path d="M3 1l4 4-4 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';
        // Settings.tabOrder key — same persisted-order mechanism the rail/console/file tab strips
        // already use (see saveTabOrder/loadedSettings in panels.js, loaded earlier in this same
        // document), just applied to this list's own `items` array (see applySavedItemOrder)
        // instead of reordering already-rendered DOM, since render() rebuilds the list from
        // scratch on every load()/search keystroke rather than keeping persistent row elements.
        const orderKey = `plugin-manager-list-${config.nounPlural}`;

        let items = [];
        let expandedId = null;
        let listEl = null;
        let filterText = "";

        function applySavedItemOrder() {
          const order = loadedSettings?.tabOrder?.[orderKey];
          if (!order || !order.length) return;
          const byId = new Map(items.map((i) => [i.id, i]));
          const ordered = order.map((id) => byId.get(id)).filter(Boolean);
          const orderedIds = new Set(ordered.map((i) => i.id));
          items = [...ordered, ...items.filter((i) => !orderedIds.has(i.id))];
        }

        async function load() {
          try {
            items = await invoke(config.listCommand);
          } catch (err) {
            showToast({ variant: "error", message: String(err) });
            items = [];
          }
          applySavedItemOrder();
          if (!items.some((i) => i.id === expandedId)) expandedId = null;
          render();
        }

        // ---------- Enable/disable + remove — shared by the row's own controls and the row's
        // right-click context menu (showRowContextMenu below), so there's exactly one place each
        // actually happens rather than three copies of the same invoke/load/error-toast dance.
        async function setItemEnabled(item, enabled) {
          try {
            await invoke(config.enableCommand, { id: item.id, enabled });
            await load();
          } catch (err) {
            showToast({ variant: "error", message: String(err) });
          }
        }

        async function removeItem(item) {
          const confirmed = await showPopup("manager-remove", { nounSingular: config.nounSingular, name: item.name });
          if (!confirmed) return;
          try {
            await invoke(config.removeCommand, { id: item.id });
            expandedId = null;
            await load();
          } catch (err) {
            showToast({ variant: "error", message: String(err) });
          }
        }

        // Right-click menu — same openMenuOverlay primitive (menus.js) the rail/file-tab-bar
        // context menus already use, anchored at the raw pointer position rather than a trigger
        // element's rect (see showFileTabContextMenu in tabs-inspector.js for the closest existing
        // analog: a per-row menu with a toggle-style item plus a destructive one).
        function showRowContextMenu(item, x, y) {
          openMenuOverlay([{ label: item.disabled ? "Enable" : "Disable", value: "toggle" }, { label: "Remove", value: "remove" }], { x, y }).then((value) => {
            if (value === "toggle") setItemEnabled(item, item.disabled);
            else if (value === "remove") removeItem(item);
          });
        }

        function render() {
          listEl.innerHTML = "";
          if (items.length === 0) {
            const empty = document.createElement("div");
            empty.className = "plugin-manager-empty";
            empty.textContent = `No ${config.nounPlural} installed.`;
            listEl.appendChild(empty);
            return;
          }

          const query = filterText.trim().toLowerCase();
          const shown = query ? items.filter((i) => i.name.toLowerCase().includes(query) || i.id.toLowerCase().includes(query)) : items;
          if (shown.length === 0) {
            const empty = document.createElement("div");
            empty.className = "plugin-manager-empty";
            empty.textContent = `No ${config.nounPlural} match "${filterText.trim()}".`;
            listEl.appendChild(empty);
            return;
          }
          for (const item of shown) listEl.appendChild(renderRow(item));
          // Each row's own toggle carries a fresh data-tooltip element every render (typing in the
          // search box re-renders the list on every keystroke) — initTooltips() is idempotent per
          // element (dataset.tooltipInit guard), so re-running it here is cheap and keeps every
          // current row's tooltip actually wired up, not just the first render's.
          initTooltips();
        }

        // The row header's own enable/disable toggle — the app's one standard control for this
        // (.checkbox-row/.checkbox-box, same as createManagerPage's detail-pane toggle in
        // primitives.js) in place of what used to be a purely decorative, unclickable .status-dot.
        // Its own click is stopped from bubbling so it doesn't also trigger the row header's
        // expand/collapse.
        function renderEnabledToggle(item) {
          const label = document.createElement("label");
          label.className = "checkbox-row plugin-manager-row-toggle";
          label.dataset.tooltip = item.disabled ? "Disabled" : "Enabled";
          label.addEventListener("click", (e) => e.stopPropagation());

          const checkbox = document.createElement("input");
          checkbox.type = "checkbox";
          checkbox.checked = !item.disabled;
          const box = document.createElement("span");
          box.className = "checkbox-box";
          box.innerHTML = CHECKMARK_SVG;
          label.appendChild(checkbox);
          label.appendChild(box);

          checkbox.addEventListener("change", () => setItemEnabled(item, checkbox.checked));
          return label;
        }

        function renderRow(item) {
          const row = document.createElement("div");
          row.className = "plugin-manager-row";
          // initReorderable's default itemSelector/keyAttr ([data-tab-value] / .tabValue) — same
          // identity attribute every other reorderable strip in this app already uses, so this
          // list needs no custom itemSelector/keyAttr passed to initReorderable below.
          row.dataset.tabValue = item.id;

          const header = document.createElement("button");
          header.type = "button";
          header.className = "plugin-manager-row-header";

          header.appendChild(renderEnabledToggle(item));

          const name = document.createElement("span");
          name.className = "plugin-manager-row-name";
          name.textContent = item.name;
          header.appendChild(name);

          const chevron = document.createElement("span");
          chevron.className = "plugin-manager-row-chevron" + (expandedId === item.id ? " expanded" : "");
          chevron.innerHTML = CHEVRON_SVG;
          header.appendChild(chevron);

          header.addEventListener("click", () => {
            expandedId = expandedId === item.id ? null : item.id;
            render();
          });
          header.addEventListener("contextmenu", (e) => {
            e.preventDefault();
            e.stopPropagation();
            showRowContextMenu(item, e.clientX, e.clientY);
          });
          row.appendChild(header);

          if (expandedId === item.id) row.appendChild(renderRowBody(item));
          return row;
        }

        function renderRowBody(item) {
          const body = document.createElement("div");
          body.className = "plugin-manager-row-body";

          const sub = document.createElement("div");
          sub.className = "plugin-manager-row-sub";
          sub.textContent = item.id + (item.version ? ` · v${item.version}` : "");
          body.appendChild(sub);

          for (const field of config.extraFields(item)) {
            const fieldEl = document.createElement("div");
            fieldEl.className = "plugin-manager-row-desc";
            fieldEl.textContent = field.label ? `${field.label}: ${field.value}` : field.value;
            body.appendChild(fieldEl);
          }

          const actions = document.createElement("div");
          actions.className = "plugin-manager-row-actions";

          const removeBtn = document.createElement("button");
          removeBtn.type = "button";
          removeBtn.className = "btn btn-xs btn-danger btn-icon-only";
          removeBtn.dataset.tooltip = `Remove this ${config.nounSingular.toLowerCase()}`;
          removeBtn.innerHTML = DELETE_SVG;
          removeBtn.addEventListener("click", () => removeItem(item));
          actions.appendChild(removeBtn);

          body.appendChild(actions);
          return body;
        }

        function mount(el) {
          const toolbar = document.createElement("div");
          toolbar.className = "plugin-manager-toolbar";

          // One row: search (flex:1) + two icon-only buttons — a labeled "Add X…" button doesn't
          // fit the sidebar's 240px width alongside a second button (confirmed live: it pushed
          // Marketplace off the edge, clipping it), so both are icon-only with a tooltip carrying
          // the label instead, same as the row header's own icon-only controls.
          const controlsRow = document.createElement("div");
          controlsRow.className = "plugin-manager-toolbar-controls";

          const searchInput = document.createElement("input");
          searchInput.type = "text";
          searchInput.className = "input plugin-manager-search";
          // Just "Search" — the fuller "Search plugins…"/"Search modules…" got clipped in the
          // sidebar's 240px width now that the search box shares its row with two icon buttons.
          searchInput.placeholder = "Search";
          searchInput.addEventListener("input", () => {
            filterText = searchInput.value;
            render();
          });
          controlsRow.appendChild(searchInput);

          const addBtn = document.createElement("button");
          addBtn.type = "button";
          addBtn.className = "btn btn-sm btn-outline btn-icon-only";
          addBtn.dataset.tooltip = `Add ${config.nounSingular}…`;
          addBtn.innerHTML = '<svg viewBox="0 0 16 16" fill="none"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>';

          const marketplaceBtn = document.createElement("button");
          marketplaceBtn.type = "button";
          marketplaceBtn.className = "btn btn-sm btn-outline btn-icon-only";
          marketplaceBtn.dataset.tooltip = `Browse the ${config.nounSingular} marketplace`;
          marketplaceBtn.innerHTML =
            '<svg viewBox="0 0 16 16" fill="none"><path d="M2.5 5.5 3.5 2h9l1 3.5M2.5 5.5v7a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1v-7M2.5 5.5h11M6 5.5v1.5a2 2 0 0 0 4 0V5.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" /></svg>';
          marketplaceBtn.addEventListener("click", () => {
            showPopup(config.popupId, { url: `${config.pageFile}?tab=marketplace` });
          });

          const progressEl = document.createElement("div");
          progressEl.className = "progress";
          progressEl.style.display = "none";
          progressEl.innerHTML = '<div class="progress-fill"></div>';

          // This panel lives in editor.html's own top-level document (not an iframe, unlike the
          // standalone Modules/Plugins pages), so listening for the raw Tauri event directly here
          // is the proven pattern — no postMessage relay needed, same as dev-run-log/dev-run-ended.
          window.__TAURI__.event.listen("install-progress", (event) => {
            if (event.payload.kind !== config.nounSingular.toLowerCase()) return;
            setProgress(progressEl, event.payload.totalBytes ? event.payload.bytesDone / event.payload.totalBytes : 0);
          });

          addBtn.addEventListener("click", async () => {
            const sourceDir = await openDialog({ directory: true, title: `Choose a ${config.nounSingular.toLowerCase()} folder to install` });
            if (!sourceDir) return;

            addBtn.disabled = true;
            progressEl.style.display = "";
            setProgress(progressEl, 0);

            try {
              const id = await invoke(config.installCommand, { sourceDir });
              showToast({ variant: "success", message: `Installed "${id}".` });
              expandedId = id;
              await load();
            } catch (err) {
              showToast({ variant: "error", message: String(err) });
            } finally {
              addBtn.disabled = false;
              progressEl.style.display = "none";
            }
          });
          controlsRow.appendChild(addBtn);
          controlsRow.appendChild(marketplaceBtn);
          toolbar.appendChild(controlsRow);
          toolbar.appendChild(progressEl);
          el.appendChild(toolbar);

          listEl = document.createElement("div");
          listEl.className = "plugin-manager-list";
          listEl.setAttribute("data-reorderable", "");
          el.appendChild(listEl);

          // mount() runs exactly once per app session (see showSlotTab's own "mounting it once,
          // ever" comment) — listEl itself is never recreated after this, only its children
          // (render() rebuilds those from `items` on every load()/search keystroke), so a single
          // initReorderable call here covers every future render: it delegates from listEl itself
          // rather than binding per-row, the same way every other reorderable strip in this app
          // works (see initReorderable's own header comment in primitives.js).
          // Header-only drag handle: an expanded row's body is real content (description, version,
          // action buttons) that the user reads and clicks, so it shouldn't double as a grab area.
          // See initReorderable's handleSelector in primitives.js.
          initReorderable(listEl, {
            axis: "y",
            handleSelector: ".plugin-manager-row-header",
            onReorder: (order) => saveTabOrder(orderKey, order),
          });

          initTooltips();
        }

        return { mount, onShow: load };
      }

      // Keyed with no "::" so a reserved id can never collide with panelKey()'s "pluginId::panelId"
      // shape. order is negative so both managers sort ahead of any plugin-contributed icon (which
      // defaults to order 0) regardless of how many plugins are installed — matching the fixed
      // "Modules, then Plugins, then everything else" position this HTML used to hard-code.
      const pluginManagerPanel = createManagerPanel({
        listCommand: "list_installed_plugins",
        enableCommand: "set_plugin_enabled",
        removeCommand: "remove_plugin",
        installCommand: "install_plugin",
        nounSingular: "Plugin",
        nounPlural: "plugins",
        popupId: "plugins",
        pageFile: "plugins.html",
        extraFields: (item) => [{ value: item.description || "No description." }, { label: "Command", value: item.command }],
      });
      contribute("sidebar", {
        id: "__plugin-manager",
        sourceType: "host",
        label: "Plugins",
        order: -10,
        iconHtml:
          '<svg viewBox="0 0 16 16" fill="none"><path d="M5.5 2v3M10.5 2v3M4 5h8v2a4 4 0 0 1-4 4 4 4 0 0 1-4-4V5Z" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" /><path d="M8 11v3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" /></svg>',
        mount: pluginManagerPanel.mount,
        onShow: pluginManagerPanel.onShow,
      });

      const moduleManagerPanel = createManagerPanel({
        listCommand: "list_installed_modules",
        enableCommand: "set_module_enabled",
        removeCommand: "remove_module",
        installCommand: "install_module",
        nounSingular: "Module",
        nounPlural: "modules",
        popupId: "modules",
        pageFile: "modules.html",
        extraFields: (item) => [
          { value: item.description || "No description." },
          { label: "Load order", value: String(item.loadOrder) },
          { label: "Requires", value: item.requires.length ? item.requires.join(", ") : "Nothing." },
        ],
      });
      contribute("sidebar", {
        id: "__module-manager",
        sourceType: "host",
        label: "Modules",
        order: -20,
        iconHtml:
          '<svg viewBox="0 0 16 16" fill="none"><path d="M8 2l5.5 3.2v5.6L8 14l-5.5-3.2V5.2L8 2Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" /><path d="M8 8v6M8 8L2.5 4.8M8 8l5.5-3.2" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" /></svg>',
        mount: moduleManagerPanel.mount,
        onShow: moduleManagerPanel.onShow,
      });

      const FALLBACK_RAIL_ICON_SVG = '<svg viewBox="0 0 16 16" fill="none"><rect x="3" y="3" width="10" height="10" rx="2" stroke="currentColor" stroke-width="1.3" /></svg>';

      // Fetched as raw text over IPC (read_plugin_asset) rather than used as an <img src="..."> —
      // inlining it as a real <svg> lets it inherit currentColor for free, the same as the
      // fallback icon already does, which a rasterized <img> never could. Parsing + sanitizing
      // (strip anything that could execute if this ends up in the HOST's own document) is
      // primitives.js's shared parseSanitizedSvg — same treatment the Modules/Plugins manage
      // pages' own item icon uses, since both need identical "a plugin/module-supplied SVG can't
      // be trusted blindly" handling.
      async function loadRailIconSvg(pluginId, railIcon) {
        try {
          const text = await invoke("read_plugin_asset", { pluginId, relPath: railIcon });
          return parseSanitizedSvg(text);
        } catch (err) {
          return null;
        }
      }

      // "__run" is the one console tab the app itself owns (dev-run + plugin log output) — every
      // other key is a plugin-contributed tab, both registered via contribute("console", ...) and
      // shown through the same showSlotTab() every other tab-strip region uses. activeConsoleTabKey
      // is still tracked separately (rather than always re-querying the DOM for it) since
      // requestNewTerminal() below needs it on every click, not just on activation.
      let activeConsoleTabKey = "__run";

      // Generalizes what activateConsoleTab(btn, key) used to be — takes just the key now (not a
      // button reference) so it can be called both from a real click (via renderTabStrip's
      // onActivate, below) and programmatically (startRun(), which has no click event to hand it).
      // Manually re-toggling is-active here is redundant on the click path (primitives.js's
      // initTabs() already did it, since #console-tabs is a [data-tabs] container) but necessary on
      // the programmatic one — cheap enough either way not to bother with two separate functions.
      function activateConsoleTab(key) {
        document.querySelectorAll("#console-tabs .console-tab").forEach((t) => {
          t.classList.remove("is-active");
          t.setAttribute("aria-selected", "false");
        });
        const btn = document.querySelector(`#console-tabs .console-tab[data-tab-value="${key}"]`);
        if (btn) {
          btn.classList.add("is-active");
          btn.setAttribute("aria-selected", "true");
        }

        activeConsoleTabKey = key;
        showSlotTab("console", "console-body", key);

        // The new-terminal controls only make sense while a session-mode tab (Terminal) is
        // active — "__run" has no pluginPanels entry at all, so it falls through to hidden too.
        const entry = key !== "__run" ? pluginPanels.get(key) : null;
        document.getElementById("console-session-controls").style.display = entry && entry.session ? "flex" : "none";
        document.getElementById("console-clear-run").style.display = key === "__run" ? "flex" : "none";
      }

      // ---------- Terminal's "new instance" controls (host chrome, not the plugin's own UI —
      // see the console header comment) ----------
      // Both just ask whichever session-mode tab is currently active to open another instance —
      // the plugin itself (Terminal) owns creating the session and adding it to its own sidebar,
      // this is only ever "tell the active session-mode iframe a new-instance request happened".
      function requestNewTerminal(shell) {
        const entry = pluginPanels.get(activeConsoleTabKey);
        if (entry && entry.iframe) {
          entry.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:newTerminal", payload: { shell } }, "*");
        }
      }

      document.getElementById("console-new-session").addEventListener("click", () => requestNewTerminal(null));

      const CONSOLE_SESSION_SHELL_OPTIONS = [
        { value: "__default", label: "Default" },
        { value: "powershell", label: "PowerShell" },
        { value: "pwsh", label: "PowerShell 7 (pwsh)" },
        { value: "cmd", label: "Command Prompt" },
        { value: "bash", label: "Bash" },
        { value: "zsh", label: "Zsh" },
      ];
      document.getElementById("console-session-menu-trigger").addEventListener("click", (e) => {
        e.stopPropagation();
        openMenuFromTrigger(e.currentTarget, CONSOLE_SESSION_SHELL_OPTIONS).then((value) => {
          if (value) requestNewTerminal(value === "__default" ? null : value);
        });
      });

      // ---------- Maximize console (permanent, like Close — not scoped to any one tab) ----------
      let consoleMaximized = false;
      function setConsoleMaximized(maximized) {
        consoleMaximized = maximized;
        document.getElementById("console-expand").classList.toggle("is-active", maximized);
        if (maximized) {
          // Not calc(100vh - Npx) — CSS Grid doesn't shrink the *other* explicit/auto rows to
          // make room for one row that would overflow the container, it just lets the grid
          // overflow instead (confirmed live: the CSS var applied correctly but nothing visually
          // grew). Computing the real leftover pixels here, from the shell's own actual height
          // minus what the menu bar/status bar/divider genuinely take up, is what actually
          // collapses the center row to ~0 instead of merely overflowing past it.
          const shellHeight = shell.getBoundingClientRect().height;
          const menuHeight = document.querySelector(".menu-bar").getBoundingClientRect().height;
          const statusHeight = document.querySelector(".status-bar").getBoundingClientRect().height;
          const dividerHeight = 4; // the fixed 4px row between the center view and the console
          const available = shellHeight - menuHeight - statusHeight - dividerHeight;
          shell.style.setProperty("--console-height", `${Math.max(PANELS.console.min, available)}px`);
        } else {
          applyPanel("console"); // restores the normal, persisted (draggable) size
        }
      }
      document.getElementById("console-expand").addEventListener("click", () => setConsoleMaximized(!consoleMaximized));

      function appendConsoleLine(level, message) {
        const panel = document.getElementById("console-run-panel");
        const line = document.createElement("div");
        line.className = "line";
        line.textContent = message;
        if (level === "error") line.style.color = "var(--danger)";
        else if (level === "warn" || level === "warning") line.style.color = "var(--yellow)";
        panel.appendChild(line);
        panel.scrollTop = panel.scrollHeight;
      }

      // order:-10 so Run always sorts first, matching its old fixed "always first" position in the
      // static HTML. closeable:false — nothing currently renders a per-tab close control for ANY
      // console tab (Terminal manages closing its own sessions through its own UI, not a host-drawn
      // X), so this is forward-looking metadata, not something consumed yet. mount(el) just
      // re-parents the one #console-run-panel element that already exists and that
      // appendConsoleLine() already writes into regardless of migration — its own "is-active" class
      // (already set in its static HTML) is left untouched and permanent, exactly like a plugin
      // sidebar iframe's is-active is (see showSlotTab's wrapper comment); the WRAPPER's is-active
      // is what actually governs visibility once mounted.
      contribute("console", {
        id: "__run",
        sourceType: "host",
        label: "Run",
        order: -10,
        closeable: false,
        mount(el) {
          el.appendChild(document.getElementById("console-run-panel"));
        },
      });

