// Multi-document, same "one instance, several open files" shape Monaco/media-viewer already use —
// see monaco.js's own header comment for the reasoning.
//
// File format (.lan — "LowArc Node", plain JSON): { nodes: [{id, label, x, y, inputs: [name...],
// outputs: [name...]}], connections: [{from: "nodeId:socketName", to: "nodeId:socketName"}] }.
// Deliberately minimal — sockets are untyped names, a connection is just two socket references.
// This is a pure editing surface for now: nothing here feeds into the runtime or a compiler, it
// only draws and saves a graph. Wiring it into actual execution is a separate, much later task.

const container = document.getElementById("container");
const docs = new Map(); // path -> Doc
let activePath = null;

function activeDoc() {
  return activePath ? docs.get(activePath) : null;
}

function newNodeId(doc) {
  doc.nextId += 1;
  return `n${doc.nextId}`;
}

function emptyGraph() {
  return { nodes: [], connections: [] };
}

function parseGraph(text) {
  if (!text || !text.trim()) return emptyGraph();
  try {
    const parsed = JSON.parse(text);
    return {
      nodes: Array.isArray(parsed.nodes) ? parsed.nodes : [],
      connections: Array.isArray(parsed.connections) ? parsed.connections : [],
    };
  } catch (err) {
    return emptyGraph();
  }
}

function serializeGraph(graph) {
  // Only the fields this format actually understands — never round-trips whatever else might
  // have been sitting in a hand-edited file's node/connection objects.
  const nodes = graph.nodes.map((n) => ({ id: n.id, label: n.label, x: n.x, y: n.y, inputs: n.inputs, outputs: n.outputs }));
  const connections = graph.connections.map((c) => ({ from: c.from, to: c.to }));
  return JSON.stringify({ nodes, connections }, null, 2) + "\n";
}

// ---------- Dirty tracking ----------

function markDirty(doc, dirty) {
  if (doc.dirty === dirty) return;
  doc.dirty = dirty;
  window.lowarc.markDirty(doc.path, dirty);
}

function graphChanged(doc) {
  markDirty(doc, true);
  renderConnections(doc);
}

// ---------- Save ----------

let saving = false;
async function save() {
  const doc = activeDoc();
  if (!doc || saving) return;
  saving = true;
  try {
    await window.lowarc.saveFile(doc.path, serializeGraph(doc.graph));
    markDirty(doc, false);
  } catch (err) {
    // The host already surfaces a toast for a failed write.
  } finally {
    saving = false;
  }
}
window.lowarc.on("lowarc:requestSave", save);
window.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    save();
  }
});

// ---------- Node/connection lookup helpers ----------

function findNode(doc, nodeId) {
  return doc.graph.nodes.find((n) => n.id === nodeId) || null;
}

function socketDot(doc, nodeId, socketName, isOutput) {
  const el = doc.nodeEls.get(nodeId);
  if (!el) return null;
  const selector = `.ng-socket-dot[data-socket="${CSS.escape(socketName)}"][data-kind="${isOutput ? "output" : "input"}"]`;
  return el.querySelector(selector);
}

// Center of a socket dot, in .ng-canvas's own coordinate space (unaffected by scroll/zoom — this
// format has no zoom, but staying canvas-relative rather than viewport-relative is still what
// keeps connections correctly positioned while the user scrolls/pans).
function socketCenter(doc, dot) {
  const canvasRect = doc.canvasEl.getBoundingClientRect();
  const dotRect = dot.getBoundingClientRect();
  return {
    x: dotRect.left + dotRect.width / 2 - canvasRect.left,
    y: dotRect.top + dotRect.height / 2 - canvasRect.top,
  };
}

function bezierPath(p1, p2) {
  const dx = Math.max(40, Math.abs(p2.x - p1.x) / 2);
  return `M ${p1.x} ${p1.y} C ${p1.x + dx} ${p1.y}, ${p2.x - dx} ${p2.y}, ${p2.x} ${p2.y}`;
}

function parseEndpoint(ref) {
  const idx = ref.lastIndexOf(":");
  if (idx === -1) return null;
  return { nodeId: ref.slice(0, idx), socket: ref.slice(idx + 1) };
}

// ---------- Rendering ----------

function renderConnections(doc) {
  doc.svgEl.innerHTML = "";
  doc.graph.connections.forEach((conn, index) => {
    const from = parseEndpoint(conn.from);
    const to = parseEndpoint(conn.to);
    if (!from || !to) return;
    const dotA = socketDot(doc, from.nodeId, from.socket, true);
    const dotB = socketDot(doc, to.nodeId, to.socket, false);
    if (!dotA || !dotB) return;
    const d = bezierPath(socketCenter(doc, dotA), socketCenter(doc, dotB));

    const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
    group.setAttribute("class", "ng-connection" + (doc.selectedConnection === index ? " is-selected" : ""));

    const hit = document.createElementNS("http://www.w3.org/2000/svg", "path");
    hit.setAttribute("class", "ng-connection-hit");
    hit.setAttribute("d", d);
    hit.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      selectConnection(doc, index);
    });
    hit.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      e.stopPropagation();
      selectConnection(doc, index);
      openConnectionMenu(doc, index, e.clientX, e.clientY);
    });

    const line = document.createElementNS("http://www.w3.org/2000/svg", "path");
    line.setAttribute("class", "ng-connection-line");
    line.setAttribute("d", d);

    group.appendChild(hit);
    group.appendChild(line);
    doc.svgEl.appendChild(group);
  });
  updateConnectedDots(doc);
}

function updateConnectedDots(doc) {
  doc.nodeEls.forEach((el) => {
    el.querySelectorAll(".ng-socket-dot").forEach((dot) => dot.classList.remove("is-connected"));
  });
  for (const conn of doc.graph.connections) {
    const from = parseEndpoint(conn.from);
    const to = parseEndpoint(conn.to);
    if (from) socketDot(doc, from.nodeId, from.socket, true)?.classList.add("is-connected");
    if (to) socketDot(doc, to.nodeId, to.socket, false)?.classList.add("is-connected");
  }
}

function renderSocketColumn(doc, node, names, isOutput) {
  const col = document.createElement("div");
  col.className = "ng-node-sockets" + (isOutput ? " is-outputs" : "");
  for (const name of names) {
    const row = document.createElement("div");
    row.className = "ng-socket";
    const dot = document.createElement("span");
    dot.className = "ng-socket-dot";
    dot.dataset.socket = name;
    dot.dataset.kind = isOutput ? "output" : "input";
    dot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      beginConnectionDrag(doc, node.id, name, isOutput, e);
    });
    const label = document.createElement("span");
    label.textContent = name;
    row.appendChild(dot);
    row.appendChild(label);
    col.appendChild(row);
  }
  return col;
}

function renderNode(doc, node) {
  const el = document.createElement("div");
  el.className = "ng-node";
  el.style.left = `${node.x}px`;
  el.style.top = `${node.y}px`;
  el.dataset.nodeId = node.id;

  const title = document.createElement("div");
  title.className = "ng-node-title";
  title.textContent = node.label;
  title.addEventListener("dblclick", (e) => {
    e.stopPropagation();
    beginRenameNode(doc, node);
  });
  title.addEventListener("pointerdown", (e) => beginNodeDrag(doc, node, el, e));
  title.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    selectNode(doc, node.id);
    openNodeMenu(doc, node, e.clientX, e.clientY);
  });
  el.appendChild(title);

  const body = document.createElement("div");
  body.className = "ng-node-body";
  body.appendChild(renderSocketColumn(doc, node, node.inputs, false));
  body.appendChild(renderSocketColumn(doc, node, node.outputs, true));
  el.appendChild(body);

  el.addEventListener("pointerdown", (e) => {
    e.stopPropagation();
    selectNode(doc, node.id);
  });

  doc.canvasEl.appendChild(el);
  doc.nodeEls.set(node.id, el);
}

function renderAll(doc) {
  doc.nodeEls.forEach((el) => el.remove());
  doc.nodeEls.clear();
  for (const node of doc.graph.nodes) renderNode(doc, node);
  renderConnections(doc);
}

// ---------- Selection ----------

function clearSelection(doc) {
  if (doc.selectedNode) doc.nodeEls.get(doc.selectedNode)?.classList.remove("is-selected");
  doc.selectedNode = null;
  doc.selectedConnection = null;
}

function selectNode(doc, nodeId) {
  clearSelection(doc);
  doc.selectedNode = nodeId;
  doc.nodeEls.get(nodeId)?.classList.add("is-selected");
  renderConnections(doc);
}

function selectConnection(doc, index) {
  clearSelection(doc);
  doc.selectedConnection = index;
  renderConnections(doc);
}

function deleteSelected(doc) {
  if (doc.selectedNode) {
    const id = doc.selectedNode;
    doc.graph.nodes = doc.graph.nodes.filter((n) => n.id !== id);
    doc.graph.connections = doc.graph.connections.filter((c) => {
      const from = parseEndpoint(c.from);
      const to = parseEndpoint(c.to);
      return from?.nodeId !== id && to?.nodeId !== id;
    });
    doc.nodeEls.get(id)?.remove();
    doc.nodeEls.delete(id);
    clearSelection(doc);
    graphChanged(doc);
  } else if (doc.selectedConnection !== null) {
    doc.graph.connections.splice(doc.selectedConnection, 1);
    clearSelection(doc);
    graphChanged(doc);
  }
}

// ---------- Add / rename nodes ----------

function addNode(doc, x, y) {
  const node = { id: newNodeId(doc), label: "New Node", x: Math.round(x), y: Math.round(y), inputs: ["in"], outputs: ["out"] };
  doc.graph.nodes.push(node);
  renderNode(doc, node);
  selectNode(doc, node.id);
  graphChanged(doc);
}

function beginRenameNode(doc, node) {
  const el = doc.nodeEls.get(node.id);
  const title = el.querySelector(".ng-node-title");
  const input = document.createElement("input");
  input.className = "ng-node-title-input";
  input.value = node.label;
  title.textContent = "";
  title.appendChild(input);
  input.focus();
  input.select();

  const commit = () => {
    const value = input.value.trim();
    if (value && value !== node.label) {
      node.label = value;
      graphChanged(doc);
    }
    title.textContent = node.label;
  };
  input.addEventListener("blur", commit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") input.blur();
    else if (e.key === "Escape") {
      input.value = node.label;
      input.blur();
    }
  });
  input.addEventListener("pointerdown", (e) => e.stopPropagation());
}

// ---------- Context menus (host's own floating-menu, via window.lowarc.showMenu) ----------

function openCanvasMenu(doc, x, y, canvasX, canvasY) {
  window.lowarc.showMenu([{ label: "Add Node", value: "add" }], x, y).then((value) => {
    if (value === "add") addNode(doc, canvasX, canvasY);
  });
}

function openNodeMenu(doc, node, x, y) {
  window.lowarc.showMenu(
    [
      { label: "Rename", value: "rename" },
      { label: "Add Input", value: "add-input" },
      { label: "Add Output", value: "add-output" },
      { divider: true },
      { label: "Delete Node", value: "delete" },
    ],
    x,
    y
  ).then((value) => {
    if (value === "rename") beginRenameNode(doc, node);
    else if (value === "add-input") {
      node.inputs.push(`in${node.inputs.length + 1}`);
      rebuildNode(doc, node);
      graphChanged(doc);
    } else if (value === "add-output") {
      node.outputs.push(`out${node.outputs.length + 1}`);
      rebuildNode(doc, node);
      graphChanged(doc);
    } else if (value === "delete") {
      selectNode(doc, node.id);
      deleteSelected(doc);
    }
  });
}

// Re-renders one node in place (used after adding a socket, where the row layout changes) —
// simpler and less error-prone than trying to patch the DOM incrementally for a rare edit.
function rebuildNode(doc, node) {
  doc.nodeEls.get(node.id)?.remove();
  doc.nodeEls.delete(node.id);
  renderNode(doc, node);
  if (doc.selectedNode === node.id) doc.nodeEls.get(node.id)?.classList.add("is-selected");
}

function openConnectionMenu(doc, index, x, y) {
  window.lowarc.showMenu([{ label: "Delete Connection", value: "delete" }], x, y).then((value) => {
    if (value === "delete") {
      selectConnection(doc, index);
      deleteSelected(doc);
    }
  });
}

// ---------- Dragging: pan / node move / connection draw ----------

function beginNodeDrag(doc, node, el, e) {
  if (e.button !== 0) return;
  e.stopPropagation();
  selectNode(doc, node.id);
  el.classList.add("is-dragging");
  const startX = e.clientX;
  const startY = e.clientY;
  const startLeft = node.x;
  const startTop = node.y;
  let moved = false;

  const onMove = (ev) => {
    moved = true;
    node.x = startLeft + (ev.clientX - startX);
    node.y = startTop + (ev.clientY - startY);
    el.style.left = `${node.x}px`;
    el.style.top = `${node.y}px`;
    renderConnections(doc);
  };
  const onUp = () => {
    el.classList.remove("is-dragging");
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    if (moved) graphChanged(doc);
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
}

function beginConnectionDrag(doc, nodeId, socket, isOutput, e) {
  e.preventDefault();
  const startDot = socketDot(doc, nodeId, socket, isOutput);
  const start = socketCenter(doc, startDot);

  const dragPath = document.createElementNS("http://www.w3.org/2000/svg", "path");
  dragPath.setAttribute("class", "ng-drag-line");
  doc.svgEl.appendChild(dragPath);

  const canvasRect = () => doc.canvasEl.getBoundingClientRect();

  const onMove = (ev) => {
    const rect = canvasRect();
    const p2 = { x: ev.clientX - rect.left, y: ev.clientY - rect.top };
    dragPath.setAttribute("d", bezierPath(start, p2));
  };
  const onUp = (ev) => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    dragPath.remove();

    const target = document.elementFromPoint(ev.clientX, ev.clientY);
    const targetDot = target && target.classList.contains("ng-socket-dot") ? target : null;
    if (targetDot) {
      const targetIsOutput = targetDot.dataset.kind === "output";
      const targetNodeEl = targetDot.closest(".ng-node");
      const targetNodeId = targetNodeEl && targetNodeEl.dataset.nodeId;
      if (targetNodeId && targetIsOutput !== isOutput) {
        const outEndpoint = isOutput ? `${nodeId}:${socket}` : `${targetNodeId}:${targetDot.dataset.socket}`;
        const inEndpoint = isOutput ? `${targetNodeId}:${targetDot.dataset.socket}` : `${nodeId}:${socket}`;
        // Replaces any existing connection into that input — an input only ever has one source,
        // matching how every node-graph tool handles a socket that's already wired.
        doc.graph.connections = doc.graph.connections.filter((c) => c.to !== inEndpoint);
        doc.graph.connections.push({ from: outEndpoint, to: inEndpoint });
        graphChanged(doc);
      }
    }
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
}

function beginPan(doc, e) {
  if (e.button !== 0) return;
  clearSelection(doc);
  renderConnections(doc);
  doc.viewportEl.classList.add("is-panning");
  const startX = e.clientX;
  const startY = e.clientY;
  const startLeft = doc.viewportEl.scrollLeft;
  const startTop = doc.viewportEl.scrollTop;

  const onMove = (ev) => {
    doc.viewportEl.scrollLeft = startLeft - (ev.clientX - startX);
    doc.viewportEl.scrollTop = startTop - (ev.clientY - startY);
  };
  const onUp = () => {
    doc.viewportEl.classList.remove("is-panning");
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
}

// ---------- Multi-document contract ----------

window.lowarc.on("lowarc:openFile", (payload) => {
  if (!payload || typeof payload.path !== "string" || docs.has(payload.path)) return;

  const wrap = document.createElement("div");
  wrap.className = "ng-doc";

  const toolbar = document.createElement("div");
  toolbar.className = "ng-toolbar";
  const addBtn = document.createElement("button");
  addBtn.type = "button";
  addBtn.className = "btn btn-ghost btn-sm";
  addBtn.textContent = "+ Add Node";
  toolbar.appendChild(addBtn);
  wrap.appendChild(toolbar);

  const viewport = document.createElement("div");
  viewport.className = "ng-viewport";
  const canvas = document.createElement("div");
  canvas.className = "ng-canvas";
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ng-connections");
  canvas.appendChild(svg);
  viewport.appendChild(canvas);
  wrap.appendChild(viewport);

  container.appendChild(wrap);

  const doc = {
    path: payload.path,
    graph: parseGraph(payload.contents),
    dirty: false,
    nextId: 0,
    wrapEl: wrap,
    viewportEl: viewport,
    canvasEl: canvas,
    svgEl: svg,
    nodeEls: new Map(),
    selectedNode: null,
    selectedConnection: null,
  };
  // nextId starts past every existing numeric-looking id ("n7" -> 7) so a freshly added node in a
  // loaded file never collides with one already on disk.
  for (const n of doc.graph.nodes) {
    const m = /^n(\d+)$/.exec(n.id || "");
    if (m) doc.nextId = Math.max(doc.nextId, Number(m[1]));
  }
  docs.set(payload.path, doc);

  addBtn.addEventListener("click", () => addNode(doc, viewport.scrollLeft + 80, viewport.scrollTop + 80));
  canvas.addEventListener("pointerdown", (e) => {
    if (e.target !== canvas && e.target !== svg) return;
    beginPan(doc, e);
  });
  canvas.addEventListener("contextmenu", (e) => {
    if (e.target !== canvas && e.target !== svg) return;
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    openCanvasMenu(doc, e.clientX, e.clientY, e.clientX - rect.left, e.clientY - rect.top);
  });

  renderAll(doc);
});

window.lowarc.on("lowarc:activateFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  for (const d of docs.values()) d.wrapEl.classList.remove("is-active");
  doc.wrapEl.classList.add("is-active");
  activePath = payload.path;
  renderConnections(doc); // socket positions can only be measured once actually laid out/visible
});

window.lowarc.on("lowarc:closeFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  docs.delete(payload.path);
  doc.wrapEl.remove();
  if (activePath === payload.path) activePath = null;
});

// The host's one HOST-initiated request (see requestPluginContent() in editor.html, used when
// moving a file to the other editor group) — replies with this instance's actual current graph
// for that path, unsaved edits included, same contract Monaco's own lowarc:getContent uses.
window.lowarc.on("lowarc:getContent", (payload) => {
  if (!payload) return;
  const doc = docs.get(payload.path);
  window.parent.postMessage({ type: "hostRequestReply", replyId: payload.replyId, content: doc ? serializeGraph(doc.graph) : null }, "*");
});

window.addEventListener("keydown", (e) => {
  if (e.key !== "Delete" && e.key !== "Backspace") return;
  if (document.activeElement && document.activeElement.tagName === "INPUT") return; // renaming a node
  const doc = activeDoc();
  if (doc && (doc.selectedNode || doc.selectedConnection !== null)) {
    e.preventDefault();
    deleteSelected(doc);
  }
});
