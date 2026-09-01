// Multi-document, same "one instance, several open files" shape Monaco/media-viewer already use —
// see monaco.js's own header comment for the reasoning.
//
// File format (.lan — "LowArc Node", plain JSON, schema locked 2026-08-29):
//   { lowarcNodeGraph: 1,
//     nodes: [{id, label, x, y, files: [path...], hasInput, hasOutput}],
//     connections: [{from: nodeId, to: nodeId}] }
// A node has AT MOST one input and one output (booleans, not a named/listed thing) — a "port"
// here never had a type or validation anyway, so there was nothing gained by allowing several.
// Fan-in and fan-out are both allowed: any number of wires can touch one node's single input or
// single output. This is a pure editing surface: nothing here feeds into the runtime or a
// compiler, it only draws and saves a graph. Wiring it into actual execution is a separate, much
// later task, and belongs to whichever module a project writes to consume its own graphs.
//
// A node's label/files/hasInput/hasOutput are edited through the Inspector (see inspector.js),
// not on the canvas — clicking a node (without dragging it) asks the host to open it there. The
// canvas and the Inspector are two separate iframes of this same plugin; edits made in the
// Inspector reach this file via window.lowarc.broadcastToSelf (lowarc:nodeEdit/lowarc:createNode),
// since there's no other channel for one plugin's iframe to reach another.

const container = document.getElementById("container");
const docs = new Map(); // path -> Doc
let activePath = null;

// This plugin's own configured default (Settings > Plugins > Node Graph, see plugin.json's
// `settings` declaration). getSettings() is async, so nodes can (and do) paint once already, using
// this optimistic default, before the real stored value comes back — without the refresh below,
// that first paint would just sit there until the next edit happened to call renderBadges() again,
// which looks exactly like "badges vanish the moment I touch a node" even though the node edit
// itself has nothing to do with it. Refreshing every already-open doc right when settings resolve
// makes the correction happen immediately instead of appearing to piggyback on whatever the user
// does next.
let showBadges = true;
window.lowarc.getSettings().then((settings) => {
  showBadges = !(settings && settings.showBadges === "false");
  for (const doc of docs.values()) {
    if (!doc.graph) continue;
    renderConnections(doc);
    refreshAllBadges(doc);
  }
});

function activeDoc() {
  return activePath ? docs.get(activePath) : null;
}

function docForPath(path) {
  return docs.get(path) || null;
}

function newNodeId(doc) {
  doc.nextId += 1;
  return `n${doc.nextId}`;
}

// A brand-new file is never truly empty — same reasoning a new document in most tools starts with
// something rather than a blank canvas nobody can right-click their way out of confidently. This
// is what a freshly created, still-zero-byte .lan file (e.g. from File Explorer's own "New File")
// turns into the moment it's opened here.
function starterGraph() {
  return { nodes: [{ id: "n1", label: "New Node", x: 120, y: 80, files: [], hasInput: true, hasOutput: true }], connections: [] };
}

// Returns { graph } on success or { error } on failure — never silently substitutes an empty
// graph for genuinely invalid content, since that would hide a real problem (a corrupted file, a
// bad hand-edit) behind what looks like a normal blank file.
function parseGraph(text) {
  if (!text || !text.trim()) return { graph: starterGraph() };
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (err) {
    return { error: "This file isn't valid JSON." };
  }
  if (!parsed || typeof parsed !== "object" || parsed.lowarcNodeGraph !== 1) {
    return { error: "This file isn't a LowArc node graph." };
  }
  return {
    graph: {
      nodes: Array.isArray(parsed.nodes) ? parsed.nodes : [],
      connections: Array.isArray(parsed.connections) ? parsed.connections : [],
    },
  };
}

function serializeGraph(graph) {
  // Only the fields this format actually understands — never round-trips whatever else might
  // have been sitting in a hand-edited file's node/connection objects.
  const nodes = graph.nodes.map((n) => ({
    id: n.id,
    label: n.label,
    x: n.x,
    y: n.y,
    files: Array.isArray(n.files) ? n.files : [],
    hasInput: Boolean(n.hasInput),
    hasOutput: Boolean(n.hasOutput),
  }));
  const connections = graph.connections.map((c) => ({ from: c.from, to: c.to }));
  return JSON.stringify({ lowarcNodeGraph: 1, nodes, connections }, null, 2) + "\n";
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
  refreshAllBadges(doc);
}

// ---------- Save ----------

let saving = false;
async function save() {
  const doc = activeDoc();
  if (!doc || !doc.graph || saving) return;
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

// The dot is a small visual circle nested inside a much larger ".ng-socket" hit area (see
// index.html) — this returns the dot itself, since that's what socket positions are measured from
// and what "is-connected" styling applies to; the hit area is only ever addressed for its pointer
// events, in renderSocket() below.
function socketDot(doc, nodeId, isOutput) {
  const el = doc.nodeEls.get(nodeId);
  if (!el) return null;
  return el.querySelector(isOutput ? ".ng-socket.ng-out .ng-socket-dot" : ".ng-socket.ng-in .ng-socket-dot");
}

// Center of a socket dot, in .ng-canvas's own coordinate space (unaffected by scroll — connections
// stay correctly positioned while the user pans).
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

// ---------- Badges ----------
// Small, fixed-size, wordless indicators stuck on a node's corners (or a connection's midpoint) —
// how many scripts are attached, what kind of port this node has, how many wires touch each side.
// No label is ever drawn on the node itself; the actual number/meaning only shows in a tooltip on
// hover, same reasoning as any icon-only control elsewhere in this app. Controlled by this
// plugin's own "Show badges" setting (see plugin.json).

// Tooltips are the shared window.lowarc.initTooltips()/data-tooltip primitive (see __lowarc.js) —
// badges just set el.dataset.tooltip and the render passes below call initTooltips() once their
// batch of badges exists.

// A badge's visible size never changes with its content — "+99" (its widest possible value) has
// to fit exactly as well as a single "S" does — so counts cap out visually rather than ever
// growing the oval to fit a fourth digit.
function capCount(n) {
  return n >= 100 ? "+99" : String(n);
}

function connectionCounts(doc, nodeId) {
  let inCount = 0;
  let outCount = 0;
  for (const c of doc.graph.connections) {
    if (c.to === nodeId) inCount++;
    if (c.from === nodeId) outCount++;
  }
  return { inCount, outCount };
}

// S(tart) = output only, E(nd) = input only, B(ridge) = both. No badge at all if the node has
// neither — there's no "port type" to speak of for a node nothing can connect to. The tooltip is
// just the letter spelled out, same as any icon-only button's tooltip elsewhere in this app — not
// an explanation of what the letter means.
function portBadgeInfo(node) {
  if (node.hasInput && node.hasOutput) return { text: "B", tooltip: "Bridge" };
  if (node.hasOutput) return { text: "S", tooltip: "Start" };
  if (node.hasInput) return { text: "E", tooltip: "End" };
  return null;
}

function makeBadge(cls, text, tooltip) {
  const badge = document.createElement("span");
  badge.className = `ng-badge ${cls}`;
  badge.textContent = text;
  badge.dataset.tooltip = tooltip;
  return badge;
}

function renderBadges(doc, node, el) {
  el.querySelectorAll(".ng-badge").forEach((b) => b.remove());
  if (!showBadges) return;

  const filesCount = Array.isArray(node.files) ? node.files.length : 0;
  if (filesCount > 0) {
    el.appendChild(makeBadge("ng-badge-files", capCount(filesCount), `${filesCount} attached file${filesCount === 1 ? "" : "s"}`));
  }

  const portInfo = portBadgeInfo(node);
  if (portInfo) el.appendChild(makeBadge("ng-badge-porttype", portInfo.text, portInfo.tooltip));

  const { inCount, outCount } = connectionCounts(doc, node.id);
  if (node.hasInput) el.appendChild(makeBadge("ng-badge-inputs", capCount(inCount), `${inCount} incoming connection${inCount === 1 ? "" : "s"}`));
  if (node.hasOutput) el.appendChild(makeBadge("ng-badge-outputs", capCount(outCount), `${outCount} outgoing connection${outCount === 1 ? "" : "s"}`));
  window.lowarc.initTooltips(el);
}

const SVG_NS = "http://www.w3.org/2000/svg";
const BADGE_W = 22;
const BADGE_H = 15;

// A connection doesn't carry any metadata of its own yet (it's just {from, to}) — this format may
// never grow any, since a wire's actual meaning is entirely up to whatever module ends up
// consuming the graph. Rather than invent a number that isn't really there, the visible glyph is
// just a direction arrow; the genuinely useful bit (which node feeds which) is what the hover
// tooltip shows, in the endpoints' own labels. Sits directly on the line, at the curve's own
// midpoint (not just the average of its two ends, which would sit off the actual curve).
function connectionBadge(doc, lineEl, index) {
  const conn = doc.graph.connections[index];
  const length = lineEl.getTotalLength();
  const mid = lineEl.getPointAtLength(length / 2);

  const fo = document.createElementNS(SVG_NS, "foreignObject");
  fo.setAttribute("class", "ng-badge-fo");
  fo.setAttribute("x", mid.x - BADGE_W / 2);
  fo.setAttribute("y", mid.y - BADGE_H / 2);
  fo.setAttribute("width", BADGE_W);
  fo.setAttribute("height", BADGE_H);

  const badge = document.createElementNS("http://www.w3.org/1999/xhtml", "div");
  badge.setAttribute("class", "ng-badge ng-badge-connection");
  badge.textContent = ">";
  // The badge sits visually on top of the line's own (invisible, wide) hit stroke — without
  // forwarding these, hovering/clicking it would just miss the connection underneath entirely.
  badge.addEventListener("pointerdown", (e) => {
    e.stopPropagation();
    selectConnection(doc, index);
  });
  badge.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    selectConnection(doc, index);
    openConnectionMenu(doc, index, e.clientX, e.clientY);
  });
  const fromLabel = (findNode(doc, conn.from) || {}).label || "?";
  const toLabel = (findNode(doc, conn.to) || {}).label || "?";
  badge.dataset.tooltip = `${fromLabel} > ${toLabel}`;

  fo.appendChild(badge);
  return fo;
}

// Connection badges depend on data (which nodes' labels, how many wires) that changes independent
// of any single node being rebuilt, so this re-derives every node's badges from scratch — cheap
// enough at any graph size this format is meant for, and simpler than tracking exactly which two
// nodes a given connection change touched.
function refreshAllBadges(doc) {
  for (const node of doc.graph.nodes) {
    const el = doc.nodeEls.get(node.id);
    if (el) renderBadges(doc, node, el);
  }
}

// ---------- Rendering ----------

function renderConnections(doc) {
  doc.svgEl.innerHTML = "";
  doc.graph.connections.forEach((conn, index) => {
    const dotA = socketDot(doc, conn.from, true);
    const dotB = socketDot(doc, conn.to, false);
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

    if (showBadges) group.appendChild(connectionBadge(doc, line, index));
  });
  updateConnectedDots(doc);
  window.lowarc.initTooltips(doc.svgEl);
}

function updateConnectedDots(doc) {
  doc.nodeEls.forEach((el) => {
    el.querySelectorAll(".ng-socket-dot").forEach((dot) => dot.classList.remove("is-connected"));
  });
  for (const conn of doc.graph.connections) {
    socketDot(doc, conn.from, true)?.classList.add("is-connected");
    socketDot(doc, conn.to, false)?.classList.add("is-connected");
  }
}

// The hit area (".ng-socket") spans the node's full height at that edge — much easier to grab
// than the small visual dot alone would be — while the dot itself stays a small circle centered
// in it, so nothing looks different from before.
function renderSocket(doc, node, isOutput) {
  const wrap = document.createElement("span");
  wrap.className = `ng-socket ${isOutput ? "ng-out" : "ng-in"}`;
  const dot = document.createElement("span");
  dot.className = "ng-socket-dot";
  wrap.appendChild(dot);
  wrap.addEventListener("pointerdown", (e) => {
    e.stopPropagation();
    beginConnectionDrag(doc, node.id, isOutput, e);
  });
  return wrap;
}

function renderNode(doc, node) {
  const el = document.createElement("div");
  el.className = "ng-node";
  el.style.left = `${node.x}px`;
  el.style.top = `${node.y}px`;
  el.dataset.nodeId = node.id;
  const label = document.createElement("span");
  label.className = "ng-node-label";
  label.textContent = node.label;
  el.appendChild(label);

  if (node.hasInput) el.appendChild(renderSocket(doc, node, false));
  if (node.hasOutput) el.appendChild(renderSocket(doc, node, true));

  el.addEventListener("pointerdown", (e) => beginNodeDrag(doc, node, el, e));
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    selectNode(doc, node.id);
    openNodeMenu(doc, node, e.clientX, e.clientY);
  });

  doc.canvasEl.appendChild(el);
  doc.nodeEls.set(node.id, el);
  renderBadges(doc, node, el);
}

// Re-renders one node in place (used after the Inspector changes something that affects layout —
// label, files, hasInput/hasOutput) — simpler and less error-prone than patching the DOM
// incrementally for an edit that isn't on the hot path.
function rebuildNode(doc, node) {
  const wasSelected = doc.selectedNode === node.id;
  doc.nodeEls.get(node.id)?.remove();
  doc.nodeEls.delete(node.id);
  renderNode(doc, node);
  if (wasSelected) doc.nodeEls.get(node.id)?.classList.add("is-selected");
}

function renderAll(doc) {
  doc.nodeEls.forEach((el) => el.remove());
  doc.nodeEls.clear();
  for (const node of doc.graph.nodes) renderNode(doc, node);
  renderConnections(doc);
  // A freshly-opened doc's wrapper isn't necessarily laid out (real size, actually visible) yet at
  // this exact point — the group/iframe around it can still be settling its own box — so the
  // socket rects renderConnections just measured may have been zeroed, drawing nothing or garbage.
  // Re-measuring once more after the browser's next paint corrects for that without having to pin
  // down the exact reason the first measurement was too early; this was previously only ever fixed
  // by something else (a click, a tab switch) happening to call renderConnections again later.
  requestAnimationFrame(() => renderConnections(doc));
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
    doc.graph.connections = doc.graph.connections.filter((c) => c.from !== id && c.to !== id);
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

// ---------- Add nodes ----------

function addNode(doc, x, y) {
  const node = { id: newNodeId(doc), label: "New Node", x: Math.round(x), y: Math.round(y), files: [], hasInput: true, hasOutput: true };
  doc.graph.nodes.push(node);
  renderNode(doc, node);
  selectNode(doc, node.id);
  graphChanged(doc);
  return node;
}

// The node's own labeled fields (label, files, sockets, position) are ONLY ever edited through the
// Inspector — there's deliberately no rename-in-place on the canvas anymore. That used to exist as
// a dblclick handler here, but having two separate editors for the same field was worse than
// having one: this way there's exactly one place a node's identity gets changed.

// Which OTHER nodes this node's input/output currently connects to, by label — for the Inspector's
// read-only "Input"/"Output" lists. direction "in" = nodes feeding into this one; "out" = nodes
// this one feeds into.
function connectedNodes(doc, nodeId, direction) {
  const ids = doc.graph.connections
    .filter((c) => (direction === "in" ? c.to === nodeId : c.from === nodeId))
    .map((c) => (direction === "in" ? c.from : c.to));
  return ids.map((id) => ({ id, label: (findNode(doc, id) || {}).label || "" }));
}

function openInspectorForNode(doc, node, onlyIfOpen) {
  window.lowarc.openInspector(
    {
      path: doc.path,
      nodeId: node.id,
      label: node.label,
      x: node.x,
      y: node.y,
      files: node.files,
      hasInput: node.hasInput,
      hasOutput: node.hasOutput,
      inputs: connectedNodes(doc, node.id, "in"),
      outputs: connectedNodes(doc, node.id, "out"),
    },
    onlyIfOpen
  );
}

// Scrolls the node's own center point to the middle of the viewport — reads the node element's
// REAL layout size (offsetWidth/offsetHeight) rather than guessing, since a node's width isn't
// fixed (it shrink-wraps to its label).
function centerViewportOn(doc, node) {
  const el = doc.nodeEls.get(node.id);
  if (!el) return;
  const centerX = node.x + el.offsetWidth / 2;
  const centerY = node.y + el.offsetHeight / 2;
  doc.viewportEl.scrollLeft = centerX - doc.viewportEl.clientWidth / 2;
  doc.viewportEl.scrollTop = centerY - doc.viewportEl.clientHeight / 2;
}

// Jumping to a node from the Inspector's Input/Output list — select it, pan it into view, and
// (re)point the Inspector at it. Unlike a plain click on the canvas, this always forces the
// Inspector open: the request can only originate from inside the Inspector in the first place, so
// it's already open by definition.
function focusNode(doc, node) {
  selectNode(doc, node.id);
  centerViewportOn(doc, node);
  openInspectorForNode(doc, node);
}

// ---------- Context menus (host's own floating-menu, via window.lowarc.showMenu) ----------

function openCanvasMenu(doc, x, y, canvasX, canvasY) {
  window.lowarc.showMenu([{ label: "Add Node", value: "add" }], x, y).then((value) => {
    if (value === "add") addNode(doc, canvasX, canvasY);
  });
}

function openNodeMenu(doc, node, x, y) {
  window.lowarc.showMenu([{ label: "Delete Node", value: "delete" }], x, y).then((value) => {
    if (value === "delete") {
      selectNode(doc, node.id);
      deleteSelected(doc);
    }
  });
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

// A plain click always opens the Inspector on this node. An actual drag only REFOCUSES it — if
// the Inspector's already open (on this node or a different one), dragging switches it over to
// match; if the Inspector's closed, dragging leaves it closed rather than yanking it open.
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
    if (Math.abs(ev.clientX - startX) > 2 || Math.abs(ev.clientY - startY) > 2) moved = true;
    if (!moved) return;
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
    if (moved) {
      graphChanged(doc);
      openInspectorForNode(doc, node, true); // onlyIfOpen — refocus, don't force the panel open
    } else {
      openInspectorForNode(doc, node);
    }
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
}

function beginConnectionDrag(doc, nodeId, isOutput, e) {
  e.preventDefault();
  const startDot = socketDot(doc, nodeId, isOutput);
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

    // elementFromPoint lands on ".ng-socket" (the hit area), never ".ng-socket-dot" — the dot has
    // pointer-events:none precisely so it never wins the hit test over its own wrapper (see
    // renderSocket()), which means a plain ".ng-socket-dot" check here would never match anything.
    const target = document.elementFromPoint(ev.clientX, ev.clientY);
    const targetSocket = target && target.closest(".ng-socket");
    if (targetSocket) {
      const targetIsOutput = targetSocket.classList.contains("ng-out");
      const targetNodeEl = targetSocket.closest(".ng-node");
      const targetNodeId = targetNodeEl && targetNodeEl.dataset.nodeId;
      // Fan-in and fan-out are both fine — the only real rules are "opposite kinds" (an output
      // has to feed an input, not another output) and "not the same node" (a node feeding itself
      // isn't a meaningful connection for anything this format could mean).
      if (targetNodeId && targetNodeId !== nodeId && targetIsOutput !== isOutput) {
        const from = isOutput ? nodeId : targetNodeId;
        const to = isOutput ? targetNodeId : nodeId;
        const exists = doc.graph.connections.some((c) => c.from === from && c.to === to);
        if (!exists) {
          doc.graph.connections.push({ from, to });
          graphChanged(doc);
        }
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
  container.appendChild(wrap);

  const { graph, error } = parseGraph(payload.contents);
  if (error) {
    const errorEl = document.createElement("div");
    errorEl.className = "ng-error";
    errorEl.textContent = error;
    wrap.appendChild(errorEl);
    docs.set(payload.path, { path: payload.path, graph: null, dirty: false, wrapEl: wrap, nodeEls: new Map() });
    return;
  }

  const viewport = document.createElement("div");
  viewport.className = "ng-viewport";
  const canvas = document.createElement("div");
  canvas.className = "ng-canvas";
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ng-connections");
  canvas.appendChild(svg);
  viewport.appendChild(canvas);
  wrap.appendChild(viewport);

  const doc = {
    path: payload.path,
    graph,
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
  if (doc.graph) {
    renderConnections(doc); // socket positions can only be measured once actually laid out/visible
    requestAnimationFrame(() => renderConnections(doc)); // safety net — see renderAll()'s comment
  }
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
  window.parent.postMessage({ type: "hostRequestReply", replyId: payload.replyId, content: doc && doc.graph ? serializeGraph(doc.graph) : null }, "*");
});

// ---------- Edits arriving from this plugin's OWN Inspector iframe ----------
// The Inspector never touches graph data directly — it sends intents here via
// window.lowarc.broadcastToSelf, and this (the iframe that actually owns and renders the graph)
// applies them. path disambiguates which open doc/node the edit is about, since this one iframe
// can hold several open .lan files at once (one per group, or several tabs in one group).
window.lowarc.on("lowarc:nodeEdit", (payload) => {
  if (!payload || typeof payload.path !== "string" || typeof payload.nodeId !== "string") return;
  const doc = docForPath(payload.path);
  if (!doc || !doc.graph) return;
  const node = findNode(doc, payload.nodeId);
  if (!node || !payload.patch || typeof payload.patch !== "object") return;
  Object.assign(node, payload.patch);
  rebuildNode(doc, node);
  graphChanged(doc);
});

// A connected-node row in the Inspector's Input/Output list was clicked — jump the canvas to it.
window.lowarc.on("lowarc:focusNode", (payload) => {
  if (!payload || typeof payload.path !== "string" || typeof payload.nodeId !== "string") return;
  const doc = docForPath(payload.path);
  if (!doc || !doc.graph) return;
  const node = findNode(doc, payload.nodeId);
  if (!node) return;
  focusNode(doc, node);
});

// Hovering that same row — a lighter-weight "just show me where it is", no selection/panning.
window.lowarc.on("lowarc:highlightNode", (payload) => {
  if (!payload || typeof payload.path !== "string" || typeof payload.nodeId !== "string") return;
  const doc = docForPath(payload.path);
  if (!doc || !doc.graph) return;
  const el = doc.nodeEls.get(payload.nodeId);
  if (el) el.classList.toggle("is-hover-highlight", Boolean(payload.on));
});

window.lowarc.on("lowarc:createNode", (payload) => {
  if (!payload || typeof payload.path !== "string") return;
  const doc = docForPath(payload.path);
  if (!doc || !doc.graph) return;
  // Offset from the viewport's current scroll so a new node lands somewhere visible rather than
  // always stacking at a fixed canvas coordinate the user might have scrolled away from.
  const node = addNode(doc, doc.viewportEl.scrollLeft + 80, doc.viewportEl.scrollTop + 80);
  openInspectorForNode(doc, node);
});

window.addEventListener("keydown", (e) => {
  if (e.key !== "Delete" && e.key !== "Backspace") return;
  if (document.activeElement && document.activeElement.tagName === "INPUT") return; // typing somewhere, not selecting on the canvas
  const doc = activeDoc();
  if (doc && doc.graph && (doc.selectedNode || doc.selectedConnection !== null)) {
    e.preventDefault();
    deleteSelected(doc);
  }
});
