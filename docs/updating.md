# Updating

How LowArc Studio updates itself, and how it updates everything that isn't itself. Written while the
website doesn't exist yet, so the parts that depend on it are marked as decisions rather than
described as facts.

## The layering

Three things can change independently, and they are **not** three update systems.

| Layer | What it covers | Mechanism |
| --- | --- | --- |
| 1. App | The binary, built-in plugins shipped as bundle resources | Tauri updater (this doc) |
| 2. Content | User-installed plugins and modules | App code, so updated *by* layer 1 |
| 3. Preferences | `settings.json`, projects, recents | Migration, not updating |

The thing that makes this tractable: **the machinery for layer 2 is app code**, not plugin code.
`install_plugin`, `install_module`, `AppPaths`, the manager UI: all compiled into the binary. So
"update the plugin update system" is just "update the app." There is no circular dependency to
design around.

**Layer 3 is a different problem wearing similar clothes.** You don't update `settings.json`, you
migrate it: a version field plus migration steps that run on launch after the app changes shape.
Grouping it with the other two would be a category error: its failure mode is "your preferences
silently became wrong," not "you're on an old version."

**Plugins and modules share one system.** They're structurally identical: a versioned folder with a
`manifest.json` and an id, in user data, and the UI already treats them as one implementation
parameterized by noun (`createManagerPage` / `createManagerPanel`). Splitting them would break a
symmetry the code already found.

### What an app update is allowed to touch

User data lives outside the bundle, so an app update doesn't touch it. **One exception:** built-in
plugins are refreshed from bundle resources when the bundled copy is newer
(`ensure_installed_copy_resources` / `needs_copy`).

That is intended (it's how a shipped plugin fix reaches users), but it has a consequence worth
stating out loud: **a user who edits a built-in plugin loses those edits on the next app update,
silently.** Currently accepted. If that becomes a real complaint, the fix is a marker file recording
the shipped hash, and a warning when the on-disk copy diverges from it.

## Bootstrapping: the real risk is the format, not the mechanism

The mechanism can't strand you, because it lives in the app and the app updater replaces it. The
**manifest format** can.

If a shipped version reads format A and a later release switches to format B, every client on the
old version is stranded, in precisely the component that would otherwise have rescued them.

Two rules, nearly free now and expensive to retrofit:

1. **Version the manifest, and only ever add fields.** Old clients ignore what they don't know. Never
   remove or repurpose a field an older client requires.
2. **The installer is the floor.** A user with a broken updater can always reinstall over the top.
   The Microsoft Store path requires a working standalone installer anyway, so keep that true and no
   failure is unrecoverable.

Corollary: keep the updater code boring. It's the one component whose bugs can't be fixed by
shipping a fix.

## Distribution: one channel, for everyone

Tauri does not produce MSIX. The [Microsoft Store path](https://v2.tauri.app/distribute/microsoft-store/)
is a listing that links to your own EXE/MSI: the Store never takes ownership of the package, and
their docs require the linked installer to *"handle auto-updates"* itself.

So there is no store-managed update path to defer to, and no second code path to maintain. Same
binary, same updater, whether it came from the website or the Store listing.

The Store adds three requirements to the installer, none of which change the update design:
code signed, silent-install capable (`/S` for NSIS, registered in Partner Center), and the offline
WebView2 option.

**The Mac App Store is the one genuine exception**: a real sandboxed store where self-updating is
forbidden. Not investigated, because macOS isn't a target yet (see `rust-toolchain.toml`). If it
becomes one, that needs its own design; it's the only case where the "one channel" premise breaks.

## What's wired up now

- `tauri-plugin-updater` added (desktop-targeted, beside `tauri-plugin-window-state`) and registered
  in the builder in `lib.rs`.
- `updater:default` permission in `capabilities/default.json`.
- `"createUpdaterArtifacts": true` in `tauri.conf.json`, so a bundle produces signed update artifacts.
- **Version single-sourced.** `tauri.conf.json` no longer carries `"version"`; it inherits from
  `Cargo.toml`. Verified: the built exe reports the Cargo.toml version in its file metadata. This matters because
  the updater compares versions: two copies that can disagree is a correctness bug in waiting, and
  the git tag made a third.
- `.github/workflows/release.yml`, tag-triggered, which checks the tag against `Cargo.toml` before
  building and refuses to publish a release whose contents disagree with its own name.

**The plugin is not registered in the builder yet, and must not be until the config below exists.**
An earlier version of this document claimed registering it early was harmless: that the feature
would sit inert and `check()` would error until configured. That is wrong. The plugin refuses to
initialize at all without a `plugins.updater` block:

```
PluginInitialization("updater", "Error deserializing 'plugins.updater' within your
 Tauri configuration: invalid type: null, expected struct Config")
```

which panics at startup and takes the whole app down, rather than leaving one feature unavailable.

Worth noting how it got missed: `cargo build` and `cargo test` both pass with the plugin registered
and unconfigured, because nothing about it fails until a window is actually created. Only launching
the app catches it. Add the registration line in the same change that adds the pubkey and endpoints,
and launch the app afterwards: not just build it.

## What's left, and what it depends on

### 1. Generate the signing key: yours to create and hold

```bash
cargo tauri signer generate -w ~/.tauri/lowarc-studio.key
```

This is the security root of the entire feature: **anyone holding the private key can push arbitrary
code to every user.** Deliberately not generated here. It's a long-lived credential that should be
created by you and never pass through anything else.

- Private key goes in the repository secret `TAURI_SIGNING_PRIVATE_KEY` (plus
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Never committed.
- Public key goes in `tauri.conf.json`, committed. Safe by design; it only verifies.

### 2. Hosting — **decided: lowarc.com**

The repo is private, so GitHub release assets can't be fetched anonymously by the updater. The
alternative was making releases public while the source stayed closed; **lowarc.com was chosen
instead**: no coupling to GitHub, full control over the manifest, and the site is being built
anyway.

The site doesn't exist yet, so nothing here can be wired up. What the site will need to implement is
specified below so it can be built against a fixed contract rather than reverse-engineered later.

Note this needs nothing from the webview's CSP. The updater performs its request from Rust, so
`connect-src 'self'` does not apply to it: that restriction only governs plugins.

#### The contract lowarc.com has to satisfy

The configured endpoint is a URL template. With `endpoints` set to
`https://lowarc.com/updates/{{target}}/{{arch}}/{{current_version}}`, a Windows machine on 0.58.6
requests exactly:

```
GET https://lowarc.com/updates/windows/x86_64/0.58.6
```

`{{target}}` is `windows` | `darwin` | `linux`; `{{arch}}` is `x86_64` | `aarch64` | `i686` |
`armv7`; `{{current_version}}` is the running version.

Two valid responses:

- **`204 No Content`**: already current. This is the common case and should be cheap.
- **`200`** with the manifest below: an update exists.

```jsonc
{
  "version": "0.2.0",
  "notes": "What changed.",
  "pub_date": "2026-01-01T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of the .sig file produced by the release build>",
      "url": "https://lowarc.com/downloads/lowarc-studio_0.2.0_x64-setup.exe"
    }
  }
}
```

The server is free to decide *whether* an update applies (it knows the requesting version), but the
client independently refuses anything not newer than itself, and refuses anything whose signature
doesn't verify against the baked-in public key. A compromised server cannot push code without the
private key.

Serve artifacts over HTTPS. The `url` need not be on lowarc.com. It just has to be publicly
fetchable.

#### Getting artifacts to the site

`release.yml` currently creates a **draft** GitHub release, which works as a staging area regardless
of hosting: it builds and signs, and nothing is published publicly. Once the site exists, either add
an upload step to the workflow or copy the artifacts and their `.sig` files across by hand. Nothing
about the current workflow needs to change to keep that option open.

### 3. Add the config block

```jsonc
"plugins": {
  "updater": {
    "pubkey": "<public key from step 1>",
    "endpoints": ["https://<decided in step 2>/{{target}}/{{arch}}/{{current_version}}"]
  }
}
```

Template variables: `{{target}}` (`windows`/`darwin`/`linux`), `{{arch}}` (`x86_64`, `aarch64`, …),
`{{current_version}}`. The endpoint serves:

```jsonc
{
  "version": "0.2.0",
  "notes": "…",
  "pub_date": "2026-01-01T00:00:00Z",
  "platforms": {
    "windows-x86_64": { "signature": "…", "url": "https://…/lowarc-studio_0.2.0_x64-setup.exe" }
  }
}
```

### 4. The UI

VSCode's model: a button near the searchbar, hidden until there's something to install. States:
available, then downloading, then ready to restart.

Reuse what exists rather than inventing: the toast system, `setProgress`, and the `install-progress`
event shape already used for plugin and module installs.

Deliberately not built yet. It can't be verified against a live endpoint, and an update UI that
looks right while checking the wrong place is the classic failure here. Small once the plumbing is
proven.

### 5. Restart safety: do not skip

Applying an update restarts the app. It **must** route through `confirmAppClose()`, which already
guards unsaved files and an open Draft. A dev-run in flight also needs stopping cleanly, since
`dev_run_host` is a child process that would otherwise be orphaned.

This is a developer tool holding unsaved work. A surprise restart is hostile; never auto-install
without consent.

### Still undecided

- Check cadence (on launch, then every N hours?), and prompt-before-download vs download-then-prompt.
- Whether a beta/nightly channel is wanted. Cheap to allow for in the manifest now, annoying to
  retrofit. See the additive-only rule above.
- Plugin/module compatibility across app versions. If the harness API changes, an older
  user-installed plugin can break. Eventually wants a compatibility field in the plugin manifest;
  not now, but the manifest-versioning rule is what keeps that door open.
