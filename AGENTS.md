# AGENTS.md

This file provides guidance to AI Agents when working with code in this repository.

## Commands

```bash
# Development (hot reload)
npm run dev

# Build all processes
npm run build

# Type checking (no emit)
npm run typecheck

# Build installers
npm run dist:win      # Windows NSIS installer + portable
npm run dist          # Default cross-platform build
```

There are no automated tests in this project.

## Architecture

Electron app with three separate processes:

**Main Process** (`src/main/`)
- `index.ts` — app entrypoint, all IPC handlers, window setup
- `acp.ts` — spawns and manages the `vibe-acp` subprocess via JSON-RPC 2.0 over stdio
- `config.ts` — persists app settings to Electron userData (`config.json`)
- `vibeConfig.ts` — reads/writes `~/.vibe/config.toml`
- `vibeStats.ts` — parses session metadata from `~/.vibe/logs/session/*/meta.json`
- `setup.ts` — first-run dependency detection/installation (`uv`, `mistral-vibe`)

**Preload** (`src/preload/index.ts`) — typed IPC bridge exposing `window.api` to the renderer via contextBridge. Groups: `window`, `config`, `dialog`, `acp`, `stats`, `setup`, `fs`, `shell`, `vibe`.

**Renderer** (`src/renderer/src/`) — React 18 + Vite, state via Zustand (`store.ts`).
- Three views: Chat, Settings, Stats
- `MessageBubble.tsx` handles markdown, code blocks, thinking blocks, and tool call rendering
- `InputBar.tsx` includes @ file picker for file attachment

## Key Patterns

**Message flow:** User input → `store.addUserMessage` + `window.api.acp.sendPrompt` → ACP emits `acp:event` on main → renderer store updates (streaming via `appendChunk`, finalized via `finalizeAiMessage`).

**IPC:** All renderer↔main communication goes through `window.api` (preload bridge). Main→renderer events use `webContents.send('acp:event', ...)`.

**Session model:** Local sessions in Zustand store, optionally linked to a vibe backend session via `acpSessionId`. Sessions can be imported from existing vibe sessions.

**State:** All app state in `src/renderer/src/store.ts` (Zustand). Config is loaded from main via IPC on startup and kept in sync.

## Internationalization

All UI strings must use the translation system — no hardcoded strings in components.

```ts
const { t, tr } = useTranslation(config.language);
// Usage:
t(tr, 'key')
t(tr, 'key', { count: 5 })  // with interpolation
```

Translation files: `src/renderer/src/i18n/{en,pl,fr,es,de,it}.ts`. Add new keys to all locale files.

## TypeScript Setup

Three separate tsconfig files (not inheriting from each other):
- `tsconfig.main.json` — CommonJS, targets `out/main`
- `tsconfig.preload.json` — CommonJS, targets `out/preload`
- `tsconfig.renderer.json` — ESNext, Vite, targets `out/renderer`

Path alias: `@renderer` → `src/renderer/src`

## Runtime Dependencies

Requires `vibe-acp` binary (installed via `uv tool install mistral-vibe`). On Windows, PATH is augmented at runtime to include `%APPDATA%\uv\bin`, `%APPDATA%\uv\tools\mistral-vibe\Scripts`, and `%USERPROFILE%\.local\bin`.

**Supported languages:** `auto` (system), `en`, `pl`, `fr`, `es`, `de`, `it`

**Adding new features/changes:**
- All user-facing text must be added to every language file in `i18n/`
- Components must use `useTranslation` hook to fetch translations dynamically
- Never hardcode visible strings — always use translation keys
- Translation keys should be descriptive (e.g., `agentReady` not `r1`)
