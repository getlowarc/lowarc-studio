# Changelog

All notable changes to the Media Viewer plugin are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.2.1 - 2026-09-11

### Changed

- The zoom controls are drawn as SVG rather than borrowed from the font as Unicode glyphs, which
  render as nothing wherever the font lacks them.

## 0.2.0 - 2026-09-03

### Added

- Audio playback (MP3, WAV, FLAC, M4A, AAC, Opus, OGG) with standard controls, a live waveform
  visualizer while playing, and an embedded-art thumbnail parsed from ID3v2 tags where present.

### Changed

- `.ogg` moved from the video extension list to the audio one; `.ogv` stays video.

## 0.1.0 - 2026-08-28

Initial release.

### Added

- Native image viewing (PNG, JPG/JPEG, GIF, WebP, BMP, ICO, SVG, AVIF).
- Native video playback (MP4, WebM, OGV).
