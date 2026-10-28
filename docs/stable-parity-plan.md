# Stable-edition parity plan for the Rust edition (branch `next`)

> Working plan for reaching full functional and visual parity with the stable
> Electron edition of Halny Vibe Deck (`main`, v1.0.x). Round 1: plan + code
> review (this document). Round 2: implementation. Tests run **only in GitHub
> CI** (or a throwaway Docker container) — never on the dev machine, which
> runs live `vibe` / `halny-vibe-deck` sessions that must not be disturbed.
>
> Sources of truth:
> - The stable repo checkout (`~/projects/halny-vibe-deck`, `main`) — UI structure,
>   assets, `CHANGELOG.md` v1.0.28–v1.0.34.
> - `docs/stable-bugs-report.md` — item-by-item parity audit (older stable fixes).
> - `docs/tui-parity-plan.md` — TUI parity (L1–L4) and `docs/migration-appserver.md`
>   — the app-server migration (F0–F7; F4-UI still open). This plan **adds** to
>   them, it does not replace them.
>
> Constraint from the repo owner: the **logo in the top-left header, the logo in
> the chat empty state, and the app icon must be exactly 1:1 from the stable
> edition** — same geometry, same colors, same files' visual content.

## 1. Branding 1:1 (explicit user requirement)

Current state in `next` is **not** 1:1: `crates/mistraldeck/src/components/brand.rs`
draws an invented "Tatra ridge with foehn-wind gusts" mark (viewBox 96×70, path
geometry, theme-variable colors). The stable edition uses a **pixel-art mark**
(viewBox 1024): rounded-square `#10131A` background, 26 crisp-edge rects forming
an orange/amber pyramid, two wind-swoosh paths (`#F2F5FA`, `#FFD800`), and — for
the full logo — "HALNY / VIBE DECK" wordmark text.

| Item | Stable source | Target in `next` | Acceptance |
|---|---|---|---|
| Header brand (top-left) | `src/renderer/src/components/Header.tsx` → `<img src="./icon.png">` + app name; Sidebar brand row on web | `components/brand.rs` rewritten to the exact stable geometry (rects + paths, hardcoded stable colors — **not** theme variables) | Pixel-identical geometry/colors to stable's `BrandIcon` at the same sizes |
| Chat empty state | `ChatView.tsx` → `<BrandIcon size=72>` | `chat.rs` empty state uses the same rewritten mark | Same |
| Setup screen | `SetupView.tsx` → `<BrandIcon size=32>` | `setup.rs` | Same |
| App icon (window/taskbar/exe) | `resources/halny-icon.svg` → `scripts/make-icons.js` (sharp + png-to-ico: 512/256/128/64/48/32/16) | Regenerate `crates/mistraldeck-app/assets/icon.png` (512) + `icon.ico` (multi-size) + `crates/mistraldeck/assets/icon.ico.b64` (PWA) from the stable SVG | Rendered icon matches stable exe/PWA icon |
| PWA manifest | stable `manifest.webmanifest` has 512 + 256 icons, `id` field | `pwa.rs`: add a 512 icon route, keep 256, add `id` (fixes stable-bugs LOW #14 remainder) | Lighthouse/PWA install shows both sizes |

Implementation note: keep the mark as a Dioxus component (inline SVG) for the
crisp UI renders, and generate the binary assets (png/ico/b64) **in CI or a
Docker container** with the same pipeline as `make-icons.js` (sharp is
Node-only; alternatively replicate with `resvg`/ImageMagick inside the container
and verify sizes). The wordmark logo (`resources/logo.svg`) is used for
README/docs/splash; copy it into `next`'s docs assets if any splash screen needs
it — the UI itself uses the mark, not the wordmark (same as stable).

## 2. Recent stable changes (v1.0.28 – v1.0.34) to port

Audit against the stable `CHANGELOG.md`:

| Stable change | Status in `next` | Work |
|---|---|---|
| v1.0.34 — no false session/prompt timeout on long turns; no mid-turn probing; empty resume history must not wipe the chat | The app-server transport returns on turn lifecycle events, not one giant request — verify no 1 h `REQUEST_TIMEOUT`-style cap exists on the prompt path in `mistraldeck-appsrv`/`mistraldeck-engine`; ensure a resume returning an empty snapshot keeps already-shown messages | Audit + fix + regression test |
| v1.0.33 — theme/logout at the sidebar bottom (under Settings), inline SVG power icon (U+23FB missing from phone fonts) | Verify `sidebar.rs` bottom placement and the logout icon; replace any power-glyph text with the same inline SVG as stable | UI parity |
| v1.0.32 — web connection resilience (parked RPCs during the 3 s reconnect window), heartbeat does not kill slow-flushing clients, permessage-deflate, WS lifecycle logging; SSH groups use the compact "New project…" pill (no blue NEW button) | `serve.rs`/client store: check reconnect parking + `bufferedAmount`-style flush guard equivalents; sidebar SSH group pill | Port |
| v1.0.31 — panel padding cleanup (mobile: no empty gaps beside open panels); sd_notify cgroup self-heal | Padding CSS parity in `shell.rs`/`mobile.rs`; appserver systemd unit support (notify + cgroup move-back) only if the headless server targets systemd units | CSS parity; appserver systemd is a candidate, confirm scope |
| v1.0.30/29 — working sd_notify, mutually exclusive web fabs, event-loop wedge fix, systemd-aware self-update | Mostly Electron-specific; the wedge/wdt equivalents live in `serve.rs` | Audit, port what applies to headless |
| v1.0.28 — mobile web UI fixes, systemd watchdog | Subsumed by the items above | Audit |

## 3. Remaining stable-bugs-report items (from §5, re-checked after a32f9f4)

- **HIGH #1** — app/CLI update must be refused while a turn is in flight
  (`api_update_app` / `api_upgrade_local_cli` need a `slot.busy` check). Still open.
- **HIGH #2** — serve dies after an in-app update under systemd (no pidfile /
  EADDRINUSE recovery). Still open; design together with the v1.0.29–31 systemd items.
- **HIGH #3** — dock/sidebar resize is mouse-only (`RESIZE_JS`); needs pointer
  events for touch/PWA. Still open.
- **HIGH #4** — images to the LLM: **done** in a32f9f4 (image blocks through app-server).
- **MEDIUM #5–#9** (back-gesture popstate stack, SSH host-key hang, yolo auto-approval,
  pending-map drain on worker death, `visibilitychange` resync + queue drain) — open;
  keep the report's §-references as the implementation specs.
- **LOW #10** send-now — **done** in a32f9f4; #12 tool-approval wrapping/mobile, #13
  `CREATE_NO_WINDOW`, #14 PWA 512 icon + `id` (PWA part now in §1 of this plan) — open.

## 4. E2E tests — GitHub CI only

No tests on the dev machine. Strategy:

1. **Scenario suite (golden master from stable).** A Playwright suite driving the
   headless web mode — both editions expose the same browser UI:
   - login page → session shell;
   - cold start without a key → setup wizard (browser-login choice, key login
     error paths, old-CLI `method_not_found` degradation);
   - new session, prompt round-trip, tool-approval dialog, cancel;
   - queue + "send now", theme cycle, logout button placement;
   - sidebar groups incl. SSH pill rendering, mobile viewport layout.
   Host the suite in the `next` repo; run it against stable's headless web
   server (from the stable repo, in CI) to record golden flows/screenshots, then
   against `next`'s headless server.
2. **Mock agent backend.** A scripted `vibe-app-server` stand-in (NDJSON
   JSON-RPC per `docs/appserver-protocol.md`, fixtures from the F0 probe)
   answers `setup/status`, `session/start`, `turn/*`, `session/statsUpdated` —
   deterministic, no API key needed. The same fixtures feed the existing unit
   tests in `mistraldeck-appsrv`.
3. **CI wiring.** Extend `.github/workflows/ci-dioxus.yml` (and release checks)
   with an `e2e` job: build `halny-vibe-deck-appserver` + mock backend, install
   Playwright + webkit2gtk deps, run the suite on every push to `next`. The
   stable-side golden run lives in the stable repo's CI (build headless, run the
   same suite, upload traces); stable stays on `main` and is never broken by
   `next` work.
4. **Docker fallback.** A `Dockerfile.e2e` (webkit2gtk + node + playwright) so
   the whole e2e can run in a container locally *if ever needed* — default is CI.

## 5. Full ACP decoupling — vibe-app-server only (explicit user requirement)

Since a32f9f4 the engine and the web UI speak **only** `vibe-app-server`
(`mistraldeck-appsrv::ApsHandle`). What is still ACP-tainted:

| Residue | Where | Action |
|---|---|---|
| The ACP client crate | `crates/mistraldeck-acp` (client.rs, convert.rs, ~2.2k lines) | **Delete the crate**; remove from workspace + workspace deps |
| Its only consumer | `crates/mistraldeck-app` — the legacy egui desktop GUI, superseded by the Dioxus UI; not shipped in the 1.99.x channel | **Delete the crate** (removing it is what makes deleting `mistraldeck-acp` possible) |
| Legacy release workflow | `.github/workflows/release.yml` (workflow_dispatch, VERSION 0.8.3) builds `mistraldeck-app` | **Delete** the workflow — the release path is `release-dioxus.yml` |
| Legacy type names | `mistraldeck-core::proto`: `AcpEvent`, `AcpCommand`; `BusEvent::Acp` in engine | Rename: `AgentEvent`, `AgentCommand`, `BusEvent::Agent` (mechanical, affects engine + `mistraldeck/src/state.rs` match arms) |
| Legacy field names | `acp_session_id` (session meta), `acp_error` (ClientState), `acpSessionId` if serialized | Rename to `agent_session_id` / `agent_error`; keep the JSON keys if they are part of a persisted/published contract, otherwise rename them too |
| UI strings | `i18n.rs`: `"ACP / Agent"` in 6 languages | `"Agent"` (the transport choice no longer exists) |
| Docs / comments | README ("vibe-acp CLI"), crate docs, `state.rs` comments | Reword to `vibe-app-server` |

Acceptance: `grep -ri "acp" crates/ .github/ README.md docs/` finds no live code
path (only historical changelog entries and the migration doc's history);
`cargo test --workspace` green in CI; binary names and config format unchanged.

## 6. App-server migration leftovers (not superseded)

- **F4-UI** — model / reasoning-effort / agent pickers from `RuntimeSnapshot`
  (`runtime/read`, `agents/list`, `runtime/updated`) — required for visual parity
  with stable's chips.
- **L2/L3** from the TUI plan (queue/steer/retry UI, checkpoints/subagents/lease)
  proceed after stable parity blocks above are green.

## 7. Order of work (round 2)

1. **ACP decoupling** (§5) — structural, removes dead code before parity work.
2. Branding 1:1 (`brand.rs`, icons, PWA manifest) — small, user-visible, unblocks screenshots.
3. Stable v1.0.32–34 parity ports (sidebar logout/theme, SSH pill, reconnect
   parking, false-timeout audit, empty-resume guard).
4. Remaining HIGH parity bugs (#1 update-in-flight, #2 systemd serve recovery, #3 touch resize).
5. E2E: mock backend + Playwright suite + CI `e2e` job; golden run against stable in its repo.
6. F4-UI pickers, then MEDIUM/LOW parity leftovers.

Each step: CI green (`cargo test --workspace`, e2e job) before moving on; dev
builds land as v1.99.x tag artifacts per AGENTS.md.
