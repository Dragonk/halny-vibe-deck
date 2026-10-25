# Changelog

## Unreleased (M9.4 — parity with Mistral Vibe Nexus 0.7.1)

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
- **Windows:** panic messages are appended to `vibedeck.log` next to the executable
  (previously invisible when built with `windows_subsystem = "windows"`).
- **Icon:** the executable now embeds the mistral-vibe-nexus icon (PE resource via
  `windres`, added by `crates/vibedeck-web/build.rs`).
- CI: new `ci-dioxus.yml` (workspace tests + windows-gnu check with icon build
  script) and `tag-on-marker.yml` (commit messages containing `[tag vX.Y.Z]`
  create that tag automatically).
- Build & release pipeline: artifacts are built by GitHub Actions only (no local builds);
  a commit whose message contains `[release vX.Y.Z]` creates/moves the tag and runs the
  full release pipeline (test -> bundles -> GitHub release).
