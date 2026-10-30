# TUI parity audit — round 10 (1.99.16): verified against the INSTALLED CLI, not just reports

Sources used this round (the old reports were v3.0.3; the installed package is
**vibe 3.0.4**):

- `vibe/_bin/vibe-rs` (the actual cli-rust TUI binary, with debug info) —
  the real slash-command set extracted from it:
  `agents branch clear compact config connect delete diff exit export feedback
   fork help leanstall login logout loop model permissions plugins quit reload
   rename resume retry rewind status teleport theme trust update version whoami`
  plus the ACP registry (`help compact reload log mcp teleport proxy-setup
   retry leanstall unleanstall data-retention`) and `/skill` invocations.
- `vibe/app_server/protocol.py` + `models.py` (3.0.4) — exact wire param shapes:
  `SessionForkParams`, `SessionRewindParams`/`SessionRewindReadParams`,
  `SessionPinParams`/`SessionArchiveParams`/`SessionMarkAsSeenParams`,
  `SessionLeaseUpdate {leaseHolder}`, `SessionShellCommandParams`,
  `PublicChildSession`, `SessionListParams {pinned, includeArchived}`,
  `PublicSession {pinnedAt, archivedAt, isUnseen, worktree}`.
- notifications: `session/childSessionUpdated`, `session/lease/updated|ended`,
  `turn/queueUpdated`, `turn/retrying`, `runtime/updated`.

## Implemented this round

| Feature (TUI) | Wire | Implementation |
|---|---|---|
| `/branch` / `/fork` — fork session at a checkpoint | `session/fork` `{sourceSessionId, entryId?, attach:true}` | `/branch` opens a checkpoint picker modal (history entries with wire ids); confirm → `AgentCommand::SessionFork` → worker applies the returned `state` as a snapshot (GUI switches to the fork) + refreshes the shelf |
| `/rewind` — rewind to a checkpoint | `session/rewind` `{sessionId, entryId, restoreFiles}` + `session/rewind/read` `{hasFileChanges, paths}` | same picker modal; shows affected files from `rewind/read` (RewindInfo event) + "restore file changes" checkbox; response state repaints the chat |
| Subagent list | `PublicSessionState.childSessions` + `session/childSessionUpdated` | parsed into `ChatSession.subagents` (`SubagentView`), rendered as a clickable chip strip above the input (click opens the child transcript); live updates via the new `ChildSessionUpdated` event |
| Session pin / archive / unread | `session/pin`, `session/archive`, `session/markAsSeen`, `session/list {includeArchived:true}` | context menu on backend session rows (pin 📌 / archive 📦 / mark read / delete from disk); badges on rows; pinned sort first, archived dimmed at the bottom; `session/delete` for permanent delete |
| Lease "open elsewhere" | `session/lease/subscribe` + `session/lease/updated\|ended` | subscribed on every SessionReady; badge "open elsewhere" in the chat |
| Remaining slash commands | — | `/status` (session stats), `/version`, `/delete`, `/update` (app update check), `/diff` (opens dock Git tab), `/theme` (cycles), `/config` `/permissions` `/connect` `/agents` `/plugins` `/mcp` (open Settings), `/resume` (hint), `/quit` `/exit` (window.close); picker lists the full TUI set |
| `turn/queueUpdated`, `turn/retrying`, `runtime/updated` | notifications | now force a resync (were debug-log only) |

## Verified already at parity (no changes needed)

prompt queue w/ edit+remove, streaming/thinking/tool cards, approvals +
user questions overlays, stop (`turn/interrupt`), todo/plan card, @/drop/paste
attachments incl. image content blocks, model/mode/thinking chips (also
pre-prompt), multi-session sidebar (local + SSH mirrors + remote sessions),
settings (agents/skills/tools/TOML/server), SSH connections management,
file dock (tree/preview/edit/git), stats/history views, updater.

## Still intentionally GUI-absent (terminal-only or Desktop-only per the wire)

teleport/vibeCode, review/*, imports/*, projectLinks, loops admin (loops still
work via the model's cron tool), voice/narration, debug console, worktree
admin (badge shown), artifacts/PR status. Session spawn request from the
server is not advertised (same as the TUI).
