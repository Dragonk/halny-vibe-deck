# vibe-app-server — referencja protokołu

> Dokument zbudowany z kodu źródłowego `mistralai/mistral-vibe` (branch `main`,
> inspekcja 2026-10-26, wersje ~3.x). Baza wiedzy dla klienta `vibe-app-server`
> w GUI. Zweryfikowano w: `vibe/app_server/protocol.py`, `events.py`, `models.py`,
> `client.py`, `client_state.py`, `server.py`, `stdio.py`, `transport.py`,
> `_connection_protocol.py`, `_unified_harness_backend_adapter.py`,
> `vibe/cli-rust/src/server/*`.

## 1. Czym jest vibe-app-server

`vibe-app-server` to **centralny harness i granica runtime** Mistral Vibe. Wszystkie
"surface'e" (Rust TUI, oficjalny Desktop, ACP, tryb programistyczny) są **klientami**
tego serwera. Klient nigdy nie rozmawia z `AgentLoop`/Unified Harness bezpośrednio —
wymienia wyłącznie serializowane JSON-y.

Trzy binarne entry pointy pakietu `mistral-vibe`:

| Entry point | Funkcja | Rola |
|---|---|---|
| `vibe` | `vibe.cli.entrypoint:main` | CLI — od 3.x zawsze startuje Rust TUI |
| `vibe-acp` | `vibe.acp.entrypoint:main` | most ACP dla IDE (Zed/VS Code/JetBrains) |
| `vibe-app-server` | `vibe.app_server.stdio:main` | **serwer JSON-RPC 2.0 — ten dokument** |

Rust TUI (referencyjny klient) spawnuje proces `vibe-app-server`
(`vibe/cli-rust/src/server/process.rs`) i opcjonalnie używa `--multiplex` do poolingu
wielu sesji w jednym procesie (`multiplex.rs`, opcja `runs_per_app_server`).

## 2. Transport i ramka

- **JSON-RPC 2.0 po stdio, jedna linia = jedna wiadomość (NDJSON)** — identyczny framing
  jak `vibe-acp` (`transport.py`: `json.dumps(message, separators=(",", ":")) + b"\n"`).
- **Brak nagłówka Content-Length** (to nie LSP).
- Envelopy (`protocol.py:2606+`):
  - request klienta: `{"jsonrpc":"2.0","id":<string>,"method":...,"params":{}}` —
    **ID muszą być stringami** (serwer odrzuca int-y; referencyjny klient generuje
    `"client-1"`, `"client-2"`, …)
  - notyfikacja: `{"jsonrpc":"2.0","method":...,"params":{}}`
  - odpowiedź: `{"jsonrpc":"2.0","id":...,"result":{}}` lub
    `{"jsonrpc":"2.0","id":...,"error":{code,message,data}}`
  - **request serwera→klienta**: `{"jsonrpc":"2.0","id":<int>,"method":...,"params":{}}`
    (np. `callback/call`) — klient MUSI umieć odpowiadać na requesty serwera.
- Serwer pisze logi do pliku (nie na stdout) — stdout jest czystym protokołem.

## 3. Handshake

**1) `initialize`** (request klienta, `client.py:82`):

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
  - `callback_kinds`: subset `["approval","user_input","connector_auth"]` —
    **serwer wywali RuntimeError, jeśli wyśle callback rodzaju, którego klient
    nie zadeklarował** (`server.py:1614`)
  - `client_tools`: subset `["filesystem/read","filesystem/write","terminal"]`
  - `session_spawn`: bool — czy klient potrafi obsłużyć server-request `session/spawn`
  - `disabled_notifications`: lista notyfikacji, których klient nie chce dostawać
  - pole `extra="ignore"` — nieznane capabilities są ignorowane
- Odpowiedź: `InitializeResponse {serverInfo: {name, version}, processId}` — serwer
  przedstawia się jako `vibe-app-server` z wersją paczki.

## 4. Metody serwera (pełny rejestr z `protocol.py:108+`, SERVER_METHODS)

**Sesja:** `session/start`, `session/resume`, `session/continue` (resume ostatniej
kwalifikującej się), `session/list`, `session/read`, `session/delete`, `session/fork`,
`session/rename`, `session/title/update`, `session/pin`, `session/archive`,
`session/relocate`, `session/stop`, `session/ready/read`, `session/ready/wait`,
`session/lease/subscribe`, `session/markAsSeen`, `session/log/read`,
`session/history/list`, `session/history/get`, `session/history/clear`,
`session/turns/list`, `session/rewind`, `session/rewind/read`, `session/compact`,
`session/context/inject`, `session/agent/update`, `session/settings/update`,
`session/shellCommand`, `session/backgroundProcess/output`,
`session/backgroundProcess/stop`, `session/subagents/read|interrupt|wait`,
`session/spawn` (request serwera!)

**Turn/kolejka:** `session/turn/enqueue`, `turn/start`, `turn/steer`, `turn/interrupt`,
`session/turn/queue/read|replace|remove|steer|resume`,
`session/turn/queue/edit/start|save|cancel|abandon`

**Eventy/recovery:** `events/read`, `callback/result`

**Konfiguracja/runtime:** `config/read`, `config/write`, `config/reload`,
`config/schema`, `config/fields/read`, `config/effective/read`, `config/model/write`,
`config/proxy/read|write`, `runtime/read`, `agents/list`, `agents/install|uninstall`,
`skills/*`, `mcp/*`, `mcp_catalog/*`, `connectors/*`, `connector_catalog/*`,
`plugins/read`, `plugin/*`, `plugin_catalog/read`

**Workspace:** `workspace/trust/status|decision|untrustedConfig`,
`workspace/prompt/prepare`, `workspace/git/*` (worktrees), `projectLinks/*`,
`vibeCode/projects/*`, `vibeCode/teleport/*`

**Różne:** `identity/read`, `account/read`, `feedback/record|shouldShow`, `imports/*`,
`loops/*`, `narration/summarize`, `review/*`, `shell/run|interrupt`,
`setup/status|store-credential|submit-choices`, `telemetry/record`,
`diagnostics/logs/read`

**Ważne historycznie (usunięte!):** `tools/list`, `stats/read`, `diagnostics/list`
zostały usunięte w 3.x — nie implementować. Niektóre metody są **experimental**
(`EXPERIMENTAL_SERVER_METHODS`, m.in. `session/subagents/*`) — mogą się zmieniać.

## 5. Cykl życia sesji

### session/start (`protocol.py:440`)

```json
{
  "method": "session/start",
  "params": {
    "agentConfig": {
      "cwd": "/ścieżka/projektu",
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

- `agentConfig` to `AgentConfig` (`protocol.py:374`): `cwd`/`workdir` (**pusty string ""
  jest odrzucany walidatorem!** — chcesz default, pomiń pole), `agent` (nazwa profilu),
  `model` przez `completion`, `mcp_servers` (http/stdio, dyskryminowane polem
  `transport`), `enabled_tools`/`disabled_tools`, `auto_approve`, `max_turns`,
  `max_price`, `max_session_tokens`, `headless`, `trust_workspace`, `worktree`.
- `kind`: **`"NORMAL"`** — prawdziwa sesja użytkownika; **`"EPHEMERAL"`** — sesja
  rozgrzewkowa (porzucana przy resume, zero telemetrii) — natywny odpowiednik "priming".
- `model` + `reasoning_effort` + `session_only` — ustaw model/poziom myślenia **per sesja
  bez zmiany defaultu użytkownika**.
- `first_turn` — można od razu przekazać pierwszą turę przy starcie.
- Odpowiedź: `SessionStartResponse = {state: PublicSessionState, last_event_id}`.

### session/resume / session/read

- `session/resume {session_id, agent_config?, history_limit}` → `PublicSessionState` +
  `last_event_id` — **pełny stan (snapshot) w odpowiedzi, zero replayu przed odpowiedzią**.
- `session/read {session_id, history?: PageRequest, turns?: PageRequest}` — paginowana
  historia; `PageRequest {cursor?, limit(1-500, default 200), direction: forward|backward}`.
- `session/list` → lista sesji; sesje mają `cwd`, `title`, `updated_at`, `pinned_at`,
  `archived_at`, `is_unseen`, `harness: "unified"|"legacy"`.
- PublicSessionStatus: `idle | running | blocked | failed | archived` (dyskryminowane
  polem `type`).

Zamknięcie: `session/stop` — kończy aktywną turę; `session/delete` usuwa sesję.

## 6. Turn: wysyłka promptu

**`session/turn/enqueue`** (`protocol.py:2287`) — podstawowa wysyłka:

```json
{
  "method": "session/turn/enqueue",
  "params": {
    "sessionId": "…",
    "idempotencyKey": null,
    "entries": [
      {"role": "user", "content": [{"type": "text", "text": "napraw bug w auth.py"}]}
    ]
  }
}
```

- `entries`: lista `TurnInputEntry` — **dokładnie jeden wpis `role:"user"` i MUSI być
  ostatni** (walidator `validate_turn_input_entries`); przed nim mogą być wpisy
  `role:"context"` (`TurnContextInputEntry`).
- Bloki content: `{"type":"text","text":...}`, `{"type":"image","attachment":{...}}`,
  `{"type":"resource","resource":…}` (`models.py:200-215`).
- Odpowiedź: `TurnEnqueueResponse {queue_item_id}` — prompt ląduje w **kolejce tur**
  (`PublicTurnQueue`).
- Odpowiedź na turę przychodzi przez **notyfikacje** `turn/started` →
  `history/entryAdded|entryUpdated` → `turn/completed`.

**Sterowanie w locie:**
- `turn/steer {session_id, expected_turn_id, message}` — wtrącenie do trwającej tury
  (błąd `NOT_STEERABLE` gdy tura nie pozwala).
- `turn/interrupt {session_id, expected_turn_id}` — przerwanie tury.
- `session/turn/queue/steer|replace|remove|resume|read` + edytor kolejki:
  `queue/edit/start → save|cancel|abandon`.
- `session/compact {session_id, extra_instructions}` — ręczna kompakcja.
- `session/context/inject` — wstrzyknięcie kontekstu bez tury.
- **Idempotencja**: `idempotency_key` na enqueue/start/steer — deduplikacja retrasy
  (kluczowe przy reconnect!).

## 7. Callbacki: approvals i pytania (server→client request!)

Zamiast requestów `session/request_permission`/`session/request_input`, serwer wysyła
**JSON-RPC request do klienta** metodą `callback/call` (`server.py:1609`):

```
S->>K: request "callback/call" {callback: PublicCallbackEntry}
K-->>S: response {callbackId, accepted: true}
K->>S: request "callback/result" {sessionId, result: {callbackId, output}}
S-->>K: response {accepted: true, lastEventId}
```

- `PublicCallbackEntry` (`models.py:1030`): `{id, type:"callback", callback_id, title,
  detail, state}`.
- `detail` — dyskryminowane polem `kind`:
  - **`approval`** (`models.py:324`): `{effect: EffectDetail, required_permissions,
    choices: [approve|approve_for_session|approve_permanently|deny|cancel_turn],
    path_scope_choices, reason?}`; output:
    `{"type":"approval","decision":{"type":"approve","pathScope":null},"feedback":null}`.
  - **`user_input`**: `{request: UserQuestionRequest, related_entry_id?}`; output:
    `{"type":"user_input","result": {...UserQuestionResult}}`.
  - `connector_auth` — OAuth connectora (deklarować tylko po implementacji).
- `callback/result` (`protocol.py:2513`): `{session_id, result: CallbackResult
  {callback_id, output, error?}}`; błędy: `CALLBACK_CLOSED`.
- Klient odpowiada **dwukrotnie**: natychmiastowym ack na request `callback/call` ORAZ
  osobnym requestem `callback/result` z decyzją. Entry callbacku pojawia się też
  w historii sesji i w `state.active_callbacks`.
- **Serwer nie wysyła callbacku rodzaju nieobecnego w `callbackKinds` klienta**.

## 8. Notyfikacje serwera

Każda notyfikacja ma params bazowe `EventNotificationParams` (`protocol.py:2546`):
**`{event_id: int, session_id, emitted_at: int}`** — `event_id` to globalny, rosnący
kursor stanu!

| Metoda | Params | Znaczenie |
|---|---|---|
| `session/snapshot` | `{state: PublicSessionState}` | pełny zrzut stanu (startup, attach) |
| `session/updated` | `{patch: [JsonPatchOperation]}` | JSON-Patch na PublicSession |
| `history/entryAdded` | `{turn_id?, entry: PublicHistoryEntry}` | nowy wpis historii |
| `history/entryUpdated` | `{turn_id?, entry_id, patch}` | JSON-Patch na wpisie — streaming treści to patche! |
| `turn/started` | `{turn: PublicTurn}` | start tury |
| `turn/completed` | `{turn: PublicTurn}` | koniec tury (`status`, `error`, `stop_reason`) |
| `turn/retrying` | `{params}` | retry po awarii streamu |
| `turn/queueUpdated` | `{queue: PublicTurnQueue}` | zmiana kolejki promptów |
| `session/statsUpdated` | `{stats: AgentStatsSnapshot, context_window}` | statystyki tokenów/kosztów natywnie |
| `session/childSessionUpdated` | `{child_session: PublicChildSession}` | subagent (postęp, token_usage) |
| `session/compacted` | `{old_session_id, state, summary_length}` | **natywny event kompaktu** — handoff: nowe session_id! |
| `session/contextCleared` | `{old_session_id, state, session_log}` | wyczyszczenie kontekstu — też handoff |
| `mcp_catalog/authRequired` | `{params}` | serwer MCP wymaga OAuth |
| `connector_catalog/authRequired` | `{params}` | connector wymaga logowania |
| `runtime/updated` | `{runtime}` | runtime się zmienił → przeczytać `runtime/read` |
| `warning` | `{params}` | ServerWarning (niekrytyczne) |

## 9. Model stanu: PublicSessionState

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

- **Streaming treści**: wpis assistant dodawany raz (`history/entryAdded`), potem
  aktualizowany **JSON-Patchem** (`history/entryUpdated`). Patch ops:
  `add|append|replace|remove|test` — **to nie RFC6902: ma dodatkowe `append`!**
- `PublicHistoryEntry` (dyskryminowane `type`): `user`, `assistant`, `reasoning`,
  `effect` (title, detail: EffectDetail — tool calle, edycje, komendy; state),
  `callback`, `notice` (hook_run_*, agent_changed, session_title_updated, …).
- Referencyjny reduktor: `vibe/app_server/events.py` → `ClientProjection.consume()`
  (`EventSequenceError` przy niespójnym event_id/session_id, handoff przy
  compacted/contextCleared).
- Referencyjny stan klienta: `vibe/app_server/client_state.py`.

## 10. Spójność i recovery (watermark + events/read)

- **Każda mutująca odpowiedź** (`session/start`, `resume`, `steer`, `interrupt`,
  `callback/result`…) zawiera `last_event_id` (`EventWatermarkResponse`,
  `protocol.py:284`).
- Notyfikacje niosą własne `event_id`. Luka = ostatni widziany event_id <
  `last_event_id` → doszło coś, co przegapiłeś.
- **`events/read`** (`protocol.py:322`): `{after_event_id, filters: {session_ids, …},
  batch_size}` → `EventBatch` — odtwarzanie lukii. Błąd `STALE_CURSOR` gdy okno
  retencji minione → pełny resync przez `session/read`.
- W referencyjnym kliencie odpowiedzi „attach" sesji wchodzą do tego samego strumienia
  co notyfikacje (response boundary — `client.py:_dispatch`, ADR 0009: kolejność
  redukcji ma znaczenie).
- **Reconnect**: `initialize` → `session/resume` (pełny snapshot) → applied od zera.
  Odpowiedź zawsze autorytatywnie zastępuje stan.

## 11. Konfiguracja, model, agent mid-session

- **`session/agent/update`** (`AgentSwitchParams`, `protocol.py:1075`):
  `{session_id, agent_name}` — zmiana trybu/agenta w trakcie sesji.
- **`config/model/write`** — zmiana modelu (z `session_only`).
- **`session/settings/update`**: `{session_id, max_turns?, max_tokens?}`.
- **`runtime/read`** → `RuntimeSnapshot` (`protocol.py:1095`): `{config, active_agent,
  agents[], skills[], tools[], stats, context_window, issues[], hooks_count,
  connectors, mcp, bypass_tool_permissions}` — źródło danych dla pickerów.
  Notyfikacja `runtime/updated` sygnalizuje zmiany.
- **`agents/list`** → `{active, agents[]}` — lista profilów agentów (trybów).
- **`config/read` / `config/write`** — TOML config przez ops, `config/model/write`
  do zmiany modelu.

## 12. Kody błędów (ProtocolErrorCode, `protocol.py:2617`)

`INVALID_REQUEST, INVALID_PARAMS, NOT_INITIALIZED, NOT_FOUND, CONFLICT, STALE_TURN,
NOT_STEERABLE, CALLBACK_CLOSED, COMPACTION_FAILED, UNAUTHORIZED, FORBIDDEN,
METHOD_NOT_FOUND, NOT_IMPLEMENTED, STALE_CURSOR, INTERNAL_ERROR`

`INVALID_PARAMS` niesie w `data.issues[]` listę `{path, message}` — renderuj ścieżki
pól przy debugowaniu.

## 13. Opcjonalne client tools (serwer→klient)

Jeśli klient zadeklaruje `client_tools`, serwer może **delegować narzędzia do klienta**
(requesty, `_connection_protocol.py:26`): `clientTool/readTextFile`,
`clientTool/writeTextFile`, `clientTool/terminal/create|wait|output|kill|release`.
Dla GUI: nie deklarować w v1.

## 14. Mapa źródeł (do dalszej lektury)

| Plik | Zawartość |
|---|---|
| `vibe/app_server/protocol.py` (2741 linii) | **cała wierza**: SERVER_METHODS, Params/Response, enumy błędów |
| `vibe/app_server/models.py` | Public* modele, content blocks |
| `vibe/app_server/events.py` | ClientProjection — referencyjny reduktor |
| `vibe/app_server/client.py` | referencyjny klient JSON-RPC (sekwencjonowanie, response boundary) |
| `vibe/app_server/client_state.py` | referencyjny stan klienta (bootstrap) |
| `vibe/app_server/_connection_protocol.py` | initialize, ClientCapabilities, client tools |
| `vibe/app_server/server.py` | routing, callbacki, pending notifications |
| `vibe/cli-rust/src/server/` | **referencyjny klient w Rust**: process.rs, connect.rs, callback.rs, proto_approval.rs, proto_questions.rs, frames.rs, multiplex.rs, effect.rs, types/ |
| `client-e2e/` | golden harness testów E2E klienta |

## 15. Sonda protokołu

`scripts/probe_appserver.py` — minimalny klient w Pythonie: spawn `vibe-app-server`,
initialize, session/start, turn/enqueue, nasłuch notyfikacji, callback ack.
Kolejność weryfikacji: (1) initialize zwraca serverInfo; (2) session/start → state +
lastEventId; (3) enqueue → turn/started, history/entryAdded (user), entryAdded+
entryUpdated (assistant), turn/completed; (4) `session/list` pokazuje sesję;
(5) reconnect: drugi proces, `session/resume` oddaje pełny snapshot.

## 16. Pułapki (empirycznie z kodu)

1. **ID requestów klienta muszą być stringami** — serwer rzuca
   `JsonRpcProtocolError("Client request responses require string IDs")`.
2. **`cwd: ""` odrzucone** walidatorem — pomijaj pole zamiast wysyłać puste.
3. **Callback kinds muszą być zadeklarowane** w initialize, inaczej delivery callbacku
   = RuntimeError serwera.
4. **`history/entryUpdated` używa własnego JSON-Patch z op `append`** — nie używaj
   gotowej biblioteki RFC6902 bez rozszerzenia o `append`.
5. **Handoff** (`session/compacted`/`contextCleared`) podmienia `session_id`
   (pole `old_session_id`) — przemapuj wszystkie mapowania po starym ID.
6. **Kolejność redukcji ma znaczenie**: odpowiedzi attach i notyfikacje są w jednym
   strumieniu (response boundary) — nie parsuj odpowiedzi poza kolejnością.
7. **Ważność `event_id`**: ClientProjection rzuca `EventSequenceError` przy cofniętym
   kursorze — pełny resync zamiast składania na skróty.
8. **Wersjonowanie**: app-server **nie jest stabilnym API** — w 3.x usunięto
   `tools/list`, `stats/read`, `diagnostics/list`. Pinuj wersję CLI w testach.
9. **Handshake jest LSP-podobny**: po odpowiedzi `initialize` serwer jest w stanie
   `INITIALIZE_RECEIVED` i dopiero notyfikacja **`initialized`** przenosi go do
   `INITIALIZED` — bez niej każde kolejne żądanie dostaje `NOT_INITIALIZED`
   (`server.py: _handle_notification`; zweryfikowano na 3.0.3).
10. **`kind` w `session/start` jest lowercase**: `"normal"` / `"ephemeral"`
    (StrEnum) — `"NORMAL"` daje INVALID_PARAMS.
11. **Wpisy historii to `type: "message"`** z polem `role` (`user`/`assistant`/
    `system`) — nie osobne typy `user`/`assistant`; `PublicMessageEntry`
    ma też `generationStatus`, `createdAt`/`updatedAt` jako **int (epoch ms)**.
    Streaming: treść przyszła w `entryAdded`, patche `entryUpdated` niosą
    zmiany statusu/`updatedAt` (op `replace`), nie appendy treści.
