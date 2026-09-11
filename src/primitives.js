// Behavior for the dropdown / checkbox-dropdown / searchbar / numeric-input / toast / popup
// primitives. Dropdowns, checkbox-dropdowns, and popups are driven entirely by data attributes,
// so any page can drop in the markup from primitives.css with no per-instance wiring. A searchbar
// needs a real data source to filter against, so it's exposed as a function (initSearchbar)
// instead of auto-init, and a toast has no fixed markup to init, being created on demand.

// The one checkmark glyph this app uses: a checkbox's own check, a menu's checked-item indicator
// (menus.js), a manager panel's "Enabled" checkbox (plugin-hosting.js). Defined once here (loads
// first, see editor.html's script order) rather than hand-copied at each of those.
const CHECKMARK_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M3 8l3.5 3.5L13 5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>';
// Same X glyph plugins/terminal/terminal.js, plugins/debugger/debugger.js, and
// plugins/node-graph/inspector.js each already declare their own copy of (as DELETE_SVG). Those
// are separate sandboxed plugin documents with no shared module system to pull this from, but a
// host-side Remove control has no such excuse, so it lives here once.
const DELETE_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>';

// Appends a .popup-actions row with a Cancel button plus one other action button, the shape most
// simple popups need: a delete or discard confirmation, a create-with-validation form's
// Cancel and Create pair. Returns the
// confirm button so a caller needing more than a bare click listener (new-project's own async
// validation, triggered by Enter in its input too; see menus.js) can hold onto it.
function appendConfirmActions(container, { cancelLabel = "Cancel", confirmLabel, confirmVariant = "confirm", onCancel, onConfirm } = {}) {
  const actions = document.createElement("div");
  actions.className = "popup-actions";
  const cancelBtn = document.createElement("button");
  cancelBtn.type = "button";
  cancelBtn.className = "btn btn-md btn-ghost";
  cancelBtn.textContent = cancelLabel;
  if (onCancel) cancelBtn.addEventListener("click", onCancel);
  const confirmBtn = document.createElement("button");
  confirmBtn.type = "button";
  confirmBtn.className = `btn btn-md btn-${confirmVariant}`;
  confirmBtn.textContent = confirmLabel;
  if (onConfirm) confirmBtn.addEventListener("click", onConfirm);
  actions.appendChild(cancelBtn);
  actions.appendChild(confirmBtn);
  container.appendChild(actions);
  return confirmBtn;
}

// Also matches .searchbar[data-open] (the header's command-center search, see initSearchbar
// below) even though it has no [data-dropdown] attribute of its own; it's a bespoke widget, not
// one of the data-attribute-driven primitives above, but shares the same open/closed convention
// and needs to close from the same triggers: an outside click, Escape, and focus moving into a
// plugin iframe (see editor.html's iframe focus listener).
function closeAllDropdowns(except) {
  document.querySelectorAll('[data-dropdown][data-open="true"], .searchbar[data-open="true"]').forEach((el) => {
    if (el !== except) el.dataset.open = "false";
  });
}

function updateMultiselectSummary(el, valueEl) {
  const checked = Array.from(el.querySelectorAll('[data-dropdown-option] input[type="checkbox"]:checked'));
  if (checked.length === 0) {
    valueEl.textContent = valueEl.dataset.placeholder || "None selected";
    valueEl.classList.add("placeholder");
  } else if (checked.length === 1) {
    valueEl.textContent = checked[0].closest("[data-dropdown-option]").dataset.label || "1 selected";
    valueEl.classList.remove("placeholder");
  } else {
    valueEl.textContent = `${checked.length} selected`;
    valueEl.classList.remove("placeholder");
  }
}

function initDropdowns(root = document) {
  root.querySelectorAll("[data-dropdown]").forEach((el) => {
    if (el.dataset.dropdownInit) return;
    el.dataset.dropdownInit = "true";

    const trigger = el.querySelector("[data-dropdown-trigger]");
    const menu = el.querySelector("[data-dropdown-menu]");
    const valueEl = el.querySelector("[data-dropdown-value]");
    const multiselect = el.dataset.multiselect === "true";

    trigger.addEventListener("click", () => {
      const willOpen = el.dataset.open !== "true";
      closeAllDropdowns(el);
      el.dataset.open = willOpen ? "true" : "false";
    });

    menu.addEventListener("click", (e) => {
      const option = e.target.closest("[data-dropdown-option]");
      if (!option || option.classList.contains("is-empty")) return;

      if (multiselect) {
        const checkbox = option.querySelector('input[type="checkbox"]');
        if (checkbox) checkbox.checked = !checkbox.checked;
        updateMultiselectSummary(el, valueEl);
      } else {
        menu.querySelectorAll("[data-dropdown-option]").forEach((o) => o.classList.remove("is-selected"));
        option.classList.add("is-selected");
        valueEl.textContent = option.dataset.label || option.textContent.trim();
        valueEl.classList.remove("placeholder");
        el.dataset.value = option.dataset.value || "";
        el.dataset.open = "false";
      }

      el.dispatchEvent(new CustomEvent("dropdown-change", { bubbles: true }));
    });
  });
}

// No system right-click menu anywhere the app doesn't build its own. A row or element with a custom
// context menu (file-explorer's tree rows, via window.lowarc.showMenu()) calls preventDefault()
// itself before this ever runs, so that path is unaffected; this only removes the default for
// everything else. Plugin iframes get the equivalent listener from HARNESS_JS (plugin_assets.rs)
// since this document-level one can't reach into a separate iframe document.
document.addEventListener("contextmenu", (e) => e.preventDefault());

document.addEventListener("click", (e) => {
  // .searchbar is excluded from the outside-click check for the same reason closeAllDropdowns()
  // itself now matches it (see that function's comment), and it has no [data-dropdown] attribute of
  // its own, so without this a click that OPENS it (via the input's own focus handler, see
  // initSearchbar) would immediately be undone by this same click bubbling here, closing it again
  // before it was ever visible. initSearchbar's own outside-click listener already excludes
  // .searchbar the same way; this just makes this listener consistent with that one.
  if (!e.target.closest("[data-dropdown]") && !e.target.closest(".searchbar")) closeAllDropdowns();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeAllDropdowns();
});

function initNumericInputs(root = document) {
  root.querySelectorAll(".numeric-input").forEach((el) => {
    if (el.dataset.numericInit) return;
    el.dataset.numericInit = "true";

    const input = el.querySelector("input");
    const up = el.querySelector(".stepper-up");
    const down = el.querySelector(".stepper-down");
    const min = input.hasAttribute("min") ? Number(input.min) : -Infinity;
    const max = input.hasAttribute("max") ? Number(input.max) : Infinity;
    const step = input.hasAttribute("step") ? Number(input.step) : 1;

    const clamp = (n) => Math.min(max, Math.max(min, n));

    const bump = (delta) => {
      const current = Number(input.value) || 0;
      input.value = clamp(current + delta);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    };

    up.addEventListener("click", () => bump(step));
    down.addEventListener("click", () => bump(-step));
    input.addEventListener("blur", () => {
      if (input.value === "") return;
      input.value = clamp(Number(input.value) || 0);
    });
  });
}

// One selection behavior for both sidebar-tab and header-tab lists, which are the same "exactly
// one active item" logic under different skins, driven by [data-tabs] wrapping [data-tab-value]
// buttons.
function initTabs(root = document) {
  root.querySelectorAll("[data-tabs]").forEach((el) => {
    if (el.dataset.tabsInit) return;
    el.dataset.tabsInit = "true";

    el.addEventListener("click", (e) => {
      const tab = e.target.closest("[data-tab-value]");
      if (!tab || !el.contains(tab)) return;

      el.querySelectorAll("[data-tab-value]").forEach((t) => {
        t.classList.remove("is-active");
        if (t.getAttribute("role") === "tab") t.setAttribute("aria-selected", "false");
      });
      tab.classList.add("is-active");
      if (tab.getAttribute("role") === "tab") tab.setAttribute("aria-selected", "true");
      el.dispatchEvent(new CustomEvent("tab-change", { bubbles: true, detail: { value: tab.dataset.tabValue } }));
    });
  });
}

// Drag-to-reorder for a tab or rail strip. Opt-in per container via data-reorderable, checked here
// rather than left to callers, since most strips in this app have no business being reorderable.
// Pointer Events rather than native drag-and-drop: setPointerCapture keeps tracking regardless of
// what is visually underneath, which native DnD does not.
//
// itemSelector picks the draggable children; keyAttr names the dataset property read for an item's
// identity; axis is "x" for a horizontal strip or "y" for the rail. onReorder(orderedKeys) fires
// once on a completed drag that ends in this container, not on every intermediate move, so a
// caller persisting to disk is not doing that per pixel.
//
// crossContainer and onMoveAcross(key, targetContainer, beforeKey) allow dropping into a SECOND
// reorderable container, the editor's two split groups each naming the other. crossZone is the
// larger element counting as "over the other side", typically the whole pane rather than its thin
// tab strip; the indicator still renders inside crossContainer, which is what has the siblings to
// position against. Defaults to crossContainer.
//
// A collapsed crossZone has offsetParent === null, checked explicitly below rather than trusting a
// hidden element's rect to sit at (0,0). On a cross-container drop the DOM node is left alone and
// onReorder does NOT fire: onMoveAcross updates the real state and re-renders both sides, which
// would discard any DOM surgery done here.
//
// handleSelector narrows where a drag can start. Omitted, the whole item is the handle, right for
// a tab that is nothing but its label. The manager rows expand to show a description and buttons,
// so they pass their header instead: an expanded row reorders by its title bar and its body
// behaves like ordinary content.
function initReorderable(container, { itemSelector = "[data-tab-value]", keyAttr = "tabValue", axis = "x", handleSelector, onReorder, crossContainer, crossZone, onMoveAcross } = {}) {
  if (!container || !container.hasAttribute("data-reorderable")) return;
  if (container.dataset.reorderInit) return;
  container.dataset.reorderInit = "true";

  const THRESHOLD = 4;
  const indicator = document.createElement("div");
  indicator.className = "reorder-indicator " + (axis === "x" ? "is-x" : "is-y");

  function itemsIn(targetContainer, except) {
    return Array.from(targetContainer.querySelectorAll(itemSelector)).filter((el) => el !== except);
  }

  function withinRect(el, x, y) {
    const rect = el.getBoundingClientRect();
    return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
  }

  // Finds where, among the OTHER items in `targetContainer`, the pointer currently sits, and moves
  // the (already-detached-looking, via .is-dragging) indicator there, before the first item whose
  // midpoint the pointer has passed, or right after the last item if the pointer is past all of
  // them (never past the container's own trailing non-item children, e.g. the console's spacer/
  // new-terminal controls. appendChild-ing straight onto the container would land the indicator
  // there instead of at the actual end of the tab strip).
  function placeIndicatorIn(targetContainer, clientPos, dragging) {
    const others = itemsIn(targetContainer, dragging);
    let before = null;
    for (const item of others) {
      const rect = item.getBoundingClientRect();
      const mid = axis === "x" ? rect.left + rect.width / 2 : rect.top + rect.height / 2;
      if (clientPos < mid) {
        before = item;
        break;
      }
    }
    if (before) targetContainer.insertBefore(indicator, before);
    else if (others.length) others[others.length - 1].after(indicator);
    else targetContainer.appendChild(indicator);
  }

  let justDragged = false;

  container.addEventListener("pointerdown", (e) => {
    const item = e.target.closest(itemSelector);
    if (!item || !container.contains(item)) return;
    // Checked against THIS item's own handle, not just any match on the page. A nested reorderable
    // would otherwise let a child's handle start a drag of its ancestor.
    if (handleSelector) {
      const handle = e.target.closest(handleSelector);
      if (!handle || !item.contains(handle)) return;
    }

    const startPos = axis === "x" ? e.clientX : e.clientY;
    let dragging = false;
    let activeContainer = container;

    const onMove = (moveEvent) => {
      const pos = axis === "x" ? moveEvent.clientX : moveEvent.clientY;
      if (!dragging) {
        if (Math.abs(pos - startPos) < THRESHOLD) return;
        dragging = true;
        item.setPointerCapture(e.pointerId);
        item.classList.add("is-dragging");
        container.classList.add("is-reordering");
      }

      const zone = crossZone || crossContainer;
      const crossVisible = crossContainer && zone.offsetParent !== null;
      const target = crossVisible && withinRect(zone, moveEvent.clientX, moveEvent.clientY) ? crossContainer : container;
      if (target !== activeContainer) {
        activeContainer.classList.remove("is-reordering");
        target.classList.add("is-reordering");
        activeContainer = target;
      }
      placeIndicatorIn(activeContainer, pos, item);
    };

    const removeListeners = () => {
      container.removeEventListener("pointermove", onMove);
      container.removeEventListener("pointerup", onUp);
      container.removeEventListener("pointercancel", onCancel);
    };

    const onUp = (upEvent) => {
      removeListeners();
      if (!dragging) return;

      item.releasePointerCapture(upEvent.pointerId);
      item.classList.remove("is-dragging");
      activeContainer.classList.remove("is-reordering");
      justDragged = true;

      if (activeContainer === container) {
        if (indicator.parentElement === container) container.insertBefore(item, indicator);
        indicator.remove();
        if (onReorder) onReorder(itemsIn(container, null).map((el) => el.dataset[keyAttr]));
      } else {
        const beforeEl = indicator.nextElementSibling;
        indicator.remove();
        if (onMoveAcross) onMoveAcross(item.dataset[keyAttr], activeContainer, beforeEl && beforeEl !== item ? beforeEl.dataset[keyAttr] : null);
      }
    };

    // A pointercancel, meaning lost pointer capture from a system dialog, alt-tab or a touch
    // interruption, ends a drag the same way pointerup would but without a real drop to commit. The item
    // was never actually moved in the DOM during the drag (only the indicator was), so just
    // dropping the indicator and clearing state IS the revert; nothing to undo beyond that.
    const onCancel = (cancelEvent) => {
      removeListeners();
      if (!dragging) return;

      item.releasePointerCapture(cancelEvent.pointerId);
      item.classList.remove("is-dragging");
      activeContainer.classList.remove("is-reordering");
      indicator.remove();
    };

    container.addEventListener("pointermove", onMove);
    container.addEventListener("pointerup", onUp);
    container.addEventListener("pointercancel", onCancel);
  });

  // A genuine drag still ends in a real click event on release (pointerup with no movement since
  // the last frame doesn't prevent the browser's own click synthesis). Capture phase, same
  // "observe and stop before the item's own click handler runs" shape as the rail's toggle-close
  // listener, so dragging a tab to reorder it doesn't also activate it as a side effect.
  container.addEventListener(
    "click",
    (e) => {
      if (justDragged) {
        justDragged = false;
        e.stopPropagation();
        e.preventDefault();
      }
    },
    true,
  );
}

// Wires a plain text input to filter `options` ({value, label, category?}[]) into the shared
// dropdown-menu popover.
//
// `categories` (optional): an ORDERED [{id, label, browsable?}] list. Matches group under a
// labeled divider per category in that order. An empty category renders no divider, and an option
// naming no listed id falls into an unlabeled bucket at the end rather than being dropped.
//
// Two states. Focusing an EMPTY bar opens a browse view: every browsable category starts collapsed
// to its divider and chevron, expanding on click (previewLimit defaults to 0, so a category is a
// navigational heading until opened). Typing switches to normal filtered results. Clearing back to
// empty returns a freshly collapsed browse view, so an expand does not outlive the search it was
// opened for.
function initSearchbar(el, { options, onSelect, categories, previewLimit = 0 } = {}) {
  const input = el.querySelector("input");
  const menu = el.querySelector("[data-dropdown-menu]");
  const clearBtn = el.querySelector(".searchbar-clear");
  const expanded = new Set();

  const renderOption = (opt) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "dropdown-option";
    button.setAttribute("data-dropdown-option", ""); // the click handler below matches on this attribute, not the class
    button.dataset.value = opt.value;
    button.textContent = opt.label;
    return button;
  };

  // The chevron is always shown, not just when a group overflows previewLimit. A category with
  // fewer items (or none at all, e.g. Command Palette before there's a real command registry) still
  // renders its divider and chevron. The point is the section exists and is discoverable now; what
  // populates it is a separate, later concern. Expanding one with nothing to reveal is a harmless
  // no-op, not something worth special-casing away.
  const renderGroup = (id, label, items, truncatable) => {
    const isExpanded = expanded.has(id);
    const overflowing = truncatable && items.length > previewLimit;
    const shown = overflowing && !isExpanded ? items.slice(0, previewLimit) : items;

    // The whole divider (title, line, chevron) is the hitbox, rather than just the chevron glyph,
    // which on its own is a target a few pixels across. The chevron is purely decorative, a plain
    // span, with the toggle living on this element instead;
    // role="button"/tabindex/keydown are what a real <button> would have given it for free.
    const labelEl = document.createElement("div");
    labelEl.className = "dropdown-menu-group-label";
    labelEl.setAttribute("role", "button");
    labelEl.tabIndex = 0;
    labelEl.setAttribute("aria-label", isExpanded ? "Show fewer" : "Show more");
    const toggle = () => {
      if (isExpanded) expanded.delete(id);
      else expanded.add(id);
      renderBrowse();
    };
    labelEl.addEventListener("click", (e) => {
      e.stopPropagation();
      toggle();
    });
    labelEl.addEventListener("keydown", (e) => {
      if (e.key !== "Enter" && e.key !== " ") return;
      e.preventDefault();
      e.stopPropagation();
      toggle();
    });

    const title = document.createElement("span");
    title.className = "dropdown-menu-group-title";
    title.textContent = label;
    labelEl.appendChild(title);
    const line = document.createElement("span");
    line.className = "dropdown-menu-group-line";
    labelEl.appendChild(line);

    const chevron = document.createElement("span");
    chevron.className = "dropdown-menu-group-chevron" + (isExpanded ? " is-expanded" : "");
    chevron.innerHTML = '<svg viewBox="0 0 10 10" fill="none"><path d="M2.5 4l2.5 2.5L7.5 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>';
    labelEl.appendChild(chevron);

    menu.appendChild(labelEl);
    shown.forEach((opt) => menu.appendChild(renderOption(opt)));
  };

  // `cats`: the exact ordered category list to render, every one of them, whether or not it has
  // any current matches (Command Palette shows up empty until there's a real command registry
  // behind it; that's the point, not a bug to gate around). truncatable: whether overflowing
  // groups get capped-with-a-chevron (browse view) or shown in full (a real search's actual
  // results, since there is nothing to truncate: a query is already the user narrowing things down
  // themselves).
  const renderGrouped = (matches, cats, truncatable) => {
    menu.innerHTML = "";
    if (!cats) {
      matches.forEach((opt) => menu.appendChild(renderOption(opt)));
      return;
    }
    for (const { id, label } of cats) {
      renderGroup(id, label, matches.filter((o) => o.category === id), truncatable);
    }
    const known = new Set(cats.map((c) => c.id));
    matches.filter((o) => !known.has(o.category)).forEach((opt) => menu.appendChild(renderOption(opt)));
  };

  // browsable: false (e.g. in-file search, before there's a query to search with) is the one thing
  // that's STILL excluded here: a category with genuinely no meaning yet versus one that's simply
  // unpopulated are different states, and only the latter is what this update stopped hiding.
  const renderBrowse = () => {
    const browsableCats = (categories || []).filter((c) => c.browsable !== false);
    const browsableIds = new Set(browsableCats.map((c) => c.id));
    const items = categories ? options.filter((o) => browsableIds.has(o.category)) : options;
    renderGrouped(items, categories ? browsableCats : null, true);
    el.dataset.open = (categories ? browsableCats.length > 0 : items.length > 0) ? "true" : "false";
  };

  const renderResults = (matches) => {
    renderGrouped(matches, categories, false);
    el.dataset.open = (categories ? categories.length > 0 : matches.length > 0) ? "true" : "false";
  };

  input.addEventListener("input", () => {
    const query = input.value.trim().toLowerCase();
    el.classList.toggle("has-value", input.value.length > 0);
    if (!query) {
      renderBrowse();
      return;
    }
    renderResults(options.filter((o) => o.label.toLowerCase().includes(query)));
  });

  input.addEventListener("focus", () => {
    if (!input.value.trim()) renderBrowse();
  });

  menu.addEventListener("click", (e) => {
    const option = e.target.closest("[data-dropdown-option]");
    if (!option) return;
    input.value = option.textContent;
    el.dataset.open = "false";
    if (onSelect) onSelect(option.dataset.value, option.textContent);
  });

  clearBtn.addEventListener("click", () => {
    input.value = "";
    el.classList.remove("has-value");
    renderBrowse();
    input.focus();
  });

  document.addEventListener("click", (e) => {
    if (!e.target.closest(".searchbar")) {
      el.dataset.open = "false";
      expanded.clear();
    }
  });
}

// ---------- Base system: slot registry ----------
// One shared content registry every "Base" region (sidebar, inspector, console, a center-panel
// group, popups) reads from, replacing the copy of "plugin vs host content, open/close, active
// tab" logic that today lives separately in HOST_SIDEBAR_PANELS/pluginPanels/addSidebarRailIcon/
// addConsoleTab/claimSingleSlot/openFiles (editor.html). Not a class hierarchy: one Map plus plain
// functions, same style as everything else in this file. Nothing calls contribute()/getSlot() yet;
// each region migrates onto this one at a time (see the Base-system plan).
const slotRegistry = new Map(); // slot id -> Contribution[]
const KNOWN_SLOTS = new Set(["sidebar", "inspector", "console", "center-0", "center-1", "popups"]);

// contribution: { id, sourceType: "host" | "plugin", pluginId, label, icon, order, closeable,
// when, mount }. id must be unique within this slot; mount(container) is called lazily, the first
// time this contribution is actually shown (mirrors editor.html's mountPanelIframe "create once"
// caching), never eagerly, so a `when`-gated contribution that isn't currently shown never runs
// whatever setup mount() does (a lesson from a related system's own regression: a hidden-but-
// mounted section can still run background work, e.g. an interval, that a truly unmounted one
// wouldn't).
function contribute(slot, contribution) {
  if (!KNOWN_SLOTS.has(slot)) throw new Error(`contribute(): unknown slot "${slot}"`);
  if (!contribution || !contribution.id) throw new Error("contribute(): contribution needs an id");
  if (!slotRegistry.has(slot)) slotRegistry.set(slot, []);
  const list = slotRegistry.get(slot);
  if (list.some((c) => c.id === contribution.id)) {
    throw new Error(`contribute(): duplicate id "${contribution.id}" in slot "${slot}"`);
  }
  list.push(contribution);
}

// Returns this slot's contributions, filtered by each contribution's own `when(ctx)` (default:
// always shown) and sorted by declared `order` (default 0). The caller is expected to apply the
// user's own drag-order on top of this as a final override, the same two-step shape
// sortByOrder()/applySavedOrder() already use in editor.html today.
function getSlot(slot, ctx) {
  const list = slotRegistry.get(slot) || [];
  return list.filter((c) => (c.when ? c.when(ctx) : true)).sort((a, b) => (a.order || 0) - (b.order || 0));
}

// Plugin disable/uninstall, or any other dynamic teardown of a previously-contributed entry.
function removeContribution(slot, id) {
  const list = slotRegistry.get(slot);
  if (!list) return;
  const idx = list.findIndex((c) => c.id === id);
  if (idx !== -1) list.splice(idx, 1);
}

// ---------- Toast / notification history ----------
// One system, not two: every toast IS a notification. showToast() pushes here unconditionally. A
// page with a bell reads it back via getNotificationHistory() and onNotification(); a page without
// one never looks, at the cost of an array push per toast. Session-only and capped, since a toast
// is an ephemeral status message and "what did I miss" means since this page loaded.
const NOTIFICATION_HISTORY_CAP = 200;
const notificationHistory = [];
let notificationAddedListener = null;
let notificationIdSeq = 0;

// Registers the one listener a page's notification UI cares about (there's only ever one bell) —
// fires once per new entry, right after it's pushed to history.
function onNotification(onAdd) {
  notificationAddedListener = onAdd;
}

function getNotificationHistory() {
  return notificationHistory;
}

function clearNotificationHistory() {
  notificationHistory.length = 0;
}

function removeNotificationHistoryEntry(id) {
  const index = notificationHistory.findIndex((n) => n.id === id);
  if (index !== -1) notificationHistory.splice(index, 1);
}

const TOAST_ICON_PATHS = {
  info: '<circle cx="8" cy="8" r="6.4" stroke="currentColor" stroke-width="1.4" /><path d="M8 7.3v4M8 5.1v.1" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />',
  warning:
    '<path d="M8 2.4l6.3 11.2H1.7L8 2.4z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" /><path d="M8 6.7v3.1M8 11.4v.1" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />',
  success:
    '<circle cx="8" cy="8" r="6.4" stroke="currentColor" stroke-width="1.4" /><path d="M5.3 8.2l1.8 1.8 3.6-4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />',
  error:
    '<circle cx="8" cy="8" r="6.4" stroke="currentColor" stroke-width="1.4" /><path d="M5.8 5.8l4.4 4.4M10.2 5.8l-4.4 4.4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />',
};

function getToastStack() {
  let stack = document.querySelector(".toast-stack");
  if (!stack) {
    stack = document.createElement("div");
    stack.className = "toast-stack";
    document.body.appendChild(stack);
  }
  return stack;
}

// variant: "info" | "warning" | "success" | "error". duration is ms before auto-dismiss, or 0 to
// require a manual close. source, if given, is the id of the plugin that asked for this (see
// window.lowarc.notify() in plugin_assets.rs), recorded in the notification history but not
// shown in the toast itself, which has no room for attribution. Returns a dismiss() function so
// the caller can close it early (e.g. once a longer operation the toast was reporting on has moved
// past what it said).
function showToast({ variant = "info", message, duration = 4000, source = null } = {}) {
  const entry = { id: ++notificationIdSeq, variant, message, time: Date.now(), source };
  notificationHistory.unshift(entry);
  if (notificationHistory.length > NOTIFICATION_HISTORY_CAP) notificationHistory.length = NOTIFICATION_HISTORY_CAP;
  if (notificationAddedListener) notificationAddedListener(entry);

  const stack = getToastStack();

  const toast = document.createElement("div");
  toast.className = `toast toast-${variant}`;
  toast.innerHTML = `
    <span class="toast-icon"><svg viewBox="0 0 16 16" fill="none">${TOAST_ICON_PATHS[variant] || TOAST_ICON_PATHS.info}</svg></span>
    <span class="toast-body"></span>
    <button type="button" class="toast-close" aria-label="Dismiss"><svg viewBox="0 0 16 16" width="12" height="12" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg></button>
  `;
  // Set via textContent, not innerHTML, so a message containing user-provided text (a project
  // name, a file path) can't be interpreted as markup.
  toast.querySelector(".toast-body").textContent = message;

  let dismissed = false;
  const dismiss = () => {
    if (dismissed) return;
    dismissed = true;
    toast.classList.add("is-leaving");
    toast.addEventListener("transitionend", () => toast.remove(), { once: true });
    setTimeout(() => toast.remove(), 300);
  };

  toast.querySelector(".toast-close").addEventListener("click", dismiss);
  stack.appendChild(toast);

  if (duration > 0) {
    setTimeout(dismiss, duration);
  }

  return dismiss;
}

// The one-line "show a caught error as a toast" wrapper every page's own catch blocks kept
// redefining independently (startup.js, and the manager page factory below, among others) —
// String(err) so both a real Error and a plain string/Rust-side error message display the same way.
function reportError(err) {
  showToast({ variant: "error", message: String(err) });
}

// ---------- Popup (Base) ----------
// A "popups" slot in the registry above, but a STACK rather than a single mounted slot: showing
// one does not replace another, and a popup can open a second on top of it. contribute() registers
// what a popup IS; showPopup(id, target) opens a fresh instance every call, never cached, since a
// nested open of the same id needs its own.
//
// Uses .popup-backdrop/.popup/.popup-header/.popup-body/.popup-actions, with visibility as a plain
// inline style.display rather than a class toggle.
//
// Any page that wants this needs a `<div class="popup-stack" id="popup-stack"></div>` in its own
// markup, as a sibling of the page's main content. Same "nothing can clip it" placement reasoning
// as editor.html's #floating-menu; see editor.html, settings.html, modules.html or plugins.html.
const popupStack = [];
const POPUP_Z_FLOOR = 200; // above floating-menu (60), below tooltip (300) / toast-stack (500)

function topPopup() {
  return popupStack.length ? popupStack[popupStack.length - 1] : null;
}

// contribution: { id, sourceType, pluginId, title (a string, or (target) => string), size (px
// width; omit for the CSS default), large (a near-fullscreen popup rather than a dialog),
// closeOnBackdrop (default true), closeOnEscape (default true), mount(container, ctx) }. mount
// receives the .popup element, already carrying the header, title and X, and appends its own
// .popup-body and .popup-actions.
//
// ctx = { close(result), target, header, onClose(fn) }. header is the .popup-header element, so
// mount() can append tabs or buttons beside the title; see .popup-header-extras for the layout.
// onClose runs exactly once whenever the instance closes, whatever triggered it, for content that
// needs teardown.
//
// A host contribution's mount() runs in this script, so it is wrapped in try/catch below. A
// plugin's cannot break anything here, since nothing calls into a plugin iframe directly.
const FOCUSABLE_SELECTOR = "input, textarea, select, button, [tabindex]";

// Every element inside `el` that's actually reachable by Tab right now, used both to pick the
// popup's initial focus target and to trap Tab at the popup's own boundary. Computed fresh on
// every call rather than cached once, since mount()'d content (or a popup's own async rendering)
// can add/remove focusable elements over the popup's lifetime. offsetParent === null filters out
// anything hidden (display:none, or a not-currently-visible tab/section within the popup body).
function focusableIn(el) {
  return Array.from(el.querySelectorAll(FOCUSABLE_SELECTOR)).filter((node) => node.offsetParent !== null && !node.disabled);
}

function showPopup(id, target) {
  const contribution = getSlot("popups").find((c) => c.id === id);
  if (!contribution) return Promise.reject(new Error(`showPopup(): no popup contributed with id "${id}"`));

  // Restored once this instance closes (see close() below). Without this, closing a popup whose
  // content focused something inside itself (see the initial-focus call at the end of this
  // function) drops focus to <body> the moment backdrop.remove() takes that element out of the
  // document, leaving a keyboard user with no idea where they are any more.
  const previouslyFocused = document.activeElement;

  return new Promise((resolve) => {
    const depth = popupStack.length;
    const backdrop = document.createElement("div");
    backdrop.className = "popup-backdrop";
    backdrop.style.display = "flex";
    backdrop.style.zIndex = String(POPUP_Z_FLOOR + depth * 10);

    const box = document.createElement("div");
    box.className = "popup" + (contribution.large ? " popup-large" : "") + (contribution.growToContent ? " popup-grow" : "");
    // growToContent replaces the width story entirely (auto, capped near the viewport; see
    // .popup-grow) rather than layering on top of a fixed one, so an explicit `size` is ignored
    // when it's set; the two are different answers to the same question, not compatible together.
    if (contribution.size && !contribution.growToContent) box.style.width = `${contribution.size}px`;

    const header = document.createElement("div");
    header.className = "popup-header";
    const h2 = document.createElement("h2");
    h2.textContent = typeof contribution.title === "function" ? contribution.title(target) : contribution.title || "";
    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.className = "popup-close";
    closeBtn.dataset.tooltip = "Close";
    closeBtn.setAttribute("aria-label", "Close");
    closeBtn.innerHTML =
      '<svg viewBox="0 0 16 16" width="14" height="14" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>';
    header.appendChild(h2);
    header.appendChild(closeBtn);
    box.appendChild(header);
    closeBtn.addEventListener("click", () => close(null));
    backdrop.appendChild(box);

    const instance = { closeOnBackdrop: contribution.closeOnBackdrop !== false, closeOnEscape: contribution.closeOnEscape !== false };
    let closed = false;
    const cleanupFns = [];
    const close = (result) => {
      if (closed) return;
      closed = true;
      cleanupFns.forEach((fn) => {
        try {
          fn();
        } catch (err) {
          console.error("[popup] onClose cleanup failed:", err);
        }
      });
      const idx = popupStack.indexOf(instance);
      if (idx !== -1) popupStack.splice(idx, 1);
      backdrop.remove();
      // Only if it's still a real, visible part of the page. The thing that opened this popup
      // may itself be gone by now (a manager row this popup just confirmed deleting, say), and
      // focusing a detached/hidden element either throws or silently does nothing useful.
      if (previouslyFocused && document.body.contains(previouslyFocused) && previouslyFocused.offsetParent !== null) {
        previouslyFocused.focus();
      }
      resolve(result === undefined ? null : result);
    };
    instance.close = close;

    backdrop.addEventListener("click", (e) => {
      if (e.target === backdrop && instance.closeOnBackdrop) close(null);
    });

    // Focus trap: Tab/Shift+Tab wraps at the popup's own first/last focusable element instead of
    // escaping to whatever's behind the (visually blocking, but not otherwise inert) backdrop.
    // Only intercepts the wrap-around case; everything in between still tabs through normally.
    box.addEventListener("keydown", (e) => {
      if (e.key !== "Tab") return;
      const focusables = focusableIn(box);
      if (!focusables.length) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    });

    try {
      contribution.mount(box, { close, target, header, onClose: (fn) => cleanupFns.push(fn) });
    } catch (err) {
      const errorBody = document.createElement("div");
      errorBody.className = "popup-body";
      errorBody.style.color = "var(--danger)";
      errorBody.textContent = `This popup failed to load: ${err}`;
      box.appendChild(errorBody);
      showToast({ variant: "error", message: `Popup "${id}" failed to render: ${err}`, source: contribution.pluginId || null });
    }

    document.getElementById("popup-stack").appendChild(backdrop);
    popupStack.push(instance);
    initTooltips(box);

    const focusable = focusableIn(box)[0];
    if (focusable) focusable.focus();
  });
}

// Only the TOP popup reacts to Escape, and closeOnBackdrop/closeOnEscape are read independently per
// contribution rather than coupled, so a dialog that disables backdrop-click doesn't also lose its
// only other way out.
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  const top = topPopup();
  if (top && top.closeOnEscape) top.close(null);
});

// ---------- Progress bar ----------
// setProgress(el, fraction) is the only way callers should touch a .progress element, since it owns
// both the fill width and the red->yellow->green color together, so nothing else has to re-derive
// that color logic per caller. Reads --danger/--yellow/--success from computed style rather than
// hard-coding their RGB values, so this automatically follows whatever theme (including a custom
// saved preset) is actually active instead of assuming the default palette.
function readThemeRgb(varName) {
  const hex = getComputedStyle(document.documentElement).getPropertyValue(varName).trim();
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  return m ? [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)] : [128, 128, 128];
}

// Two segments (red->yellow, then yellow->green) rather than one gradient sampled at a point. A
// straight three-stop RGB blend muddies through brown around the midpoint, which reads as neither
// "warning" nor "progressing." Segmenting keeps every point along the way looking like an actual
// traffic-light color, not an interpolated smear between them.
function progressColorFor(fraction) {
  const t = Math.max(0, Math.min(1, fraction));
  const red = readThemeRgb("--danger");
  const yellow = readThemeRgb("--yellow");
  const green = readThemeRgb("--success");
  const [from, to, localT] = t < 0.5 ? [red, yellow, t / 0.5] : [yellow, green, (t - 0.5) / 0.5];
  const mix = (a, b) => Math.round(a + (b - a) * localT);
  return `rgb(${mix(from[0], to[0])}, ${mix(from[1], to[1])}, ${mix(from[2], to[2])})`;
}

function setProgress(el, fraction) {
  const fill = el.classList.contains("progress-fill") ? el : el.querySelector(".progress-fill");
  if (!fill) return;
  const clamped = Math.max(0, Math.min(1, fraction));
  fill.style.width = `${clamped * 100}%`;
  fill.style.backgroundColor = progressColorFor(clamped);
}

// ---------- Settings registry (shared metadata for General + Plugins + search) ----------
// Core (non-plugin) settings have no dynamic schema source the way a plugin's plugin.json does, so
// this is the one hand-maintained list, so add here as new core settings are introduced. Shape
// matches plugin_host::protocol::PluginSettingField (key/label/type/hint/placeholder/options) so
// both flow through the exact same renderSettingRow() below; nothing downstream needs to know
// "core" and "plugin" are different origins.
const CORE_SETTINGS_SCHEMA = [
  {
    key: "devRunTargetFps",
    label: "Dev run target FPS",
    type: "number",
    hint: "Frame rate used when running a project from the editor.",
    placeholder: "60",
  },
];

// Flat list of every searchable setting, core plus every installed plugin's declared fields, as
// pure metadata (id/label/hint/type/tab/source/category), no get/set. Any page with `invoke`
// available can call this (editor.html for the header search, settings.html for its own
// rendering); neither needs the other's document, since each layers its own value-plumbing on top
// by id. `category: "settings"` is this list's own contribution to the header search's grouped
// results (see initSearchbar's `categories` option), distinct from `tab`, which is which
// settings.html tab an entry belongs to, a completely different grouping for a different UI.
async function buildSearchableSettingsList() {
  const invoke = relayableInvoke;
  const entries = CORE_SETTINGS_SCHEMA.map((field) => ({
    id: `core:${field.key}`,
    key: field.key,
    label: field.label,
    hint: field.hint,
    type: field.type,
    options: field.options || [],
    placeholder: field.placeholder,
    tab: "general",
    source: null,
    category: "settings",
  }));

  let plugins = [];
  try {
    plugins = await invoke("list_installed_plugins");
  } catch (err) {
    // A page that can't reach the plugin list (shouldn't happen) just gets core settings only —
    // better than throwing and losing search entirely.
  }
  for (const plugin of plugins) {
    for (const field of plugin.settings || []) {
      entries.push({
        id: `plugin:${plugin.id}:${field.key}`,
        key: field.key,
        label: field.label,
        hint: field.hint,
        type: field.type,
        options: field.options || [],
        placeholder: field.placeholder,
        tab: "plugins",
        source: plugin.name,
        pluginId: plugin.id,
        category: "settings",
      });
    }
  }
  return entries;
}

// The one repeatable control+description row every settings surface renders (see .setting-row in
// primitives.css). General, Plugins and the search view all call this, never hand-build their
// own field markup. `value` is the field's current string value; onCommit(newValue) fires on
// change/blur, since settings rows commit immediately and there is no "unsaved" state to track.
// Callers must re-run initDropdowns()/initNumericInputs() after inserting rows with select/number
// fields, the same as any other dynamically-inserted primitive markup in this app.
function renderSettingRow(entry, value, onCommit) {
  const row = document.createElement("div");
  row.className = "setting-row";
  row.dataset.settingId = entry.id;

  const controlWrap = document.createElement("div");
  controlWrap.className = "setting-row-control";

  if (entry.type === "checkbox") {
    const rowLabel = document.createElement("label");
    rowLabel.className = "checkbox-row";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = value === "true";
    const box = document.createElement("span");
    box.className = "checkbox-box";
    box.innerHTML = CHECKMARK_SVG;
    const text = document.createElement("span");
    text.textContent = entry.label;
    rowLabel.appendChild(checkbox);
    rowLabel.appendChild(box);
    rowLabel.appendChild(text);
    checkbox.addEventListener("change", () => onCommit(String(checkbox.checked)));
    controlWrap.appendChild(rowLabel);
  } else if (entry.type === "select") {
    const dropdown = document.createElement("div");
    dropdown.className = "dropdown dropdown-full";
    dropdown.dataset.dropdown = "";
    dropdown.dataset.open = "false";
    dropdown.innerHTML =
      '<button type="button" class="dropdown-trigger" data-dropdown-trigger><span class="dropdown-value" data-dropdown-value></span><span class="dropdown-chevron">▾</span></button><div class="dropdown-menu" data-dropdown-menu></div>';
    const menu = dropdown.querySelector("[data-dropdown-menu]");
    (entry.options || []).forEach((opt) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "dropdown-option";
      btn.setAttribute("data-dropdown-option", "");
      btn.dataset.value = opt.value;
      btn.dataset.label = opt.label;
      btn.textContent = opt.label;
      menu.appendChild(btn);
    });
    const valueEl = dropdown.querySelector("[data-dropdown-value]");
    const matched = (entry.options || []).find((o) => o.value === value) || entry.options[0];
    if (matched) {
      valueEl.textContent = matched.label;
      dropdown.dataset.value = matched.value;
      menu.querySelectorAll("[data-dropdown-option]").forEach((o) => o.classList.toggle("is-selected", o.dataset.value === matched.value));
    }
    dropdown.addEventListener("dropdown-change", () => onCommit(dropdown.dataset.value));
    controlWrap.appendChild(dropdown);
  } else if (entry.type === "number") {
    const wrap = document.createElement("div");
    wrap.className = "numeric-input";
    wrap.innerHTML =
      '<input type="text" inputmode="numeric" /><div class="numeric-steppers"><button type="button" class="stepper-up" aria-label="Increase">▲</button><button type="button" class="stepper-down" aria-label="Decrease">▼</button></div>';
    const input = wrap.querySelector("input");
    input.value = value;
    if (entry.placeholder) input.placeholder = entry.placeholder;
    input.addEventListener("change", () => onCommit(input.value.trim()));
    controlWrap.appendChild(wrap);
  } else {
    const input = document.createElement("input");
    input.className = "input";
    input.type = "text";
    input.autocomplete = "off";
    input.value = value;
    if (entry.placeholder) input.placeholder = entry.placeholder;
    input.addEventListener("change", () => onCommit(input.value.trim()));
    controlWrap.appendChild(input);
  }
  row.appendChild(controlWrap);

  const desc = document.createElement("div");
  desc.className = "setting-row-desc";
  const label = document.createElement("div");
  label.className = "setting-row-label";
  label.textContent = entry.label;
  if (entry.source) {
    const source = document.createElement("span");
    source.className = "setting-row-source";
    source.textContent = entry.source;
    label.appendChild(source);
  }
  desc.appendChild(label);
  if (entry.hint) {
    const hint = document.createElement("div");
    hint.className = "setting-row-hint";
    hint.textContent = entry.hint;
    desc.appendChild(hint);
  }
  row.appendChild(desc);

  return row;
}

// Registers a large popup whose body is an <iframe>, for a page substantial enough to stay its own
// unsandboxed document (Settings, Modules and Plugins call real Tauri APIs directly). Call once per
// page that can trigger it, since each page has its own slot registry. The url is per-call, not
// fixed at registration, so settings.html?tab=appearance is a different url on the same "settings"
// id rather than a second popup.
//
// The embedded page's own tabs and toolbar buttons have to render in the popup header, which is a
// different document, so they cannot be appended into ctx.header directly. setPopupHeaderControls()
// and onPopupHeaderAction() below are the page's half of that relay: it posts up what to render,
// this draws it, and clicks post back down. Only where the buttons are drawn moves; the page keeps
// its own logic.
//
// forwardEvents (optional): backend event names this popup's iframe needs while it is open, relayed
// by postMessage rather than the iframe listening for Tauri events itself.

// ---------- Tauri call relay (for an iframe-hosted popup page) ----------
// invoke() called from inside an iframe on WebView2 never resolves OR rejects. It hangs forever
// with no error, because the response callback lands on the parent window rather than the iframe
// that registered it (tauri-apps/tauri#6204). relayableInvoke and relayableOpenDialog are drop-in
// replacements: from a top-level document they call the real thing, and from an iframe they relay
// to window.parent over postMessage. The parent half lives in contributeIframePopup's mount()
// below, scoped to the popup iframes it created and never a plugin iframe.
let nextRelayCallId = 1;
const pendingRelayCalls = new Map();
window.addEventListener("message", (e) => {
  if (e.source !== window.parent || !e.data || e.data.type !== "relay-call-reply") return;
  const pending = pendingRelayCalls.get(e.data.id);
  if (!pending) return;
  pendingRelayCalls.delete(e.data.id);
  if (e.data.ok) pending.resolve(e.data.result);
  else pending.reject(e.data.error);
});
function relayCall(kind, payload) {
  if (window.parent === window) {
    if (kind === "invoke") return window.__TAURI__.core.invoke(payload.command, payload.params);
    if (kind === "dialog-open") return window.__TAURI__.dialog.open(payload);
    if (kind === "theme-changed") return Promise.resolve(reapplyTheme());
  }
  return new Promise((resolve, reject) => {
    const id = nextRelayCallId++;
    pendingRelayCalls.set(id, { resolve, reject });
    window.parent.postMessage({ type: "relay-call", id, kind, payload }, "*");
  });
}
function relayableInvoke(command, params) {
  return relayCall("invoke", { command, params });
}
function relayableOpenDialog(opts) {
  return relayCall("dialog-open", opts);
}

/// Re-resolves and re-applies the theme in THIS document. theme.js sets its CSS custom properties on
/// its own documentElement and nothing else, so a page that changes the theme only restyles
/// itself. The Settings page is an iframe, so it relays this to its parent (see relayCall's
/// "theme-changed" kind) rather than reaching into it directly.
///
/// Guarded because primitives.js is loaded by pages that may not have loaded theme.js; a page with
/// no theme to re-resolve simply has nothing to do here.
function reapplyTheme() {
  if (typeof resolveAndApplyTheme === "function") resolveAndApplyTheme();
}

/// Tells this document AND, when it's an iframe, its parent to re-read the theme. Called after
/// anything that changes which theme is active, but not after a mere colour preview, which is
/// deliberately local to the Settings page until it's saved.
function broadcastThemeChange() {
  reapplyTheme();
  if (window.parent !== window) relayCall("theme-changed").catch(() => {});
}

// Parent-side half, called from contributeIframePopup's own onMessage below, which has ALREADY
// verified e.source === this specific popup's own iframe.contentWindow before this ever runs, so
// there's no separate trust check needed here: a plugin iframe (sandbox="allow-scripts", a
// completely different, deliberately unprivileged trust level) is never the source of a message
// this reaches, only ever one of editor.html's own contributeIframePopup-created popups.
async function handleRelayCall(sourceWindow, data) {
  const { id, kind, payload } = data;
  try {
    let result;
    if (kind === "invoke") result = await window.__TAURI__.core.invoke(payload.command, payload.params);
    else if (kind === "dialog-open") result = await window.__TAURI__.dialog.open(payload);
    // The popup changed which theme is active; this document has to re-read it too, since theme.js
    // only ever styles the document it runs in.
    else if (kind === "theme-changed") result = reapplyTheme();
    else throw new Error(`Unknown relay-call kind "${kind}"`);
    sourceWindow.postMessage({ type: "relay-call-reply", id, ok: true, result }, "*");
  } catch (err) {
    sourceWindow.postMessage({ type: "relay-call-reply", id, ok: false, error: String(err) }, "*");
  }
}

function contributeIframePopup(id, { title, forwardEvents }) {
  contribute("popups", {
    id,
    sourceType: "host",
    title,
    large: true,
    mount(container, ctx) {
      const iframe = document.createElement("iframe");
      iframe.className = "popup-iframe";
      iframe.src = ctx.target.url;
      container.appendChild(iframe);

      // A stable closure, not rebuilt per-message, so onTabClick and onButtonClick stay live across every
      // re-render (including the optimistic one a click itself triggers), so a second click never
      // finds itself talking to handlers a previous render nulled out.
      let latestTabs = [];
      let latestButtons = [];
      let activeTab = null;
      const render = () => {
        renderPopupHeaderExtras(ctx.header, latestTabs, latestButtons, activeTab, {
          onTabClick: (value) => {
            activeTab = value;
            render(); // immediate highlight feedback, before the iframe even reacts
            iframe.contentWindow.postMessage({ type: "popup-tab-change", value }, "*");
          },
          onButtonClick: (id_) => iframe.contentWindow.postMessage({ type: "popup-button-click", id: id_ }, "*"),
        });
      };

      const onMessage = (e) => {
        if (e.source !== iframe.contentWindow || !e.data || typeof e.data !== "object") return;
        if (e.data.type === "relay-call") {
          handleRelayCall(iframe.contentWindow, e.data);
          return;
        }
        if (e.data.type !== "popup-header") return;
        latestTabs = e.data.tabs || [];
        latestButtons = e.data.buttons || [];
        if (typeof e.data.activeTab === "string") activeTab = e.data.activeTab;
        render();
      };
      window.addEventListener("message", onMessage);
      ctx.onClose(() => window.removeEventListener("message", onMessage));

      const unlistenPromises = (forwardEvents || []).map((eventName) =>
        window.__TAURI__.event.listen(eventName, (event) => {
          iframe.contentWindow.postMessage({ type: "tauri-event", event: eventName, payload: event.payload }, "*");
        })
      );
      ctx.onClose(() => {
        unlistenPromises.forEach((p) => p.then((unlisten) => unlisten()));
      });
    },
  });
}

// The actual DOM-building behind contributeIframePopup's relay. A plain function (not tied to
// postMessage) so it's just as usable by same-document popup content later, if anything ever wants
// header tabs/buttons without going through an iframe. tabs: [{value, label}], buttons:
// [{id, label, icon}]. Rebuilds from scratch on every call (cheap, and the only way to guarantee no
// stale click handlers linger from a previous render).
function renderPopupHeaderExtras(header, tabs, buttons, activeTab, { onTabClick, onButtonClick }) {
  header.querySelectorAll(".popup-header-extras").forEach((el) => el.remove());
  if (!tabs.length && !buttons.length) return;

  const extras = document.createElement("div");
  extras.className = "popup-header-extras";

  for (const btn of buttons) {
    const el = document.createElement("button");
    el.type = "button";
    el.className = "btn btn-sm btn-outline";
    el.innerHTML = (btn.icon || "") + (btn.label ? `<span>${btn.label}</span>` : "");
    if (onButtonClick) el.addEventListener("click", () => onButtonClick(btn.id));
    extras.appendChild(el);
  }

  for (const tab of tabs) {
    const el = document.createElement("button");
    el.type = "button";
    el.className = "header-tab" + (tab.value === activeTab ? " is-active" : "");
    el.textContent = tab.label;
    if (onTabClick) el.addEventListener("click", () => onTabClick(tab.value));
    extras.appendChild(el);
  }

  const closeBtn = header.querySelector(".popup-close");
  header.insertBefore(extras, closeBtn);
}

// Embedded-page side of the relay above: tells the parent popup (if this page is actually running
// inside one; a no-op otherwise, so the same call is safe regardless) what to render in ITS header.
// spec: { tabs: [{value, label}], activeTab, buttons: [{id, label, icon}] }.
function setPopupHeaderControls(spec) {
  if (window.parent === window) return;
  window.parent.postMessage({ type: "popup-header", ...spec }, "*");
}

// Embedded-page side. Registers what happens when the parent-rendered header controls (from
// setPopupHeaderControls) are actually clicked. onTabChange(value) / onButtonClick(id).
function onPopupHeaderAction({ onTabChange, onButtonClick } = {}) {
  window.addEventListener("message", (e) => {
    if (e.source !== window.parent || !e.data || typeof e.data !== "object") return;
    if (e.data.type === "popup-tab-change" && onTabChange) onTabChange(e.data.value);
    else if (e.data.type === "popup-button-click" && onButtonClick) onButtonClick(e.data.id);
  });
}

// ---------- Tooltip ----------
// Any element with data-tooltip="..." gets a small delayed popup on hover, positioned to its
// right by default (falls back to the left if there's no room). One shared popup element for the
// whole page rather than one per trigger.

function initTooltips(root = document) {
  let tooltipEl = document.querySelector(".tooltip-popup");
  if (!tooltipEl) {
    tooltipEl = document.createElement("div");
    tooltipEl.className = "tooltip-popup";
    document.body.appendChild(tooltipEl);
  }

  let showTimer = null;

  root.querySelectorAll("[data-tooltip]").forEach((el) => {
    if (el.dataset.tooltipInit) return;
    el.dataset.tooltipInit = "true";

    el.addEventListener("mouseenter", () => {
      clearTimeout(showTimer);
      showTimer = setTimeout(() => {
        tooltipEl.textContent = el.dataset.tooltip;
        tooltipEl.classList.add("is-visible");

        const rect = el.getBoundingClientRect();
        const tipRect = tooltipEl.getBoundingClientRect();
        const fitsRight = rect.right + 8 + tipRect.width <= window.innerWidth;

        tooltipEl.style.top = `${rect.top + rect.height / 2 - tipRect.height / 2}px`;
        tooltipEl.style.left = fitsRight ? `${rect.right + 8}px` : `${rect.left - tipRect.width - 8}px`;
      }, 400);
    });

    el.addEventListener("mouseleave", () => {
      clearTimeout(showTimer);
      tooltipEl.classList.remove("is-visible");
    });

    el.addEventListener("click", () => {
      clearTimeout(showTimer);
      tooltipEl.classList.remove("is-visible");
    });
  });
}

// ---------- Sanitized inline SVG (shared: rail icons in plugin-hosting.js, item icons below) ----------
// Strips anything that could execute if this SVG ends up in a trusted, non-sandboxed document (the
// editor's own rail, or these un-sandboxed Modules/Plugins popup pages — unlike a real plugin's
// `sandbox="allow-scripts"` panel iframe, neither can treat a plugin/module-supplied icon as safe
// by default). innerHTML already never runs an embedded <script> tag, but inline event-handler
// attributes (onclick=, etc.) DO fire: those are the actual thing this strips.
function sanitizeSvg(root) {
  root.querySelectorAll("script").forEach((el) => el.remove());
  root.querySelectorAll("*").forEach((el) => {
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on") || (name === "href" && attr.value.trim().toLowerCase().startsWith("javascript:"))) {
        el.removeAttribute(attr.name);
      }
    }
  });
}

function parseSanitizedSvg(text) {
  const doc = new DOMParser().parseFromString(text, "image/svg+xml");
  if (doc.querySelector("parsererror")) return null;
  const svg = doc.querySelector("svg");
  if (!svg) return null;
  sanitizeSvg(svg);
  return svg;
}

// ---------- Markdown rendering (Overview/Changelog tabs) ----------
// Vendored marked.js (src/vendor/marked/marked.umd.js, MIT; see MARKED_LICENSE) — real CommonMark
// support (tables, nested emphasis, the works) for ~44KB, the same vendor-a-real-library-under
// src/vendor call this repo already makes for three.js/xterm.js/Monaco. Only modules.html/
// plugins.html load the vendor script; everywhere else primitives.js is loaded (editor.html,
// settings.html, ...) this whole block is simply never exercised.
function escapeHtml(text) {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

// Security: modules.html and plugins.html load in an UN-sandboxed popup iframe with real Tauri
// access, and a README is local-but-foreign content that must never inject live markup into that
// document. marked's default output is not safe on its own: raw HTML passes straight through and a
// `javascript:` href is not rejected.
//
// So three renderer hooks are overridden. Raw-HTML tokens are stripped entirely, keeping the
// surrounding text. A link or image href is emitted verbatim only when it has no scheme, meaning
// an ordinary relative path, or an explicit http(s) one; anything else degrades to escaped text.
let markedConfigured = false;
function configureMarkedOnce() {
  if (markedConfigured || typeof marked === "undefined") return;
  markedConfigured = true;

  const safeHref = (href) => {
    const trimmed = (href || "").trim();
    const scheme = trimmed.match(/^([a-z][a-z0-9+.-]*):/i);
    if (!scheme) return trimmed; // no scheme at all — a relative path, always safe
    return /^https?$/i.test(scheme[1]) ? trimmed : null;
  };

  marked.use({
    renderer: {
      html: () => "",
      link(token) {
        const href = safeHref(token.href);
        const text = escapeHtml(token.text);
        return href ? `<a href="${escapeHtml(href)}" target="_blank" rel="noopener noreferrer">${text}</a>` : text;
      },
      image(token) {
        const href = safeHref(token.href);
        return href ? `<img src="${escapeHtml(href)}" alt="${escapeHtml(token.text || "")}" />` : "";
      },
    },
  });
}

function renderMarkdown(text) {
  configureMarkedOnce();
  // The vendor script not being loaded shouldn't happen for createManagerPage, its only real
  // caller — degrade to plain escaped text rather than throw if it somehow isn't.
  if (typeof marked === "undefined") return `<p>${escapeHtml(text)}</p>`;
  return marked.parse(text);
}

// ---------- Item icon (Modules/Plugins detail header) ----------
// A generic placeholder: same graceful-degradation shape as plugin-hosting.js's own
// FALLBACK_RAIL_ICON_SVG — for an item with no declared `icon`, or whose icon fails to load/parse.
const FALLBACK_ITEM_ICON_SVG = '<svg viewBox="0 0 16 16" fill="none"><rect x="2.5" y="2.5" width="11" height="11" rx="2.5" stroke="currentColor" stroke-width="1.3" /></svg>';

// Fetched as raw text over the generic read_install_text_file command (works for both a module and
// a plugin's own folder, unlike read_plugin_asset which is plugin-id-scoped) rather than used as an
// <img src="...">, so it can be inlined as a real <svg> and inherit currentColor the same as the
// fallback already does.
async function loadItemIconSvg(invoke, item) {
  if (!item.icon) return null;
  try {
    const text = await invoke("read_install_text_file", { folder: item.folder, relPath: item.icon });
    return text ? parseSanitizedSvg(text) : null;
  } catch (err) {
    return null;
  }
}

// ---------- Manager page (Modules/Plugins' Installed tab) ----------
// modules.html and plugins.html stay separate pages for separate concepts, but need the same
// two-pane list-and-detail shape over the same four commands. This factory is what they share.
// editor.html's createManagerPanel() does the same job for the narrower in-editor accordions, and
// is deliberately a different function: the two-pane layout needs width the sidebar has not got.
//
// config: { nounSingular, nounPlural, listCommand, enableCommand, removeCommand, installCommand,
// installDialogTitle, addButtonLabel, detailFields(item) -> [{label, value, mono?}] for the
// metadata column, contributionFields(item) -> [{label, value}] for the Contributions tab }.
// Self-initializing: call it once at page load and it wires everything, including the popup-header
// relay and the remove-confirm popup, and loads the list itself.
function createManagerPage(config) {
  const invoke = relayableInvoke;
  const openDialog = relayableOpenDialog;
  const popupId = `remove-${config.nounSingular.toLowerCase()}`;

  let items = [];
  let selectedId = null;

  contribute("popups", {
    id: popupId,
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

  // ---------- List (left) ----------
  function renderList() {
    const list = document.getElementById("item-list");
    list.innerHTML = "";

    if (items.length === 0) {
      const empty = document.createElement("div");
      empty.className = "manage-list-empty";
      empty.textContent = `No ${config.nounPlural} installed.`;
      list.appendChild(empty);
      return;
    }

    for (const item of items) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "sidebar-tab" + (item.id === selectedId ? " is-active" : "");
      btn.dataset.tabValue = item.id;
      if (item.disabled) btn.style.opacity = "0.55";

      const stack = document.createElement("div");
      const name = document.createElement("div");
      name.textContent = item.name;
      const sub = document.createElement("div");
      sub.className = "item-sub";
      const bits = [item.version ? `v${item.version}` : item.id];
      if (item.disabled) bits.push("Disabled");
      sub.textContent = bits.join(" — ");
      stack.appendChild(name);
      stack.appendChild(sub);
      btn.appendChild(stack);

      list.appendChild(btn);
    }
  }

  // ---------- Detail (right) ----------
  function detailField({ label, value, mono }) {
    const field = document.createElement("div");
    field.className = "detail-field";
    const lab = document.createElement("div");
    lab.className = "field-label";
    lab.textContent = label;
    const val = document.createElement("div");
    val.className = "value" + (mono ? " mono" : "");
    val.textContent = value;
    field.appendChild(lab);
    field.appendChild(val);
    return field;
  }

  // ---------- Detail tabs (Overview / Changelog / Contributions) ----------
  const DETAIL_TABS = [
    { value: "overview", label: "Overview" },
    { value: "changelog", label: "Changelog" },
    { value: "contributions", label: "Contributions" },
  ];

  function renderOverviewPanel(panel, item, readmeText) {
    panel.innerHTML = "";
    if (readmeText) {
      panel.innerHTML = renderMarkdown(readmeText);
      return;
    }
    const note = document.createElement("div");
    note.className = "field-hint detail-tab-note";
    note.textContent = `No README.md — here's what this ${config.nounSingular.toLowerCase()} declares:`;
    panel.appendChild(note);
    for (const field of config.detailFields(item)) {
      panel.appendChild(detailField(field));
    }
  }

  function renderChangelogPanel(panel, changelogText) {
    panel.innerHTML = "";
    if (changelogText) {
      panel.innerHTML = renderMarkdown(changelogText);
      return;
    }
    const empty = document.createElement("div");
    empty.className = "field-hint detail-tab-note";
    empty.textContent = "No CHANGELOG.md.";
    panel.appendChild(empty);
  }

  function renderContributionsPanel(panel, item) {
    panel.innerHTML = "";
    const fields = config.contributionFields(item);
    if (fields.length === 0) {
      const empty = document.createElement("div");
      empty.className = "field-hint detail-tab-note";
      empty.textContent = `This ${config.nounSingular.toLowerCase()} doesn't contribute anything else.`;
      panel.appendChild(empty);
      return;
    }
    for (const field of fields) {
      panel.appendChild(detailField(field));
    }
  }

  function renderDetailTabs(item) {
    const wrap = document.createElement("div");
    wrap.className = "detail-tabs-wrap";

    const nav = document.createElement("div");
    nav.className = "detail-tabs";
    nav.setAttribute("data-tabs", "");
    DETAIL_TABS.forEach((tab, i) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "detail-tab" + (i === 0 ? " is-active" : "");
      btn.dataset.tabValue = tab.value;
      btn.textContent = tab.label;
      nav.appendChild(btn);
    });
    wrap.appendChild(nav);

    const panels = {};
    DETAIL_TABS.forEach((tab, i) => {
      const panel = document.createElement("div");
      panel.className = "detail-tab-panel" + (i === 0 ? " is-active" : "");
      panel.id = `detail-tab-${tab.value}`;
      panels[tab.value] = panel;
      wrap.appendChild(panel);
    });

    nav.addEventListener("tab-change", (e) => {
      Object.values(panels).forEach((p) => p.classList.toggle("is-active", p.id === `detail-tab-${e.detail.value}`));
    });

    panels.overview.textContent = "Loading…";
    panels.changelog.textContent = "Loading…";
    renderContributionsPanel(panels.contributions, item);

    invoke("read_install_text_file", { folder: item.folder, relPath: "README.md" })
      .then((text) => renderOverviewPanel(panels.overview, item, text))
      .catch(() => renderOverviewPanel(panels.overview, item, null));
    invoke("read_install_text_file", { folder: item.folder, relPath: "CHANGELOG.md" })
      .then((text) => renderChangelogPanel(panels.changelog, text))
      .catch(() => renderChangelogPanel(panels.changelog, null));

    // License is the one CONDITIONAL tab — unlike Overview/Changelog (which always show, with a
    // fallback, even when their file is missing), a License tab only ever appears at all once its
    // file is confirmed to exist. initTabs()'s own click delegation is bound to `nav` itself
    // (see primitives.js's initTabs), so a tab button appended here after the fact is still fully
    // clickable with no extra wiring; the tab-change listener above re-reads `panels` fresh on
    // every event too, so adding to it late works the same way.
    invoke("read_install_text_file", { folder: item.folder, relPath: "LICENSE.md" })
      .then((text) => {
        if (!text) return;
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "detail-tab";
        btn.dataset.tabValue = "license";
        btn.textContent = "License";
        nav.appendChild(btn);

        const panel = document.createElement("div");
        panel.className = "detail-tab-panel";
        panel.id = "detail-tab-license";
        panel.innerHTML = renderMarkdown(text);
        wrap.appendChild(panel);
        panels.license = panel;
      })
      .catch(() => {});

    return wrap;
  }

  // ---------- Header row: icon | title/website/description/actions | file metadata ----------
  function renderDetail(item) {
    const pane = document.getElementById("detail-pane");
    pane.innerHTML = "";

    if (!item) {
      const empty = document.createElement("div");
      empty.className = "manage-detail-empty";
      empty.textContent = `Select a ${config.nounSingular.toLowerCase()} to see its details.`;
      pane.appendChild(empty);
      return;
    }

    const headerRow = document.createElement("div");
    headerRow.className = "detail-header-row";

    const iconWrap = document.createElement("div");
    iconWrap.className = "detail-icon";
    iconWrap.innerHTML = FALLBACK_ITEM_ICON_SVG;
    headerRow.appendChild(iconWrap);
    loadItemIconSvg(invoke, item).then((svg) => {
      if (!svg) return;
      iconWrap.innerHTML = "";
      iconWrap.appendChild(svg);
    });

    const mainCol = document.createElement("div");
    mainCol.className = "detail-header-main";

    const title = document.createElement("div");
    title.className = "detail-title";
    const titleText = document.createElement("span");
    titleText.textContent = item.name;
    title.appendChild(titleText);
    const idText = document.createElement("span");
    idText.className = "detail-title-id";
    idText.textContent = item.id + (item.version ? ` · v${item.version}` : "");
    title.appendChild(idText);
    mainCol.appendChild(title);

    if (item.website) {
      const websiteRow = document.createElement("div");
      websiteRow.className = "detail-meta-row";
      const link = document.createElement("a");
      link.href = item.website;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      link.textContent = item.website;
      websiteRow.appendChild(link);
      mainCol.appendChild(websiteRow);
    }

    const description = document.createElement("div");
    description.className = "detail-description";
    description.textContent = item.description || "No description.";
    mainCol.appendChild(description);

    const actions = document.createElement("div");
    actions.className = "detail-actions";

    const enabledLabel = document.createElement("label");
    enabledLabel.className = "checkbox-row";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = !item.disabled;
    const box = document.createElement("span");
    box.className = "checkbox-box";
    box.innerHTML = CHECKMARK_SVG;
    const enabledText = document.createElement("span");
    enabledText.textContent = "Enabled";
    enabledLabel.appendChild(checkbox);
    enabledLabel.appendChild(box);
    enabledLabel.appendChild(enabledText);
    checkbox.addEventListener("change", async () => {
      try {
        await invoke(config.enableCommand, { id: item.id, enabled: checkbox.checked });
        await loadItems();
      } catch (err) {
        reportError(err);
      }
    });
    actions.appendChild(enabledLabel);

    const removeBtn = document.createElement("button");
    removeBtn.type = "button";
    removeBtn.className = "btn btn-sm btn-danger btn-icon-only";
    removeBtn.dataset.tooltip = `Remove this ${config.nounSingular.toLowerCase()}`;
    removeBtn.innerHTML = DELETE_SVG;
    removeBtn.addEventListener("click", async () => {
      const confirmed = await showPopup(popupId, { nounSingular: config.nounSingular, name: item.name });
      if (!confirmed) return;
      try {
        await invoke(config.removeCommand, { id: item.id });
        selectedId = null;
        await loadItems();
      } catch (err) {
        reportError(err);
      }
    });
    actions.appendChild(removeBtn);
    mainCol.appendChild(actions);

    headerRow.appendChild(mainCol);

    const metaCol = document.createElement("div");
    metaCol.className = "detail-meta-column";
    for (const field of config.detailFields(item)) {
      metaCol.appendChild(detailField(field));
    }
    headerRow.appendChild(metaCol);

    pane.appendChild(headerRow);
    pane.appendChild(renderDetailTabs(item));

    initTabs(pane);
    initTooltips();
  }

  document.getElementById("item-list").addEventListener("tab-change", (e) => {
    selectedId = e.detail.value;
    renderDetail(items.find((i) => i.id === selectedId));
  });

  async function loadItems() {
    try {
      items = await invoke(config.listCommand);
    } catch (err) {
      reportError(err);
      items = [];
    }
    if (!items.some((i) => i.id === selectedId)) selectedId = null;
    renderList();
    renderDetail(items.find((i) => i.id === selectedId));
  }

  // ---------- Add ----------
  // Progress lives in the page body, not next to the "Add …" button: that button is actually
  // rendered by the POPUP's own header (a different document, see contributeIframePopup/
  // setPopupHeaderControls below), so a bar can't sit beside it without extending that relay
  // protocol just for this. Still worth having: a folder like Monaco's vendored ~24MB is genuinely
  // not instant to copy.
  const progressEl = document.createElement("div");
  progressEl.className = "progress";
  progressEl.style.margin = "0 0 12px";
  progressEl.style.display = "none";
  progressEl.innerHTML = '<div class="progress-fill"></div>';
  document.querySelector(".manage-list").prepend(progressEl);

  let installing = false;

  // Relayed by contributeIframePopup's forwardEvents (see primitives.js's own contributeIframePopup)
  // — this page is loaded inside an iframe, so it can't reliably listen for the raw Tauri event
  // itself (see that function's header comment for why).
  window.addEventListener("message", (e) => {
    if (!e.data || e.data.type !== "tauri-event" || e.data.event !== "install-progress") return;
    const payload = e.data.payload;
    if (payload.kind !== config.nounSingular.toLowerCase()) return;
    setProgress(progressEl, payload.totalBytes ? payload.bytesDone / payload.totalBytes : 0);
  });

  async function addItem() {
    if (installing) return;
    const sourceDir = await openDialog({ directory: true, title: config.installDialogTitle });
    if (!sourceDir) return;

    installing = true;
    progressEl.style.display = "";
    setProgress(progressEl, 0);

    try {
      const id = await invoke(config.installCommand, { sourceDir });
      showToast({ variant: "success", message: `Installed "${id}".` });
      selectedId = id;
      await loadItems();
    } catch (err) {
      reportError(err);
    } finally {
      installing = false;
      progressEl.style.display = "none";
    }
  }

  // ---------- Popup header (Installed/Marketplace tabs + Add …, rendered by the popup shell
  // itself; see setPopupHeaderControls()/onPopupHeaderAction() above) ----------
  function showOuterTab(value) {
    document.querySelectorAll(".tab-panel").forEach((p) => p.classList.toggle("is-active", p.id === `tab-panel-${value}`));
  }

  // Lets a caller deep-link straight to the Marketplace tab — e.g. the sidebar manager panel's own
  // "Marketplace" button (see plugin-hosting.js) opens this popup with `?tab=marketplace` rather
  // than always landing on Installed. Same pattern settings.html already uses for its own tabs.
  const requestedTab = new URLSearchParams(location.search).get("tab") === "marketplace" ? "marketplace" : "installed";
  if (requestedTab !== "installed") showOuterTab(requestedTab);

  setPopupHeaderControls({
    tabs: [
      { value: "installed", label: "Installed" },
      { value: "marketplace", label: "Marketplace" },
    ],
    activeTab: requestedTab,
    buttons: [
      {
        id: "add-item",
        label: config.addButtonLabel,
        icon: '<svg viewBox="0 0 16 16" fill="none"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>',
      },
    ],
  });
  onPopupHeaderAction({
    onTabChange: showOuterTab,
    onButtonClick: (id) => {
      if (id === "add-item") addItem();
    },
  });

  initTooltips();
  initTabs();
  loadItems();
}

// Wires the three caption buttons every page's custom titlebar needs since the window runs with
// decorations:false (that's a whole-window setting, not per-page, so every page draws its own).
// Expects #win-minimize / #win-maximize / #win-close to already be in the DOM.

const MAXIMIZE_ICON = '<svg viewBox="0 0 16 16" fill="none"><rect x="3.5" y="3.5" width="9" height="9" rx="0.5" stroke="currentColor" stroke-width="1.3" /></svg>';
const RESTORE_ICON = '<svg viewBox="0 0 16 16" fill="none"><rect x="5.5" y="2.5" width="8" height="8" rx="0.5" stroke="currentColor" stroke-width="1.3" /><path d="M3 5.5v7a1 1 0 001 1h7" stroke="currentColor" stroke-width="1.3" /></svg>';

// beforeClose (optional): an async () => boolean, awaited before the window actually closes —
// false cancels it. Only the editor passes one (unsaved files / an open Draft with real changes);
// every other page (settings, startup, the Modules/Plugins popups) has nothing of the sort to
// lose, so they call this exactly as before, no behavior change.
async function initWindowControls(beforeClose) {
  const tauriWindow = window.__TAURI__ && window.__TAURI__.window;
  if (!tauriWindow) return;

  const minBtn = document.getElementById("win-minimize");
  const maxBtn = document.getElementById("win-maximize");
  const closeBtn = document.getElementById("win-close");
  if (!minBtn || !maxBtn || !closeBtn) return;

  const appWindow = tauriWindow.getCurrentWindow();

  async function syncMaximizeButton() {
    const maximized = await appWindow.isMaximized();
    maxBtn.innerHTML = maximized ? RESTORE_ICON : MAXIMIZE_ICON;
    const label = maximized ? "Restore" : "Maximize";
    maxBtn.dataset.tooltip = label;
    maxBtn.setAttribute("aria-label", label);
  }

  // appWindow.close() (what our own titlebar button below calls) does NOT reliably fire
  // onCloseRequested in Tauri v2 — confirmed (tauri-apps/tauri#5288), and this app runs with
  // decorations:false, so that JS call IS the only "close" path our own X button has; gating it
  // right here, before ever calling .close(), is what actually protects it. onCloseRequested
  // below is still worth wiring too. It DOES correctly fire for Alt+F4/a taskbar "close window",
  // the paths that don't go through our own button at all.
  async function requestClose() {
    if (beforeClose && !(await beforeClose())) return;
    appWindow.close();
  }

  minBtn.addEventListener("click", () => appWindow.minimize());
  maxBtn.addEventListener("click", () => appWindow.toggleMaximize());
  closeBtn.addEventListener("click", requestClose);
  if (beforeClose) {
    appWindow.onCloseRequested(async (event) => {
      if (!(await beforeClose())) event.preventDefault();
    });
  }

  await syncMaximizeButton();
  appWindow.onResized(() => syncMaximizeButton());
}
