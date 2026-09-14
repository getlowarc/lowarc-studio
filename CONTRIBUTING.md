# Contributing

LowArc Studio is early and moving fast. Before writing anything substantial, open an issue and say
what you want to do. A change that cuts across the module system or the plugin boundary may collide
with work already in flight, and it is cheaper to find that out first.

Small fixes need no preamble. Send them.

Taking part here means following the [Code of Conduct](CODE_OF_CONDUCT.md). Concerns about someone's
behaviour go to <report@lowarc.com>, not into a public issue. A security vulnerability goes to
<security@lowarc.com> or GitHub's private reporting, never into a public issue either; see
[SECURITY.md](SECURITY.md).

## Before you push

These three must pass. CI runs the same three on every push and pull request.

```bash
cd src-tauri
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

`cargo test` includes end-to-end tests that spawn real subprocess and native modules, and launch a
real exported build. Nothing there needs network access or an external checkout.

There is no `rustfmt.toml` and `cargo fmt` is not enforced. This codebase deliberately runs wider
than rustfmt's 100-column default.

## Building the frontend

There isn't one. `frontendDist` points at `src/`, which means **the frontend is compiled into the
binary**. Editing a file under `src/` and relaunching the exe runs the old code; you have to
`cargo build` again. This catches everyone once.

## What the code reads like

Match what is already there rather than a general style guide.

- **Comments explain why, not what.** If a comment restates the line below it, delete it. If it
  explains a decision, a constraint, or something that will look wrong to the next reader, keep it
  and say the reason.
- **No emoji, and no Unicode glyph standing in for an icon.** Icons are inline SVG using
  `viewBox="0 0 16 16"` and `currentColor`. A glyph inherits font fallback and a missing one
  renders as nothing, which has already happened here once.
- **Comments stay shorter than the thing they explain.**

## Commits

A subject line saying what changed, then a couple of lines on why if the why is not obvious.
Present tense, no trailing period, no prefix tags.

## Versioning

Studio's version and the version of anything other people's code depends on follow different rules.
Read [docs/versioning.md](docs/versioning.md) before bumping either.

Do not bump Studio's version in a pull request. Add a line to `## [Unreleased]` in
[CHANGELOG.md](CHANGELOG.md) instead, if the change is one a user would notice. The number moves
only when a release is cut, and Unreleased is what decides by how much.

## Plugins and modules

A plugin renders in a sandboxed iframe and reaches the host only through the documented
`window.lowarc` surface. A module is a separate process speaking one JSON object per line. Neither
is meant to be special-cased in the host: if yours needs a host change to work, that change should
be one every plugin or module can use. Say so in the issue.

See [docs/modules.md](docs/modules.md) for how modules, contracts and stacks fit together, and
[docs/pipeline.md](docs/pipeline.md) for how a project gets from source to screen. The pipeline doc
has open questions in it, marked as such. They are open.

## Licence

Contributions are accepted under Apache-2.0, the same licence as the project. See [LICENSE](LICENSE).
