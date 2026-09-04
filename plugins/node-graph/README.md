# Node Graph

A visual node-graph editor for `.lan` (LowArc Node) files — boxes and connections.

Pure editing surface for now: nothing here feeds into the runtime/compiler yet, this just lets
you draw and save a graph. (The `node-graph-runtime` module is the piece that actually
interprets a `.lan` file at run time — see its own README.)

## Features

- Open a `.lan` file as a viewer in the center viewport: place, connect, and rearrange nodes.
- An inspector panel for editing a selected node's or connection's details (labels, attached
  files, connection metadata) without leaving the graph.
- Optional badges on nodes/connections — small indicators for attached files, port type, and
  connection counts.

## Settings

| Setting | Description |
| --- | --- |
| Show node/connection badges | Small indicators for attached files, port type, and connection counts. |
