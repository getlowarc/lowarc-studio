# Changelog

All notable changes to the File Explorer plugin are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.2.0 - 2026-09-04

### Added

- Draft Tool: a lightweight, session-only backward snapshot bolted onto the explorer. Open a
  Draft, edit freely, and every changed file shows added/removed line counts right in the tree.
  Revert restores everything to how it was when the Draft opened; Commit just stops tracking.
  New "Draft Tool" setting to show/hide it.

## 0.1.0 - 2026-08-28

Initial release.

### Added

- Full file tree with create/rename/move/delete/copy/paste, drag-and-drop, and inline editing
  (no native dialogs).
- VS Code-style compact-folder chains for single-child directory nesting.
- Real per-language file-type icons.
- Live dirty/error status dot per open file.
- Whole-project file/folder/size totals in the footer.
- Settings: show hidden files, folders-first sorting.
