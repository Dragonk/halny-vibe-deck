# Next-branch fix plan — user-reported defects (round 2)

> Working document. Every reported issue is one row below; the status column is
> updated as the work lands (`[ ]` open, `[~]` in progress, `[x]` done,
> `[—]` dropped/decided otherwise). Source of truth for "did we miss anything".
>
> Reference edition for parity: the stable Electron edition on `main` (v1.0.x).
> Tests run in CI (`cargo test --workspace`, e2e) — the dev machine runs live
> `vibe` sessions that must not be disturbed. Versioning: `1.99.x` prereleases.

## 0. UX decisions taken from the user (override parity where they conflict)

| # | Decision |
|---|---|
| D1 | A session always belongs to a project: there is **no** session without a working directory. |
| D2 | A new session starts from the **`＋` in a project group header** (the project the user picks). |
| D3 | The global button is labelled **“New project…”** and sits **right-aligned next to the `SESSIONS` label**. |
| D4 | SSH connection groups get the same **“New project…”** entry in the session list. |
| D5 | No agent process at boot: the agent is contacted only on the first prompt or when a session is resumed. |
| D6 | Statistics keep exactly: total cost, total tokens, activity, tool calls, all sessions. |
| D7 | Settings lose the agent connect/disconnect panel and the account panel. |
| D8 | Update buttons appear only when a newer version is actually available. |
| D9 | Server settings and SSH remotes are **separate tabs**; remote project directories are edited as rows, not as a textarea. |
| D10 | The server is HTTPS-only (no plain-HTTP app surface). |

## 1. Issue table

Legend: **P** = priority (C critical, H high, M medium, L low).

| ID | P | Issue (as reported) | Root cause found in `next` | Planned fix | Status |
|---|---|---|---|---|---|
| Z1 | C | After sending a message the chat shows only “Agent is working…” and nothing happens | **Self-deadlock**: `apply_slot_event` → `SessionReady` called `Engine::refresh_account()`, which locks the same `state` `Mutex` the handler already holds — the slot pump froze, the pending prompt was never sent and every other thread blocked on the engine state | Account re-read sent directly to the slot (no second lock); the helper now documents the constraint; regression test `crates/mistraldeck-engine/tests/chat_turn.rs` drives the real engine against the stub agent (session/start → turn/enqueue → chunks → completed) | [x] |
| Z2 | H | No “new project” button next to the session list | Sidebar has no `pgh-plus`/new-project entry at all | `New project…` button right-aligned in the `SESSIONS` header (D3) + project picker modal | [x] |
| Z3 | H | No “+ new session in this project” | Same as Z2 — only a single global “New session” button | `＋` in every project group header, starts a draft in that project (D2) | [x] |
| Z4 | H | “New session” is built wrongly (reuses an empty session, no directory choice) | `sidebar.rs` searches for an existing empty session; `Engine::create_local_session()` takes no cwd | Draft session model: cwd is mandatory, no agent until the first prompt | [x] |
| Z5 | H | The app connects to vibe at startup instead of on first message/resume | `launch.rs` calls `engine.prime()` → `start_agent()` + `session/list` before the first snapshot | Remove the boot prime; connect on demand (prompt/resume); history from the on-disk store | [x] |
| Z6 | H | The session list shows no historical sessions | `session/list` results render in a separate “CLI” bucket; `hidden_sessions`/`hidden_projects` (polluted by e2e) additionally hide rows | Import history into the project groups (dedupe by agent id), lazy preview on click, “show hidden” restore | [x] |
| Z7 | M | Statistics have extra elements | Extra live-usage card row + a second “recent sessions” list | Keep only cost, tokens, activity, tool calls + one all-sessions list (D6) | [x] |
| Z8 | M | Refresh and range filters are misplaced | `h2` → pill-row (refresh) → range buttons → cards; stable has a single toolbar | Single `stats-toolbar`: title + range pills + refresh | [x] |
| Z9 | M | Settings: “Connect to agent” / “Disconnect” are useless | `settings.rs` general tab buttons calling `api_agent_connect/disconnect` | Remove the section (agent is automatic, D5) | [x] |
| Z10 | M | Settings: account + connected CLI section does not report state | `account_rows()` renders `account/read` data that the CLI leaves empty | Remove the account section (D7) | [x] |
| Z11 | M | App/CLI update buttons are always visible | Buttons rendered unconditionally, only `disabled` | Render the row/button only when `update_available` | [x] |
| Z12 | H | Server and SSH remote settings share one tab | `remote` tab holds “Remote control” (server) + “Remote servers (SSH)” | Split into `serve` and `remote` tabs (D9) | [x] |
| Z13 | M | Remote project directories entered as textarea lines | `ssh_editor_el` textarea “one per line” | Row list with add/remove (stable parity) | [x] |
| Z14 | H | Server runs over plain HTTP | `launch.rs` binds `http://127.0.0.1:port`, HTTPS only for remote | HTTPS-only listener for both surfaces (D10), webview/`set_server_url` switched to `https://` | [x] |
| Z15 | H | Tools settings differ from stable | Two textareas for `enabled_tools`/`disabled_tools`; no `[tools.*] permission` in the config model | Per-tool permission panel + available-tools list | [x] |
| Z16 | H | Skills settings completely different | Generic `files_panel` | Master-detail: create, import folder/zip, open folder, file tree, delete | [x] |
| Z17 | H | Agents settings wrong | Generic `files_panel` | Master-detail + project/global scope + preview/edit | [x] |
| Z18 | H | Prompts settings wrong | Generic `files_panel` | Master-detail + scope + preview/edit | [x] |
| Z19 | H | TOML tab does not fit the screen and shows the wrong content | `raw` tab side-by-side panes without height/scroll rules; read errors render as an empty editor | Full-height side-by-side layout, explicit error state, save/reload | [x] |
| Z20 | H | Agent scope (local/SSH) and directory change do nothing | Controlled `<select>` (`value` + `selected`) not re-rendering; detail panels keep their “loaded” markers | Signal-driven selects, remount panels on scope key, shared loaded marker | [x] |
| Z21 | M | File preview: edit / open-externally buttons misplaced | Header has title+size+edit+save+close; no path, no download, no open-externally | Stable header: path, edit toggle, save/cancel, download, open externally, close | [x] |
| Z22 | H | File list navigates into folders; no tree, no remembered expansion | `dock.rs::file_row` replaces the listing on directory click | Recursive tree + expansion state in `ClientState` + context menu | [x] |
| Z23 | M | The browser build shows a title bar | `desktop_mode` comes from the `window.ipc` probe | Server-provided mode flag; no header in the browser | [x] |
| Z24 | L | Theme button label “Motyw: Automatyczny (kliknij aby zmienić)” breaks the layout | `i18n.rs` strings carry the parenthetical | Drop the parenthetical in all languages | [x] |
| Z25 | M | Chat input differs: unknown button, attach is 📎 not `＋` | Disabled folder chip + `ic-folder-chip` with 📎 | Single `＋` attach menu; project shown as text in the status bar | [x] |
| Z26 | H | Changing the model does not adapt the thinking options | Reducer never maps config-option updates; `SessionReady` from a snapshot wipes `config_options` with empty lists | `ConfigOptionsUpdated` event + `runtime/read` source; per-model thinking levels | [x] |
| Z27 | H | Mode picker shows “orchestrator” without a description, in every project | Option lists rebuilt from local TOML (`restore_session_chips`), not from the agent/project | Options from agent runtime; project agents only from `<cwd>/.vibe/agents`; descriptions shown | [x] |
| Z28 | H | Many empty black terminal windows while working | `appsrv/conn.rs::Conn::spawn` has no `CREATE_NO_WINDOW` (stable uses `windowsHide: true`) | `creation_flags(0x0800_0000)` on Windows | [x] |

## 2. Phases and acceptance criteria

| Phase | Content | Acceptance |
|---|---|---|
| F0 | Z1 — chat turn | `cargo test -p mistraldeck-engine --test chat_turn` green; stub trace shows `session/start` + `session/turn/enqueue`; a stuck turn fails with an error instead of hanging |
| F1 | Z2–Z6 — sessions/sidebar | No agent process before the first prompt; `New project…` in the `SESSIONS` header and per SSH connection; `＋` per project; history inside project groups |
| F2 | Z25–Z27 — chat | Model change updates thinking levels; modes carry descriptions and stay project-scoped; input toolbar matches stable |
| F3 | Z9–Z13, Z20 — settings order | No agent/account panels; update buttons only when available; `serve`/`remote` split; directory rows; scope switches actually reload |
| F4 | Z15–Z19 — settings panels | Tools permissions, skills, agents, prompts, raw editor match the stable edition |
| F5 | Z14 — HTTPS-only | `http://127.0.0.1:port` no longer serves the app; UI works over HTTPS |
| F6 | Z21–Z22 — files | Tree with remembered expansion; preview header with path/download/open-externally |
| F7 | Z7–Z8, Z23–Z24, Z28 — chrome | Stats reduced to the requested scope; no header in the browser; theme label clean; no console windows on Windows |

## 3. Round status

| Phase | Items | State |
|---|---|---|
| F0 chat turn | Z1 | done, regression test green |
| F1 sessions/sidebar | Z2–Z6 | done |
| F2 chat | Z25 | done; Z26, Z27 open |
| F3 settings order | Z9–Z13 | done (Z20 open) |
| F4 settings panels | Z15–Z19 | open |
| F5 HTTPS | Z14 | done for every hosted surface (desktop webview stays loopback HTTP) |
| F6 files | Z21, Z22 | tree + path + open-externally done; download pending |
| F7 chrome | Z7, Z8, Z23, Z24, Z28 | done |

### Round 2 — every reported issue is implemented

| Item | Notes |
|---|---|
| Z14 | hosted/remote surfaces HTTPS-only; desktop webview exception documented above |
| Z15 | tool permission panel + `[tools.*]` model, tested |
| Z16 | skills master-detail (create/import/zip/open folder/edit/delete), tested |
| Z17/Z18 | agents + prompts master-detail with external open |
| Z19 | raw/TOML tab full-height, error + empty states |
| Z20 | scope selects reload every panel |
| Z21 | download button (local and SSH) |
| Z26 | per-model thinking levels re-derived on model change |
| Z27 | mode options with descriptions, project agents project-scoped |

Known flake (pre-existing, unrelated): `session_token::tests::round_trip_and_tamper`
fails occasionally in a full `cargo test --workspace` run because `cred_version()`
reads the on-disk config; it passes in isolation and on re-run.
