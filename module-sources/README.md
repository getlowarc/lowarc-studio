# Module sources

Tracked source for LowArc Studio's first-party runtime modules. This is distinct from `/modules/`
at the repo root, which is per-user runtime state (installed modules, gitignored, never
committed) — the same relationship `/plugins/` has to a user's own installed plugins, just without
the "tracked by default" carve-out plugins get, since a module's real source lives in
`src-tauri/src/bin/*.rs` and only its packaging (manifest, README, changelog) belongs here.

Each subfolder here is a real, installable module source: `manifest.json`, `process.json`,
`README.md`, `CHANGELOG.md`. The compiled binary itself is Cargo build output (see the matching
`src-tauri/src/bin/<name>.rs`) and is never checked in here — copy it alongside this folder's files
when installing locally for testing, the same way `AppPaths::ensure_builtin_plugin_binaries()` does
for built-in plugins.

`process.json` is tracked here even though it's tiny, because without it a folder copied from here
isn't actually runnable: `ProcessDescriptor::read` returns `None` and the runtime skips the module.
A folder here that installs but won't run would be worse than no folder at all.

Note that modules have no auto-install path — there is no module equivalent of
`ensure_builtin_plugin_binaries()`, so getting a built binary next to its manifest is still a manual
copy.

## First-party naming

Module names are literal, because modules are tools: `vector-canvas` says which kind of surface it
is, leaving room for a pixel or 3D one beside it. (Plugins are products and can be named
artistically; modules can't afford to be.) `audio` and `input` predate this rule and are both
broader than what they actually do — renaming them is pending.

| Module | Source |
| --- | --- |
| `audio` | `src-tauri/src/bin/audio_runtime.rs` |
| `input` | `src-tauri/src/bin/input_runtime.rs` |
| `node-graph-runtime` | `src-tauri/src/bin/node_graph_runtime.rs` |
| `vector-canvas` | `src-tauri/src/bin/vector_canvas_runtime.rs` |
