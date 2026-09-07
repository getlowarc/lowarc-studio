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
is, leaving room for a pixel or 3D one beside it; `audio-playback` leaves room for capture;
`device-input` says it polls hardware globally, which is what distinguishes it from the
window-scoped input `vector-canvas` publishes. (Plugins are products and can be named
artistically; modules can't afford to be.)

| Module | Source |
| --- | --- |
| `audio-playback` | `src-tauri/src/bin/audio_playback_runtime.rs` |
| `device-input` | `src-tauri/src/bin/device_input_runtime.rs` |
| `node-graph-runtime` | `src-tauri/src/bin/node_graph_runtime.rs` |
| `vector-canvas` | `src-tauri/src/bin/vector_canvas_runtime.rs` |

## Convention roles are not modules

Some ids name a *role* a project fills, not a module that ships here. `director` is one: both
`audio-playback` and `vector-canvas` optionally require it and read what it publishes, without
caring what actually provides it. `node-graph-runtime`'s optional `input` dependency is another —
it wants `shared.input.advanceTo`, a node id, from whatever a project installs under that name.

That second one used to collide with the shipped input module, which claimed the same id while
publishing `keyboard`/`mouse`/`gamepads` and no `advanceTo` at all. Renaming it to `device-input`
separated them. When adding a module, check its id doesn't shadow a role.
