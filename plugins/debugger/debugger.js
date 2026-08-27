// Pure event-driven viewer + controller — no backend of its own (no "command" in plugin.json).
// Every action here (pause/resume/step/setBreakpoints) is a generic window.lowarc method any
// plugin could call, not something special-cased for this one; everything shown here arrives as a
// relayed dev-run-frame/dev-run-ended Tauri event, same broadcast mechanism as lowarc:fileStatus.

const DELETE_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>';

// The full, locally-held breakpoint list — always resent whole on any change (add or remove),
// matching set_breakpoints' own "replace everything" contract rather than tracking adds/removes
// as separate operations the backend would have to reconcile.
let breakpoints = [];
let paused = false;
// The frame a "generic" paused state (from lowarc:devRunState, no frame attached) can fall back
// to showing — set whenever a real frame trace arrives, cleared whenever the run ends.
let lastFrameIndex = null;

const traceLog = document.getElementById("trace-log");
const traceEmpty = document.getElementById("trace-empty");
const statusIndicator = document.getElementById("status-indicator");
const statusText = document.getElementById("status-text");
const breakpointList = document.getElementById("breakpoint-list");
const breakpointEmpty = document.getElementById("breakpoint-empty");
const kindSelect = document.getElementById("bp-kind");

function setStatus(text, isPaused) {
  statusText.textContent = text;
  statusIndicator.classList.toggle("is-paused", Boolean(isPaused));
}

function syncKindFields() {
  const kind = kindSelect.value;
  document.querySelectorAll("[data-fields-for]").forEach((el) => {
    el.classList.toggle("is-hidden", el.dataset.fieldsFor !== kind);
  });
}
kindSelect.addEventListener("change", syncKindFields);
syncKindFields();

function breakpointLabel(bp) {
  switch (bp.kind) {
    case "moduleStart":
      return `Module "${bp.module}" starts`;
    case "moduleError":
      return bp.module ? `Module "${bp.module}" errors` : "Any module errors";
    case "logLevel":
      return bp.module ? `[${bp.module}] logs ${bp.level}` : `Any module logs ${bp.level}`;
    case "frameCount":
      return `Frame ${bp.count}`;
    case "jsonMatch":
      return `[${bp.module}] ${bp.path} = ${JSON.stringify(bp.equals)}`;
    default:
      return bp.kind;
  }
}

function renderBreakpoints() {
  breakpointList.querySelectorAll(".breakpoint-item").forEach((el) => el.remove());
  breakpointEmpty.style.display = breakpoints.length ? "none" : "block";

  breakpoints.forEach((bp, index) => {
    const row = document.createElement("div");
    row.className = "breakpoint-item";

    const label = document.createElement("span");
    label.className = "breakpoint-item-label";
    label.textContent = breakpointLabel(bp);
    row.appendChild(label);

    const del = document.createElement("button");
    del.type = "button";
    del.className = "breakpoint-item-delete";
    del.innerHTML = DELETE_SVG;
    del.setAttribute("aria-label", "Remove breakpoint");
    del.addEventListener("click", () => {
      breakpoints.splice(index, 1);
      renderBreakpoints();
      window.lowarc.setBreakpoints(breakpoints);
    });
    row.appendChild(del);

    breakpointList.appendChild(row);
  });
}

document.getElementById("bp-add-btn").addEventListener("click", () => {
  const kind = kindSelect.value;
  let bp = null;

  if (kind === "moduleStart") {
    const module = document.getElementById("bp-module-start-name").value.trim();
    if (!module) return window.lowarc.notify("error", "Module name is required.");
    bp = { kind, module };
  } else if (kind === "moduleError") {
    const module = document.getElementById("bp-module-error-name").value.trim();
    bp = { kind, module: module || null };
  } else if (kind === "logLevel") {
    const level = document.getElementById("bp-log-level").value;
    const module = document.getElementById("bp-log-level-module").value.trim();
    bp = { kind, level, module: module || null };
  } else if (kind === "frameCount") {
    const count = Number(document.getElementById("bp-frame-count").value);
    if (!Number.isInteger(count) || count < 1) return window.lowarc.notify("error", "Frame number must be a positive whole number.");
    bp = { kind, count };
  } else if (kind === "jsonMatch") {
    const module = document.getElementById("bp-json-module").value.trim();
    const path = document.getElementById("bp-json-path").value.trim();
    const rawEquals = document.getElementById("bp-json-equals").value.trim();
    if (!module || !path || !rawEquals) return window.lowarc.notify("error", "Module, path, and equals are all required.");
    let equals;
    try {
      equals = JSON.parse(rawEquals);
    } catch (err) {
      return window.lowarc.notify("error", "Equals must be valid JSON.");
    }
    bp = { kind, module, path, equals };
  }

  if (!bp) return;
  breakpoints.push(bp);
  renderBreakpoints();
  window.lowarc.setBreakpoints(breakpoints);
});

// The plugin has no project access of its own (no window.__TAURI__, no filesystem) — the only way
// it ever learns a real module name is by seeing one arrive in an actual frame trace. Feeding those
// into the module-name fields' <datalist> turns "type a module name" from a guess into a pick,
// without needing a new host capability just to ask "what modules does this project have".
const knownModules = new Set();
const knownModulesList = document.getElementById("known-modules");

function rememberModuleName(name) {
  if (knownModules.has(name)) return;
  knownModules.add(name);
  const option = document.createElement("option");
  option.value = name;
  knownModulesList.appendChild(option);
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

function appendFrame(trace) {
  traceEmpty.style.display = "none";

  const details = document.createElement("details");
  details.className = "trace-entry";

  const summary = document.createElement("summary");
  const idx = document.createElement("span");
  idx.className = "frame-index";
  idx.textContent = `Frame #${trace.frameIndex}`;
  summary.appendChild(idx);

  const delta = document.createElement("span");
  delta.className = "frame-delta";
  delta.textContent = `Δ${(trace.deltaSeconds * 1000).toFixed(1)}ms`;
  summary.appendChild(delta);

  if (trace.triggered) {
    const triggered = document.createElement("span");
    triggered.className = "frame-triggered";
    triggered.textContent = `breakpoint: ${breakpointLabel(trace.triggered)}`;
    summary.appendChild(triggered);
  }
  details.appendChild(summary);

  (trace.modules || []).forEach((m) => {
    rememberModuleName(m.id);
    const pre = document.createElement("pre");
    pre.innerHTML =
      `<span class="module-name">${escapeHtml(m.id)}</span> ` +
      `<span class="module-duration">(${m.durationMs.toFixed(2)}ms)</span>\n` +
      `req: ${escapeHtml(JSON.stringify(m.request))}\n` +
      `reply: ${escapeHtml(JSON.stringify(m.reply))}`;
    details.appendChild(pre);
  });

  traceLog.appendChild(details);
  traceLog.scrollTop = traceLog.scrollHeight;
}

document.getElementById("clear-trace-btn").addEventListener("click", () => {
  traceLog.querySelectorAll(".trace-entry").forEach((el) => el.remove());
  traceEmpty.style.display = "block";
});

document.getElementById("pause-resume-btn").addEventListener("click", () => {
  window.lowarc[paused ? "resumeRun" : "pauseRun"]();
  // No local state flip here — lowarc:devRunState (below) is now the single authoritative source
  // for paused/running, pushed by the host whenever it actually changes for ANY reason, not just
  // this button. That's what makes the toolbar's own Pause button (which this plugin has no other
  // way to observe) show up here too.
});

document.getElementById("step-btn").addEventListener("click", () => {
  const count = Math.max(1, Number(document.getElementById("step-count").value) || 1);
  window.lowarc.stepRun(count);
});

// Every frame trace that reaches a plugin implies the run is currently paused — free-running ticks
// are never relayed at all (see spawn_and_run's own on_frame gate), so receiving one at all IS the
// "you're paused" signal, not just its content.
window.lowarc.on("lowarc:devRunFrame", (trace) => {
  paused = true;
  lastFrameIndex = trace.frameIndex;
  setStatus(`Paused: Frame #${trace.frameIndex}`, true);
  appendFrame(trace);
});

// The authoritative running/paused signal — pushed by the host on every actual state change,
// regardless of what caused it (this plugin's own buttons, the toolbar's Pause button, or Run
// itself starting). A "paused" here with no frame trace yet (e.g. paused via the toolbar, never
// stepped) falls back to the last frame this plugin actually saw, rather than showing nothing.
window.lowarc.on("lowarc:devRunState", (state) => {
  paused = Boolean(state.paused);
  if (!state.running) {
    setStatus("Idle", false);
  } else if (paused) {
    setStatus(lastFrameIndex !== null ? `Paused: Frame #${lastFrameIndex}` : "Paused", true);
  } else {
    setStatus("Running", false);
  }
});

window.lowarc.on("lowarc:devRunEnded", () => {
  paused = false;
  lastFrameIndex = null;
  setStatus("Idle", false);
});

renderBreakpoints();
