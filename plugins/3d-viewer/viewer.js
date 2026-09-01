// Multi-document, same "one instance, several open files" shape Monaco/Terminal/Media Viewer use
// — see media-viewer.js's own header comment. The difference here: those plugins give each open
// file its own DOM element and let the browser own it. A WebGL context isn't that cheap — browsers
// cap concurrent contexts (~16) and start evicting the oldest past that, which would silently break
// a project with several 3D tabs open. So there's exactly one <canvas>/WebGLRenderer for the whole
// plugin; each open file just gets its own {scene, camera, controls} record, and activateFile
// switches which one the shared renderer points at.
//
// Nothing here is editable — same reasoning as media-viewer.js: no dirty tracking, no saveFile,
// no lowarc:getContent. This is a viewer, not the scene editor (deliberately out of scope — a
// generic in-place 3D editor would need to understand and write back into whatever data model each
// stack's modules actually use, and there is no one such model to hardcode against).

// A bare "three" specifier would need an import map to resolve, and the plugin CSP's
// script-src 'self' (no 'unsafe-inline', no nonce) blocks inline <script> tags outright —
// including <script type="importmap">, which is inline by definition. Relative path instead,
// same reason the vendored loaders/controls under vendor/three/examples/jsm were patched to
// import three.module.js by relative path rather than by bare specifier.
import * as THREE from "./vendor/three/build/three.module.js";
import { GLTFLoader } from "./vendor/three/examples/jsm/loaders/GLTFLoader.js";
import { OBJLoader } from "./vendor/three/examples/jsm/loaders/OBJLoader.js";
import { OrbitControls } from "./vendor/three/examples/jsm/controls/OrbitControls.js";

function extOf(path) {
  const name = path.split(/[\\/]/).pop() || path;
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

const container = document.getElementById("container");
const canvas = document.getElementById("canvas");
const unavailableEl = document.getElementById("unavailable");
const loadErrorEl = document.getElementById("load-error");
const loadErrorText = document.getElementById("load-error-text");
const toolbar = document.getElementById("toolbar");
const softwareBadge = document.getElementById("software-badge");

// path -> { scene, camera, controls, root, hasGrid, gridHelper }
const docs = new Map();
let activePath = null;
let dirty = true; // on-demand: only re-render when something actually changed
let renderer = null;
let usingSoftware = false;

const gltfLoader = new GLTFLoader();
const objLoader = new OBJLoader();

// ---------- Renderer setup: GPU if available, a real "unavailable" state if not ----------
// There is no from-scratch CPU rasterizer fallback here on purpose. Three.js dropped its old
// CanvasRenderer/SoftwareRenderer years ago (unmaintained, no shader support, poor fidelity), and
// the OS already provides the actual "no GPU" fallback beneath us: when there's no real driver,
// Chrome/Firefox transparently hand WebGL to a software implementation (SwiftShader, llvmpipe)
// before any code here runs. So getContext() succeeding already means "GPU if present, software
// if not" — our job is just to (a) hint toward a real GPU when the hint helps, (b) notice, best
// effort, when we landed on software so the UI can say so instead of just feeling slow for no
// visible reason, and (c) not crash on the one case that isn't silently handled: WebGL blocked or
// disabled entirely, where getContext returns null.
function setupRenderer() {
  let ctx = null;
  try {
    ctx =
      canvas.getContext("webgl2", { antialias: true, powerPreference: "high-performance" }) ||
      canvas.getContext("webgl", { antialias: true, powerPreference: "high-performance" });
  } catch (e) {
    ctx = null;
  }

  if (!ctx) {
    unavailableEl.classList.add("is-visible");
    toolbar.style.display = "none";
    return null;
  }

  const r = new THREE.WebGLRenderer({ canvas, context: ctx, antialias: true });

  // Best-effort only — some browsers withhold WEBGL_debug_renderer_info for fingerprinting
  // reasons, in which case this just can't tell, not "definitely software".
  const dbg = ctx.getExtension("WEBGL_debug_renderer_info");
  if (dbg) {
    const unmasked = String(ctx.getParameter(dbg.UNMASKED_RENDERER_WEBGL) || "");
    usingSoftware = /swiftshader|llvmpipe|software|microsoft basic render/i.test(unmasked);
  }
  softwareBadge.classList.toggle("is-visible", usingSoftware);
  // A software rasterizer pays for every pixel — halving the effective resolution on a
  // detected-software path keeps it responsive instead of just quietly feeling slow.
  r.setPixelRatio(Math.min(window.devicePixelRatio || 1, usingSoftware ? 1 : 2));
  r.outputColorSpace = THREE.SRGBColorSpace;
  return r;
}

function addDefaultLighting(scene) {
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const key = new THREE.DirectionalLight(0xffffff, 1.2);
  key.position.set(3, 5, 4);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xffffff, 0.4);
  fill.position.set(-4, -2, -3);
  scene.add(fill);
}

function boundsOf(object) {
  const box = new THREE.Box3().setFromObject(object);
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  return { box, size, center };
}

function frameCameraToBounds(camera, controls, object) {
  const { size, center } = boundsOf(object);
  const radius = Math.max(size.length() * 0.5, 0.01);
  // The unpadded distance puts the bounding sphere exactly tangent to the frustum edges — correct
  // by construction, but for a non-spherical, elongated shape that reads as uncomfortably tight
  // (verified against a real model: it fills the frame edge to edge with no breathing room). 1.4x
  // is a standard "frame to fit" padding factor, not a magic number chosen to patch this one model.
  const distance = (radius / Math.sin((camera.fov * Math.PI) / 360)) * 1.4;

  camera.near = Math.max(distance / 100, 0.01);
  camera.far = distance * 100;
  camera.position.copy(center).add(new THREE.Vector3(1, 0.6, 1).normalize().multiplyScalar(distance));
  camera.updateProjectionMatrix();

  controls.target.copy(center);
  controls.update();
}

function aspect() {
  return Math.max(container.clientWidth, 1) / Math.max(container.clientHeight, 1);
}

function createDocShell() {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(50, aspect(), 0.01, 10000);
  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = true;
  controls.addEventListener("change", () => {
    dirty = true;
  });
  addDefaultLighting(scene);
  return { scene, camera, controls, root: null, hasGrid: false, gridHelper: null };
}

function disposeDoc(doc) {
  if (!doc.root) return;
  // Three.js doesn't garbage-collect GPU resources — geometries, materials, and textures need
  // an explicit .dispose() or every closed tab leaks VRAM for the rest of the session.
  doc.root.traverse((o) => {
    o.geometry?.dispose();
    const mats = Array.isArray(o.material) ? o.material : o.material ? [o.material] : [];
    for (const m of mats) {
      for (const key of ["map", "normalMap", "roughnessMap", "metalnessMap", "emissiveMap", "aoMap"]) {
        m[key]?.dispose?.();
      }
      m.dispose?.();
    }
  });
}

// ---------- Loading: .parse(), never .load() ----------
// GLTFLoader.load()/OBJLoader.load() do their own fetch() internally — connect-src 'none' in the
// plugin CSP blocks that outright. .parse() works entirely off bytes already pushed to us via
// lowarc:openFile, so it's the only entry point that's actually legal under this sandbox.
async function loadIntoDoc(doc, ext, bytes) {
  if (ext === ".glb") {
    const gltf = await new Promise((resolve, reject) => {
      gltfLoader.parse(bytes.buffer, "", resolve, reject);
    });
    return gltf.scene;
  }
  if (ext === ".obj") {
    // Geometry only — v1 deliberately ignores mtllib (an external .mtl this plugin has no way
    // to fetch under connect-src 'none', and no host mechanism yet resolves that reference).
    const text = new TextDecoder("utf-8").decode(bytes);
    const root = objLoader.parse(text);
    root.traverse((o) => {
      if (o.isMesh) o.material = new THREE.MeshStandardMaterial({ color: 0x9a9a9a, roughness: 0.85 });
    });
    return root;
  }
  throw new Error(`Unsupported extension "${ext}".`);
}

window.lowarc.on("lowarc:openFile", async (payload) => {
  if (!renderer || !payload || typeof payload.path !== "string" || docs.has(payload.path)) return;
  const ext = extOf(payload.path);
  const doc = createDocShell();
  docs.set(payload.path, doc);

  try {
    const bytes = base64ToBytes(payload.contents);
    const root = await loadIntoDoc(doc, ext, bytes);
    doc.root = root;
    doc.scene.add(root);
    frameCameraToBounds(doc.camera, doc.controls, root);
    doc.loadError = null;
  } catch (e) {
    doc.loadError = e && e.message ? e.message : String(e);
  }
  if (activePath === payload.path) refreshActiveState();
  dirty = true;
});

window.lowarc.on("lowarc:activateFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  activePath = payload.path;
  onResize();
  refreshActiveState();
  dirty = true;
});

window.lowarc.on("lowarc:closeFile", (payload) => {
  const doc = payload && docs.get(payload.path);
  if (!doc) return;
  disposeDoc(doc);
  doc.controls.dispose();
  docs.delete(payload.path);
  if (activePath === payload.path) {
    activePath = null;
    refreshActiveState();
  }
});

function refreshActiveState() {
  const doc = activePath ? docs.get(activePath) : null;
  const hasError = Boolean(doc && doc.loadError);
  loadErrorEl.classList.toggle("is-visible", hasError);
  if (hasError) loadErrorText.textContent = doc.loadError;
  toolbar.querySelectorAll("button").forEach((b) => (b.disabled = !doc || hasError));
}

// ---------- Toolbar ----------

function activeDoc() {
  return activePath ? docs.get(activePath) : null;
}

document.getElementById("frame-view").addEventListener("click", () => {
  const doc = activeDoc();
  if (doc && doc.root) {
    frameCameraToBounds(doc.camera, doc.controls, doc.root);
    dirty = true;
  }
});

document.getElementById("toggle-wireframe").addEventListener("click", () => {
  const doc = activeDoc();
  if (!doc || !doc.root) return;
  doc.wireframe = !doc.wireframe;
  doc.root.traverse((o) => {
    const mats = Array.isArray(o.material) ? o.material : o.material ? [o.material] : [];
    for (const m of mats) m.wireframe = doc.wireframe;
  });
  dirty = true;
});

document.getElementById("toggle-grid").addEventListener("click", () => {
  const doc = activeDoc();
  if (!doc) return;
  doc.hasGrid = !doc.hasGrid;
  if (doc.hasGrid && !doc.gridHelper) {
    const size = doc.root ? boundsOf(doc.root).size.length() || 10 : 10;
    doc.gridHelper = new THREE.GridHelper(size * 2, 20);
    doc.scene.add(doc.gridHelper);
  } else if (doc.gridHelper) {
    doc.gridHelper.visible = doc.hasGrid;
  }
  dirty = true;
});

// ---------- Resize / render loop ----------

function onResize() {
  if (!renderer) return;
  const w = Math.max(container.clientWidth, 1);
  const h = Math.max(container.clientHeight, 1);
  renderer.setSize(w, h, false);
  const doc = activeDoc();
  if (doc) {
    doc.camera.aspect = w / h;
    doc.camera.updateProjectionMatrix();
  }
  dirty = true;
}

new ResizeObserver(onResize).observe(container);

function tick() {
  requestAnimationFrame(tick);
  const doc = activeDoc();
  if (!renderer || !doc) return;
  doc.controls.update(); // no-op change events already set `dirty`; damping needs this every frame
  if (!dirty) return;
  renderer.render(doc.scene, doc.camera);
  dirty = false;
}

renderer = setupRenderer();
if (renderer) {
  onResize();
  tick();
}
