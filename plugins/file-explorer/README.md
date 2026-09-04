# File Explorer

Browse, create, rename, move, and delete a project's files, and review what's changed in them
this session.

## Features

- Full file tree: expand/collapse folders, VS Code-style "compact folders" (a chain of
  single-child directories collapses into one row), drag-and-drop to move files, cut/copy/paste,
  inline rename and delete (no native dialogs — everything happens in the tree itself).
- Real per-language file-type icons (vendored from VS Code's own Seti icon theme).
- A status dot on each open file showing unsaved/error state, live.
- Whole-project totals in the footer (file count, folder count, total size).

## Draft Tool

A lightweight, session-only backward snapshot bolted onto the explorer — not a separate plugin,
since browsing files and reviewing what changed in them are the same surface.

Open a Draft (give it a label and an optional description), edit freely, and the tool starts
tracking: the first time a tracked file is saved after the Draft opens, its original content is
kept. Every changed file shows added/removed line counts right in the tree, colored green/red.
When you're done:

- **Revert** restores every changed file to how it was when the Draft opened (deleting anything
  newly created), and discards the Draft.
- **Commit** just discards the Draft — whatever's on disk is already correct.

Storage is entirely local and session-only: it lives in the OS temp directory and is wiped on
every app launch, never inside the project folder itself. Toggle the whole feature off via the
"Draft Tool" setting if you don't want it.

## Settings

| Setting | Description |
| --- | --- |
| Show hidden files | Include files/folders whose name starts with a dot. |
| Folders first | Off sorts everything together, alphabetically. |
| Draft Tool | Shows/hides the Draft Tool section described above. |
