# Codex-Native

Codex-Native is a native Linux shell for the Codex desktop frontend. It replaces the Electron host with a Rust application built on GTK and WebKitGTK, while reusing the official frontend assets locally and talking to Codex through the Codex CLI app-server bridge.

The goal is simple: keep the existing Codex desktop look and behavior, but run it in a lighter native shell instead of bundling Chromium and Node as the app runtime.

## Status

This repository is an early native-host implementation focused on Linux first.

- Native GTK/WebKitGTK window shell is working
- Codex CLI app-server bridge is wired in
- Core frontend IPC and fetch shims are implemented in Rust
- Local plugin icon loading is supported through the native file bridge

The frontend itself is not redistributed here. You extract it locally from an official Codex desktop build and point the shell at that extracted `webview` directory.

## What It Does

- Runs the Codex desktop frontend inside a native GTK/WebKitGTK window
- Bridges the frontend's Electron-style IPC and fetch calls in Rust
- Uses Codex CLI as the backend bridge for chat, auth state, config, and app-server flows
- Includes a small ASAR library and CLI for inspecting and extracting frontend assets

## Current Repo Layout

```text
crates/
  codex-archive/   ASAR reader and extractor
  codex-native/    native shell and frontend bridge
```

## Why This Repo Exists

Most community Codex desktop ports keep Electron in the stack. This repo is for a native-first approach:

- Rust for the host runtime
- GTK/WebKitGTK for the window and webview
- Codex CLI for backend integration
- No Electron dependency in the shipped shell

## What Is In Scope

This repository contains the native host code and the ASAR tooling.

It does not ship the official frontend assets. Those should be extracted locally from an official Codex desktop build and supplied at runtime. That keeps the repository focused on the native implementation and avoids redistributing bundled upstream assets.

## Quick Start

### Requirements

- Rust toolchain
- GTK and WebKitGTK development libraries available on your system
- Codex CLI installed and working
- A locally extracted Codex frontend directory containing `webview/index.html`

### Typical Flow

1. Obtain an official Codex desktop build.
2. Extract `app.asar` locally.
3. Run the native shell against the extracted `webview` directory.
4. Let Codex CLI handle backend-side runtime and account flows.

### Extract an ASAR

```bash
cargo run -p codex-native -- extract-asar /path/to/app.asar ./extracted/app-asar
```

You can also inspect or print files:

```bash
cargo run -p codex-native -- list-asar /path/to/app.asar
cargo run -p codex-native -- print-asar-file /path/to/app.asar webview/index.html
```

### Run the Native Shell

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 cargo run -p codex-native -- run-shell ./extracted/app-asar/webview
```

If your setup does not need that environment override, you can run the same command without it.

If you omit the `webview` path, `codex-native` will try these locations in order:

- `CODEX_NATIVE_WEB_ROOT`
- `../share/codex-native/webview` relative to the installed executable
- `./extracted/app-asar/webview`
- `/usr/share/codex-native/webview`
- `/usr/local/share/codex-native/webview`

That makes packaged installs simpler because the binary can find the installed frontend assets on its own.

## Packaging

An Arch-first package definition lives in [packaging/aur/PKGBUILD](/home/jade/CodexDesktop/packaging/aur/PKGBUILD). It does four things:

- Builds the native Rust shell from this repository
- Downloads the pinned official macOS Codex bundle
- Extracts `app.asar` locally and installs the `webview` assets under `/usr/share/codex-native/webview`
- Installs a desktop launcher and launcher icon for Linux

The desktop launcher script is [packaging/aur/codex-native-launcher](/home/jade/CodexDesktop/packaging/aur/codex-native-launcher). On Wayland it defaults `WEBKIT_DISABLE_DMABUF_RENDERER=1`, because that has been the most reliable setup on Hyprland during testing.

The Linux package icon is sourced from the extracted frontend asset in `webview/assets/app-*.png`. The upstream macOS bundle still declares `electron.icns` as its native app icon in `Contents/Info.plist`, but that icon is shipped outside `app.asar`.

## Tracking Upstream Frontend Releases

OpenAI is shipping the macOS Codex bundle quickly right now. The official appcast showed five releases between April 16 and April 18, 2026, including two releases on April 18 alone. That means frontend drift is normal unless the package metadata is maintained intentionally.

This repo now includes two native maintenance scripts:

- `scripts/check-codex-upstream.sh`
- `scripts/bump-codex-frontend.sh`

Check the latest official macOS bundle:

```bash
./scripts/check-codex-upstream.sh
```

Bump the pinned frontend version and checksum in the AUR packaging files:

```bash
./scripts/bump-codex-frontend.sh --latest
```

That updates:

- [packaging/aur/PKGBUILD](/home/jade/CodexDesktop/packaging/aur/PKGBUILD)
- [packaging/aur/.SRCINFO](/home/jade/CodexDesktop/packaging/aur/.SRCINFO) when `makepkg` is available

The intended upkeep loop is simple:

1. Check the official appcast.
2. Run the bump script when the version changes.
3. Test the native shell against the new frontend bundle.
4. Commit the packaging update and push GitHub plus AUR.

For the GitHub source repository, [.github/workflows/sync-upstream-frontend.yml](/home/jade/CodexDesktop/.github/workflows/sync-upstream-frontend.yml) now checks the official appcast every six hours and commits updated AUR metadata automatically when the pinned frontend version changes.

The AUR repo is now prepared for separate automation as well. [.github/workflows/publish-aur.yml](/home/jade/CodexDesktop/.github/workflows/publish-aur.yml) will publish the AUR package repository after packaging changes, but only once the GitHub repo has an `AUR_SSH_PRIVATE_KEY` secret with a key that is authorized for your AUR package.

The sync is intentionally narrow. It publishes only the packaging files from [packaging/aur](/home/jade/CodexDesktop/packaging/aur) through [scripts/sync-aur-repo.sh](/home/jade/CodexDesktop/scripts/sync-aur-repo.sh), instead of mirroring the full source tree into AUR.

## Notes

- This project currently targets the native Linux use case first.
- The frontend bridge is implemented pragmatically: unsupported Electron behaviors are replaced with native shims where possible.
- The repository currently uses locally extracted frontend assets during development, but those assets are intentionally not part of version control.
- This repository tracks the native host implementation only. Large upstream application bundles and extracted frontend assets are excluded from git.

## Legal

Codex-Native is an independent native host project. It is not an official OpenAI desktop release.

The native Rust code in this repository is MIT-licensed. Any official frontend assets used during local development should come from the original upstream distribution and remain subject to their original licensing and terms.
