// Multi-document, same "one instance, several open files" shape Monaco and Terminal already use —
// see monaco.js's own header comment for the reasoning. Nothing here is editable: no dirty
// tracking, no saveFile, and no lowarc:getContent handler either — a media file is shown, not
// edited, so there's no unsaved state to ever ask this plugin for. moveFileToGroup() (editor.html)
// re-reads a binary viewer's file straight from disk when moving it between groups instead of
// asking the plugin, which is exactly as correct here as it would be asking this plugin, and
// simpler.

const MIME_BY_EXT = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".bmp": "image/bmp",
  ".ico": "image/x-icon",
  ".svg": "image/svg+xml",
  ".avif": "image/avif",
  ".mp4": "video/mp4",
  ".webm": "video/webm",
  ".ogg": "video/ogg",
  ".ogv": "video/ogg",
};

const VIDEO_EXTS = new Set([".mp4", ".webm", ".ogg", ".ogv"]);

const MIN_ZOOM = 10;
const MAX_ZOOM = 800;
const ZOOM_STEP = 25;

function extOf(path) {
  const name = path.split(/[\\/]/).pop() || path;
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

const container = document.getElementById("container");
const toolbar = document.getElementById("toolbar");
const zoomLabel = document.getElementById("zoom-label");

// path -> { wrap, kind: "image"|"video", el, fit: boolean, zoom: percent (only meaningful when
// !fit) }. `fit` starts true for every image — "shrink to whatever the window's own size is,
// never larger than the image's real size" is the natural default, same as most image viewers.
const docs = new Map();
let activePath = null;

window.lowarc.on("lowarc:openFile", (payload) => {
  if (!payload || typeof payload.path !== "string" || docs.has(payload.path)) return;
  const ext = extOf(payload.path);
  const mime = MIME_BY_EXT[ext] || "application/octet-stream";
  // payload.contents is already base64 (read_binary_file, host-side) — a data: URI is the
  // simplest way to hand it to <img>/<video> with no further decoding on this end. Fine at the
  // scale of "a person's own project assets"; a truly huge file would be better served as a
  // blob: object URL instead, not attempted here since nothing's shown that need yet.
  const src = `data:${mime};base64,${payload.contents}`;
  const isVideo = VIDEO_EXTS.has(ext);

  const wrap = document.createElement("div");
  wrap.className = "media-item is-fit";

  let el;
  if (isVideo) {
    el = document.createElement("video");
    el.src = src;
    el.controls = true;
  } else {
    el = document.createElement("img");
    el.src = src;
    el.alt = payload.path;
  }
  wrap.appendChild(el);
  container.appendChild(wrap);

  const doc = { wrap, kind: isVideo ? "video" : "image", el, fit: true, zoom: 100 };
  docs.set(payload.path, doc);
  if (!isVideo) initDrag(doc);
});

window.lowarc.on("lowarc:activateFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  for (const d of docs.values()) d.wrap.classList.remove("is-active");
  doc.wrap.classList.add("is-active");
  activePath = payload.path;
  updateToolbar(doc);
});

window.lowarc.on("lowarc:closeFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  docs.delete(payload.path);
  doc.wrap.remove();
  if (activePath === payload.path) activePath = null;
});

// ---------- Zoom ----------

function applyZoom(doc) {
  doc.wrap.classList.toggle("is-fit", doc.fit);
  doc.wrap.classList.toggle("is-draggable", doc.kind === "image" && !doc.fit);
  if (doc.fit) {
    doc.el.style.width = "";
    doc.el.style.height = "";
    return;
  }
  // Explicit pixel size, not a CSS transform: scale() — a transformed element's layout box
  // doesn't grow to match, so the scroll container above would have nothing bigger than itself
  // to actually scroll. Real width/height makes an enlarged image genuinely bigger than its
  // viewport, which is what makes the overflow:auto on .media-item (and drag-to-pan below) work
  // at all. naturalWidth/Height being 0 (image hasn't finished decoding yet) just no-ops here —
  // the next zoom click after it loads will apply correctly.
  const w = doc.el.naturalWidth;
  const h = doc.el.naturalHeight;
  if (!w || !h) return;
  doc.el.style.width = Math.round((w * doc.zoom) / 100) + "px";
  doc.el.style.height = Math.round((h * doc.zoom) / 100) + "px";
}

function updateToolbar(doc) {
  const showToolbar = Boolean(doc && doc.kind === "image");
  toolbar.classList.toggle("is-visible", showToolbar);
  if (showToolbar) zoomLabel.textContent = doc.fit ? "Fit" : `${doc.zoom}%`;
}

function setZoom(doc, zoom, fit) {
  doc.fit = fit;
  doc.zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.round(zoom)));
  applyZoom(doc);
  updateToolbar(doc);
}

function activeDoc() {
  return activePath ? docs.get(activePath) : null;
}

function currentZoom(doc) {
  return doc.fit ? 100 : doc.zoom;
}

document.getElementById("zoom-in").addEventListener("click", () => {
  const doc = activeDoc();
  if (doc) setZoom(doc, currentZoom(doc) + ZOOM_STEP, false);
});
document.getElementById("zoom-out").addEventListener("click", () => {
  const doc = activeDoc();
  if (doc) setZoom(doc, currentZoom(doc) - ZOOM_STEP, false);
});
document.getElementById("zoom-fit").addEventListener("click", () => {
  const doc = activeDoc();
  if (doc) setZoom(doc, 100, true);
});

// Ctrl+wheel to zoom, the same modifier convention every other app uses (a plain wheel already
// means "scroll the image" once it's bigger than the viewport, so zoom needs its own modifier to
// not fight that) — the browser's own page-zoom shortcut is Ctrl+wheel too, hence preventDefault.
container.addEventListener(
  "wheel",
  (e) => {
    if (!e.ctrlKey) return;
    const doc = activeDoc();
    if (!doc || doc.kind !== "image") return;
    e.preventDefault();
    setZoom(doc, currentZoom(doc) + (e.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP), false);
  },
  { passive: false },
);

// ---------- Drag to pan ----------
// Only relevant once an image is actually bigger than its viewport (zoomed in, not fit) — plain
// scrolling/scrollbars already work via .media-item's own overflow:auto regardless, this is an
// alternate way to do the same thing without reaching for the scrollbar or a trackpad gesture.
function initDrag(doc) {
  let dragging = false;
  let startX = 0;
  let startY = 0;
  let startLeft = 0;
  let startTop = 0;

  doc.wrap.addEventListener("pointerdown", (e) => {
    if (doc.fit || e.button !== 0) return;
    dragging = true;
    doc.wrap.setPointerCapture(e.pointerId);
    doc.wrap.classList.add("is-dragging");
    startX = e.clientX;
    startY = e.clientY;
    startLeft = doc.wrap.scrollLeft;
    startTop = doc.wrap.scrollTop;
  });
  doc.wrap.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    doc.wrap.scrollLeft = startLeft - (e.clientX - startX);
    doc.wrap.scrollTop = startTop - (e.clientY - startY);
  });
  const endDrag = (e) => {
    if (!dragging) return;
    dragging = false;
    doc.wrap.classList.remove("is-dragging");
    doc.wrap.releasePointerCapture(e.pointerId);
  };
  doc.wrap.addEventListener("pointerup", endDrag);
  doc.wrap.addEventListener("pointercancel", endDrag);
}
