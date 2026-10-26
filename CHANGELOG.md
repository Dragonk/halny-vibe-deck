# Changelog

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
