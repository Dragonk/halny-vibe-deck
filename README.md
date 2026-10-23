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
| UI language | English | English + Polski |

## Features

- **Full ACP protocol** — JSON-RPC 2.0 over stdio to `vibe-acp` on a worker
  thread: `initialize`, `session/new | load | prompt | cancel`,
  `session/set_config_option` (model/mode switching), streaming
  `agent_message_chunk` / `agent_thought_chunk` / `tool_call` updates,
  tool-approval and agent-question modals.
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
  `~/.vibe/logs/session/`, with message drill-down and subagent sessions.
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
- VibeDeck is licensed under **Apache-2.0** — see [LICENSE](LICENSE).

---

*VibeDeck is an unofficial, community-made project. It is not an official
Mistral AI product and is not affiliated with, endorsed by, or supported by
Mistral AI.*
