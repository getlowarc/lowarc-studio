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

// No system right-click menu anywhere the app doesn't build its own — a row/element with a custom
// context menu (file-explorer's tree rows, via window.lowarc.showMenu()) calls preventDefault()
// itself before this ever runs, so that path is unaffected; this only removes the default for
// everything else. Plugin iframes get the equivalent listener from HARNESS_JS (plugin_assets.rs)
// since this document-level one can't reach into a separate iframe document.
document.addEventListener("contextmenu", (e) => e.preventDefault());

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
// window.lowarc.notify() in plugin_assets.rs) — recorded in the notification history but not
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

// ---------- Popup (Base) ----------
// A "popups" slot in the shared registry above, but unlike a persistent region
// (sidebar/inspector/console in editor.html) it's a STACK, not a single mounted slot: showing one
// doesn't replace another, a popup can open a second one on top of it, and nothing about it
// persists — it exists only while shown. contribute("popups", {...}) just registers WHAT a popup
// is; showPopup(id, target) is what actually opens an instance, fresh, every call — never cached
// the way a region's mounted content is, since a popup has no reason to stay in the DOM once
// closed and a nested open of the same id needs its own independent instance.
//
// Deliberately reuses .popup-backdrop/.popup/.popup-header/.popup-body/.popup-actions as-is, but
// WITHOUT ever adding the "is-open" class openPopup()/closePopup() above toggle — this file also
// has a document-level Escape listener keyed on ".popup-backdrop.is-open" (for the simple,
// element-toggle popup system above, still used as-is by modules.html/plugins.html/settings.html's
// own popups), and this stack has its own Escape handling below; adding "is-open" here would make
// that other listener match these instances too and fight over closing them. Visibility here is a
// plain inline style.display instead.
//
// Any page that wants this needs a `<div class="popup-stack" id="popup-stack"></div>` in its own
// markup (a sibling of the page's main content, same "nothing can clip it" placement reasoning as
// editor.html's #floating-menu) — see editor.html for the original, and settings.html/modules.html/
// plugins.html for pages that adopted it afterward.
const popupStack = [];
const POPUP_Z_FLOOR = 200; // above floating-menu (60) / toast-stack (100), below tooltip (300)

function topPopup() {
  return popupStack.length ? popupStack[popupStack.length - 1] : null;
}

// contribution: { id, sourceType: "host" | "plugin", pluginId, title (string, or
// (target) => string for a title that depends on what showPopup() was called with), size (px
// width — omit for .popup's own CSS default), large (bool — a near-fullscreen popup instead of a
// small dialog, see .popup-large in primitives.css; for a page substantial enough to stay its own
// separate document rather than a small confirm/form, see contributeIframePopup below),
// closeOnBackdrop (default true), closeOnEscape (default true), mount(container, ctx) }. mount
// receives the .popup element itself — already containing the header/title/X, the Base's own
// chrome — and appends whatever .popup-body/.popup-actions markup it needs, the same split every
// popup already used before this existed, just no longer hand-copied per instance.
// ctx = { close(result), target, header, onClose(fn) }. header is the .popup-header element
// itself — mount() can append extra controls into it (tabs, buttons) alongside the title/X; see
// contributeIframePopup's use of it to relay header content posted up from an embedded iframe, and
// primitives.css's .popup-header-extras for how that content is expected to lay out. onClose
// registers cleanup that runs exactly once, whenever this popup instance actually closes —
// REGARDLESS of what triggered it (the X, backdrop click, Escape, or mount()'s own close(result)
// call) — for content that set up something needing teardown (see contributeIframePopup's message
// listener).
//
// A plugin's own content is already safe if its mount() (really just its declared existence — no
// plugin contributes a popup yet) misbehaves, since nothing here calls INTO a plugin's iframe
// directly; a HOST contribution's mount() runs in this same script, though, so it's wrapped in
// try/catch below — the same reasoning already applied to a host sidebar panel's mount() in
// editor.html.
function showPopup(id, target) {
  const contribution = getSlot("popups").find((c) => c.id === id);
  if (!contribution) return Promise.reject(new Error(`showPopup(): no popup contributed with id "${id}"`));

  return new Promise((resolve) => {
    const depth = popupStack.length;
    const backdrop = document.createElement("div");
    backdrop.className = "popup-backdrop";
    backdrop.style.display = "flex";
    backdrop.style.zIndex = String(POPUP_Z_FLOOR + depth * 10);

    const box = document.createElement("div");
    box.className = "popup" + (contribution.large ? " popup-large" : "");
    if (contribution.size) box.style.width = `${contribution.size}px`;

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
      resolve(result === undefined ? null : result);
    };
    instance.close = close;

    backdrop.addEventListener("click", (e) => {
      if (e.target === backdrop && instance.closeOnBackdrop) close(null);
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

    const focusable = box.querySelector("input, textarea, select, button, [tabindex]");
    if (focusable) focusable.focus();
  });
}

// Only the TOP popup reacts to Escape — closeOnBackdrop/closeOnEscape are read independently per
// contribution rather than coupled, so a dialog that disables backdrop-click doesn't also lose its
// only other way out.
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  const top = topPopup();
  if (top && top.closeOnEscape) top.close(null);
});

// For a page substantial enough to stay its own separate, unsandboxed document (Settings/Modules/
// Plugins use real Tauri APIs directly, unlike a plugin's own sandboxed panel) rather than being
// folded inline — registers a large popup (see the `large` contribution flag above) whose body is
// just an <iframe>. Call once per page that can trigger it (each page has its own separate
// contribute()/getSlot() registry — see the slot-registry section above — so a page needs its own
// registration even though the logic lives here, shared). showPopup(id, { url }) is what actually
// opens it — url is per-call, not fixed at registration time, so e.g. settings.html's own
// ?tab=appearance variant is just a different url on the same "settings" id, not a second popup.
//
// These pages exist ONLY as popup content now (Nolan: "those pages will only exist as popups, so
// they need to fit the popup, not be exceptions to the standard") — their own titlebar/page-title/
// close button were removed from their markup entirely, not conditionally hidden at runtime. The
// popup shell's own header/X (built by showPopup) is the only chrome; closing is entirely its job.
//
// A page's own tabs/toolbar buttons (Modules/Plugins' Installed-vs-Marketplace + "Add …") still
// need to render SOMEWHERE, though, and the popup's header is the one place left for them — but
// they're built by the embedded page's own script, in a different document, so they can't just be
// appended into ctx.header directly the way a same-document mount() could. setPopupHeaderControls()/
// onPopupHeaderAction() (below) are the embedded page's own half of this relay: it posts up what to
// render, this renders real controls into ctx.header, and posts clicks back down for the page's own
// existing handlers to react to — the page's tab/button LOGIC never moves out of its own script,
// only where the buttons themselves are drawn.
function contributeIframePopup(id, { title }) {
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

      // A stable closure, not rebuilt per-message — onTabClick/onButtonClick stay live across every
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
        if (e.data.type !== "popup-header") return;
        latestTabs = e.data.tabs || [];
        latestButtons = e.data.buttons || [];
        if (typeof e.data.activeTab === "string") activeTab = e.data.activeTab;
        render();
      };
      window.addEventListener("message", onMessage);
      ctx.onClose(() => window.removeEventListener("message", onMessage));
    },
  });
}

// The actual DOM-building behind contributeIframePopup's relay — a plain function (not tied to
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

// Embedded-page side of the relay above — tells the parent popup (if this page is actually running
// inside one; a no-op otherwise, so the same call is safe regardless) what to render in ITS header.
// spec: { tabs: [{value, label}], activeTab, buttons: [{id, label, icon}] }.
function setPopupHeaderControls(spec) {
  if (window.parent === window) return;
  window.parent.postMessage({ type: "popup-header", ...spec }, "*");
}

// Embedded-page side — registers what happens when the parent-rendered header controls (from
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

// ---------- Manager page (Modules/Plugins' Installed tab) ----------
// modules.html and plugins.html are two deliberately SEPARATE pages/popups — different concepts
// (game-runtime deps vs. sandboxed editor plugins, see installs.rs), never merged into one — that
// happen to need the exact same two-pane list+detail UI shape (.manage-list/.manage-detail,
// primitives.css) wrapping the same four-command shape (list/enable/remove/install). This factory
// is what's actually shared: each page calls it once with its own config and nothing else, instead
// of hand-rolling ~180 near-identical lines apiece. Mirrors how editor.html's own
// createManagerPanel() does the same thing for its narrower accordion-style in-editor panels — a
// different visual shape (this needs the two-pane width neither the sidebar nor a popup's header
// has room for), same underlying idea, so deliberately not the same function.
//
// config: { nounSingular, nounPlural, listCommand, enableCommand, removeCommand, installCommand,
// installDialogTitle, addButtonLabel, showStatusDot (bool — plugins shows an enabled/disabled dot
// next to the detail title, modules doesn't), detailFields(item) -> [{label, value, mono?}] }. Self-
// initializing — call it once at page load; it wires everything (including the popup-header relay
// and the shared remove-confirm popup) and loads the list itself, nothing else needs to run after.
function createManagerPage(config) {
  const { invoke } = window.__TAURI__.core;
  const { open: openDialog } = window.__TAURI__.dialog;
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

    const title = document.createElement("div");
    title.className = "detail-title";
    if (config.showStatusDot) {
      const dot = document.createElement("span");
      dot.className = "status-dot" + (item.disabled ? "" : " is-enabled");
      dot.dataset.tooltip = item.disabled ? "Disabled" : "Enabled";
      title.appendChild(dot);
    }
    const titleText = document.createElement("span");
    titleText.textContent = item.name;
    title.appendChild(titleText);
    pane.appendChild(title);

    const sub = document.createElement("div");
    sub.className = "detail-sub";
    sub.textContent = item.id + (item.version ? ` · v${item.version}` : "");
    pane.appendChild(sub);

    for (const field of config.detailFields(item)) {
      pane.appendChild(detailField(field));
    }

    const actions = document.createElement("div");
    actions.className = "detail-actions";

    const enabledLabel = document.createElement("label");
    enabledLabel.className = "checkbox-row";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = !item.disabled;
    const box = document.createElement("span");
    box.className = "checkbox-box";
    box.innerHTML = '<svg viewBox="0 0 16 16" fill="none"><path d="M3 8l3.5 3.5L13 5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>';
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
    removeBtn.className = "btn btn-sm btn-danger";
    removeBtn.textContent = "Remove";
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

    pane.appendChild(actions);
    if (config.showStatusDot) initTooltips(); // the detail title's own status-dot needs one too
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
  async function addItem() {
    const sourceDir = await openDialog({ directory: true, title: config.installDialogTitle });
    if (!sourceDir) return;

    try {
      const id = await invoke(config.installCommand, { sourceDir });
      showToast({ variant: "success", message: `Installed "${id}".` });
      selectedId = id;
      await loadItems();
    } catch (err) {
      reportError(err);
    }
  }

  // ---------- Popup header (Installed/Marketplace tabs + Add …, rendered by the popup shell
  // itself — see setPopupHeaderControls()/onPopupHeaderAction() above) ----------
  setPopupHeaderControls({
    tabs: [
      { value: "installed", label: "Installed" },
      { value: "marketplace", label: "Marketplace" },
    ],
    activeTab: "installed",
    buttons: [
      {
        id: "add-item",
        label: config.addButtonLabel,
        icon: '<svg viewBox="0 0 16 16" fill="none"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>',
      },
    ],
  });
  onPopupHeaderAction({
    onTabChange: (value) => {
      document.querySelectorAll(".tab-panel").forEach((p) => p.classList.toggle("is-active", p.id === `tab-panel-${value}`));
    },
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
