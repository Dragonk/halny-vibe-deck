# Changelog

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
