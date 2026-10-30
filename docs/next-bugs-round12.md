# Halny Vibe Deck (branch next) — user-reported defects round 12 (1.99.19)

| # | Report | Root cause | Fix |
|---|--------|-----------|-----|
| R1 | Very long time from prompt to first tokens | `apply_slot_event` published `StructureChanged` for EVERY event, so each streamed chunk made every websocket client re-fetch and re-serialize the WHOLE snapshot | streaming events (MessageChunk/ThinkingChunk/ToolCall*/Usage/Plan/…) no longer publish StructureChanged — clients already apply them in place; only structural events (ready/history/approvals/errors/disconnect) trigger a refetch |
| R2 | md file preview very slow; switching to edit froze the page | `md::to_html` / `highlight_auto` recomputed on every parent re-render; the editor's `vdEdSync` re-measured every line per keystroke | md/html render cached per source text in the preview tab; editor sync coalesced via requestAnimationFrame |
| R3 | docx TOC still wrong | TOC threshold `headings >= 2`, English-only title | generated when ≥1 heading; title localized from the document's `w:lang` ("Spis treści" for pl); inserted at the TOC field's position (round 11) |
| R4 | "open elsewhere" badge always on | lease holder with null/unknown PID was treated as "another process" | badge only when `leaseHolder.processId` is known AND differs from our own agent child PID (`Projection.own_pid`) |
| R5 | No active-session marker on the session list | — | green dot on the active session row (stable parity) |
| R6 | ✕ on session rows opened a dropdown | idle ✕ opened the context menu | ✕ renders ONLY on a running session and stops it completely (cancel turn + kill the agent slot/subprocesses via `AgentCommand::Stop`); the menu lives under right-click (PPM); history rows have no ✕ |
| R7 | Opening a file preview failed with os error 3 (paths with spaces) | local `read_file`/`save_file` confined against the ENGINE workdir, ignoring the session root the dock is rooted at | new `confine_scoped(path, remote_root)` — resolves inside the session root when provided; `open_path` also normalizes `\`/`/` + drive-letter case before stripping the root prefix |
| R8 | No red "agent working" marker while resuming | busy row was hidden while `is_resuming` | busy row renders whenever `busy`, including during wake/resume |
| R9 | Edit button on md files did nothing | edit-mode entry froze (R2) and md preview was slow | fixed by R2 caching/debounce; md was already `editable: true` |
| R10 | Wrap button visible outside edit mode | — | wrap toggle now renders only in edit mode |
| R11 | docx narrower than preview: no horizontal scrollbar | `.preview-doc` clipped | persistent bottom horizontal scrollbar (`overflow-x:auto`, `.doc-pages { min-width: max-content }`), same for pdf pages |
| R12 | Attach menu only inserted `@`, no system picker/upload | menu buttons just typed `@` | hidden `<input type=file multiple>` + `ATTACH_PICK_JS` bridge → upload to `.halny-attachments/` via `api_write_binary` → `@path` marker (File = any, Image = `accept="image/*"`); extra "🔍 @" entry keeps the fuzzy search |

Session isolation (audited): each open session owns its slot + worker + agent
process, keyed by local session id (`slot_of`); events route by
`agent_session_id` with current-session preference (`agent_session_idx`), so
3/5/10 parallel sessions never mix contexts or rendering.
