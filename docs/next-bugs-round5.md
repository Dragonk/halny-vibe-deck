# Next-branch bug report — user-reported defects (round 5)

> Working document. Status column updated as work lands (`[ ]` open, `[x]` done).
> All items are critical per the reporter.

| ID | Issue (as reported) | Status |
|---|---|---|
| R1 | **SSH sessions still NOT visible on the session list** (critical, fix first) | [x] — ssh_connect_opts() applied to EVERY ssh spawn (ssh_run, remote_argv, list_remote_sessions) — accept-new + ConnectTimeout; sidebar panes reload on expand and on snapshot change; error hint when the host is unreachable (remote.rs, sidebar.rs) |
| R2 | Sessions show "Sesja bez tytułu" (untitled) until clicked on the list | [x] — session names fall back title -> index_title -> index_preview (first user prompt) at both list sources (engine disk lister + worker session/list); truncated at 80 chars — no more untitled rows before opening |
| R3 | In the preview editor, clicking the scrollbar starts an app RESIZE (dock resize handle overlaps the scrollbar); add a word-wrap toggle in the editor | [x] — resize handles no longer overlap scrollbars: ::after hit areas clipped to the handle strip, .sidebar/.dock reserve a 5px strip (padding + border-box); editor got a persistent word-wrap toggle (localStorage) with matching highlight layer |
| R4 | DOCX preview: headings of different levels not rendered correctly | [x] — docx headings: w:outlineLvl (0-based) wins, style names normalized (Heading1 / "Heading 1" / Nagłówek 1 / Überschrift 1 …) -> h1-h6 |
| R5 | XLSX preview: own scrollbars but content overflows off-screen on smaller windows; scrollbars unusable | [x] — .file-preview-body:has(.preview-sheet) is a flex column with overflow hidden; .preview-sheet fills and .sheet-table-wrap scrolls inside — no more off-screen table on small windows |
| R6 | XLSX preview still not fully handling background colors, formatted tables etc. | [x] — xlsx styles: theme palette (xl/theme/theme1.xml, dk1/lt1 swap), indexed palette, tint, numFmt (custom + builtin: %, decimals, thousands, currency) applied to displayed values; cell css carries bold/italic/color/size/fill |
| R7 | md preview: mermaid fails with `Math.random(...).to_string is not a function` (works in stable) | [x] — MERMAID_JS typo fixed (Math.random().toString(36)) — mermaid@12 renders again in chat AND md preview |
| R8 | Switching to the Git tab flashes terminal windows — audit ALL process spawns on Windows (CREATE_NO_WINDOW) | [x] — halny-vibe-deck_core::util::no_window (CREATE_NO_WINDOW) applied at every spawn: git (fsgit), ssh lister, uv probe (setup), open-externally; existing engine/appsrv spawns audited |
| R9 | Settings → Tools on phone: the tools list should be collapsible and ABOVE the configuration, not beside it | [x] — tools tab: available-tools panel has a collapse toggle and stacks ABOVE the permission config on phones (media query, order:-1) |
| R10 | Settings → Skills: does not fit the screen, editor tiny — use the same syntax-highlighted editor as the preview | [x] — skills detail uses the preview CodeEditor (syntax highlighting by file name) in a filling editor box; mobile layout stacks file list above editor |
| R11 | Settings → Prompts: same — no real editor, doesn't fit the screen | [x] — prompts/agents file editor: textarea -> CodeEditor with highlighting, fills the panel (settings-editor-box flex), min-height 42vh on phones |
| R12 | Settings → TOML tab does not fit a phone screen (fine on desktop) | [x] — TOML tab stacks on phones (dirtree-col 32vh on top, editor 46vh below); master-detail stacks on mobile generally |
| R13 | Resumed session: agent's tool calls render as RAW JSON instead of proper cards; messages appear out of order / swap while the agent works; auto-scroll yanks to bottom while scrolling up | [x] — wire effect entries mapped to rich ToolCall (toolName/kind/display.summary/input/state.output/state.status); status_str reads state.status; cancelled->Error; auto-scroll only when near bottom or on session switch (force) |
