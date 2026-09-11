# Changelog

All notable changes to the Monaco Editor plugin are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.1.1 — 2026-09-04

### Added

- Support for reloading an already-open file's content in place (`lowarc:refreshFile`), for when
  something else changes a file on disk out from under the editor: the File Explorer's Draft
  Tool is the first thing to use it, for Revert.

## 0.1.0 — 2026-08-28

Initial release.

### Added

- Multi-language editing, one editor instance per group managing several open documents (models
  swap on tab switch, preserving per-file view state and undo history).
- Real per-language file-type icons.
- Monaco's own actions surfaced through the host's Command Palette.
- Host Edit-menu and paste integration.
- Settings: font size, tab size, insert-spaces, word wrap, minimap, line numbers.
