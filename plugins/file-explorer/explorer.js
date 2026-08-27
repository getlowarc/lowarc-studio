// No native prompt()/confirm()/alert() here — the plugin iframe is sandboxed with
// sandbox="allow-scripts" only (no allow-modals), so those are silently no-ops, not errors. Every
// interaction that would normally be a dialog (new file/folder naming, rename, delete
// confirmation) is built as inline UI instead — the same reason VS Code's own explorer edits tree
// rows in place rather than popping a native prompt.

const CHEVRON_SVG = '<svg viewBox="0 0 10 10" fill="none"><path d="M3 1l4 4-4 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const FOLDER_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M2 4.5C2 3.67 2.67 3 3.5 3H6.5L8 4.5H12.5C13.33 4.5 14 5.17 14 6V11.5C14 12.33 13.33 13 12.5 13H3.5C2.67 13 2 12.33 2 11.5V4.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></svg>';
const FILE_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 2h5l3 3v9H4V2Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M9 2v3h3" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></svg>';

// Read synchronously off the iframe's own URL fragment, not pushed in later via postMessage — no
// race between "did the host's push arrive before I started listening" to get wrong.
// URLSearchParams parses a "#"-free string, so the leading "#" gets stripped first.
let root = new URLSearchParams(window.location.hash.replace(/^#/, "")).get("project");
let selectedPath = null;
let selectedIsDir = false;
const expandedDirs = new Set();
const treeCache = new Map(); // absolute path -> sorted [{name, isDir}]
let clipboard = null; // { path, mode: "cut" | "copy" } | null
let pendingCreate = null; // { dir, isDir } | null — an inline create row is showing
let renamingPath = null; // path whose row is currently an editable input
let pendingDelete = null; // path awaiting the inline delete confirmation
let fileStatus = {}; // absolute path -> { dirty, missing, hasErrors } — see broadcastFileStatus() host-side

function sep() {
  return root && root.includes("\\") ? "\\" : "/";
}

function joinPath(dir, name) {
  const s = sep();
  return dir.endsWith(s) ? dir + name : dir + s + name;
}

function dirName(path) {
  const s = sep();
  const idx = path.lastIndexOf(s);
  return idx === -1 ? path : path.slice(0, idx);
}

function baseName(path) {
  const s = sep();
  const idx = path.lastIndexOf(s);
  return idx === -1 ? path : path.slice(idx + 1);
}

async function callBackend(method, params) {
  return window.lowarc.call(method, Object.assign({}, params, { root }));
}

function showError(message) {
  const banner = document.getElementById("banner");
  banner.className = "banner is-error";
  banner.textContent = String(message);
  clearTimeout(showError._t);
  showError._t = setTimeout(() => {
    if (banner.className === "banner is-error") banner.className = "banner";
  }, 5000);
}

function clearBanner() {
  const banner = document.getElementById("banner");
  banner.className = "banner";
  banner.innerHTML = "";
}

function invalidate(dirPath) {
  treeCache.delete(dirPath);
}

function invalidateAll() {
  treeCache.clear();
}

// ---------- Tree data ----------

async function loadChildren(path) {
  if (!treeCache.has(path)) {
    const entries = await callBackend("listDir", { path });
    entries.sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
    });
    treeCache.set(path, entries);
  }
  return treeCache.get(path);
}

// VS Code's "compact folders": a chain of directories that each have exactly one child, itself a
// directory, collapses into a single row labelled with the whole chain. Stops at the first real
// branch point (0 or 2+ children) or a file. loadChildren() populates the cache for every
// directory walked along the way, so expanding the resulting row never re-fetches anything.
async function resolveCompactChain(path, nameParts) {
  const children = await loadChildren(path);
  if (children.length === 1 && children[0].isDir) {
    return resolveCompactChain(joinPath(path, children[0].name), nameParts.concat(children[0].name));
  }
  return { path, label: nameParts.join(sep()), children };
}

// ---------- Rendering ----------

function targetDirFor(target) {
  if (!target) return selectedPath ? (selectedIsDir ? selectedPath : dirName(selectedPath)) : root;
  return target.isDir ? target.path : dirName(target.path);
}

function createInlineInput(initialValue, onCommit) {
  const input = document.createElement("input");
  input.type = "text";
  input.value = initialValue;
  input.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      onCommit(input.value.trim());
    } else if (e.key === "Escape") {
      e.preventDefault();
      onCommit(null);
    }
  });
  input.addEventListener("blur", () => onCommit(null));
  input.addEventListener("click", (e) => e.stopPropagation());
  return input;
}

function createRow({ path, label, isDir, depth }) {
  const row = document.createElement("div");
  row.className = "row" + (path === selectedPath ? " selected" : "");
  row.style.paddingLeft = depth * 12 + 4 + "px";

  const chevron = document.createElement("span");
  chevron.className = "chevron" + (isDir ? (expandedDirs.has(path) ? " expanded" : "") : " hidden");
  if (isDir) chevron.innerHTML = CHEVRON_SVG;
  row.appendChild(chevron);

  const icon = document.createElement("span");
  icon.className = "row-icon";
  icon.innerHTML = isDir ? FOLDER_SVG : FILE_SVG;
  row.appendChild(icon);

  const nameEl = document.createElement("span");
  nameEl.className = "row-name";

  if (renamingPath === path) {
    const input = createInlineInput(baseName(path), (value) => {
      renamingPath = null;
      if (value && value !== baseName(path)) commitRename(path, value);
      else renderTree();
    });
    nameEl.appendChild(input);
    queueMicrotask(() => {
      input.focus();
      input.select();
    });
  } else {
    nameEl.textContent = label;
    nameEl.title = path;
  }
  row.appendChild(nameEl);

  // Right-side status dot, mirroring the tab bar's own status colors — errors take priority over
  // a plain unsaved edit, since a file can be both at once and "it doesn't even parse" is the
  // more urgent fact. Only ever shown for files, never folders (fileStatus is keyed by open file
  // paths, which folders never are), and not while the row is showing a rename input in its place.
  if (!isDir && renamingPath !== path) {
    const info = fileStatus[path];
    if (info && (info.hasErrors || info.dirty)) {
      const dot = document.createElement("span");
      dot.className = "row-status-dot " + (info.hasErrors ? "status-error" : "status-dirty");
      row.appendChild(dot);
    }
  }

  row.addEventListener("click", () => onRowClick(path, isDir));
  row.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    selectedPath = path;
    selectedIsDir = isDir;
    renderTree();
    openContextMenu(e.clientX, e.clientY, { path, isDir });
  });

  row.draggable = true;
  row.addEventListener("dragstart", (e) => {
    e.dataTransfer.setData("text/plain", path);
    e.dataTransfer.effectAllowed = "move";
  });
  if (isDir) {
    row.addEventListener("dragover", (e) => {
      e.preventDefault();
      row.classList.add("drop-target");
    });
    row.addEventListener("dragleave", () => row.classList.remove("drop-target"));
    row.addEventListener("drop", async (e) => {
      e.preventDefault();
      row.classList.remove("drop-target");
      const sourcePath = e.dataTransfer.getData("text/plain");
      if (!sourcePath || sourcePath === path) return;
      await moveInto(sourcePath, path);
    });
  }

  return row;
}

function createInlineCreateRow(dir, isDir, depth) {
  const row = document.createElement("div");
  row.className = "row";
  row.style.paddingLeft = depth * 12 + 4 + "px";

  const chevron = document.createElement("span");
  chevron.className = "chevron hidden";
  row.appendChild(chevron);

  const icon = document.createElement("span");
  icon.className = "row-icon";
  icon.innerHTML = isDir ? FOLDER_SVG : FILE_SVG;
  row.appendChild(icon);

  const nameEl = document.createElement("span");
  nameEl.className = "row-name";
  const input = createInlineInput("", (value) => {
    pendingCreate = null;
    if (value) commitCreate(dir, isDir, value);
    else renderTree();
  });
  nameEl.appendChild(input);
  row.appendChild(nameEl);
  queueMicrotask(() => input.focus());

  return row;
}

// Every renderTree() call gets a generation stamp; if a newer call starts before an older one's
// awaits (loadChildren/resolveCompactChain, each a real round-trip to the backend) finish, the
// older one abandons itself at the next checkpoint instead of appending stale rows into a
// container a newer render has already cleared and started refilling. Without this, two
// overlapping renders (e.g. beginCreate's render still in flight when commitCreate's own render
// starts right after the backend call resolves) interleave their DOM writes into the same
// container — confirmed live: creating a file duplicated the whole tree until a chevron click
// forced a fresh, non-overlapping render.
let renderGeneration = 0;

async function renderDirChildren(dirPath, container, depth, generation) {
  if (generation !== renderGeneration) return;
  if (pendingCreate && pendingCreate.dir === dirPath) {
    container.appendChild(createInlineCreateRow(dirPath, pendingCreate.isDir, depth));
  }

  const children = await loadChildren(dirPath);
  if (generation !== renderGeneration) return;
  for (const entry of children) {
    if (generation !== renderGeneration) return;
    const childPath = joinPath(dirPath, entry.name);
    if (entry.isDir) {
      const chain = await resolveCompactChain(childPath, [entry.name]);
      if (generation !== renderGeneration) return;
      container.appendChild(createRow({ path: chain.path, label: chain.label, isDir: true, depth }));
      if (expandedDirs.has(chain.path)) {
        await renderDirChildren(chain.path, container, depth + 1, generation);
      }
    } else {
      container.appendChild(createRow({ path: childPath, label: entry.name, isDir: false, depth }));
    }
  }
}

async function renderTree() {
  const generation = ++renderGeneration;
  const container = document.getElementById("tree");
  container.innerHTML = "";
  if (!root) return;
  try {
    await renderDirChildren(root, container, 0, generation);
    if (generation !== renderGeneration) return; // a newer renderTree() call has since superseded this one
    if (!container.firstChild) {
      const empty = document.createElement("div");
      empty.className = "empty-hint";
      empty.textContent = "This folder is empty.";
      container.appendChild(empty);
    }
  } catch (err) {
    if (generation === renderGeneration) showError(String(err));
  }
}

// ---------- Footer totals ----------

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

// Whole-project totals, not just what's currently expanded — recomputed after every mutation
// (create/rename/move/copy/delete/refresh) rather than tracked incrementally, since the backend
// already has to walk the tree to answer any of those and a stale total would be worse than a
// briefly-blank one.
async function refreshTotals() {
  const footer = document.getElementById("footer");
  try {
    const totals = await callBackend("countTree", {});
    const fileLabel = `${totals.files} file${totals.files === 1 ? "" : "s"}`;
    const folderLabel = `${totals.folders} folder${totals.folders === 1 ? "" : "s"}`;
    footer.textContent = `${fileLabel}, ${folderLabel} · ${formatBytes(totals.bytes)}`;
  } catch (err) {
    footer.textContent = "";
  }
}

function onRowClick(path, isDir) {
  selectedPath = path;
  selectedIsDir = isDir;
  if (isDir) {
    if (expandedDirs.has(path)) expandedDirs.delete(path);
    else expandedDirs.add(path);
  } else {
    // The open-files list is host-owned, not this plugin's — clicking a file just asks the host
    // to add it (or activate it, if already open) rather than this plugin rendering anything
    // itself. Same list the tab bar renders from and Monaco will read from.
    window.lowarc.openFile(path);
  }
  renderTree();
}

// ---------- Mutating actions ----------

async function commitCreate(dir, isDir, name) {
  try {
    await callBackend(isDir ? "createFolder" : "createFile", { path: joinPath(dir, name) });
    invalidate(dir);
    expandedDirs.add(dir);
    renderTree();
    refreshTotals();
  } catch (err) {
    showError(String(err));
    renderTree();
  }
}

async function commitRename(path, newName) {
  try {
    await callBackend("rename", { path, newName });
    const parent = dirName(path);
    invalidate(parent);
    if (selectedPath === path) selectedPath = joinPath(parent, newName);
    renderTree(); // renaming changes neither the file/folder count nor total bytes — no refreshTotals()
    // Only ever flags the exact renamed path's own open tab, if there is one — a renamed folder
    // doesn't cascade to everything open underneath it (see the host's own comment on this).
    window.lowarc.notifyPathRenamed(path, joinPath(parent, newName));
  } catch (err) {
    showError(String(err));
    renderTree();
  }
}

async function moveInto(sourcePath, destDir) {
  if (dirName(sourcePath) === destDir) return; // already there
  try {
    await callBackend("move", { sourcePath, destDir });
    invalidate(dirName(sourcePath));
    invalidate(destDir);
    expandedDirs.add(destDir);
    renderTree(); // relocating within the project changes neither total — no refreshTotals()
    window.lowarc.notifyPathRenamed(sourcePath, joinPath(destDir, baseName(sourcePath)));
  } catch (err) {
    showError(String(err));
  }
}

async function pasteInto(destDir) {
  if (!clipboard) return;
  const wasCut = clipboard.mode === "cut";
  const sourcePath = clipboard.path;
  try {
    await callBackend(wasCut ? "move" : "copy", { sourcePath, destDir });
  } catch (err) {
    showError(String(err));
    return;
  }
  if (wasCut) invalidate(dirName(sourcePath));
  invalidate(destDir);
  expandedDirs.add(destDir);
  if (wasCut) clipboard = null;
  renderTree();
  // A paste is either a move (no total change) or a copy (adds files/bytes) — refreshing
  // unconditionally is simpler than threading that distinction through, and cheap either way.
  refreshTotals();
  // A copy leaves the original in place — only a cut (move) actually displaces the source path.
  if (wasCut) window.lowarc.notifyPathRenamed(sourcePath, joinPath(destDir, baseName(sourcePath)));
}

async function deleteConfirmed(path) {
  try {
    await callBackend("deletePath", { path });
    invalidate(dirName(path));
    if (selectedPath === path) selectedPath = null;
    clearBanner();
    renderTree();
    refreshTotals();
    window.lowarc.notifyPathDeleted(path);
  } catch (err) {
    showError(String(err));
  }
}

// ---------- Context menu ----------
// Rendered by the HOST, not this iframe — window.lowarc.showMenu() asks editor.html to open the
// shared .floating-menu overlay at this iframe's (x, y) plus wherever the host has this panel
// mounted, and resolves to whichever item's value was picked (or null if dismissed). This used to
// be a menu built and positioned entirely inside this document, clamped by hand to this iframe's
// own viewport — position:fixed can't escape a sandboxed iframe's box no matter what, so a menu
// opened flush against this panel's edge would otherwise clip there even though the rest of the
// window has room. showMenu() replaces all of that; see lowarc-studio-floating-menu memory for the
// host-side half of this.
async function openContextMenu(x, y, target) {
  const dir = targetDirFor(target);
  const items = [{ label: "New File", value: "new-file" }, { label: "New Folder", value: "new-folder" }];

  if (target && !target.isDir) {
    items.push({ divider: true }, { label: "Open in Split View", value: "open-split" });
  }

  if (target) {
    items.push(
      { divider: true },
      { label: "Rename", value: "rename" },
      { label: "Delete", value: "delete" },
      { divider: true },
      { label: "Cut", value: "cut" },
      { label: "Copy", value: "copy" },
    );
  }

  items.push({ label: "Paste", value: "paste", disabled: !clipboard });

  if (target) {
    items.push({ divider: true }, { label: "Copy Path", value: "copy-path" });
  } else {
    items.push({ divider: true }, { label: "Refresh", value: "refresh" });
  }

  const picked = await window.lowarc.showMenu(items, x, y);
  switch (picked) {
    case "new-file":
      beginCreate(dir, false);
      break;
    case "new-folder":
      beginCreate(dir, true);
      break;
    case "open-split":
      if (target) window.lowarc.openFile(target.path, { openInSplit: true });
      break;
    case "rename":
      if (target) {
        renamingPath = target.path;
        renderTree();
      }
      break;
    case "delete":
      if (target) beginDelete(target.path);
      break;
    case "cut":
      if (target) clipboard = { path: target.path, mode: "cut" };
      break;
    case "copy":
      if (target) clipboard = { path: target.path, mode: "copy" };
      break;
    case "paste":
      pasteInto(dir);
      break;
    case "copy-path":
      if (target) copyPathToClipboard(target.path);
      break;
    case "refresh":
      refreshAll();
      break;
  }
}

function copyPathToClipboard(path) {
  if (!navigator.clipboard || !navigator.clipboard.writeText) {
    showError(`Clipboard isn't available here — path: ${path}`);
    return;
  }
  navigator.clipboard.writeText(path).catch(() => showError(`Couldn't copy — path: ${path}`));
}

function beginCreate(dir, isDir) {
  expandedDirs.add(dir);
  pendingCreate = { dir, isDir };
  renderTree();
}

function beginDelete(path) {
  pendingDelete = path;
  const banner = document.getElementById("banner");
  banner.className = "banner is-confirm";
  banner.innerHTML = "";
  const text = document.createElement("span");
  text.textContent = `Delete "${baseName(path)}"?`;
  banner.appendChild(text);
  const actions = document.createElement("span");
  actions.className = "banner-actions";
  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "btn btn-ghost btn-xs";
  cancel.textContent = "Cancel";
  cancel.addEventListener("click", () => {
    pendingDelete = null;
    clearBanner();
  });
  const confirm = document.createElement("button");
  confirm.type = "button";
  confirm.className = "btn btn-danger btn-xs";
  confirm.textContent = "Delete";
  confirm.addEventListener("click", () => {
    pendingDelete = null;
    deleteConfirmed(path);
  });
  actions.appendChild(cancel);
  actions.appendChild(confirm);
  banner.appendChild(actions);
}

function refreshAll() {
  invalidateAll();
  renderTree();
  refreshTotals();
}

// ---------- Wiring ----------

document.getElementById("new-file-btn").addEventListener("click", () => beginCreate(targetDirFor(null), false));
document.getElementById("new-folder-btn").addEventListener("click", () => beginCreate(targetDirFor(null), true));
document.getElementById("refresh-btn").addEventListener("click", refreshAll);

document.getElementById("tree").addEventListener("contextmenu", (e) => {
  if (e.target.id !== "tree") return; // a row already handled its own contextmenu and stopped propagation
  e.preventDefault();
  selectedPath = null;
  selectedIsDir = false;
  renderTree();
  openContextMenu(e.clientX, e.clientY, null);
});

// Pushed by the host any time a file's dirty/error/missing state changes, or a new plugin panel
// (this one) mounts — see broadcastFileStatus() in editor.html. A full snapshot, not a delta, so
// this just replaces the whole map and re-renders rather than merging.
window.lowarc.on("lowarc:fileStatus", (status) => {
  fileStatus = status || {};
  renderTree();
});

if (root) {
  const header = document.getElementById("header");
  header.textContent = baseName(root) || root;
  header.title = root; // full path on hover, since a long root name truncates with an ellipsis
  renderTree();
  refreshTotals();
} else {
  showError("No project path was provided to this panel.");
}
