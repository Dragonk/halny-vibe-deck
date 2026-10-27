# Mistral Vibe Rust TUI — user-facing features and the vibe-app-server wire protocol

> Code study of `mistralai/mistral-vibe`, checkout **v3.0.3** (`/home/dragonk/projects/mistral-vibe-src`,
> commit `7bb48f2`). The code takes precedence over `docs/appserver-protocol.md`.
> Paths: `cli-rust/src/…` = Rust TUI, `app_server/…` = Python backend (the JSON-RPC server).
> The given `file:line` references apply to the v3.0.3 checkout.

---

## 0. Architecture TL;DR

The Rust TUI (`cli-rust/`) is a **thin client** of the vibe-app-server (a Python `vibe-app-server`
process, spawn: `cli-rust/src/server/process.rs`). All session logic lives on the server side; the TUI:

- sends JSON-RPC 2.0 requests over stdio (NDJSON); the full list of methods the **TUI actually calls**:
  `cli-rust/src/server/types.rs:10` (`pub mod method` — ~70 constants);
- handles server→client notifications: `cli-rust/src/server/types.rs:128` (`pub mod notification`);
- responds to **server requests**: `callback/call` (`server_method::CALLBACK_CALL`, `types.rs:120`)
  for kind `approval` and `user_input` (`types.rs:124`, `server/callback.rs:85`);
- reduces notifications into UI state: `cli-rust/src/event_handler.rs` (e.g. `history/entryAdded` :659,
  `history/entryUpdated` :697, `session/snapshot` :713, `turn/queueUpdated` :852,
  `session/statsUpdated` :857, `session/childSessionUpdated` :894).

Methods that exist in `app_server/protocol.py:108` (`SERVER_METHODS`) but are **unused by the
TUI** (they are for Desktop/ACP): `session/pin`, `session/archive`, `session/markAsSeen`,
`session/title/update`, `session/relocate`, `session/settings/update`, `session/turns/list`,
`session/history/get`, `loops/*` (the TUI does not call them directly — see §Loops), `projectLinks/*`,
`imports/*`, `review/*`, `vibeCode/projects/*` except the teleport picker, `skills/*` (the TUI has no
skills screen), `agents/install|uninstall` (it has `/leanstall`), `client tools` (`filesystem/*`,
`terminal` — the TUI does not advertise them).

---

## 1. Full list of TUI user-facing features

### 1.1 Chat screen

| Feature | Code (file:line) | Wire | What to do in the GUI |
|---|---|---|---|
| Sending a prompt (queuing) | `message_queue/requests.rs:16,210` | `session/turn/enqueue` (`TurnEnqueueParams` with `entries`, idempotency key) | text field + send button that enqueues |
| Immediate start (bypassing the queue) | `server/types.rs:36` (`TURN_START`), `message_queue/` | `turn/start` | no such GUI path — use enqueue |
| Streaming response (incremental) | `event_handler.rs:659,697` | `history/entryAdded` / `history/entryUpdated` (JSON Patch, `generationStatus:"in_progress"`) | render Markdown with replace-in-place on patches |
| Thinking / reasoning | `server/types.rs:594` (`ReasoningEntry`), `event_handler.rs:659` | the `reasoning` history type (text, streamed via entryUpdated) | a collapsible "thinking" block |
| Tool calls (tool effects) | `server/effect.rs`, `server/effect_output.rs`, `ui/transcript/entry/effect/` | the `effect` history type (EffectEntry; input/output/detail per tool) | a tool-call card: name, input, result, status |
| Approving tool calls (approval) | `ui/approval/`, `server/callback.rs:25` | server request `callback/call` (kind `approval`) → client response `callback/result` | an approval modal (yes/no/always) |
| Questions to the user (user_input) | `ui/question_layout.rs`, `question_app/`, `server/proto_questions.rs:27` | `callback/call` (kind `user_input`, payload `UserQuestionRequest`) → `callback/result` | a form/choice question |
| Prompt queue (edit/remove) | `message_queue/edit.rs`, `replacement.rs:123`, `requests.rs:170` | `session/turn/queue/edit/start|save|cancel|abandon`, `session/turn/queue/replace`, `…/remove` | a list of pending prompts with edit/remove actions |
| Steer (injection into the active turn) | `message_queue/steering.rs:75` | `session/turn/queue/steer` (`expectedTurnId`), alternatively `turn/steer` | a "send now while responding" button |
| Resuming the queue after a server pause | `message_queue/requests.rs:111` | `session/turn/queue/resume` | automatic; no UI |
| Interrupting a turn (Esc) | `server/types.rs:62` (`TURN_INTERRUPT`) | `turn/interrupt` (`expectedTurnId`) | a stop button |
| Retry after an error | `event_handler.rs:958` (`turn/retrying`), `event_handler.rs:1040-1070` | the `turn/retrying` notification + `session/snapshot.retrying` (`PublicRetryState`); a turn error with `turn.error.code == "rate_limit"` | a "retrying" banner + the error message with `/retry` |
| Manual compaction | `commands/compact.rs:55` | `session/compact` → the `session/compacted` notification | the `/compact [instructions]` button/command |
| Clearing the context | `server/types.rs:52` | `session/history/clear` | `/clear`, `/new` |
| Custom `!` shell commands | `server/types.rs:64-70` | `session/shellCommand` (`action: run|interrupt`) | an inline terminal |
| Background processes | `server/proto_approval.rs`, `app_server/protocol.py` (`session/backgroundProcess/output/stop`) | `session/backgroundProcess/output`, `…/stop` | a view of a long-running process's output |
| Model / thinking level change | `config_write.rs:69` | `config/model/write` (the `sessionOnly` flag) | a model picker in the footer (click) and `/model`, `/thinking` |
| Agent change | `server/types.rs:55` | `session/agent/update` | `/agent`, `/leanstall` (`agents/install`) |
| Skill invocation from slash-mode | `skill.rs`, `input_modes.rs:75` | a prompt with `injectInvokedSkill` / `session/context/inject` (`ContextInjectParams`) | `@`/`/` autocomplete for skills |
| Images / pasting | `server/images.rs`, `paste_image/`, `server/types.rs` (`ContentBlock::Image`) | `ContentBlock {type:"image", uri, mediaType}` in entries | pasting an image into the composer |
| `@file` mentions | `paste_path/`, `mention_stats` in `turn/start` | `MentionStats` in params | a context-statistics tooltip |
| Voice (narration, the `voice` feature) | `voice/`, `voice_app.rs:131`, `turn_summary.rs:248` | `narration/summarize`, `telemetry/record` | a microphone button + playing back the summary |
| Retry/resend generation | `commands/simple.rs` (`/retry`) | no separate method — a new `turn/start` | a retry button next to an interrupted response |

### 1.2 Sessions

| Feature | Code | Wire | What to do in the GUI |
|---|---|---|---|
| List of saved sessions + preview | `resume_picker/request.rs:30` | `session/list` (`{cwd}`) | a "Resume" screen with a list and preview |
| Resuming a session | `resume_picker/request.rs:51`, `startup/attach.rs:99` | `session/resume` (`sessionId` + `agentConfig`) | a click on the session |
| `--continue` (the last session) | `startup/attach.rs:172`, `headless/open.rs:40` | `session/list` → `continueSessionId`, `session/continue` | a "Continue" button |
| New session | `startup/` | `session/start` (+ `firstTurn`, `title`, `model`) | a "New chat" button |
| Deleting a session | `resume_picker/request.rs:64` | `session/delete` | a trash icon in the list |
| Renaming | `commands/simple.rs:150` | `session/rename` | `/rename <title>`; the server also assigns titles (`autoTitle` in `turn/start`) |
| **Pin / archive / unseen** | — the TUI **does not use it** (no calls in cli-rust) | `session/pin`, `session/archive`, `session/markAsSeen`, `SessionListParams.pinned/includeArchived` | for the Desktop GUI: pin/archive as toggles + an unread counter; the TUI does not show this |
| Lease (a session lock between processes) | `resume_picker/mod.rs:103-138` | `session/lease/subscribe` + the `session/lease/updated|ended` notifications | a "session is open elsewhere" marker |
| Session worktree in the preview | `server/types.rs:630` (`PublicSessionWorktree`) | the `worktree` field in `PublicSession` | a worktree-name badge on the session |
| Run export (headless) | `run_export/`, the `--output-dir` flag (`cli.rs:110`) | no wire — local files | (desktop: not applicable) |

### 1.3 Compaction / context

- `/compact` → `session/compact` (`commands/compact.rs:55`), the result via `session/compacted`
  (a handoff with a new session id — `SessionHandoffParams`, `app_server/protocol.py:2538`).
- Auto-compaction on the server side; the TUI receives the same notification.
- The context counter in the footer: `ui/bottom_bar.rs:52,277` — `"{current}/{max} tokens ({pct}%)"`,
  data from the `session/statsUpdated` notification (`StatsUpdatedParams.stats` + `contextWindow`).
- GUI: a context-usage bar + a compaction action at a threshold.

### 1.4 Statistics / usage

- `/status` → `commands/simple.rs:71` renders **locally** from the `session/statsUpdated` state
  (`AgentStatsSnapshot`): steps, session prompt/completion/total tokens, cached tokens, last turn, **cost USD**.
- Footer: the context token counter (`ui/bottom_bar.rs:277`).
- No per-model / per-day usage screen — this data does not exist on the wire (see §2).

### 1.5 Subagents

- Interactively: the TUI **only displays** child sessions from `PublicSessionState.childSessions`
  (`server/types.rs:447`) and the `session/childSessionUpdated` notification (`event_handler.rs:894`);
  UI: `ui/subagent_list.rs:35` (a docked list, opening a child's view on click).
- The `session/subagents/read|wait|interrupt` methods (experimental, `app_server/protocol.py:279`)
  are used **only in headless** (`headless/subagents.rs:36,71,92,107`).
- The server may ask the client to spawn a new session: the `session/spawn` request
  (`app_server/protocol.py:2276+`, `SESSION_SPAWN_METHOD`) — the TUI **does not advertise it**
  (`capabilities` without `sessionSpawn`); Desktop does.
- GUI: a subagent list with status + opening their transcripts.

### 1.6 Checkpoints / rewind / fork

- `/rewind` + double Esc: `rewind/mod.rs:22,58-112` → reads `session/rewind/read`
  (`rewind/request.rs:31` — `hasFileChanges`, `paths`) → `session/rewind`
  (`rewind/request.rs:68`, params `entryId`, `restoreFiles`, `inplace`) → result: the new state + the restored prompt in the composer.
- Checkpoint entries in history: `server/types.rs:611` (`CheckpointEntry`), server: `models.py:1094`.
- `/branch` (fork): `commands/branch.rs:22` → `session/fork` (`SessionForkParams`: `entryId`/`afterTurnId`, `attach`).
- GUI: a click on a history entry → "rewind here / restore files" + "create a branch".

### 1.7 Worktrees

- Starting in a worktree: the `--worktree [NAME]` flag (`cli.rs:143`) → `AgentConfig.worktree`
  (`WorktreeInput {kind: create|auto|existing}`, `app_server/protocol.py:300`) in `session/start`.
- Cleanup on exit: `worktree_exit.rs:125` → `workspace/git/worktrees/remove`
  (with `force`/`inspect`/`deleteBranch`, the response `WorktreeRemoveResponse.outcome`).
- Full worktree administration (`workspace/git/worktrees/list|prune|limit/update`) — the TUI has no screen; for a Desktop GUI to manage.
- GUI: a "new worktree / existing" choice at startup + a removal dialog with an `outcome`/`holders` summary.

### 1.8 Loops (scheduled prompts)

- `/loop [schedule] [prompt]` **does not call `loops/*` from the TUI** — it is classified as a prompt
  (`input_modes.rs:75`, `commands/submission.rs:259`) and the model schedules it with the `cron` tool
  on the server side (`app_server/_cron.py:3`, `loops/create` inside the runtime).
- Loop firings are visible as a notice: `ScheduledLoopFiredNoticeDetail` (`models.py:1079`,
  parsed in `server/types.rs:813`) → an entry in the transcript.
- GUI: it can call `loops/list|create|delete|clear` directly (`ScheduledLoop`: `id`, `prompt`,
  `intervalSeconds`|`cron`, `nextFireAt` — `models.py:1357`).

### 1.9 Skills / agents / MCP / configuration

| Feature | Code | Wire |
|---|---|---|
| Runtime snapshot (model, agent, skills, tools, stats, context window, issues, connectors, mcp) | `startup/reads.rs`, `event_handler.rs` | `runtime/read` + the `runtime/updated` notification |
| Listing/editing the config | `config.rs:34`, `config_command.rs:61`, `config_write.rs:49` | `config/fields/read`, `config/schema`, `config/write` (batch ops `set`/`remove`), `config/reload`, `config/effective/read` |
| MCP + connectors UI | `mcp/mod.rs:113,133`, `mcp/commands.rs:42,96,153`, `mcp/toggle.rs:86` | `mcp_catalog/read`, `connector_catalog/read`, `mcp_catalog/toggle|add|remove|login|logout|refresh`, `connector_catalog/toggle|connect|refresh` |
| OAuth MCP / connector | `mcp_oauth.rs`, `connector_auth/mod.rs:105,131` | the `mcp_catalog/authUrl` notification (a URL to open), then `connectors/auth/read` + `connectors/refresh` |
| Plugins | `ui/plugins.rs`, `plugins/request.rs:73` | `plugin_catalog/read`, `plugin/reload` (`/plugins`, `/reload-plugins`) |
| Agent installation `leanstall` | `commands/commands.rs:74-75` | `agents/install`, `agents/uninstall` |
| Proxy/SSL setup | `proxy_setup.rs:154,208` | `config/proxy/read`, `config/proxy/write` |
| Theme | `theme/`, saved via `config/write` (`/theme`) | `config/write` path `/theme` |
| Feedback nudge | `feedback.rs:65,164,176` | `feedback/shouldShow`, `feedback/record`, `telemetry/record` |
| Debug console | `debug_console.rs:43`, `debug_console/paging.rs:110` | `diagnostics/logs/read`, `session/log/read` |
| Trust workspace | `trust_folders.rs`, `startup/trust.rs` | `workspace/trust/status|decision|untrustedConfig` |

### 1.10 Vibe Code Web / teleport

- `/teleport` and `&prompt`: `teleport/mod.rs:41,53` → the project picker
  `vibeCode/projects/open|loadMore|select|create|unlink|cancel|recover` (`vibe_code_project/request.rs:26`)
  → `vibeCode/teleport/start` (`teleport/request.rs:73`) → progress via the
  `vibeCode/teleport/event` notification (`teleport/mod.rs:97`, the `--teleport` flag `cli.rs:151`).
- PR/MR artifacts: `artifacts.rs:24` → `artifacts/list`, the `ui/artifacts_modal.rs` modal;
  the `artifacts/created|updated` notifications (enriched `prStatus`, `checks`, `mergeStatus` fields —
  `app_server/protocol.py:1470+`).

### 1.11 CLI update

- Checking during a session: `event_handler.rs:132` → `get_update_if_available`
  (`update_notifier/update.rs:134`) — source: the **PyPI simple index** (`update_notifier/gateway.rs:13`
  `https://pypi.org`) + the brew/uv oracles; caching and dismissal in a cache file (`update_notifier/cache.rs`).
- The install prompt: `update_prompt/mod.rs:124-148`; execution: `uv tool upgrade mistral-vibe`
  / `brew upgrade mistral-vibe` (`update_notifier/update.rs:18`); `vibe update`/`--check-upgrade`
  (`main.rs:139`, `update_flow.rs`).
- "What's new" after the update: `startup/banners.rs:90-99`.
- GUI: an "update available" badge + an update dialog; the "what's new" content is not provided by the server (it is client cache).

---

## 2. Mistral Vibe Pro / subscription — state on the wire ⚠️

**Conclusion: the protocol carries no amounts, limits, or "remaining" values.** Greps for
`quota|remaining|subscription|credits|billing|rate_limit` in `cli-rust/src` find no
structure with counters. The only account check is `account/read`:

- request: `{sessionId}` → the `AccountView` response (`app_server/models.py:119-126`):

```json
{
  "account": {
    "status": "ready",                    // ready | missing_key | unauthorized | unavailable (models.py:83)
    "plan": { "kind": "chat", "name": "Pro", "title": "[Subscription] Pro" },   // AccountPlanView models.py:101
    "planOffer": { "kind": "switch_api_key", "url": "…/code/extensions?focus=key" },
    "rateLimitAction": { "kind": "upgrade_to_pro", "url": "https://admin.mistral.ai/subscriptions/upgrade/plans?…" },
    "teleportEligible": true,
    "apiKey": { "preview": "abc…****", "scope": "vibe", "accessScope": "personal_and_shared" }
  }
}
```

- The TUI reads **only** `plan.title` from this (`cli-rust/src/post_ready.rs:37-43`) —
  an addition to the model line in the banner (`ui/banner/mod.rs:23,88`), `planOffer.url`
  (`startup/banners.rs:120-131` — a CTA in "what's new"), `rateLimitAction`
  (`event_handler.rs:1061-1070` — it changes the rate-limit error text), and `apiKey.preview/scope/accessScope`
  and `identity/read` for `/whoami` (`commands/simple.rs:232-280`).
- Types: `AccountStatus` (`models.py:83`), `AccountPlanKind` = `api|chat|mistral_code` (`models.py:90`),
  `AccountActionKind` = `switch_api_key|upgrade_to_pro` (`models.py:96`), `AccountApiKeyView` (`models.py:112`).
- Where the server gets the data: `GET {console_base_url}/api/vibe/whoami` (`app_server/../setup/auth/whoami.py:30`
  `_WHOAMI_PATH`, `HttpAccountGateway.read:82`), the response mapped to `WhoAmIResult`
  (`plan_type`, `plan_name`, `prompt_switching_to_pro_plan`, `organization_kind`, `api_base`, `vibe_base`,
  `primitive_access_scope`) + a 6 h cache per key (`whoami.py:117`). Checks: `app_server/_account.py:166-181`
  (`offers_upgrade`, `rate_limit_upgrade_available`), the upgrade URL: `_UPGRADE_PLANS_URL` (`_account.py:62`).
- Rate limit in practice: the turn ends with the `turn.error.code == "rate_limit"` error
  (`event_handler.rs:1043`), the TUI shows the text from `rate_limit_message` (`event_handler.rs:1061`).
- **Missing**: remaining requests/credits, reset windows, per-org limits, Pro consumption. A GUI that wants
  to show "how much is left of the Pro plan" must either wait for an `account/read` extension (adding
  fields to `AccountView` — the shortest path, the server caches whoami anyway), or read the
  `/api/vibe/whoami` endpoint itself (the `plan_name`/`prompt_switching_to_pro_plan` fields are the only signal).
- A non-null `rateLimitAction` field = the server decided an upgrade makes sense (an API plan or Mistral Code F) —
  it can be read as "YOU are hitting the API plan limit"; it does not apply to the CHAT Pro plan.

---

## 3. Logging into Mistral Vibe — the flow step by step

### 3.0 Who does what

A surprise: **the browser login is done by the Rust TUI itself** (the `cli-rust/src/setup/auth/` module), not the
app-server. The app-server participates via `setup/*` (seed + persistence), and its Python copy of the flow
(`setup/auth/browser_sign_in.py`) serves ACP/Desktop. The wire sequence:

1. **`setup/status`** (`setup/auth/rpc.rs:24-70`, server: `app_server/_setup.py`):
   param `{provider?}`; the response (camelCase):

```json
{
  "provider": { "name": "mistral", "apiBase": "https://api.mistral.ai/v1", "apiKeyEnvVar": "MISTRAL_API_KEY",
                "browserAuthBaseUrl": "https://console.mistral.ai", "browserAuthApiBaseUrl": "https://api.mistral.ai",
                "browserAuthAllowOriginRewrite": false },
  "consoleBaseUrl": "https://console.mistral.ai",
  "vibeBaseUrl": "https://code.mistral.ai",
  "activeModel": "…", "theme": "…",
  "supportsBrowserSignIn": true,
  "hasApiKey": false,
  "enableSystemTrustStore": false,
  "pinnedConsoleUrl": null
}
```

   `supportsBrowserSignIn` and `hasApiKey` decide which screens the wizard shows; the `browserAuth*`
   fields are the endpoints for the browser flow; `pinnedConsoleUrl` (`VIBE_CUSTOM_CONSOLE_URL`) locks the login domain.

2. **The browser flow (PKCE, no localhost callback)** — `setup/auth/sign_in_flow.rs:30`:
   - generation of `code_verifier` (64 B random) + `code_challenge` = BASE64URL(SHA256(verifier)) (`sign_in_flow.rs:137-150`);
   - `POST {browserAuthApiBaseUrl}/vibe/sign-in` with `{"code_challenge":…, "code_challenge_method":"S256"}` →
     `{process_id, sign_in_url, poll_url, expires_at}` (`setup/auth/sign_in_gateway.rs`; the Python equivalent:
     `setup/auth/http_browser_sign_in_gateway.py:60-95`);
   - opening `sign_in_url` in the browser (`external_url::open`, `sign_in_flow.rs:47`; by default
     `https://console.mistral.ai/codestral/cli/authenticate?process_id=…`);
   - **polling `poll_url` every 3 s** until `expires_at` (`sign_in_flow.rs:95-137`): statuses
     `pending | completed (returns exchange_token) | expired | denied | error`; after 3 consecutive network errors — failure;
   - **exchange**: `gateway.exchange(process_id, exchange_token, code_verifier)` → `{api_key}`
     (`http_browser_sign_in_gateway.py:150+`; handling: 403 = workspace not eligible, 410 = expired, code 9004 = no access);
   - no localhost listening — this is polling, not an OAuth redirect.
3. **`setup/store-credential`** (`rpc.rs:73-100`, server `SetupStoreCredentialParams`): `{provider, apiKey, customDomain}`
   → the response `{outcome: "completed"|"env_var_error"|"save_error", detail?}`; the server persists into the
   **keyring**, falls back to `$VIBE_HOME/.env`, and bumps its own process env (`_setup.py`; `api_key_persistence.py`).
4. **`setup/submit-choices`**: `{provider?, consoleBaseUrl?, vibeBaseUrl?, theme?}` (absence = "leave as is") →
   `{outcome: "completed"|"provider_config_error", failures[]}` — it closes out the onboarding (config + theme saved).
5. After the session restarts, the account state: `identity/read` + `account/read` (`post_ready.rs:20-45`).

Via an **API key**: the ApiKey screen → paste → `SubmitApiKey` (`setup/wizard/mod.rs:165-175`) →
`setup/store-credential` + `setup/submit-choices` (`setup/wizard/actions.rs:124,167`).

Additional auth paths in the TUI (not login): connector OAuth via the `mcp_catalog/authUrl` notification
(`mcp_oauth.rs`, `server/types.rs:139`) and `connectors/auth/read` (`connector_auth/mod.rs:105`).

---

## 4. The login screens in the TUI (what you see without a key)

The wizard screens (`cli-rust/src/setup/wizard/mod.rs:29-37`):

1. `Welcome` — animation + typing (gate: waits for the boot `setup/status` before Enter, `setup/wizard/flow.rs:69-81`),
2. `ThemeSelection`,
3. `AuthMethod` — a choice: **browser vs API key** (shown per `supportsBrowserSignIn`),
4. `SignInTarget` / `CustomDomain` — the default console vs a custom domain (URL validation: `setup/wizard/context.rs:91-100`),
5. `BrowserSignIn` — a stepper: Open browser → Waiting → Exchanging → Complete (`ui/transcript`-like; paint: `setup/wizard/paint/browser_sign_in.rs`; the URL to copy when there is no browser — the `CopySignInUrl` action),
6. `ApiKey` — key input with validation (`ValidationState`).

The wizard runs **before the session** (a separate event loop, `setup/wizard/flow.rs:31`); it is entered via
`--setup`, a missing key at startup, or after `session/start` with an auth error (`startup/mod.rs:346-364`).
The TUI in chat mode has no separate "login screen" — not being logged in = the wizard at startup.

---

## 5. Versioning / compatibility / feature detection

- Handshake: `initialize` with `clientInfo{name,version,entrypoint,terminalEmulator}` + `capabilities{callbackKinds, clientTools, disabledNotifications}` (`server/types.rs:196-236`); the response `serverInfo{name,version}` — the server version shown in the banner (`startup/mod.rs:162`, `ui/banner/mod.rs:56`).
- **Feature detection via errors**: `method_not_found`/-32601 = an older server (`server/types.rs:692` `is_method_not_found`); used e.g. in the wizard (`setup/auth/rpc.rs:183,193`), `config_write.rs:71`, `plugins/request.rs:73`. Additional codes: `unauthorized`, `conflict`+`harnessCode:"session_busy"` (`types.rs:668-680`).
- **Experimental methods**: `session/subagents/*` — they may disappear from the `initialize` list; the client must tolerate their absence (`app_server/protocol.py:279-284`).
- Strict mode of versioning: `config/write` ops with `extra="forbid"` — an unknown field = `invalid_params` and nothing saved (ADR 0014, `protocol.py:1806+`); OPTIONAL extensions are sent as "absent on older servers" (`WorktreeRemoveParams.force/inspect/deleteBranch`, `types.rs:726-740`; `apiKey.accessScope` — "None on older backends", `models.py:115`).
- Polyfill: `bin/replay.rs` maps old→new methods when replaying recordings.
- The transport flag: multiplexing many sessions in one server process (`server/multiplex.rs`, `runs_per_app_server`).

---

## 6. Registry: methods called by the TUI vs unused

**Called (the constants in `server/types.rs:10-121`)**: initialize/initialized, session/start|list|read|resume|continue|delete|stop|rename|rewind|rewind/read|fork|compact|history/clear|ready/wait|lease/subscribe|shellCommand, session/turn/enqueue|queue/*|turn/start|steer|interrupt, session/agent/update, runtime/read, config/read|schema|fields/read|effective/read|write|model/write|reload|proxy/read|proxy/write, setup/status|store-credential|submit-choices, workspace/prompt/prepare, workspace/trust/*, workspace/git/worktrees/remove, mcp_catalog/read|refresh|toggle|add|remove|login|logout, connector_catalog/read|refresh|toggle|connect, connectors/auth/read|refresh, plugin_catalog/read, plugin/reload, agents/install|uninstall, callback/result, feedback/*, telemetry/record, narration/summarize, diagnostics/logs/read, session/log/read, identity/read, account/read, providerAuth/read, artifacts/list.

**Not called by the TUI (available to Desktop/GUI)**: session/pin, session/archive, session/markAsSeen, session/title/update, session/relocate, session/settings/update, session/turns/list, session/history/get|list, session/subagents/* (experimental), session/spawn (request), loops/*, projectLinks/*, imports/*, review/*, vibeCode/projects/* (except the teleport picker), skills/*, artifacts/markViewed|sync, client tools filesystem/terminal.
