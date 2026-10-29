# Next-branch bug report — user-reported defects (round 4)

> Working document. Every reported issue is one row below; the status column is
> updated as the work lands (`[ ]` open, `[~]` in progress, `[x]` done).
> All items are treated as **critical** by the reporter: none may be skipped.
>
> Reference implementations: Mistral Vibe CLI source (agent/subagent config,
> chat behavior, queueing, `@`/`/` pickers, question/todo tools) and the stable
> Electron edition of Halny Vibe Deck on `main` (v1.0.x UI parity).

## A. Settings (Ustawienia)

| ID | Issue (as reported) | Status |
|---|---|---|
| A1 | Server tab still shows **two Save buttons** | [x] — one Save button persists remote creds/port/enable AND cert paths together (settings.rs serve tab) |
| A2 | Server tab does **not** hint the computer's real IP, only the VPS one; stable hints both | [x] — local_lan_ips now uses if-addrs on all platforms (GetAdaptersAddresses on Windows) — every non-loopback IPv4 like os.networkInterfaces(); UDP trick kept as fallback (serve.rs) |
| A3 | Agents tab: tool permissions broken — cannot pick tools, cannot add new ones; the permission dropdown with nothing selected looks like a nameless button; missing label for the model dropdown; "builtin" entries in system prompt — what are they? Map ALL possible agent/subagent configurations from the Mistral Vibe CLI code | [x] — agents tab: tool-permission rows got add-row + per-row remove + always-visible labeled section; added missing i18n keys "model"/"agentSafety" (6 langs); instructions field mapped end-to-end; builtin prompts explained in a hint; CLI agent model mapped from vibe/core/agents/models.py (builtins ask/accept-edits/smart-approve/auto-approve/explore/lean + custom overrides incl. tools.*, enabled/disabled_tools, system_prompt_id, active_model, instructions) |
| A4 | Skills tab still does not look/behave like the stable edition | [x] — skills tab: zip import detects .zip; master list shows file counts; detail uses stable layout (add-file row, per-file delete, filling editor box); .skill-detail-body no longer fixed 440px |

## B. Session list (Lista sesji)

| ID | Issue (as reported) | Status |
|---|---|---|
| B1 | SSH group with expanded project sessions scrolls **off screen** — cannot scroll low enough; sessions and SSH should be physically ONE list with collapse/expand of projects and SSH connections | [x] — projects + SSH groups now live in ONE scrollable nav (sidebar.rs + .ssh-section-label) |
| B2 | SSH sessions still not visible | [x] — single list fixes the off-screen SSH section; panes load on mount; session/list parses the CLI 3.0.x "items" wrapper (worker.rs) |
| B3 | Sessions not yet opened show `(CLI)` instead of a name | [x] — session names fall back index_title -> preview (first prompt), never "(CLI)" (worker.rs, remote.rs, sidebar.rs) |
| B4 | Missing marker for **active/running** sessions on the session list | [x] — busy marker is now the stable SVG arc spinner (was an invisible empty span) |

## C. File preview (Podgląd plików)

| ID | Issue (as reported) | Status |
|---|---|---|
| C1 | After switching to file edit, the editor does not adapt to the screen/app height and can be far too short | [x] — .file-preview-body:has(.preview-edit) becomes a flex column and .code-editor fills 100% height |
| C2 | Mermaid does not render correctly in md files / md preview | [x] — MERMAID_JS moved to shell level (chat AND md preview), mermaid@12 CDN, securityLevel strict, theme from data-theme, re-render on theme flip, promise API (v10-v12) |
| C3 | DOCX preview: embedded images/photos and the table of contents do not render; text formatting, indents and many other docx formatting elements are missing | [x] — docx parser: embedded images via rels+media as data URIs, generated TOC from headings when a TOC field/SDT exists, underline/color/highlight/size/sup-sub, w:ind/jc/spacing paragraph css, real hyperlinks |
| C4 | XLSX like DOCX — no formatting, only a raw table in preview | [x] — xlsx preview extracts xl/styles.xml (bold/italic/color/fill/size) + per-cell s= index; sheet_table renders styled cells, header class, zebra rows |
| C5 | Context menu on a **folder** offers preview/edit — it should only offer rename and delete; on `.md` files all options; on docx-like files no edit option (no built-in editor) | [x] — ctx menu: folder = rename+delete only; files = preview/edit/download/open/rename/delete; Edit only for text-editable files and opens the tab in edit mode |

## D. Browser (Przeglądarka)

| ID | Issue (as reported) | Status |
|---|---|---|
| D1 | With the preview panel expanded on desktop, the FAB does not move left and covers the preview | [x] — .web-fab-right.dock-open moves to calc(var(--vd-dock-w) + 12px) on desktop widths |

## E. Chat

| ID | Issue (as reported) | Status |
|---|---|---|
| E1 | After adding a new chat over SSH and opening it, the session list shows `session/resume: Session not found: <uuid>` | [x] — remote mirror rows switch via api_switch_session — never the LOCAL backend open (that caused session/resume Session not found) |
| E2 | Default model is not auto-selected in new sessions | [x] — build_chip_options falls back to global active_model -> first model, so new sessions show the default model chip |
| E3 | When the SSH agent replies, the message I sent and the system messages disappear; during "Agent is working…" no thinking blocks are shown, and thinking blocks are not visible after the agent's answer either | [x] — SessionHistory now MERGES: server history keeps thinking/tools/user msgs; local system notices and the not-yet-persisted user prompt are preserved |
| E4 | The model selected in a session is not loaded when entering the session from the left list | [x] — switch_session -> restore_session_chips reads last_model from session/disk; chip model falls back to default (E2 fix covers it) |
| E5 | No thinking/tool-call info in a resumed session — they disappear after the first agent response | [x] — entries_to_messages groups wire history: reasoning -> thinking blocks, effect -> tool calls on the AI message; snapshot emits UsageUpdate from tokenUsage |
| E6 | Verify prompt queueing, incl. whether the **headless server** guards the prompt queue | [x] — engine-side per-slot queue (SlotState.queue) drained on PromptCompleted/SessionReady — works headless without a UI client (unit-tested) |
| E7 | Prompt queue does not work — everything is sent immediately before the agent answers the 1st prompt | [x] — send_message_with queues behind a busy slot instead of firing immediately + publishes StructureChanged so clients see busy at once |
| E8 | "Agent is working" indicator wrong — the send button stayed active while the agent was still streaming messages | [x] — same publish makes the client flip to the stop button immediately after send |
| E9 | Reconnect check: enable agent in browser, give a task, leave the tab for 15 min, come back — ALL messages and intermediate agent actions must be fetched, not only new ones | [x] — stream Err branch now also api_resync_current (authoritative session/resume) before refetch; visibility bridge already did |
| E10 | `@` does not work like in the stable agent — folders/files render differently and are right-aligned in the list | [x] — @ picker item: name is left-aligned .picker-item-label, only the path hint rides right (was .picker-item-kind with margin-left:auto) |
| E11 | Slash `/` renders no command hints; stable has a sizable list of possibilities | [x] — / picker: CLI builtin slash commands (vibe/acp/commands/registry.py) always offered + agent-reported ones + session-scoped skills/prompts (global+project, remote conn for SSH) |
| E12 | Cache/input/output/cost/project statistics disappear in chat during the conversation with the agent | [x] — stats_numbers reads the real wire fields (sessionPromptTokens/sessionCachedTokens/contextWindow/lastTurnDuration) and derives cost from price fields; usage bar no longer blanks mid-conversation |
