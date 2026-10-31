# Halny Vibe Deck (Rust edition) — an unofficial desktop GUI for Mistral Vibe CLI

> **⚠️ Unofficial community project.** Halny Vibe Deck is **not** an official Mistral AI
> product and is **not** affiliated with, endorsed by, or supported by Mistral AI.
> "Mistral" and "Vibe" are trademarks of their respective owners.
> Halny Vibe Deck is an independent GUI for the open-source
> **Mistral Vibe CLI** (it drives `vibe-app-server` over its JSON-RPC protocol).

**Halny Vibe Deck (Rust edition)** is a single-binary GUI for the Mistral Vibe CLI,
built with **Dioxus 0.7 (fullstack)**: the UI is compiled to WASM and served by an
embedded Rust (Axum) server in the same process. It runs in two ways:

- **Desktop app** — a native webview window (WebView2 on Windows, webkit2gtk on
  Linux) with a frameless title bar.
- **Headless server** — the exact same UI served over **HTTPS** to any browser
  (PWA-installable), for local use, LAN, or remote machines.

This is the **Rust edition** of [Dragonk/halny-vibe-deck](https://github.com/Dragonk/halny-vibe-deck),
developed on the **`next`** branch as the future replacement of the Electron/TypeScript
edition on `main`.

> ℹ️ **Status: prerelease (1.99.x).** Until the Rust edition matures, official Halny Vibe
> Deck releases come from the Electron edition on `main`. Builds from this branch are
> published as **prereleases tagged v1.99.x** — usable, but expect rough edges.

## Why the rewrite?

The Electron edition ships a whole Chromium per window; the Rust edition is a single
binary plus the OS webview, with the UI compiled to WASM and a Rust server in one
process. Lower RAM, faster startup, one binary to update — and when it reaches parity,
only this edition will be maintained.

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

- `./halny-vibe-deck` (`HalnyVibeDeck.exe`) — `--mode auto` (default): opens the desktop
  window when a display is available (always on Windows), otherwise starts headless.
- `./halny-vibe-deck --mode gui` — force the desktop window.
- `./halny-vibe-deck --mode headless` — headless server. The UI is served over
  **HTTPS at `https://127.0.0.1:8080`** with a self-signed certificate generated at
  startup (accept it once in the browser; loopback needs no login). Open it from any
  browser on the machine — or install it as a PWA.
- **Remote access** (LAN/SSH tunnel): enable it in Settings → Remote, or set
  credentials with `--set-login user pass`. The server then also listens on
  `0.0.0.0` over HTTPS with a login page: `https://<host-ip>:8080`.
- `./halny-vibe-deck-appserver` — the same headless server as a dedicated console
  binary (no webview code compiled in; the GUI is always disabled).
- `--port 1234` — change the port; `HALNY_VIBE_DECK_PORT` env var overrides it
  (legacy `MISTRALDECK_PORT` still works).

Configuration lives in `~/.config/halny-vibe-deck/config.json`; existing
`mistraldeck` / `vibedeck` configs are migrated automatically on first run.

## Features

- Real-time chat with the Mistral Vibe agent: streaming text and thinking, tool steps
  (green done / amber running / red error) with collapsible bodies, plan/todo card,
  subagent shelf, per-session state.
- Session sidebar: active/agent-running markers, stop-session (kills the agent process
  tree), pin/archive, "open elsewhere" lease badge, right-click context menu, groups by
  project.
- File explorer + preview tabs: Markdown (rendered, editable), code with syntax
  highlighting, docx/odt/pptx as real pages (fit-to-width or 1:1, always-visible
  horizontal scrollbar) with generated TOC, PDF (pdfium), spreadsheets (xlsx/ods/csv),
  images, zip listings.
- Git tab: status, diffs, branches, worktrees.
- Slash commands (the full CLI set), attach menu with the system file picker
  (files + images), drag & drop.
- Settings with Global/Project scope: providers and models edited against the full
  `vibe` CLI config schema (API keys masked), MCP servers, skills, prompts, raw TOML,
  remote (SSH) connections.
- Auto UI language (follows browser/OS) and theme; English / Polski / Deutsch /
  Français / Español / Italiano.
- Built-in updater checking the latest GitHub release.

## Development

```bash
git clone https://github.com/Dragonk/halny-vibe-deck
cd halny-vibe-deck
git checkout next

cargo check --workspace
cargo test --workspace
cargo test -p halny-vibe-deck --features server   # UI crate's server-side tests

# Desktop/web bundle via dioxus-cli (dx):
cd crates/halny-vibe-deck && dx build --fullstack --web --release

# Server binary profile used by CI bundles:
cargo build --profile server-release -p halny-vibe-deck --features server,gui
```

Workspace layout (shipped binaries: `halny-vibe-deck` / `halny-vibe-deck-appserver`):

- `crates/halny-vibe-deck` — the app itself: Dioxus 0.7 fullstack UI + Axum server
  (`server` feature: headless/remote with HTTPS + login + PWA; `gui` feature: desktop
  webview via wry/tao; `web` feature: browser WASM).
- `crates/halny-vibe-deck-appserver` — dedicated headless-server binary
  (`halny-vibe-deck-appserver`).
- `crates/halny-vibe-deck-appsrv` — the `vibe-app-server` worker: JSON-RPC connection,
  session projection, wire→event mapping.
- `crates/halny-vibe-deck-engine` — session/slot engine: parallel agent processes, event
  bus, snapshots, stop/cancel, in-app updater.
- `crates/halny-vibe-deck-vibe` — read/write of the Vibe CLI config (`~/.vibe/config.toml`,
  agents, prompts, skills), session stats.
- `crates/halny-vibe-deck-core` — shared domain types and protocol events.
- `crates/halny-vibe-deck-setup` — first-run setup: detecting/installing `vibe-app-server`.

## Versioning

- `main` — the Electron edition (official releases, 1.x).
- `next` — this Rust edition; **prereleases on the 1.99.x line** (v1.99.0, v1.99.1, …)
  until it reaches parity, at which point it replaces the Electron edition on `main`.

## License

Apache-2.0 — same as the upstream project.
