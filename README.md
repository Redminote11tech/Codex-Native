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

## Notes

- This project currently targets the native Linux use case first.
- The frontend bridge is implemented pragmatically: unsupported Electron behaviors are replaced with native shims where possible.
- The repository currently uses locally extracted frontend assets during development, but those assets are intentionally not part of version control.
- This repository tracks the native host implementation only. Large upstream application bundles and extracted frontend assets are excluded from git.

## Legal

Codex-Native is an independent native host project. It is not an official OpenAI desktop release.

The native Rust code in this repository is MIT-licensed. Any official frontend assets used during local development should come from the original upstream distribution and remain subject to their original licensing and terms.
