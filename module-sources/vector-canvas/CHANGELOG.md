# Changelog

All notable changes to the Vector Canvas module are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.1.0 — 2026-09-07

Initial release — the first module that gives a LowArc run a window.

### Added

- A windowed 2D drawing surface on OpenGL (femtovg + winit + glutin), cross-platform across
  Windows, macOS and Linux.
- Draw commands read every frame from `shared.director.draw`, executed in order: `clear`, `rect`
  (optionally rounded), `ellipse`, `circle`, `line`, `path`, `image`, `text`, the `push`/`pop`/
  `translate`/`rotate`/`scale` transform stack, and `camera`.
- Images and fonts resolved relative to the project root and cached by path.
- Window input published every frame — pointer position in both canvas and window coordinates,
  buttons, wheel delta, held physical keys, and focus. The canvas owns the window, so it is the
  only thing that can report a pointer position in the space a caller is drawing in.
- Closing the window ends the run, via the same `requestStop` any module can send.
- Window `title`, `width`, `height`, `resizable` and `vsync` configurable through module settings.
- No display available (a headless CI runner, a locked-down environment) is handled the same way
  the Audio module handles a missing audio device: the module keeps speaking the protocol and
  simply never draws, rather than failing the run.
