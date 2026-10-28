# Regression report: stable-branch fixes (origin/main, Electron) vs the `next` branch (Rust)

Analysis date: 2026-10-27 07:52 UTC. Window: the last 24 h of stable ≈ v0.13.5 → **v1.0.19**
(commits `bef87ae`…`48c1738`), with emphasis on the priority list (v0.13.15, v1.0.1–v1.0.19).

Method: `git show <sha>` / `git show origin/main:CHANGELOG.md` for stable; reading the
`next` code from the local checkout (crates/mistraldeck*, semantic branches, not file names).

Legend: **YES** = the bug is present in next, **PARTIAL** = partially mitigated / the same gap in a different shape, **NO** = not applicable or already fixed in next.

---

## 1. Table: stable bugfixes → status in next

| Version / commit (stable) | Bugfix | next affected | File(s) in next | Suggested fix |
|---|---|---|---|---|
| v1.0.5 / `45d7ce8` | Refuse app/CLI updates while an ACP turn is in flight | **YES** | `crates/mistraldeck/src/api.rs` (`api_update_app`, `api_upgrade_local_cli`), `crates/mistraldeck-engine/src/updates.rs` (`update_app`) | Add `Engine::any_turn_in_flight()` (iterate `state.slots`, `slot.busy`), refuse the update when true — see §3.1 |
| v1.0.4 / `815354b` | Reject pending ACP requests when the worker dies mid-turn | **PARTIAL** | `crates/mistraldeck-acp/src/client.rs` (`request`, stdout reader) | Fail-fast already exists (poll `exited()` every 50 ms); add a drain of `pending` on `RawMsg::StdoutClosed` — see §3.2 |
| `5ab6b5a` | Never route a prompt into another session | **NO** | — | next has no "last active session" fallback; a prompt always goes to the current session's slot (`send_message_with` → `slot.pending_prompt`, sent with `session_id` in `cmd_prompt`) |
| `a4bcf51` | Fix first prompt going to an invisible session on a fresh install | **NO** | — | next creates and publishes the session server-side: `send_message_with` → `create_local_session()` → `BusEvent::StructureChanged` (lib.rs ~660, ~314); the client receives the event via the stream |
| `faa86ac` | Fix dock and sidebar resizing with pointer events | **YES** | `crates/mistraldeck/src/components/shell.rs` (`RESIZE_JS`, ~238–288), `crates/mistraldeck/assets/main.css` (`.sidebar-resize-handle`, `.dock-resize-handle`) | Switch mouse events to pointer events + `setPointerCapture`/`pointercancel`, `touch-action:none`, a wider hit-area — see §3.3 |
| `d551ecd` | Fix tool-approval dialog layout on desktop and mobile | **PARTIAL** | `crates/mistraldeck/assets/main.css` (`.tool-approval-*`, 3854–3935), `crates/mistraldeck/src/components/shell.rs` (`approval_el`) | Short name + mono chip already there; missing: `overflow-wrap` instead of `nowrap+ellipsis` for long commands, and a mobile `@media` (full-width bar, button wrapping) — see §3.4 |
| v0.13.15 / `bef87ae` (headless/web ×10) | progress updates in headless, binary git diff → preview, replay guard, sendQueuedNow, thinking levels, web update, sidebar 🆕 | **PARTIAL** (see §3.5) | mostly `crates/mistraldeck/src/state.rs` (`drain_queue`), the rest NO | Most don't apply (next's server-side snapshot model); the real remainder: the queue with a stuck `busy` — see §3.5 |
| v1.0.6 / `f54ff21` | Auto-approval follows the session's built-in mode (yolo) | **YES** | `crates/mistraldeck-engine/src/lib.rs` (~1595, `AcpEvent::ToolApprovalRequest`), `crates/mistraldeck-acp/src/client.rs` | When the session's `current_mode_id` == the built-in auto-approve (`"yolo"`/localized equivalent — cf. `lib.rs:1935` `"auto-approve"`), approve it ourselves instead of surfacing — see §3.6 |
| v1.0.8 / `6f726b8` | Images actually reach the LLM (ACP image content blocks) | **YES** | `crates/mistraldeck-acp/src/client.rs` (`cmd_prompt`, ~751), `crates/mistraldeck/src/api.rs` (`resolve_attachments`, ~219) | `cmd_prompt` sends only `[{type:"text"}]`; add base64+mime image blocks — see §3.7 |
| v1.0.9 / `dc3dacf` | SSH session list hang on first launch (host-key prompt) | **YES** | `crates/mistraldeck-engine/src/remote.rs` (`ssh_run`, ~68), `remote_vibe.rs` (ssh spawn) | Add `-o StrictHostKeyChecking=accept-new -o ConnectTimeout=10` to every ssh spawn — see §3.8 |
| v1.0.10 / `f3cc789` | Attachments ENOENT (`fs:write-binary` without mkdir) | **NO** | — | next does not write uploads to `/tmp` during resolve (read-only `resolve_attachments`); upload already does `create_dir_all` (`api.rs:845`) |
| v1.0.10 / `f3cc789` | Message sent in one browser never appears on other clients | **NO** | — | next broadcasts server-side: `BusEvent::UserMessageAdded` (`lib.rs:674`), all clients receive it via the event stream |
| v1.0.10 / `f3cc789` | PWA staleness (SW cache purge, 512px icon, `id`) | **PARTIAL** | `crates/mistraldeck/src/pwa.rs` (MANIFEST, SW) | Verify the manifest has `icons[512]` (any+maskable) and a stable `id`; the SW purges old caches on activate — see §3.9 |
| v1.0.11 / `47c557b` | Joining a running session showed only new messages | **NO** | — | next loads sessions from disk (preview) before joining; the snapshot provides the full history regardless of the stream |
| v1.0.12 / `e486122` | Chrome could not install the PWA (manifest 401) | **NO** | — | `serve.rs::remote_auth` (~286) exempts `/manifest.webmanifest`, `/sw.js`, `/icon-256.png` from auth (comment: "Nexus serves them pre-auth too") |
| v1.0.13 / `0b7d06a` | Reopened PWA saw running turns as idle (`busyTurns`, `acp:busy-sessions`) | **NO** | — | next is snapshot-based: per-session `busy` is included in `api_snapshot` (lib.rs ~236) and after every reconnect the client does a full `refresh()` (shell.rs ~36) |
| v1.0.14 / v1.0.15 / v1.0.18 | Back gesture: LIFO nav stack, pushBack on every entry, double-back-to-exit, `/login` guard | **YES** | `crates/mistraldeck/src/components/mobile.rs`, `shell.rs`, `state.rs` (View) | Zero `pushState`/`popstate` in the web client — see §3.10 |
| v1.0.16 / `9f86dc5` | "Resume failed: Connection closed" — auto-retry resume (4×, 3 s) | **PARTIAL** | `crates/mistraldeck-engine/src/lib.rs` (~1385 `AcpEvent::SessionError`) | Load-failure has a fallback to a fresh session (respawn with `pending_prompt`), but there is no proper retry of the resume on a transient transport hiccup; `pending_prompt` with the `load_session` fallback is lost on a non-load error — see §3.11 |
| v1.0.17 / `438f57d` | Returning to the PWA shows stale data (full re-sync on reconnect + visibility) | **PARTIAL** | `crates/mistraldeck/src/components/shell.rs` (~30–41) | Reconnect → full `refresh()` IS there; no `visibilitychange` handler and no `drain_queue` after the reconnect-refresh — see §3.12 |
| v1.0.19 / `48c1738` | Serving silently dies after an in-app update (systemd orphan, EADDRINUSE) | **YES** | `crates/mistraldeck-engine/src/updates.rs` (`update_app_linux`, ~766), `crates/mistraldeck/src/serve.rs` (`bind`, ~551) | Linux relaunch through the systemd unit when active; pidfile + EADDRINUSE recovery in serve — see §3.13 |
| v1.0.3 / `f2eecd9` | Windows self-update black terminal (`detached` → missing `CREATE_NO_WINDOW`) | **YES** | `crates/mistraldeck-engine/src/updates.rs` (~673: `creation_flags(0x0000_0008)` — only `DETACHED_PROCESS`) | Add `CREATE_NO_WINDOW (0x0800_0000)`: `creation_flags(0x0800_0008)` — see §3.14 |
| v1.0.3 / `f2eecd9` | "Update" from browser in headless mode | **NO** | — | `api_update_app` in next has no web block (only `check(&token)`); progress polling via `api_update_progress` works without a window |
| v1.0.7 / `ae0aab3` | Boot prime: no auto-opening session; chips from config before connecting | **NO / different design** | — | next does not create a session at boot (the session is created in `send_message_with`/`create_local_session` on demand); model/mode choices are applied when the session wakes (`pending_options`, lib.rs ~790) |
| `722279a`–`c865545` (v0.13.14), `f6398d0` | Send-now for queued prompts; always-visible usage bar; restore of the plan on resume | **PARTIAL** | `crates/mistraldeck/src/state.rs` (`queue`, `drain_queue`), `components/input.rs` | The queue exists, but there is **no "send now" button** and no send-now for idle sessions — see §3.5 |
| v0.13.12 / `a769afd` | Resume races: replay guard, Enter guard, hung-worker probe | **PARTIAL** | `crates/mistraldeck-acp/src/client.rs` (`replaying`), `lib.rs` (cancel-watchdog ~831, ~2142) | Replay is server-side (no leakage between sessions); there is a cancel watchdog; there is no "hung worker" probe for the prompt itself (1 h timeout `REQUEST_TIMEOUT`) — see §3.11 |
| v0.13.11 / `e2e4aac` | Duplicate sessions on failed resume; stuck busy on worker death | **NO (mostly)** | — | "Stuck busy" is solved: `AcpEvent::SessionError` clears `busy`/`stopping` (lib.rs ~1385); no duplicate sessions (one slot per session-key, dedupe by `acp_session_id`) |

---

## 2. Verification of the priority list (item by item)

- **"Refuse app/CLI updates while an ACP turn is in flight"** → **YES**, missing in next. Details in §3.1.
- **"Reject pending ACP requests when the worker dies mid-turn"** → **PARTIAL**: fail-fast semantics already exist (a different architecture: a blocking `request()` instead of promises), but there is no drain of the pending map on stdout close. §3.2.
- **"Never route a prompt into another session"** → **NO**: `send_message_with` (crates/mistraldeck-engine/src/lib.rs ~635) routes exclusively to the current session (`st.current_session_mut()`), the prompt is sent with an explicit `session_id` (`cmd_prompt`, client.rs ~751); there is no "worker's last active session" fallback. The SessionInitialized-vs-reply race does not exist, because `pending_prompt` is kept in the session slot and sent after `SessionReady` (lib.rs ~1328, ~1399).
- **"Fix first prompt going to an invisible session on a fresh install"** → **NO**: when `current.is_none()`, next creates a session and publishes `StructureChanged` (lib.rs ~659), so the first message has a visible session.
- **"Fix dock and sidebar resizing with pointer events"** → **YES**: `RESIZE_JS` in shell.rs uses exactly the mouse-only pattern. §3.3.
- **"Fix tool-approval dialog layout"** → **PARTIAL**: the `.tool-approval-input` chip exists, but without wrapping and without a mobile media query. §3.4.
- **v0.13.15 headless/web ×10** → mostly **NO** (next's server-side model eliminates the whole class of renderer-side bugs); the remainder: queue/stale-busy, thinking-levels picker, send-now. §3.5.
- **Prompt queue** → exists (`submit` → `queue`, `drain_queue`), but without "send now" and without resilience to a stale `busy`. §3.5.
- **Reconnect** → the snapshot-refresh on reconnect is there; missing `visibilitychange` and a queue drain. §3.12.
- **UI fixes (layout/mobile/scroll)** → tool-approval §3.4, back-gesture §3.10, resize §3.3.

---

## 3. Implementation guidance for YES/PARTIAL items

### 3.1 Update killing an in-flight turn (v1.0.5) — **YES, HIGH**

Stable: `updates:update-cli` killed the workers, `update-app` did `app.exit(0)`/taskkills — without checking for a turn.

Next: `crates/mistraldeck/src/api.rs`:

```rust
pub async fn api_update_app(token: String) -> ... {
    check(&token)?;
    tokio::task::spawn_blocking(mistraldeck_engine::updates::update_app)  // no busy check
```

`api_upgrade_local_cli` (~422) — same as above. In `crates/mistraldeck-engine/src/updates.rs::update_app()` (453) the only guard is `UPDATE_STATE.running` (a concurrent update, not a turn).

Fix: add an `Engine::any_turn_in_flight() -> bool` method in `crates/mistraldeck-engine/src/lib.rs` (lock `state`, `st.slots.values().any(|s| s.busy)`; slots per local and remote session — also covers remote). Call it at the start of `api_update_app` and `api_upgrade_local_cli`, returning `AppUpdateResult { ok:false, error: Some("A turn is running — update refused") }` (and an analogous `CliUpgradeResult::error`).

### 3.2 Worker death → pending requests (v1.0.4) — **PARTIAL, MEDIUM**

Next in `crates/mistraldeck-acp/src/client.rs::request()` (274–310) checks `exited()` every 50 ms and immediately returns `bail!("vibe-acp exited while handling {method}")` — this gives the same "fail fast" effect as the stable fix (reject instead of waiting for the timeout). Differences:

1. On EOF the reader thread only sends `RawMsg::StdoutClosed` (client.rs ~211) and **does not drain** the `pending` map — requests "hang" in the map until the poll in `request()` — for a blocking client someone is always waiting, so semantically OK; the drain still tidies up state on an unexpected death mid-replay (`drain_raw`).
2. The prompt timeout is `REQUEST_TIMEOUT` (up to 1 h) — if a turn is stuck and the worker is alive but hung, next has no probe (v0.13.12's "hung-worker probe" from stable). `SessionError` clears `busy` (lib.rs 1385), so after an actual death the UI does unblock.

Fix (low cost): in the reader thread after `break`, do a drain: `for (_, tx) in pending_reader.lock().drain() { let _ = tx.send(Err(anyhow!("ACP worker exited before responding"))); }` before `RawMsg::StdoutClosed`. Optionally: a "hung worker" probe in the engine (a slot `busy` for > N minutes with no events → Stop + respawn, as in stable v0.13.12).

### 3.3 Resize handle with mouse events only (faa86ac) — **YES, HIGH (web/mobile)**

`crates/mistraldeck/src/components/shell.rs`, `RESIZE_JS` (~238–288):

```js
document.addEventListener('mousedown', ...)   // arm()
function move(e) { apply(startX - e.clientX); }        // mousemove
document.addEventListener('mouseup', up);               // no pointercancel/pointer capture
```

The same three symptoms as in stable: touch/PWA does not work (mouse events do not fire for touch), a lost `mouseup` outside the window → the panel stuck in resize mode, events swallowed by an iframe. The handles in `components/dock.rs:519` and the sidebar have no `touch-action:none`.

Fix: rewrite `arm`/`drag` to `pointerdown` + `el.setPointerCapture(e.pointerId)` + `pointermove`/`pointerup`/`pointercancel`; add to `crates/mistraldeck/assets/main.css` for `.sidebar-resize-handle`/`.dock-resize-handle`: `touch-action:none;` and a widened pseudo-element hit-area (e.g. `::after { width:12px; margin:-6px }`). Optionally a fallback to the default dock width when nothing is saved (next has defaults in `load()`, so OK).

### 3.4 Tool-approval layout (d551ecd) — **PARTIAL, LOW**

Next already has the name+chip split: `.tool-approval-name` + `.tool-approval-input` (mono, 11px) in `assets/main.css` (3854–3935) and inline `flex-wrap:wrap` on the actions (shell.rs:133). What is missing from the stable fix:

- `.tool-approval-input` uses `overflow:hidden; text-overflow:ellipsis; white-space:nowrap` — stable deliberately switched to wrapping (`overflow-wrap:anywhere`), because truncating a command hides what exactly you are approving;
- no `min-width`/`max-width` clamps with `width:max-content` (present: `min-width:360px; max-width:640px` — OK) and **no `@media (max-width:768px)`** for `.tool-approval-bar` (full-width, smaller bottom offset above the keyboard).

Fix: in main.css `.tool-approval-input { white-space:normal; overflow-wrap:anywhere; }` + a `@media (max-width:768px){ .tool-approval-bar{ left:12px; right:12px; transform:none; min-width:0; width:auto; } }` block.

### 3.5 v0.13.15 (bef87ae) and the prompt queue — **PARTIAL, LOW/MEDIUM**

Breakdown of the ten fixes vs next:

| Fix v0.13.15 | next |
|---|---|
| updates:progress broadcast in headless | NO — progress is polled (`api_update_progress`), works without a window |
| binary git diff → file preview (GitPanel) | to check in `crates/mistraldeck/src/fsgit.rs` / `components/history.rs` — if next renders a binary diff as text, port the `BINARY_EXTS`/"Binary files differ" heuristic |
| replay guard (`setPendingLoadSessionId`) | NO — replay is server-side in `client.rs` (`replaying` map → `SessionHistory`), with no leakage between sessions |
| user message disappearing after a failed resume | PARTIAL — `pending_prompt` exists, but on a non-`load_session` error it is cleared (lib.rs ~1409); see §3.11 |
| attach menu without directories | a UX decision, not a bug — needs a product call |
| web can update the app | NO — no web block in `api_update_app` |
| 🆕 visible without scrolling (sidebar header) | a mobile layout decision — check `components/sidebar.rs` |
| `sendQueuedNow` for idle sessions | **YES-ish part** — next: `drain_queue` (state.rs ~515) only sends when `!busy`; with a stale `busy` (a dead worker without SessionError) the message sits in the queue forever |
| thinking levels: don't override a live session; explicit lists only | to check in the picker (`components/header.rs`/`settings.rs`) — the same principle |

Queue fix: in `drain_queue` add a "busy for longer than X with no event → force send" condition, or add a "send now" button next to a queue entry (UI: `components/input.rs` queue rendering) that clears `queue` and calls `api_send_message` despite `busy` (the equivalent of `sendQueuedNow` from v0.13.15/f6398d0).

### 3.6 Auto-approval per built-in mode (v1.0.6) — **YES, MEDIUM**

Next: `AcpEvent::ToolApprovalRequest` ALWAYS lands in `slot.approval` (lib.rs ~1595) and is surfaced in the UI (shell.rs `approval_el`). There is, however, a parser of `current_mode_id` (client.rs `parse_new_session`, lib.rs ~1242/1280), so the data is available.

Fix: when handling `ToolApprovalRequest` in a slot, check the stored `current_mode_id` (and the result of `set_config_option("mode")`), and when the mode == the built-in auto-approve (`"yolo"`/"auto-approve" — cf. `lib.rs:1935` and localized equivalents), send the approval automatically (`AcpCommand::Approve`) instead of setting `slot.approval`.

### 3.7 Images never reach the LLM (v1.0.8) — **YES, HIGH (feature parity)**

`crates/mistraldeck-acp/src/client.rs::cmd_prompt` (~751): `prompt: [ { "type": "text", "text": text } ]` — text only. `resolve_attachments` (api.rs ~219) inlines files as text (MAX 256 KB); an image read as text is useless to the model.

Fix: (1) in `mistraldeck-core::types::Attachment` add `mime` + `data_base64` (or `kind: Image`); (2) in `resolve_attachments` detect PNG/JPEG/GIF/WebP, read binary, encode base64 (limits: 8/prompt, 10 MB, as per the CLI contract); (3) `cmd_prompt` builds `{"type":"image","data":...,"mimeType":...}` blocks (alias from the file name); (4) UI (input.rs) validation and a banner when the model lacks `supports_images` (cf. v1.0.8 vision-capability detection).

### 3.8 SSH: host-key prompt hangs the first connection (v1.0.9) — **YES, MEDIUM**

`crates/mistraldeck-engine/src/remote.rs::ssh_run` (~68) builds a `Command` from the tokens of `conn.ssh_cmd` and **adds no `-o` options**; the same in `remote_vibe.rs` (spawning vibe-acp over ssh). The first contact with a new host hangs on the interactive host-key prompt until timeout.

Fix: after `tokenize_cmd`, append `arg("-o").arg("StrictHostKeyChecking=accept-new").arg("-o").arg("ConnectTimeout=10")` before `remote_cmd` (note: only for the ssh binary, not plink — possibly a flag in `RemoteConnection`). Bonus from stable: parallel per-host fetching of the session lists (next loads on demand per conn — `api.rs:738` — so the risk is lower, but the timeout is critical anyway).

### 3.9 PWA manifest (v1.0.10) — **PARTIAL, LOW**

`crates/mistraldeck/src/pwa.rs::MANIFEST` — check whether it has a 512×512 icon (`purpose: "any maskable"`) and a stable `"id"`. The SW (`SW` const) only purges old caches if it has an `activate` handler with the list of versions — verify (the CACHE definition is versioned; without purging old names the phone may stay on a stale bundle).

### 3.10 Back gesture / LIFO navigation (v1.0.14/15/18) — **YES, MEDIUM (PWA mobile only)**

The next web client has no `history.pushState`/`popstate` at all (grep: 0 hits in `components/mobile.rs`, `shell.rs`, `state.rs`). On an installed PWA, back will close/reload the app instead of dismissing a layer (dock → preview → file list → sessions).

Fix: in `components/shell.rs` (next to `MOBILE_JS`) implement a layer stack: every open of a session/dock/preview/edit/settings → `history.pushState({nav:layer})`; `popstate` → close exactly one layer (reconcile with `event.state.nav`); a manual panel close → `history.back()` (consume the entry); from boot state: first back → toast, a second one within 2 s → exit. The equivalent of stable's `store.pushBack/removeBack`.

### 3.11 Resume retry (v1.0.16) and hung worker (v0.13.12) — **PARTIAL, LOW**

Next: a `session/load` error → fallback to a fresh session with `pending_prompt` (lib.rs ~1399, comment "CLI session gone from disk") — but that is different semantics than stable: **there is no retry** of the actual resume on a transient transport hiccup (e.g. an app restart mid-resume); next falls back to a new session or shows an error. `pending_prompt` on other errors is cleared (~1409) — the sent text only survives as a user message.

Fix: in the slot, on a `SessionError{context:"session/load"|"session/resume"}` coming from the transport (not "session not found"), retry `LoadBackend/LoadRemote` up to 4× every 3 s before falling back/stopping.

### 3.12 Re-sync after returning to the PWA (v1.0.17) — **PARTIAL, MEDIUM**

`shell.rs` (~30–41): `Err(_)` from the stream → `sleep_ms(2000)` → `refresh(state)` — a full snapshot, a good equivalent of the re-sync after reconnect. Gaps vs stable: (1) no `document.addEventListener('visibilitychange', ...)` → returning to a frozen tab without a torn-down WS waits for the next event; (2) after the reconnect-refresh `drain_queue` is not called (only on `refetch==true` from an event) — a queued prompt is only sent after the next structural events.

Fix: in `shell.rs` add a `use_future` with a `visibilitychange` listener (a JS eval → trigger refresh) and call `drain_queue` after the reconnect-refresh.

### 3.13 Serve dies after an update in systemd mode (v1.0.19) — **YES, HIGH (headless)**

`crates/mistraldeck-engine/src/updates.rs::update_app_linux` (~766): `Command::new(&exe).spawn(); std::process::exit(0);` — a direct relaunch outside the systemd unit. On Linux in serve mode this is exactly the stable scenario: an orphaned instance holds the port, the unit gets EADDRINUSE, restarts accomplish nothing.

Fix: before the relaunch, check the active unit (e.g. `systemctl is-active --quiet <unit>`, the `INVOCATION_ID`/`NOTIFY_SOCKET` env), and if so restart via `systemctl restart <unit>` (the manager survives the cgroup reap); a direct relaunch only for installations without systemd. In `crates/mistraldeck/src/serve.rs` (~551): write a pidfile (e.g. `data_dir/serve.pid`), and on a `bind` error with `EADDRINUSE`: read the pidfile + `/proc/<pid>/cmdline`, identify our own binary (never kill blindly), kill the orphaned predecessor and retry the bind (up to 3×).

### 3.14 Windows: black terminal window on self-update (v1.0.3) — **YES, LOW**

`crates/mistraldeck-engine/src/updates.rs` (~673): `creation_flags(0x0000_0008)` = only `DETACHED_PROCESS`, without `CREATE_NO_WINDOW`. `powershell.exe` (console-subsystem) will show a window and (depending on the scenario) block the sequence.

Fix: `creation_flags(0x0800_0008)` (DETACHED_PROCESS | CREATE_NO_WINDOW) — in Rust the flags are OR-ed; no exception is needed anyway, since the helper is a detached PowerShell.

---

## 4. NO items — not applicable to next (with brief justification)

- **`5ab6b5a` prompt routing** — the bug came from the "worker's last active session" fallback in `sendPrompt`; next never had that fallback (routing via `current_session` + one slot per session + `pending_prompt` in the slot).
- **`a4bcf51` invisible session** — in next session creation is server-side and immediately broadcast (`StructureChanged`); the whole class of "renderer did not link the boot prime" bugs does not exist.
- **v1.0.10 attachments ENOENT** — next does not write uploads to a temp directory on the send path (read-only resolve); upload already does `create_dir_all` (api.rs:845).
- **v1.0.10 prompt broadcast** — built into the bus model (`UserMessageAdded`).
- **v1.0.11 join-session history** — the snapshot/preview from disk provides the full history regardless of the stream.
- **v1.0.12 manifest 401** — `remote_auth` in serve.rs exempts the PWA statics from auth (deliberately, with a comment).
- **v1.0.13 busyTurns** — the per-session snapshot + full refresh on reconnect covers the semantics.
- **v1.0.3 headless update** — no web block in next; progress via polling.
- **v1.0.7 boot prime / chips** — next never opens a session at startup from day one; choices are applied on wake (a convergent design).
- **Electron-specific**: `windowsHide`/taskkills of Electron (v1.0.3), userData migration (14930bf), branding/rebranding assets (14930bf, 6c4f44f, a87db50, d697885) — these concern the Electron/shell JS layer, which next replaces with Dioxus/egui.

---

## 5. Summary: bugs PRESENT in next (priorities)

> Re-audited after the ACP-removal and v1.0.32–34 parity work
> (see docs/stable-parity-plan.md). Items marked **FIXED** below were
> verified in code; the detailed §-references keep the original analysis.

### HIGH
1. **App/CLI update interrupts an in-flight turn** (v1.0.5) — **FIXED**:
   `api_update_app` refuses while `any_turn_in_flight()`.
2. **Serve dies after an in-app update under systemd** (v1.0.19) — **FIXED**:
   the self-update restarts THROUGH the systemd unit (`systemctl --user
   restart`, exit(1) failsafe for `Restart=on-failure`), the bind retries for
   ~9 s during the handover, and the server speaks `sd_notify` (READY=1 +
   WATCHDOG=1 from `WATCHDOG_USEC`). No cgroup self-heal is needed — the Rust
   server never leaves the unit cgroup (Chromium did). → §3.13.
3. **Dock/sidebar resize: mouse-only** (faa86ac) — **FIXED**: `RESIZE_JS`
   uses pointer events + setPointerCapture.
4. **Images never reach the LLM** (v1.0.8) — **FIXED** (image blocks through
   the app-server transport).

### MEDIUM
5. **Back gesture in the PWA closes the app** (v1.0.14/15/18) — **FIXED**:
   `BACK_JS` keeps an in-app LIFO history guard while a panel is open and turns
   `popstate` into "close the topmost panel" (sidebar, then dock). → §3.10.
6. **SSH first-connect hang on the host-key prompt** (v1.0.9) — **FIXED**:
   `StrictHostKeyChecking=accept-new` + `ConnectTimeout=10` in remote.rs.
7. **Auto-approval ignores the built-in yolo mode** (v1.0.6) — **FIXED**:
   engine approves on its own when the session mode is `auto-approve`. → §3.6.
8. **Worker death: the pending map is not drained** (v1.0.4) — **FIXED**: the
   appsrv `call_res` loop probes `conn.exited()` and fails fast, draining
   the pending waiter (v1.0.34 port).
9. **Re-sync after returning to the tab** (v1.0.17) — **FIXED**: `VISIBILITY_JS`
   resync + `drain_queue` after the reconnect refresh.

### LOW
10. **Prompt queue: no "send now" and no resilience to a stale busy** (v0.13.15/f6398d0) — **FIXED** ("Send now" in the input panel).
11. **Resume without auto-retry on a transport hiccup** (v1.0.16) — **FIXED**: `resume_session` reconnects with the remembered argv and retries once on the internal `-32000` transport class. → §3.11.
12. **Tool-approval: long commands truncated (ellipsis), no mobile layout** (d551ecd) — **FIXED**: `.tool-approval-input` wraps (`pre-wrap` + `overflow-wrap:anywhere`) and scrolls inside the dialog. → §3.4.
13. **Windows self-update: black terminal window** (v1.0.3) — **FIXED**: `creation_flags(0x0800_0008)` = DETACHED_PROCESS | CREATE_NO_WINDOW. → §3.14.
14. **PWA manifest/SW details (512 icon, `id`, cache purge)** (v1.0.10) — **FIXED** (version-scoped cache; manifest `id` + 512 icon).
