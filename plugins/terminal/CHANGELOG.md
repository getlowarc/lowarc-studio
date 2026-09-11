# Changelog

All notable changes to the Terminal plugin are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.1.2 - 2026-09-11

### Fixed

- Typing `exit` now ends the session properly. On Windows the pty's reader never reports
  end-of-file when the shell exits, so the backend never noticed, never said the shell was gone,
  and stayed running with nothing to talk to. It now watches the shell process itself.

## 0.1.1 - 2026-09-01

### Changed

- Internal: adopted the shared tooltip/numeric-input primitives other plugins use, and namespaced
  its own debug/session identifiers to avoid collisions. No user-facing behavior change.

## 0.1.0 - 2026-08-27

Initial release.

### Added

- Multi-instance terminal, each a real PTY session (ConPTY on Windows, native pty elsewhere).
- Configurable shell command, font size, and scrollback.
- "Terminal: New Terminal" Command Palette entry.
