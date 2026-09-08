# Updating

How LowArc Studio updates itself, and how it updates everything that isn't itself. Written while the
website doesn't exist yet, so the parts that depend on it are marked as decisions rather than
described as facts.

## The layering

Three things can change independently, and they are **not** three update systems.

| Layer | What it covers | Mechanism |
| --- | --- | --- |
| 1. App | The binary, built-in plugins shipped as bundle resources | Tauri updater (this doc) |
| 2. Content | User-installed plugins and modules | App code — so updated *by* layer 1 |
| 3. Preferences | `settings.json`, projects, recents | Migration, not updating |

The thing that makes this tractable: **the machinery for layer 2 is app code**, not plugin code.
`install_plugin`, `install_module`, `AppPaths`, the manager UI — all compiled into the binary. So
"update the plugin update system" is just "update the app." There is no circular dependency to
design around.

**Layer 3 is a different problem wearing similar clothes.** You don't update `settings.json`, you
migrate it: a version field plus migration steps that run on launch after the app changes shape.
Grouping it with the other two would be a category error — its failure mode is "your preferences
silently became wrong," not "you're on an old version."

**Plugins and modules share one system.** They're structurally identical — a versioned folder with a
`manifest.json` and an id, in user data — and the UI already treats them as one implementation
parameterized by noun (`createManagerPage` / `createManagerPanel`). Splitting them would break a
symmetry the code already found.

### What an app update is allowed to touch

User data lives outside the bundle, so an app update doesn't touch it. **One exception:** built-in
plugins are refreshed from bundle resources when the bundled copy is newer
(`ensure_installed_copy_resources` / `needs_copy`).

That is intended — it's how a shipped plugin fix reaches users — but it has a consequence worth
stating out loud: **a user who edits a built-in plugin loses those edits on the next app update,
silently.** Currently accepted. If that becomes a real complaint, the fix is a marker file recording
the shipped hash, and a warning when the on-disk copy diverges from it.

## Bootstrapping: the real risk is the format, not the mechanism

The mechanism can't strand you, because it lives in the app and the app updater replaces it. The
**manifest format** can.

If a shipped version reads format A and a later release switches to format B, every client on the
old version is stranded — in precisely the component that would otherwise have rescued them.

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
is a listing that links to your own EXE/MSI — the Store never takes ownership of the package, and
their docs require the linked installer to *"handle auto-updates"* itself.

So there is no store-managed update path to defer to, and no second code path to maintain. Same
binary, same updater, whether it came from the website or the Store listing.

The Store adds three requirements to the installer, none of which change the update design:
code signed, silent-install capable (`/S` for NSIS, registered in Partner Center), and the offline
WebView2 option.

**The Mac App Store is the one genuine exception** — a real sandboxed store where self-updating is
forbidden. Not investigated, because macOS isn't a target yet (see `rust-toolchain.toml`). If it
becomes one, that needs its own design; it's the only case where the "one channel" premise breaks.

## What's wired up now

- `tauri-plugin-updater` added (desktop-targeted, beside `tauri-plugin-window-state`) and registered
  in the builder in `lib.rs`.
- `updater:default` permission in `capabilities/default.json`.
- `"createUpdaterArtifacts": true` in `tauri.conf.json`, so a bundle produces signed update artifacts.
- **Version single-sourced.** `tauri.conf.json` no longer carries `"version"`; it inherits from
  `Cargo.toml`. Verified: the built exe reports `0.1.0` in its file metadata. This matters because
  the updater compares versions — two copies that can disagree is a correctness bug in waiting, and
  the git tag made a third.
- `.github/workflows/release.yml`, tag-triggered, which checks the tag against `Cargo.toml` before
  building and refuses to publish a release whose contents disagree with its own name.

The updater is **inert**: `check()` errors until the config below exists. That's deliberate — an
honest failure rather than a silent no-op.

## What's left, and what it depends on

### 1. Generate the signing key — yours to create and hold

```bash
cargo tauri signer generate -w ~/.tauri/lowarc-studio.key
```

This is the security root of the entire feature: **anyone holding the private key can push arbitrary
code to every user.** Deliberately not generated here — it's a long-lived credential that should be
created by you and never pass through anything else.

- Private key → repository secret `TAURI_SIGNING_PRIVATE_KEY` (plus
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Never committed.
- Public key → `tauri.conf.json`, committed. Safe by design; it only verifies.

### 2. Decide where artifacts and the manifest live — **open**

The repo is private, so GitHub release assets can't be fetched anonymously by the updater. Two ways
out:

- **Public releases on a private repo.** Cheapest. Release assets become world-readable while source
  stays closed.
- **Host on the website.** More control, no coupling to GitHub, and `lowarc.com` is already an
  allowed opener URL in `capabilities/default.json`. Likely the better fit, since the site is coming
  anyway for leaderboards.

Not decided. It determines the `endpoints` value below.

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
available → downloading → ready to restart.

Reuse what exists rather than inventing: the toast system, `setProgress`, and the `install-progress`
event shape already used for plugin and module installs.

Deliberately not built yet — it can't be verified against a live endpoint, and an update UI that
looks right while checking the wrong place is the classic failure here. Small once the plumbing is
proven.

### 5. Restart safety — do not skip

Applying an update restarts the app. It **must** route through `confirmAppClose()`, which already
guards unsaved files and an open Draft. A dev-run in flight also needs stopping cleanly, since
`dev_run_host` is a child process that would otherwise be orphaned.

This is a developer tool holding unsaved work. A surprise restart is hostile; never auto-install
without consent.

### Still undecided

- Check cadence (on launch, then every N hours?), and prompt-before-download vs download-then-prompt.
- Whether a beta/nightly channel is wanted. Cheap to allow for in the manifest now, annoying to
  retrofit — see the additive-only rule above.
- Plugin/module compatibility across app versions. If the harness API changes, an older
  user-installed plugin can break. Eventually wants a compatibility field in the plugin manifest;
  not now, but the manifest-versioning rule is what keeps that door open.
