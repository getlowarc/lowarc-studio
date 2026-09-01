      const params = new URLSearchParams(window.location.search);
      const projectPath = params.get("project") || "(unknown project)";
      const projectName = projectPath.split(/[\\/]/).pop() || projectPath;
      document.getElementById("status-project").textContent = projectName;

      const { invoke } = window.__TAURI__.core;
      const { open: openDialog, save: saveDialog } = window.__TAURI__.dialog;
      const { openUrl } = window.__TAURI__.opener;

      // Plugin panel/viewer content is hosted on a real loopback HTTP origin, not a Tauri custom
      // URI scheme — on Windows/WebView2, a sub-frame (iframe) navigation to a custom scheme
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
      // behaves) rather than requiring a fresh click. The rail's account/settings flyouts and the
      // console's "New Terminal With…" used to be part of this same MENU_IDS-driven list (each its
      // own local .menu-dropdown) — they've moved to the shared .floating-menu overlay instead (see
      // openMenuFromTrigger below), since all three sit near a window edge that a locally-anchored
      // dropdown could run off of.
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

      // ---------- Floating menu (shared overlay — see .floating-menu in primitives.css) ----------
      // A menu rendered here is a sibling of every panel and every iframe (see the HTML above,
      // right after .shell closes), so nothing can clip it and it isn't bound to any one trigger's
      // local DOM position the way .menu-dropdown-list is — it's positioned and clamped to the real
      // window on every open instead. Everything that used to be a local .menu-dropdown near a
      // window edge (the rail's account/settings flyouts, the console's "New Terminal With…") now
      // opens through this; a plugin's own content can ask for one too, via window.lowarc.showMenu
      // (see the "showMenu" branch in the message listener below).
      let floatingMenuResolve = null;

      function closeFloatingMenu(result) {
        if (!floatingMenuResolve) return;
        const resolve = floatingMenuResolve;
        floatingMenuResolve = null;
        document.getElementById("floating-menu").classList.remove("is-open");
        resolve(result === undefined ? null : result);
      }

      // Every "click outside closes it" overlay this document has, called together — not just
      // from an actual outside click, but also (see the iframe "focus" listeners in
      // mountPanelIframe/mountFileInGroup below) whenever focus moves INTO a plugin iframe, since a
      // click that lands inside a sandboxed iframe never bubbles up to this document at all and so
      // never reaches any of these overlays' own individual document-click listeners. Without this,
      // clicking into Monaco (or any other plugin) while a menu was open left it stuck open —
      // reported live, not theoretical.
      function closeAllOverlays() {
        closeAllMenus();
        closeFloatingMenu();
        closeAllDropdowns();
      }

      // Low-level: positions the shared #floating-menu-list at `anchor` ({x, y} in viewport
      // coordinates, the corner it tries to open from — below-right by default, matching where
      // .menu-dropdown-list used to open) and shows the overlay. Callers fill the list's content
      // themselves first — shared by openMenuOverlay (action-menu items) and the notification
      // bell's read-only history panel below, since the actual position/clamp/show mechanics are
      // identical for both; only what's inside, and what happens on close, differs.
      function showFloatingOverlay(anchor) {
        const overlay = document.getElementById("floating-menu");
        const list = document.getElementById("floating-menu-list");

        // Positioned at the naive anchor first, then measured and clamped — the list has to
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

      const MENU_CHECK_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M3 8l3.5 3.5L13 5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>';

      // items: [{label, value, disabled, checked}], a divider is {divider: true}. `checked` is
      // optional — only present it for a genuinely checkable menu (see the Rail/Console visibility
      // toggle below); omitting it on every item keeps an ordinary action menu (File/Edit/View, …)
      // looking exactly as it always has, no reserved checkmark gutter. Returns a Promise resolving
      // to the chosen item's value, or null if the menu was dismissed without a choice.
      function openMenuOverlay(items, anchor) {
        closeFloatingMenu();
        return new Promise((resolve) => {
          const list = document.getElementById("floating-menu-list");
          list.innerHTML = "";
          list.classList.remove("notif-panel-list");

          for (const item of items) {
            if (item.divider) {
              const div = document.createElement("div");
              div.className = "menu-dropdown-divider";
              list.appendChild(div);
              continue;
            }
            const btn = document.createElement("button");
            btn.type = "button";
            btn.className = "menu-dropdown-item";
            if (item.checked !== undefined) {
              const check = document.createElement("span");
              check.className = "menu-dropdown-item-check";
              if (item.checked) check.innerHTML = MENU_CHECK_SVG;
              btn.appendChild(check);
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

      // Convenience for a real trigger element — opens just below it, same default corner
      // .menu-dropdown-list used to, minus the per-trigger CSS overrides that used to be needed
      // for triggers near an edge (openMenuOverlay's own clamping replaces those).
      function openMenuFromTrigger(triggerEl, items) {
        const rect = triggerEl.getBoundingClientRect();
        return openMenuOverlay(items, { x: rect.left, y: rect.bottom + 2, gap: rect.height + 4 });
      }

      // ---------- Popup contributions (host) ----------
      // showPopup()/the "popups" stack itself now lives in primitives.js (promoted there once
      // settings.html/modules.html/plugins.html adopted it too — see the comment on it there for
      // the full mechanics). These are the three popups editor.html used to hand-author as static
      // HTML + openPopup(id)/closePopup(id) (primitives.js's older, simple element-toggle pair —
      // still used as-is by anything that hasn't migrated).

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

          const actions = document.createElement("div");
          actions.className = "popup-actions";
          const cancelBtn = document.createElement("button");
          cancelBtn.type = "button";
          cancelBtn.className = "btn btn-md btn-ghost";
          cancelBtn.textContent = "Cancel";
          cancelBtn.addEventListener("click", () => ctx.close(null));
          const createBtn = document.createElement("button");
          createBtn.type = "button";
          createBtn.className = "btn btn-md btn-confirm";
          createBtn.textContent = "Create";
          actions.appendChild(cancelBtn);
          actions.appendChild(createBtn);
          container.appendChild(actions);

          const create = async () => {
            try {
              const path = await invoke("create_project", { parentDir: ctx.target.parentDir, name: nameInput.value });
              ctx.close(path);
            } catch (err) {
              errorEl.textContent = String(err);
            }
          };
          createBtn.addEventListener("click", create);
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

      // A plain reference list, not a settings surface — nothing here is configurable yet, so
      // there's no rebinding UI, just what each combination currently does. The Undo/Redo/Cut/
      // Copy/Paste/Find/Replace rows are Monaco's own built-in keybindings (only live while an
      // editor tab has focus); Save/Save As/Command Palette are the host's own, added alongside
      // the matching menu items and working regardless of which panel has focus, EXCEPT while a
      // plugin iframe (an editor tab included) is the one actually focused — a sandboxed iframe's
      // keydown never reaches this document, so the host-level bindings only fire from host chrome
      // (the sidebar, empty space, etc.), a real cross-iframe limitation, not a bug.
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

      // Shared by both in-editor managers (plugin, module) — only one is ever open at a time, and
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

          const actions = document.createElement("div");
          actions.className = "popup-actions";
          const cancelBtn = document.createElement("button");
          cancelBtn.type = "button";
          cancelBtn.className = "btn btn-md btn-ghost";
          cancelBtn.textContent = "Cancel";
          cancelBtn.addEventListener("click", () => ctx.close(false));
          const removeBtn = document.createElement("button");
          removeBtn.type = "button";
          removeBtn.className = "btn btn-md btn-danger";
          removeBtn.textContent = "Remove";
          removeBtn.addEventListener("click", () => ctx.close(true));
          actions.appendChild(cancelBtn);
          actions.appendChild(removeBtn);
          container.appendChild(actions);
        },
      });

      // Gates closeFile() (see the tab-bar/open-files section below) on a real confirmation
      // whenever the file being closed is dirty — target is just { title }, resolves true/false
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

          const actions = document.createElement("div");
          actions.className = "popup-actions";
          const cancelBtn = document.createElement("button");
          cancelBtn.type = "button";
          cancelBtn.className = "btn btn-md btn-ghost";
          cancelBtn.textContent = "Cancel";
          cancelBtn.addEventListener("click", () => ctx.close(false));
          const discardBtn = document.createElement("button");
          discardBtn.type = "button";
          discardBtn.className = "btn btn-md btn-danger";
          discardBtn.textContent = "Close Anyway";
          discardBtn.addEventListener("click", () => ctx.close(true));
          actions.appendChild(cancelBtn);
          actions.appendChild(discardBtn);
          container.appendChild(actions);
        },
      });

      // Settings/Modules/Plugins stay their own separate documents (real Tauri API access,
      // multiple entry points already before this) — see contributeIframePopup() in primitives.js.
      // The url passed to each showPopup() call below is what actually varies per trigger (e.g.
      // settings.html's own ?tab=appearance), not the registration itself.
      contributeIframePopup("settings", { title: "Settings" });
      contributeIframePopup("modules", { title: "Modules", forwardEvents: ["install-progress"] });
      contributeIframePopup("plugins", { title: "Plugins", forwardEvents: ["install-progress"] });
      contributeIframePopup("export", { title: "Export", forwardEvents: ["export-log", "export-ended"] });

