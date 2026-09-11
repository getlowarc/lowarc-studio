// The first real "consuming module" for a .lan node graph (see plugins/node-graph/node-graph.js's
// own header comment for the file format — that file is a pure editing surface and explicitly
// says wiring one up is "a separate, much later task, and belongs to whichever module a project
// writes to consume its own graphs"; this is that module). Interprets the graph as a simple state
// machine: the project's entry file IS the .lan graph, handed to this module's own compile() phase
// as sourceCode the same way any other module's entry file would be. start() enters the initial
// node (see pick_start_node); every frame() publishes exactly where the graph currently stands —
// {"activeNodeId", "activeNodeLabel", "activeNodeFiles", "choices"} — for any OTHER module to react
// to however makes sense for it (an audio module playing something in a node's own files list, a
// dialogue module rendering one, ...). This module only ever tracks POSITION in the graph; it never
// interprets what a node's files mean, on purpose: that's a different module's job, whichever one
// a project actually installs for it.
//
// Advancing is deliberately NOT automatic from topology alone. A node with exactly one outgoing
// connection does not immediately race to it: nothing here has any notion of "how long to wait",
// and blasting through an entire linear sequence within the first frame would defeat the point of
// having a sequence at all. Instead this module declares (in its own manifest.json) a fixed,
// conventional, OPTIONAL dependency on a module with id "input" (optional so a project that never
// installs one doesn't fail to run at all; see manifest::Dependency's own doc comment on why that
// distinction exists), and looks at shared.input.advanceTo (a node id) every frame — present and
// directly reachable from the current node, it moves there; otherwise it stays exactly where it
// is. No "input" module ships with this one (a real one — a player pressing a key, a timer, a
// dialogue choice UI — is a separate, later piece of work in its own right); until a project
// provides one, an interpreted graph simply sits on its start node forever, which is a safe,
// inert default rather than a broken one.
//
// Speaks the same one-JSON-object-per-line wire protocol every process module does (see
// runtime::process_module's own header comment for the shared/publish half of it specifically).

use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

#[derive(Debug, Deserialize, Clone)]
struct LanNode {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default, rename = "hasInput")]
    has_input: bool,
}

#[derive(Debug, Deserialize)]
struct LanConnection {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
struct LanGraph {
    #[serde(rename = "lowarcNodeGraph")]
    version: i64,
    nodes: Vec<LanNode>,
    #[serde(default)]
    connections: Vec<LanConnection>,
}

struct Graph {
    nodes: HashMap<String, LanNode>,
    /// node id -> ids directly reachable from it, in declared (connections array) order — fan-out
    /// is legitimate (a node's single output socket can carry several wires, see the .lan format's
    /// own comment on that), so this is a Vec, not a single Option<String>.
    outgoing: HashMap<String, Vec<String>>,
    /// Declaration order, purely as a deterministic fallback for pick_start_node when no node is
    /// unambiguously marked as an entry point.
    order: Vec<String>,
}

fn parse_graph(source: &str) -> Result<Graph, String> {
    let parsed: LanGraph = serde_json::from_str(source).map_err(|e| format!("not readable as a .lan file: {e}"))?;
    if parsed.version != 1 {
        return Err("not a LowArc node graph (missing or unexpected lowarcNodeGraph version).".into());
    }
    if parsed.nodes.is_empty() {
        return Err("this graph has no nodes.".into());
    }
    let order: Vec<String> = parsed.nodes.iter().map(|n| n.id.clone()).collect();
    let nodes: HashMap<String, LanNode> = parsed.nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    for c in &parsed.connections {
        // A connection naming a node that doesn't exist (a hand-edited or corrupted file) is
        // simply not a real edge — silently dropped here rather than failing the whole graph over
        // one bad entry, same "tolerant of missing ids" spirit as manifest::order_by_requires.
        if nodes.contains_key(&c.from) && nodes.contains_key(&c.to) {
            outgoing.entry(c.from.clone()).or_default().push(c.to.clone());
        }
    }
    Ok(Graph { nodes, outgoing, order })
}

/// The exact same rule the IDE's own "S" (Start) badge uses (see node-graph.js's renderBadges) —
/// !hasInput && hasOutput once Bridge is ruled out — reused here rather than inventing a second,
/// parallel definition of "where a graph starts." The first node satisfying it wins if more than
/// one does (deterministic, not an error — an author declaring several start-eligible nodes hasn't
/// done anything invalid, this module just has to pick one). Falls back to the first node overall
/// if none qualify, so a graph is always runnable even before an author has set that up.
fn pick_start_node(graph: &Graph) -> String {
    graph
        .order
        .iter()
        .find(|id| graph.nodes.get(*id).is_some_and(|n| !n.has_input))
        .cloned()
        .unwrap_or_else(|| graph.order[0].clone())
}

fn write_line(value: &Value) {
    let mut out = io::stdout();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn reply_ok(mut extra: Map<String, Value>) {
    extra.insert("ok".into(), Value::Bool(true));
    write_line(&Value::Object(extra));
}

fn reply_err(error: &str) {
    write_line(&json!({"ok": false, "error": error}));
}

fn main() {
    let mut graph: Option<Graph> = None;
    let mut active_node: String = String::new();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let phase = msg.get("phase").and_then(|p| p.as_str()).unwrap_or("");

        match phase {
            "compile" => {
                let source = msg.get("sourceCode").and_then(|s| s.as_str()).unwrap_or("");
                match parse_graph(source) {
                    Ok(g) => {
                        graph = Some(g);
                        reply_ok(Map::new());
                    }
                    Err(e) => reply_err(&e),
                }
            }
            "start" => match &graph {
                Some(g) => {
                    active_node = pick_start_node(g);
                    reply_ok(Map::new());
                }
                None => reply_err("start() called before a graph was ever compiled."),
            },
            "frame" => {
                let Some(g) = &graph else {
                    reply_err("frame() called before a graph was ever compiled.");
                    continue;
                };

                // The one input this module reads, not just publishes. See this file's own
                // header comment for why "input" is a fixed convention id rather than something
                // configurable per project.
                let requested = msg.pointer("/shared/input/advanceTo").and_then(|v| v.as_str());
                if let Some(target) = requested {
                    let reachable = g.outgoing.get(&active_node).is_some_and(|edges| edges.iter().any(|e| e == target));
                    if reachable {
                        active_node = target.to_string();
                    }
                }

                let node = g.nodes.get(&active_node);
                let choices: Vec<Value> = g.outgoing.get(&active_node).cloned().unwrap_or_default().into_iter().map(Value::String).collect();
                let publish = json!({
                    "activeNodeId": active_node,
                    "activeNodeLabel": node.map(|n| n.label.clone()).unwrap_or_default(),
                    "activeNodeFiles": node.map(|n| n.files.clone()).unwrap_or_default(),
                    "choices": choices,
                });
                let mut extra = Map::new();
                extra.insert("publish".into(), publish);
                reply_ok(extra);
            }
            "stop" => {
                reply_ok(Map::new());
                break;
            }
            other => reply_err(&format!("unknown phase \"{other}\"")),
        }
    }
}
