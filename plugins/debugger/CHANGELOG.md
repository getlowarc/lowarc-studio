# Changelog

All notable changes to the Debugger plugin are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.1.1 - 2026-09-01

### Changed

- Internal: adopted the shared tooltip/numeric-input primitives other plugins use, and namespaced
  its own debug/session identifiers to avoid collisions. No user-facing behavior change.

## 0.1.0 - 2026-08-27

Initial release.

### Added

- Pause/step/breakpoints UI, driven entirely by the standard module wire protocol.
- Dedicated Debug console tab.
