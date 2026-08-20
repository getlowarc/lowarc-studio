// Behavior for the dropdown / checkbox-dropdown / searchbar / numeric-input primitives.
// Dropdowns and checkbox-dropdowns are driven entirely by data attributes, so any page can drop
// in the markup from primitives.css with no per-instance wiring. A searchbar needs a real data
// source to filter against, so it's exposed as a function (initSearchbar) instead of auto-init.

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
