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
