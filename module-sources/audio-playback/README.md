# Audio Playback

Plays sound files for a LowArc project, built on [rodio](https://docs.rs/rodio) (itself on
`cpal`), with broad format decoding (WAV, MP3, OGG, FLAC, and more, via Symphonia) rather than
hand-rolling a decoder. Cross-platform: Windows, macOS, Linux.

## How it works

Optionally consumes the **`audio-cues`** contract, whose specification ships beside this module
(`module-sources/audio-cues`). Any number of modules may provide it; this one gathers every
provider's list in run order, so several modules can ask for sound at once without one of them
being elected to speak for the others.

Every frame, it reads that gathered, declarative "what should be playing right now" list:

```json
[{ "handle": "music", "file": "bgm.mp3", "volume": 0.8, "loop": true, "paused": false }]
```

This is reconciled against what's currently playing, not a one-shot command queue — a director
re-publishing the same list every frame (the normal case) never restarts anything already
playing. Volume and paused state can both change live, in either direction, without restarting a
sound or losing its position; changing the file or loop flag restarts it fresh. It publishes back
`{"playing": [...], "justFinished": [...]}` so a director can learn when a one-shot sound reaches
its natural end.

A platform or environment with no real audio output device at all (a locked-down or headless CI
runner, say) doesn't fail the whole module — `playing`/`justFinished` just stay empty every frame,
the same graceful-degradation shape `input`'s own unavailable-gamepad-backend case uses. Opening
the device is raced against a 3-second timeout so a device-less environment can't hang the whole
run waiting on it.
