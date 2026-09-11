// A small, single-select dropdown matching the IDE's own look (.dropdown/.dropdown-trigger/
// .dropdown-menu/.dropdown-option; see primitives-shared.css) but with its own standalone
// behavior rather than reusing the host's primitives.js: the host's own initDropdowns() also
// handles multiselect, grouped/searchable menus, and other host-chrome-specific concerns a plugin
// has no reason to carry just to get a plain single-select dropdown. Two separate, smaller
// implementations sharing one look, not one implementation trying to serve both.
//
// createLowarcDropdown(options, value, onChange) builds and returns the whole element: options
// is [{value, label}], value is the currently-selected one's value, onChange(newValue) fires
// whenever a different option is picked (never called for re-picking the same one).
function createLowarcDropdown(options, value, onChange) {
  const root = document.createElement("div");
  root.className = "dropdown";
  root.dataset.open = "false";

  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "dropdown-trigger";
  const valueEl = document.createElement("span");
  valueEl.className = "dropdown-value";
  const chevron = document.createElement("span");
  chevron.className = "dropdown-chevron";
  chevron.innerHTML = "<svg viewBox=\"0 0 16 16\" width=\"10\" height=\"10\" fill=\"none\"><path d=\"M4 6.5l4 4 4-4\" stroke=\"currentColor\" stroke-width=\"1.7\" stroke-linecap=\"round\" stroke-linejoin=\"round\" /></svg>";
  trigger.appendChild(valueEl);
  trigger.appendChild(chevron);

  const menu = document.createElement("div");
  menu.className = "dropdown-menu";

  function labelFor(v) {
    const opt = options.find((o) => o.value === v);
    return opt ? opt.label : String(v);
  }

  function renderOptions() {
    menu.innerHTML = "";
    for (const opt of options) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "dropdown-option" + (opt.value === value ? " is-selected" : "");
      btn.textContent = opt.label;
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        root.dataset.open = "false";
        if (opt.value === value) return;
        value = opt.value;
        valueEl.textContent = labelFor(value);
        renderOptions();
        onChange(value);
      });
      menu.appendChild(btn);
    }
  }

  valueEl.textContent = labelFor(value);
  renderOptions();

  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    root.dataset.open = root.dataset.open === "true" ? "false" : "true";
  });
  // Closing on any outside click, same convention the host's own dropdowns use: menu's own
  // click already stops propagation above so picking an option doesn't immediately reopen/close
  // through this same listener.
  document.addEventListener("click", () => {
    root.dataset.open = "false";
  });

  root.appendChild(trigger);
  root.appendChild(menu);
  return root;
}
