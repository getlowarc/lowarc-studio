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

// ---------- Toast ----------

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
// require a manual close. Returns a dismiss() function so the caller can close it early (e.g. once
// a longer operation the toast was reporting on has moved past what it said).
function showToast({ variant = "info", message, duration = 4000 } = {}) {
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
