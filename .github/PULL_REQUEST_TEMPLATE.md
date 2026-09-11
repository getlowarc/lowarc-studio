## What this changes

<!-- And why. If it closes an issue, say "Closes #N". -->

## How you know it works

<!-- What you ran, or what you clicked. "Builds clean" is not the same as "works". -->

## Checklist

- [ ] `cargo build`, `cargo test` and `cargo clippy --all-targets -- -D warnings` all pass
- [ ] Launched the app and used the thing that changed, if it touches the frontend (`src/` is
      compiled into the binary, so a rebuild is required before the change is even running)
- [ ] No emoji, and no Unicode glyph standing in for an icon
- [ ] Studio's version in `Cargo.toml` is untouched; that happens at release
