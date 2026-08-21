// Startup screen — new/open project, recents, and links out to the other top-level pages.
// Talks to the Rust side purely through the commands registered in lib.rs. Load order matters:
// primitives.js must load before this file since it defines openPopup/closePopup/showToast/
// initPopups, all used below.

const { invoke } = window.__TAURI__.core;
const { open: openDialog } = window.__TAURI__.dialog;
const { openUrl } = window.__TAURI__.opener;

const recentListEl = document.getElementById("recent-list");

function reportError(err) {
  showToast({ variant: "error", message: String(err) });
}

function openEditor(projectPath) {
  window.location.href = `editor.html?project=${encodeURIComponent(projectPath)}`;
}

async function loadRecents() {
  const recents = await invoke("list_recent_projects");
  recentListEl.innerHTML = "";

  if (recents.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.textContent = "No recent projects yet — create or open one to get started.";
    recentListEl.appendChild(empty);
    return;
  }

  for (const project of recents) {
    const item = document.createElement("div");
    item.className = "recent-item" + (project.exists ? "" : " missing");

    const name = document.createElement("div");
    name.className = "name";
    name.textContent = project.name;

    const path = document.createElement("div");
    path.className = "path";
    path.textContent = project.path;

    item.appendChild(name);
    item.appendChild(path);

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

    recentListEl.appendChild(item);
  }
}

initPopups();
initTooltips();

document.getElementById("new-project").addEventListener("click", async () => {
  const parentDir = await openDialog({ directory: true, title: "Choose a folder for the new project" });
  if (!parentDir) return;

  const nameInput = document.getElementById("new-project-name");
  const errorEl = document.getElementById("new-project-error");
  nameInput.value = "";
  errorEl.textContent = "";
  openPopup("new-project-popup");

  const create = async () => {
    try {
      const projectPath = await invoke("create_project", { parentDir, name: nameInput.value });
      closePopup("new-project-popup");
      openEditor(projectPath);
    } catch (err) {
      errorEl.textContent = String(err);
    }
  };

  document.getElementById("new-project-create").onclick = create;
  nameInput.onkeydown = (e) => {
    if (e.key === "Enter") create();
  };
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

document.getElementById("open-settings").addEventListener("click", () => {
  window.location.href = "settings.html";
});

document.getElementById("open-modules").addEventListener("click", () => {
  window.location.href = "modules.html";
});

document.getElementById("open-plugins").addEventListener("click", () => {
  window.location.href = "plugins.html";
});

document.getElementById("open-website").addEventListener("click", () => {
  openUrl("https://lowarc.com");
});

loadRecents().catch(reportError);
