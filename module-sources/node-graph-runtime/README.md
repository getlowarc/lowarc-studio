# Node Graph Runtime

Interprets a `.lan` (LowArc Node) file — the first module that actually consumes what the Node
Graph plugin draws and saves, rather than just editing it.

## How it works

Parses the graph as a simple state machine: each node is a state, each connection an edge. On
start, picks a start node the same way the Node Graph plugin's own editor UI badges one — a node
with no incoming connections ("S" badge rule) — so there's exactly one definition of "start"
shared between editing and running a graph.

Every frame, publishes where things currently stand:

```json
{ "activeNodeId": "n2", "activeNodeLabel": "Chapter 2", "activeNodeFiles": ["ch2.txt"], "choices": ["n3", "n4"] }
```

Optionally requires a convention-based `input` role — any module willing to fill it, no hardcoded
coupling to keyboard/mouse/gamepad specifically (a node graph shouldn't need to know how "advance"
was actually triggered). Reads `shared.input.advanceTo` to transition, only if the target is
reachable via a direct outgoing edge from the current node. Auto-advancing through a whole linear
sequence in one frame is deliberately not supported — one edge, one frame, always.

With no `input` module installed, the interpreter simply stays on its start node forever — a
graph with no way to drive it is a valid (if inert) state, not an error.
