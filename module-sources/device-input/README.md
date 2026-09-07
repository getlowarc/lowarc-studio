# Device Input

Polls keyboard, mouse and gamepads at the OS level (screen coordinates, regardless of window
focus), cross-platform, with no coupling to any other module — a pure producer. Publishes state
every frame; what (if anything) consumes it is entirely up to the project.

For input *about a window* — a pointer position in the space you're drawing in — see
`vector-canvas`, which owns its window and so is the only thing that can report that. This module
is the right source for gamepads, and for input that should register whether or not a window has
focus.

## How it works

- Keyboard/mouse polling via [`device_query`](https://docs.rs/device_query) — Windows, macOS, and
  Linux/X11 (not Wayland: Wayland has no global input outside a focused window, by design).
- Gamepad polling via [`gilrs`](https://docs.rs/gilrs) — Windows, macOS, Linux, BSD, Wasm.

Publishes every frame:

```json
{
  "keyboard": { "keysDown": ["A", "Space"] },
  "mouse": { "x": 512, "y": 300, "buttonsDown": ["Left"] },
  "gamepads": [{ "id": 0, "name": "Xbox Controller", "buttonsDown": ["South"], "axes": { "LeftStickX": 0.5 } }]
}
```

An unavailable gamepad backend (or no keyboard/mouse polling support on the current platform)
degrades gracefully — the relevant section just stays empty, rather than failing the module.

## Not supported

Drawing-tablet/stylus input isn't read by this module. Modern platforms scope pen/tablet input to
a specific window by design (the same reason Wayland has no global keyboard/mouse polling) — the
best cross-platform crate found (`octotablet`) needs a real `raw_window_handle` to attach to,
which no process module in this engine currently has. Bigger architectural step, not attempted
here.
