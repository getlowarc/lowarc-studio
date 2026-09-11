// No native prompt()/confirm()/alert() here: the plugin iframe is sandboxed with
// sandbox="allow-scripts" only (no allow-modals), so those are silently no-ops, not errors. Every
// interaction that would normally be a dialog (new file/folder naming, rename, delete
// confirmation) is built as inline UI instead: the same reason VS Code's own explorer edits tree
// rows in place rather than popping a native prompt.

const CHEVRON_SVG = '<svg viewBox="0 0 10 10" fill="none"><path d="M3 1l4 4-4 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const FILE_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 2h5l3 3v9H4V2Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M9 2v3h3" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></svg>';

// Read synchronously off the iframe's own URL fragment, not pushed in later via postMessage: no
// race between "did the host's push arrive before I started listening" to get wrong.
// URLSearchParams parses a "#"-free string, so the leading "#" gets stripped first.
let root = new URLSearchParams(window.location.hash.replace(/^#/, "")).get("project");
let selectedPath = null;
let selectedIsDir = false;
const expandedDirs = new Set();
const treeCache = new Map(); // absolute path -> sorted [{name, isDir}]
let clipboard = null; // { path, mode: "cut" | "copy" } | null
let pendingCreate = null; // { dir, isDir } | null: an inline create row is showing
let renamingPath = null; // path whose row is currently an editable input
let pendingDelete = null; // path awaiting the inline delete confirmation
let fileStatus = {}; // absolute path -> { dirty, missing, hasErrors }. See broadcastFileStatus() host-side

// This plugin's own configured defaults (Settings > Plugins > File Explorer, see plugin.json's
// `settings` declaration): read once at load, before the very first renderTree(), via
// window.lowarc.getSettings(). Like every setting read this way, a change made while this panel
// is already open takes effect on its next mount (reload), not live.
let configuredShowHidden = false;
let configuredFoldersFirst = true;
let configuredDraftTool = true;

// ---------- Draft Tool state ----------
// A lightweight, disk-based backward snapshot for the CURRENT session only: bolted onto the
// explorer rather than a separate plugin (see file_explorer_backend.rs's own header for how it
// actually works). draftId is null when no Draft is open; diffCounts only ever has entries while
// one is.
let draftId = null;
let diffCounts = {}; // absolute path -> {added, removed}, from the last diffStatus() call

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
    let entries = await callBackend("listDir", { path });
    if (!configuredShowHidden) entries = entries.filter((e) => !e.name.startsWith("."));
    entries.sort((a, b) => {
      if (configuredFoldersFirst && a.isDir !== b.isDir) return a.isDir ? -1 : 1;
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

// One wrapper per row, not `depth` separate children of `.row` directly: `.row` uses flex `gap`
// to space out its real children (chevron/icon/name), which would just as happily shove the guide
// lines apart from each other too and break the illusion of one continuous line per ancestor.
// Wrapping them keeps `.row`'s gap out of it; the guides pack flush against each other inside.
function appendIndentGuides(row, depth) {
  if (depth === 0) return;
  const wrap = document.createElement("span");
  wrap.className = "indent-guides";
  for (let i = 0; i < depth; i++) {
    const guide = document.createElement("span");
    guide.className = "indent-guide";
    wrap.appendChild(guide);
  }
  row.appendChild(wrap);
}

// Files get the real per-language glyph from the vendored Seti set (see window.lowarcIconClass,
// __lowarc-icons.js) when it is loaded. Falling back to the generic FILE_SVG keeps this plugin
// working even if a build omits the icon assets, rather than leaving a blank icon slot.
function fileIconHtml(name) {
  if (typeof window.lowarcIconClass !== "function") return FILE_SVG;
  return `<span class="file-icon ${window.lowarcIconClass(name)}"></span>`;
}

// ONE slot, not a chevron slot followed by a separate icon slot: a folder's chevron and a file's
// icon are the same thing positionally (the row's one "what is this" marker), so they need to
// literally share one element's box, not two sequential ones. Two separate slots would make every
// file row reserve an invisible chevron-width column before its icon started, so a file's icon and
// a sibling folder's chevron would not line up.
function createRowMarker(isDir, isExpanded, label) {
  const marker = document.createElement("span");
  marker.className = "row-marker" + (isDir ? " chevron" + (isExpanded ? " expanded" : "") : "");
  marker.innerHTML = isDir ? CHEVRON_SVG : fileIconHtml(label);
  return marker;
}

function appendDiffCounts(row, path) {
  const counts = diffCounts[path];
  if (!counts || (!counts.added && !counts.removed)) return;
  const wrap = document.createElement("span");
  wrap.className = "row-diff";
  if (counts.added) {
    const added = document.createElement("span");
    added.className = "row-diff-added";
    added.textContent = lowarcFormatCompact(counts.added);
    wrap.appendChild(added);
  }
  if (counts.removed) {
    const removed = document.createElement("span");
    removed.className = "row-diff-removed";
    removed.textContent = lowarcFormatCompact(counts.removed);
    wrap.appendChild(removed);
  }
  row.appendChild(wrap);
}

function createRow({ path, label, isDir, depth }) {
  const row = document.createElement("div");
  row.className = "row" + (path === selectedPath ? " selected" : "");
  appendIndentGuides(row, depth);
  row.appendChild(createRowMarker(isDir, expandedDirs.has(path), label));

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

  // Right-side status dot, mirroring the tab bar's own status colors: errors take priority over
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

  // Draft Tool decoration: only while a Draft is actually open, and only for a file that's
  // genuinely changed since it opened (diffCounts is empty for anything untouched).
  if (!isDir && draftId) appendDiffCounts(row, path);

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
  appendIndentGuides(row, depth);

  // Nothing's been typed yet to pick an extension-specific glyph from: this always shows the
  // generic file icon regardless of what's ultimately created, until the row re-renders as a real
  // row (createRow, above) with the committed name. A pending folder's chevron is non-functional
  // (nothing to expand yet) but shown anyway, matching createRow's marker for visual consistency.
  const marker = document.createElement("span");
  marker.className = "row-marker" + (isDir ? " chevron" : "");
  marker.innerHTML = isDir ? CHEVRON_SVG : FILE_SVG;
  row.appendChild(marker);

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

// Every renderTree() call gets a generation stamp. If a newer call starts while an older one is
// still awaiting a backend round-trip, the older abandons itself at the next checkpoint rather than
// appending stale rows into a container the newer one has already cleared. Without it, two
// overlapping renders interleave their DOM writes and the tree duplicates itself.
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

// Whole-project totals, not just what's currently expanded: recomputed after every mutation
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
    // The open-files list is host-owned, not this plugin's: clicking a file just asks the host
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
    renderTree(); // renaming changes neither the file/folder count nor total bytes: no refreshTotals()
    // Only ever flags the exact renamed path's own open tab, if there is one: a renamed folder
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
    renderTree(); // relocating within the project changes neither total: no refreshTotals()
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
  // A paste is either a move (no total change) or a copy (adds files/bytes): refreshing
  // unconditionally is simpler than threading that distinction through, and cheap either way.
  refreshTotals();
  // A copy leaves the original in place: only a cut (move) actually displaces the source path.
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
// Rendered by the HOST, not this iframe: window.lowarc.showMenu() asks editor.html to open the
// shared .floating-menu overlay at this iframe's (x, y) plus wherever the host has this panel
// mounted, and resolves to whichever item's value was picked, or null if dismissed. Building the
// menu in this document instead does not work: position:fixed cannot escape a sandboxed iframe's
// box, so a menu opened flush against this panel's edge clips there even though the rest of the
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

// ---------- Draft Tool ----------

function showDraftForm() {
  document.getElementById("draft-open-form").style.display = "";
  document.getElementById("draft-summary").style.display = "none";
  document.getElementById("draft-controls").style.display = "none";
}

function showOpenDraft(label, description) {
  document.getElementById("draft-open-form").style.display = "none";
  document.getElementById("draft-summary").style.display = "";
  document.getElementById("draft-controls").style.display = "";
  document.getElementById("draft-summary-label").textContent = label;
  document.getElementById("draft-summary-description").textContent = description || "";
}

async function refreshDiffStatus() {
  if (!draftId) return;
  try {
    diffCounts = await callBackend("diffStatus", { draftId });
  } catch (err) {
    diffCounts = {};
  }
  window.lowarc.setDiffStatus(diffCounts);
  renderTree();
}

document.getElementById("open-draft-btn").addEventListener("click", async () => {
  const label = document.getElementById("draft-label-input").value.trim() || "Untitled Draft";
  const description = document.getElementById("draft-description-input").value.trim();
  const id = crypto.randomUUID();
  try {
    await callBackend("openDraft", { draftId: id, label, description });
  } catch (err) {
    showError(String(err));
    return;
  }
  draftId = id;
  diffCounts = {};
  window.lowarc.setDiffStatus(diffCounts);
  showOpenDraft(label, description);
  document.getElementById("draft-label-input").value = "";
  document.getElementById("draft-description-input").value = "";
  renderTree();
});

// A lightweight inline confirm, same reasoning as beginDelete's own banner: Revert discards real
// edits, that's worth one extra click to avoid an accidental loss.
document.getElementById("revert-btn").addEventListener("click", () => {
  const btn = document.getElementById("revert-btn");
  if (btn.dataset.confirming) {
    doRevertDraft();
    return;
  }
  btn.dataset.confirming = "true";
  btn.textContent = "Confirm Revert";
  setTimeout(() => {
    delete btn.dataset.confirming;
    btn.textContent = "Revert";
  }, 3000);
});

async function doRevertDraft() {
  const id = draftId;
  // Grab the reverted paths BEFORE clearing diffCounts below. Revert writes straight to disk,
  // bypassing whatever editor already has one of these files open, so each one needs an explicit
  // push (window.lowarc.refreshFile) or it just keeps showing the now-stale edited content.
  const revertedPaths = Object.keys(diffCounts);
  try {
    await callBackend("revertAll", { draftId: id });
    revertedPaths.forEach((path) => window.lowarc.refreshFile(path));
  } catch (err) {
    showError(String(err));
  }
  draftId = null;
  diffCounts = {};
  window.lowarc.setDiffStatus(diffCounts);
  invalidateAll();
  showDraftForm();
  renderTree();
}

document.getElementById("commit-btn").addEventListener("click", async () => {
  const id = draftId;
  try {
    await callBackend("commit", { draftId: id });
  } catch (err) {
    showError(String(err));
  }
  draftId = null;
  diffCounts = {};
  window.lowarc.setDiffStatus(diffCounts);
  showDraftForm();
  renderTree();
});

// The host's generic "about to overwrite" broadcast (see write_text_file in lib.rs): the actual
// backward-snapshot moment. Only matters while a Draft is open; every other plugin just ignores
// this same broadcast.
window.lowarc.on("lowarc:beforeSave", ({ path, previousContent }) => {
  if (!draftId) return;
  callBackend("captureBaseline", { draftId, path, previousContent })
    .then(refreshDiffStatus)
    .catch((err) => showError(String(err)));
});

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
// (this one) mounts. See broadcastFileStatus() in editor.html. A full snapshot, not a delta, so
// this just replaces the whole map and re-renders rather than merging.
window.lowarc.on("lowarc:fileStatus", (status) => {
  fileStatus = status || {};
  renderTree();
});

// This iframe's own JS state (draftId, diffCounts) is gone the instant a page refresh happens,
// but the backend process and its disk storage aren't, so this is the one thing standing between
// "just reload the window" and silently losing track of an open Draft. Safe to call unconditionally
// on every load (a fresh mount included): with nothing open, the backend just replies null and this
// is a no-op. Restores regardless of the draftTool setting, if it's off, #draft-tool stays hidden
// either way, but the underlying tracking (and status bar diff) shouldn't depend on that toggle.
async function restoreActiveDraftIfAny() {
  let active;
  try {
    active = await callBackend("getActiveDraft", {});
  } catch (err) {
    return;
  }
  if (!active) return;
  draftId = active.draftId;
  showOpenDraft(active.label, active.description);
  await refreshDiffStatus();
}

// Live counterpart to the getSettings() read below. See set_plugin_setting/plugin-setting-changed
// in lib.rs and its relay to lowarc:settingsChanged in split-view.js. showHidden/foldersFirst just
// need a re-render; draftTool also needs the section's own visibility toggled (it's independent of
// the underlying tracking, same as the initial read below).
window.lowarc.on("lowarc:settingsChanged", ({ key, value }) => {
  if (key === "showHidden") configuredShowHidden = value === "true";
  else if (key === "foldersFirst") configuredFoldersFirst = value !== "false";
  else if (key === "draftTool") {
    configuredDraftTool = value !== "false";
    document.getElementById("draft-tool").style.display = configuredDraftTool ? "" : "none";
  } else return;
  renderTree();
});

if (root) {
  const header = document.getElementById("header");
  header.textContent = baseName(root) || root;
  header.title = root; // full path on hover, since a long root name truncates with an ellipsis
  window.lowarc.getSettings().then((settings) => {
    configuredShowHidden = (settings && settings.showHidden) === "true";
    configuredFoldersFirst = !(settings && settings.foldersFirst === "false");
    configuredDraftTool = !(settings && settings.draftTool === "false");
    document.getElementById("draft-tool").style.display = configuredDraftTool ? "" : "none";
    renderTree();
    refreshTotals();
  });
  restoreActiveDraftIfAny();
} else {
  showError("No project path was provided to this panel.");
}
