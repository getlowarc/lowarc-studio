# Changelog

All notable changes to LowArc Studio are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/). How the numbers are chosen, and why Studio's rules
differ from the rules for anything other people's code depends on, is in
[docs/versioning.md](docs/versioning.md).

Anything not yet in a tagged release goes under Unreleased as it lands, so cutting a release never
means reading back through the commit log.

## [Unreleased]

### Added

- The About box, the logo tooltip and `lowarc --version` all report the major's name alongside the
  number ("0.58.6 Daedalus"), through one `version::label()`, so no build can have three surfaces
  disagreeing about what it is.
- A manifest field this version of Studio does not read is reported by name, including one inside a
  `requires` or `provides` entry. A warning rather than an error: the same shape is what a manifest
  written for a newer Studio looks like from here.
- `resolve()` rejects a manifest that states one thing two ways, naming the module and the entry: a
  contract that also ships a `process.json`, a requirement naming both a module and a contract, and
  a `kind` that is neither `"contract"` nor absent.
- Apache-2.0 licence, a `NOTICE` covering the five bundled third-party libraries, `SECURITY.md` and
  `CONTRIBUTING.md`.
- Requirement ranges are enforced. `resolve()` matches every requirement against the version of
  whatever resolved to satisfy it, with real semver rather than a hand-rolled comparator. `"*"`
  still accepts anything, including a module that declares no version, since that is what a
  requirement written before ranges meant anything says.

### Changed

- The version number lives only in `src-tauri/Cargo.toml`. `tauri.conf.json` inherits it, and the
  frontend asks for it rather than carrying its own copy.
- `loadOrder` is now `priority`: optional, and dropped from every manifest that was only restating
  the default. It is a tiebreak between modules nothing else orders, never an order in its own
  right. A manifest still saying `loadOrder` warns and falls back rather than failing.
- `requires_rank` and `order_by_requires` were one walk under two names. Both are now `run_order`
  and `in_run_order`.
- `Dependency::is_contract` is now `names_contract`. `Manifest::is_contract` meant the opposite
  thing, and one name for both read as though a requirement could itself be a contract.
- A `provides` entry states a single concrete contract version; a requirement's `version` stays a
  range. Version enforcement is matching one against the other, so they cannot be the same shape.
- Chevrons, steppers, clear and zoom controls are drawn as inline SVG rather than borrowed from the
  font as Unicode glyphs.
- Comments cut back throughout, and most em dashes replaced with ordinary punctuation.

### Fixed

- `Manifest::read` reports why a manifest could not be read, down to the line and column, instead of
  discarding the parse error and leaving every caller to say "unreadable".

---

**0.58.6 Daedalus** is where this changelog starts, and nothing before it is listed. There has never
been a tagged release: the number was reconstructed by walking the commit history under the rules in
[docs/versioning.md](docs/versioning.md), to give the first real release something honest to follow
from. It is a starting point, not a release that happened.
