# Roadmap

What LowArc is working towards, roughly in the order it has to happen. No dates: this is one
person's project and a date would be a guess dressed up as a commitment. Order and dependency are
the real information here.

Everything in [CHANGELOG.md](CHANGELOG.md) under Unreleased is already done and waiting on a
release. This file is only what is not.

---

## The first release

**0.59.0 Daedalus.** Everything for it is written. The tag, the changelog check, the release notes
and the draft all work. It is blocked on one thing.

- [ ] **Updater signing key.** `cargo tauri signer generate`, then the private key and its password
      as repository secrets on `getlowarc`. Until they exist a tag push fails at the signing step
      rather than shipping something unsigned, which is the intended behaviour, not a bug.
- [ ] Tag, let the draft build, publish it.

## Signing, all of it

Three separate things that the word "signing" covers, in increasing order of cost:

- [ ] **Updater signatures** (free). The key pair above. Clients verify every update against the
      public key baked into `tauri.conf.json`. Without this there is no safe update channel at all.
- [ ] **Windows code signing** (an annual certificate, real money). Without it SmartScreen warns
      every person who runs the installer, and that warning is most of the reason a stranger does
      not install something. An EV certificate clears the warning immediately; an OV certificate
      has to build reputation first.
- [ ] **macOS notarisation** (Apple Developer Program, annual). Only once there is a macOS build to
      sign. See cross-platform below.

## The websites

Two sites, and one of them is load-bearing for the product rather than marketing.

- [ ] **lowarc.com.** The brand, what LowArc is, downloads.
- [ ] **The update endpoint.** `docs/updating.md` already specifies exactly what it has to serve:
      `GET /updates/{target}/{arch}/{current_version}`, returning either 204 or a signed manifest.
      This is a hard dependency of the updater, not a nice-to-have.
- [ ] **Module and plugin distribution.** First-party modules and plugins are deliberately not
      bundled into the installer; they are downloaded. Until the site serves them, every module and
      plugin has to be installed by hand from a folder, which means **the website gates the whole
      assemble-your-own-stack idea** that LowArc exists for. This is the single highest-leverage
      item on this list.
- [ ] **The Studio site.** Documentation, the manual, the API reference. See below.

## Documentation

There is a lot of design writing (`docs/modules.md`, `docs/pipeline.md`, `docs/updating.md`,
`docs/versioning.md`, `docs/cli.md`) and almost no reference material. Specifically missing:

- [ ] **The plugin API.** 23 `window.lowarc.*` methods exist and not one is documented outside the
      code that implements it. Anyone writing a plugin today has to read `plugin_assets.rs`.
- [ ] **The module wire protocol.** `compile`/`start`/`frame`/`stop`, `shared`/`publish`,
      `requestStop`, `degraded`. Specified only in a header comment in `process_module.rs`. This is
      the contract every third-party module depends on and it is written down nowhere a module
      author would look.
- [ ] **The manifest format.** Every field, what it means, what is enforced. Partly covered by
      `docs/modules.md`, which is a design document rather than a reference.
- [ ] **Writing a module** and **writing a plugin**: a walkthrough each, start to finish.
- [ ] **The native module C ABI**, for modules that are a dynamic library rather than a process.
- [ ] **A manual for Studio itself**: what the panels do, how a run works, what the debugger offers.

## A test game

One real game, built in LowArc, by the person who wrote LowArc. Possibly two if one cannot reach
everything.

This is not a demo. It is the only honest test of whether any of this works, and it is the thing
most likely to change the design, so it should happen earlier than it feels ready for.

- [ ] **Game one: 2D.** Exercises `vector-canvas`, `draw-commands`, `device-input`,
      `audio-playback` and `audio-cues`. Enough of a game to need state, input, sound and a loop.
- [ ] **Game two, if needed: something the first cannot reach.** 3D, or heavy asset loading, or
      whatever the first one proves is untested.

The real purpose: **the test game is what answers the open pipeline question.** Writing a game is
how you find out whether user code can be written however the author likes and still run at speed,
or whether it cannot. Nothing else will settle that argument.

## Cross-platform

Studio is Windows-only today, and deliberately so: `rust-toolchain.toml` pins
`stable-x86_64-pc-windows-msvc`, and both workflows run on `windows-latest`. Nothing in the design
is Windows-specific, but nothing has been built or tested elsewhere either.

- [ ] **Linux**, likely the cheapest second platform.
- [ ] **macOS**, which also unlocks the notarisation item above.
- [ ] Widen the CI matrix as the toolchain pin widens. The manifest format already keys update
      artifacts per platform, so adding one is a matrix entry rather than a redesign.

## Finishing the contract work

The mechanism, the manifest validation, requirement range enforcement and provider dialect tagging
are done. One piece is not.

- [ ] **The dependency graph view.** Modules, contracts and what provides what, drawn. The most
      valuable thing it can show is a **hole**: a required contract nothing installed provides.
      `resolve()` can already tell a missing module from a missing contract from a range that does
      not match from a provider speaking the wrong dialect. Four distinct things a graph could draw,
      and nothing draws them.

## Winding

[winding](https://github.com/nolanbaxter/winding) is a dependency-free WebGPU renderer for the
browser, already at 1.5.0. LowArc's only output surface today is `vector-canvas`, which is 2D.

**What is undecided is what "port" means**, and the answer depends on the pipeline question rather
than on effort:

- As a **plugin**, winding is a 3D viewport inside Studio, running in the WebView that plugins
  already run in. Nearly free, and the smallest useful version.
- As an **output module**, winding would need a native WebGPU host rather than a browser, which is
  a rewrite rather than a port.
- As an **engine** in LowArc's sense, winding sits behind a `scene` contract the way `vector-canvas`
  sits behind `draw-commands`, and user code describes a scene without naming the renderer.

- [ ] Decide which of those it is, and say why, before writing any of it.

## Smaller things worth not forgetting

- [ ] **Installing a module should not require a file picker.** Resolution already reports exactly
      what is missing by id; with the website serving modules, it could offer to fetch them.
- [ ] **Ship the `lowarc` CLI.** It exists and is tested. Nothing distributes it.
- [ ] **Full licence text in distributed folders.** Each plugin and module states Apache-2.0 and
      links it. Section 4 wants a copy to travel with the work, so whatever packages a plugin for
      download should put `LICENSE` in the folder.
- [ ] **Decide what happens to the node graph.** `node-graph-runtime` was deliberately left on the
      old `requires: { id: "input" }` form while everything else moved to contracts.

---

## The question under all of it

`docs/pipeline.md` records it and it is still open: **two models for how user code runs.** Either
LowArc translates it, or it runs natively in its own language's runtime and modules provide the
words. The second is where LowArc started.

Nothing above is wasted either way. But the answer decides what the test game looks like, what a
winding port means, and whether an interpreter module is a thing that exists at all. It should be
answered by building something real, not by more design.
