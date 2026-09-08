// Theme resolution and application — loaded early (right after style.css, before body content)
// on every page so the resolved theme applies before first paint as often as possible. style.css's
// :root values are the fallback for the instant before this runs at all; everything after that is
// this file's job.
//
// Three theme-mode values live in Settings.themeMode: "system" (follow the OS), "light", "dark",
// or the name of a user-saved custom preset (see theme.rs / settings.html's Appearance tab).
// Built-in Light/Dark are plain constants here, not files — every install has them regardless of
// what's on disk.

const THEME_CACHE_KEY = "lowarc-theme-cache";

// Ordered schema for the whole app: camelCase key (matches theme.rs's ThemeColors/serde and what
// Tauri commands send), the CSS custom property it drives, a human label, and a group — the
// Appearance page builds its color-editor grid straight from this instead of hand-listing fields.
const THEME_TOKENS = [
  { key: "bg", cssVar: "--bg", label: "Background", group: "Neutrals" },
  { key: "bgRaised", cssVar: "--bg-raised", label: "Raised surface", group: "Neutrals" },
  { key: "bgHover", cssVar: "--bg-hover", label: "Hover surface", group: "Neutrals" },
  { key: "border", cssVar: "--border", label: "Border", group: "Neutrals" },
  { key: "fg", cssVar: "--fg", label: "Text", group: "Neutrals" },
  { key: "fgDim", cssVar: "--fg-dim", label: "Dim text", group: "Neutrals" },
  { key: "cyan", cssVar: "--cyan", label: "Cyan", group: "Cyan" },
  { key: "cyanDark", cssVar: "--cyan-dark", label: "Cyan (sub)", group: "Cyan" },
  { key: "cyanDim", cssVar: "--cyan-dim", label: "Cyan (dim fill)", group: "Cyan" },
  { key: "cyanInk", cssVar: "--cyan-ink", label: "Cyan (ink)", group: "Cyan" },
  { key: "yellow", cssVar: "--yellow", label: "Yellow", group: "Yellow" },
  { key: "yellowDark", cssVar: "--yellow-dark", label: "Yellow (sub)", group: "Yellow" },
  { key: "yellowDim", cssVar: "--yellow-dim", label: "Yellow (dim fill)", group: "Yellow" },
  { key: "yellowInk", cssVar: "--yellow-ink", label: "Yellow (ink)", group: "Yellow" },
  { key: "danger", cssVar: "--danger", label: "Danger", group: "Status" },
  { key: "success", cssVar: "--success", label: "Success", group: "Status" },
];

const DARK_THEME = {
  bg: "#1e1e1e",
  bgRaised: "#252526",
  bgHover: "#2a2d2e",
  border: "#3c3c3c",
  fg: "#d4d4d4",
  fgDim: "#8a8a8a",
  cyan: "#00ffff",
  cyanDark: "#0096c8",
  cyanDim: "#06272c",
  cyanInk: "#04191b",
  yellow: "#ffff00",
  yellowDark: "#e8960a",
  yellowDim: "#332b00",
  yellowInk: "#1a1600",
  danger: "#ff3b30",
  success: "#00e676",
};

// Same hues as dark (cyan 180°, yellow ~48°) recalibrated for a light ground: pure cyan/yellow are
// nearly invisible on white, so "cyan"/"yellow" here are what "cyan-dark"/"yellow-dark" were in
// dark mode — deep enough for real contrast — and *-ink flips to white since the fill is now the
// dark end instead of the light end. danger/success are unchanged: their contrast requirement is
// against their own fill (usually white text), not against the page, so they don't need a second
// calibration.
const LIGHT_THEME = {
  bg: "#f5f5f7",
  bgRaised: "#ffffff",
  bgHover: "#ececee",
  border: "#dcdce0",
  fg: "#1c1c1e",
  fgDim: "#6c6c70",
  cyan: "#0097a8",
  cyanDark: "#00707d",
  cyanDim: "#e3f6f8",
  cyanInk: "#ffffff",
  yellow: "#a87900",
  yellowDark: "#7a5800",
  yellowDim: "#fbf0d9",
  yellowInk: "#ffffff",
  danger: "#ff3b30",
  success: "#00e676",
};

function builtInTheme(mode) {
  if (mode === "light") return LIGHT_THEME;
  if (mode === "dark") return DARK_THEME;
  return null;
}

function systemPrefersDark() {
  return window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/// Rough perceived brightness of a #rrggbb, 0–1. Only ever used to answer "is this a light theme or
/// a dark one", so the cheap Rec. 601 weighting is plenty — nothing here needs real colorimetry.
function luminanceOf(hex) {
  const m = /^#?([0-9a-f]{6})$/i.exec((hex || "").trim());
  if (!m) return 0;
  const n = parseInt(m[1], 16);
  return (0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
}

function applyThemeColors(colors) {
  const root = document.documentElement.style;
  for (const token of THEME_TOKENS) {
    const value = colors[token.key];
    if (value) root.setProperty(token.cssVar, value);
  }

  // Which way a button should move on hover, derived from the theme's own background rather than
  // hardcoded per built-in theme — so a user's own preset gets the right direction too.
  //
  // Buttons hover by brightening, which only works on a dark ground. On a light theme it pushes a
  // filled button UP toward its own ink: light ink is typically pure white, which cannot get any
  // brighter, so the two converge and the label or icon vanishes into the button. The Run button in
  // the header was exactly this — a white play glyph on teal, fading into a brightening surface.
  // Darkening instead moves them apart, which is also what light UIs conventionally do.
  //
  // A filter is used rather than per-type hover colors because it brightens the whole rendered
  // button, content included; there is no way to filter only the background. Getting the DIRECTION
  // right is what makes that acceptable, since the content then moves away from the surface rather
  // than into it.
  root.setProperty("--btn-hover-brightness", luminanceOf(colors.bg) > 0.5 ? "0.92" : "1.1");

  // Announced rather than pushed anywhere from here: this file has no business knowing which
  // iframes a page happens to be hosting. The editor listens and forwards to its plugin panels (see
  // split-view.js), and anything else that needs to react can do the same without theme.js growing
  // a list of consumers. Fires on EVERY apply, so an OS light/dark switch propagates the same way a
  // deliberate one does.
  try {
    window.dispatchEvent(new CustomEvent("lowarc-theme-applied"));
  } catch {
    // CustomEvent is unavailable in some minimal contexts; the theme itself is already applied.
  }
  try {
    localStorage.setItem(THEME_CACHE_KEY, JSON.stringify(colors));
  } catch {
    // localStorage can throw in some restricted contexts — a failed cache write just means the
    // next load repaints from scratch instead of instantly; not worth surfacing to the user.
  }
}

function applyCachedThemeIfAny() {
  try {
    const cached = localStorage.getItem(THEME_CACHE_KEY);
    if (cached) applyThemeColors(JSON.parse(cached));
  } catch {
    // Corrupt or missing cache — resolveAndApplyTheme() below still runs and repaints correctly.
  }
}

// Resolves Settings.themeMode against the OS / built-ins / saved presets and applies it. Safe to
// call repeatedly (e.g. from the OS theme-change listener, or after saving a preset in the
// Appearance page) — always re-reads current settings rather than assuming nothing changed.
async function resolveAndApplyTheme() {
  // Outside the real app (e.g. previewing a page through a plain static file server, with no
  // Tauri runtime injected) there's nowhere to read Settings.themeMode from — fall back to
  // whatever the OS prefers instead of throwing.
  if (!window.__TAURI__) {
    applyThemeColors(systemPrefersDark() ? DARK_THEME : LIGHT_THEME);
    return;
  }

  const { invoke } = window.__TAURI__.core;
  let mode = "system";
  try {
    mode = (await invoke("get_settings")).themeMode || "system";
  } catch {
    // Settings unreadable — fall back to system rather than leaving the cached/default theme.
  }

  const resolvedMode = mode === "system" ? (systemPrefersDark() ? "dark" : "light") : mode;
  const builtIn = builtInTheme(resolvedMode);
  if (builtIn) {
    applyThemeColors(builtIn);
    return;
  }

  try {
    const presets = await invoke("list_theme_presets");
    const preset = presets.find((p) => p.name === mode);
    if (preset) {
      applyThemeColors(preset.colors);
      return;
    }
  } catch {
    // Presets unreadable — fall through to the system fallback below.
  }

  // themeMode pointed at a custom preset that's gone missing (deleted from disk, etc.) — fall back
  // to what the OS prefers rather than leaving whatever was cached from a previous session.
  applyThemeColors(systemPrefersDark() ? DARK_THEME : LIGHT_THEME);
}

let systemListenerArmed = false;

function initTheme() {
  applyCachedThemeIfAny();
  resolveAndApplyTheme();

  if (!systemListenerArmed && window.matchMedia) {
    systemListenerArmed = true;
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      resolveAndApplyTheme();
    });
  }
}

initTheme();
