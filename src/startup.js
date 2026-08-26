// Startup screen — new/open project, recents, and links out to the other top-level pages.
// Talks to the Rust side purely through the commands registered in lib.rs. Load order matters:
// primitives.js must load before this file since it defines showPopup/contributeIframePopup/
// showToast/reportError, all used below.

const { invoke } = window.__TAURI__.core;
const { open: openDialog } = window.__TAURI__.dialog;
const { openUrl } = window.__TAURI__.opener;

const pinnedListEl = document.getElementById("pinned-list");
const recentListEl = document.getElementById("recent-list");

function openEditor(projectPath) {
  window.location.href = `editor.html?project=${encodeURIComponent(projectPath)}`;
}

// Icons for the row actions (sized by the .btn-icon-only.btn-sm primitive, not by the SVG itself —
// matching every other icon button in the app). PIN_ICON does double duty as both the "Pin"
// ⋮-menu item's icon-less label and the standalone Unpin button — a pin glyph reads fine for
// "unpin" too given the row it's sitting on.
const PIN_ICON = '<svg viewBox="0 0 16 16" fill="none"><circle cx="8" cy="5.5" r="3" stroke="currentColor" stroke-width="1.3" /><path d="M8 8.5v5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" /></svg>';
const TRASH_ICON = '<svg viewBox="0 0 16 16" fill="none"><path d="M3 4.5h10M6.5 4.5V3a1 1 0 011-1h1a1 1 0 011 1v1.5M6 7v4M10 7v4M4 4.5l.6 8a1 1 0 001 .9h4.8a1 1 0 001-.9l.6-8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" /></svg>';
const MORE_ICON = '<svg viewBox="0 0 16 16" fill="none"><circle cx="8" cy="3.5" r="1.3" fill="currentColor" /><circle cx="8" cy="8" r="1.3" fill="currentColor" /><circle cx="8" cy="12.5" r="1.3" fill="currentColor" /></svg>';

// Any open ⋮ menu closes on an outside click — same mechanic as editor.html's menu bar, just not
// tracked by a fixed id list since there's one per row and rows get rebuilt on every reload.
document.addEventListener("click", () => {
  document.querySelectorAll(".menu-dropdown.is-open").forEach((d) => d.classList.remove("is-open"));
});

function buildRecentItem(project, isPinned) {
  const item = document.createElement("div");
  item.className = "recent-item" + (project.exists ? "" : " missing");

  const main = document.createElement("div");
  main.className = "recent-item-main";
  const name = document.createElement("div");
  name.className = "name";
  name.textContent = project.name;
  const path = document.createElement("div");
  path.className = "path";
  path.textContent = project.path;
  main.appendChild(name);
  main.appendChild(path);
  item.appendChild(main);

  if (project.exists) {
    item.addEventListener("click", async () => {
      try {
        await invoke("open_project", { path: project.path });
        openEditor(project.path);
      } catch (err) {
        reportError(err);
      }
    });
  }

  const actions = document.createElement("div");
  actions.className = "recent-item-actions";

  // Primary action: Delete on a Recents row, Unpin on a Pinned row — never both, and never
  // folded into the ⋮ menu (a standalone button here, not a menu item, on purpose).
  const primaryBtn = document.createElement("button");
  primaryBtn.type = "button";
  primaryBtn.className = "btn btn-icon-only btn-sm btn-ghost";
  primaryBtn.setAttribute("aria-label", isPinned ? "Unpin" : "Delete");
  primaryBtn.dataset.tooltip = isPinned ? "Unpin" : "Delete";
  primaryBtn.innerHTML = isPinned ? PIN_ICON : TRASH_ICON;
  primaryBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      if (isPinned) {
        await invoke("set_recent_pinned", { path: project.path, pinned: false });
      } else {
        await invoke("remove_recent_project", { path: project.path });
      }
      await loadRecents();
    } catch (err) {
      reportError(err);
    }
  });
  actions.appendChild(primaryBtn);

  const menuWrap = document.createElement("div");
  menuWrap.className = "menu-dropdown";

  const menuTrigger = document.createElement("button");
  menuTrigger.type = "button";
  menuTrigger.className = "btn btn-icon-only btn-sm btn-ghost";
  menuTrigger.setAttribute("aria-label", "More");
  menuTrigger.dataset.tooltip = "More";
  menuTrigger.innerHTML = MORE_ICON;
  menuTrigger.addEventListener("click", (e) => {
    e.stopPropagation();
    const wasOpen = menuWrap.classList.contains("is-open");
    document.querySelectorAll(".menu-dropdown.is-open").forEach((d) => d.classList.remove("is-open"));
    if (!wasOpen) menuWrap.classList.add("is-open");
  });
  menuWrap.appendChild(menuTrigger);

  const menuList = document.createElement("div");
  menuList.className = "menu-dropdown-list anchor-right";

  // Pin only makes sense from a Recents row — a Pinned row's equivalent action is the standalone
  // Unpin button above, so there's nothing to duplicate here.
  if (!isPinned) {
    const pinItem = document.createElement("button");
    pinItem.type = "button";
    pinItem.className = "menu-dropdown-item";
    pinItem.textContent = "Pin";
    pinItem.addEventListener("click", async (e) => {
      e.stopPropagation();
      try {
        await invoke("set_recent_pinned", { path: project.path, pinned: true });
        await loadRecents();
      } catch (err) {
        reportError(err);
      }
    });
    menuList.appendChild(pinItem);
  }

  // Disabled — dev-run isn't wired up anywhere in the app yet (same as the editor's own Run
  // controls), this is just a placeholder for where a quick-run action will go.
  const runItem = document.createElement("button");
  runItem.type = "button";
  runItem.className = "menu-dropdown-item";
  runItem.textContent = "Run";
  runItem.disabled = true;
  menuList.appendChild(runItem);

  menuWrap.appendChild(menuList);
  actions.appendChild(menuWrap);

  item.appendChild(actions);
  return item;
}

function renderRecentList(container, projects, isPinned) {
  container.innerHTML = "";
  for (const project of projects) {
    container.appendChild(buildRecentItem(project, isPinned));
  }
}

async function loadRecents() {
  const all = await invoke("list_recent_projects");
  const pinned = all.filter((p) => p.pinned);
  const recents = all.filter((p) => !p.pinned);

  // Both sections always show — an empty one gets a short standard "nothing here" line instead
  // of disappearing, so Pinned doesn't just vanish the moment nothing's pinned.
  document.getElementById("pinned-empty").style.display = pinned.length ? "none" : "";
  document.getElementById("recents-empty").style.display = recents.length ? "none" : "";

  renderRecentList(pinnedListEl, pinned, true);
  renderRecentList(recentListEl, recents, false);

  initTooltips();
}

initTooltips();
initWindowControls();

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
        const projectPath = await invoke("create_project", { parentDir: ctx.target.parentDir, name: nameInput.value });
        ctx.close(projectPath);
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

document.getElementById("new-project").addEventListener("click", async () => {
  const parentDir = await openDialog({ directory: true, title: "Choose a folder for the new project" });
  if (!parentDir) return;
  const projectPath = await showPopup("new-project", { parentDir });
  if (projectPath) openEditor(projectPath);
});

document.getElementById("open-project").addEventListener("click", async () => {
  const path = await openDialog({ directory: true, title: "Open a LowArc Studio project" });
  if (!path) return;

  try {
    await invoke("open_project", { path });
    openEditor(path);
  } catch (err) {
    reportError(err);
  }
});

contributeIframePopup("settings", { title: "Settings" });
contributeIframePopup("modules", { title: "Modules" });
contributeIframePopup("plugins", { title: "Plugins" });

document.getElementById("open-settings").addEventListener("click", () => {
  showPopup("settings", { url: "settings.html" });
});

document.getElementById("open-modules").addEventListener("click", () => {
  showPopup("modules", { url: "modules.html" });
});

document.getElementById("open-plugins").addEventListener("click", () => {
  showPopup("plugins", { url: "plugins.html" });
});

document.getElementById("open-website").addEventListener("click", () => {
  openUrl("https://lowarc.com");
});

loadRecents().catch(reportError);
