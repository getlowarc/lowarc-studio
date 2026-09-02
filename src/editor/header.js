      // ---------- Header search ----------
      // A real searchbar now, not a button that opens one — types in place, shows matches in the
      // same dropdown popover initSearchbar() already drives everywhere else. Sourced from
      // buildSearchableSettingsList() (primitives.js) plus this file's own command registry below —
      // this bar isn't a "settings search," it's the app's one general search entry point; more
      // sources (open files, whatever comes later) fold into the same options list, not a second
      // search UI.
      //
      // HEADER_SEARCH_CATEGORIES is the fixed, ordered set of result groups this bar knows about —
      // a category with nothing matching a given query just doesn't render its divider (see
      // initSearchbar's `categories` option), so "results" having no source yet today is invisible,
      // not a placeholder gap. "results" (in-open-file text search) is deliberately not built here
      // — that becomes the Monaco plugin's own contribution once it exists, not core's business.
      // "commands" (Command Palette — the term predates VS Code, from Sublime Text, and is the
      // industry-standard name for "searchable list of actions" across editors generally, so it's
      // kept rather than invented fresh) is populated below.
      const HEADER_SEARCH_CATEGORIES = [
        // browsable: false — in-file text search has nothing meaningful to preview with no query
        // typed yet, unlike Settings/Command Palette, which are real fixed lists worth browsing.
        { id: "results", label: "Results", browsable: false },
        { id: "commands", label: "Command Palette" },
        { id: "settings", label: "Settings" },
      ];

      // Command Palette registry, half of it anyway — the host-owned half. Every ENABLED item in
      // the File/Edit/View/Run/Help menu bar is already a real DOM element with a label and a
      // click handler; scanning it rather than hand-maintaining a second list is what keeps this
      // from drifting out of sync with the actual menu the moment someone adds an item and forgets
      // the palette entry. Disabled placeholders (Save, Undo, Command Palette's own menu label
      // wouldn't be one of these anyway) are skipped — nothing to run yet.
      function buildHostMenuCommands() {
        const commands = [
          // Settings has no menu-bar item of its own to scan below — it only lives in the
          // gear icon's own dynamically-built floating menu (openMenuFromTrigger, not a real
          // .menu-dropdown-item at any point), so it's added by hand here instead.
          { id: "command:host:settings", label: "Settings", category: "commands", kind: "command", run: () => showPopup("settings", { url: "settings.html" }) },
        ];
        document.querySelectorAll(".menu-dropdown-item").forEach((btn) => {
          if (btn.disabled) return;
          const label = btn.textContent.trim();
          if (!label) return;
          commands.push({ id: `command:host:${btn.id}`, label, category: "commands", kind: "command", run: () => btn.click() });
        });
        return commands;
      }

      // The other half — plugin-declared. A sandboxed plugin's content can't be introspected the
      // way the host's own menu bar can, so it has to say what it offers itself, via plugin.json's
      // `commands` array (same shape/spirit as `settings` — see PluginCommand in protocol.rs).
      async function buildPluginCommands() {
        let plugins = [];
        try {
          plugins = await invoke("list_installed_plugins");
        } catch (err) {
          return [];
        }
        const commands = [];
        for (const plugin of plugins) {
          for (const cmd of plugin.commands || []) {
            commands.push({
              id: `command:plugin:${plugin.id}:${cmd.id}`,
              label: cmd.label,
              hint: cmd.hint,
              category: "commands",
              kind: "command",
              run: () => runPluginCommand(plugin.id, cmd.id),
            });
          }
        }
        return commands;
      }

      // Opens whichever slot `pluginId` actually contributes (sidebar panel, inspector, or console
      // tab), mounting its iframe fresh if it wasn't already, and resolves once that iframe is
      // genuinely ready to receive a message — not just once the element exists. A freshly-created
      // iframe's own scripts (including whatever registers window.lowarc.on("lowarc:runCommand", ...))
      // haven't run yet the instant mount() returns; waiting for the iframe's `load` event is what
      // actually guarantees the plugin's own listener exists before anything gets sent to it. An
      // already-mounted iframe's `load` already fired in the past, so it's returned immediately —
      // attaching a new `load` listener to it here would just wait forever. Resolves null for a
      // plugin with no panel/console-tab contribution at all (e.g. viewer-only) — nothing to open.
      function ensurePluginMounted(pluginId) {
        for (const [key, entry] of pluginPanels) {
          if (entry.pluginId !== pluginId) continue;
          const wasMounted = Boolean(entry.iframe);
          const location = entry.panel.location; // "sidebar" | "inspector" | undefined (a console tab)

          if (location === "sidebar") {
            showSlotTab("sidebar", "left-panel-body", key);
            setPanelOpen("sidebar", true);
          } else if (location === "inspector") {
            showInspector(key);
          } else {
            setPanelOpen("console", true);
            activateConsoleTab(key);
          }

          if (!entry.iframe) return Promise.resolve(null); // contribution registered but mount() itself failed
          if (wasMounted) return Promise.resolve(entry.iframe);
          return new Promise((resolve) => entry.iframe.addEventListener("load", () => resolve(entry.iframe), { once: true }));
        }
        return Promise.resolve(null);
      }

      // A viewer plugin (Monaco, so far) can have several iframes mounted at once — one per open
      // file, possibly across both split groups — so "the" mounted instance a panel/console-tab
      // plugin has doesn't apply; its commands mean "run this in the file I'm currently looking
      // at." The active file's own iframe is tried first (only really matches a viewer); anything
      // else (Debug, Terminal, file explorer) has no open-file iframe of its own, so this falls
      // through to the existing panel/console-tab path unchanged.
      async function runPluginCommand(pluginId, commandId) {
        const activeFile = openFiles.get(groupActiveFilePath[activeGroupId]);
        if (activeFile && activeFile.pluginId === pluginId && activeFile.iframe) {
          activeFile.iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:runCommand", payload: { commandId } }, "*");
          return;
        }
        const iframe = await ensurePluginMounted(pluginId);
        if (!iframe) {
          showToast({ variant: "error", message: "That plugin has nothing to open — its command can't run." });
          return;
        }
        iframe.contentWindow.postMessage({ type: "emit", event: "lowarc:runCommand", payload: { commandId } }, "*");
      }

      // The live array backing the Command Palette / Settings section of the header search — kept
      // as ONE array object, mutated in place (push/splice, never reassigned), because initSearchbar
      // (primitives.js) closes over whatever array it's handed at init time with no way to hand it
      // a fresh one later. Each entry carries `value` (identical to `id`) purely because that's the
      // field name initSearchbar's own rendering reads — letting `options` below just BE this same
      // array, instead of a separate .map()'d copy that would silently stop tracking it.
      const commandCenterEntries = [];

      // The dynamic counterpart to buildPluginCommands() below — see window.lowarc.setCommands() in
      // plugin_assets.rs. Re-registering (Monaco does this on every editor mount) replaces this
      // plugin's previous set rather than piling up duplicates.
      function setDynamicPluginCommands(pluginId, commands) {
        for (let i = commandCenterEntries.length - 1; i >= 0; i--) {
          if (commandCenterEntries[i].dynamicPluginId === pluginId) commandCenterEntries.splice(i, 1);
        }
        for (const cmd of commands) {
          if (!cmd || typeof cmd.id !== "string" || typeof cmd.label !== "string") continue;
          const id = `command:plugin:${pluginId}:${cmd.id}`;
          commandCenterEntries.push({
            id, value: id, label: cmd.label, hint: cmd.hint, category: "commands", kind: "command",
            dynamicPluginId: pluginId,
            run: () => runPluginCommand(pluginId, cmd.id),
          });
        }
      }

      const commandCenterSearch = document.getElementById("command-center-search");
      const commandCenterInput = commandCenterSearch.querySelector("input");

      Promise.all([buildSearchableSettingsList(), buildPluginCommands()]).then(([settingEntries, pluginCommands]) => {
        commandCenterEntries.push(...settingEntries, ...buildHostMenuCommands(), ...pluginCommands);
        commandCenterEntries.forEach((e) => { e.value = e.id; });
        initSearchbar(commandCenterSearch, {
          options: commandCenterEntries,
          categories: HEADER_SEARCH_CATEGORIES,
          onSelect: (value) => {
            const entry = commandCenterEntries.find((e) => e.id === value);
            commandCenterInput.value = "";
            commandCenterSearch.classList.remove("has-value");
            if (!entry) return;
            if (entry.kind === "command") {
              entry.run();
              return;
            }
            showPopup("settings", { url: `settings.html?search=${encodeURIComponent(entry.label)}&id=${encodeURIComponent(entry.id)}` }).catch(reportError);
          },
        });
      });

      // ---------- Notification bell ----------
      // Reads the exact same history every showToast() call (host chrome's own, and any plugin's
      // via window.lowarc.notify()) already feeds — see primitives.js's toast/notification-history
      // section. This panel is read-only history, not an action menu, so it bypasses
      // openMenuOverlay's items/resolve-a-value shape entirely and builds #floating-menu-list's
      // content by hand instead, reusing only the low-level positioning (showFloatingOverlay).
      let unreadNotifications = 0;

      function updateBellBadge() {
        document.getElementById("bell-badge").style.display = unreadNotifications > 0 ? "block" : "none";
      }

      function formatNotificationTime(ms) {
        const diff = Date.now() - ms;
        if (diff < 60000) return "just now";
        if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
        if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`;
        return new Date(ms).toLocaleDateString();
      }

      function renderNotificationList(list) {
        list.innerHTML = "";
        const history = getNotificationHistory();

        const header = document.createElement("div");
        header.className = "notif-panel-header";
        const title = document.createElement("span");
        title.textContent = "Notifications";
        header.appendChild(title);
        if (history.length) {
          const clearBtn = document.createElement("button");
          clearBtn.type = "button";
          clearBtn.className = "notif-panel-clear";
          clearBtn.textContent = "Clear";
          clearBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            clearNotificationHistory();
            renderNotificationList(list);
          });
          header.appendChild(clearBtn);
        }
        list.appendChild(header);

        if (!history.length) {
          const empty = document.createElement("div");
          empty.className = "notif-panel-empty";
          empty.textContent = "No notifications yet.";
          list.appendChild(empty);
          return;
        }

        for (const entry of history) {
          const row = document.createElement("div");
          row.className = `notif-row toast-${entry.variant}`;

          const icon = document.createElement("span");
          icon.className = "toast-icon";
          icon.innerHTML = `<svg viewBox="0 0 16 16" fill="none">${TOAST_ICON_PATHS[entry.variant] || TOAST_ICON_PATHS.info}</svg>`;
          row.appendChild(icon);

          const body = document.createElement("span");
          body.className = "notif-row-body";
          const message = document.createElement("span");
          message.className = "notif-row-message";
          message.textContent = entry.message;
          body.appendChild(message);
          const meta = document.createElement("span");
          meta.className = "notif-row-meta";
          meta.textContent = formatNotificationTime(entry.time) + (entry.source ? ` · ${entry.source}` : "");
          body.appendChild(meta);
          row.appendChild(body);

          const dismissBtn = document.createElement("button");
          dismissBtn.type = "button";
          dismissBtn.className = "notif-row-dismiss";
          dismissBtn.setAttribute("aria-label", "Dismiss");
          dismissBtn.innerHTML = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>';
          dismissBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            removeNotificationHistoryEntry(entry.id);
            renderNotificationList(list);
          });
          row.appendChild(dismissBtn);

          list.appendChild(row);
        }
      }

      function openNotificationPanel(triggerEl) {
        closeFloatingMenu();
        const list = document.getElementById("floating-menu-list");
        list.classList.add("notif-panel-list");
        // Not a menu — a read-only list with per-row dismiss buttons, not a set of actions the
        // overlay itself resolves a choice from (see floatingMenuResolve below).
        list.setAttribute("role", "region");
        list.setAttribute("aria-label", "Notifications");
        renderNotificationList(list);
        floatingMenuResolve = () => {}; // nothing to resolve — dismissal alone is the only outcome

        unreadNotifications = 0;
        updateBellBadge();

        const rect = triggerEl.getBoundingClientRect();
        showFloatingOverlay({ x: rect.left, y: rect.bottom + 2, gap: rect.height + 4 });
      }

      onNotification(() => {
        unreadNotifications++;
        updateBellBadge();
      });

      document.getElementById("bell-trigger").addEventListener("click", (e) => {
        e.stopPropagation();
        openNotificationPanel(e.currentTarget);
      });

      document.getElementById("floating-menu").addEventListener("click", (e) => {
        if (e.target.id === "floating-menu") closeFloatingMenu();
      });

      // Finds the <iframe> a given window belongs to — lets showMenu() (see the message listener
      // below) translate a plugin's own local click coordinates into host/screen coordinates. Every
      // plugin-hosted iframe (sidebar/inspector/console-tab panels AND open-file viewers, in either
      // editor group) carries this same class, so one query covers all of them without a second map
      // to keep in sync with mountPanelIframe/openFile/closeFile.
      function iframeForWindow(win) {
        for (const iframe of document.querySelectorAll("iframe.plugin-panel-frame")) {
          if (iframe.contentWindow === win) return iframe;
        }
        return null;
      }

      document.getElementById("account-menu-trigger").addEventListener("click", (e) => {
        e.stopPropagation();
        openMenuFromTrigger(e.currentTarget, [
          { label: "Not Signed In", disabled: true },
          { divider: true },
          { label: "Sign In…", disabled: true },
        ]);
      });

      document.getElementById("settings-menu-trigger").addEventListener("click", (e) => {
        e.stopPropagation();
        openMenuFromTrigger(e.currentTarget, [
          { label: "Settings", value: "settings" },
          { label: "Appearance…", value: "appearance" },
          { divider: true },
          { label: "Check for Updates", disabled: true },
        ]).then((value) => {
          if (value === "settings") showPopup("settings", { url: "settings.html" });
          else if (value === "appearance") showPopup("settings", { url: "settings.html?tab=appearance" });
        });
      });

