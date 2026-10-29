# Changelog

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
