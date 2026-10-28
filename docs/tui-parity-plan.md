# Parity plan with the Mistral Vibe Rust TUI — implementation in the Halny Vibe Deck GUI (branch `next`)

> Implementation plan (plan only, no code changes). Sources of truth:
> [tui-capabilities-report.md](tui-capabilities-report.md) — **the v3.0.3 TUI study; in case of
> a conflict with [appserver-protocol.md](appserver-protocol.md) the report WINS**
> (the protocol reference mostly describes the Python layer, the report — what the client
> actually sends). Transport status: [migration-appserver.md](migration-appserver.md)
> (F0–F7 done: the `mistraldeck-appsrv` transport works E2E, the `transport=appserver|acp` flag).
>
> Code: `crates/mistraldeck` (a Dioxus fullstack UI: `src/api.rs` = server-fn,
> `src/components/*` = views), `mistraldeck-engine` (the state machine, slots,
> `TransportHandle { Aps | Acp }`), `mistraldeck-appsrv` (the worker: conn/proto/patch/
> reducer), `mistraldeck-core` (shared types: `proto.rs` = `AcpCommand`/
> `AgentEvent`, `snapshot.rs` = `Snapshot`).

## 0. Overriding principles (from the TUI report)

1. **Amounts / "how much of Pro is left" DO NOT EXIST on the wire.** Parity with the TUI means exactly:
   - `plan.title` in the banner (`post_ready.rs:37-43` → the `ui/banner/mod.rs` banner),
   - `planOffer.{kind,url}` — a CTA in the "what's new" banner (`startup/banners.rs:120-131`),
   - `rateLimitAction` — a change of the rate_limit error text + a CTA (`event_handler.rs:1061-1070`),
   - `apiKey.preview/scope/accessScope` + `identity/read` — `/whoami` (`commands/simple.rs:232-280`).
   All from a single source: **`account/read` → `AccountView`** (`models.py:119-126`).
   No counters, reset windows, or Pro consumption — do not design UI for data that does not exist.
2. **The browser login is done by the CLIENT (PKCE), not the server** (`setup/auth/sign_in_flow.rs:30`):
   - `code_verifier` (64 B random), `code_challenge` = BASE64URL(SHA256(verifier)),
   - `POST {browserAuthApiBaseUrl}/vibe/sign-in` `{code_challenge, code_challenge_method:"S256"}`
     → `{process_id, sign_in_url, poll_url, expires_at}`,
   - opening `sign_in_url` (in our case: `window.open` in the client's browser),
   - **poll `poll_url` every 3 s** until `expires_at`: `pending | completed(→exchange_token) |
     expired | denied | error` (3 consecutive network errors = failure),
   - exchange `(process_id, exchange_token, code_verifier)` → `{api_key}` (403 = the workspace
     is not eligible, 410 = expired, 9004 = no access),
   - then `setup/store-credential` → `setup/submit-choices`. No localhost callback.
3. **Key login**: paste → `setup/store-credential {provider, apiKey, customDomain?}`
   → `{outcome: completed|env_var_error|save_error, detail?}` → `setup/submit-choices`
   → `{outcome: completed|provider_config_error, failures[]}`.
4. **Seed**: `setup/status` → `supportsBrowserSignIn`, `hasApiKey`, `provider.browserAuth*`,
   `consoleBaseUrl`, `pinnedConsoleUrl` (a lock on the custom login domain).
5. **Feature detection via `method_not_found` (-32601)** — older CLIs without `setup/*` /
   `account/read`: the account UI degrades, it does not break (the same in the TUI: `setup/auth/rpc.rs:183,193`).
6. The JSON-RPC client already has all the pitfalls handled (string IDs, the `append` patch,
   the watermark, the handoff) — L1-L4 **do not change** the conn/streaming reducer, they only
   **add request/response methods and notifications**.

### Architectural decisions for L1 (before touching files)

| Decision | Choice | Rationale |
|---|---|---|
| Where PKCE + HTTP? | **The server (engine)**, not wasm | The wasm client has no network (no `fetch` in the sandbox, CORS on `api.mistral.ai`); the key is saved by the server anyway (`store-credential` writes to the host keyring). Desktop/webview goes through the same server-fns. HTTP: extend the existing `curl` pattern (`remote.rs::http_get_json`) with a POST + JSON body — zero new heavy dependencies. |
| Randomness source for the verifier | the **`getrandom`** dep (0.2) in `mistraldeck-engine` | No RNG in the graph (no `rand`/`getrandom`; `sha2` is not an RNG). 64 B from the system CSPRNG. Rejected alternative: manual `/dev/urandom` (Windows). |
| BASE64URL | **our own encoder** (~15 lines, unpadded) in `auth_browser.rs` | `base64` is not in the graph; unpadded base64url is trivial code with a vector test. (Alternative: the `base64` dep — acceptable if you prefer the dependency.) |
| SHA256 | **already present** (`sha2 = "0.10"` in `mistraldeck-engine`) | we add nothing. |
| Who keeps the login flow state | `Engine: Mutex<Option<BrowserSignInFlow>>` | Polling goes through separate server-fns (the client every 3 s, `gloo-timers`); the verifier MUST survive between calls → state on the server; `expires_at` enforces the timeout. |
| Which connection calls `setup/*` / `account/read` / `identity/read` | the **lister** (the existing `vibe-app-server` connection) | These are global, not session methods — the TUI calls them before/alongside a session. The engine does `Start` on the list anyway (`start_agent`); the app-server accepts `setup/status` without a session. |
| Opening the browser | the wasm client (`window.open(sign_in_url)`) | The browser is on the user's side; the TUI does `external_url::open`, we already have an open window. `CopySignInUrl` as the fallback (TUI parity). |
| Rate limit | `turn/completed.turn.error.code == "rate_limit"` + `rateLimitAction` from `account/read` | Report §2/§1.1; requires a tolerant change of `Turn.error` to `Value` (currently `Option<String>` — the `{code,message}` object would fail to parse). |
| whoami directly (`GET {console}/api/vibe/whoami`) | **NOT in L1**; described in §Risks as a possible addition | `plan.name` ("Pro") and `plan.title` come from `account/read` anyway; whoami would only add `plan_type`/`prompt_switching_to_pro_plan` at the cost of risk (see §6). |

---

## 1. Stage L1 — must-have (for the v1.99.2 build): Login & Account

**Goal:** the GUI user can log in (browser PKCE or an API key), see the plan in the
banner, get a CTA on a rate limit and a key preview in settings — 1:1 with what the
Rust TUI shows. App start gated when there is no key.

Order of work (build dependencies): **core → appsrv → engine → api.rs → UI → i18n → tests**.

### 1.0 `crates/mistraldeck-core` — types (step 1)

**`src/proto.rs`** — new `AcpCommand` variants (at the end of the enum, without breaking the order):

```rust
    // --- L1: setup / account (global app-server methods, not session ones) ---
    /// setup/status — the wizard seed (supportsBrowserSignIn, hasApiKey, browserAuth*).
    SetupStatus,
    /// setup/store-credential — key persistence (keyring / $VIBE_HOME/.env).
    SetupStoreCredential { provider: Option<String>, api_key: String, custom_domain: Option<String> },
    /// setup/submit-choices — closing out the onboarding (theme/console stay = "leave as is").
    SetupSubmitChoices,
    /// identity/read — /whoami (login, org).
    IdentityRead,
    /// account/read — AccountView (plan.title, planOffer, rateLimitAction, apiKey.preview).
    AccountRead,
```

**`src/proto.rs`** — new `AcpEvent` variants:

```rust
    /// L1: the setup/status response; `unsupported` = method_not_found (old CLI).
    SetupStatusReady { status: crate::snapshot::SetupStatusView },
    /// L1: the key persistence result: "completed" | "env_var_error" | "save_error" (+detail).
    SetupStoreOutcome { outcome: String, detail: Option<String> },
    /// L1: the submit-choices result: "completed" | "provider_config_error" (+failures).
    SetupChoicesOutcome { outcome: String, failures: Vec<String> },
    /// L1: the identity/read response (tolerantly: raw JSON → string).
    IdentityReady { identity: crate::snapshot::IdentityView },
    /// L1: the account/read response.
    AccountReady { account: crate::snapshot::AccountView },
    /// L1: a turn ended with an error (rate_limit etc.) — the wire `turn.error`.
    TurnFailed { session_id: String, code: String, message: String },
```

**`src/snapshot.rs`** — new structs (all `Serialize+Deserialize`, `#[serde(default)]`
on optional fields — parity with the tolerant proto style) + fields in `Snapshot`:

```rust
/// The `provider` from setup/status (report §3.0; models/_setup.py).
pub struct SetupProviderView { pub name: String, pub api_base: String,
    pub api_key_env_var: String, pub browser_auth_base_url: String,
    pub browser_auth_api_base_url: String, pub browser_auth_allow_origin_rewrite: bool }
/// The wizard seed: `setup/status`.
pub struct SetupStatusView {
    pub provider: Option<SetupProviderView>,
    pub console_base_url: String, pub vibe_base_url: String,
    pub supports_browser_sign_in: bool, pub has_api_key: bool,
    pub pinned_console_url: Option<String>,
    /// true when the server does not know the method (method_not_found) — the UI degrades.
    pub unsupported: bool,
}
/// `AccountPlanView` (models.py:101): kind api|chat|mistral_code.
pub struct AccountPlanView { pub kind: String, pub name: String, pub title: String }
/// `AccountActionView` (models.py:96): kind switch_api_key|upgrade_to_pro + url.
pub struct AccountActionView { pub kind: String, pub url: String }
/// `AccountApiKeyView` (models.py:112); accessScope: None on older backends.
pub struct AccountApiKeyView { pub preview: String, pub scope: String,
    pub access_scope: Option<String> }
/// `AccountView` (models.py:119-126) — the ONLY source of account data.
pub struct AccountView {
    pub status: String,                       // ready|missing_key|unauthorized|unavailable
    pub plan: Option<AccountPlanView>,
    pub plan_offer: Option<AccountActionView>,
    pub rate_limit_action: Option<AccountActionView>,
    pub teleport_eligible: bool,
    pub api_key: Option<AccountApiKeyView>,
    pub unsupported: bool,                    // method_not_found on an old CLI
}
/// identity/read → /whoami (commands/simple.rs:232-280); tolerant fields.
pub struct IdentityView { pub raw_json: String }  // the UI reads name/email/org from the raw JSON
```

in `Snapshot`:

```rust
    /// L1: the account (account/read) — None = not queried yet.
    #[serde(default)] pub account: Option<AccountView>,
    /// L1: the login seed (setup/status) — None = not queried / old CLI.
    #[serde(default)] pub setup: Option<SetupStatusView>,
    /// L1: whoami (identity/read) — optional, for settings.
    #[serde(default)] pub identity: Option<IdentityView>,
```

(+ `Default` extended with `None`s.)

### 1.1 `crates/mistraldeck-appsrv` — transport (step 2)

**`src/proto.rs`** — tolerant wire models (no `deny_unknown_fields`):

```rust
pub struct SetupStatusWire { pub provider: Option<Value>, pub console_base_url: Option<String>,
    pub vibe_base_url: Option<String>, pub supports_browser_sign_in: Option<bool>,
    pub has_api_key: Option<bool>, pub pinned_console_url: Option<String> }
// AccountViewWire: status, plan{kind,name,title}, planOffer{kind,url},
// rateLimitAction{kind,url}, teleportEligible, apiKey{preview,scope,accessScope}
pub struct AccountViewWire { /* as above, everything Option */ }
```

**`src/worker.rs`** — new branches in `Worker::handle()` and methods (the same pattern as
`list_sessions`: `self.call(...)` → wire mapping → `AcpEvent` → `self.emit`):

```rust
fn setup_status(&mut self)          // "setup/status" params {} → SetupStatusReady{unsupported when -32601}
fn store_credential(&mut self, provider: Option<&str>, api_key: &str, custom_domain: Option<&str>)
                                    // "setup/store-credential" → SetupStoreOutcome
fn submit_choices(&mut self)        // "setup/submit-choices" params {} → SetupChoicesOutcome
fn read_identity(&mut self)         // "identity/read" params {} → IdentityReady (raw JSON)
fn read_account(&mut self)          // "account/read" params {sessionId?} → AccountReady
```

- `read_account`: the `{sessionId}` param per the report (the `AccountView` request has a `sessionId`) —
  send `{}` when there is no session; the server accepts it (the TUI calls it after starting a session,
  and we also refresh after `SessionReady`).
- **method_not_found detection**: `Worker::call` returns `Err` with `(code, message)`;
  recognize `code == "method_not_found"` or `-32601` (cf. `is_method_not_found`,
  report §5) → set `unsupported: true` instead of emitting `SessionError`.
- **`reducer.rs`** — `turn/completed`: change `Turn.error` from `Option<String>` to
  `Option<Value>` (the wire is a `{code, message}` object; the current parse silently yields `None`)
  and on a non-empty `error.code` additionally emit `AcpEvent::TurnFailed { code, message }`
  (the existing `PromptCompleted` stays — the UI does not lose stop_reason).

### 1.2 `crates/mistraldeck-engine` — the login flow + snapshot (step 3)

**`src/auth_browser.rs`** (a NEW file) — PKCE on the server side:

```rust
/// The browser login flow state; lives in the Engine: Mutex<Option<..>>.
pub struct BrowserSignInFlow {
    pub process_id: String,
    pub sign_in_url: String,
    pub poll_url: String,
    pub expires_at_ms: i64,
    pub verifier: String,          // never leaves for the UI
}
/// The result of one poll (the client calls it every 3 s).
pub enum PollOutcome {
    Pending,
    Completed { api_key: String }, // the exchange happens inside
    Expired, Denied, Failed(String),
}
/// Start: verifier(64 B from getrandom) → challenge=S256 → POST {base}/vibe/sign-in.
pub fn browser_sign_in_begin(browser_auth_api_base_url: &str) -> Result<BrowserSignInFlow, String>;
/// One poll step: GET poll_url (curl); "completed" → an immediate exchange.
pub fn browser_sign_in_poll(flow: &mut BrowserSignInFlow) -> PollOutcome;
fn gen_verifier() -> String;                       // getrandom 64 B → hex
fn base64url_no_pad(bytes: &[u8]) -> String;       // + a vector test
fn http_post_json(url: &str, body: &serde_json::Value, timeout_s: u64) -> Result<String, String>;
//  ← completes http_get_json from remote.rs (curl -fsSL -X POST -H "Content-Type: application/json" -d …)
```

**`src/lib.rs` (`Engine`)**:

```rust
// state
signin_flow: Mutex<Option<BrowserSignInFlow>>,
last_setup: Mutex<Option<SetupStatusView>>,   // the seed for snapshots
last_account: Mutex<Option<AccountView>>,

// API (called from server-fns; they send AcpCommand to the lister / operate on the flow)
pub fn setup_status(&self);                       // → AcpCommand::SetupStatus
pub fn login_browser_start(&self) -> Result<(sign_in_url, expires_at_ms), String>;
    // 1. if last_setup is missing → SetupStatus first; 2. begin() on the provider endpoints
pub fn login_browser_poll(&self) -> PollOutcome;  // poll per call; after Completed → store_credential + submit_choices automatically
pub fn login_with_api_key(&self, key: &str);      // → AcpCommand::SetupStoreCredential + (after a completed outcome) SetupSubmitChoices
pub fn refresh_account(&self);                    // → IdentityRead + AccountRead (also after SessionReady)
```

- the event pump (`drain_*` of the existing loop) handles the new `AcpEvent::SetupStatusReady |
  SetupStoreOutcome | SetupChoicesOutcome | IdentityReady | AccountReady | TurnFailed`:
  storing into `last_setup`/`last_account`, `StructureChanged`/`Acp` on the bus; on
  `SetupStoreOutcome.completed` → an automatic `SetupSubmitChoices`.
- `snapshot()` adds `account: last_account.clone()`, `setup: last_setup.clone()`,
  `identity`.
- **Gating**: in `snapshot()`, when `setup.has_api_key == false` (and `!unsupported`) →
  `Snapshot.status` stays, but the UI decides based on `setup`/`account.status` —
  without forcing it in the engine (the view logic in `app.rs`, the pattern like `setup_needed`).
- `start_agent()` unchanged — the lister starts anyway; call `setup/status` right after
  `Initialized` (the first snapshot will call `setup_status()` when `last_setup.is_none()`).

**`Cargo.toml`**: `getrandom = "0.2"` (+ possibly `base64 = "0.22"` instead of the hand-rolled encoder).

### 1.3 `crates/mistraldeck/src/api.rs` — server-fns (step 4)

```rust
#[server] pub async fn api_setup_account_status(token: String)
    -> Result<(), ServerFnError>;            // trigger: SetupStatus + (after load) Account/Identity
#[server] pub async fn api_login_start(token: String)
    -> Result<LoginStartView, ServerFnError>; // {sign_in_url, expires_at_ms}
#[server] pub async fn api_login_poll(token: String)
    -> Result<String, ServerFnError>;        // "pending"|"completed"|"expired"|"denied"|error
#[server] pub async fn api_login_with_api_key(token: String, api_key: String, custom_domain: Option<String>)
    -> Result<(), ServerFnError>;
```

The UI observes the results through `Snapshot` (`setup`, `account`, `identity`) — in line with the
existing "snapshot + StructureChanged" model, without new channels.

### 1.4 `crates/mistraldeck/src/components` — UI (step 5)

**`src/components/login.rs`** (NEW) — the login wizard, the same screens as the TUI
(`setup/wizard/mod.rs:29-37`):

```rust
pub fn LoginView() -> Element;      // phases: method → browser | apikey (+ error)
// internally:
//  - the method choice screen: browser (if supports_browser_sign_in) / API key
//  - BrowserSignIn: a stepper OpenBrowser → Waiting → Exchanging → Complete;
//    poll in use_future with gloo-timers every 3 s until expires_at; window.open(sign_in_url);
//    the "copy URL" fallback (CopySignInUrl parity); expired/denied → an error screen with retry
//  - ApiKey: input (type=password) + empty-value validation; the env_var_error/
//    save_error outcome → a message with detail; completed → refresh_snapshot
//  - an old CLI (setup.unsupported) → a notice + manual instructions (env var) — no wizard
```

**`src/app.rs`** — start gating:

```rust
// after !s.loaded and setup_needed, BEFORE the Shell:
if let Some(setup) = &s.snapshot.setup {
    if !setup.unsupported && !setup.has_api_key { return rsx! { LoginView {} }; }
}
```

**`src/components/header.rs`** — the account banner (`ui/banner/mod.rs` parity):
a line under/next to the status with `account.plan.title` (e.g. "[Subscription] Pro") when
`account.status == "ready"`; `planOffer` → a small CTA link (opens `planOffer.url`).

**`src/components/chat.rs`** (+ possibly `components/ratelimit.rs`) — rate-limit handling:

- `AcpEvent::TurnFailed` → a red banner above the input: the `message` from the turn error;
  when `code == "rate_limit"` **and** `account.rate_limit_action` is present → the text
  tailored to the action (`upgrade_to_pro` → a CTA link to `rateLimitAction.url`,
  `switch_api_key` → go to LoginView/the key).
  Parity: `rateLimitAction != null` = the server decided the upgrade makes sense (an API plan /
  Mistral Code) — it does not concern the CHAT Pro plan (do not show the Pro CTA to Pro holders).
- the retry banner (`turn/retrying` / `snapshot.retrying`) — **L2** (see §2); L1 leaves it out.

**`src/components/settings.rs`** — a new "Account" section: `account.status`, the plan,
`apiKey.preview` (+ `scope`/`accessScope` when present), a "Refresh" button
(`api_setup_account_status`), whoami from `identity` (the login/e-mail from the raw JSON).
`customDomain` only behind an advanced field (pinnedConsoleUrl locks it).

### 1.5 `crates/mistraldeck/src/i18n.rs` — PL/EN strings (step 6)

New keys: the login method/browser stepper/api-key outcomes, the account banner, rate
limit CTAs, settings-account. (The `tr/tf` pattern exists.)

### 1.6 L1 tests

- **Unit (`mistraldeck-appsrv`)**: parse `setup/status` and `AccountView` from
  JSON fixtures (variants: planOffer/rateLimitAction null; `accessScope` absent —
  "None on older backends"); `method_not_found` → `unsupported: true`;
  `turn/completed` with `error: {code:"rate_limit", message}` → `TurnFailed`.
- **Unit (`mistraldeck-engine`)**: `base64url_no_pad` (RFC vectors),
  the challenge = SHA256(verifier) matching the TUI (`sign_in_flow.rs:137-150`),
  the poll status mapping, expired after `expires_at`.
- **E2E (manual / a marker script, see §7)**: the full wizard, rate limit, banner.

**L1 estimate: 3–5 days.** The highest risk is in the browser flow (live endpoints) —
a `curl` probe before writing the UI (as with F0).

---

## 2. Stage L2 — the next build: queue + steer + retry

- **Transport** (`appsrv`): `AcpCommand::QueueReplace|QueueRemove|QueueSteer|QueueResume|
  QueueEditStart|QueueEditSave|QueueEditCancel|QueueEditAbandon` + `AcpCommand::Steer` →
  the `session/turn/queue/*` methods and `turn/steer` (expectedTurnId — the `STALE_TURN`/
  `NOT_STEERABLE` validation); the reducer: the `turn/queueUpdated` notification → `AcpEvent::QueueUpdated`
  (currently `turnQueue` is kept as a raw `Value` — add a `PublicTurnQueue` model),
  `turn/retrying` + `session/snapshot.retrying` (`PublicRetryState`) →
  `AcpEvent::Retrying { attempt, message }`.
- **Core**: `Snapshot.queue: Vec<QueueItemView>` (the prompt, position, edit state).
- **UI**: `components/queue.rs` — a list of pending prompts under the input
  (inline editing via edit/start→save, removal, "send now" = steer); a
  "retrying…" banner above the transcript; the composer captures the retry prompt (as the TUI
  does on rewind — here: after `retrying`, do not block the input).
- **AC**: a prompt added mid-turn is visible in the list; edit/remove/steer works;
  a dropped connection → after the reconnect the queue is restored from the snapshot.

## 3. Stage L3: checkpoints + subagents + lease

- **Rewind/fork**: `AcpCommand::RewindRead|Rewind|Fork` → `session/rewind/read`
  (`hasFileChanges`, `paths`), `session/rewind` (`entryId`, `restoreFiles`, `inplace` —
  the result: the restored prompt returns to the composer), `session/fork` (`entryId`/`afterTurnId`,
  `attach`) → a new session in the sidebar. UI: a menu on the history entry ("rewind here",
  "restore files", "create a branch"); `checkpoint` entries in the history (report §1.6).
- **Subagents**: `PublicSessionState.childSessions` + `session/childSessionUpdated`
  → `AcpEvent::ChildSessionUpdated`; UI: a docked list with progress/token_usage, opening
  a child's transcript (resume by `session_id`). We leave `session/subagents/*` alone
  (headless-only, experimental). We do not declare the `sessionSpawn` capability (GUI v1
  does not handle the `session/spawn` server request).
- **Lease**: `session/lease/subscribe` + `session/lease/updated|ended` → a
  "session is open elsewhere" badge in the sidebar (blocking concurrent resume/edit).

## 4. Stage L4 (optional)

- **Loops**: `loops/list|create|delete|clear` (the TUI does not call it — the GUI can be first;
  `ScheduledLoop`: `id`, `prompt`, `intervalSeconds`|`cron`, `nextFireAt`); firings
  arrive as the `ScheduledLoopFiredNoticeDetail` notice anyway.
- **Worktrees**: `AgentConfig.worktree` in `session/start` (create|auto|existing) +
  `workspace/git/worktrees/remove` (outcome/holders) — a dialog at startup and cleanup.
- **MCP/connectors UI**: `mcp_catalog/*`, `connector_catalog/*`, `connectors/auth/read`;
  requires adding the `connector_auth` capability + a handler for the `mcp_catalog/authUrl` notification.
- **Teleport**: `vibeCode/projects/*` + `vibeCode/teleport/start` + events.
- **Artifacts**: `artifacts/list` + `artifacts/created|updated` (prStatus, checks).

## 5. Risks and feature detection

| Risk | Mitigation |
|---|---|
| **app-server = an internal API** (`setup/*`, `account/read` may change without deprecation) | tolerant wire models (everything `Option`), pin the CLI version in tests, a curl probe before every release |
| **Older CLIs**: `method_not_found` on `setup/*`, `account/read`, `identity/read` | `unsupported` flags in the views → the wizard hidden, a "account: unknown" banner, manual instructions (env var). The TUI pattern: `is_method_not_found` (report §5) |
| **`apiKey.accessScope` absent** on older backends | an `Option` field; it does not block rendering the preview |
| **The browser flow**: expires_at, denied, 403/410/9004, 3 network errors | a 1:1 mapping from `sign_in_flow.rs`; every status has a screen; the client timeout = the server's `expires_at` |
| **Closing the GUI mid-flow** — the verifier is lost | the flow is one-shot: restarting the wizard does a new POST `/vibe/sign-in` (the old process_id expires on its own) |
| **Remote SSH sessions**: the key is saved on the server host, not the client | `store-credential` via the HTTPS listener concerns the host with the agent — state it clearly in the UI ("the key will be saved on …"); consider `require_local()` as in `api_set_remote` |
| **`pinnedConsoleUrl`** locks the custom domain | the wizard reads the field from `setup/status` and does not offer CustomDomain when the pin is set |
| **whoami directly (`GET {console}/api/vibe/whoami`)** as an addition to "Pro state" | see below — **do not do it in L1** |
| Rate limit: `Turn.error` is an object, not a string | a type change in `proto.rs` + a test (a silent loss of the error with the current code) |
| Wallet creep: showing Pro amounts that are not on the wire | forbidden by principle §0.1; the UI only from `AccountView` fields |

**On the risk of direct whoami (a description — on request, not implemented in L1):**
the server does `GET {console}/api/vibe/whoami` anyway, with a 6-hour cache per key
(`whoami.py:117`), and the only data beyond `account/read` is `plan_type`,
`prompt_switching_to_pro_plan`, `organization_kind`, `api_base`/`vibe_base`.
The client would have to: (1) know `consoleBaseUrl` (from `setup/status`, or honor
`pinnedConsoleUrl`), (2) have the key in plaintext — and after persistence it sits in the
keyring / `$VIBE_HOME/.env` of the server host, so the GUI would have to read `.env` from disk
(fragile, bypasses the abstraction), (3) rely on an internal endpoint that is not
part of the protocol and may change shape without notice. The benefit is marginal
(whoami has nothing that `account/read` does not provide, beyond the
`prompt_switching_to_pro_plan` signal). Decision: **the flow through `account/read`**; whoami
directly only once the server adds fields to `AccountView` or a real need for
`plan_type` in the UI arises.

## 6. What to test — E2E scenarios (against a real `vibe-app-server`)

**L1:**
1. **Cold start without a key**: start the GUI → `setup/status` (`hasApiKey=false`) →
   a LoginView instead of the Shell; the "browser" choice.
2. **Browser login, happy path**: the sign-in POST → `sign_in_url` opened →
   polling `pending`×N → `completed` → exchange → `store-credential completed` →
   `submit-choices completed` → the Shell, a banner with `plan.title`.
3. **Browser login — denied / expired**: polling returns `denied`/`expired`
   (waiting > expires_at) → an error screen; the retry creates a new process.
4. **Key login**: pasting a valid key → `completed`; an invalid one →
   `save_error`/`env_var_error` with detail, no crash; an old CLI (simulating
   `method_not_found`) → the wizard hidden, a message + the env instructions.
5. **Rate limit**: a forced `turn.error.code=="rate_limit"` (an API plan under the limit)
   → a banner with a CTA per `rateLimitAction`; no `rateLimitAction` (the chat Pro plan) →
   just the message, no CTA.
6. **Banner + settings**: after login `plan.title` in the header; the settings
   show `apiKey.preview` (masking), the scope, whoami; "Refresh" works.
7. **Reconnect/restart**: after a GUI restart (a new process) the account/status is restored
   from `account/read`; the snapshot does not lose sessions.

**L2**: enqueue mid-turn → visible in the queue; edit-save changes the content;
remove deletes; steer mid-turn (a `NOT_STEERABLE` error when quiescent — handled);
`turn/retrying` → a banner; a reconnect during a retry → a consistent state.

**L3**: rewind with `restoreFiles=true` restores the files + the prompt returns to the input;
a fork creates a sibling session; live childSessions updates; the lease badge when opening
the same session in the official TUI.

## 7. Verification procedure (shared)

1. A curl probe of `setup/status` + `account/read` against the installed CLI (as with F0)
   before starting L1 work — confirming the wire shape on the version in use.
2. `cargo test -p mistraldeck-appsrv -p mistraldeck-engine` (JSON fixtures from the probe).
3. A GUI smoke: login → session → prompt → banner/rate-limit on a dev build (the next tag).
4. A comparison with the official TUI: the same account data in `/whoami` as in our settings.
