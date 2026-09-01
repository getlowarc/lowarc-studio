// Multi-instance: this one iframe owns every open terminal tab, each a separate session (its own
// PTY/shell process — see plugin_session.rs) identified by a sessionId this file mints itself
// (crypto.randomUUID()). The host never knows "Terminal has instances" — it only knows session
// ids as opaque strings and relays lowarc:sessionOutput/lowarc:newTerminal by pluginId, same as
// any other emit. All the instance bookkeeping (the sidebar, which one's visible, numbering) is
// entirely this plugin's own business.

const DELETE_SVG = '<svg viewBox="0 0 16 16" fill="none"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>';

const sessions = new Map(); // sessionId -> { term, fitAddon, container, sidebarItem, label }
let activeSessionId = null;
let instanceCounter = 0;

// This plugin's own configured defaults (Settings > Plugins > Terminal, see plugin.json's
// `settings` declaration) — read once at load via window.lowarc.getSettings(), the generic
// per-plugin settings mechanism; like every setting read this way, a change while a terminal is
// already open takes effect on its next instance, not live. configuredShell is only used when an
// instance isn't given an explicit per-instance override (the console header's "..." menu); the
// host itself no longer knows or cares about any of these three values, unlike before shell was
// moved here.
let configuredShell = null;
let configuredFontSize = 13;
let configuredScrollback = 1000;

// Not just crypto.randomUUID() directly — this only needs to be unique within one running app
// instance, not cryptographically unguessable, and a sandboxed iframe without allow-same-origin
// is an untested enough environment for the Web Crypto API that a fallback is worth having rather
// than finding out live that instance creation silently breaks.
function makeSessionId() {
  if (window.crypto && typeof window.crypto.randomUUID === "function") {
    try {
      return window.crypto.randomUUID();
    } catch (err) {
      // fall through
    }
  }
  return `s-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

const mainEl = document.getElementById("terminal-main");
const sidebarEl = document.getElementById("terminal-sidebar");
const emptyEl = document.getElementById("terminal-empty");

function updateEmptyState() {
  emptyEl.classList.toggle("is-visible", sessions.size === 0);
}

function renderSidebar() {
  sidebarEl.innerHTML = "";
  for (const [sessionId, instance] of sessions) {
    const item = document.createElement("div");
    item.className = "terminal-sidebar-item" + (sessionId === activeSessionId ? " is-active" : "");

    const label = document.createElement("span");
    label.className = "terminal-sidebar-item-label";
    label.textContent = instance.label;
    item.appendChild(label);

    const del = document.createElement("button");
    del.type = "button";
    del.className = "btn btn-icon-only btn-ghost-danger btn-xs";
    del.innerHTML = DELETE_SVG;
    del.setAttribute("aria-label", `Close ${instance.label}`);
    del.addEventListener("click", (e) => {
      e.stopPropagation();
      closeInstance(sessionId);
    });
    item.appendChild(del);

    item.addEventListener("click", () => switchTo(sessionId));
    instance.sidebarItem = item;
    sidebarEl.appendChild(item);
  }
}

function sendResize(sessionId) {
  const instance = sessions.get(sessionId);
  if (instance) window.lowarc.session.send(sessionId, { type: "resize", cols: instance.term.cols, rows: instance.term.rows });
}

// A single rAF after making a container visible/creating a terminal wasn't always enough —
// confirmed live: xterm's own renderer sometimes hasn't settled real font-metric measurements
// that fast, so fit() would occasionally compute a much-too-narrow size (the classic symptom:
// text wrapping every 7-8 characters despite a wide container). Re-fitting again a beat later
// catches that case; it's a harmless no-op the rest of the time since fitting an
// already-correctly-sized terminal just proposes the same dimensions again.
function refit(sessionId) {
  const instance = sessions.get(sessionId);
  if (!instance) return;
  instance.fitAddon.fit();
  sendResize(sessionId);
}

function switchTo(sessionId) {
  if (!sessions.has(sessionId) || sessionId === activeSessionId) {
    if (sessions.has(sessionId)) {
      refit(sessionId);
      sessions.get(sessionId).term.focus();
    }
    return;
  }
  if (activeSessionId && sessions.has(activeSessionId)) {
    sessions.get(activeSessionId).container.classList.remove("is-active");
  }
  activeSessionId = sessionId;
  const instance = sessions.get(sessionId);
  instance.container.classList.add("is-active");
  renderSidebar();
  // The newly-shown container was display:none, so its size was never measurable until now.
  requestAnimationFrame(() => {
    refit(sessionId);
    instance.term.focus();
    setTimeout(() => refit(sessionId), 80);
  });
}

function createInstance(shell) {
  const sessionId = makeSessionId();
  instanceCounter += 1;

  const container = document.createElement("div");
  container.className = "terminal-instance";
  mainEl.appendChild(container);

  const term = new Terminal({
    convertEol: true,
    cursorBlink: true,
    fontSize: configuredFontSize,
    scrollback: configuredScrollback,
    fontFamily: "Consolas, 'Cascadia Mono', Menlo, monospace",
    theme: { background: "#1e1e1e", foreground: "#d4d4d4" },
  });
  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  term.open(container);

  term.onData((data) => window.lowarc.session.send(sessionId, { type: "input", data }));

  const resolvedShell = shell || configuredShell || null;
  sessions.set(sessionId, { term, fitAddon, container, sidebarItem: null, label: `${instanceCounter}: ${resolvedShell || "Default"}` });
  updateEmptyState();
  window.lowarc.session.start(sessionId, resolvedShell);
  switchTo(sessionId);
}

function closeInstance(sessionId) {
  const instance = sessions.get(sessionId);
  if (!instance) return;
  window.lowarc.session.stop(sessionId);
  instance.term.dispose();
  instance.container.remove();
  sessions.delete(sessionId);
  updateEmptyState();

  if (activeSessionId === sessionId) {
    activeSessionId = null;
    const next = sessions.keys().next().value;
    if (next) switchTo(next);
    else renderSidebar();
  } else {
    renderSidebar();
  }
}

window.lowarc.on("lowarc:sessionOutput", (payload) => {
  if (!payload || typeof payload.sessionId !== "string") return;
  const instance = sessions.get(payload.sessionId);
  if (!instance) return;
  if (payload.type === "output" && typeof payload.data === "string") {
    instance.term.write(payload.data);
  } else if (payload.type === "exit") {
    // Matches VS Code's default: a terminal whose shell exited on its own (the user typed
    // "exit", or the process just ended) closes its tab rather than sitting there inert.
    closeInstance(payload.sessionId);
  }
});

// The host's console-header "+"/"..." controls (see editor.html), and — since Command Palette
// entries below — the Command Palette's "Terminal: New Terminal" both ultimately call this one
// function, so there's exactly one code path for "open a terminal" regardless of which UI asked.
window.lowarc.on("lowarc:newTerminal", (payload) => {
  createInstance(payload && payload.shell ? payload.shell : null);
});

// This plugin's own Command Palette entries (declared in plugin.json's `commands`) — the host
// sends every command the same generic way (an emit carrying just the id back), so this is the
// one place that maps "new-terminal" onto what it actually means for this plugin.
window.lowarc.on("lowarc:runCommand", (payload) => {
  if (payload && payload.commandId === "new-terminal") createInstance(null);
});

new ResizeObserver(() => {
  if (activeSessionId) refit(activeSessionId);
}).observe(mainEl);

// The very first instance waits for the configured-shell fetch so it launches with the right
// default immediately, instead of starting on "Default" and only respecting the setting from the
// second instance on. A slow/failed fetch still can't hang this — getSettings() always resolves
// (falls back to {} on the host side), so this is a short real delay, never an indefinite one.
window.lowarc.getSettings().then((settings) => {
  configuredShell = (settings && settings.shell) || null;
  // Plugin settings are always stored/returned as plain strings (Settings.plugin_settings is a
  // HashMap<String, HashMap<String, String>> — no per-field type on the Rust side), so a "number"
  // field is this plugin's own job to parse and validate, same as CORE_SETTINGS_SCHEMA's FPS field
  // does on the host side for its own General setting.
  const fontSize = Number(settings && settings.fontSize);
  if (Number.isInteger(fontSize) && fontSize > 0) configuredFontSize = fontSize;
  const scrollback = Number(settings && settings.scrollback);
  if (Number.isInteger(scrollback) && scrollback >= 0) configuredScrollback = scrollback;
  createInstance(null);
});
