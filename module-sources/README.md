# Module sources

Tracked source for LowArc Studio's first-party runtime modules. This is distinct from `/modules/`
at the repo root, which is per-user runtime state (installed modules, gitignored, never
committed) — the same relationship `/plugins/` has to a user's own installed plugins, just without
the "tracked by default" carve-out plugins get, since a module's real source lives in
`src-tauri/src/bin/*.rs` and only its packaging (manifest, README, changelog) belongs here.

Each subfolder here is a real, installable module source: `manifest.json`, `README.md`,
`CHANGELOG.md`. The compiled binary itself is Cargo build output (see the matching `src-tauri/src/
bin/<name>.rs`) and is never checked in here — copy it alongside this folder's files when
installing locally for testing, the same way `AppPaths::ensure_builtin_plugin_binaries()` does for
built-in plugins.

| Module | Source |
| --- | --- |
| `audio` | `src-tauri/src/bin/audio_runtime.rs` |
| `input` | `src-tauri/src/bin/input_runtime.rs` |
| `node-graph-runtime` | `src-tauri/src/bin/node_graph_runtime.rs` |
