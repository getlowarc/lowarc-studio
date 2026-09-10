# Draw Commands

A contract, not a module — it has no process and never runs. Installing it puts this specification
and its version in the module store, so a module that draws and a module that renders can agree on
a vocabulary without either depending on the other's binary.

Owned by `vector-canvas`, deliberately. This is a 2D vector vocabulary, not an engine-wide drawing
protocol: a 3D or tile surface should define its own rather than pretend to speak this one.

## Providing it

Declare it, then publish a `draw` array each frame under your own id:

```json
{ "provides": [{ "contract": "draw-commands", "version": "^1" }] }
```

```json
{ "publish": { "draw": [ { "op": "clear", "color": "#101014" }, { "op": "circle", "x": 100, "y": 100, "r": 20, "fill": "#00ffff" } ] } }
```

Any number of modules may provide this at once. A renderer concatenates every provider's list **in
run order** — a provider always runs before its consumers, so the resulting order is deterministic,
and since draw order is list order, run order is z-order. Nothing is elected; a debug overlay and a
UI layer each simply draw.

## Consuming it

`shared["draw-commands"]` is an array with one entry per provider, each tagged with its origin:

```json
[ { "from": "my-director", "draw": [ ... ] }, { "from": "debug-overlay", "draw": [ ... ] } ]
```

Drawing is immediate-mode: a frame draws exactly what it was handed, and a frame handed nothing
draws nothing. There is no retained scene and no state carried between frames except what the
transform ops below explicitly push.

## Commands

Every command is an object with an `"op"`. Coordinates are in canvas space. Colors are `#rrggbb` or
`#rrggbbaa` strings. An unknown op is skipped with a warning rather than failing the frame — one bad
command must not take down everything else being drawn.

| Op | Fields |
| --- | --- |
| `clear` | `color` |
| `rect` | `x`, `y`, `w`, `h`, `radius`, `fill`, `stroke`, `strokeWidth` |
| `ellipse` | `x`, `y`, `rx`, `ry`, `fill`, `stroke`, `strokeWidth` |
| `circle` | `x`, `y`, `r`, `fill`, `stroke`, `strokeWidth` |
| `line` | `x1`, `y1`, `x2`, `y2`, `stroke`, `strokeWidth` |
| `path` | `points`, `close`, `fill`, `stroke`, `strokeWidth` |
| `image` | `file`, `x`, `y`, `w`, `h`, `rotation`, `alpha`, `tint` |
| `text` | `text`, `font`, `x`, `y`, `size`, `align` (`left`/`center`/`right`), `fill` |
| `push` / `pop` | — (save and restore the transform) |
| `translate` | `x`, `y` |
| `rotate` | `angle` (radians) |
| `scale` | `x`, `y` |
| `camera` | `x`, `y`, `zoom` |

`fill` and `stroke` are independent: a shape carrying both is filled and then stroked, and one
carrying neither draws nothing. `w`/`h` on `image` default to the file's natural size.

`file` and `font` paths are relative to the project root, and are cached by path — decoding a PNG or
parsing a TTF every frame would be the obvious performance trap here.

## Versioning

Adding an op or an optional field is a minor version. Removing one, or changing what an existing
field means, is a major version. A consumer pins with `"version": "^1"`.
