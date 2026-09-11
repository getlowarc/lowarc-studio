# Modules, contracts and stacks

**Status: the contract mechanism is implemented; stacks and engines are not.** Everything through
"Migration" ships. The "layer above" section is a design note, deliberately unbuilt.

## What's wrong today

The manifest itself is small and clear. What's obscure is what `requires` *means*, because it
currently means three different things at once:

- **this module must exist**: the resolver fails or skips without it
- **run me after it**: `order_by_requires` sorts the run by this list
- **let me read what it publishes** — `ProcessModule::frame` filters the run-wide `shared` map down
  to exactly the ids in this list

Three concepts, one field, no way to tell which one an entry is about. That's the obscurity.

The second problem is bigger. A module's published state is keyed by its **own manifest id**
(`shared.entry(m.id)`), and a consumer reads by that same id. So `vector-canvas` reading
`shared.director.draw` means one thing literally: **some module's manifest id is `"director"`.**
That has consequences nobody chose:

- There can only ever be one director in a run, because ids are unique.
- A module can only take the job by being *named* the job. `node-graph-runtime` cannot drive
  `vector-canvas` no matter what it publishes, because it publishes under `node-graph-runtime`.
- Nothing anywhere says what a director is supposed to publish. `vector-canvas` expects
  `shared.director.draw`: an ordered list of immediate-mode draw ops. `audio-playback` expects
  `shared.director.play`: a declarative list of what should be sounding right now. Different
  shapes, different philosophies, same key, and each one documented only in its own consumer's
  header comment.

So "director" is doing the work of an interface while being nothing but a name. That's the wrong
shape, and it's the part to fix.

## The idea

**A role is not an id. A role is a contract, and a contract is a module.**

The vocabulary of draw commands that `vector-canvas` understands becomes a real, installable,
versioned thing with a name and a document. `vector-canvas` doesn't implement "whatever the module
called director happens to send". It declares that it **consumes** that contract. Anything that
declares it **provides** the same contract can drive it, whatever that thing is called.

This is the smallest change that gets order without restriction:

- The engine matches **producers to consumers by contract**, and enforces the version.
- The engine **never validates payloads**. What a draw op looks like is the contract document's
  business, not the runtime's.

So authors keep every degree of freedom they have now. What they gain is a place where the
agreement is written down, versioned, and shipped, instead of a comment in someone's `.rs` file.

## Contracts

A contract is a module with no executable. Its whole content is a name, a version, and a document
describing what flows under it.

```json
{
  "id": "draw-commands",
  "kind": "contract",
  "version": "1.0",
  "name": "Draw Commands",
  "description": "An ordered list of immediate-mode drawing operations, executed in order.",
  "providers": "one"
}
```

- **`kind: "contract"`** is what tells the resolver this is a definition, not something to run. It
  takes part in dependency resolution and version checking, and is never spawned.
A contract always has **many** possible providers. There is no cardinality field, because no
contract has yet wanted one: a surface gathers, and if some future contract genuinely needs
exactly one provider, that constraint can be added when something asks for it rather than
speculatively now.

Gathering is the whole point. The surface doesn't take a frame from one privileged module and
comply with it: every module that draws publishes its own list of ops, and the canvas concatenates
them in run order, which is already deterministic (a provider always runs before its consumers, so
ordering falls out of the existing requires-rank sort). Draw order is list order, so run order
becomes z-order.

That inverts the current arrangement, and it is the better direction: the canvas stops being
something each producer must funnel through a single director to reach, and becomes something any
number of modules aim at. A debug overlay, a UI layer and a particle module can each draw without
one of them being elected to speak for the others.

Worth naming the alternative that was considered and rejected: having the canvas expose a mutable
buffer that other modules write into directly. It reads as the same idea, but it would need a new
write channel in the protocol (today a module can only publish under its own id), and it brings
questions gathering doesn't have: who owns the buffer, when is it cleared, what happens on a partial
write, and what order two writers land in. Gathering gets the same result with no new machinery.

A contract ships a `README.md` the same way every module does, and that README *is* the
specification. Its version is what consumers pin against.

A contract defines nothing about who is allowed to author it. **A surface owns its own drawing
vocabulary**: `vector-canvas` ships the `draw-commands` contract beside itself, rather than
complying with a neutral engine-wide drawing spec. That is already the position this codebase
took: `vector_canvas_runtime.rs` says outright that its command vocabulary "is this module's own
interface, not an engine-wide drawing protocol; a different surface module is free to speak
differently." A 3D surface could not honestly speak a 2D vector vocabulary anyway, and forcing one
would produce a lowest common denominator neither surface wants.

The contract is separate from the module only so it can be **versioned and pinned independently**,
so a producer can target the vocabulary without depending on the binary that consumes it.

### Contracts don't replace modules

A contract is a definition, not an implementation. Adding `draw-commands` doesn't retire, merge or
change what `vector-canvas` does: the module keeps its code, its window, its femtovg renderer and
its name. It gains one line saying which vocabulary it speaks. Same for every other module here.

## The manifest

`requires` stops being ambiguous by saying what each entry is:

```json
{
  "id": "vector-canvas",
  "name": "Vector Canvas",
  "version": "0.2.0",
  "requires": [
    { "contract": "draw-commands", "version": "^1", "optional": true }
  ],
  "provides": [
    { "contract": "input-state", "version": "1.0.0" }
  ]
}
```

- **`{ "contract": ... }`** — "I read whatever provides this." Resolved to whichever installed
  module provides it.
- **`{ "module": ... }`** — "I depend on this specific module." The escape hatch, for when you
  genuinely mean one implementation and not a role.
- **`{ "id": ... }`** — the current form, still accepted, still meaning `module`. Existing manifests
  keep working.
- **`provides`** is new, and it is what makes a module eligible to fill a role.

Setting both `id` and `contract` on one entry means two different things at once, so `resolve()`
rejects it and says to split them rather than silently picking one.

The two `version` fields are deliberately different kinds of thing. On a **requirement** it is a
RANGE (`^1`, `*`): what this module will accept. On a **provision** it is a single concrete
version (`1.0.0`): what this module actually speaks. Matching one against the other is what
version enforcement will mean, and it only works because they are not the same shape.

`kind` may only be `"contract"` or absent. Any other value is an error naming it, because an
unrecognised kind decides whether the thing gets RUN, and guessing "ordinary module" for something
calling itself something else spawns a process nobody asked for.

A field this version of LowArc does not read is a **warning**, not an error, and names every
offender. That covers the common case (`provdies` for `provides`, which otherwise parses fine and
silently fills no role) without making a manifest written for a newer LowArc unloadable in an older
one. Every module release would be a breaking one if it were an error.

There is no `priority` in the example because most manifests should not have one. It is a tiebreak
between modules that nothing else orders, and `requires` always wins over it, so writing one when
no tie exists states nothing. A contract never has one at all, since a contract never runs.

`optional` keeps its current meaning exactly: nothing provides it, the module runs anyway and simply
sees nothing under that key. That is how `vector-canvas` degrades to an empty window today, and it
should stay that way.

## Publishing and reading

Two addressing modes, both live at once:

- `shared["<module-id>"]`: unchanged. A module always publishes under its own id, and anything that
  `requires` that specific module reads it there.
- `shared["<contract-id>"]`: new. The engine aliases each contract a module provides onto that same
  published object.

A contract key holds an **ordered array**, one entry per provider that published this tick, each
tagged with the id it came from:

```json
"draw-commands": [
  { "from": "my-director", "draw": [ ... ] },
  { "from": "debug-overlay", "draw": [ ... ] }
]
```

An array rather than a map keyed by provider id, for one concrete reason: `serde_json`'s map is
sorted, not insertion-ordered, so a map would silently make z-order alphabetical by module id. The
array preserves run order, which is the order that actually means something.

Load order follows the contract graph the same way it follows the module graph today: a provider
runs before its consumers, so a consumer's frame sees this tick's output, not last tick's. That
property already exists and doesn't change.

## Director is a title, not a field

Nothing in the manifest says "director" any more, and nothing in the engine looks for it.

"Director" is what we *call* the module that drives a run: the one holding the state, deciding what
happens this frame, and telling the surfaces about it. It's a description of a job, the way "the
renderer" or "the physics module" is. What's formal is the contracts it speaks: a director is simply
a module that provides `draw-commands`, or `audio-cues`, or both, or neither if it drives something
else entirely.

That also settles the p5.js question. p5 fuses two jobs: the drawing API, and the host your sketch
runs inside. LowArc splits them. `vector-canvas` is the first half only: femtovg, whose API is
modelled on HTML5 Canvas, which is the same drawing model p5 wraps. A p5-shaped module here is a
**director**: something that runs a user's code and emits draw ops, with `vector-canvas` behind it.
That's a coherent thing for someone to build, and after this change they can build it without
having to name their module `director` to be allowed to.

## Stacks

A **stack** is a set of modules that work together, named as a product. A project's `requires` is
already an ad-hoc stack; the term just gives it a name and a version so it can be published,
recommended and installed as a unit.

Stacks are where names get to be artistic, because a stack is something a person chooses. Modules
and contracts stay literal, because they're tools.

### The first stack

Three contracts, four modules, and a director-shaped hole the project fills:

| Contract | Owned by | Providers | Consumers |
| --- | --- | --- | --- |
| `draw-commands` (`many`) | `vector-canvas` | anything that draws | `vector-canvas` |
| `audio-cues` (`many`) | `audio-playback` | anything that makes sound | `audio-playback` |
| `input-state` (`many`) | the stack | `vector-canvas` (canvas space), `device-input` (screen space) | anything that reacts to input |

| Module | Provides | Requires | Changes |
| --- | --- | --- | --- |
| `vector-canvas` | `input-state` | `draw-commands` (optional) | manifest only |
| `audio-playback` | — | `audio-cues` (optional) | manifest only |
| `device-input` | `input-state` | — | manifest only |
| `node-graph-runtime` | — | `input-state` (optional) | manifest only |

Every module keeps its code, its name and its job. The right-hand column is the entire scope of
this change for the four that exist.

`input-state` having two honest providers is the case that proves `providers: "many"` earns its
place. `vector-canvas` reports the pointer in canvas coordinates because it owns the window;
`device-input` reports globally in screen space, gamepads included. Both are input, neither is
wrong, and a consumer wants to choose.

The stack has no name here on purpose. Naming it is a product decision.

## The layer above: engines

A **stack** is a set of modules. An **engine** is a set of modules *and plugins*: a whole shape for
LowArc, runtime and IDE together. Downloading an engine reconfigures what the app is: which
surfaces exist, which panels appear, which file types have editors, what Run means.

**Both live in name only.** Neither is a folder layout, an install format or a container: a stack
or an engine is a name plus a list of references, recorded in a manifest. Nothing on disk moves to
join one, and a module can belong to several without being copied anywhere. That keeps this layer
free. It can be designed later without any of the work below having to anticipate it.

That's a bigger idea than this document covers, and it's mostly unbuilt: nothing in the app today
says "stack" or "engine" anywhere. Two structural notes worth recording now, since they constrain
the manifest work above:

- **An engine is what a person chooses; modules and plugins become its detail.** That inverts
  today's UI, where the Modules and Plugins pages are the top level and there is nothing above them.
- **The contract graph is the natural visualization.** Providers and consumers joined by contracts
  is a directed graph, and the most valuable thing it can show is a *hole*: a required contract
  nothing provides. That is exactly the state that produces a window that opens and stays blank
  today, with nothing anywhere telling you why.

## Migration

Deliberately additive: nothing has to change at once.

1. `provides` and the `contract`/`module` requirement forms are added; bare `{ "id": ... }` keeps
   meaning `module`, so every existing manifest still resolves.
2. `shared` gains contract aliases alongside the existing per-module-id keys. Nothing that reads by
   module id breaks.
3. The three contracts above get written as real contract modules, with their READMEs as the
   specifications, including writing down the draw-op vocabulary and the audio cue shape, which
   have never been written down anywhere but their consumers' header comments.
4. `vector-canvas` and `audio-playback` switch from `requires: director` to
   `requires: { contract: ... }`. This is the point where "the module named director" stops being a
   thing. Both are manifest edits; neither module's code changes, beyond reading its op list from
   the gathered set rather than from one fixed key.

**Requirement ranges are enforced.** `resolve()` matches every requirement's range against the
version of whatever resolved to satisfy it, using real semver rather than a hand-rolled comparator.
`"*"` still accepts anything, including a module that declares no version, since that is what every
requirement written before ranges meant anything says.

**What is not yet checked is the pairing that matters most for contracts**: whether a PROVIDER's
declared contract version satisfies a CONSUMER's range. A module requiring `draw-commands ^1` is
currently handed the commands of a provider speaking `2.0.0` without complaint. `shared` holds one
array per contract, read by every consumer, so filtering it per consumer is a real design question
rather than a missing `if`.

## What this deliberately does not do

- **No payload validation.** The engine matches names and versions; it never inspects what flows.
  Constraining the shape of a draw op is the contract document's job and the consumer's job.
- **No capability or permission system.** `requires` remains the read permission, as it is today.
- **No registry.** Contracts are modules, so they install and distribute exactly like modules do.
