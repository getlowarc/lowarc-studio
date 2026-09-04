# Changelog

All notable changes to the Input module are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.1.0 — 2026-09-03

Initial release.

### Added

- Keyboard/mouse polling (Windows, macOS, Linux/X11) and gamepad polling (Windows, macOS, Linux,
  BSD, Wasm), published every frame.
- Graceful degradation when a backend is unavailable on the current platform.
