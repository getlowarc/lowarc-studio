# Monaco Editor

Code editor for opened files, powered by [Monaco](https://microsoft.github.io/monaco-editor/)
(vendored from the `monaco-editor` npm package; see `LICENSE.md`).

## Features

- Full editing surface for a broad range of languages (JS/TS, JSON, HTML/CSS, Markdown, Python,
  Rust, Go, Java, C/C++/C#, PHP, Ruby, shell, YAML/XML/SQL, Lua, Swift, Kotlin, Dart, PowerShell,
  Batch, INI, and plain text).
- One editor instance per editor group, managing multiple open documents at once — switching tabs
  swaps models rather than tearing down and recreating the editor, so per-file scroll position,
  cursor, and undo history all survive a tab switch.
- Real per-language file-type icons, contributed to the shared icon set.
- Monaco's own actions (Find/Replace, Command Palette actions, formatting, etc.) surface through
  the host's own Command Palette rather than a second, competing command surface.
- The host's Edit menu (Undo/Redo/Cut/Copy/Find/Replace) and paste all route through here.
- Reloads an already-open file's content in place when something else changes it on disk (e.g.
  the File Explorer's Draft Tool reverting a file): a distinct signal from opening a file for the
  first time, so a normal open never clobbers in-progress edits.

## Settings

| Setting | Description |
| --- | --- |
| Font size | |
| Tab size | |
| Insert spaces on Tab | Off inserts an actual tab character instead. |
| Word wrap | |
| Minimap | |
| Line numbers | |
