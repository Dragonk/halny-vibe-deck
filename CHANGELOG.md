# Changelog

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
