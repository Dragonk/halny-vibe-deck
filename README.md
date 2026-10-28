# Halny Vibe Deck (Rust edition) — an unofficial desktop GUI for Mistral Vibe CLI

> **⚠️ Unofficial community project.** Halny Vibe Deck is **not** an official Mistral AI
> product and is **not** affiliated with, endorsed by, or supported by Mistral AI.
> "Mistral" and "Vibe" are trademarks of their respective owners.
> Halny Vibe Deck is an independent desktop GUI for the open-source
> **Mistral Vibe CLI** (`vibe-app-server`).

**Halny Vibe Deck (Rust edition) — fast GUI for the Mistral Vibe CLI, built with Rust.**

This is the **Rust edition** of [Dragonk/halny-vibe-deck](https://github.com/Dragonk/halny-vibe-deck),
developed on the **`next`** branch as the future replacement of the Electron/TypeScript
edition on `main`. It is built with **Dioxus 0.7 (fullstack)** — a native desktop webview
on Linux/Windows plus an optional headless server mode you can reach from any browser.

> ℹ️ **Status: prerelease (1.99.x).** Until the Rust edition matures, official Halny Vibe
> Deck releases come from the Electron edition on `main`. Builds from this branch are
> published as **prereleases tagged v1.99.x** — usable, but expect rough edges.

## Why the rewrite?

The Electron edition ships a whole Chromium per window; the Rust edition is a single
binary plus the OS webview (WebView2 / webkit2gtk), with the UI compiled to WASM and a
Rust server (Axum) in one process. Lower RAM, faster startup, one binary to update —
and when it reaches parity, only this edition will be maintained.

## Download

Grab the latest **v1.99.x prerelease** from the
[releases page](https://github.com/Dragonk/halny-vibe-deck/releases):

| Platform | File |
|---|---|
| Linux (x86_64, glibc) | `halny-vibe-deck-vX.Y.Z-linux-x64.tar.gz` |
| Windows (x86_64) | `halny-vibe-deck-vX.Y.Z-windows-x64.zip` |
| Linux headless server | `halny-vibe-deck-appserver-vX.Y.Z-linux-x64.tar.gz` |
| Windows headless server | `halny-vibe-deck-appserver-vX.Y.Z-windows-x64.zip` |

The Windows zip contains `HalnyVibeDeck.exe` (+ `WebView2Loader.dll`, `pdfium.dll`, `public/`);
the Linux tarball contains `halny-vibe-deck` (+ `libpdfium.so`, `public/`). No installer —
unpack anywhere and run.

## Requirements

- **Linux:** webkit2gtk 4.1 runtime — Debian/Ubuntu:
  `sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0`; glibc-based distro.
- **Windows:** 10/11 with WebView2 (preinstalled on current systems; otherwise
  install it from Microsoft).
- **Vibe CLI / `vibe-app-server`** — see the first-run Setup in the app.

## Usage

- `./halny-vibe-deck` (`HalnyVibeDeck.exe`) — auto: opens the desktop window
  (on Windows always; on Linux when a display is available), otherwise starts a headless server.
- `./halny-vibe-deck --mode gui` — force the desktop window.
- `./halny-vibe-deck --mode headless` — headless server; the UI is at
  `http://127.0.0.1:8080` (login required).
- `./halny-vibe-deck-appserver` — the same headless server as a dedicated console
  binary (no webview code compiled in; GUI always disabled).
- `--port 1234` / `--set-login user pass` — configure the remote server.
- `HALNY_VIBE_DECK_PORT` env var overrides the port (legacy `MISTRALDECK_PORT` still works).

Configuration lives in `~/.config/halny-vibe-deck/config.json`; existing
`mistraldeck` / `vibedeck` configs are migrated automatically on first run.

## Features

- Real-time chat with the Mistral Vibe coding agent (ACP): streaming responses,
  tool calls color-coded (green done / amber running / red error) with collapsible bodies.
- Project sidebar with collapsible groups, busy spinner + cancel, context menu.
- File and diff previews as tabs (incl. PDF via pdfium), resizable panels.
- Settings with Global/Project scope for skills and prompts; auto UI language
  (follows browser/OS) and auto color mode; English / Polski / Deutsch / Français /
  Español / Italiano.
- Frameless desktop window with custom title bar (minimize / maximize / close).
- PWA-ready headless mode (installable from the browser), HTTPS-optional login page.
- Built-in updater checking the latest GitHub release.

## Development

```bash
git clone https://github.com/Dragonk/halny-vibe-deck
cd halny-vibe-deck
git checkout next

cargo check --workspace
cargo test --workspace
cargo test -p mistraldeck --features server

# Desktop/web bundle via dioxus-cli (dx):
cd crates/mistraldeck && dx build --fullstack --web --release

# Server binary profile used by CI bundles:
cargo build --profile server-release -p mistraldeck --features server,gui
```

Workspace layout (crate names are historical; the project is Halny Vibe Deck):

- `crates/mistraldeck` — the app itself: Dioxus 0.7 fullstack UI (server feature:
  Axum server + login + PWA; gui feature: desktop webview).
- `crates/mistraldeck-appserver` — dedicated headless-server binary (`halny-vibe-deck-appserver`).
- `crates/mistraldeck-app` — egui frontend (legacy GUI shell).
- `crates/mistraldeck-engine` — chat/session engine, config, in-app updater.
- `crates/mistraldeck-acp` — ACP client for the Mistral Vibe CLI.
- `crates/mistraldeck-vibe` — bindings to the `vibe` CLI/agent protocol.
- `crates/mistraldeck-core` — shared types and utilities.
- `crates/mistraldeck-setup` — first-run setup / login flows.

## Versioning

- `main` — the Electron edition (official releases, 1.x).
- `next` — this Rust edition; **prereleases on the 1.99.x line** (v1.99.0, v1.99.1, …)
  until it reaches parity, at which point it replaces the Electron edition on `main`.

## License

Apache-2.0 — same as the upstream project.
