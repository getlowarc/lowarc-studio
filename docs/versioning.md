# Versioning

There are two different things here that need numbering, and they do not follow the same rules.

## Studio, the application

`0.58.6 Daedalus`. The number lives in exactly one place, `src-tauri/Cargo.toml`. Everything else
derives from it:

| Surface | Reads |
| --- | --- |
| Installer, exe file metadata, updater baseline | `tauri.conf.json` has no `version` key, so Tauri takes Cargo's |
| About box, logo tooltip | the `app_version` command, which calls `version::label()` |
| `lowarc --version` | `version::label()` |
| Release workflow | refuses to publish when the git tag and `Cargo.toml` disagree |

The major's **name** is the one thing Cargo cannot hold, so it lives in `NAMES` in
`src-tauri/src/version.rs`. Majors get names; minors and patches do not.

Bumping a release is one edit to `Cargo.toml`, then `git tag v<number>`.

### What each level means

Levels are chosen by the **nature of the change**, not by whether anything broke:

- **major** structure and architecture changed, usually alongside features. Reset minor and patch,
  and give the new major a name.
- **minor** features added or removed. Reset patch.
- **patch** fixes, debugging, and anything too small to be a feature.

Major is pinned at `0` through the beta, so architecture currently has nowhere of its own to go and
lands in minor with everything else. That resolves at 1.0.

This is deliberately *not* semver's definition, which keys on whether existing consumers break.
Nothing consumes Studio, so compatibility is not the useful axis for it. Compatibility is exactly
the right axis for everything in the next section.

## Everything other people's code depends on

A module, a contract, the manifest format, the wire protocol and the CLI's flags are all public
API. Someone else's project pins them and keeps working, or does not. Those use semver as written:

- **major** an existing consumer breaks. A field removed or renamed, a meaning changed, a draw op
  that now takes different arguments.
- **minor** something added that an existing consumer can ignore. A new optional field, a new op.
- **patch** the behaviour was always meant to be this and now is.

`requires: [{ "contract": "draw-commands", "version": "^1" }]` has to be answerable without asking
a human, which is the whole reason this half cannot follow the same rules as Studio's own number.

Each of these carries its own version, independent of Studio's:

| Artifact | Where its version lives |
| --- | --- |
| A module | `version` in its `manifest.json` |
| A contract | `version` in the contract module's `manifest.json` |
| Manifest format | not yet versioned. See `docs/updating.md` on stranding. |
| Module wire protocol | not yet versioned |
