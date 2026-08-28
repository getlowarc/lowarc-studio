// The sidebar's own half of this plugin — just two buttons that hand off to the host's real
// native dialogs (via window.lowarc.createFile/pickOpenFile, generic harness primitives added
// for exactly this) and then ask the host to open whatever came back. The actual graph editing
// happens in the center-viewport half (index.html/node-graph.js), same split File Explorer and
// Monaco already have between "browse/open" (sidebar or File menu) and "edit" (center viewport).

const root = new URLSearchParams(window.location.hash.replace(/^#/, "")).get("project") || "";

function sep() {
  return root.includes("\\") ? "\\" : "/";
}

const GRAPH_FILTER = [{ name: "LowArc Node Graph", extensions: ["lan"] }];

document.getElementById("new-graph-btn").addEventListener("click", async () => {
  const path = await window.lowarc.createFile({
    title: "Create a new node graph",
    defaultPath: root ? `${root}${sep()}new-graph.lan` : "new-graph.lan",
    filters: GRAPH_FILTER,
    contents: "",
  });
  if (path) window.lowarc.openFile(path);
});

document.getElementById("open-graph-btn").addEventListener("click", async () => {
  const path = await window.lowarc.pickOpenFile({
    title: "Open a node graph",
    defaultPath: root || undefined,
    filters: GRAPH_FILTER,
  });
  if (path) window.lowarc.openFile(path);
});
