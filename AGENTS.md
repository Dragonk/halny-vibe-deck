# Mistral Vibe Nexus

Electron desktop app wrapping the `vibe-acp` CLI (Mistral Vibe AI coding agent).

## Architecture

Three-process Electron app:
- `src/main/` - Main process: window creation, IPC handlers, ACP manager, config/vibe-stats loading
- `src/preload/` - Preload script: typed IPC bridge exposed via `window.api`
- `src/renderer/` - React UI (Vite, Vite root is `src/renderer`, source in `src/renderer/src/`)

Output dirs: `out/main`, `out/preload`, `out/renderer`

## Commands

```bash
npm run dev          # dev:main (watch) + dev:renderer (vite) + dev:electron (nodemon)
npm run build        # build:main + build:preload + build:renderer
npm run build:main   # tsc -p tsconfig.main.json
npm run build:preload
npm run build:renderer
npm run typecheck    # tsc on all 3 tsconfig files (noEmit)
npm run dist         # build + electron-builder
npm run start        # electron . (requires built out/)
```

## External Dependencies

The app requires these on the host (auto-detected at startup):
- `vibe-acp` - Mistral Vibe agent binary (installed via `uv tool install mistral-vibe`)
- `uv` - Python package manager

The setup flow (`src/main/setup.ts`) augments PATH with Windows-specific locations:
- `%APPDATA%\uv\bin`
- `%APPDATA%\uv\tools\mistral-vibe\Scripts`
- `%USERPROFILE%\.local\bin`

Vibe session stats are read from `~/.vibe/logs/session/*/meta.json`.

## Config

App config stored in Electron's `userData` path as `config.json` (loaded by `src/main/config.ts`).

## Key IPC Channels

| Channel | Direction | Purpose |
|---------|-----------|---------|
| `window:{minimize,maximize,close}` | renderer→main | Window controls |
| `acp:start`, `acp:send-prompt`, `acp:stop` | renderer→main | ACP session control |
| `acp:event` | main→renderer | ACP events (MessageChunk, ToolCall, etc.) |
| `setup:check`, `setup:install` | renderer→main | First-run dependency setup |

## Dev Notes

- Frameless window (`frame: false`), custom title bar in Header component
- ACP manager (`src/main/acp.ts`) auto-approves all agent requests (no permission prompts)
- Development: main process watches `out/main`, electron restarts on changes
- The renderer Vite dev server runs on port 5173

## Internationalization (i18n)

Translations are located in `src/renderer/src/i18n/`:
- `en.ts`, `pl.ts`, `fr.ts`, `es.ts`, `de.ts`, `it.ts` - language files
- `index.ts` - exports `useTranslation` hook and `t()` function

**How to use translations:**
```tsx
import { useTranslation, t } from '../i18n'

// In component:
const { config } = useStore()
const tr = useTranslation(config.language)

// Static text:
<span>{tr.someKey}</span>

// With interpolation:
<span>{t(tr, 'sessionsCount', { count: 5 })}</span>
// Output: "5 sessions" (or translated equivalent)
```

**Supported languages:** `auto` (system), `en`, `pl`, `fr`, `es`, `de`, `it`

**Adding new features/changes:**
- All user-facing text must be added to every language file in `i18n/`
- Components must use `useTranslation` hook to fetch translations dynamically
- Never hardcode visible strings — always use translation keys
- Translation keys should be descriptive (e.g., `agentReady` not `r1`)
