# Vector Canvas

A real 2D drawing surface for a LowArc project: this is the module that makes a run stop being
headless. It opens a window, owns an OpenGL context, and draws whatever your project tells it to.

Built on [femtovg](https://github.com/femtovg/femtovg), a GPU vector renderer (OpenGL ES 3.0+)
whose API is modelled on the HTML5 Canvas API: the same drawing model p5.js wraps. Windowing is
[winit](https://docs.rs/winit) with [glutin](https://docs.rs/glutin) for the GL context.
Cross-platform: Windows, macOS, Linux.

## Why "vector-canvas" and not "canvas"

It's a *vector* renderer (antialiased paths and strokes), not a sprite blitter, and it's one
possible surface rather than the only one. A pixel/tile surface or a 3D one should be able to sit
beside it as a peer instead of being "the other canvas". The command vocabulary below is this
module's own interface, not an engine-wide drawing protocol.

## How it works

Optionally consumes the **`draw-commands`** contract, whose specification ships beside this module
(`module-sources/draw-commands`). Any number of modules may provide it, and this one gathers: every
frame it concatenates the `draw` list from every provider, **in run order**, and executes the
result **in order**, which is also the draw order.

Nothing is elected to speak for the rest. A world module, a UI layer and a debug overlay each
publish their own commands and each simply draws; since a provider always runs before its
consumers, run order is z-order, and a module that requires the one it annotates therefore draws on
top of it without anyone arranging that.

Unlike audio's declarative "what should be playing" list, drawing is immediate-mode: a frame draws
exactly what it was handed, and a frame handed nothing draws nothing.

An unknown `op` is logged and skipped rather than failing the frame: one bad command must not take
down everything else being drawn.

### Commands

```jsonc
{"op":"clear","color":"#101820"}

// Shapes. Any of them take "fill" and/or "stroke" (+ "strokeWidth").
{"op":"rect","x":0,"y":0,"w":64,"h":64,"radius":0,"fill":"#ff8800","stroke":"#fff","strokeWidth":2}
{"op":"ellipse","x":100,"y":100,"rx":20,"ry":12,"fill":"#39f"}
{"op":"circle","x":100,"y":100,"r":20,"fill":"#39f"}
{"op":"line","x1":0,"y1":0,"x2":50,"y2":50,"stroke":"#fff","strokeWidth":1}
{"op":"path","points":[[0,0],[10,20],[30,5]],"close":true,"fill":"#0f0"}

// Images. "w"/"h" default to the file's own size; "tint" and "alpha" are alternatives.
{"op":"image","file":"art/player.png","x":0,"y":0,"w":32,"h":32,"rotation":0,"alpha":1}

// Text. "font" is required: there is no built-in font.
{"op":"text","text":"Score: 10","x":8,"y":8,"font":"fonts/Roboto.ttf","size":16,"fill":"#fff","align":"left"}

// Transform stack.
{"op":"push"} {"op":"translate","x":10,"y":10} {"op":"rotate","angle":0.5} {"op":"scale","x":2,"y":2} {"op":"pop"}

// Camera: puts world point (x,y) at the middle of the window, scaled by zoom.
{"op":"camera","x":0,"y":0,"zoom":1}
```

Colors are CSS-style hex strings (`#rgb`, `#rrggbb`, `#rrggbbaa`). Angles are radians.

Without a `camera` command the canvas behaves the way a 2D surface is normally expected to: `(0,0)`
is the top-left pixel and one unit is one pixel.

`file` and `font` paths resolve relative to the **project root**, so a director can say
`"art/player.png"` without knowing where the project lives. Both are **cached by path**: decoding a
PNG or parsing a TTF every frame would be the obvious performance trap here.

### What it publishes

Every frame, under its own id:

```jsonc
{
  "width": 960, "height": 640,
  "mouse": {
    "x": 120.0, "y": 88.0,           // canvas coordinates — the camera transform inverted
    "windowX": 120.0, "windowY": 88.0, // raw window pixels
    "left": false, "right": false, "middle": false,
    "scroll": 0.0,                    // this frame's wheel delta, not a running total
    "inside": true
  },
  "keys": ["KeyA", "Space"],          // physical keys currently held
  "focused": true,
  "closeRequested": false
}
```

Input is published here because this module owns the window, so it's the only thing that can report
a pointer position in canvas space. That deliberately overlaps the `input` module, which polls the
OS globally in screen space: that one remains the right source for gamepads and for input that
isn't about this window.

Keys are reported as **physical** key codes, so checking for "the W key" gives the same answer on a
QWERTY and an AZERTY keyboard.

Closing the window ends the run, the same way any module asking to stop does.

## Settings

Read once, at start:

| Setting | Default | |
| --- | --- | --- |
| `title` | `"LowArc"` | Window title |
| `width` / `height` | `960` / `640` | Initial size, in logical pixels |
| `resizable` | `true` | |
| `vsync` | `true` | A driver refusing the request just changes frame pace, it isn't an error |

## No display available

On a machine with no display at all (a headless CI runner, a locked-down environment), the module
still speaks the protocol, still replies, and simply never draws. The run continues rather than
dying on a machine that was never going to show a window. Same promise the `audio` module makes for
a missing audio device.

## Not supported

Gradients, composition modes and SVG are all available in the underlying renderer and simply aren't
exposed yet. femtovg itself does not support stroke dashing, path scissoring, custom shaders, or 3D
transforms.
