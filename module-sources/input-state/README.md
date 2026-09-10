# Input State

A contract, not a module — it has no process and never runs. Installing it puts this specification
and its version in the module store.

Unlike `draw-commands` and `audio-cues`, this one is owned by no single module, because two very
different things honestly provide it and neither is the authority.

## Why there are several providers

- **A surface** (`vector-canvas`) reports the pointer in **canvas coordinates**, because it owns the
  window and is the only thing that can invert the camera transform. It knows nothing outside its
  own window.
- **A device poller** (`device-input`) reports globally in **screen coordinates**, gamepads
  included, regardless of window focus.

Both are input. Neither is wrong. A consumer reads `shared["input-state"]` — an array with one entry
per provider, each tagged with `from` — and picks whichever it actually wants, rather than being
handed one blend of the two by something that had to guess.

## Fields

Every field is optional; a provider publishes what it is in a position to know, and a consumer must
tolerate absence rather than assume a provider it never named.

| Field | Meaning |
| --- | --- |
| `width`, `height` | Surface size in pixels, if the provider owns one. |
| `mouse.x`, `mouse.y` | Pointer position in the provider's own coordinate space. |
| `mouse.windowX`, `mouse.windowY` | Pointer in window pixels, when those differ from the above. |
| `mouse.left`, `mouse.right`, `mouse.middle` | Button held. |
| `mouse.scroll` | Wheel delta **for this frame**, not a running total. |
| `mouse.inside` | Pointer within the surface. |
| `keys` | Held key names. |
| `focused` | Surface has focus. |
| `closeRequested` | The user asked to close the surface. |

`mouse.scroll` being a per-frame delta is deliberate: a wheel movement belongs to the frame it
happened in, not to every frame after it.

## Versioning

Adding an optional field is a minor version. Removing one, or changing what an existing field
means, is a major version. A consumer pins with `"version": "^1"`.
