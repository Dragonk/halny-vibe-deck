# Changelog

## 1.0.10 — 2026-10-26

### Fixed
- **Attachments silently failed with ENOENT.** `fs:write-binary` never
  created the parent directory - `/tmp/nexus-attachments` does not exist
  on a fresh boot, so every uploaded attachment (web client "+", drag &
  drop) failed with a swallowed error and nothing appeared in the chat.
  Both the local handler and the SSH variant now `mkdir -p` first.
- **A message sent from one browser never appeared on other clients.**
  User prompts are now broadcast by the main process to every connected
  client (desktop window + all web/PWA views) as a `UserMessage` event
  with a deterministic id; the sending client recognises its own echo by
  id, so there are never duplicates.
- **PWA staleness**: the service worker deletes caches of previous
  releases on activation, and the app reloads once when a new service
  worker takes over - a phone can no longer keep running the previous
  bundle. The web manifest now declares a 512x512 icon (any + maskable)
  and a stable `id`, completing installability requirements.
  Note: Firefox Android does not install full PWAs (no WebAPK) - it only
  creates a home-screen shortcut by design; use Chrome for a full
  install.

### Changed
- **Mobile**: the 🆕 new-project button sits at the right edge of the
  sessions header (the close button is gone); preview header actions
  (edit / download) are finger-sized.

## 1.0.9 — 2026-10-26

### Fixed
- **The SSH session list no longer takes minutes on first launch.** Every
  ssh spawn now passes `StrictHostKeyChecking=accept-new` and
  `ConnectTimeout=10` - previously the FIRST connection to a host hung on
  the interactive host-key prompt until the timeout, the status check
  failed and the session list was silently skipped until a later refresh
  cycle. The per-connection session lists are also fetched CONCURRENTLY
  (the old loop stacked whole SSH+ACP handshakes), and the remote group
  header shows a spinner while the list is being fetched instead of
  looking like "no sessions".

### Added
- **File-list context menu** (right-click on desktop, long-press on
  touch): preview, edit (opens the CodeMirror editor with save),
  download (always available - a save dialog on desktop, /api/download
  or a client-side blob in the browser, SSH files included), open
  externally (desktop + local files), rename and delete (both
  remote-aware, delete asks for confirmation).
- **System back gesture/button pops one view layer at a time** on the
  phone: an open panel, then edit mode, preview tabs, Stats/History/
  Settings - exactly like a native app. With nothing open the gesture
  behaves as the browser default.
- **Mobile: the sessions panel has no ✕ close button** anymore - close
  it with the back gesture/swipe or by picking a session.
- **Mobile: the file list gets finger-sized rows** (44 px, bigger text
  and context-menu items).

## 1.0.8 — 2026-10-26

### Fixed
- **Images actually reach the LLM now.** Pasted/attached images were
  preview-only: `sendPrompt` carried text alone and the `@name` marker
  of a pasted image pointed at a blob URL that does not exist on disk.
  Images now travel as real ACP image content blocks (base64 + mime,
  alias from the file name), matching the CLI contract (PNG/JPEG/GIF/
  WebP, max 8 per message, 10 MB each) - validated in the UI with
  clear error messages.
- **The 🆕 new-project button is unusable on phones** (too small and
  covered by the floating ✕ close button): on mobile it gets a real
  44×40 px touch target and the sessions header keeps its distance
  from the ✕; the bottom duplicate button was already removed in
  1.0.7.
- **Per-project "new session" row button on mobile**: every project
  (and SSH remote) group renders a full-size, session-row-sized
  button between the header and its session list - desktop keeps
  the compact "+".

### Added
- **Drag & drop files onto the chat**: dropping files anywhere over
  the conversation attaches them - images become prompt blocks (with
  preview chips), other files are referenced by their host path
  (Electron `webUtils`) or uploaded to the session's host first
  (web client / SSH remotes). Sidebar, dock and modals are
  excluded; drops there behave as before.
- **Vision capability detection**: the app tracks `supports_images`
  per model from the vibe config. When images are attached while the
  active model has no vision, a warning banner explains that they
  may be described by the CLI's vision fallback model or not be
  seen at all - never a silent loss, never a hard block.

### Changed
- **Phones boot into the session list**: the preview dock is closed
  and the sidebar (sessions) panel is open after login, so the
  user can start a new session or resume an existing one instead
  of staring at an auto-expanded preview panel.

## 1.0.7 — 2026-10-26

### Changed
- **Starting the app no longer opens a session.** The main-process boot
  prime (which created an empty "no project" session on every start) is
  gone: no ACP session and no sidebar entry are opened at startup. The user
  starts a session consciously — "+" in a project, the 🆕 new-project
  button — or resumes an existing one. A message typed with no session at
  all creates its draft at send time (a conscious act, not an auto-open).
- **Model / mode / thinking chips are populated from the vibe config
  before any ACP connection** (new `loadDefaultChips` store action): the
  model list comes from `config.toml` `[[models]]`, thinking levels from
  the active model, and the built-in CLI modes (ask / accept-edits /
  smart-approve / auto-approve) are offered up front. Picks made before a
  session exists are carried into the draft and applied inside
  `session/new` with the first prompt.
- **The session list still loads at boot without a session**: local
  sessions arrive with the worker's initial `session/list`, SSH sessions
  via the periodic remote refresh — both independent of the current
  session.
- **Clicking a session loads it from disk, not from ACP.** History
  sessions are previewed from the unified log files (measured ~20 ms for
  the biggest session); a full `session/load` replay over ACP happens only
  when the user actually resumes by sending a message.
- **The local "New project…" button is no longer duplicated**: the bottom
  list button is removed; the single compact 🆕 button in the sessions
  header matches the remote groups' style.
- **The new-project picker starts in the home directory** of the browsed
  host (local/serve host or the SSH remote via `fs:home-dir`), not in the
  filesystem root — navigation up and elsewhere stays available.

## 1.0.6 — 2026-10-26

### Fixed
- **Auto-approval now follows the session's built-in mode.** The permission
  gating was wired to agent profile files (which the UI never writes — the
  GUI's "Autozatwierdzanie" selection was silently dropped on save), so the
  CLI kept asking for every tool call. The manager now tracks the session's
  active mode (from `session/new|load`, `current_mode_update` events and
  confirmed `set_config_option('mode')` calls) and, when the active mode is
  the built-in auto-approve mode ('yolo' or a localized equivalent),
  approves permission requests itself without surfacing them.

## 1.0.5 — 2026-10-26

### Fixed
- **App/CLI updates no longer abort a running turn.** `updates:update-cli`
  killed every vibe worker up front (the binaries are swapped on disk) and
  `updates:update-app` quits or taskkills the whole process tree — neither
  checked whether a turn was running, so an update click mid-turn silently
  aborted the session. Both handlers now refuse with a clear error while any
  local or remote worker has a turn in flight.

## 1.0.4 — 2026-10-26

### Fixed
- **A dead vibe-acp worker no longer leaves the UI hanging.** When the ACP
  worker process exited during a turn (killed by the update helper's
  `app.exit(0)`, or a crash in a long session), every in-flight request sat
  unresolved until its timeout — up to an hour for `session/prompt` — with
  no feedback. The worker exit handler now rejects all pending requests
  immediately with a clear `ACP worker exited before responding (code N)`
  error, so the UI fails fast instead of spinning.
- Note: release 1.0.3 was tagged before this fix landed, so the published
  1.0.3 does not include it; this patch release does.

## 1.0.3 — 2026-10-26

### Fixed
- **Windows self-update no longer hangs on a black terminal.** The update
  helper was spawned with `detached: true`, which on Windows makes libuv
  create the child with `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` and
  skip `CREATE_NO_WINDOW` — `windowsHide` was silently ignored, so `cmd.exe`
  (a console-subsystem program) allocated a visible terminal that never
  closed and blocked the whole update sequence. The helper is now spawned
  with `detached` only on POSIX platforms; on Windows it runs windowless
  (`CREATE_NO_WINDOW`), still outlives the quitting app, and the update
  finishes with the app relaunching automatically.
- **"Update" from the browser now works in headless mode.** The
  `updates:update-app` channel was on the web-blocked list, so the button in
  the hosted (serve) UI did nothing. It is now allowed for authenticated web
  clients: the running app quits, the detached helper swaps the payload and
  relaunches it, and serving comes back on its own (a running server is
  preserved across the restart even when serving is disabled in the saved
  config).
- Auto-approval mode ("Autozatwierdzanie") no longer asks for permission on
  every tool call: when the active agent profile is in `yolo` mode, the main
  process approves permission requests itself instead of surfacing them.

### Changed
- Version bump to 1.0.3; release notes in English.
