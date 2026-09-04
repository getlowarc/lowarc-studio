# Media Viewer

Views images, video, and audio in formats the browser already renders natively — no bundled
decoder, unlike Monaco/xterm. Exotic formats (RAW, PSD, HEIC, TIFF) aren't supported yet.

## Features

- Images: PNG, JPG/JPEG, GIF, WebP, BMP, ICO, SVG, AVIF.
- Video: MP4, WebM, OGV, with standard playback controls.
- Audio: MP3, WAV, FLAC, M4A, AAC, Opus, OGG, with the same playback controls, a live waveform
  visualizer while playing, and embedded album art (parsed from ID3v2 tags where present) shown
  as a thumbnail.

## Notes

Runs fully offline in a sandboxed iframe with no network access — embedded art comes only from
what's already inside the audio file itself.
