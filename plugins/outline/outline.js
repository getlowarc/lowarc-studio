// Lightweight, per-language PATTERN MATCHING against raw source lines: not a real parser, and
// deliberately not one: this app supports ~30 languages via Monaco, and only 4 of those get a
// real AST from Monaco's own rich language services (see the session note on why this plugin
// can't reach those anyway — cross-iframe, a separate Monaco instance's worker state isn't
// reachable). A regex can't tell a real declaration from one that's inside a string or a comment,
// and it can't see scope: this is genuinely best-effort, not exhaustive, and every result should
// be read that way. Every language not explicitly listed below falls back to GENERIC_PATTERNS, a
// handful of common declaration keywords across many C-like/scripting languages — better than
// nothing, not a promise of coverage.

// Short words, not single letters: a bare "V" read as a chevron/checkmark glyph at this size in
// practice (reasonable mistake, arrow-like shapes and single capital letters aren't that
// different at 10px), which is actively misleading since these aren't buttons and don't expand
// anything. A short word can't be mistaken for an icon.
const KIND_LABEL = { constant: "const", variable: "var", function: "fn", class: "class", property: "key" };

const GENERIC_PATTERNS = [
  { re: /^\s*(?:export\s+)?const\s+([A-Za-z_$][\w$]*)/, kind: "constant" },
  { re: /^\s*(?:export\s+)?(?:let|var|local|my|our)\s+([A-Za-z_$][\w$]*)/, kind: "variable" },
  { re: /^\s*(?:export\s+)?function\s+([A-Za-z_$][\w$]*)/, kind: "function" },
];

// name -> kind for a plain `NAME = value` line with no declaration keyword at all (Python,
// shell-ish languages). ALL_CAPS reads as a constant by convention in those languages even
// though nothing in the syntax itself distinguishes it, the same convention a human reader uses.
function kindForBareAssignment(name) {
  return /^[A-Z][A-Z0-9_]*$/.test(name) ? "constant" : "variable";
}

const PATTERNS_BY_EXT = {
  ".js": jsPatterns(), ".jsx": jsPatterns(), ".mjs": jsPatterns(), ".cjs": jsPatterns(),
  ".ts": jsPatterns(), ".tsx": jsPatterns(),
  ".py": [
    { re: /^def\s+([A-Za-z_]\w*)/, kind: "function" },
    { re: /^class\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^([A-Za-z_]\w*)\s*(?::[^=]+)?=(?!=)/, kind: (name) => kindForBareAssignment(name) },
  ],
  ".rs": [
    { re: /^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([A-Za-z_]\w*)/, kind: "function" },
    { re: /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Za-z_]\w*)/, kind: "constant" },
    { re: /^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+(?:mut\s+)?([A-Za-z_]\w*)/, kind: "constant" },
    { re: /^\s*let\s+(?:mut\s+)?([A-Za-z_]\w*)/, kind: "variable" },
  ],
  ".go": [
    { re: /^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)/, kind: "function" },
    { re: /^\s*type\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^\s*const\s+([A-Za-z_]\w*)/, kind: "constant" },
    { re: /^\s*var\s+([A-Za-z_]\w*)/, kind: "variable" },
  ],
  ".java": javaLikePatterns(), ".cs": javaLikePatterns(), ".kt": javaLikePatterns(), ".dart": javaLikePatterns(),
  ".c": cLikePatterns(), ".h": cLikePatterns(), ".cpp": cLikePatterns(), ".hpp": cLikePatterns(), ".cc": cLikePatterns(),
  ".php": [
    { re: /^\s*function\s+([A-Za-z_]\w*)/, kind: "function" },
    { re: /^\s*class\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^\s*(?:public|private|protected)?\s*const\s+([A-Za-z_]\w*)/, kind: "constant" },
    { re: /^\s*define\(\s*['"]([A-Za-z_]\w*)['"]/, kind: "constant" },
    { re: /\$([A-Za-z_]\w*)\s*=(?!=)/, kind: "variable" },
  ],
  ".rb": [
    { re: /^\s*def\s+([A-Za-z_]\w*[?!]?)/, kind: "function" },
    { re: /^\s*(?:class|module)\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^\s*([A-Z][A-Z0-9_]*)\s*=(?!=)/, kind: "constant" },
    { re: /^\s*([a-z_]\w*)\s*=(?!=)/, kind: "variable" },
  ],
  ".lua": [
    { re: /^\s*(?:local\s+)?function\s+([A-Za-z_][\w.]*)/, kind: "function" },
    { re: /^\s*local\s+([A-Za-z_]\w*)/, kind: "variable" },
  ],
  ".sh": shellPatterns(), ".bash": shellPatterns(), ".ps1": shellPatterns(),
  ".sql": [{ re: /^\s*(?:CREATE\s+(?:OR\s+REPLACE\s+)?)?(?:TABLE|VIEW|FUNCTION|PROCEDURE)\s+([A-Za-z_][\w.]*)/i, kind: "class" }],
  ".css": [{ re: /^\s*(--[\w-]+)\s*:/, kind: "variable" }],
  ".scss": [{ re: /^\s*(\$[\w-]+)\s*:/, kind: "variable" }],
  // @media/@import/etc. are at-rules, not variables — excluded so they don't show up as if they
  // were declarations.
  ".less": [{ re: /^\s*(@(?!media|import|charset|font-face|keyframes|supports|page|namespace|document)[\w-]+)\s*:/, kind: "variable" }],
  ".json": [{ re: /^\s*"([^"]+)"\s*:/, kind: "property" }],
  ".jsonc": [{ re: /^\s*"([^"]+)"\s*:/, kind: "property" }],
  ".yml": [{ re: /^([A-Za-z_][\w-]*)\s*:/, kind: "property" }],
  ".yaml": [{ re: /^([A-Za-z_][\w-]*)\s*:/, kind: "property" }],
  ".ini": [{ re: /^\s*([A-Za-z_][\w.]*)\s*=/, kind: "property" }],
};

function jsPatterns() {
  return [
    { re: /^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+([A-Za-z_$][\w$]*)/, kind: "function" },
    { re: /^\s*(?:export\s+)?(?:default\s+)?class\s+([A-Za-z_$][\w$]*)/, kind: "class" },
    { re: /^\s*(?:export\s+)?const\s+([A-Za-z_$][\w$]*)/, kind: "constant" },
    { re: /^\s*(?:export\s+)?(?:let|var)\s+([A-Za-z_$][\w$]*)/, kind: "variable" },
  ];
}

function javaLikePatterns() {
  return [
    { re: /^\s*(?:public|private|protected|internal)?\s*(?:static\s+)?(?:class|interface|enum)\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^\s*(?:public|private|protected|internal)?\s*(?:static\s+)?(?:final|readonly|const|val)\s+[\w<>\[\],.\s]+\s+([A-Za-z_]\w*)\s*[=;]/, kind: "constant" },
    { re: /^\s*(?:public|private|protected|internal)?\s*(?:static\s+)?[\w<>\[\],.]+\s+([A-Za-z_]\w*)\s*\([^;]*\)\s*[{;]/, kind: "function" },
  ];
}

function cLikePatterns() {
  return [
    { re: /^\s*(?:typedef\s+)?(?:struct|enum|union|class)\s+([A-Za-z_]\w*)/, kind: "class" },
    { re: /^\s*#define\s+([A-Za-z_]\w*)/, kind: "constant" },
    { re: /^\s*(?:static\s+)?const\s+[\w*\s]+\s+([A-Za-z_]\w*)\s*=/, kind: "constant" },
  ];
}

function shellPatterns() {
  return [
    { re: /^\s*function\s+([A-Za-z_]\w*)/, kind: "function" },
    { re: /^\s*([A-Za-z_]\w*)\s*\(\)\s*\{/, kind: "function" },
    { re: /^\s*(?:export\s+)?([A-Za-z_]\w*)=/, kind: (name) => kindForBareAssignment(name) },
  ];
}

function extOf(path) {
  const name = path.split(/[\\/]/).pop() || path;
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

// ---------- Editable values ----------
// Deliberately narrow: only a bare boolean, number or quoted string at the very END of the line
// counts. Anything more complex is left alone, since this cannot tell what a call or an object
// literal means well enough to rewrite it safely. Two operators, `=` for assignment and `:` for
// JSON, YAML and INI-style lines, since a line only ever matches one. The `d` flag gives group 1's
// own offsets, which is what allows splicing a replacement without disturbing the rest of the
// line.
const VALUE_RE = /(?<![!<>=])=(?!=)\s*(true|false|-?\d+(?:\.\d+)?|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*;?\s*$/d;
const COLON_VALUE_RE = /:\s*(true|false|-?\d+(?:\.\d+)?|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')\s*,?\s*$/d;

function findValueMatch(line) {
  return line.match(VALUE_RE) || line.match(COLON_VALUE_RE);
}

function extractValue(line) {
  const m = findValueMatch(line);
  if (!m) return null;
  const raw = m[1];
  const [start, end] = m.indices[1];
  if (raw === "true" || raw === "false") return { type: "boolean", value: raw === "true", start, end };
  if (/^-?\d+(?:\.\d+)?$/.test(raw)) return { type: "number", value: Number(raw), start, end };
  return { type: "string", value: raw.slice(1, -1), quote: raw[0], start, end };
}

// Rebuilds one line with a new literal spliced into the exact span extractValue found — start/end
// are recorded from the line's ORIGINAL text and stay valid as long as this is the first edit
// made to that particular line since the last real refresh (see commitEdit in the rendering
// section below, which keeps a symbol's own cached lineText/value in sync after every edit so a
// second edit to the same line before the next refresh still targets the right offsets).
function buildEditedLine(line, valueInfo, newValue) {
  let literal;
  if (valueInfo.type === "boolean") literal = newValue ? "true" : "false";
  else if (valueInfo.type === "number") literal = String(newValue);
  else literal = valueInfo.quote + String(newValue).split(valueInfo.quote).join("\\" + valueInfo.quote) + valueInfo.quote;
  return line.slice(0, valueInfo.start) + literal + line.slice(valueInfo.end);
}

// Only these three kinds can meaningfully have a "value" the way this feature means it: a
// function/class name is an identifier, not a value slot, so there's nothing to extract for those.
const VALUE_KINDS = new Set(["variable", "constant", "property"]);

function extractSymbols(path, contents) {
  const patterns = PATTERNS_BY_EXT[extOf(path)] || GENERIC_PATTERNS;
  const lines = contents.split(/\r\n|\r|\n/);
  const symbols = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    for (const pattern of patterns) {
      const match = line.match(pattern.re);
      if (!match) continue;
      const name = match[1];
      const kind = typeof pattern.kind === "function" ? pattern.kind(name) : pattern.kind;
      const value = VALUE_KINDS.has(kind) ? extractValue(line) : null;
      symbols.push({ name, kind, line: i + 1, lineText: line, value });
      break; // first matching pattern per line: a line is exactly one declaration, not several
    }
  }
  return symbols;
}

// ---------- Rendering ----------

const emptyEl = document.getElementById("empty");
const listEl = document.getElementById("list");
// Same chevron the file explorer draws for its own expand/collapse rows: a real one this time,
// not a kind-letter that happened to resemble one.
const CHEVRON_SVG = '<svg viewBox="0 0 10 10" fill="none"><path d="M3 1l4 4-4 4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';

let currentPath = null;
// "line:name" -> expanded — only meaningful for the currently-rendered file's own symbols; reset
// whenever render() is handed a genuinely different path, kept as-is across a same-file refresh
// (a save, or a live edit elsewhere) so an open row doesn't collapse out from under someone mid-edit.
let expandedKeys = new Set();
let lastRenderedPath = null;

function symbolKey(symbol) {
  return `${symbol.line}:${symbol.name}`;
}

// Pushes an edited value into the file that's actually open, then updates this symbol's own
// cached lineText/value in place: not a full re-extract-and-rerender, so a second edit to the
// same line before the next natural refresh (see updateInspectorForActiveFile's own trigger
// points in editor.html: file focus changes and saves, not every keystroke) still targets the
// right offsets, and the control doesn't visually snap back to a stale value in the meantime.
function commitEdit(symbol, newValue) {
  const newLine = buildEditedLine(symbol.lineText, symbol.value, newValue);
  window.lowarc.editFile(currentPath, symbol.line, newLine);
  symbol.lineText = newLine;
  symbol.value = extractValue(newLine) || symbol.value;
}

// Every value type EXCEPT boolean can be anything: a dropdown only makes sense when the full set
// of valid options is actually known ahead of time, which is only ever true here for true/false.
// Numbers and strings stay a plain input for exactly that reason, not because a dropdown wouldn't
// look nicer.
function buildControl(symbol) {
  if (symbol.value.type === "boolean") {
    return createLowarcDropdown(
      [
        { value: true, label: "true" },
        { value: false, label: "false" },
      ],
      symbol.value.value,
      (newValue) => commitEdit(symbol, newValue),
    );
  }

  const input = document.createElement("input");
  input.className = "input outline-value-input";
  if (symbol.value.type === "number") {
    input.type = "number";
    input.value = symbol.value.value;
    input.addEventListener("change", () => {
      const n = Number(input.value);
      if (!Number.isNaN(n)) commitEdit(symbol, n);
    });
  } else {
    input.type = "text";
    input.value = symbol.value.value;
    input.addEventListener("change", () => commitEdit(symbol, input.value));
  }
  // Commit on Enter too, not just on blur (the native "change" event) — matches how every other
  // inline-edit field in this app behaves (see explorer.js's rename/create inputs).
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") input.blur();
  });
  return input;
}

function render(path, symbols) {
  currentPath = path;
  if (path !== lastRenderedPath) {
    expandedKeys.clear();
    lastRenderedPath = path;
  }

  if (!path) {
    emptyEl.textContent = "No file open.";
    emptyEl.classList.add("is-visible");
    listEl.classList.remove("is-visible");
    listEl.innerHTML = "";
    return;
  }
  if (!symbols.length) {
    emptyEl.textContent = "Nothing recognized in this file.";
    emptyEl.classList.add("is-visible");
    listEl.classList.remove("is-visible");
    listEl.innerHTML = "";
    return;
  }
  emptyEl.classList.remove("is-visible");
  listEl.classList.add("is-visible");
  listEl.innerHTML = "";
  for (const symbol of symbols) {
    const key = symbolKey(symbol);
    const expanded = symbol.value && expandedKeys.has(key);

    const row = document.createElement("div");
    row.className = "outline-row" + (expanded ? " is-open" : "");
    row.title = `Line ${symbol.line}`;

    // Only a symbol with an editable value actually expands into anything: a function/class row
    // gets the same reserved-but-invisible chevron space file explorer gives a leaf row, so names
    // still line up in a column instead of editable and non-editable rows drifting out of sync.
    const chevronEl = document.createElement("span");
    chevronEl.className = "chevron" + (symbol.value ? (expanded ? " expanded" : "") : " hidden");
    if (symbol.value) chevronEl.innerHTML = CHEVRON_SVG;
    row.appendChild(chevronEl);

    const nameEl = document.createElement("span");
    nameEl.className = "outline-name";
    nameEl.textContent = symbol.name;
    row.appendChild(nameEl);

    if (symbol.value) {
      row.classList.add("is-expandable");
      row.addEventListener("click", () => {
        if (expandedKeys.has(key)) expandedKeys.delete(key);
        else expandedKeys.add(key);
        render(currentPath, symbols);
      });
    }

    if (expanded) {
      // The control takes the SAME trailing slot the kind label normally occupies — inline, on
      // this one row, not a second row underneath it (that's what made this feel cramped before).
      const controlEl = document.createElement("span");
      controlEl.className = "outline-control";
      controlEl.appendChild(buildControl(symbol));
      // A click inside the control (opening the dropdown, focusing the text input) shouldn't also
      // toggle the row itself closed.
      controlEl.addEventListener("click", (e) => e.stopPropagation());
      row.appendChild(controlEl);
    } else {
      const kindEl = document.createElement("span");
      kindEl.className = `outline-kind kind-${symbol.kind}`;
      kindEl.textContent = KIND_LABEL[symbol.kind] || "?";
      row.appendChild(kindEl);
    }

    listEl.appendChild(row);
  }
}

render(null, []);

window.lowarc.on("lowarc:activeFile", (payload) => {
  if (!payload || typeof payload.path !== "string" || typeof payload.contents !== "string") {
    render(null, []);
    return;
  }
  render(payload.path, extractSymbols(payload.path, payload.contents));
});
