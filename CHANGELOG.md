# Changelog

## 1.0.37 — 2026-10-29

Stability release: five long-standing problems reported from real use (a phone
PWA and a desktop window sharing one server).

### Fixed

- **A hidden tab no longer loses the middle of an answer.** The transcript is
  assembled in the main process once, with a monotonic per-session `seq`.
  Every event carries `msgId` + `seq`, the client detects a gap (frozen tab,
  dropped socket) and pulls a snapshot instead of waiting for F5. Before, the
  only "catch-up" was re-reading the session file, and sessions that were still
  busy - exactly the ones that lost events - were skipped.
- **Long sessions no longer show reordered, duplicated or glued messages.**
  Text/thinking deltas are offset-checked (a duplicate is dropped, a missing
  chunk triggers a snapshot) and the old prefix heuristic with its 40-character
  threshold - which duplicated every resent text shorter than that - is gone.
  Tool calls attach to the message the registry assigned them to instead of
  "the last AI message", and events of an unknown session are no longer routed
  into the currently viewed one.
- **The login survives app restarts and long absences.** Session tokens are
  stateless HMAC blobs (safeStorage-protected key, sliding 180-day window, a
  one-year absolute cap, `ver` bumped on password change, a persistent revoke
  list). They used to live in a `Map`, so every self-update or watchdog restart
  logged every device out, and the cookie was never renewed. The SPA now calls
  `POST /api/auth/refresh` (a websocket cannot set cookies), and the login rate
  limiter counts failed attempts only - eight successful logins used to return
  429.
- **SSH sessions are visible on the phone again.** A browser `config:save` could
  overwrite the whole config server-side (only the `serve` section was
  protected), wiping `remoteConnections` permanently. `remoteConnections` is now
  protected too. The SSH list itself is owned by a main-process registry
  (single-flight, 30 s TTL, persisted last known list, broadcast to every
  client), so a fresh client no longer depends on one fragile probe: the old
  status gate silently dropped every session when `ssh` timed out, and failures
  had no message at all.
- **"Send now" from the queue actually delivers that prompt next.** The queue is
  server-owned: `promote` + cancel + wait for the turn to end + flush. The old
  button only reordered the local view (overwritten by the next broadcast) while
  `deliverQueued` refuses to deliver during a turn. The panel also keeps
  `acpSessionId` now - it used to list prompts of other sessions, which then went
  to the wrong chat - and editing a queued prompt no longer moves it.
- **Subagents show real progress instead of a green tick.** The app itself closed
  a spawn on `PromptComplete` while the child kept working. A tracker now follows
  the child session (`unified/child-*`), pushing elapsed time, steps and live
  work, and the card keeps a subagent running until the child finishes.
  `agents.list` renders its agent table (the result was dropped by the raw-output
  filter, so the card showed only "Listed subagents").
- **Every tool call shows a spinner and a live counter while it runs** - builds,
  test suites and agent turns can take 10, 30 or more minutes, and a card no
  longer flips to "done" or "error" before the tool reports it.
- A half-open websocket is detected in seconds (client ping/pong, 70 s budget)
  instead of waiting for the server's 60 s termination, and `pageshow`, `online`
  and `focus` trigger the same catch-up as `visibilitychange`.

### Added

- `npm run test:unit` (node:test, no extra dependencies): token format
  (signature, expiry, absolute cap, credentials version, revoke list) and the
  session registry (delta offsets, resend handling, segment rules).

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.36...v1.0.37

## 1.0.36 — 2026-10-28

### Fixed

- **Model, mode and thinking can now be changed on a session that has not been
  resumed yet (sidebar preview).** A lazy session has no ACP session in the
  CLI worker, so session/set_config_option always failed with
  "Session not found" — v1.0.35 made that failure loud, which surfaced the
  gap. Selecting a value on a lazy session now records it on the session and
  applies it inside session/new|load (main process, before any prompt can
  race it); the chip self-corrects to the CLI-confirmed value once the load
  settles.
- **The answer of the first prompt after a resume is no longer swallowed.**
  The recorded-model apply inside session/load makes the CLI emit a
  checkpoint tool call that seeded the streaming state before the replayed
  SessionHistory replaced the message array — every later ThinkingChunk and
  MessageChunk then appended to a deleted message id and the UI looked
  frozen. The streaming state is now cleared when a replay replaces a
  session, and chunk handlers verify their target bubble still exists
  before appending.
- **The user's own prompt bubble survives a resume replay.** The optimistic
  echo could be wiped by the SessionHistory replace when the session/load
  result was processed before the replay event: the re-add check saw the
  echo still present, skipped, and the replace then deleted it.
  applySessionHistory now preserves trailing local user messages the
  replay does not know about, whichever side of the race wins.
- **Renderer event-handler exceptions are no longer swallowed silently.**
  The web bridge catches listener errors with an empty catch; the App's
  ACP event handler is now wrapped so a throwing case logs to the console
  instead of invisibly eating a stream of events.

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.35...v1.0.36

## 1.0.35 — 2026-10-28

### Fixed

- **A model/mode change the CLI silently refuses is now visible instead of
  silently ignored.** When session/set_config_option fails (for example the
  CLI process does not know the requested model, so it answers with a
  JSON-RPC error), the chip no longer keeps the optimistic value: the previous
  setting is restored, a red banner explains what failed and why, and the
  main process logs the refused set_config_option with its error code. Before,
  the failure was swallowed: the chip showed the new model while the session
  kept running on the old one.
- **Main-side verification that a model change actually applied.** The parsed
  set_config_option confirmation is now checked for model changes: if the CLI
  acknowledges the request but the running model is still the old one, an
  error is thrown so the UI can revert and inform.

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.34...v1.0.35

## 1.0.34 — 2026-10-28

### Fixed

- **Long-running agent turns no longer hit a false ACP request timeout on
  session/prompt.** In ACP, session/prompt only responds when the agent
  finishes its whole turn. A 1-hour cap wrongly fired while an agent was
  legitimately still working (SSH agents can run for many hours), and the
  error cascaded: busy-tracking was cleared mid-turn, a stop+restart could
  kill the worker in the middle of a turn, and resuming afterwards could
  leave the chat empty. The prompt request now waits without a timeout,
  which is safe because the worker exit handler rejects all pending requests.
- **No probing of a running agent mid-turn.** acp:start no longer sends a
  test prompt to a running worker whose turn is in flight (the same guard
  ensure-worker already used).
- **Empty resume history no longer wipes the chat.** If a resume returns an
  empty history while messages are already on screen, the visible messages
  are kept and a notice is shown instead of replacing the chat with nothing.

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.33...v1.0.34

## 1.0.33 — 2026-10-28

### Fixed

- **Web client: theme and logout live only at the bottom of the sessions
  list (under Settings), as before.** v1.0.32 wrongly added a second pair
  to the top brand row - that pair is gone. The bottom logout icon is now
  an inline SVG instead of the U+23FB power-symbol glyph, which is
  missing from phone system fonts and rendered as an empty box, so the
  button icon now renders on phones exactly like in the desktop browser.

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.32...v1.0.33

## 1.0.32 — 2026-10-28

### Fixed

- **Web client: connection lost while resuming a big session.** The whole
  session history is broadcast as one WebSocket frame; on a slow mobile
  link (through a reverse proxy) the frame flush could outlast the
  30 s heartbeat rounds and the server terminated the client mid-resume
  ("Connection to the app server was lost"). Frames are now compressed
  (permessage-deflate, roughly an order of magnitude smaller on the
  wire), the heartbeat no longer terminates clients that are still
  flushing a pending payload, and RPC calls made during the 3 s
  reconnect window are parked and replayed instead of failing instantly.
  The serve module also logs WebSocket connect/close/terminate and large
  broadcasts for future diagnosis.
- **Web client (SSH sessions): the blue NEW button is gone.** Remote
  connection groups now use the same compact „New project…” pill as
  the sessions header (mobile row included).
- **Web client: the logout button renders again (all widths, including
  phones).** It used to exist only in the desktop Header, which is not
  rendered in web mode at all; the sidebar brand row now owns the theme
  toggle and the logout button.

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.31...v1.0.32

## 1.0.31 — 2026-10-28

### Fixed

- **Headless serve: the watchdog restart loop, properly this time.**
  Chromium migrates the browser process into a transient app-*.scope a
  few seconds after startup; sd_notify messages sent from the scope are
  ignored by the service unit, so even with the v1.0.30 systemd-notify
  fix the watchdog never saw a single ping and restarted the healthy
  service every WatchdogSec (90 s). The app now remembers the service
  cgroup it was born in and moves itself back before every notify call,
  so READY and WATCHDOG pings always arrive attributed to the unit.
- **Web client (mobile): no more empty gaps beside open panels.** The
  sessions header and the preview-dock tabs no longer reserve space for
  the floating buttons, which stay hidden while a panel is open.

**Full Changelog**: https://github.com/Dragonk/halny-vibe-deck/compare/v1.0.30...v1.0.31

## 1.0.30 — 2026-10-28

### Fixed

- **Headless serve: the systemd watchdog restart loop.** The v1.0.28/1.0.29
  sd_notify implementation never sent a single byte - Node's dgram has no
  unix_dgram support, createSocket("unix_dgram") throws, and the silent
  try/catch hid it. A perfectly healthy service was restarted every
  WatchdogSec (90 s) while serving fine in between. Notifications now go
  through the systemd-notify helper (--no-block; abstract and filesystem
  NOTIFY_SOCKET paths handled natively).
- **Web client (mobile): the floating buttons are mutually exclusive.** An
  open sessions panel hides the preview-dock FAB and an open preview dock
  hides the sessions FAB; each panel closes with its own X button.

## 1.0.29 — 2026-10-28

### Fixed

- **Headless serve: the event loop can no longer be wedged by CLI version
  checks.** setup/update checks ran execSync (`which`, `vibe --version`)
  in the main process - when the spawned shell died but its grandchildren
  kept the stdio pipes open, the whole loop froze forever: port open,
  zero responses - the silent 521 behind the reverse proxy. All of
  these now run asynchronously with process-group kills on timeout;
  nothing in the serve path blocks the loop anymore.
- **Self-update on a systemd deployment no longer kills the service.**
  The Linux self-update used app.relaunch() + exit(0): under a user unit
  the relaunched child dies with the cgroup and exit(0) reads as success,
  so Restart=on-failure never fired. The updater now asks systemd to
  restart the unit (NOTIFY_SOCKET detection, v1.0.28+ deployments) and
  exits non-zero as a failsafe.

### Added

- Event-loop lag monitor: stalls over 2 s are logged to the journal as
  postmortem evidence for watchdog restarts.
- Windows: the failed-update notice dialog is skipped in headless mode.
## 1.0.28 — 2026-10-28

### Fixed
- **Mobile web: no more covered elements.** With the preview dock open, the
  sessions button stays reachable and the dock closes with its own X button;
  with the sessions list open, the dock button stays reachable. Tab strips
  and the sidebar header reserve space for the floating buttons. Edge swipe
  gestures are unchanged; picking a session or tab still hides the list.
- **Headless serve: systemd watchdog.** The app now sends sd_notify pings
  from the main loop; when the loop wedges (port open, no HTTP responses),
  systemd restarts the service instead of leaving a dead endpoint behind
  the reverse proxy.

### Changed
- Sessions sidebar: the NEW icon is now a translated 'New project…' text
  button (en/de/es/fr/it/pl).
- Release workflow uploads only latest.yml / latest-linux.yml (no more
  builder-debug.yml leak in release assets).

## 1.0.27 — 2026-10-28

### Added
- Web client: no title bar chrome - the sessions list and the preview dock
  open via floating buttons and edge swipe gestures; theme and logout are
  available from the sessions sidebar.

### Fixed
- ACP worker cwd for all-users (Program Files) installs.

## 1.0.26 — 2026-10-27

### Added
- **Drag & drop upload into the project file tree.** Drop files from the
  host onto any folder in the Files dock (or the tree root) to upload
  them into that directory - works locally and over SSH remote
  connections (`fs:write-binary` creates parent directories).
- Folders highlight while a drag hovers over them; hovering a folder
  briefly auto-expands it so the drop target is visible.

### Fixed
- **"Edit" on a `.md` file opened preview instead of the editor.** The
  preview load effect unconditionally reset `editMode` after the tab
  was opened with `startEdit`. The editor now re-arms from the tab's
  `startEditSeq` once the content finishes loading, so both "Edit"
  (existing tab or new) and "Preview" behave as chosen.

## 1.0.22 — 2026-10-27

### Fixed
- **The MCP warning blocked ALL sends with a permanent stop button.**
  Full audit (CLI sources + how other ACP clients behave): the CLI's
  session warm-up emits the MCP-OAuth hint as an OUT-OF-BAND
  agent_message_chunk - BEFORE any turn - and the app's chunk handler
  marked the session BUSY on every chunk. With no turn there is no
  PromptComplete, so the flag never cleared: the send button stayed a
  stop button, sends queued up waiting for a turn end that would never
  come, and even /mcp login could not be delivered. A client-side
  busy flag from chunks is a HEURISTIC, not the truth.

  Now - like other ACP clients - the SERVER owns the turn state:
  - sends check acp:busy-sessions when the client thinks the session
    is busy: no server-side turn = send immediately (the stale flag
    is healed);
  - chunk-driven busy flags self-verify 1.5 s after the last chunk
    and clear when the server reports no turn;
  - the PWA wake-up re-sync heals busy sessions whose turn ended
    while the page was frozen (a missed PromptComplete).

  After the fix the flow works end to end: the warning appears (just
  a warning), /mcp login <alias> sends normally, the CLI streams the
  OAuth URL, the login completes in the local browser and the same
  session continues.

## 1.0.21 — 2026-10-27

### Fixed
- **A queued message died with the worker after a stop.** The exact
  MCP-over-SSH deadlock: the agent's turn hangs (the MCP server waits
  for OAuth), the `/mcp login <alias>` command goes to the queue (the
  session is busy - correct), the user hits stop - and the turn-end
  queue flush sent the command with NO recovery: the worker had died
  with the hung turn, so the flush surfaced the raw
  "Remote ACP worker not started" error and the command was lost.

  Both delivery paths (the turn-end queue flush in App and the
  force-send button in the store) now use the SAME recovery as the
  send-time reviver: revive the worker, restore the CLI session from
  disk (or recreate it when it never had history), then deliver.
  Queued messages also carry the deterministic echo id now, so other
  clients (the phone) see them too.

## 1.0.20 — 2026-10-27

### Fixed
- **A dead draft session no longer dead-ends the chat.** After a
  cancelled turn (e.g. an MCP-OAuth login waiting in flight) the CLI
  session could be really gone - the next send failed with "ACP session
  does not exist" / "Resume failed" and nothing could be delivered.
  For a draft with no AI messages yet the send now RECOVERS: it relinks
  the draft to a fresh ACP session (with the recorded model/mode/
  thinking) and delivers the prompt there - the sidebar entry and the
  message stay, nothing splits. Sessions with real history still
  surface the error (no silent chat splits).

### Documented
- **MCP OAuth over SSH works from the chat**: `/mcp login <alias>` sent
  as a normal message reaches the CLI (an ACP command), which streams
  the authentication URL - open it in the local browser, complete the
  login and the session continues ("authenticated."). While the login
  turn waits for your browser, the session is busy - that is the
  expected state; use stop only to abandon the login.

## 1.0.19 — 2026-10-27

### Fixed
- **Serving silently dies after an in-app update (critical for
  headless).** The Linux portable update helper relaunched the
  executable DIRECTLY, outside the systemd unit: the orphan
  instance holds the serve port forever, every later unit
  start fails with `EADDRINUSE` (the unit runs but serves
  nothing), restarts never take the port back, and two
  instances fight over the user data dir. Clients hanging
  on the orphan lost the WS whenever the orphan was
  replaced, and the unit's instance never served.
  - The update helper now restarts **through the systemd
    unit** when one is active (the manager survives the
    cgroup reap, so the restart always proceeds); the
    direct relaunch stays for non-systemd installs.
  - Serving now writes a **pidfile** and RECOVERS from
    `EADDRINUSE`: the orphaned previous instance is
    identified (via /proc cmdline - never killed blind)
    and killed, and the port is rebound (up to 3 attempts).

## 1.0.18 — 2026-10-27

### Fixed
- **The back gesture in the installed PWA closed the app or reloaded
  through /login instead of going one view back.** Full audit of the
  navigation contract per the documented PWA behaviour ("installed
  PWA with no history: back closes the app"): every forward
  navigation must PUSH a history entry. The previous implementation
  registered view layers only in the app store and never pushed
  history entries on forward navigation, so the browser stack
  stayed empty and every back press fell out of the app (or
  into the /login 302 redirect - a full reload landing on the
  boot state).

  Now the documented contract is implemented end to end:
  - every layer pushes a real history entry (store.pushBack),
  - popstate pops exactly ONE layer, reconciled against the
    landed entry (`event.state.nav`) - manual panel closes
    (menu/swipe) consume their entry too (removeBack), so the
    stack never desyncs,
  - a guard entry + a 2 s window: back from the boot state
    shows a toast; a second back within 2 s closes the app.

  The full gesture path: edit → preview → file list → dock →
  session list → toast → double-back closes.

## 1.0.17 — 2026-10-27

### Fixed
- **Returning to the PWA shows only stale data.** Closing the PWA
  never stops anything server-side (the turn keeps running), but
  the client coming back pulled nothing: a background-frozen page
  misses streamed events (even with a live WS it is suspended),
  and a WS reconnect only re-synced busy flags. Now every return
  - WS reconnect OR the page becoming visible again - triggers a
  FULL re-sync: busy/live flags, the complete on-disk history of
  every loaded session and the SSH session lists. The agent's
  whole story from the background period is always there; a
  locally-streaming session is skipped so its live tail is
  never clobbered.

## 1.0.16 — 2026-10-27

### Fixed
- **"Resume failed: Connection closed" after a server restart.** A
  transport hiccup (the app restarting while a session was being
  resumed, a WS blip) rejected the pending session/load and left a
  dead-end error banner with an empty chat. Both resume paths (the
  sidebar open + the send-time reviver) now retry automatically
  (up to 4 attempts, 3 s apart) while the web bridge reconnects -
  a session opened during an app update resumes on its own once
  the server is back.

## 1.0.15 — 2026-10-27

### Fixed
- **Every way of entering a session now registers a back layer.**
  Starting a session with "+" (per-project, per-remote or the new
  project picker) pushes the same "return to the session list"
  layer as opening an existing one - previously only the
  existing-session path did, so the back gesture could hit an
  empty stack (toast / app close) right after starting a
  session.

## 1.0.14 — 2026-10-26

### Fixed
- **The back gesture skipped whole screens and could close the app.**
  The navigation is now a strict LIFO stack (Inboxora-style): every
  navigation pushes one layer - entering a session (back reopens the
  session list), opening the dock (back closes it), opening a file
  preview (back returns to the file list), entering edit mode (back
  returns to the preview), opening Stats/History/Settings (back
  returns to the chat). Closing a panel by any path removes its
  layer, so the order always matches what the user sees. The app
  closes only from the boot state - first back shows a toast,
  a second back within 2 s confirms.

## 1.0.13 — 2026-10-26

### Fixed
- **The PWA back gesture closed the whole app.** With nothing left to pop
  the gesture now shows a "Press back again to exit" toast; only a
  confirmed double-back within 2 s lets the app close - the native-app
  pattern. Popping view layers (panels, edit mode, previews, Settings)
  works as before.
- **A reopened PWA saw running turns as idle/"stalled".** The main
  process now tracks which ACP sessions have a turn in flight
  (`busyTurns`) and every freshly booted / reconnected client marks them
  busy + live (`acp:busy-sessions` + `syncBusySessions`), including a
  disk-history load for the current one - a turn started on the computer
  keeps reporting into the phone correctly after the PWA is closed and
  reopened.

## 1.0.12 — 2026-10-26

### Fixed
- **Chrome could not install the app as a PWA.** The serve mode guards
  every route behind the auth cookie, but Chrome fetches the web app
  manifest (and the install icons) WITHOUT credentials - it always got a
  401 and the install prompt never appeared. The manifest link now sends
  `crossorigin="use-credentials"`, and the PWA install assets (manifest,
  service worker, icons) are served without the auth cookie - none of
  them carries user data.

## 1.0.11 — 2026-10-26

### Fixed
- **Joining a running session showed only the new messages, no history.**
  The click handler loaded the on-disk history only when the session had
  ZERO messages - but a client joining a turn in flight had already
  received the streaming tail (ACP events go to every client), so the
  guard never fired and the phone showed just the last messages with the
  whole history missing. Sessions are now loaded from disk until the
  history IS loaded (a per-client flag set by applySessionHistory); the
  file preview is a ~20 ms idempotent read, and a client driving a busy
  turn skips the reload so its live tail is never clobbered.

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
