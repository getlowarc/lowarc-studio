# Run automatically by `cargo tauri build` (see tauri.conf.json's build.beforeBundleCommand) right
# after compiling, right before packaging. Cargo already built every one of this workspace's
# binaries into target/release/ as a normal side effect of the release build cargo tauri build
# itself triggers first — lowarc-studio.exe (the main app, found and bundled correctly via
# Cargo.toml's default-run) plus three backend binaries that need to end up somewhere OTHER than
# next to the main exe for an installed copy to actually work:
#
#   - file_explorer_backend.exe / terminal_backend.exe belong INSIDE their own plugin folders
#     (that's where AppPaths expects a plugin's own backend to live) — copied here the exact same
#     way app_paths.rs's ensure_builtin_plugin_binaries() already copies them for a dev run, just
#     done once at build time instead of on every dev launch.
#   - native_module_host.exe isn't part of any plugin, so it goes into runtime-helpers/ instead — a
#     small resource folder bundle.resources also ships, that
#     AppPaths::ensure_installed_copy_resources() unpacks into an installed copy's own per-user
#     data folder on first run (see that function's own comment for the full story of why an
#     installed copy needs a first-run unpack step at all, unlike a source checkout).
#
# Both destinations feed tauri.conf.json's bundle.resources, which is what actually gets these
# files (and the rest of plugins/, including Monaco's ~24MB vendored bundle) into the installer —
# this script's only job is making sure they're PRESENT on disk before that packaging step reads
# from them.
#
# Windows-only for now, matching how this project has actually been built and tested so far
# (Cargo's own portable-pty dependency is the only genuinely cross-platform-tested piece) — a real
# multi-platform release process would need this rewritten per-platform, not attempted here.

$ErrorActionPreference = "Stop"
$release = Join-Path $PSScriptRoot "target\release"

function Copy-BackendBinary($binaryName, $destDir) {
  $src = Join-Path $release "$binaryName.exe"
  if (-not (Test-Path $src)) {
    throw "prepare-bundle.ps1: expected $src to exist (cargo tauri build should have compiled it already)"
  }
  New-Item -ItemType Directory -Force -Path $destDir | Out-Null
  Copy-Item $src (Join-Path $destDir "$binaryName.exe") -Force
}

Copy-BackendBinary "file_explorer_backend" (Join-Path $PSScriptRoot "..\plugins\file-explorer")
Copy-BackendBinary "terminal_backend" (Join-Path $PSScriptRoot "..\plugins\terminal")
Copy-BackendBinary "native_module_host" (Join-Path $PSScriptRoot "runtime-helpers")

# lowarc-bootstrap.exe joins runtime-helpers/ the exact same way native_module_host.exe does above
# — same destination, same generic unpack step on the installed-copy side (AppPaths::
# ensure_installed_copy_resources -> unpack_installed_resources in app_paths.rs, which copies
# every FILE under runtime-helpers/ with no per-name special-casing, so nothing there needed to
# change for this to just work). What's different is where the binary comes from: it isn't part of
# this workspace, it's `lowarc`'s own Bootstrap crate, in the sibling repo checked out next to this
# one — this is the one place in the packaging process that dependency belongs. Building it here,
# ONCE, at package time, is what lets export/bootstrap_source.rs stop needing that sibling checkout
# at EXPORT time (see its own header comment) — an installed copy just uses the bundled binary this
# produces, the same as any other end user's machine, no source checkout in sight. A packaging
# machine without the sibling repo present still produces a working bundle for everything else;
# only the export feature would fail at runtime with export/bootstrap_source.rs's own clear error,
# same as it already does today when neither the sibling checkout nor a bundled copy exists.
$siblingBootstrap = Join-Path $PSScriptRoot "..\..\lowarc\Bootstrap"
if (Test-Path (Join-Path $siblingBootstrap "Cargo.toml")) {
  Write-Host "prepare-bundle.ps1: building lowarc-bootstrap.exe from the sibling lowarc checkout..."
  & cargo build --release --manifest-path (Join-Path $siblingBootstrap "Cargo.toml")
  if ($LASTEXITCODE -ne 0) {
    throw "prepare-bundle.ps1: building lowarc-bootstrap.exe failed (exit code $LASTEXITCODE)"
  }
  $builtBootstrap = Join-Path $siblingBootstrap "target\release\lowarc-bootstrap.exe"
  if (-not (Test-Path $builtBootstrap)) {
    throw "prepare-bundle.ps1: expected $builtBootstrap to exist after building it"
  }
  $helpersDir = Join-Path $PSScriptRoot "runtime-helpers"
  New-Item -ItemType Directory -Force -Path $helpersDir | Out-Null
  Copy-Item $builtBootstrap (Join-Path $helpersDir "lowarc-bootstrap.exe") -Force
} else {
  Write-Warning "prepare-bundle.ps1: no sibling lowarc checkout found at $siblingBootstrap — this bundle will ship without lowarc-bootstrap.exe, so its export feature won't work until one is added to runtime-helpers/ some other way."
}
