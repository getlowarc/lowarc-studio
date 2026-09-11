# Audio Cues

A contract, not a module. It has no process and never runs. Installing it puts this specification
and its version in the module store, so a module that wants sound and a module that plays sound can
agree without either depending on the other's binary.

Owned by `audio-playback`.

## Providing it

Declare it, then publish a `play` array each frame under your own id:

```json
{ "provides": [{ "contract": "audio-cues", "version": "^1" }] }
```

```json
{ "publish": { "play": [ { "handle": "music", "file": "bgm.mp3", "volume": 0.8, "loop": true, "paused": false } ] } }
```

Any number of modules may provide this at once; a player concatenates every provider's list in run
order.

## Consuming it

`shared["audio-cues"]` is an array with one entry per provider, each tagged with its origin:

```json
[ { "from": "my-director", "play": [ ... ] } ]
```

## The cue

| Field | Meaning |
| --- | --- |
| `handle` | Your name for this sound. Identity across frames: the same handle means the same sound. |
| `file` | Path relative to the project root. |
| `volume` | 0–1, default 1. Changes live without restarting. |
| `loop` | Default false. |
| `paused` | Default false. Changes live, keeping position: not a stop and restart. |

**This list is declarative: it describes what should be sounding right now, not what to start.**
That distinction is the whole design. Re-publishing an unchanged list every frame is a no-op rather
than a re-trigger, so the shape is idempotent by construction: a provider states the world it
wants and never has to track what it already did. Dropping a handle from the list stops that sound.

A player reports back under its own id which non-looping handles reached their natural end, so a
provider can learn a one-shot finished without polling.

## Versioning

Adding an optional field is a minor version. Removing one, or changing what an existing field
means, is a major version. A consumer pins with `"version": "^1"`.
