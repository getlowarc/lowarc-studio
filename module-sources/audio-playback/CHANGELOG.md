# Changelog

All notable changes to the Audio Playback module are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.3.0 — 2026-09-09

### Changed

- Consumes the `audio-cues` contract instead of a module named `director`, gathering every provider's cue list in run order.

## 0.2.1 — 2026-09-08

### Added

- Reports a degraded start when no audio output device is available, so a run with no sound says
  why instead of silently playing nothing.

## 0.2.0 — 2026-09-07

### Changed

- Renamed from `audio` to `audio-playback`. First-party module names are literal: this one plays
  sound files and does not capture or synthesize, and the old name claimed the whole category.
  A project requiring `audio` must update its `project.json`.

## 0.1.1 — 2026-09-03

### Fixed

- On an environment with no real audio device (confirmed live on a CI runner), opening the
  default output device could hang indefinitely instead of failing quickly — freezing the whole
  run, since modules start sequentially. Opening the device is now raced against a 3-second
  timeout; a device that never opens in time is treated the same as no device at all.

## 0.1.0 — 2026-09-03

Initial release.

### Added

- Real cross-platform audio playback (rodio/cpal), broad format support via Symphonia.
- Declarative per-frame playback list (`shared.director.play`), reconciled idempotently — live
  volume/pause changes, gapless looping, `justFinished` reporting.
- Optional `director` role dependency — works with any module willing to fill it, no hardcoded
  coupling.
- Graceful degradation on a device-less environment.
