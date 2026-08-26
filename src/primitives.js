// Behavior for the dropdown / checkbox-dropdown / searchbar / numeric-input / toast / popup
// primitives. Dropdowns, checkbox-dropdowns, and popups are driven entirely by data attributes,
// so any page can drop in the markup from primitives.css with no per-instance wiring. A searchbar
// needs a real data source to filter against, so it's exposed as a function (initSearchbar)
// instead of auto-init, and a toast has no fixed markup to init — it's created on demand.

function closeAllDropdowns(except) {
  document.querySelectorAll('[data-dropdown][data-open="true"]').forEach((el) => {
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

document.addEventListener("click", (e) => {
  if (!e.target.closest("[data-dropdown]")) closeAllDropdowns();
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

// One selection behavior for both sidebar-tab and header-tab lists — they're the same "exactly
// one active item" logic under different skins, driven by [data-tabs] wrapping [data-tab-value]
// buttons.
function initTabs(root = document) {
  root.querySelectorAll("[data-tabs]").forEach((el) => {
    if (el.dataset.tabsInit) return;
    el.dataset.tabsInit = "true";

    el.addEventListener("click", (e) => {
      const tab = e.target.closest("[data-tab-value]");
      if (!tab || !el.contains(tab)) return;

      el.querySelectorAll("[data-tab-value]").forEach((t) => t.classList.remove("is-active"));
      tab.classList.add("is-active");
      el.dispatchEvent(new CustomEvent("tab-change", { bubbles: true, detail: { value: tab.dataset.tabValue } }));
    });
  });
}

// Drag-to-reorder for a tab/rail strip — opt-in per container via the data-reorderable attribute
// (checked here, not left to callers to remember), not wired onto every [data-tabs] container
// automatically: most tab strips in this app (the top menu bar, a standalone page's view tabs)
// have no business being reorderable, and the caller shouldn't have to think about that each time
// — the container either declares data-reorderable in its own markup or this is a silent no-op.
// Pointer Events, not native HTML5 drag-and-drop — same reasoning as every other drag interaction
// in this app (see editor.html's initDividerDrag): setPointerCapture keeps tracking the pointer
// reliably regardless of what's visually underneath it, and native DnD's dataTransfer/ghost-image
// machinery is unneeded complexity for an in-page reorder with no drop target outside the page.
// itemSelector picks which children count as draggable items (default [data-tab-value], override
// for a container using a different identity attribute); keyAttr names the matching dataset
// property read for that item's identity when reporting the final order. axis is "x" for a
// horizontal strip, "y" for a vertical one (the rail). onReorder(orderedKeys) fires once, after a
// completed drag that ends back in this same container, with the container's full new order — not
// on every intermediate move, so a caller doing something non-trivial with it (like persisting to
// disk) isn't doing that on every pixel of pointer movement.
//
// crossContainer + onMoveAcross(key, targetContainer, beforeKey) are optional — pass a SECOND
// reorderable container an item from this one can be dropped into (the editor's two split groups,
// each passing the other as its crossContainer — see editor.html). crossZone is the (typically
// larger) element that actually counts as "hovering over the other side" — e.g. the whole editor
// group/pane rather than just its thin tab strip, a far easier drop target — while the indicator
// itself still only ever renders inside crossContainer (it has to; that's what has the sibling
// tab elements to position relative to). Defaults to crossContainer when omitted.
//
// Pointer Capture keeps clientX/clientY reporting the real screen position throughout the drag
// regardless of which element the events are captured to, which is what makes detecting "the
// pointer is now over the OTHER side" possible at all without a second listener tree. A
// collapsed/hidden crossZone (e.g. the split isn't open) has offsetParent === null, checked
// explicitly below rather than relying on a hidden element's rect happening to sit at (0,0). On
// drop, if the pointer ended up over the other side, the actual DOM node is deliberately left
// alone (only the indicator moves during the drag) — onMoveAcross is expected to update whatever
// real state governs group membership and re-render both sides from scratch, which would discard
// any manual DOM surgery done here anyway; onReorder does NOT fire for a cross-container drop.
function initReorderable(container, { itemSelector = "[data-tab-value]", keyAttr = "tabValue", axis = "x", onReorder, crossContainer, crossZone, onMoveAcross } = {}) {
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
  // the (already-detached-looking, via .is-dragging) indicator there — before the first item whose
  // midpoint the pointer has passed, or right after the last item if the pointer is past all of
  // them (never past the container's own trailing non-item children, e.g. the console's spacer/
  // new-terminal controls — appendChild-ing straight onto the container would land the indicator
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

    const onUp = (upEvent) => {
      container.removeEventListener("pointermove", onMove);
      container.removeEventListener("pointerup", onUp);
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

    container.addEventListener("pointermove", onMove);
    container.addEventListener("pointerup", onUp);
  });

  // A genuine drag still ends in a real click event on release (pointerup with no movement since
  // the last frame doesn't prevent the browser's own click synthesis) — capture phase, same
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

// Wires a plain text input to filter `options` ({value, label}[]) into the shared dropdown-menu
// popover. The popover only ever appears when there's something to suggest — an empty query or a
// query with no matches keeps it closed, per "only show a dropdown when the searchbar can show
// options."
function initSearchbar(el, { options, onSelect } = {}) {
  const input = el.querySelector("input");
  const menu = el.querySelector("[data-dropdown-menu]");
  const clearBtn = el.querySelector(".searchbar-clear");

  const render = (matches) => {
    menu.innerHTML = "";
    matches.forEach((opt) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "dropdown-option";
      button.dataset.value = opt.value;
      button.textContent = opt.label;
      menu.appendChild(button);
    });
  };

  input.addEventListener("input", () => {
    const query = input.value.trim().toLowerCase();
    const matches = query ? options.filter((o) => o.label.toLowerCase().includes(query)) : [];
    render(matches);
    el.classList.toggle("has-value", input.value.length > 0);
    el.dataset.open = matches.length > 0 ? "true" : "false";
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
    el.dataset.open = "false";
    el.classList.remove("has-value");
    input.focus();
  });

  document.addEventListener("click", (e) => {
    if (!e.target.closest(".searchbar")) el.dataset.open = "false";
  });
}

// ---------- Base system: slot registry ----------
// One shared content registry every "Base" region (sidebar, inspector, console, a center-panel
// group, popups) reads from — replacing the copy of "plugin vs host content, open/close, active
// tab" logic that today lives separately in HOST_SIDEBAR_PANELS/pluginPanels/addSidebarRailIcon/
// addConsoleTab/claimSingleSlot/openFiles (editor.html). Not a class hierarchy — one Map plus plain
// functions, same style as everything else in this file. Nothing calls contribute()/getSlot() yet;
// each region migrates onto this one at a time (see the Base-system plan).
const slotRegistry = new Map(); // slot id -> Contribution[]
const KNOWN_SLOTS = new Set(["sidebar", "inspector", "console", "center-0", "center-1", "popups"]);

// contribution: { id, sourceType: "host" | "plugin", pluginId, label, icon, order, closeable,
// when, mount }. id must be unique within this slot; mount(container) is called lazily, the first
// time this contribution is actually shown (mirrors editor.html's mountPanelIframe "create once"
// caching) — never eagerly, so a `when`-gated contribution that isn't currently shown never runs
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
// user's own drag-order on top of this as a final override — same two-step shape
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
// One system, not two — every toast IS a notification (Nolan: "The toast and notif system need to
// be very integrated together. They are the same system."). showToast() below pushes into this
// history unconditionally; a page with somewhere to show that history (editor.html's bell, so far
// — see openNotificationPanel there) reads it back via getNotificationHistory()/onNotification(),
// a page with nowhere to show it (index.html, settings.html, ...) just never looks, at the cost of
// one array push per toast either way. Session-only — an in-memory array, not persisted — toasts
// are inherently ephemeral status messages, so "what did I miss" only ever means "since this page
// loaded," never forever. Capped so a very long session can't grow this unboundedly.
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
// window.lowarc.notify() in plugin_protocol.rs) — recorded in the notification history but not
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

// ---------- Popup ----------

function openPopup(idOrEl) {
  const el = typeof idOrEl === "string" ? document.getElementById(idOrEl) : idOrEl;
  if (!el) return;
  el.classList.add("is-open");
  const focusable = el.querySelector("input, textarea, select, button, [tabindex]");
  if (focusable) focusable.focus();
}

function closePopup(idOrEl) {
  const el = typeof idOrEl === "string" ? document.getElementById(idOrEl) : idOrEl;
  if (!el) return;
  el.classList.remove("is-open");
}

// Wires every [data-popup] backdrop found under root: clicking the backdrop itself (not its
// contents) closes it, and any [data-popup-close] inside (a header's X, a Cancel button) does too.
function initPopups(root = document) {
  root.querySelectorAll("[data-popup]").forEach((el) => {
    if (el.dataset.popupInit) return;
    el.dataset.popupInit = "true";

    el.addEventListener("click", (e) => {
      if (e.target === el) closePopup(el);
    });

    el.querySelectorAll("[data-popup-close]").forEach((btn) => {
      btn.addEventListener("click", () => closePopup(el));
    });
  });
}

document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  const open = document.querySelector(".popup-backdrop.is-open");
  if (open) closePopup(open);
});

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

// Wires the three caption buttons every page's custom titlebar needs since the window runs with
// decorations:false (that's a whole-window setting, not per-page, so every page draws its own).
// Expects #win-minimize / #win-maximize / #win-close to already be in the DOM.

const MAXIMIZE_ICON = '<svg viewBox="0 0 16 16" fill="none"><rect x="3.5" y="3.5" width="9" height="9" rx="0.5" stroke="currentColor" stroke-width="1.3" /></svg>';
const RESTORE_ICON = '<svg viewBox="0 0 16 16" fill="none"><rect x="5.5" y="2.5" width="8" height="8" rx="0.5" stroke="currentColor" stroke-width="1.3" /><path d="M3 5.5v7a1 1 0 001 1h7" stroke="currentColor" stroke-width="1.3" /></svg>';

async function initWindowControls() {
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

  minBtn.addEventListener("click", () => appWindow.minimize());
  maxBtn.addEventListener("click", () => appWindow.toggleMaximize());
  closeBtn.addEventListener("click", () => appWindow.close());

  await syncMaximizeButton();
  appWindow.onResized(() => syncMaximizeButton());
}
