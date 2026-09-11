      // ---------- Base system: shared region renderers ----------
      // Built on primitives.js's contribute()/getSlot() registry.

      // Builds a button per contribution in `slot` into `container`, which initTabs() and any
      // initReorderable() call have already wired. This only adds the buttons and their
      // activate/close behaviour on top. Safe to call repeatedly: it rebuilds only the elements it
      // owns, and listener wiring is guarded so repeated calls do not stack duplicates.
      //
      // opts:
      //   itemClass
      //   anchorEl        a trailing non-item child tabs must stay before, like the console's
      //                   "+" and spacer. Omit when there is none.
      //   iconBased       default true, for icon-only tabs with a tooltip label. False for a
      //                   text-label strip like the console's or a file tab.
      //   emptyText       shown in the strip when the slot has no contributions. Omit for a strip
      //                   that is fine looking empty, like the rail.
      //   onActivate      (contribution) a tab became the active one.
      //   onToggleClose   (contribution, btn) optional. Clicking the ALREADY-active tab calls this
      //                   instead of onActivate. Return false to let the click fall through.
      //   onClose         (contribution) optional, only for a contribution with closeable === true.
      //   decorate        (contribution, tabEl) optional, called per tab as it is built, so a
      //                   caller can apply status classes from its own live state rather than from
      //                   the contribution, which stays a static declaration.
      //
      // An icon-based contribution's icon is either static (iconHtml) or async (resolveIcon(), a
      // plugin's railIcon over IPC, showing FALLBACK_RAIL_ICON_SVG until it resolves).
      function renderTabStrip(container, slot, opts) {
        const { itemClass = "sidebar-tab", anchorEl = null, iconBased = true, emptyText = null, onActivate, onToggleClose, onClose, decorate } = opts;

        container.setAttribute("role", "tablist");
        container.querySelectorAll("[data-tab-value], .tab-bar-empty").forEach((el) => el.remove());

        // Filtered here, not inside getSlot() itself. Hiding a tab/icon is purely about whether
        // it's SHOWN in this one strip, never about disabling the contribution. Every other
        // consumer of getSlot() (showSlotTab, activateConsoleTab, ensurePluginMounted, the Command
        // Palette, …) still needs to see and reach a hidden contribution exactly as before.
        const contributions = getSlot(slot).filter((c) => !isSlotItemHidden(slot, c.id));

        if (!contributions.length && emptyText) {
          const empty = document.createElement("div");
          empty.className = "tab-bar-empty";
          empty.textContent = emptyText;
          container.insertBefore(empty, anchorEl);
        }

        for (const contribution of contributions) {
          const closeable = contribution.closeable === true;
          // A closeable tab hosts a real nested <button> (the close control). Buttons can't nest,
          // so it is a div standing in for one (role, tabindex, keydown-triggers-click). A
          // non-closeable tab stays a real <button>.
          const tab = document.createElement(closeable ? "div" : "button");
          tab.className = itemClass;
          tab.dataset.tabValue = contribution.id;
          tab.setAttribute("role", "tab");
          tab.setAttribute("aria-selected", "false");

          if (closeable) {
            tab.tabIndex = 0;
            tab.addEventListener("keydown", (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                tab.click();
              } else if ((e.key === "Delete" || e.key === "Backspace") && onClose) {
                // The close button itself is deliberately NOT in the Tab order (see below): this
                // is the actual keyboard path to closing a tab, reachable the moment the tab
                // itself has focus rather than requiring a second, invisible-until-hover target.
                e.preventDefault();
                onClose(contribution);
              }
            });
          } else {
            tab.type = "button";
          }

          if (iconBased) {
            tab.dataset.tooltip = contribution.label;
            tab.setAttribute("aria-label", contribution.label);
            tab.innerHTML = contribution.iconHtml || FALLBACK_RAIL_ICON_SVG;
            if (contribution.resolveIcon) {
              contribution.resolveIcon().then((svg) => {
                if (!svg) return;
                svg.setAttribute("width", "16");
                svg.setAttribute("height", "16");
                tab.innerHTML = "";
                tab.appendChild(svg);
              });
            }
          } else if (closeable) {
            const label = document.createElement("span");
            label.textContent = contribution.label;
            tab.appendChild(label);
          } else {
            tab.textContent = contribution.label;
          }

          if (closeable) {
            const close = document.createElement("button");
            close.type = "button";
            close.className = "btn btn-icon-only btn-xs btn-ghost-danger file-tab-close";
            close.setAttribute("aria-label", `Close ${contribution.label}`);
            // Visually hidden except on hover/active (see .file-tab-close in editor.html's CSS) —
            // out of the Tab order entirely rather than a focusable target a keyboard user could
            // land on without being able to see it. Still a real, mouse-clickable button; Delete/
            // Backspace on the focused tab itself (see the keydown handler above) is the keyboard
            // equivalent.
            close.tabIndex = -1;
            close.innerHTML =
              '<svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>';
            close.addEventListener("click", (e) => {
              // Without this, the click bubbles up to the tab bar and the primitive's own
              // delegation would treat it as "activate this tab" too, right as it's closing.
              e.stopPropagation();
              if (onClose) onClose(contribution);
            });
            tab.appendChild(close);
          }

          if (decorate) decorate(contribution, tab);
          container.insertBefore(tab, anchorEl);
        }

        applySavedOrder(container, "[data-tab-value]", "tabValue", anchorEl);
        initTooltips();

        if (container.dataset.tabStripWired) return;
        container.dataset.tabStripWired = "true";

        if (onActivate) {
          container.addEventListener("tab-change", (e) => {
            const contribution = getSlot(slot).find((c) => c.id === e.detail.value);
            if (contribution) onActivate(contribution);
          });
        }

        // Toggle-close-on-reclick, generic here rather than hand-wired per region, so every tab
        // strip gets it from one place. Capture phase: observes pre-click state
        // before [data-tabs]' own bubble-phase handler (primitives.js's initTabs()) mutates
        // is-active, and (only once onToggleClose actually handles it), stops that handler from
        // also firing and immediately reopening what this just closed.
        if (onToggleClose) {
          container.addEventListener(
            "click",
            (e) => {
              const btn = e.target.closest("[data-tab-value]");
              if (!btn || !btn.classList.contains("is-active")) return;
              const contribution = getSlot(slot).find((c) => c.id === btn.dataset.tabValue);
              if (!contribution) return;
              if (onToggleClose(contribution, btn) !== false) e.stopPropagation();
            },
            true,
          );
        }
      }

      // ---------- Rail/Console icon visibility (right-click to show/hide individual entries) ----------
      // A display filter only, independent of whether a plugin is enabled: hiding an icon removes
      // it from THIS strip, while the contribution stays reachable everywhere else, including the
      // Command Palette. Scoped to the Rail and Console, though renderTabStrip's filter is generic
      // enough to work for any slot once a menu is wired up for one.
      function isSlotItemHidden(slot, id) {
        return (loadedSettings?.hiddenSlotItems?.[slot] || []).includes(id);
      }

      // renderTabStrip rebuilds its buttons from scratch on every call (see its own comment): that
      // means whichever one was is-active loses that class too, since it's a fresh element with no
      // memory of it. The startup restore path (restoredSidebarActiveKey, below) already has to
      // solve this same problem once; toggling hidden state re-solves it here every time instead of
      // only at load. A now-hidden active item legitimately has nothing to re-highlight: that's
      // correct, not a bug: its content stays open regardless (a plugin's own tab or panel is never
      // unmounted by any of this, only the tab/icon that points at it stops rendering).
      async function toggleSlotItemHidden(slot, id) {
        await updateSettings((fresh) => {
          const current = new Set(fresh.hiddenSlotItems?.[slot] || []);
          if (current.has(id)) current.delete(id);
          else current.add(id);
          return { hiddenSlotItems: { ...(fresh.hiddenSlotItems || {}), [slot]: [...current] } };
        });
        if (slot === "sidebar") {
          const activeKey = document.querySelector("#rail-tabs .sidebar-tab.is-active")?.dataset.tabValue;
          renderRailTabs();
          const btn = activeKey && document.querySelector(`#rail-tabs .sidebar-tab[data-tab-value="${activeKey}"]`);
          if (btn) {
            btn.classList.add("is-active");
            btn.setAttribute("aria-selected", "true");
          }
        } else if (slot === "console") {
          renderConsoleTabs();
          activateConsoleTab(activeConsoleTabKey);
        }
      }

      function renderRailTabs() {
        renderTabStrip(document.getElementById("rail-tabs"), "sidebar", {
          itemClass: "sidebar-tab",
          onActivate: (contribution) => {
            showSlotTab("sidebar", "left-panel-body", contribution.id);
            setPanelOpen("sidebar", true);
          },
          onToggleClose: (contribution, btn) => {
            if (!PANELS.sidebar.open) return false;
            btn.classList.remove("is-active");
            btn.setAttribute("aria-selected", "false");
            setPanelOpen("sidebar", false);
          },
        });
      }

      function renderConsoleTabs() {
        renderTabStrip(document.getElementById("console-tabs"), "console", {
          itemClass: "console-tab",
          iconBased: false,
          anchorEl: document.querySelector(".console-tabs-spacer"),
          onActivate: (contribution) => activateConsoleTab(contribution.id),
        });
      }

      // Lists every contribution currently registered for `slot`, including hidden ones: the whole
      // point of this menu is letting a hidden one be turned back on, so it can't just read what's
      // already shown. One click toggles that item and closes the menu (right-click again for more),
      // matching how every other menu in the app already behaves rather than inventing a
      // stays-open-across-clicks variant just for this.
      function openSlotVisibilityMenu(slot, x, y) {
        const items = getSlot(slot).map((c) => ({ label: c.label, value: c.id, checked: !isSlotItemHidden(slot, c.id) }));
        if (!items.length) return;
        openMenuOverlay(items, { x, y }).then((id) => {
          if (id) toggleSlotItemHidden(slot, id);
        });
      }

      // Bound to the whole #rail nav, not just #rail-tabs — #rail-tabs is only as tall as its own
      // icons (its own spacer, .rail-spacer, is a SIBLING outside it, unlike the console's own
      // spacer which lives INSIDE #console-tabs and so already made that whole row clickable). A
      // right-click below the last icon, or with every icon currently hidden, would otherwise hit
      // #rail with nothing listening — exactly the case this menu most needs to be reachable from.
      document.getElementById("rail").addEventListener("contextmenu", (e) => {
        e.preventDefault();
        e.stopPropagation();
        openSlotVisibilityMenu("sidebar", e.clientX, e.clientY);
      });
      document.getElementById("console-tabs").addEventListener("contextmenu", (e) => {
        e.preventDefault();
        e.stopPropagation();
        openSlotVisibilityMenu("console", e.clientX, e.clientY);
      });

      // extension (lowercase, dot-included) -> { pluginId, viewer }. Same "first wins, no silent
      // override" rule showSlotTab's slot registry follows generally, for the same reason: two
      // viewers silently fighting over one file type would be worse than an honest, logged
      // "already taken".
      const viewersByExtension = new Map();

      function registerViewer(pluginId, viewer) {
        for (const ext of viewer.extensions || []) {
          const key = ext.toLowerCase();
          if (viewersByExtension.has(key)) {
            console.warn(`[plugins] "${key}" is already viewed by "${viewersByExtension.get(key).pluginId}" — ignoring the same contribution from "${pluginId}".`);
            continue;
          }
          viewersByExtension.set(key, { pluginId, viewer });
        }
      }

      async function loadPluginContributions() {
        let plugins = [];
        try {
          plugins = await invoke("list_installed_plugins");
        } catch (err) {
          showToast({ variant: "error", message: String(err) });
          return;
        }

        for (const plugin of plugins) {
          if (plugin.disabled || !plugin.contributes) continue;

          for (const panel of plugin.contributes.panels || []) {
            const key = panelKey(plugin.id, panel.id);
            pluginPanels.set(key, { pluginId: plugin.id, panel, iframe: null, session: plugin.session });
            if (panel.location === "sidebar") {
              contribute("sidebar", {
                id: key,
                sourceType: "plugin",
                pluginId: plugin.id,
                label: panel.title,
                resolveIcon: panel.railIcon ? () => loadRailIconSvg(plugin.id, panel.railIcon) : undefined,
                mount(container) {
                  mountPanelIframe(key, container).classList.add("is-active");
                },
              });
            } else if (panel.location === "inspector") {
              contribute("inspector", {
                id: key,
                sourceType: "plugin",
                pluginId: plugin.id,
                mount(container) {
                  mountPanelIframe(key, container).classList.add("is-active");
                },
              });
            }
          }

          for (const tab of plugin.contributes.consoleTabs || []) {
            const key = panelKey(plugin.id, tab.id);
            pluginPanels.set(key, { pluginId: plugin.id, panel: tab, iframe: null, session: plugin.session });
            contribute("console", {
              id: key,
              sourceType: "plugin",
              pluginId: plugin.id,
              label: tab.title,
              mount(el) {
                mountPanelIframe(key, el).classList.add("is-active");
              },
            });
          }

          for (const viewer of plugin.contributes.viewers || []) {
            registerViewer(plugin.id, viewer);
          }
        }

        // Deliberately NOT mounted here: unlike the sidebar/console, nothing about the Inspector
        // should exist until something actually asks for it (see showInspector()/openInspector()
        // below). A plugin contributing an inspector panel just sits registered in the slot
        // registry until its first real claim.

        // Built once, here, from everything contribute("sidebar", ...) registered above (both
        // managers, registered earlier at top-level script scope, plus whatever this loop just
        // added) — applySavedOrder's drag-order pass happens inside renderTabStrip itself, so it
        // only needs applying once, now that every icon that could be in it actually exists.
        renderRailTabs();

        // Same "built once, from the registry" shape as the rail above, just text-labeled
        // (iconBased: false) and anchored before .console-tabs-spacer instead of nothing, since
        // this container also holds the maximize/close/new-terminal controls after its tabs.
        renderConsoleTabs();

        // "__run" (Run) needs its one-time mount(), which reparents the static #console-run-panel
        // out of console-body and into its own wrapper — to happen right now, unconditionally, not
        // just whenever a user happens to click the Run tab first. Until that reparenting happens,
        // #console-run-panel's own permanently-baked-in "is-active" class (see its static HTML)
        // makes it render on its own regardless of any other tab's wrapper: a same-height phantom
        // sibling stacking underneath whatever tab genuinely is active, which shows up as
        // unexplained extra scroll height as soon as any other tab becomes active first. This does
        // not open the console panel, since activateConsoleTab() never does; it just establishes
        // Run as the active tab underneath, matching what the static HTML visually implies.
        activateConsoleTab("__run");

        // [data-tabs] only marks .is-active in response to a real click (see initTabs() in
        // primitives.js). It doesn't know about restoredSidebarActiveKey, the tab that was active
        // when this layout was last saved (see savePanelLayout/loadPanelLayout). Re-activate that
        // same rail icon now that the icons actually exist, so a session that closed with a rail
        // tab selected reopens showing that same panel instead of an empty one.
        const restoredBtn = restoredSidebarActiveKey && document.querySelector(`#rail-tabs .sidebar-tab[data-tab-value="${restoredSidebarActiveKey}"]`);
        if (restoredBtn) {
          restoredBtn.classList.add("is-active");
          restoredBtn.setAttribute("aria-selected", "true");
          showSlotTab("sidebar", "left-panel-body", restoredSidebarActiveKey);
        }

        // Whether or not that restore succeeded (the saved key might belong to a plugin that's no
        // longer installed), PANELS.sidebar.open can still be stale — it's restored from settings
        // independently of whether any rail tab ended up active. A panel claiming "open" with no
        // active tab behind it is a blank strip that looks broken, the same failure shape as the
        // console-maximize/xterm bugs already fixed this session. Force it shut if that's the state.
        if (PANELS.sidebar.open && !document.querySelector("#rail-tabs .sidebar-tab.is-active")) {
          setPanelOpen("sidebar", false);
        }
      }

