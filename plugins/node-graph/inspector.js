// The Inspector half of this plugin: never touches graph data directly. It shows whatever the
// canvas iframe (node-graph.js) last pushed via window.lowarc.openInspector(), and sends edits
// back the same way node-graph.js sends host actions — except there's no host action for "tell my
// OTHER iframe something", so this uses window.lowarc.broadcastToSelf() instead, addressed by
// path+nodeId since the canvas iframe can have several .lan files open at once.

const root = new URLSearchParams(window.location.hash.replace(/^#/, "")).get("project") || "";

const labelInput = document.getElementById("ng-label-input");
const xInput = document.getElementById("ng-x-input");
const yInput = document.getElementById("ng-y-input");
const filesList = document.getElementById("ng-files-list");
const attachFileBtn = document.getElementById("ng-attach-file-btn");
const hasInputCheckbox = document.getElementById("ng-has-input");
const hasOutputCheckbox = document.getElementById("ng-has-output");
const inputsList = document.getElementById("ng-inputs-list");
const outputsList = document.getElementById("ng-outputs-list");
const newNodeBtn = document.getElementById("ng-new-node-btn");

// { path, nodeId, label, x, y, files, hasInput, hasOutput, inputs, outputs } | null — null before
// anything's ever been selected, which is a legitimate state (the Inspector can be shown with
// nothing to inspect yet if something else in this plugin ever calls openInspector() without a
// node context). inputs/outputs are [{id, label}]: the OTHER nodes this node's input/output
// connects to, a snapshot as of whenever this node was last selected (same as every other field
// here — there's no live push back from the canvas while the Inspector stays open on one node).
let current = null;

function basename(path) {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

function renderFiles() {
  filesList.innerHTML = "";
  if (!current || !current.files.length) {
    const empty = document.createElement("div");
    empty.className = "ng-files-empty";
    empty.textContent = "None";
    filesList.appendChild(empty);
    return;
  }
  current.files.forEach((path, index) => {
    const row = document.createElement("div");
    row.className = "ng-file-row";
    const pathEl = document.createElement("span");
    pathEl.className = "ng-file-path";
    pathEl.textContent = basename(path);
    pathEl.dataset.tooltip = path;
    const removeBtn = document.createElement("button");
    removeBtn.type = "button";
    removeBtn.className = "btn btn-icon-only btn-ghost-danger btn-xs";
    removeBtn.setAttribute("aria-label", `Remove ${path}`);
    removeBtn.innerHTML = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>';
    removeBtn.addEventListener("click", () => {
      current.files.splice(index, 1);
      renderFiles();
      commit({ files: current.files });
    });
    row.appendChild(pathEl);
    row.appendChild(removeBtn);
    filesList.appendChild(row);
  });
  window.lowarc.initTooltips(filesList);
}

// The wiring itself only ever changes on the canvas by dragging a connection: this list is read-
// only for THAT, but clicking a row still jumps you to the node it names (selects it, centers the
// canvas on it, and re-points the Inspector there), since "show me that one" is a real action even
// when "rewire it from here" isn't. Hovering a row highlights that same node on the canvas (a
// yellow outline, distinct from the cyan "selected" styling) so you can spot it before committing
// to a click.
function renderConnectedList(listEl, items) {
  listEl.innerHTML = "";
  if (!items || !items.length) {
    const empty = document.createElement("div");
    empty.className = "ng-connected-empty";
    empty.textContent = "None";
    listEl.appendChild(empty);
    return;
  }
  items.forEach((item) => {
    const row = document.createElement("div");
    row.className = "ng-connected-row";
    row.textContent = item.label || "New Node";
    row.addEventListener("click", () => {
      if (!current) return;
      window.lowarc.broadcastToSelf("lowarc:focusNode", { path: current.path, nodeId: item.id });
    });
    row.addEventListener("mouseenter", () => {
      if (!current) return;
      window.lowarc.broadcastToSelf("lowarc:highlightNode", { path: current.path, nodeId: item.id, on: true });
    });
    row.addEventListener("mouseleave", () => {
      if (!current) return;
      window.lowarc.broadcastToSelf("lowarc:highlightNode", { path: current.path, nodeId: item.id, on: false });
    });
    listEl.appendChild(row);
  });
}

function render() {
  const has = Boolean(current);
  labelInput.disabled = !has;
  xInput.disabled = !has;
  yInput.disabled = !has;
  attachFileBtn.disabled = !has;
  hasInputCheckbox.disabled = !has;
  hasOutputCheckbox.disabled = !has;
  newNodeBtn.disabled = !has;
  if (!has) {
    labelInput.value = "";
    xInput.value = "";
    yInput.value = "";
    filesList.innerHTML = "";
    hasInputCheckbox.checked = false;
    hasOutputCheckbox.checked = false;
    inputsList.innerHTML = "";
    outputsList.innerHTML = "";
    return;
  }
  labelInput.value = current.label;
  xInput.value = current.x;
  yInput.value = current.y;
  hasInputCheckbox.checked = Boolean(current.hasInput);
  hasOutputCheckbox.checked = Boolean(current.hasOutput);
  renderFiles();
  renderConnectedList(inputsList, current.inputs);
  renderConnectedList(outputsList, current.outputs);
}

function commit(patch) {
  if (!current) return;
  Object.assign(current, patch);
  window.lowarc.broadcastToSelf("lowarc:nodeEdit", { path: current.path, nodeId: current.nodeId, patch });
}

window.lowarc.on("lowarc:inspectorContext", (payload) => {
  current = payload
    ? {
        path: payload.path,
        nodeId: payload.nodeId,
        label: payload.label || "",
        x: Number(payload.x) || 0,
        y: Number(payload.y) || 0,
        files: Array.isArray(payload.files) ? payload.files.slice() : [],
        hasInput: Boolean(payload.hasInput),
        hasOutput: Boolean(payload.hasOutput),
        inputs: Array.isArray(payload.inputs) ? payload.inputs : [],
        outputs: Array.isArray(payload.outputs) ? payload.outputs : [],
      }
    : null;
  render();
});

labelInput.addEventListener("change", () => {
  const value = labelInput.value.trim();
  if (value) commit({ label: value });
  else labelInput.value = current.label; // an empty label means nothing — revert rather than save it
});

xInput.addEventListener("change", () => {
  const value = Number(xInput.value);
  if (Number.isFinite(value)) commit({ x: value });
  else xInput.value = current.x;
});
yInput.addEventListener("change", () => {
  const value = Number(yInput.value);
  if (Number.isFinite(value)) commit({ y: value });
  else yInput.value = current.y;
});

hasInputCheckbox.addEventListener("change", () => commit({ hasInput: hasInputCheckbox.checked }));
hasOutputCheckbox.addEventListener("change", () => commit({ hasOutput: hasOutputCheckbox.checked }));

attachFileBtn.addEventListener("click", async () => {
  if (!current) return;
  const path = await window.lowarc.pickOpenFile({ title: "Attach a file", defaultPath: root || undefined });
  if (!path) return;
  current.files.push(path);
  renderFiles();
  commit({ files: current.files });
});

newNodeBtn.addEventListener("click", () => {
  if (!current) return;
  window.lowarc.broadcastToSelf("lowarc:createNode", { path: current.path });
});

window.lowarc.initNumericInputs();
render();
