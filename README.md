# VibeDeck

> **⚠️ Unofficial community project.** VibeDeck is **not** an official Mistral AI
> product and is **not** affiliated with, endorsed by, or supported by Mistral AI.
> "Mistral" and "Vibe" are trademarks of their respective owners.
> VibeDeck is an independent desktop GUI (nakładka) for the open-source
> **Mistral Vibe CLI** (`vibe-acp`).

**VibeDeck — fast, native desktop GUI for the Mistral Vibe CLI.**

A Rust rewrite of [Dragonk/mistral-vibe-nexus](https://github.com/Dragonk/mistral-vibe-nexus)
(Electron/TypeScript, Apache-2.0) with the same goal: chat with the Mistral Vibe
coding agent from your desktop — but with **no Electron, no Chromium, no webview**.
Built with [egui](https://github.com/emilk/egui) + [wgpu](https://wgpu.rs) for
instant startup and a tiny memory footprint, in the spirit of the
[ArtCraft](https://getartcraft.com/apps) native apps.

| | mistral-vibe-nexus | VibeDeck |
|---|---|---|
| Runtime | Electron + Chromium | native Rust binary |
| Renderer | React / DOM | egui / wgpu (GPU-first) |
| Language | TypeScript | Rust |
| RAM | hundreds of MB | tens of MB |
| UI language | English | System / English + Polski |

## Features

- **1:1 Nexus look** — the original Mistral Vibe Nexus palette (dark & light, or follow the OS theme),
  frameless window with a custom titlebar (drag / ─ □ ✕, edge resize), header
  with agent status, a permanent collapsible sidebar (icon rail when collapsed),
  AI messages with the accent flame avatar and user bubbles with the original
  rounding, and the pixel-flame empty state.
- **Projects** — recent folders list in the sidebar; add or switch the working
  directory of a session in one click.
- **Session history** — sessions recorded by the CLI (`session/list`) with
  name, date and folder, resumable with one click — the full conversation
  (messages, thinking, tool calls) is replayed into the UI.
- **Fast startup** — dependency probing and file indexing happen lazily, never
  blocking the first frame. The original window icon is embedded (Linux &
  Windows), and no stray console window opens on Windows.
- **Model / mode / thinking pickers** — the toolbar reads live
  `configOptions` from vibe-acp ≥ 3.0 (`session/new`): switch the model,
  session mode (ask / accept-edits / …) and thinking level (low / high / max)
  per session — plus a working-directory chip and `@file` / `/command`
  autocomplete in the input bar.
- **Workspace dock** — a right panel with a lazy project file tree with
  previews and a Git tab (branch, changed files, per-file diffs), refreshed
  after every completed turn (in the spirit of Claude Cowork). Previews are
  real renders, not text dumps: **PDF** pages are rasterized with the bundled
  [pdfium](https://github.com/bblanchon/pdfium-binaries) library, **DOCX**
  renders as a styled document (headings, emphasis, lists, tables), **XLSX**
  opens with sheet tabs and an Excel-style grid, **PPTX** shows actual slides
  with shape geometry and embedded pictures, and Markdown supports
  **Mermaid** diagrams (rendered via [mermaid.ink](https://mermaid.ink) —
  the diagram source is sent to that public service, so don't put secrets
  in diagrams if you're offline-shy).
- **Full ACP protocol** — JSON-RPC 2.0 over stdio to `vibe-acp` on a worker
  thread: `initialize`, `session/new | load | prompt | cancel`,
  `session/set_config_option` (model/mode/thinking), streaming
  `agent_message_chunk` / `agent_thought_chunk` / `tool_call` updates,
  `usage_update` live token counters, tool-approval and agent-question modals.
- **Latest CLI config** — a round-trip-safe editor for `~/.vibe/config.toml`:
  `active_model`, `default_agent`, `theme`, `[[providers]]`, `[[models]]`,
  `[[mcp_servers]]` (http / streamable-http / stdio, auth headers, timeouts),
  `enabled_tools` / `disabled_tools`, `[tools.<name>] permission`,
  `skill_paths` / skills, `enable_telemetry` / OTel, update checks,
  notifications, system trust store, `[session_logging]`, and the read-only
  project-level `.vibe/config.toml`. **Unknown keys written by newer CLI
  versions always survive a save.**
- **Agents, prompts, skills** — manage `~/.vibe/agents/*.toml` (incl.
  subagents), `~/.vibe/prompts/*.md` and inspect discovered skills.
- **Stats dashboard** — totals and per-session cost/token/tool-call usage from
  `~/.vibe/logs/session/` (including the new `unified/` layout), with message
  drill-down, subagent sessions, **cached-token cost and cache hit rate**, and
  a live usage panel for the current session (tokens, tok/s, context, cost
  estimate from the CLI price table).
- **Setup pane** — detects/installs the CLI chain
  (`uv tool install mistral-vibe`) with one click.
- **i18n** — English and Polski, runtime switchable.
- **Zero native dependencies** — in-app folder picker and `@file` /
  `/command` autocomplete instead of heavy dialog crates.

## Prerequisites

- Rust 1.75+ (rustup recommended)
- The Mistral Vibe CLI — VibeDeck's Setup pane can install it for you, or:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh   # uv
uv tool install mistral-vibe                        # vibe + vibe-acp
```

## Build & run

```bash
cargo run            # dev build
cargo build --release # optimized (LTO, stripped) → target/release/vibedeck
```

Linux: `libgtk-3-dev`/wayland/x11 system libs required by winit/wgpu
(Debian/Ubuntu: `sudo apt install libxkbcommon-dev libwayland-dev libgtk-3-dev`).

## Architecture

```
crates/
├── vibedeck-core    # shared domain types (messages, tool calls) + ACP protocol types
├── vibedeck-acp     # JSON-RPC 2.0 client over stdio: worker thread owns the
│                    # vibe-acp process; mpsc command/event channels to the UI
├── vibedeck-vibe    # ~/.vibe layer: round-trip-safe config.toml, agents,
│                    # prompts, skills, session-log stats
├── vibedeck-setup   # dependency detection (uv / mistral-vibe / vibe-acp)
│                    # and guided install
└── vibedeck-app     # egui/eframe UI (wgpu renderer)
```

Mirrors the Electron main/renderer split of the original: the worker thread is
the "main process", the egui loop is the "renderer", and events are drained
non-blocking each frame.

## Status & roadmap

- [x] ACP client, chat, approvals, questions, modes/models
- [x] Config editor (full current CLI schema), agents/prompts/skills
- [x] Stats dashboard, setup pane, EN/PL i18n
- [ ] Packaging (`.deb`, `.AppImage`, Windows/macOS bundles)
- [ ] Session resume/branching UI polish
- [ ] More languages (i18n is a single struct per language)

## Credits & license

- Original project: [Dragonk/mistral-vibe-nexus](https://github.com/Dragonk/mistral-vibe-nexus) (Apache-2.0)
- Window move/resize behaviour follows [storytold/photocraft](https://github.com/storytold/photocraft)'s approach to frameless egui windows.
- Bundled [Noto Sans Symbols 2](https://fonts.google.com/noto/specimen/Noto+Sans+Symbols+2) for interface glyphs — SIL OFL 1.1, see `crates/vibedeck-app/assets/NotoSansSymbols2-OFL.txt`.
- File previews use the open-source crates [calamine](https://crates.io/crates/calamine) (Excel), [quick-xml](https://crates.io/crates/quick-xml) (docx/pptx parsing), [zip](https://crates.io/crates/zip) (OOXML containers), [pdfium-render](https://crates.io/crates/pdfium-render) (PDF rendering) and [pdf-extract](https://crates.io/crates/pdf-extract) (text-only PDF fallback).
- Bundled [pdfium](https://github.com/bblanchon/pdfium-binaries) binaries (Apache-2.0 / BSD) ship inside the release archives so PDF previews work out of the box; pdfium is loaded from the executable directory first.
- VibeDeck is licensed under **Apache-2.0** — see [LICENSE](LICENSE).

---

*VibeDeck is an unofficial, community-made project. It is not an official
Mistral AI product and is not affiliated with, endorsed by, or supported by
Mistral AI.*
