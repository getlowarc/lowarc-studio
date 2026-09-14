# LowArc Studio

A Tauri v2 desktop IDE for [LowArc](https://lowarc.com), a game engine built around small,
independently-loadable modules (native, process, or otherwise). LowArc Studio is where a project's
modules get assembled, a project's code gets edited, and a dev build gets run and debugged: all
in one self-contained app.

## Status

Windows-only for now. The toolchain pin (`rust-toolchain.toml`) targets
`stable-x86_64-pc-windows-msvc`, and a few pieces the app depends on (ConPTY for the Terminal
plugin, native `.dll` module loading, WebView2) are Windows-specific today. Extending to other
platforms would start with that pin.

## Getting started

**Prerequisites**: a Rust toolchain (rustup will pick up the pin in `rust-toolchain.toml`
automatically) and Windows 10/11 with WebView2 (already present on any current Windows install).

```bash
cd src-tauri
cargo run
```

That's the whole setup: no `npm install`, no separate frontend build step. The frontend
(`src/`) is plain HTML/CSS/JS served directly by Tauri (`frontendDist` in `tauri.conf.json`, no
bundler in front of it), and first launch from a fresh clone takes care of its own bookkeeping
(`AppPaths::ensure_directories()`/`ensure_builtin_plugin_binaries()` in `app_paths.rs`): it creates
`modules/`, `plugins/`'s runtime bits, `themes/`, and copies the two backend binaries
(`file_explorer_backend`, `terminal_backend`) into their plugin folders automatically.

Running from a source checkout keeps all of that per-user state inside the repo itself (at the
repo root; see `.gitignore`) rather than scattering it into a hidden per-user app-data folder, so
a dev checkout never touches anything outside its own directory.

## Testing

```bash
cd src-tauri
cargo test
```

Runs the full unit + integration suite, including real end-to-end tests that spawn actual
subprocess/native modules and (for the export path) actually launch an exported build. No
external checkout or network access needed for any of it.

```bash
cargo clippy --all-targets -- -D warnings
```

Clean as of this writing: CI (`.github/workflows/ci.yml`) runs both this and `cargo test` on
every push/PR. There's no `rustfmt.toml` yet, so `cargo fmt` isn't enforced: this codebase
deliberately runs wider than rustfmt's 100-char default on long, well-commented single-line
statements.

## Building an installer

```bash
cargo install tauri-cli --version "^2.0.0" --locked  # once
cd src-tauri
cargo tauri build
```

This is the one thing plain `cargo build`/`cargo run` doesn't cover. Packaging needs the actual
Tauri CLI, which drives `prepare-bundle.ps1` (stages the backend binaries the bundler needs to
find) before invoking the platform bundler.

## Project layout

- **`src/`** — the frontend: `editor.html` (the main IDE shell) plus `src/editor/*.js` (its script,
  split across files in load order), `primitives.js`/`primitives.css` (shared host-side UI
  components), and a handful of standalone pages (`settings.html`, `modules.html`, `plugins.html`,
  `export.html`) that each host their own popup/page via the same primitives.
- **`src-tauri/`**: the Rust backend. Notable modules:
  - `runtime/`: the actual game-engine runtime (loaders, driver, manifest resolution) embedded in
    the IDE for dev-run, and reused by `bin/lowarc_runtime.rs` for exported, standalone builds.
  - `export/`: stages a project into a self-contained folder for distribution.
  - `plugin_host/`, `plugin_assets.rs`, `plugin_asset_server.rs`, `plugin_session.rs`: the plugin
    system: process isolation, the sandboxed-iframe asset server, and long-lived plugin sessions
    (Terminal).
  - `bin/` holds the extra binary targets built alongside the main app: `lowarc_runtime` (the exported
    runtime), `native_module_host` (isolates a native-kind module in its own process),
    `file_explorer_backend`/`terminal_backend` (the two built-in plugins with their own backends).
- **`plugins/`**: every built-in plugin, each its own folder (`plugin.json` + assets, optionally a
  built backend binary). Tracked as source by default. See `.gitignore`'s own note on this.
- **`modules/`, `themes/`, `settings.json`, `recent.json`**: per-user runtime state created on
  first launch (see Getting Started above). Not tracked; entirely disposable.

## Plugins

A plugin runs inside a fully sandboxed iframe (`sandbox="allow-scripts"`, no `allow-same-origin`,
no Tauri capability of its own) and can only reach the host through a small `window.lowarc.*`
harness: everything else (file access, native dialogs, running its own backend process) is
relayed through the host, which enforces per-plugin identity on every relayed action. See
`plugin_assets.rs`'s `HARNESS_JS` for the current API surface, and any existing plugin under
`plugins/` for a working example of the manifest shape (`plugin.json`).

## Contributing

Only commits that pass `cargo build`, `cargo test` and `cargo clippy --all-targets -- -D warnings`
should land on `main`; CI enforces the same three. See [CONTRIBUTING.md](CONTRIBUTING.md) before
you start on anything large, and [SECURITY.md](SECURITY.md) if you found a vulnerability rather
than a bug.

## Contact

- **Bugs and features**: [open an issue](https://github.com/getlowarc/lowarc-studio/issues).
- **Questions and help**: <support@lowarc.com>.
- **Security**: <security@lowarc.com>, or GitHub's private reporting. See [SECURITY.md](SECURITY.md).
- **Conduct**: <report@lowarc.com>. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
- **Licensing, trademark, anything legal**: <legal@lowarc.com>.
- **Anything else**: <info@lowarc.com>.

## License

Apache-2.0. See [LICENSE](LICENSE), and [NOTICE](NOTICE) for the third-party code LowArc Studio
bundles.

The licence covers the code, not the name: Apache-2.0 grants no trademark rights (section 6), so
"LowArc" and the bolt mark are not yours to use by having forked this. Ask <legal@lowarc.com> if
you need to.
