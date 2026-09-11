// Multi-document, same "one instance, several open files" shape Monaco and Terminal already use.
// See monaco.js's own header comment for the reasoning. Nothing here is editable: no dirty
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
  ".ogv": "video/ogg",
  ".mp3": "audio/mpeg",
  ".wav": "audio/wav",
  ".flac": "audio/flac",
  ".m4a": "audio/mp4",
  ".aac": "audio/aac",
  ".opus": "audio/opus",
  // .ogg is treated as audio, not video: a plain .ogg is overwhelmingly Vorbis audio in practice,
  // since Ogg video almost always uses .ogv instead.
  ".ogg": "audio/ogg",
};

const VIDEO_EXTS = new Set([".mp4", ".webm", ".ogv"]);
const AUDIO_EXTS = new Set([".mp3", ".wav", ".flac", ".m4a", ".aac", ".opus", ".ogg"]);

// A simple two-note glyph shown behind the visualizer. Audio has no picture of its own the way
// video has a first frame, so this is what fills that same visual space instead of a blank pane.
const AUDIO_ICON_SVG =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">' +
  '<path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>';

const MIN_ZOOM = 10;
const MAX_ZOOM = 800;
const ZOOM_STEP = 25;

// ---------- Embedded album art (ID3v2 only, for now) ----------
// No metadata library, since this iframe is sandboxed and pulling one in would mean vendoring a
// dependency for a small, well-documented binary format. ID3v2 covers the overwhelming majority of
// real files with cover art. FLAC's METADATA_BLOCK_PICTURE and MP4's covr atom are different
// formats and show the plain icon instead; adding them is more of the same work, not a redesign.
//
// Decodes base64 to bytes, then walks the ID3v2 header looking for an APIC frame. Returns
// {mime, bytes} or null. Anything malformed returns null rather than throwing, since this is a
// nice-to-have that must never break opening the file.
function base64ToBytes(base64) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

// ID3v2's own "syncsafe" integer: 4 bytes, each holding only 7 real bits (the high bit is always
// 0, so a byte that looks like a sync frame marker can never appear inside a size field by
// accident): used for the tag's own overall size always, and for each frame's size too under
// ID3v2.4 specifically (2.3 uses a plain 4-byte big-endian size for frames instead, see below).
function readSyncsafeInt(bytes, offset) {
  return ((bytes[offset] & 0x7f) << 21) | ((bytes[offset + 1] & 0x7f) << 14) | ((bytes[offset + 2] & 0x7f) << 7) | (bytes[offset + 3] & 0x7f);
}

function readUint32BE(bytes, offset) {
  return (bytes[offset] << 24) | (bytes[offset + 1] << 16) | (bytes[offset + 2] << 8) | bytes[offset + 3];
}

function readAsciiRun(bytes, offset, length) {
  let s = "";
  for (let i = 0; i < length; i++) s += String.fromCharCode(bytes[offset + i]);
  return s;
}

// Finds where a frame's null-terminated string ends, honoring the frame's own declared text
// encoding: encoding 0 (ISO-8859-1) and 3 (UTF-8) terminate on a single 0x00 byte; 1 (UTF-16
// with BOM) and 2 (UTF-16BE) terminate on a 0x00 0x00 PAIR, since a lone 0x00 is a perfectly valid
// byte inside a UTF-16 code unit. Getting this wrong walks straight into the middle of the image
// bytes that follow, corrupting everything after it: this is the one part of APIC parsing that's
// actually easy to get subtly wrong.
function findStringEnd(bytes, offset, textEncoding) {
  const wide = textEncoding === 1 || textEncoding === 2;
  for (let i = offset; i < bytes.length - (wide ? 1 : 0); i += wide ? 2 : 1) {
    if (bytes[i] === 0 && (!wide || bytes[i + 1] === 0)) return i + (wide ? 2 : 1);
  }
  return bytes.length;
}

function parseId3v2AlbumArt(bytes) {
  if (bytes.length < 10 || readAsciiRun(bytes, 0, 3) !== "ID3") return null;
  const majorVersion = bytes[3];
  const tagSize = readSyncsafeInt(bytes, 6);
  const tagEnd = Math.min(bytes.length, 10 + tagSize);

  let offset = 10;
  while (offset + 10 <= tagEnd) {
    const frameId = readAsciiRun(bytes, offset, 4);
    if (!/^[A-Z0-9]{4}$/.test(frameId)) break; // padding or corruption: nothing more to read
    // ID3v2.4 frame sizes are syncsafe like the tag header; 2.3 (and earlier 2.2, not handled
    // here — 2.2 uses 3-character frame ids and a different layout entirely) uses a plain
    // big-endian size instead. Getting this wrong misreads every frame after the first.
    const frameSize = majorVersion >= 4 ? readSyncsafeInt(bytes, offset + 4) : readUint32BE(bytes, offset + 4);
    const frameStart = offset + 10;
    const frameEnd = frameStart + frameSize;
    if (frameSize <= 0 || frameEnd > tagEnd) break;

    if (frameId === "APIC") {
      const textEncoding = bytes[frameStart];
      const mimeEnd = findStringEnd(bytes, frameStart + 1, 0); // MIME type is always ISO-8859-1/ASCII, regardless of textEncoding
      const mime = readAsciiRun(bytes, frameStart + 1, mimeEnd - (frameStart + 1) - 1) || "image/jpeg";
      const pictureType = bytes[mimeEnd]; // 3 = "Cover (front)": not filtered on, first APIC wins
      void pictureType;
      const descriptionEnd = findStringEnd(bytes, mimeEnd + 1, textEncoding);
      if (descriptionEnd < frameEnd) {
        return { mime, bytes: bytes.slice(descriptionEnd, frameEnd) };
      }
    }
    offset = frameEnd;
  }
  return null;
}

function bytesToBase64(bytes) {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

function extOf(path) {
  const name = path.split(/[\\/]/).pop() || path;
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

const container = document.getElementById("container");
const toolbar = document.getElementById("toolbar");
const zoomLabel = document.getElementById("zoom-label");

// path -> { wrap, kind: "image"|"video", el, fit: boolean, zoom: percent (only meaningful when
// !fit) }. `fit` starts true for every image: "shrink to whatever the window's own size is,
// never larger than the image's real size" is the natural default, same as most image viewers.
const docs = new Map();
let activePath = null;

window.lowarc.on("lowarc:openFile", (payload) => {
  if (!payload || typeof payload.path !== "string" || docs.has(payload.path)) return;
  const ext = extOf(payload.path);
  const mime = MIME_BY_EXT[ext] || "application/octet-stream";
  // payload.contents is already base64 (read_binary_file, host-side): a data: URI is the
  // simplest way to hand it to <img>/<video>/<audio> with no further decoding on this end. Fine
  // at the scale of "a person's own project assets"; a truly huge file would be better served as
  // a blob: object URL instead, not attempted here since nothing's shown that need yet.
  const src = `data:${mime};base64,${payload.contents}`;
  const kind = VIDEO_EXTS.has(ext) ? "video" : AUDIO_EXTS.has(ext) ? "audio" : "image";

  const wrap = document.createElement("div");
  wrap.className = `media-item is-fit is-${kind}`;

  let el;
  if (kind === "video") {
    el = document.createElement("video");
    el.src = src;
    el.controls = true;
    wrap.appendChild(el);
  } else if (kind === "audio") {
    // Same HTMLMediaElement API/native control bar as <video> above: play/pause/seek/volume all
    // come for free, nothing hand-rolled here just because there's no picture to show alongside.
    const visual = document.createElement("div");
    visual.className = "audio-visual";
    visual.innerHTML = AUDIO_ICON_SVG;
    const canvas = document.createElement("canvas");
    canvas.className = "audio-canvas";
    visual.appendChild(canvas);

    // Cheap and safe to always attempt. ParseId3v2AlbumArt returns null immediately for anything
    // that doesn't start with an ID3v2 tag at all, not just files that happen to be .mp3.
    const art = parseId3v2AlbumArt(base64ToBytes(payload.contents));
    if (art) {
      const thumb = document.createElement("img");
      thumb.className = "audio-thumb";
      thumb.src = `data:${art.mime};base64,${bytesToBase64(art.bytes)}`;
      thumb.alt = "";
      visual.appendChild(thumb);
    }

    wrap.appendChild(visual);

    el = document.createElement("audio");
    el.src = src;
    el.controls = true;
    el.className = "audio-controls";
    wrap.appendChild(el);
  } else {
    el = document.createElement("img");
    el.src = src;
    el.alt = payload.path;
    wrap.appendChild(el);
  }
  container.appendChild(wrap);

  const doc = { wrap, kind, el, fit: true, zoom: 100 };
  docs.set(payload.path, doc);
  if (kind === "image") initDrag(doc);
  if (kind === "audio") initVisualizer(doc);
});

window.lowarc.on("lowarc:activateFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  for (const d of docs.values()) {
    d.wrap.classList.remove("is-active");
    if (d.visualizer) d.visualizer.onDeactivate();
  }
  doc.wrap.classList.add("is-active");
  activePath = payload.path;
  updateToolbar(doc);
  if (doc.visualizer) doc.visualizer.onActivate();
});

window.lowarc.on("lowarc:closeFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  docs.delete(payload.path);
  if (doc.visualizer) doc.visualizer.destroy();
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
  // at all. naturalWidth/Height being 0 (image hasn't finished decoding yet) just no-ops here:
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
// not fight that): the browser's own page-zoom shortcut is Ctrl+wheel too, hence preventDefault.
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
// Only relevant once an image is actually bigger than its viewport (zoomed in, not fit): plain
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

// ---------- Audio visualizer ----------
// A live frequency-bar display driven by the Web Audio API, tapped straight off the <audio>
// element's own output: createMediaElementSource() reroutes playback through this graph, so the
// analyser has to stay connected all the way to destination or the file would go silent while
// open here, not just skip the visual.
//
// Only actually drawing (the rAF loop) while this doc is both the active tab AND actually
// playing: an audio view sitting in a background tab, or one that's paused, has no reason to
// keep spending CPU on a canvas nobody's looking at. Switching tabs away does NOT pause playback
// itself (same as video already doesn't), only the drawing.
function initVisualizer(doc) {
  const canvas = doc.wrap.querySelector(".audio-canvas");
  const ctx2d = canvas.getContext("2d");
  let audioCtx = null;
  let analyser = null;
  let dataArray = null;
  let rafId = null;

  function ensureGraph() {
    if (audioCtx) return;
    const AudioContextCtor = window.AudioContext || window.webkitAudioContext;
    audioCtx = new AudioContextCtor();
    const source = audioCtx.createMediaElementSource(doc.el);
    analyser = audioCtx.createAnalyser();
    analyser.fftSize = 128;
    dataArray = new Uint8Array(analyser.frequencyBinCount);
    source.connect(analyser);
    analyser.connect(audioCtx.destination);
  }

  function resizeCanvas() {
    const rect = canvas.getBoundingClientRect();
    canvas.width = Math.max(1, Math.round(rect.width * window.devicePixelRatio));
    canvas.height = Math.max(1, Math.round(rect.height * window.devicePixelRatio));
  }

  function draw() {
    analyser.getByteFrequencyData(dataArray);
    const w = canvas.width;
    const h = canvas.height;
    ctx2d.clearRect(0, 0, w, h);
    const barCount = dataArray.length;
    const gap = w * 0.006;
    const barWidth = (w - gap * (barCount - 1)) / barCount;
    ctx2d.fillStyle = getComputedStyle(document.documentElement).getPropertyValue("--cyan").trim() || "#4dd0e1";
    for (let i = 0; i < barCount; i++) {
      const barHeight = Math.max(h * 0.01, (dataArray[i] / 255) * h);
      ctx2d.fillRect(i * (barWidth + gap), h - barHeight, barWidth, barHeight);
    }
    rafId = requestAnimationFrame(draw);
  }

  function start() {
    if (rafId !== null) return;
    ensureGraph();
    if (audioCtx.state === "suspended") audioCtx.resume();
    resizeCanvas();
    draw();
  }

  function stop() {
    if (rafId === null) return;
    cancelAnimationFrame(rafId);
    rafId = null;
    ctx2d.clearRect(0, 0, canvas.width, canvas.height);
  }

  doc.el.addEventListener("play", () => {
    if (doc.wrap.classList.contains("is-active")) start();
  });
  doc.el.addEventListener("pause", stop);
  doc.el.addEventListener("ended", stop);
  window.addEventListener("resize", () => {
    if (rafId !== null) resizeCanvas();
  });

  doc.visualizer = {
    onActivate() {
      if (!doc.el.paused) start();
    },
    onDeactivate: stop,
    destroy() {
      stop();
      if (audioCtx) audioCtx.close();
    },
  };
}
