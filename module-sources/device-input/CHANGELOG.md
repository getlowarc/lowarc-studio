# Changelog

All notable changes to the Device Input module are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.3.0 - 2026-09-09

### Changed

- Provides the `input-state` contract, so a consumer can read input by role rather than by this module's name: alongside `vector-canvas`, which reports the same kind of state in canvas coordinates.

## 0.2.0 - 2026-09-07

### Changed

- Renamed from `input` to `device-input`. Two reasons. First-party module names are literal, and
  this one polls hardware globally in screen coordinates: distinct from the window-scoped input
  `vector-canvas` publishes. It also collided with `node-graph-runtime`'s optional `input`
  dependency, which is a convention ROLE wanting `shared.input.advanceTo` and never matched what
  this module publishes; the two are now separate. A project requiring `input` must update its
  `project.json`.

## 0.1.0 - 2026-09-03

Initial release.

### Added

- Keyboard/mouse polling (Windows, macOS, Linux/X11) and gamepad polling (Windows, macOS, Linux,
  BSD, Wasm), published every frame.
- Graceful degradation when a backend is unavailable on the current platform.
