# vibe-app-server — protocol reference

> Document built from the source code of `mistralai/mistral-vibe` (branch `main`,
> inspected 2026-10-26, versions ~3.x). Knowledge base for the `vibe-app-server`
> client in the GUI. Verified in: `vibe/app_server/protocol.py`, `events.py`, `models.py`,
> `client.py`, `client_state.py`, `server.py`, `stdio.py`, `transport.py`,
> `_connection_protocol.py`, `_unified_harness_backend_adapter.py`,
> `vibe/cli-rust/src/server/*`.

## 1. What vibe-app-server is

`vibe-app-server` is Mistral Vibe's **central harness and runtime boundary**. All
"surfaces" (Rust TUI, official Desktop, ACP, programmatic mode) are **clients** of
this server. A client never talks to the `AgentLoop`/Unified Harness directly —
it only exchanges serialized JSON.

The three binary entry points of the `mistral-vibe` package:

| Entry point | Function | Role |
|---|---|---|
| `vibe` | `vibe.cli.entrypoint:main` | CLI — since 3.x always starts the Rust TUI |
| `vibe-acp` | `vibe.acp.entrypoint:main` | ACP bridge for IDEs (Zed/VS Code/JetBrains) |
| `vibe-app-server` | `vibe.app_server.stdio:main` | **JSON-RPC 2.0 server — this document** |

The Rust TUI (the reference client) spawns the `vibe-app-server` process
(`vibe/cli-rust/src/server/process.rs`) and optionally uses `--multiplex` to pool
multiple sessions into a single process (`multiplex.rs`, the `runs_per_app_server` option).

## 2. Transport and framing

- **JSON-RPC 2.0 over stdio, one line = one message (NDJSON)** — identical framing
  to `vibe-acp` (`transport.py`: `json.dumps(message, separators=(",", ":")) + b"\n"`).
- **No Content-Length header** (this is not LSP).
- Envelopes (`protocol.py:2606+`):
  - client request: `{"jsonrpc":"2.0","id":<string>,"method":...,"params":{}}` —
    **IDs must be strings** (the server rejects ints; the reference client generates
    `"client-1"`, `"client-2"`, …)
  - notification: `{"jsonrpc":"2.0","method":...,"params":{}}`
  - response: `{"jsonrpc":"2.0","id":...,"result":{}}` or
    `{"jsonrpc":"2.0","id":...,"error":{code,message,data}}`
  - **server→client request**: `{"jsonrpc":"2.0","id":<int>,"method":...,"params":{}}`
    (e.g. `callback/call`) — the client MUST be able to respond to server requests.
- The server writes logs to a file (not to stdout) — stdout is pure protocol.

## 3. Handshake

**1) `initialize`** (client request, `client.py:82`):

```json
{
  "id": "client-1", "method": "initialize",
  "params": {
    "clientInfo": {"name": "VibeDeck", "version": "1.0.0", "entrypoint": "unknown", "terminalEmulator": "unknown"},
    "capabilities": {
      "callbackKinds": ["approval", "user_input"],
      "clientTools": [],
      "sessionSpawn": false
    }
  }
}
```

- `ClientInfo`: `name`, `version`, `title?`, `entrypoint`, `terminal_emulator`
  (`_connection_protocol.py:11`).
- `ClientCapabilities` (`_connection_protocol.py:32`):
  - `callback_kinds`: subset of `["approval","user_input","connector_auth"]` —
    **the server will throw a RuntimeError if it has to send a callback kind the
    client did not declare** (`server.py:1614`)
  - `client_tools`: subset of `["filesystem/read","filesystem/write","terminal"]`
  - `session_spawn`: bool — whether the client can handle the server-request `session/spawn`
  - `disabled_notifications`: list of notifications the client does not want to receive
  - the `extra="ignore"` field — unknown capabilities are ignored
- Response: `InitializeResponse {serverInfo: {name, version}, processId}` — the server
  identifies itself as `vibe-app-server` with the package version.

## 4. Server methods (full registry from `protocol.py:108+`, SERVER_METHODS)

**Session:** `session/start`, `session/resume`, `session/continue` (resume the last
qualifying session), `session/list`, `session/read`, `session/delete`, `session/fork`,
`session/rename`, `session/title/update`, `session/pin`, `session/archive`,
`session/relocate`, `session/stop`, `session/ready/read`, `session/ready/wait`,
`session/lease/subscribe`, `session/markAsSeen`, `session/log/read`,
`session/history/list`, `session/history/get`, `session/history/clear`,
`session/turns/list`, `session/rewind`, `session/rewind/read`, `session/compact`,
`session/context/inject`, `session/agent/update`, `session/settings/update`,
`session/shellCommand`, `session/backgroundProcess/output`,
`session/backgroundProcess/stop`, `session/subagents/read|interrupt|wait`,
`session/spawn` (a server request!)

**Turn/queue:** `session/turn/enqueue`, `turn/start`, `turn/steer`, `turn/interrupt`,
`session/turn/queue/read|replace|remove|steer|resume`,
`session/turn/queue/edit/start|save|cancel|abandon`

**Events/recovery:** `events/read`, `callback/result`

**Configuration/runtime:** `config/read`, `config/write`, `config/reload`,
`config/schema`, `config/fields/read`, `config/effective/read`, `config/model/write`,
`config/proxy/read|write`, `runtime/read`, `agents/list`, `agents/install|uninstall`,
`skills/*`, `mcp/*`, `mcp_catalog/*`, `connectors/*`, `connector_catalog/*`,
`plugins/read`, `plugin/*`, `plugin_catalog/read`

**Workspace:** `workspace/trust/status|decision|untrustedConfig`,
`workspace/prompt/prepare`, `workspace/git/*` (worktrees), `projectLinks/*`,
`vibeCode/projects/*`, `vibeCode/teleport/*`

**Misc:** `identity/read`, `account/read`, `feedback/record|shouldShow`, `imports/*`,
`loops/*`, `narration/summarize`, `review/*`, `shell/run|interrupt`,
`setup/status|store-credential|submit-choices`, `telemetry/record`,
`diagnostics/logs/read`

**Historically important (removed!):** `tools/list`, `stats/read`, `diagnostics/list`
were removed in 3.x — do not implement them. Some methods are **experimental**
(`EXPERIMENTAL_SERVER_METHODS`, including `session/subagents/*`) — they may change.

## 5. Session lifecycle

### session/start (`protocol.py:440`)

```json
{
  "method": "session/start",
  "params": {
    "agentConfig": {
      "cwd": "/path/to/project",
      "agent": null,
      "mcpServers": [],
      "disabledTools": [],
      "trustWorkspace": false,
      "historyLimit": 200
    },
    "kind": "NORMAL",
    "model": null,
    "reasoningEffort": null,
    "sessionOnly": false,
    "title": null,
    "firstTurn": null,
    "idempotencyKey": null
  }
}
```

- `agentConfig` is `AgentConfig` (`protocol.py:374`): `cwd`/`workdir` (**an empty
  string "" is rejected by the validator!** — if you want the default, omit the field),
  `agent` (profile name), `model` via `completion`, `mcp_servers` (http/stdio,
  discriminated by the `transport` field), `enabled_tools`/`disabled_tools`,
  `auto_approve`, `max_turns`, `max_price`, `max_session_tokens`, `headless`,
  `trust_workspace`, `worktree`.
- `kind`: **`"NORMAL"`** — a real user session; **`"EPHEMERAL"`** — a warm-up
  session (abandoned on resume, no telemetry) — the native equivalent of "priming".
- `model` + `reasoning_effort` + `session_only` — set the model/thinking level
  **per session without changing the user's default**.
- `first_turn` — you can pass the first turn right at start.
- Response: `SessionStartResponse = {state: PublicSessionState, last_event_id}`.

### session/resume / session/read

- `session/resume {session_id, agent_config?, history_limit}` → `PublicSessionState` +
  `last_event_id` — **the full state snapshot is in the response, no replay before
  the response**.
- `session/read {session_id, history?: PageRequest, turns?: PageRequest}` — paginated
  history; `PageRequest {cursor?, limit(1-500, default 200), direction: forward|backward}`.
- `session/list` → list of sessions; sessions carry `cwd`, `title`, `updated_at`,
  `pinned_at`, `archived_at`, `is_unseen`, `harness: "unified"|"legacy"`.
- PublicSessionStatus: `idle | running | blocked | failed | archived` (discriminated
  by the `type` field).

Closing: `session/stop` — ends the active turn; `session/delete` removes the session.

## 6. Turn: sending a prompt

**`session/turn/enqueue`** (`protocol.py:2287`) — the basic way to send:

```json
{
  "method": "session/turn/enqueue",
  "params": {
    "sessionId": "…",
    "idempotencyKey": null,
    "entries": [
      {"role": "user", "content": [{"type": "text", "text": "fix the bug in auth.py"}]}
    ]
  }
}
```

- `entries`: a list of `TurnInputEntry` — **exactly one entry with `role:"user"`,
  and it MUST be last** (the `validate_turn_input_entries` validator); before it there
  can be entries with `role:"context"` (`TurnContextInputEntry`).
- Content blocks: `{"type":"text","text":...}`, `{"type":"image","attachment":{...}}`,
  `{"type":"resource","resource":…}` (`models.py:200-215`).
- Response: `TurnEnqueueResponse {queue_item_id}` — the prompt lands in the **turn
  queue** (`PublicTurnQueue`).
- The turn's outcome arrives via **notifications**: `turn/started` →
  `history/entryAdded|entryUpdated` → `turn/completed`.

**In-flight control:**
- `turn/steer {session_id, expected_turn_id, message}` — inject into an in-progress
  turn (error `NOT_STEERABLE` if the turn does not allow it).
- `turn/interrupt {session_id, expected_turn_id}` — interrupt the turn.
- `session/turn/queue/steer|replace|remove|resume|read` + the queue editor:
  `queue/edit/start → save|cancel|abandon`.
- `session/compact {session_id, extra_instructions}` — manual compaction.
- `session/context/inject` — context injection without a turn.
- **Idempotency**: `idempotency_key` on enqueue/start/steer — deduplication of retries
  (crucial on reconnect!).

## 7. Callbacks: approvals and questions (a server→client request!)

Instead of `session/request_permission`/`session/request_input` requests, the server
sends a **JSON-RPC request to the client** using the `callback/call` method
(`server.py:1609`):

```
S->>K: request "callback/call" {callback: PublicCallbackEntry}
K-->>S: response {callbackId, accepted: true}
K->>S: request "callback/result" {sessionId, result: {callbackId, output}}
S-->>K: response {accepted: true, lastEventId}
```

- `PublicCallbackEntry` (`models.py:1030`): `{id, type:"callback", callback_id, title,
  detail, state}`.
- `detail` — discriminated by the `kind` field:
  - **`approval`** (`models.py:324`): `{effect: EffectDetail, required_permissions,
    choices: [approve|approve_for_session|approve_permanently|deny|cancel_turn],
    path_scope_choices, reason?}`; output:
    `{"type":"approval","decision":{"type":"approve","pathScope":null},"feedback":null}`.
  - **`user_input`**: `{request: UserQuestionRequest, related_entry_id?}`; output:
    `{"type":"user_input","result": {...UserQuestionResult}}`.
  - `connector_auth` — connector OAuth (declare only once implemented).
- `callback/result` (`protocol.py:2513`): `{session_id, result: CallbackResult
  {callback_id, output, error?}}`; errors: `CALLBACK_CLOSED`.
- The client responds **twice**: an immediate ack on the `callback/call` request AND
  a separate `callback/result` request carrying the decision. The callback entry also
  appears in the session history and in `state.active_callbacks`.
- **The server never sends a callback kind absent from the client's `callbackKinds`.**

## 8. Server notifications

Every notification has base params `EventNotificationParams` (`protocol.py:2546`):
**`{event_id: int, session_id, emitted_at: int}`** — `event_id` is the global,
monotonically increasing cursor of state!

| Method | Params | Meaning |
|---|---|---|
| `session/snapshot` | `{state: PublicSessionState}` | full state dump (startup, attach) |
| `session/updated` | `{patch: [JsonPatchOperation]}` | JSON-Patch on PublicSession |
| `history/entryAdded` | `{turn_id?, entry: PublicHistoryEntry}` | new history entry |
| `history/entryUpdated` | `{turn_id?, entry_id, patch}` | JSON-Patch on the entry — content streaming is done via patches! |
| `turn/started` | `{turn: PublicTurn}` | turn start |
| `turn/completed` | `{turn: PublicTurn}` | turn end (`status`, `error`, `stop_reason`) |
| `turn/retrying` | `{params}` | retry after a stream failure |
| `turn/queueUpdated` | `{queue: PublicTurnQueue}` | prompt queue changed |
| `session/statsUpdated` | `{stats: AgentStatsSnapshot, context_window}` | native token/cost statistics |
| `session/childSessionUpdated` | `{child_session: PublicChildSession}` | subagent (progress, token_usage) |
| `session/compacted` | `{old_session_id, state, summary_length}` | **native compaction event** — handoff: new session_id! |
| `session/contextCleared` | `{old_session_id, state, session_log}` | context cleared — also a handoff |
| `mcp_catalog/authRequired` | `{params}` | MCP server requires OAuth |
| `connector_catalog/authRequired` | `{params}` | connector requires login |
| `runtime/updated` | `{runtime}` | runtime changed → read `runtime/read` |
| `warning` | `{params}` | ServerWarning (non-critical) |

## 9. State model: PublicSessionState

`PublicSessionState` (`models.py:1330`, format `"vibe.public-session-state/v1"`):

```
{
  event_id: int,
  session: PublicSession,        // id, title, status, cwd, model, reasoning_effort,
                                 // agent, token_usage, context_usage, workspace_roots,
                                 // worktree, loops, is_unseen…
  is_quiescent: bool?,
  history: [PublicHistoryEntry]?,
  history_before_cursor: str?,
  turns: [PublicTurn]?,
  active_callbacks: [PublicCallbackEntry],   // pending approvals/questions!
  child_sessions: [PublicChildSession],
  background_processes: [PublicBackgroundProcess],
  turn_queue: PublicTurnQueue,
  retrying: PublicRetryState?
}
```

- **Content streaming**: an assistant entry is added once (`history/entryAdded`),
  then updated via **JSON-Patch** (`history/entryUpdated`). Patch ops:
  `add|append|replace|remove|test` — **this is not RFC6902: it has an extra `append`!**
- `PublicHistoryEntry` (discriminated by `type`): `user`, `assistant`, `reasoning`,
  `effect` (title, detail: EffectDetail — tool calls, edits, commands; state),
  `callback`, `notice` (hook_run_*, agent_changed, session_title_updated, …).
- Reference reducer: `vibe/app_server/events.py` → `ClientProjection.consume()`
  (`EventSequenceError` on inconsistent event_id/session_id, handoff on
  compacted/contextCleared).
- Reference client state: `vibe/app_server/client_state.py`.

## 10. Consistency and recovery (watermark + events/read)

- **Every mutating response** (`session/start`, `resume`, `steer`, `interrupt`,
  `callback/result`…) carries `last_event_id` (`EventWatermarkResponse`,
  `protocol.py:284`).
- Notifications carry their own `event_id`. A gap = the last seen event_id <
  `last_event_id` → something happened that you missed.
- **`events/read`** (`protocol.py:322`): `{after_event_id, filters: {session_ids, …},
  batch_size}` → `EventBatch` — gap replay. Error `STALE_CURSOR` when the retention
  window has passed → full resync via `session/read`.
- In the reference client, "attach" responses join the same stream as notifications
  (the response boundary — `client.py:_dispatch`; ADR 0009: reduction order matters).
- **Reconnect**: `initialize` → `session/resume` (full snapshot) → applied from scratch.
  The response always authoritatively replaces state.

## 11. Configuration, model, agent mid-session

- **`session/agent/update`** (`AgentSwitchParams`, `protocol.py:1075`):
  `{session_id, agent_name}` — changing the mode/agent mid-session.
- **`config/model/write`** — model change (with `session_only`).
- **`session/settings/update`**: `{session_id, max_turns?, max_tokens?}`.
- **`runtime/read`** → `RuntimeSnapshot` (`protocol.py:1095`): `{config, active_agent,
  agents[], skills[], tools[], stats, context_window, issues[], hooks_count,
  connectors, mcp, bypass_tool_permissions}` — the data source for pickers.
  The `runtime/updated` notification signals changes.
- **`agents/list`** → `{active, agents[]}` — list of agent profiles (modes).
- **`config/read` / `config/write`** — TOML config via ops, `config/model/write`
  to change the model.

## 12. Error codes (ProtocolErrorCode, `protocol.py:2617`)

`INVALID_REQUEST, INVALID_PARAMS, NOT_INITIALIZED, NOT_FOUND, CONFLICT, STALE_TURN,
NOT_STEERABLE, CALLBACK_CLOSED, COMPACTION_FAILED, UNAUTHORIZED, FORBIDDEN,
METHOD_NOT_FOUND, NOT_IMPLEMENTED, STALE_CURSOR, INTERNAL_ERROR`

`INVALID_PARAMS` carries a list of `{path, message}` in `data.issues[]` — render
field paths when debugging.

## 13. Optional client tools (server→client)

If the client declares `client_tools`, the server may **delegate tools to the
client** (requests, `_connection_protocol.py:26`): `clientTool/readTextFile`,
`clientTool/writeTextFile`, `clientTool/terminal/create|wait|output|kill|release`.
For the GUI: do not declare these in v1.

## 14. Source map (for further reading)

| File | Contents |
|---|---|
| `vibe/app_server/protocol.py` (2741 lines) | **the whole tower**: SERVER_METHODS, Params/Response, error enums |
| `vibe/app_server/models.py` | Public* models, content blocks |
| `vibe/app_server/events.py` | ClientProjection — the reference reducer |
| `vibe/app_server/client.py` | the reference JSON-RPC client (sequencing, response boundary) |
| `vibe/app_server/client_state.py` | the reference client state (bootstrap) |
| `vibe/app_server/_connection_protocol.py` | initialize, ClientCapabilities, client tools |
| `vibe/app_server/server.py` | routing, callbacks, pending notifications |
| `vibe/cli-rust/src/server/` | **the reference client in Rust**: process.rs, connect.rs, callback.rs, proto_approval.rs, proto_questions.rs, frames.rs, multiplex.rs, effect.rs, types/ |
| `client-e2e/` | golden harness of client E2E tests |

## 15. Protocol probe

`scripts/probe_appserver.py` — a minimal Python client: spawns `vibe-app-server`,
initializes, session/start, turn/enqueue, listens for notifications, acks callbacks.
Verification order: (1) initialize returns serverInfo; (2) session/start → state +
lastEventId; (3) enqueue → turn/started, history/entryAdded (user), entryAdded+
entryUpdated (assistant), turn/completed; (4) `session/list` shows the session;
(5) reconnect: second process, `session/resume` returns the full snapshot.

## 16. Pitfalls (empirically, from the code)

1. **Client request IDs must be strings** — the server throws
   `JsonRpcProtocolError("Client request responses require string IDs")`.
2. **`cwd: ""` is rejected** by the validator — omit the field instead of sending empty.
3. **Callback kinds must be declared** in initialize, otherwise delivering a
   callback = a server RuntimeError.
4. **`history/entryUpdated` uses its own JSON-Patch with the `append` op** — do not
   use an off-the-shelf RFC6902 library without extending it with `append`.
5. **Handoff** (`session/compacted`/`contextCleared`) swaps the `session_id`
   (the `old_session_id` field) — remap all mappings keyed by the old ID.
6. **Reduction order matters**: attach responses and notifications are in one
   stream (the response boundary) — do not parse responses out of order.
7. **`event_id` strictness**: ClientProjection throws `EventSequenceError` on a
   regressed cursor — do a full resync instead of cutting corners.
8. **Versioning**: the app-server **is not a stable API** — 3.x removed
   `tools/list`, `stats/read`, `diagnostics/list`. Pin the CLI version in tests.
9. **The handshake is LSP-like**: after the `initialize` response the server is in
   state `INITIALIZE_RECEIVED`, and only the **`initialized`** notification moves it
   to `INITIALIZED` — without it every subsequent request gets `NOT_INITIALIZED`
   (`server.py: _handle_notification`; verified on 3.0.3).
10. **`kind` in `session/start` is lowercase**: `"normal"` / `"ephemeral"`
    (StrEnum) — `"NORMAL"` yields INVALID_PARAMS.
11. **History entries are `type: "message"`** with a `role` field (`user`/`assistant`/
    `system`) — not separate `user`/`assistant` types; `PublicMessageEntry` also has
    `generationStatus` and `createdAt`/`updatedAt` as **int (epoch ms)**.
    Streaming: content arrives in `entryAdded`; `entryUpdated` patches carry
    status/`updatedAt` changes (the `replace` op), not content appends.
