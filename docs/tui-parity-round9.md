# TUI parity audit — round 9 (1.99.15)

Goal: the Halny Vibe Deck chat fully replaces the official Mistral Vibe CLI
terminal UI. Sources: `docs/tui-capabilities-report.md` (cli-rust v3.0.3 study),
the installed `vibe` package (`app_server/protocol.py`, `models.py`,
`_skill_invocation.py`, `_turn_input.py`), and this repo's chat
(`components/chat.rs`, `components/input.rs`, `state.rs`, engine + appserver).

## Key architectural finding

The cli-rust TUI is a thin JSON-RPC client; **slash commands are executed
client-side** (`cli-rust/src/commands/*`) and translated into method calls:
`/compact` → `session/compact`, `/clear|/new` → `session/history/clear`,
`/rename` → `session/rename`, `/reload` → `config/reload`,
`/model` → `config/model/write` (sessionOnly), `/agent` → `session/agent/update`,
`!cmd` → `session/shellCommand`, `/retry` → resend. A leading `/name` in a
prompt is **skill invocation** resolved server-side
(`app_server/_skill_invocation.py`) — that part the deck already got right.

## Gaps found and fixed

| TUI feature | Was | Now |
|---|---|---|
| `/compact /clear /new /rename /reload /help` | sent to the model as text | intercepted in `state::submit` → `Engine::slash_command` → new `AgentCommand`s → worker calls with exact v3 params (`session/compact` `{sessionId, extraInstructions}`, `session/history/clear`, `session/rename` `{sessionId,title}`, `config/reload`) + resync |
| `/model /thinking /agent` | — | mapped to the existing `SetConfigOption` path (same as the chips) |
| `/retry [instr]` | — | resends the last user message (`snapshot.current_messages`), args appended |
| `!cmd` | sent to the model | `session/shellCommand` `{sessionId, command, action:"run", timeoutSeconds:600}`; output streams as history entries |
| `/` picker | builtin ACP list only | extended with the client-side commands (clear/new/rename/model/thinking/agent/retry/status/whoami/plugins/rewind/branch/loop) |
| Paste image into composer | missing | `PASTE_JS` bridge → upload to `.halny-attachments/` → `@path` marker (same as drag&drop); server inlines images as base64 `image` content blocks in `session/turn/enqueue` |
| Attach files | `@` picker + drop + ＋ menu (already) | unchanged; `@markers` resolved server-side against the session cwd (round 8), remote over SSH |
| `runtime/updated`, `turn/queueUpdated`, `turn/retrying` | DebugLog only | now set `resync_needed` so chips/picker/retry banner track the server |

## Already at parity (verified)

- streaming (entryAdded/entryUpdated deltas), thinking blocks, tool-call cards
  (incl. diff hunks, subagent cards), approvals/questions overlays (round 8
  wire fix), todo/plan card (round 8), stop (`turn/interrupt` with
  expectedTurnId), prompt queue with edit/remove (local queue + drain),
  model/mode/thinking chips incl. pre-prompt defaults (round 7/8), session
  list/resume/delete, usage + context bar, path links in messages, markdown
  + mermaid rendering, SSH remote sessions.

## Known deliberate differences

- The server-side turn queue is not mirrored item-by-item; the GUI queues
  locally and drains on turn end (same UX, edit/remove supported).
- `/rewind`, `/branch`, worktrees, loops admin, teleport, MCP/connectors
  admin, voice: not in the chat UI (loops still work via the model's cron
  tool; `/loop` passes through). Session lease ("open elsewhere") not shown.
