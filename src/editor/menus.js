      const params = new URLSearchParams(window.location.search);
      const projectPath = params.get("project") || "(unknown project)";
      const projectName = projectPath.split(/[\\/]/).pop() || projectPath;
      document.getElementById("status-project").textContent = projectName;

      const { invoke } = window.__TAURI__.core;
      const { open: openDialog, save: saveDialog } = window.__TAURI__.dialog;
      const { openUrl } = window.__TAURI__.opener;

      // Plugin panel/viewer content is hosted on a real loopback HTTP origin, not a Tauri custom
      // URI scheme: on Windows/WebView2, a sub-frame (iframe) navigation to a custom scheme
      // silently never loads at all (confirmed directly: the Rust-side handler was never even
      // invoked). A plain http://127.0.0.1:<port> origin has none of that baggage. The port is
      // ephemeral (OS-assigned at app startup), fetched once here and cached.
      let pluginAssetPortPromise = null;
      function pluginAssetPort() {
        if (!pluginAssetPortPromise) pluginAssetPortPromise = invoke("plugin_asset_port");
        return pluginAssetPortPromise;
      }
      async function pluginAssetUrl(pluginId, relPath, fragment) {
        const port = await pluginAssetPort();
        return `http://127.0.0.1:${port}/${pluginId}/${relPath}${fragment ? `#${fragment}` : ""}`;
      }

      function openEditor(path) {
        window.location.href = `editor.html?project=${encodeURIComponent(path)}`;
      }

      // ---------- Menu bar ----------
      // Every top-bar dropdown shares the same open/close mechanics: click a trigger to open it
      // (closing any other open menu first), click outside/Escape/pick an item to close it, and
      // hovering a different trigger while one is already open switches to it (how a real menu bar
      // behaves) rather than requiring a fresh click. The rail's account and settings flyouts and
      // the console's session menu are not in this list: they open through the shared .floating-menu
      // overlay instead (see openMenuFromTrigger below), because all three sit near a window edge
      // that a locally-anchored dropdown could run off of.
      const MENU_IDS = ["file-menu", "edit-menu", "view-menu", "run-menu", "help-menu"];

      function closeAllMenus() {
        MENU_IDS.forEach((id) => {
          document.getElementById(id).classList.remove("is-open");
          document.getElementById(`${id}-trigger`).setAttribute("aria-expanded", "false");
        });
      }

      MENU_IDS.forEach((id) => {
        const menu = document.getElementById(id);
        const trigger = document.getElementById(`${id}-trigger`);

        // Static markup, so the roles are applied here rather than hand-repeated on every item:
        // the same ARIA shape openMenuOverlay's dynamically-built menus use (menuitem/menuitemcheckbox/
        // separator), just applied once at setup instead of per render since this content never changes.
        const list = menu.querySelector(".menu-dropdown-list");
        if (list) {
          list.setAttribute("role", "menu");
          list.querySelectorAll(".menu-dropdown-item").forEach((item) => {
            item.setAttribute("role", item.querySelector(".menu-dropdown-item-check") ? "menuitemcheckbox" : "menuitem");
          });
          list.querySelectorAll(".menu-dropdown-divider").forEach((div) => div.setAttribute("role", "separator"));
        }

        trigger.addEventListener("click", (e) => {
          e.stopPropagation();
          const opening = !menu.classList.contains("is-open");
          closeAllMenus();
          if (opening) {
            menu.classList.add("is-open");
            trigger.setAttribute("aria-expanded", "true");
          }
        });

        trigger.addEventListener("mouseenter", () => {
          const anyOpen = MENU_IDS.some((mid) => document.getElementById(mid).classList.contains("is-open"));
          if (anyOpen && !menu.classList.contains("is-open")) {
            closeAllMenus();
            menu.classList.add("is-open");
            trigger.setAttribute("aria-expanded", "true");
          }
        });
      });

      document.addEventListener("click", closeAllMenus);
      document.addEventListener("keydown", (e) => {
        if (e.key === "Escape") {
          closeAllMenus();
          closeFloatingMenu();
        }
      });

      document.querySelectorAll(".menu-dropdown-item:not(:disabled)").forEach((btn) => {
        btn.addEventListener("click", closeAllMenus);
      });

      // ---------- Floating menu (shared overlay; see .floating-menu in primitives.css) ----------
      // A menu rendered here is a sibling of every panel and every iframe (see the HTML above,
      // right after .shell closes), so nothing can clip it and it isn't bound to any one trigger's
      // local DOM position the way .menu-dropdown-list is: it's positioned and clamped to the real
      // window on every open instead. Anything that opens near a window edge uses this: the rail's
      // account and settings flyouts, and the console's session menu. A plugin's own content can ask
      // for one too, via window.lowarc.showMenu (see the "showMenu" branch in the listener below).
      let floatingMenuResolve = null;

      function closeFloatingMenu(result) {
        if (!floatingMenuResolve) return;
        const resolve = floatingMenuResolve;
        floatingMenuResolve = null;
        document.getElementById("floating-menu").classList.remove("is-open");
        resolve(result === undefined ? null : result);
      }

      // Every "click outside closes it" overlay this document has, called together: not just
      // from an actual outside click, but also (see the iframe "focus" listeners in
      // mountPanelIframe/mountFileInGroup below) whenever focus moves INTO a plugin iframe, since a
      // click that lands inside a sandboxed iframe never bubbles up to this document at all and so
      // never reaches any of these overlays' own individual document-click listeners. Without this,
      // clicking into Monaco (or any other plugin) while a menu was open left it stuck open:
      // reported live, not theoretical.
      function closeAllOverlays() {
        closeAllMenus();
        closeFloatingMenu();
        closeAllDropdowns();
      }

      // Low-level: positions the shared #floating-menu-list at `anchor` ({x, y} in viewport
      // coordinates, the corner it tries to open from, below-right by default) and shows the
      // overlay. Callers fill the list's content
      // themselves first: shared by openMenuOverlay (action-menu items) and the notification
      // bell's read-only history panel below, since the actual position/clamp/show mechanics are
      // identical for both; only what's inside, and what happens on close, differs.
      function showFloatingOverlay(anchor) {
        const overlay = document.getElementById("floating-menu");
        const list = document.getElementById("floating-menu-list");

        // Positioned at the naive anchor first, then measured and clamped: the list has to
        // actually exist in the DOM with real content before its own rendered size is knowable.
        list.style.left = `${anchor.x}px`;
        list.style.top = `${anchor.y}px`;
        overlay.classList.add("is-open");

        requestAnimationFrame(() => {
          const rect = list.getBoundingClientRect();
          const margin = 6;
          let left = anchor.x;
          let top = anchor.y;
          if (left + rect.width > window.innerWidth - margin) left = Math.max(margin, window.innerWidth - margin - rect.width);
          if (top + rect.height > window.innerHeight - margin) top = Math.max(margin, anchor.y - rect.height - (anchor.gap || 0));
          if (left < margin) left = margin;
          if (top < margin) top = margin;
          list.style.left = `${left}px`;
          list.style.top = `${top}px`;
        });
      }

      // items: [{label, value, disabled, checked}], a divider is {divider: true}. `checked` is
      // optional: only present it for a genuinely checkable menu (see the Rail/Console visibility
      // toggle below); omitting it on every item keeps an ordinary action menu (File/Edit/View, …)
      // looking exactly as it always has, no reserved checkmark gutter. Returns a Promise resolving
      // to the chosen item's value, or null if the menu was dismissed without a choice.
      function openMenuOverlay(items, anchor) {
        closeFloatingMenu();
        return new Promise((resolve) => {
          const list = document.getElementById("floating-menu-list");
          list.innerHTML = "";
          list.classList.remove("notif-panel-list");
          list.setAttribute("role", "menu");

          for (const item of items) {
            if (item.divider) {
              const div = document.createElement("div");
              div.className = "menu-dropdown-divider";
              div.setAttribute("role", "separator");
              list.appendChild(div);
              continue;
            }
            const btn = document.createElement("button");
            btn.type = "button";
            btn.className = "menu-dropdown-item";
            if (item.checked !== undefined) {
              btn.setAttribute("role", "menuitemcheckbox");
              btn.setAttribute("aria-checked", String(Boolean(item.checked)));
              const check = document.createElement("span");
              check.className = "menu-dropdown-item-check";
              if (item.checked) check.innerHTML = CHECKMARK_SVG;
              btn.appendChild(check);
            } else {
              btn.setAttribute("role", "menuitem");
            }
            const label = document.createElement("span");
            label.textContent = item.label;
            btn.appendChild(label);
            btn.disabled = Boolean(item.disabled);
            btn.addEventListener("click", (e) => {
              e.stopPropagation();
              closeFloatingMenu(item.value ?? null);
            });
            list.appendChild(btn);
          }

          floatingMenuResolve = resolve;
          showFloatingOverlay(anchor);
        });
      }

      // Opens just below a real trigger element, with no per-trigger CSS needed near an edge since
      // openMenuOverlay clamps to the window. Also owns aria-expanded on the trigger, so no caller
      // has to remember it.
      //
      // .finally() rather than .then(), because a menu can close through a chosen item, Escape, an
      // outside click, or another menu opening over it, and aria-expanded and focus should return
      // in all of those, not only a deliberate selection.
      function openMenuFromTrigger(triggerEl, items) {
        const rect = triggerEl.getBoundingClientRect();
        triggerEl.setAttribute("aria-expanded", "true");
        return openMenuOverlay(items, { x: rect.left, y: rect.bottom + 2, gap: rect.height + 4 }).finally(() => {
          triggerEl.setAttribute("aria-expanded", "false");
          triggerEl.focus();
        });
      }

      // ---------- Popup contributions (host) ----------
      // showPopup() and the "popups" stack itself live in primitives.js, since settings.html,
      // modules.html and plugins.html all use them too. See the comment there for the mechanics.
      // These are the three editor.html registers for itself.

      // The project's run configuration, which is what project.json actually holds: the entry file
      // and which modules the project requires. Both are editable here; neither was reachable from
      // the IDE before, so a project's requires meant hand-editing the file.
      //
      // target is { projectPath }; resolves true if it saved, false otherwise, so a caller that
      // opened it to fix an unrunnable config knows whether to retry.
      contribute("popups", {
        id: "run-config",
        sourceType: "host",
        title: "Run configuration",
        size: 460,
        mount(container, ctx) {
          const projectPath = ctx.target.projectPath;
          const body = document.createElement("div");
          body.className = "popup-body";

          // Held here and only written on Save, so nothing takes effect until you say so,
          // including a file picker you opened and cancelled.
          let entry = "";
          const selected = new Set();

          // ---- entry file ----
          const entryField = document.createElement("div");
          entryField.className = "field";
          const entryLabel = document.createElement("div");
          entryLabel.className = "field-label";
          entryLabel.textContent = "Entry file";
          entryField.appendChild(entryLabel);

          const entryRow = document.createElement("div");
          entryRow.style.display = "flex";
          entryRow.style.gap = "8px";
          entryRow.style.alignItems = "center";
          const entryValue = document.createElement("input");
          entryValue.className = "input";
          entryValue.type = "text";
          entryValue.readOnly = true;
          entryValue.placeholder = "none set";
          entryValue.style.flex = "1";
          const browseBtn = document.createElement("button");
          browseBtn.type = "button";
          browseBtn.className = "btn btn-md btn-ghost";
          browseBtn.textContent = "Browse…";
          entryRow.appendChild(entryValue);
          entryRow.appendChild(browseBtn);
          entryField.appendChild(entryRow);

          const entryHint = document.createElement("div");
          entryHint.className = "field-hint";
          entryHint.textContent = "Handed to every module as source. Must be inside the project.";
          entryField.appendChild(entryHint);
          body.appendChild(entryField);

          browseBtn.addEventListener("click", async () => {
            const picked = await openDialog({ title: "Choose the project's entry file", defaultPath: projectPath });
            if (!picked) return;
            try {
              // Converted but NOT saved: project.json is only written by the Save button below.
              entry = await invoke("project_relative_entry", { projectDir: projectPath, absoluteEntryPath: picked });
              entryValue.value = entry;
            } catch (err) {
              showToast({ variant: "error", message: String(err) });
            }
          });

          // ---- modules ----
          const modulesField = document.createElement("div");
          modulesField.className = "field";
          modulesField.style.marginTop = "16px";
          const modulesLabel = document.createElement("div");
          modulesLabel.className = "field-label";
          modulesLabel.textContent = "Modules";
          modulesField.appendChild(modulesLabel);

          const list = document.createElement("div");
          list.className = "run-config-modules";
          modulesField.appendChild(list);
          const modulesHint = document.createElement("div");
          modulesHint.className = "field-hint";
          modulesHint.textContent = "Whatever these require is pulled in too, so a dependency needn't be ticked itself.";
          modulesField.appendChild(modulesHint);
          body.appendChild(modulesField);

          container.appendChild(body);

          const actions = document.createElement("div");
          actions.className = "popup-actions";
          const cancelBtn = document.createElement("button");
          cancelBtn.type = "button";
          cancelBtn.className = "btn btn-md btn-ghost";
          cancelBtn.textContent = "Cancel";
          cancelBtn.addEventListener("click", () => ctx.close(false));
          const saveBtn = document.createElement("button");
          saveBtn.type = "button";
          saveBtn.className = "btn btn-md btn-confirm";
          saveBtn.textContent = "Save";
          saveBtn.disabled = true;
          actions.appendChild(cancelBtn);
          actions.appendChild(saveBtn);
          container.appendChild(actions);

          saveBtn.addEventListener("click", async () => {
            saveBtn.disabled = true;
            // The full preset every time, not a patch: same "the frontend always resends
            // everything" convention set_breakpoints and plugin settings already use.
            const preset = {
              entry,
              requires: [...selected].map((id) => ({ id, version: "*", optional: false })),
            };
            try {
              await invoke("set_project_preset", { projectDir: projectPath, preset });
              ctx.close(true);
            } catch (err) {
              showToast({ variant: "error", message: String(err) });
              saveBtn.disabled = false;
            }
          });

          // Loaded after the shell is on screen so the popup never appears empty while waiting on
          // two backend calls; Save stays disabled until there is something real to save.
          (async () => {
            let preset = { entry: "", requires: [] };
            let modules = [];
            try {
              [preset, modules] = await Promise.all([
                invoke("get_project_preset", { projectDir: projectPath }),
                invoke("list_installed_modules"),
              ]);
            } catch (err) {
              showToast({ variant: "error", message: String(err) });
            }

            entry = (preset.entry || "").trim();
            entryValue.value = entry;
            for (const dep of preset.requires || []) {
              if (dep && dep.id) selected.add(dep.id);
            }

            list.textContent = "";
            if (!modules.length) {
              const empty = document.createElement("div");
              empty.className = "field-hint";
              empty.textContent = "No modules are installed. Install one from File > Modules first.";
              list.appendChild(empty);
            }
            for (const m of modules) {
              const row = document.createElement("label");
              row.className = "run-config-module";
              const box = document.createElement("input");
              box.type = "checkbox";
              box.checked = selected.has(m.id);
              box.addEventListener("change", () => {
                if (box.checked) selected.add(m.id);
                else selected.delete(m.id);
              });
              const text = document.createElement("span");
              text.className = "run-config-module-name";
              text.textContent = m.name;
              const meta = document.createElement("span");
              meta.className = "run-config-module-meta";
              meta.textContent = m.id + (m.version ? ` · v${m.version}` : "");
              row.appendChild(box);
              row.appendChild(text);
              row.appendChild(meta);
              list.appendChild(row);
            }

            // A requirement whose module has since been uninstalled has no checkbox of its own, so
            // saving would quietly drop it. It gets a row anyway: still ticked, so saving keeps it
            // — and stays untickable so the stale entry can actually be removed. Showing it as
            // plain text would make it visible but unfixable, which is the worse half of both.
            const installedIds = new Set(modules.map((m) => m.id));
            for (const id of [...selected].filter((i) => !installedIds.has(i))) {
              const row = document.createElement("label");
              row.className = "run-config-module is-missing";
              const box = document.createElement("input");
              box.type = "checkbox";
              box.checked = true;
              box.addEventListener("change", () => {
                if (box.checked) selected.add(id);
                else selected.delete(id);
              });
              const text = document.createElement("span");
              text.className = "run-config-module-name";
              text.textContent = id;
              const meta = document.createElement("span");
              meta.className = "run-config-module-meta";
              meta.textContent = "required but not installed";
              row.appendChild(box);
              row.appendChild(text);
              row.appendChild(meta);
              list.appendChild(row);
            }

            saveBtn.disabled = false;
          })();
        },
      });

      contribute("popups", {
        id: "new-project",
        sourceType: "host",
        title: "New project name",
        size: 360,
        mount(container, ctx) {
          const body = document.createElement("div");
          body.className = "popup-body";
          const field = document.createElement("div");
          field.className = "field";
          const nameInput = document.createElement("input");
          nameInput.className = "input";
          nameInput.type = "text";
          nameInput.placeholder = "my-game";
          nameInput.autocomplete = "off";
          field.appendChild(nameInput);
          body.appendChild(field);
          const errorEl = document.createElement("div");
          errorEl.className = "field-hint";
          errorEl.style.color = "var(--danger)";
          errorEl.style.minHeight = "16px";
          errorEl.style.marginTop = "6px";
          body.appendChild(errorEl);
          container.appendChild(body);

          const create = async () => {
            try {
              const path = await invoke("create_project", { parentDir: ctx.target.parentDir, name: nameInput.value });
              ctx.close(path);
            } catch (err) {
              errorEl.textContent = String(err);
            }
          };
          appendConfirmActions(container, { confirmLabel: "Create", onCancel: () => ctx.close(null), onConfirm: create });
          nameInput.addEventListener("keydown", (e) => {
            if (e.key === "Enter") create();
          });
        },
      });

      contribute("popups", {
        id: "about",
        sourceType: "host",
        title: "About",
        size: 320,
        mount(container, ctx) {
          const body = document.createElement("div");
          body.className = "popup-body";
          body.style.textAlign = "center";
          body.style.padding = "8px 16px 20px";
          const img = document.createElement("img");
          img.src = "la-logo.svg";
          img.alt = "";
          img.style.height = "48px";
          img.style.marginBottom = "12px";
          body.appendChild(img);
          const name = document.createElement("div");
          name.style.fontSize = "15px";
          name.style.fontWeight = "600";
          name.textContent = "LowArc Studio";
          body.appendChild(name);
          const version = document.createElement("div");
          version.className = "field-hint";
          version.style.marginTop = "4px";
          version.textContent = (ctx.target && ctx.target.version) || "";
          body.appendChild(version);
          container.appendChild(body);
        },
      });

      // A reference list, not a settings surface: nothing here is rebindable yet. The editing rows
      // are Monaco's own bindings, live only while an editor tab has focus. Save, Save As and the
      // Command Palette are the host's, working from any panel EXCEPT while a plugin iframe is
      // focused, since a sandboxed iframe's keydown never reaches this document. That is a real
      // cross-iframe limitation rather than a bug.
      const SHORTCUT_GROUPS = [
        { keys: "Ctrl+S", label: "Save" },
        { keys: "Ctrl+Shift+S", label: "Save As…" },
        { keys: "Ctrl+Shift+P", label: "Command Palette" },
        { keys: "Ctrl+Z", label: "Undo" },
        { keys: "Ctrl+Y", label: "Redo" },
        { keys: "Ctrl+X", label: "Cut" },
        { keys: "Ctrl+C", label: "Copy" },
        { keys: "Ctrl+V", label: "Paste" },
        { keys: "Ctrl+F", label: "Find" },
        { keys: "Ctrl+H", label: "Replace" },
      ];

      contribute("popups", {
        id: "shortcuts",
        sourceType: "host",
        title: "Keyboard Shortcuts",
        size: 340,
        mount(container, ctx) {
          const body = document.createElement("div");
          body.className = "popup-body";
          for (const { keys, label } of SHORTCUT_GROUPS) {
            const row = document.createElement("div");
            row.className = "shortcut-row";
            const labelEl = document.createElement("span");
            labelEl.textContent = label;
            const keysEl = document.createElement("span");
            keysEl.className = "shortcut-keys";
            keysEl.textContent = keys;
            row.appendChild(labelEl);
            row.appendChild(keysEl);
            body.appendChild(row);
          }
          container.appendChild(body);
        },
      });

      // Shared by both in-editor managers (plugin, module): only one is ever open at a time, and
      // each call just passes its own nounSingular/name as target, so there's no reason for two
      // near-identical contributions.
      contribute("popups", {
        id: "manager-remove",
        sourceType: "host",
        title: (target) => `Remove ${target.nounSingular}?`,
        size: 340,
        mount(container, ctx) {
          const body = document.createElement("div");
          body.className = "popup-body";
          body.textContent = `This deletes "${ctx.target.name}" from disk. This can't be undone.`;
          container.appendChild(body);

          appendConfirmActions(container, {
            confirmLabel: "Remove",
            confirmVariant: "danger",
            onCancel: () => ctx.close(false),
            onConfirm: () => ctx.close(true),
          });
        },
      });

      // Gates closeFile() (see the tab-bar/open-files section below) on a real confirmation
      // whenever the file being closed is dirty. Target is just { title }, resolves true/false
      // like manager-remove above.
      contribute("popups", {
        id: "confirm-close-dirty",
        sourceType: "host",
        title: "Unsaved changes",
        size: 360,
        mount(container, ctx) {
          const body = document.createElement("div");
          body.className = "popup-body";
          body.textContent = `"${ctx.target.title}" has unsaved changes. Close it anyway?`;
          container.appendChild(body);

          appendConfirmActions(container, {
            confirmLabel: "Close Anyway",
            confirmVariant: "danger",
            onCancel: () => ctx.close(false),
            onConfirm: () => ctx.close(true),
          });
        },
      });

      // Gates the whole APP window closing (see confirmAppClose() in tabs-inspector.js and
      // initWindowControls's beforeClose param in primitives.js): same shape as
      // confirm-close-dirty above, just for "the window itself is about to close" rather than one
      // tab. target is { message, canSave }: canSave is false when the only unsaved work is an
      // open Draft (nothing here for a Save button to write — Commit/Revert are its resolutions,
      // not this popup's). Resolves "save" | "discard" | false, one 3-button row hand-built here
      // instead of via appendConfirmActions (which only ever does Cancel + one other button).
      contribute("popups", {
        id: "confirm-close-app",
        sourceType: "host",
        title: "Unsaved changes",
        size: 380,
        mount(container, ctx) {
          const body = document.createElement("div");
          body.className = "popup-body";
          body.textContent = ctx.target.message;
          container.appendChild(body);

          const actions = document.createElement("div");
          actions.className = "popup-actions";

          const cancelBtn = document.createElement("button");
          cancelBtn.type = "button";
          cancelBtn.className = "btn btn-md btn-ghost";
          cancelBtn.textContent = "Cancel";
          cancelBtn.addEventListener("click", () => ctx.close(false));
          actions.appendChild(cancelBtn);

          const discardBtn = document.createElement("button");
          discardBtn.type = "button";
          discardBtn.className = "btn btn-md btn-danger";
          discardBtn.textContent = "Close Anyway";
          discardBtn.addEventListener("click", () => ctx.close("discard"));
          actions.appendChild(discardBtn);

          if (ctx.target.canSave) {
            const saveBtn = document.createElement("button");
            saveBtn.type = "button";
            saveBtn.className = "btn btn-md btn-confirm";
            saveBtn.textContent = "Save & Close";
            saveBtn.addEventListener("click", () => ctx.close("save"));
            actions.appendChild(saveBtn);
          }

          container.appendChild(actions);
        },
      });

      // Settings, Modules and Plugins stay their own separate documents, since they need real
      // Tauri API access and several entry points each. See contributeIframePopup() in
      // primitives.js.
      // The url passed to each showPopup() call below is what actually varies per trigger (e.g.
      // settings.html's own ?tab=appearance), not the registration itself.
      contributeIframePopup("settings", { title: "Settings" });
      contributeIframePopup("modules", { title: "Modules", forwardEvents: ["install-progress"] });
      contributeIframePopup("plugins", { title: "Plugins", forwardEvents: ["install-progress"] });
      contributeIframePopup("export", { title: "Export", forwardEvents: ["export-log", "export-ended"] });

