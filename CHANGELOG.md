# Changelog

## 1.99.23 — workspace renamed to halny-vibe-deck (2026-10-30)

The product was renamed to Halny Vibe Deck but the crate package/dir names
still carried the legacy `mistraldeck` prefix. All seven crates are renamed:

- `crates/halny-vibe-deck` (was `mistraldeck`, package `halny-vibe-deck`, binary `halny-vibe-deck`)
- `crates/halny-vibe-deck-appserver` (binary `halny-vibe-deck-appserver`)
- `crates/halny-vibe-deck-appsrv`, `-engine`, `-vibe`, `-core`, `-setup`

All Rust paths (`halny_vibe_deck_core::` etc.), `Dioxus.toml`, the CI workflows
(`cargo test -p halny-vibe-deck --features server`, `working-directory:
crates/halny-vibe-deck`) and the README were updated to match. Intentionally kept:
the `MISTRALDECK_PORT` env fallback, legacy `~/.config/mistraldeck`/`vibedeck`
config migration, and old `mistraldeck.exe`/`MistralDeck.exe` names in the
self-updater (it must still replace legacy installs). Shipped binary and release
artifact names are unchanged. README rewritten for the current architecture
(Dioxus fullstack: desktop webview + headless HTTPS server for browsers).

## 1.99.22 — user-reported defects round 15, re-verified in code (2026-10-30)

Root cause + fix per row in `docs/next-bugs-round15.md`.

- **No more duplicated agent actions**: replayed `history/entryAdded`
  (resync/reconnect) is deduped by entry id in the appserver mirror and by
  tool-call id in the engine + client — tool steps render exactly once.
- **"open elsewhere" fixed on Windows**: own PIDs now include the real
  app-server PID from the `initialize` handshake (on Windows the uv shim's
  child is not the lease holder).
- **✕ stops the session for real**: on Windows the whole process tree is
  killed (`taskkill /F /T`), not just the launcher shim.
- **Session list**: green dot + ✕ follow the session where the agent is
  actually on (busy/stopping/connected), not the open tab.
- **Session switching is instant**: "Loading session…" marker immediately,
  structural-event refetch storm coalesced (one in flight + one queued).
- **Chat scroll**: user scroll-up intent respected — streaming no longer
  yanks the view down while reading.
- **md edit no longer freezes**: editor highlight/gutter cached per text;
  wrap-gutter DOM walk capped.
- **docx preview**: fit-to-width by default with a Fit/1:1 toggle; sticky
  horizontal scrollbar always visible at the bottom of the preview.

## 1.99.21 — user-reported defects round 14, re-verified in code (2026-10-30)

Root cause + fix per row in `docs/next-bugs-round14.md`.

- **Error banner**: long error text wraps inside the block (`.chat-error-text`
  with `overflow-wrap:anywhere`, banner `width:100%`) — no more clipped block
  with text spilling outside.
- **Providers tab**: API key is masked (`type=password` + Show/Hide toggle,
  6 languages); all vibe 3.0.4 `ProviderConfig` fields are now editable and
  persisted — `extra_headers` (JSON), `browser_auth_base_url`,
  `browser_auth_api_base_url`, `browser_auth_allow_origin_rewrite`,
  `preserve_api_base`, `emits_finish_reason`.
- **Models tab**: audited against `ModelConfig` — all fields already
  configurable (no gap).
- **Per-session error banner**: `turn_failed` is keyed by agent session id
  and cleared on session switch — a rate-limit error from a remote session no
  longer leaks into a local session.

## 1.99.20 — user-reported defects round 13, re-verified in code (2026-10-30)

Root cause + fix per row in `docs/next-bugs-round13.md`.

- **docx preview**: pages keep their real width (`max-width:none`) so a narrow
  preview gets a persistent horizontal scrollbar instead of clipped text.
- **Streaming visibility**: running thinking/tool steps are expanded by
  default — live output is visible while the agent works, not only at the end.
- **Tool names humanized**: dotted wire names (`file_system.write_file`) map to
  readable labels with the file/command ("Zapis pliku …").
- **Tool path chips**: absolute agent-written paths (with spaces/long paths on
  Windows) read directly; confinement compares verbatim-prefix-normalized
  forms — no more os error 2/3 on preview open.
- **Session list**: ✕ only on running sessions (stops session + agent
  subprocesses); active-session green dot; context menu fully translated (6
  languages); stale "open elsewhere" flag reset on wake, badge only for a
  genuinely different lease PID.
- **Attach menu**: real system file picker via `<label for>` (gesture-safe in
  WebView2 and browsers) — file/image pickers upload and insert `@path`.
- Wrap toggle only in edit mode.

## 1.99.19 — user-reported defects round 12 (2026-10-30)

Root cause + fix per row in `docs/next-bugs-round12.md`.

- **Latency fixed**: streaming events no longer trigger a full snapshot
  refetch per token (time-to-first-token back to terminal levels).
- **md preview/edit performance**: rendered markdown + syntax highlight are
  cached per source text; editor gutter sync is rAF-coalesced (no more freeze
  on entering edit).
- **Lease badge**: "open elsewhere" only when the lease holder PID is known
  and is not our own agent child.
- **Session list**: green active-session dot; ✕ only on running sessions and
  stops the whole session (cancel + kill agent slot); menus under right-click.
- **File preview paths**: reads/saves confine inside the session root (not the
  engine workdir) and `open_path` normalizes separators/drive case — no more
  os error 2/3 on paths with spaces.
- **Busy marker during resume** shows like on live turns.
- **Wrap toggle only in edit mode**; docx/pdf get a persistent horizontal
  scrollbar when the page is wider than the preview; docx TOC localized
  ("Spis treści") and generated for ≥1 heading at the field's position.
- **Attach menu opens the real system file picker** (files/images) and uploads
  to `.halny-attachments/` with `@path` markers.

## 1.99.18 — user-reported defects round 11 (2026-10-30)

Root cause + fix per row in `docs/next-bugs-round11.md`.

- **Pre-resume tool calls** now carry input/output (were empty shells).
- **Lease badge**: our own agent child PID is filtered out — "open elsewhere"
  only for a genuinely different process.
- **Perf/stall regression fixed**: `turn/queueUpdated`/`runtime/updated` no
  longer force a full `session/resume` per event (round-10 resync storm made
  streaming slow and could break a running turn).
- **Slash picker layout fixed** (glyph left, `/name` label, kind chip right)
  and commands with `input_hint` now offer sub-arguments (`/mcp status|login|…`).
- **Tool-call path chips** resolve on Windows (separator + drive-case
  normalization) — no more `os error 2` on preview open.
- **Editor keeps a visible scrollbar** in edit mode via a custom overlay
  scrollbar driven by the textarea (wrap parity preserved).
- **md previews/chat**: list indentation tightened (1.5em) to match prose.
- **DOCX**: generated TOC renders at the TOC field's position (not forced to
  page top); mid-paragraph page breaks split in place — no more empty pages
  between a heading and an image.
- **Settings raw TOML**: going back to config.toml repaints the editor with
  the freshly loaded config instead of the last opened file.

## 1.99.17 — teleport to Vibe Code Web + loops/log/worktree/artifacts admin (2026-10-30)

- **`/teleport [prompt]`** — full TUI parity with the installed v3.0.4 wire:
  `vibeCode/projects/open` (purpose "teleport") → project picker modal
  (list/select/create/load-more via `vibeCode/projects/*`), then
  `vibeCode/teleport/start`; progress events (`vibeCode/teleport/event`:
  summarizing_context → checking_git → push_required (Push/Skip buttons via
  `teleport/push/respond`) → pushing → starting_workflow → complete with the
  cloud URL / failed) render live in a modal; abort via `teleport/cancel`.
- **`/loops`** — scheduled-loop admin modal (`loops/list|delete|clear`).
- **`/log`** — session log + diagnostics modal (`session/log/read`,
  `diagnostics/logs/read`).
- **Worktrees** — `workspace/git/worktrees/list` renders in the dock Git tab
  (name/branch/+a/-d, click reveals the worktree in the file tree); worktree
  badge on session rows.
- **Artifacts** — `artifacts/list` on session open + `artifacts/created|updated`
  → PR chips in the chat (click opens the PR).

## 1.99.16 — full TUI parity, verified against the installed CLI v3.0.4 (2026-10-30)

Re-audited against the actual `vibe-rs` TUI binary and `vibe/app_server` 3.0.4
sources (not just the v3.0.3 reports) — details in `docs/tui-parity-round10.md`.

- **Fork & rewind**: `/branch` (`/fork`) and `/rewind` open a checkpoint
  picker modal (history entries with wire ids); `session/fork` (attach) and
  `session/rewind` + `session/rewind/read` (affected-files preview with a
  restore-files checkbox) wired end-to-end; fork response state switches the
  GUI to the new session.
- **Subagents**: `childSessions` snapshots + `session/childSessionUpdated`
  render as a live chip strip above the composer; click opens the child.
- **Session shelf extras** (beyond the TUI): pin 📌 / archive 📦 / unread dot /
  worktree badge via `session/pin|archive|markAsSeen`, permanent
  `session/delete`, pinned-first + archived-last sorting,
  `session/list {includeArchived:true}`; lease badge "open elsewhere" via
  `session/lease/subscribe` + updated/ended.
- **Slash command coverage**: the picker lists the full cli-rust command set;
  additionally implemented client-side: `/status`, `/version`, `/delete`,
  `/update`, `/diff`, `/theme`, `/config`, `/permissions`, `/connect`,
  `/agents`, `/resume`, `/quit`/`/exit`.
- `turn/queueUpdated`, `turn/retrying`, `runtime/updated` now resync.

## 1.99.15 — TUI parity round: slash commands executed client-side + attachment paste (2026-10-30)

Re-audited the chat against the official Mistral Vibe CLI TUI (`docs/tui-capabilities-report.md`, `vibe` v3.x app-server sources) so the GUI can fully replace the terminal:

- **Slash commands work like the TUI**: in cli-rust `/compact`, `/clear`, `/rename`, `/reload`, `/model`, `/thinking`, `/agent`, `/retry`, `/help` are intercepted CLIENT-side and translated to JSON-RPC (`session/compact`, `session/history/clear`, `session/rename`, `config/reload`, `config/model/write`/`session/agent/update`, resend, local help). The deck sent them as plain prompts before (the server only knows `/name` as a SKILL invocation). Now `submit()` dispatches: `/compact [instr]`, `/clear`/`/new`, `/rename <title>`, `/reload`, `/model <name>`, `/thinking <level>`, `/agent <name>`, `/retry [instr]` (resends the last user prompt), `/help` (renders the command list in the chat); unknown `/name` still goes to the server as a skill invocation. New `AgentCommand` variants (`SessionCompact`/`HistoryClear`/`SessionRename`/`ShellCommand`/`ConfigReload`) + worker wire calls with the exact v3 param shapes; `!cmd` runs `session/shellCommand`. The `/` picker lists all of them.
- **Attachments**: `@` picker, drag&drop onto the chat, and the ＋ menu already existed; added **clipboard paste of images** into the composer (PASTE_JS bridge → upload to `.halny-attachments/` → `@path` marker, same flow as drop; the server inlines images as base64 content blocks, text files as attach blocks). Images already travel as `image` content blocks in `session/turn/enqueue`.
- **Notifications**: `runtime/updated`, `turn/queueUpdated` and `turn/retrying` now trigger a resync so chips, the `/` picker and the retry banner track the server instead of being logged only.

## 1.99.14 — user-reported defects round 8 (2026-10-30)

Round 7 areas re-reported; each row re-audited, real root causes fixed. Root cause + fix per row in `docs/next-bugs-round8.md`.

- **Preview/editor**: wrap toggle re-syncs the line-number gutter immediately (was only on scroll/content); the highlight layer is now byte-identical to the source (keywords were rendered doubled, md `#`/`>`/`**`/backtick markers dropped, TOML had a phantom trailing newline) — this also fixes caret alignment and wrap-mode gutter offsets; textarea scrollbar hidden so both layers wrap identically; `.code-hl` tab-size matches; raw-TOML settings editor gets TOML highlighting. Icon-only wrap toggle and edit→eye "back to preview" verified.
- **Window chrome**: the viewport scrollbar at the window edge is now detected (`onScrollbar` walks up to `<html>`), so grabbing it no longer shows the window-resize cursor.
- **DOCX preview**: TOC renders again — `fldSimple` TOC fields (instruction in an attribute), localized gallery values ("Spis treści"…), and ASCII-folded localized heading style names ("Nagłówek 1", "Überschrift 2") all recognized. Pages are page-shaped (`min-height` from `w:pgSz w:h`, previously parsed and discarded), `sectPrChange` no longer misparsed, no empty trailing sheet after a final page break. PDF page cards verified.
- **Settings → Skills**: rename now reloads the file tree for the new name (stale rows could resurrect the old folder on save), rewrites `enabled_skills`/`disabled_skills` in config.toml, handles CRLF/quoting in SKILL.md frontmatter, and the "new file" input no longer shares its signal with "create skill". Tools-collapse 1:1 and in-button labels verified.
- **Chat**: relative agent links resolve against the session cwd (was the global workdir for local sessions), md path tokens tolerate emphasis markers and Unicode-leading paths, directory reveals split `\` too. Live-turn merge fixed at the root: wire message ids stamped on streaming bubbles + disk-preview entry ids (no more duplicate/reordered messages on resync), snapshot primes assistant delta length correctly (no full-text re-emit), unknown `entryUpdated` ids flag a resync instead of corrupting `/history/0`, pre-resume tool cards honor wire `state.status`. Pre-prompt model/mode/thinking choices are mirrored into the session so later agent payloads can't clobber them.
- **SSH new session**: callbacks now parse the real wire shape (`detail.kind`, `request.questions[]`/`options[]`, `effect.toolName`, `UserQuestionResult` answers) so approvals/questions actually surface and get answered; dead agent/ssh processes emit `Disconnected` (no more zombie busy slots with queued prompts); the local-binary guard no longer blocks pure-SSH hosts; `busy` survives the post-wake resync; `todo_write` feeds the plan card and resyncs keep it; `@file` markers in remote chats read over SSH; approval overlays clear immediately.

## 1.99.13 — user-reported defects round 7 (2026-10-30)

All 14 reported defects fixed; root cause + fix per row in `docs/next-bugs-round7.md`.

- **Preview/editor**: word-wrap toggle is an icon-only button (in preview AND edit); in wrap mode the line-number gutter counts LOGICAL lines (`vdEdSync` positions each number at its logical line's pixel offset); entering edit turns the edit button into an eye-icon "back to preview" button; the editor highlights Markdown (headings, bold/italic, code, links, lists, quotes, fences).
- **Window chrome**: hovering a scrollbar no longer shows the window-resize cursor.
- **DOCX/PDF preview**: the generated table of contents is inserted at the TOP of the first page (it used to land at the end of the document); DOCX renders as real `.doc-page` sheets split at `w:br w:type="page"` / `w:lastRenderedPageBreak` with page size/margins from `w:sectPr`; PDF pages get the same page-card styling.
- **Settings → Skills**: import/open button labels live inside the buttons; skills can be renamed (`api_rename_skill` renames the folder + SKILL.md frontmatter `name:`); the available-tools collapse in the Agents tab is now 1:1 with the Tools tab (slim header stays when collapsed).
- **Chat**: tool-call bodies never dump raw JSON (readable text extracted from output JSON, pretty-printed otherwise; detail-shaped `state.output` rejected); mid-turn resyncs no longer reorder messages or drop in-flight thinking/tool calls (server history wins, live state merges by entry id, local-only messages append in order); path-like prose tokens and relative `[text](path)` links in agent messages are clickable and open the preview resolved against the session cwd; clicking a directory reveals it in the file tree with every ancestor expanded.
- **New session**: model/mode/thinking chips are enabled before the first prompt with config-file defaults and apply to the new session (pending options applied at SessionReady before the prompt).
- **SSH new session**: a prompt sent to a remote session mirror with no slot now spawns the agent over SSH (`NewRemote`) instead of a local process in a remote path; approvals/questions and streaming render through the same slot event path as local sessions.

## 1.99.12 — user-reported defects round 6 (2026-10-30)

All 11 reported defects fixed; root cause + fix per row in `docs/next-bugs-round6.md`.

- **Preview edit freeze**: the word-wrap toggle init spawned on every render (infinite signal loop) — one-shot guarded.
- **Tools tab**: the collapsed available-tools panel no longer leaves an empty block on screen (slim header only, like Agents).
- **Prompts/skills editors fill the window** (settings section → master-detail → editor box now stretch end-to-end); skills imports use the system file picker (hidden file inputs + base64 upload API) with the whole header in one row.
- **Chat tool calls**: wire `file_edit` effects now build git-style `DiffHunk`s (occurrences / old-new pair / previousContent); the read_file path chip is clickable and opens the file in the preview dock.
- **Session isolation**: resync targets only the current session's own slot; agent events prefer the current session when duplicate mirrors share an agent id.
- **Resume**: wake failures with context `session/resume` now trigger the respawn fallback — the pending prompt is no longer dropped with no busy marker.
- **Thinking blocks**: `history/entryUpdated` patch paths are entry-relative; they are now prefixed before applying to the projection, so reasoning deltas stream correctly in active sessions.
- **Window resize**: edge/corner resize wins over the header drag region (top-left corner fixed); clicks inside a scrollbar zone no longer start a window resize (panel handles also no longer overlap scrollbars).
- **DOCX/XLSX on real files**: heading levels resolved from `word/styles.xml` (localized style ids, basedOn chains) with a generated TOC; XLSX renders merged cells (colspan/rowspan), column widths, theme/indexed/tint colors and number formats.
- **CI-only flaky test fixed**: the session-token signing secret is now generated once per process (`OnceLock`) and written atomically (`create_new`), so concurrent logins/tests on a fresh install can no longer mint tokens under different keys (`revoked_token_is_rejected` failed only on clean CI runners).

## 1.99.11 — user-reported defects round 5 (2026-10-29)

## 1.99.11 — user-reported defects round 5 (2026-10-29)

All 13 reported defects fixed; root cause + fix per row in `docs/next-bugs-round5.md`.

**SSH sessions (critical)**
- `ssh_connect_opts()` (StrictHostKeyChecking=accept-new + ConnectTimeout=10) is now applied to **every** ssh spawn — `ssh_run`, `remote_argv` (remote agent slots) and `list_remote_sessions` (the sidebar lister). Previously the lister spawned ssh without them and hung on the interactive host-key prompt, so the session list showed no SSH sessions at all. Sidebar panes also reload when the group is expanded and when the snapshot changes, and show an error hint when the host is unreachable.

**Session list**
- Untitled sessions get a real name immediately (title → session-index title → first user prompt `preview`), at both list sources (disk lister + `session/list`).

**Preview / editor**
- Resize handles no longer sit on top of scrollbars (hit areas clipped to the handle strip; sidebar/dock reserve a 5px strip) — clicking a scrollbar no longer starts a panel resize.
- The preview editor has a persistent **word-wrap toggle** (localStorage) with a matching highlight layer.
- DOCX headings render at the right levels: `w:outlineLvl` wins, style names normalized across locales (Heading1 / "Heading 1" / Nagłówek 1 / …).
- XLSX preview: the sheet scrolls INSIDE the preview body (no more off-screen table on small windows), and cell styles now include theme colors (theme1.xml palette), indexed colors, tint, plus number formats (%, decimals, thousands, currency) applied to the displayed values.
- Mermaid in md previews fixed (a `to_string` typo in the renderer bootstrap) — mermaid@12 renders in chat and file preview again.

**Windows console flashing**
- `mistraldeck_core::util::no_window` (CREATE_NO_WINDOW) applied at every process spawn — git commands (the Git tab), the ssh lister, the `uv tool dir` probe, open-externally — audited all `Command::new` sites across the workspace.

**Settings on phones**
- Tools tab: the available-tools list is collapsible and stacks ABOVE the permission configuration on narrow screens.
- Skills and Prompts/Agents editors now use the same syntax-highlighted `CodeEditor` as the file preview and fill the panel (42–46vh on phones).
- TOML tab stacks vertically on phones (file tree on top, editor below).

**Resumed sessions**
- Wire `effect` entries map to rich tool cards (toolName / kind / display.summary / input / state.output / state.status) instead of raw JSON; `status` (not just `type`) is read and `cancelled` maps to error.
- Chat auto-scroll only jumps to the bottom when the user is already near it (or on session switch) — reading up while the agent streams no longer yanks the view down.

## 1.99.10 — user-reported defects round 4 (2026-10-29)

## 1.99.10 — user-reported defects round 4 (2026-10-29)

All 26 reported defects fixed; each row in `docs/next-bugs-round4.md` carries its root cause + fix. Highlights:

**Settings**
- **Server tab**: ONE Save button (previously two identical "Zapisz") persists remote credentials/port/enable **and** the custom certificate paths in a single click.
- **Server tab IP hint**: `local_lan_ips()` now enumerates every non-loopback IPv4 on ALL platforms via `if-addrs` (GetAdaptersAddresses on Windows) — stable `os.networkInterfaces()` parity. The old unix-only getifaddrs + single-UDP fallback showed only the outbound (e.g. VPN/VPS) address on Windows.
- **Agents tab**: tool-permission rows are a real editor — per-row ask/allow/deny select, per-row remove, and an always-visible labeled add row offering every unconfigured tool; the model dropdown has its label again (missing `model` i18n key added in all 6 languages, plus `agentSafety`); `instructions` is mapped end-to-end (local + SSH); the system-prompt dropdown explains that `builtin` = the prompts bundled with the Mistral Vibe CLI (`cli`, `explore`, `tests`, `lean`, `minimal` — `vibe/core/prompts`), the rest are the user's prompt `.md` files. The full CLI agent/subagent model (`vibe/core/agents/models.py`) is covered: builtins `ask`/`accept-edits`/`smart-approve`/`auto-approve`/`explore`/`lean` plus custom `~/.vibe/agents/*.toml` overrides (`agent_type`, `safety`, `display_name`, `description`, `active_model`, `system_prompt_id`, `enabled_tools`/`disabled_tools`, `[tools.*]` permissions, `instructions`).
- **Skills tab**: stable layout again — header row (Create / Import folder / Import .zip / Open folder; `.zip` detection fixed), file counts in the master list, detail with add-file row, per-file delete, and an editor box that fills the panel.

**Sidebar**
- Projects and SSH connection groups are physically ONE scrollable list — expanded sessions can no longer push SSH off screen.
- SSH sessions visible (mount-time pane load + `session/list` now parses the CLI 3.0.x `items` wrapper, not only the legacy bare array).
- Untitled sessions get a real name (index title → first user prompt `preview`), never `(CLI)`.
- Running sessions show the stable SVG arc spinner (the old marker was an invisible empty span).

**Preview / dock**
- The file editor fills the preview body height (was a fixed 420px).
- Mermaid fences render in md previews too (renderer installed once at shell level; mermaid@12, `securityLevel: strict`, theme follows `data-theme`, re-renders on theme flip).
- DOCX: embedded images (rels → media, data URIs), a generated table of contents when the document carries a TOC field/SDT, real hyperlinks, and full run/paragraph formatting (underline, color, highlight, font size, super/subscript, `w:ind`/`w:jc`/`w:spacing`).
- XLSX: cell styles extracted from `xl/styles.xml` + per-cell `s=` attributes (bold/italic/color/fill/size) render in the sheet table with a styled sticky header and zebra rows.
- Context menu per type: folder → rename/delete only; text files → preview/edit/download/open/rename/delete; docx-like files → no Edit (no built-in editor).
- On desktop the preview FAB moves to the dock's left edge when the dock is open instead of covering it.

**Chat**
- Remote (SSH) mirror rows switch correctly — no more `session/resume: Session not found` after adding an SSH chat.
- New sessions auto-select the default model (global `active_model` → first configured model); reopening a session restores its recorded model.
- `SessionHistory` now MERGES instead of replacing: the just-sent user message and local system notices survive a resync; wire history is grouped properly (reasoning → thinking blocks, effect entries → tool calls), so resumed sessions keep thinking/tool info after the first answer.
- Server-side prompt queue per slot (drained on turn end) — the queue is guarded even headless; the client sees `busy` immediately after send (send button flips to stop), and queued prompts send one at a time.
- Event-stream reconnects (and the visibility bridge) resync the authoritative session state, so returning to a tab after minutes backfills ALL missed messages/actions.
- `@` picker: file/folder names are left-aligned again (only the path hint rides right).
- `/` picker always lists the CLI's built-in slash commands (from `vibe/acp/commands/registry.py`) plus agent-reported ones and the session-scoped skills/prompts (global + project, remote host for SSH).
- Usage bar no longer blanks mid-conversation: `session/statsUpdated` is parsed with the real wire fields (`sessionPromptTokens`, `sessionCachedTokens`, `contextWindow`, `lastTurnDuration`, cost derived from the per-million price fields), and snapshots emit usage from `session.tokenUsage`/`contextUsage`.

## 1.99.9 — preview drop-upload + tools/subagent/usage round (2026-10-29)

**Preview (dock)**
- **Drag & drop upload onto the file tree** (stable v1.0.26 parity): dropping files on a folder row uploads them into that directory (dropping on a file row uses its parent; dropping on the root crumb uses the displayed directory), local or over SSH via `api_write_binary`, then reloads the listing. Hover highlight (`drop-target`), 800 ms hover auto-expands a collapsed folder, `filetree-uploading` indicator while writing, failure count in the tree message line.

**Chat — tool rendering audit (stable v1.0.37 parity)**
- `agents.list` / `agents.list_configs` now render a real **result card** (subagent table with live status spinner / ✓ / ✕ + error text, empty state) instead of falling through to the glob card or the bare title — new `agents_list_block` + `subagentNoActive` i18n in all 6 languages.
- Bash calls whose wire output is only a "Ran X" summary now re-read the **full tool result from the session log on disk** (new `api_session_tool_result`, local sessions) and show the real stdout/stderr — same as stable's IPC enrichment. Structured `command:/stdout:/stderr:/returncode:` parsing with stderr only on non-zero exit was already in place and is unchanged.
- **Active-task marker**: `agents.spawn` / `task` calls with a child session are no longer force-closed on `PromptCompleted` — they stay `running` (spinner) while the child works and close when the CLI reports the terminal status (stable v1.0.37 behavior); all other stuck calls are still closed.
- Edit diffs (git-style hunks with line numbers, `@@ -a,b +c,d @@` headers, success badge, `<tool_error>` output), read_file, grep/glob, process and todo lines were audited against stable and match.

**Subagents**
- Historical sessions keep the subagent card fully working: the `kind == "subagent"` effect meta (agent, task prompt, `childSessionId`) is now parsed from the on-disk projection into the preview transcript, so the prompt is visible and **"Show work"** loads the child session's transcript (prompt + tool calls + answers) via `api_session_preview`.
- **Usage bar now includes subagent tokens**: the CLI reports child-session usage separately from the parent's `usage_update`, so the appserver rolls each child's `tokenUsage` (input / **cached input** / output / total) plus its cost into every `UsageUpdate`, and the lazy-resume disk preview does the same (`subagent_usage_totals` over `unified/child-<id>`). Cache in/out of subagents is therefore counted in the bottom-of-chat stats.

## 1.99.8 — settings/dock/sidebar round (2026-10-29)

**Server**
- With remote access enabled the **app port itself** is now the public surface: `0.0.0.0` + HTTPS + login (no more unauthenticated `https://localhost:8080` when the server is on, and no more silently-disabled listener when the configured port equals the app port). Remote off = loopback-only HTTPS without login, as before.
- The access hint in Settings → Server lists **every** device IP (NIC *and* VPN — getifaddrs on Unix); loopback is no longer shown.

**Settings**
- **Server tab**: LAN-only URLs, custom certificate paths (`cert.pem`/`key.pem`), cert-in-use indicator.
- **Agents tab** is the stable edition's real configurator: master list (display name + type), detail form with type select, display name, description, safety, model select (project+global models), system-prompt select (builtins + prompts), enabled/disabled tools, per-agent `[tools.*]` permissions — saved through a new structured `api_save_agent` (local + SSH). Right-hand collapsible available-tools panel like stable.
- **Tools tab**: two-panel stable layout — permissions configurator on the left, the **available tools** list on the right.
- **Skills tab**: stable header (name + Create / Import folder / Import .zip / Open folder), file counts in the master list, open-externally + delete in the detail header.
- **Prompts tab** and the shared file panel restyled to the stable master-detail look (header add-row, `master-item` rows, field-textarea editor, rename-on-change, remove).
- **TOML tab**: the file list is now a **tree** — `config.toml` plus expandable `agents/` `prompts/` `skills/` folders (click to expand, click again to collapse) with a per-folder ＋ add-file row.
- **General tab**: the bare "Detected agent" row is gone; the CLI (mistral-vibe) row shows its version, and when the CLI is missing it offers **Install** instead of an update button.

**Preview (dock)**
- Save button is an icon (floppy) with the label as tooltip.
- Folder rows expand on click (children were keyed by name instead of path — nested folders never opened).
- The file tree roots at the **current session's project** (local `confine` now uses the session root, not the engine workdir) and works over SSH the same way.
- Right-click context menu on file rows: Preview / Download / Open externally (desktop) / Rename / Delete (local + SSH via new `api_rename_path`, `api_delete_path`, `api_write_binary`).
- Drag & drop files over the chat uploads them into `<project>/.halny-attachments/` (local or SSH) and inserts `@path` markers.

**Browser/mobile**
- On desktop-width browsers the preview **FAB stays visible and toggles** the dock (it used to vanish when the panel was open, leaving no way to close it).
- On phones the preview panel now slides in with the same animation as the sessions list (dock stays mounted, `.open` class animates `translateX`).

**Chat / sidebar**
- Chat auto-scroll is robust now: a body-level MutationObserver plus a Rust-side effect on session/message change — streaming and opening a historical session both land at the bottom (verified in e2e).
- The input textarea has no native border (`border: none`) — no more "two bars" at the bottom of the input card.
- Sidebar: each project group shows **max 10 sessions** with a "show all (N more)" row; the projects list fills the whole sidebar height and scrolls on its own, so SSH groups and the Stats/History/Settings nav are always visible.
- History rows no longer show "CLI session": titles fall back to the unified-store index entry and then to the first user prompt.

## 1.99.7 — user-reported defects round 3 (2026-10-29)

**Server / browser**
- **HTTPS-only app surface.** The main port is never plain HTTP anymore: every mode serves TLS (the desktop webview gets its own private loopback listener on an ephemeral port, because webview engines reject self-signed certificates — that listener never leaves 127.0.0.1 and is not the app's server surface).
- **Remote access actually starts and is reachable by IP.** `remote_port` now defaults to 8443 (it used to default to 8080, colliding with the app port, so the 0.0.0.0 listener was silently disabled). The generated self-signed certificate includes the device's LAN IPs in its SANs, and the startup log prints `https://<ip>:<port>`.
- **Custom TLS certificate** (`cert_path`/`key_path` in the config, editable in Settings → Server): when both are set they are used for the HTTPS listeners; otherwise the cached self-signed certificate is generated as before.
- **Settings → Server** lists every access URL including the device's IP addresses (`https://192.168.x.x:8443` etc.) and shows which certificate is in use.
- **Floating buttons**: the preview-panel FAB now works from any view (switches to chat, persists the dock state) and on phones each FAB hides when the *other* panel is open.
- **Mobile**: the chat toolbar (＋, model, mode, thinking, send) stays in a single row — chips shrink with ellipsis instead of wrapping; tapping Settings/History/Statistics closes the session list; statistics render one chart per full-width row; settings get a scrollable tab bar and stacked master-detail panels.

**Native app**
- **"New project…"** button is text + `＋` again (no 🆕 icon), same for the SSH groups.
- **Sidebar resize handle** no longer inverted (dragging the session-list edge right widens it).
- **Update checks / curl / ssh / systemctl** spawn with `CREATE_NO_WINDOW` on Windows — no more flashing terminal windows.
- **SSH sessions** in the sidebar load on mount (the pane was only fetched when re-expanding a group, so remote sessions looked absent).
- **"unauthorized" errors** in Settings, the file tree and Statistics are gone: the server functions no longer re-check the vestigial token (the remote HTTPS listener's cookie middleware already gates every request; loopback is trusted) — previously every settings/file/stats call failed in the local app whenever remote credentials were configured.
- **Settings tabs** match the stable edition again (underline tabs, order: general, server, remote, agents, mcp, tools, prompts, skills, providers, models, TOML); the scope path preview sits beside the switch, vertically centered.
- **Tools tab** rebuilt like stable: one row per tool (name + ask/allow/deny + remove), an "add tool" row, and the built-in defaults with an explanatory note when nothing is configured.
- **Chat input**: no manual resize handle (`resize: none`); the box auto-grows with content up to 160 px; the attach button is a single `＋` offering **file or image** only (directory and branch-diff entries removed); clicking outside any dropdown (model/mode/thinking, attach, `@`, `/`) closes it.
- **New session in a project** offers the model/mode/thinking pickers immediately (chip lists are built from the project's vibe config at creation) and shows an empty chat instead of the previous session's messages (the resume guard now compares session IDs, not list indices).
- **First prompt in a new session** no longer dies with `WARNING config/model/write: Invalid request parameters`: empty values are never sent, and a rejected config write is a debug log instead of a fatal session error that killed the turn.
- **Model/mode/thinking changes** leave a visible notice in the chat ("Model changed to …", localized, incl. thinking).
- **Session names** update from `session/list` (CLI-generated titles) without reopening the session.
- **Opening a session scrolls the chat to the bottom**; the todo/plan card renders even before the first message; historical tool calls show the tool name and status instead of a bare "Running".
- **History** renders the local rows immediately; remote SSH listings load in the background so a slow host no longer leaves "Loading…" forever.

## 1.99.6 — the remaining reported defects (2026-10-29)

- **Tools** are the stable edition's permission panel again: one row per tool
  with ask / allow / deny, persisted as `[tools.<name>] permission` in the
  selected scope (global or project, local or over SSH). The free-text
  enabled/disabled lists are gone from this tab.
- **Skills** became a master-detail view: create a skill, import a folder, import
  a `.zip`, open the skills folder, browse the skill's files and edit them, delete
  the whole skill. Server-side names are validated so a skill can never escape
  `skills/`.
- **Agents and prompts** share the same two-column master-detail layout with the
  editor, save, delete, create and “open externally”.
- **Raw config (TOML)** fills the window instead of a fixed 420 px box, and shows
  a read error or an empty-file hint instead of a blank editor.
- **Scope switches** (local ↔ SSH, global ↔ project) now remount the controls and
  reload every panel, so a directory change is visible immediately.
- **File preview** gained a download button (local and over SSH) next to the path,
  the edit toggle and “open externally”.
- **Chat option chips**: a model change re-derives that model's thinking levels
  (`thinking_levels`), an empty agent payload no longer blanks the pickers, and
  mode choices carry descriptions; project agents stay scoped to their project.

## 1.99.5 — user-reported defects round (2026-10-29)

- **Chat works again (critical).** `session/start` reached the agent but the turn
  never started: the `SessionReady` handler called `refresh_account()`, which
  takes the engine state mutex the handler already holds — a self-deadlock that
  froze the slot pump and left “Agent is working…” on screen forever. The account
  re-read now goes straight to the slot; a regression test drives the real engine
  against the stub agent (`crates/mistraldeck-engine/tests/chat_turn.rs`).
- **Sessions belong to a project.** `New project…` sits next to the `SESSIONS`
  label (right-aligned) and opens a folder picker (local browse, remote path over
  SSH); every project group header gained a `＋` that starts a session there.
  The old “New session” button (which hijacked an existing empty session) is gone.
- **History is back in the sidebar.** Past sessions from the local store render
  inside their project groups (lazy preview on click, no agent process); the
  separate “CLI” bucket is gone.
- **No agent at boot.** The startup prime only indexes history from disk; the
  agent is contacted on the first prompt or when a session is resumed.
- **Statistics** reduce to the requested scope (total cost, total tokens,
  activity, tool calls, all sessions) with a single toolbar: title, range filters
  and refresh in one row.
- **Settings**: the agent connect/disconnect and account panels are gone; update
  buttons appear only when a newer version exists; the server and SSH remotes are
  separate tabs and remote project directories are edited as rows.
- **HTTPS**: the headless/server build serves the local UI over HTTPS (self-signed);
  the desktop webview keeps loopback HTTP because webview engines reject a
  self-signed certificate — that listener never leaves 127.0.0.1 and every
  network-exposed surface is HTTPS-only.
- **Files**: the tree expands in place and remembers what is open; the preview
  header shows the full path and gains “open externally” (desktop).
- **Browser layout**: the title bar is selected by `/?desktop=1` instead of the
  unreliable `window.ipc` probe, so the web client never shows one.
- **Windows**: the agent process is spawned with `CREATE_NO_WINDOW` — no more
  empty black console windows (one appeared per session).
- The theme labels lost the “(click to change)” suffix in all six languages.

## 1.99.1 — stable parity pass (2026-10-28)

- **Branding is now 1:1 with the stable edition.** The header mark, the chat
  empty state and the setup screen use the stable pixel-art artwork (1024
  viewBox, annular #10131A tile, orange/amber pyramid, wind swooshes), and the
  Windows/PWA icons were regenerated from the same SVG.
- **ACP is gone.** The `mistraldeck-acp` client, the legacy egui app it served
  and the legacy release workflow were removed; the Rust edition speaks only
  `vibe-app-server`. Legacy `Acp*` names in the shared protocol were renamed to
  `Agent*`.
- **PWA:** manifest gains `id` and 512×512 icons (any + maskable); `/icon-512.png`
  is served from the binary and cached by the service worker.
- **Web sidebar:** theme cycle and logout live at the bottom of the sessions
  list, under Settings (stable v1.0.33), with an inline SVG power icon.
- **Long turns:** the 180 s request cap is gone (stable v1.0.34); a dead agent
  fails the pending request immediately instead of hanging, and an empty resume
  no longer wipes the chat. Resume retries once after a transport hiccup.
- **systemd:** the server sends `READY=1`/`WATCHDOG=1`, the port bind retries
  during update handover, and self-update restarts through the unit
  (`systemctl --user restart`, exit(1) failsafe).
- **PWA back gesture** closes the open panel instead of leaving the app; long
  commands in the approval dialog wrap instead of being cut off.
- **E2E:** a Playwright suite plus an `e2e` CI job drive the real headless
  server (PWA surface, rendered web shell, remote login/logout).

## v0.8.3-dioxus.7 (2026-10-25) — M9.5: local no-auth + HTTPS remote login + SSH remote connections

- **Local start without password:** the app no longer asks for login when the
  interface is served on the local loopback (127.0.0.1) — the desktop UI opens
  straight into the chat. Credentials are only requested when connecting over
  HTTPS from another machine.
- **Remote HTTPS serve (Nexus parity):** a new serve stack binds HTTPS with a
  self-signed cert when remote access is enabled; browser sessions authenticate
  with a secure cookie after a redesigned Nexus-style login screen (flame brand,
  card layout, error/hint states). Credential editing (username/password) is
  available in local Settings only.
- **SSH remote connections (Nexus v0.9.0 parity):** Settings → Remote lets you
  add SSH servers (name, ssh command, project locations, optional custom install
  command). Each connection can be tested (reachable / vibe-acp installed /
  version), and vibe-acp can be installed remotely (`uv tool install mistral-vibe`).
- **Remote sidebar:** remote servers appear as 📍 groups with a live status dot
  (green/red); quick-start locations (📁) start a remote session in one click,
  remote CLI sessions are listed and clickable, local mirrors are grouped per
  remote working directory with a ＋ quick-start button.
- **Remote sessions:** remote ACP clients run over SSH (`ssh … bash -lc
  "exec vibe-acp"`) using the same JSON-RPC protocol — remote chat, modes and
  config options work like local ones; session mirrors carry the remote id.

## v0.8.3-dioxus.6 (2026-10-25) — parity with Mistral Vibe Nexus 0.7.1

- **Panels:** left sidebar and right dock are now resizable by dragging
  (`sidebar-resize-handle`, `dock-resize-handle`; widths persist via CSS variables
  `--vd-sidebar-w` / `--vd-dock-w` and localStorage). Handles hide below 900px width.
- **Dock:** file and diff previews open as tabs next to Files/Git (with ✕ close),
  matching the Nexus dock. Tab bar: `dock-tab-sep`, `dock-tab-preview`,
  `dock-tab-name`, `dock-tab-close` (close does not propagate to tab switch).
  Active preview tabs survive switching to Files/Git and back. The dock keeps its
  width (no more content-driven resizing, `flex-shrink: 0`).
- **Settings:** Global/Project scope switch for skills & prompts (scope pills),
  settings tabs clear stale selection on switch, language list gained "auto
  (system)" — resolved from the browser/OS language; color mode already had "auto".
- **Window:** frameless desktop window (no native title bar on Windows) with
  custom window buttons in the header (minimize / maximize / close) and a draggable
  header area; edge resize support via `vd-resize:<dir>` IPC (tao `ResizeDirection`).
- **Tool calls:** rendered as blocks with status-colored headers (`tool-call-done`
  green, `tool-call-running` amber, `tool-call-error` red) with a collapsible body —
  tools in progress and finished are now visibly marked, like Nexus.
- **Stats/History tab:** fixed crash on load — `now_ms()` used `std::time` on wasm,
  which panics in the browser; it now uses `js_sys::Date::now()` on wasm32.
- **Chips:** model/mode/thinking dropdowns open mid-session again — the chip is only
  disabled when the *model* is missing or the agent is busy, not for every chip
  during a session.
- **Sidebar:** project groups are collapsible (chevron), busy sessions show a
  spinner and can be cancelled with ✕, right-click context menu (cancel / delete
  with confirm modal, `ctx-overlay` backdrop).
- **Messages:** Mistral logo avatar with streaming spinner, action row with copy
  button and local-time timestamp, collapsible thinking sections, SVG send/stop
  buttons (input enabled state highlighted).
- **Desktop build:** `prime()` warms up the ACP engine on serve so config options /
  sessions appear immediately; panic hooks log to file on Windows and console on web.
- Branch consolidation: development moved to `main` (the `dioxus` branch was merged
  and deleted); CI/release workflows target `main`.

## v0.8.3-dioxus.5 (2026-10-25)

- **Windows:** fixed invisible GUI on double-click launch — tao event loop is now
  created with `with_any_thread(true)` on Windows (same as on Linux), so the webview
  window opens correctly when launched from Explorer. Previously the app process was
  alive (visible in Task Manager) but no window appeared.
- **Windows:** closing the GUI window now exits the whole app (server included)
  instead of leaving the process running in the background.
- **Windows:** panic messages are appended to `mistraldeck.log` next to the executable
  (previously invisible when built with `windows_subsystem = "windows"`).
- **Icon:** the executable now embeds the mistral-vibe-nexus icon (PE resource via
  `windres`, added by `crates/mistraldeck/build.rs`).
- CI: new `ci-dioxus.yml` (workspace tests + windows-gnu check with icon build
  script) and `tag-on-marker.yml` (commit messages containing `[tag vX.Y.Z]`
  create that tag automatically).
- Build & release pipeline: artifacts are built by GitHub Actions only (no local builds);
  a commit whose message contains `[release vX.Y.Z]` creates/moves the tag and runs the
  full release pipeline (test -> bundles -> GitHub release).
