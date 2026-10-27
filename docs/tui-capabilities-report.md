# Mistral Vibe Rust TUI — funkcjonalności user-facing i wire protokół vibe-app-server

> Badanie kodu: `mistralai/mistral-vibe`, checkout **v3.0.3** (`/home/dragonk/projects/mistral-vibe-src`,
> commit `7bb48f2`). Kod ma pierwszeństwo przed `docs/appserver-protocol.md`.
> Ścieżki: `cli-rust/src/…` = Rust TUI, `app_server/…` = Python backend (serwer JSON-RPC).
> Podane `plik:linia` dotyczą checkoutu v3.0.3.

---

## 0. TL;DR architektury

Rust TUI (`cli-rust/`) jest **cienkim klientem** vibe-app-servera (proces Python `vibe-app-server`,
spawn: `cli-rust/src/server/process.rs`). Cała logika sesji jest po stronie serwera; TUI:

- wysyła requesty JSON-RPC 2.0 po stdio (NDJSON), pełna lista metod, które **TUI faktycznie woła**:
  `cli-rust/src/server/types.rs:10` (`pub mod method` — ok. 70 stałych);
- obsługuje notyfikacje serwer→klient: `cli-rust/src/server/types.rs:128` (`pub mod notification`);
- odpowiada na **requesty serwera**: `callback/call` (`server_method::CALLBACK_CALL`, `types.rs:120`)
  dla kind `approval` i `user_input` (`types.rs:124`, `server/callback.rs:85`);
- redukcja notyfikacji do stanu UI: `cli-rust/src/event_handler.rs` (np. `history/entryAdded` :659,
  `history/entryUpdated` :697, `session/snapshot` :713, `turn/queueUpdated` :852,
  `session/statsUpdated` :857, `session/childSessionUpdated` :894).

Metody serwera istniejące w `app_server/protocol.py:108` (`SERVER_METHODS`), ale **niewykorzystywane
przez TUI** (są dla Desktop/ACP): `session/pin`, `session/archive`, `session/markAsSeen`,
`session/title/update`, `session/relocate`, `session/settings/update`, `session/turns/list`,
`session/history/get`, `loops/*` (TUI nie woła ich bezpośrednio — patrz §Loops), `projectLinks/*`,
`imports/*`, `review/*`, `vibeCode/projects/*` poza teleport-pickerem, `skills/*` (TUI nie ma
ekranu skills), `agents/install|uninstall` (ma `/leanstall`), `client tools` (`filesystem/*`,
`terminal` — TUI ich nie reklamuje).

---

## 1. Pełna lista funkcjonalności user-facing TUI

### 1.1 Ekran czatu

| Funkcjonalność | Kod (plik:linia) | Wire | Co zrobić w GUI |
|---|---|---|---|
| Wysłanie promptu (kolejkowanie) | `message_queue/requests.rs:16,210` | `session/turn/enqueue` (`TurnEnqueueParams` z `entries`, idempotency key) | pole tekstowe + przycisk wysyłania dodające do kolejki |
| Start natychmiastowy (z pominięciem kolejki) | `server/types.rs:36` (`TURN_START`), `message_queue/` | `turn/start` | takiej ścieżki GUI nie ma — używać enqueue |
| Streaming odpowiedzi (przyrostowy) | `event_handler.rs:659,697` | `history/entryAdded` / `history/entryUpdated` (JSON Patch, `generationStatus:"in_progress"`) | render Markdown z replace-in-place po patchach |
| Thinking / reasoning | `server/types.rs:594` (`ReasoningEntry`), `event_handler.rs:659` | typ historii `reasoning` (tekst, streaming przez entryUpdated) | zwijany blok „thinking” |
| Tool cally (efekty narzędzi) | `server/effect.rs`, `server/effect_output.rs`, `ui/transcript/entry/effect/` | typ historii `effect` (EffectEntry; input/output/detail per tool) | karta tool-callu: nazwa, input, wynik, status |
| Zatwierdzanie tool calli (approval) | `ui/approval/`, `server/callback.rs:25` | request serwera `callback/call` (kind `approval`) → odpowiedź klienta `callback/result` | modal zatwierdzenia (tak/nie/always) |
| Pytania do użytkownika (user_input) | `ui/question_layout.rs`, `question_app/`, `server/proto_questions.rs:27` | `callback/call` (kind `user_input`, payload `UserQuestionRequest`) → `callback/result` | formularz/pytanie wyboru |
| Kolejka promptów (edytuj/usuń) | `message_queue/edit.rs`, `replacement.rs:123`, `requests.rs:170` | `session/turn/queue/edit/start|save|cancel|abandon`, `session/turn/queue/replace`, `…/remove` | lista oczekujących promptów z akcjami edycji/usunięcia |
| Steer (wstrzyknięcie w aktywny turn) | `message_queue/steering.rs:75` | `session/turn/queue/steer` (`expectedTurnId`), alternatywnie `turn/steer` | przycisk „wyślij teraz w trakcie odpowiedzi” |
| Resume kolejki po pauzie serwera | `message_queue/requests.rs:111` | `session/turn/queue/resume` | automatyczne; brak UI |
| Przerwanie turna (Esc) | `server/types.rs:62` (`TURN_INTERRUPT`) | `turn/interrupt` (`expectedTurnId`) | przycisk stop |
| Retry po błędzie | `event_handler.rs:958` (`turn/retrying`), `event_handler.rs:1040-1070` | notyfikacja `turn/retrying` + `session/snapshot.retrying` (`PublicRetryState`); błąd turna z `turn.error.code == "rate_limit"` | baner „retrying” + komunikat błędu z `/retry` |
| Kompakt ręczny | `commands/compact.rs:55` | `session/compact` → notyfikacja `session/compacted` | przycisk/komenda `/compact [instructions]` |
| Wyczyszczenie kontekstu | `server/types.rs:52` | `session/history/clear` | `/clear`, `/new` |
| Własne komendy `!` shell | `server/types.rs:64-70` | `session/shellCommand` (`action: run|interrupt`) | terminal inline |
| Procesy w tle | `server/proto_approval.rs`, `app_server/protocol.py` (`session/backgroundProcess/output/stop`) | `session/backgroundProcess/output`, `…/stop` | podgląd wyniku długiego procesu |
| Zmiana modelu / thinking level | `config_write.rs:69` | `config/model/write` (`sessionOnly` flag) | picker modelu w stopce (klik) i `/model`, `/thinking` |
| Zmiana agenta | `server/types.rs:55` | `session/agent/update` | `/agent`, `/leanstall` (`agents/install`) |
| Skill invocation ze slash-mode | `skill.rs`, `input_modes.rs:75` | prompt z `injectInvokedSkill` / `session/context/inject` (`ContextInjectParams`) | autocomplete `@`/`/` dla skilli |
| Obrazki / wklejanie | `server/images.rs`, `paste_image/`, `server/types.rs` (`ContentBlock::Image`) | `ContentBlock {type:"image", uri, mediaType}` w entries | wklejanie obrazka do kompozytora |
| Wspominki `@plik` | `paste_path/`, `mention_stats` w `turn/start` | `MentionStats` w params | dymek statystyk kontekstu |
| Głos (narracja, feature `voice`) | `voice/`, `voice_app.rs:131`, `turn_summary.rs:248` | `narration/summarize`, `telemetry/record` | przycisk mikrofonu + odtwarzanie podsumowania |
| Retry/ponów generację | `commands/simple.rs` (`/retry`) | brak osobnej metody — nowy `turn/start` | przycisk retry przy przerwanej odpowiedzi |

### 1.2 Sesje

| Funkcjonalność | Kod | Wire | Co zrobić w GUI |
|---|---|---|---|
| Lista zapisanych sesji + preview | `resume_picker/request.rs:30` | `session/list` (`{cwd}`) | ekran „Resume” z listą i podglądem |
| Resume sesji | `resume_picker/request.rs:51`, `startup/attach.rs:99` | `session/resume` (`sessionId` + `agentConfig`) | klik na sesję |
| `--continue` (ostatnia sesja) | `startup/attach.rs:172`, `headless/open.rs:40` | `session/list` → `continueSessionId`, `session/continue` | przycisk „Continue” |
| Nowa sesja | `startup/` | `session/start` (+ `firstTurn`, `title`, `model`) | przycisk „New chat” |
| Usunięcie sesji | `resume_picker/request.rs:64` | `session/delete` | ikona kosza w liście |
| Zmiana tytułu | `commands/simple.rs:150` | `session/rename` | `/rename <title>`; tytuł nadaje też serwer (`autoTitle` w `turn/start`) |
| **Pin / archive / unseen** | — TUI **nie używa** (brak wywołań w cli-rust) | `session/pin`, `session/archive`, `session/markAsSeen`, `SessionListParams.pinned/includeArchived` | dla GUI Desktop: pin/archive jako przełączniki + licznik nieprzeczytanych; TUI tego nie pokazuje |
| Lease (blokada sesji między procesami) | `resume_picker/mod.rs:103-138` | `session/lease/subscribe` + notyfikacje `session/lease/updated|ended` | znacznik „sesja otwarta gdzie indziej” |
| Worktree sesji w podglądzie | `server/types.rs:630` (`PublicSessionWorktree`) | pole `worktree` w `PublicSession` | badge nazwy worktree przy sesji |
| Eksport runa (headless) | `run_export/`, flaga `--output-dir` (`cli.rs:110`) | brak wire — pliki lokalne | (desktop: nie dotyczy) |

### 1.3 Kompakt / kontekst

- `/compact` → `session/compact` (`commands/compact.rs:55`), wynik przez `session/compacted`
  (handoff nowego session id — `SessionHandoffParams`, `app_server/protocol.py:2538`).
- Auto-kompakt po stronie serwera; TUI dostaje to samo notyfikacją.
- Licznik kontekstu w stopce: `ui/bottom_bar.rs:52,277` — `"{current}/{max} tokens ({pct}%)"`,
  dane z notyfikacji `session/statsUpdated` (`StatsUpdatedParams.stats` + `contextWindow`).
- GUI: pasek zużycia kontekstu + akcja kompaktu przy progu.

### 1.4 Statystyki / usage

- `/status` → `commands/simple.rs:71` renderuje **lokalnie** ze stanu `session/statsUpdated`
  (`AgentStatsSnapshot`): steps, session prompt/completion/total tokens, cached tokens, last turn, **cost USD**.
- Stopka: licznik tokenów kontekstu (`ui/bottom_bar.rs:277`).
- Brak ekranu usage per model / per dzień — te dane nie istnieją na wire (patrz §2).

### 1.5 Subagenci

- Interaktywnie: TUI **tylko wyświetla** child sessions z `PublicSessionState.childSessions`
  (`server/types.rs:447`) i notyfikacji `session/childSessionUpdated` (`event_handler.rs:894`);
  UI: `ui/subagent_list.rs:35` (docked lista, widok dziecka po kliknięciu).
- Metody `session/subagents/read|wait|interrupt` (eksperymentalne, `app_server/protocol.py:279`)
  używa **tylko headless** (`headless/subagents.rs:36,71,92,107`).
- Serwer może zapytać klienta o spawn nowej sesji: request `session/spawn`
  (`app_server/protocol.py:2276+`, `SESSION_SPAWN_METHOD`) — TUI go **nie reklamuje**
  (`capabilities` bez `sessionSpawn`); Desktop tak.
- GUI: lista subagentów ze statusem + wejście w ich transkrypt.

### 1.6 Checkpoints / rewind / fork

- `/rewind` + podwójny Esc: `rewind/mod.rs:22,58-112` → czyta `session/rewind/read`
  (`rewind/request.rs:31` — `hasFileChanges`, `paths`) → `session/rewind`
  (`rewind/request.rs:68`, params `entryId`, `restoreFiles`, `inplace`) → wynik: nowy stan + przywrócony prompt w kompozytorze.
- Wpisy checkpoint w historii: `server/types.rs:611` (`CheckpointEntry`), serwer: `models.py:1094`.
- `/branch` (fork): `commands/branch.rs:22` → `session/fork` (`SessionForkParams`: `entryId`/`afterTurnId`, `attach`).
- GUI: klik na wpisie historii → „cofnij tu / przywróć pliki” + „utwórz gałąź”.

### 1.7 Worktrees

- Start w worktree: flaga `--worktree [NAME]` (`cli.rs:143`) → `AgentConfig.worktree`
  (`WorktreeInput {kind: create|auto|existing}`, `app_server/protocol.py:300`) w `session/start`.
- Sprzątanie na wyjściu: `worktree_exit.rs:125` → `workspace/git/worktrees/remove`
  (z `force`/`inspect`/`deleteBranch`, odpowiedź `WorktreeRemoveResponse.outcome`).
- Pełna administracja worktree (`workspace/git/worktrees/list|prune|limit/update`) — TUI nie ma ekranu; dla Desktop GUI zarządzania.
- GUI: wybór „nowy worktree / istniejący” przy starcie + dialog usuwania z podsumowaniem `outcome`/`holders`.

### 1.8 Loops (scheduled prompts)

- `/loop [schedule] [prompt]` **nie woła `loops/*` z TUI** — jest klasyfikowany jako prompt
  (`input_modes.rs:75`, `commands/submission.rs:259`) i model scheduluje go narzędziem `cron`
  po stronie serwera (`app_server/_cron.py:3`, `loops/create` wewnątrz runtime).
- Fajerwerki loopa widoczne jako notice: `ScheduledLoopFiredNoticeDetail` (`models.py:1079`,
  parsowane w `server/types.rs:813`) → wpis w transkrypcji.
- GUI: może wołać `loops/list|create|delete|clear` bezpośrednio (`ScheduledLoop`: `id`, `prompt`,
  `intervalSeconds`|`cron`, `nextFireAt` — `models.py:1357`).

### 1.9 Skills / agenci / MCP / konfiguracja

| Funkcjonalność | Kod | Wire |
|---|---|---|
| Runtime snapshot (model, agent, skills, tools, stats, context window, issues, connectors, mcp) | `startup/reads.rs`, `event_handler.rs` | `runtime/read` + notyfikacja `runtime/updated` |
| Lista/edycja configu | `config.rs:34`, `config_command.rs:61`, `config_write.rs:49` | `config/fields/read`, `config/schema`, `config/write` (batch op `set`/`remove`), `config/reload`, `config/effective/read` |
| MCP + connectors UI | `mcp/mod.rs:113,133`, `mcp/commands.rs:42,96,153`, `mcp/toggle.rs:86` | `mcp_catalog/read`, `connector_catalog/read`, `mcp_catalog/toggle|add|remove|login|logout|refresh`, `connector_catalog/toggle|connect|refresh` |
| OAuth MCP / connector | `mcp_oauth.rs`, `connector_auth/mod.rs:105,131` | notyfikacja `mcp_catalog/authUrl` (URL do otwarcia), potem `connectors/auth/read` + `connectors/refresh` |
| Plugins | `ui/plugins.rs`, `plugins/request.rs:73` | `plugin_catalog/read`, `plugin/reload` (`/plugins`, `/reload-plugins`) |
| Instalacja agenta `leanstral` | `commands/commands.rs:74-75` | `agents/install`, `agents/uninstall` |
| Proxy/SSL setup | `proxy_setup.rs:154,208` | `config/proxy/read`, `config/proxy/write` |
| Motyw | `theme/`, zapis przez `config/write` (`/theme`) | `config/write` path `/theme` |
| Feedback nudge | `feedback.rs:65,164,176` | `feedback/shouldShow`, `feedback/record`, `telemetry/record` |
| Debug console | `debug_console.rs:43`, `debug_console/paging.rs:110` | `diagnostics/logs/read`, `session/log/read` |
| Trust workspace | `trust_folders.rs`, `startup/trust.rs` | `workspace/trust/status|decision|untrustedConfig` |

### 1.10 Vibe Code Web / teleport

- `/teleport` oraz `&prompt`: `teleport/mod.rs:41,53` → picker projektów
  `vibeCode/projects/open|loadMore|select|create|unlink|cancel|recover` (`vibe_code_project/request.rs:26`)
  → `vibeCode/teleport/start` (`teleport/request.rs:73`) → postęp przez notyfikację
  `vibeCode/teleport/event` (`teleport/mod.rs:97`, flaga `--teleport` `cli.rs:151`).
- Artefakty PR/MR: `artifacts.rs:24` → `artifacts/list`, modal `ui/artifacts_modal.rs`;
  notyfikacje `artifacts/created|updated` (wzbogacane pola `prStatus`, `checks`, `mergeStatus` —
  `app_server/protocol.py:1470+`).

### 1.11 Update CLI

- Sprawdzanie w trakcie sesji: `event_handler.rs:132` → `get_update_if_available`
  (`update_notifier/update.rs:134`) — źródło: **PyPI simple index** (`update_notifier/gateway.rs:13`
  `https://pypi.org`) + oracles brew/uv; cache i dismiss w pliku cache (`update_notifier/cache.rs`).
- Prompt instalacji: `update_prompt/mod.rs:124-148`, wykonanie: `uv tool upgrade mistral-vibe`
  / `brew upgrade mistral-vibe` (`update_notifier/update.rs:18`); `vibe update`/`--check-upgrade`
  (`main.rs:139`, `update_flow.rs`).
- „What's new” po aktualizacji: `startup/banners.rs:90-99`.
- GUI: badge „dostępna aktualizacja” + dialog update; treść „what's new” serwer nie dostarcza (to cache klienta).

---

## 2. Mistral Vibe Pro / subskrypcja — stan na wire ⚠️

**Wniosek: protokół nie niesie żadnych kwot, limitów ani „remaining”.** Grepy po
`quota|remaining|subscription|credits|billing|rate_limit` w `cli-rust/src` nie znajdują żadnej
struktury z licznikami. Jedyna kontrola konta to `account/read`:

- request: `{sessionId}` → odpowiedź `AccountView` (`app_server/models.py:119-126`):

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

- TUI czyta z tego **wyłącznie** `plan.title` (`cli-rust/src/post_ready.rs:37-43`) —
  dopisek do linii modelu w bannerze (`ui/banner/mod.rs:23,88`), `planOffer.url`
  (`startup/banners.rs:120-131` — CTA w „what's new”), `rateLimitAction`
  (`event_handler.rs:1061-1070` — zmienia tekst błędu rate-limitu), oraz `apiKey.preview/scope/accessScope`
  i `identity/read` dla `/whoami` (`commands/simple.rs:232-280`).
- Typy: `AccountStatus` (`models.py:83`), `AccountPlanKind` = `api|chat|mistral_code` (`models.py:90`),
  `AccountActionKind` = `switch_api_key|upgrade_to_pro` (`models.py:96`), `AccountApiKeyView` (`models.py:112`).
- Skąd serwer bierze dane: `GET {console_base_url}/api/vibe/whoami` (`app_server/../setup/auth/whoami.py:30`
  `_WHOAMI_PATH`, `HttpAccountGateway.read:82`), odpowiedź mapowana na `WhoAmIResult`
  (`plan_type`, `plan_name`, `prompt_switching_to_pro_plan`, `organization_kind`, `api_base`, `vibe_base`,
  `primitive_access_scope`) + cache 6 h per klucz (`whoami.py:117`). Kontrola: `app_server/_account.py:166-181`
  (`offers_upgrade`, `rate_limit_upgrade_available`), URL upgradu: `_UPGRADE_PLANS_URL` (`_account.py:62`).
- Rate limit w praktyce: turn kończy się błędem `turn.error.code == "rate_limit"`
  (`event_handler.rs:1043`), TUI pokazuje tekst z `rate_limit_message` (`event_handler.rs:1061`).
- **Brak**: remaining requests/credits, okna resetu, per-org limitów, zużycia Pro. GUI chcące pokazać
  „ile zostało w pakiecie Pro” musi albo czekać na rozszerzenie `account/read` (dodać pola do
  `AccountView` — to najkrótsza droga, serwer i tak cache'uje whoami), albo czytać endpoint
  `/api/vibe/whoami` samodzielnie (pole `plan_name`/`prompt_switching_to_pro_plan` to jedyny sygnał).
- Pole `rateLimitAction` nie-null = serwer uznał, że upgrade ma sens (plan API albo Mistral Code F) —
  można traktować jako „JESTEŚ na limicie planu API”; planu CHAT Pro nie dotyczy.

---

## 3. Logowanie do Mistral Vibe — przepływ krok po kroku

### 3.0 Kto robi co

Zaskoczenie: **przeglądowe logowanie robi sam Rust TUI** (moduł `cli-rust/src/setup/auth/`), nie
app-server. App-server uczestniczy przez `setup/*` (seed + zapis), a jego Pythonowa kopia flow
(`setup/auth/browser_sign_in.py`) służy ACP/desktopowi. Sekwencja wire:

1. **`setup/status`** (`setup/auth/rpc.rs:24-70`, serwer: `app_server/_setup.py`):
   param `{provider?}`; odpowiedź (camelCase):

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

   `supportsBrowserSignIn` i `hasApiKey` decydują, które ekrany pokaże wizard; `browserAuth*`
   to endpointy do przepływu przeglądarkowego; `pinnedConsoleUrl` (`VIBE_CUSTOM_CONSOLE_URL`) blokuje domenę logowania.

2. **Przepływ przeglądarkowy (PKCE, bez localhost callbacku)** — `setup/auth/sign_in_flow.rs:30`:
   - generacja `code_verifier` (64 B random) + `code_challenge` = BASE64URL(SHA256(verifier)) (`sign_in_flow.rs:137-150`);
   - `POST {browserAuthApiBaseUrl}/vibe/sign-in` z `{"code_challenge":…, "code_challenge_method":"S256"}` →
     `{process_id, sign_in_url, poll_url, expires_at}` (`setup/auth/sign_in_gateway.rs`; Python odpowiednik:
     `setup/auth/http_browser_sign_in_gateway.py:60-95`);
   - otwarcie `sign_in_url` w przeglądarce (`external_url::open`, `sign_in_flow.rs:47`; domyślnie
     `https://console.mistral.ai/codestral/cli/authenticate?process_id=…`);
   - **polling `poll_url` co 3 s** aż do `expires_at` (`sign_in_flow.rs:95-137`): statusy
     `pending | completed (zwraca exchange_token) | expired | denied | error`; po 3 kolejnych błędach sieci — porażka;
   - **exchange**: `gateway.exchange(process_id, exchange_token, code_verifier)` → `{api_key}`
     (`http_browser_sign_in_gateway.py:150+`; obsługa 403 = workspace nieeligible, 410 = wygasło, kod 9004 = brak dostępu);
   - brak nasłuchiwania na localhost — to polling, nie OAuth redirect.
3. **`setup/store-credential`** (`rpc.rs:73-100`, serwer `SetupStoreCredentialParams`): `{provider, apiKey, customDomain}`
   → odpowiedź `{outcome: "completed"|"env_var_error"|"save_error", detail?}`; serwer zapisuje do
   **keyringu**, fallback `$VIBE_HOME/.env` i podbija swój process env (`_setup.py`; `api_key_persistence.py`).
4. **`setup/submit-choices`**: `{provider?, consoleBaseUrl?, vibeBaseUrl?, theme?}` (absencja = „zostaw”) →
   `{outcome: "completed"|"provider_config_error", failures[]}` — domyka onboarding (zapis configu + theme).
5. Po restarcie sesji stan konta: `identity/read` + `account/read` (`post_ready.rs:20-45`).

Przez **klucz API**: ekran ApiKey → paste → `SubmitApiKey` (`setup/wizard/mod.rs:165-175`) →
`setup/store-credential` + `setup/submit-choices` (`setup/wizard/actions.rs:124,167`).

Dodatkowe ścieżki auth w TUI (nie-logowanie): OAuth connectorów przez notyfikację `mcp_catalog/authUrl`
(`mcp_oauth.rs`, `server/types.rs:139`) i `connectors/auth/read` (`connector_auth/mod.rs:105`).

---

## 4. Ekran logowania w TUI (co widać bez klucza)

Ekrany wizarda (`cli-rust/src/setup/wizard/mod.rs:29-37`):

1. `Welcome` — animacja + typowanie (gate: czeka na boot `setup/status` przed Enterem, `setup/wizard/flow.rs:69-81`),
2. `ThemeSelection`,
3. `AuthMethod` — wybór: **przeglądarka vs klucz API** (pokazane wg `supportsBrowserSignIn`),
4. `SignInTarget` / `CustomDomain` — domyślny console vs własna domena (walidacja URL: `setup/wizard/context.rs:91-100`),
5. `BrowserSignIn` — stepper: Open browser → Waiting → Exchanging → Complete (`ui/transcript`-like; paint: `setup/wizard/paint/browser_sign_in.rs`; URL do skopiowania przy braku przeglądarki — akcja `CopySignInUrl`),
6. `ApiKey` — input klucza z walidacją (`ValidationState`).

Wizard działa **przed sesją** (osobna pętla eventów, `setup/wizard/flow.rs:31`), wpisuje się przez
`--setup`, brak klucza przy starcie, lub po `session/start` z błędem auth (`startup/mod.rs:346-364`).
TUI w trybie czatu nie ma osobnego „login screen” — niezaalogowanie = wizard na starcie.

---

## 5. Wersjonowanie / kompatybilność / feature detection

- Handshake: `initialize` z `clientInfo{name,version,entrypoint,terminalEmulator}` + `capabilities{callbackKinds, clientTools, disabledNotifications}` (`server/types.rs:196-236`); odpowiedź `serverInfo{name,version}` — wersja serwera pokazywana w bannerze (`startup/mod.rs:162`, `ui/banner/mod.rs:56`).
- **Feature detection przez błędy**: `method_not_found`/-32601 = starszy serwer (`server/types.rs:692` `is_method_not_found`); użyte m.in. w wizardzie (`setup/auth/rpc.rs:183,193`), `config_write.rs:71`, `plugins/request.rs:73`. Dodatkowe kody: `unauthorized`, `conflict`+`harnessCode:"session_busy"` (`types.rs:668-680`).
- **Metody eksperymentalne**: `session/subagents/*` — mogą zniknąć z listy `initialize`; klient ma tolerować brak (`app_server/protocol.py:279-284`).
- Strict mode wersjonowania: `config/write` op-uje z `extra="forbid"` — niezgodne pole = `invalid_params` i nic nie zapisane (ADR 0014, `protocol.py:1806+`); rozszerzenia OPCJONALNE idą jako „absent on older servers” (`WorktreeRemoveParams.force/inspect/deleteBranch`, `types.rs:726-740`; `apiKey.accessScope` — „None on older backends”, `models.py:115`).
- Polyfill: `bin/replay.rs` mapuje stare→nowe metody przy odtwarzaniu nagrań.
- Flaga transportowa: multiplexing wielu sesji w jednym procesie serwera (`server/multiplex.rs`, `runs_per_app_server`).

---

## 6. Rejestr: metody wołane przez TUI vs nieużywane

**Wołane (stałe `server/types.rs:10-121`)**: initialize/initialized, session/start|list|read|resume|continue|delete|stop|rename|rewind|rewind/read|fork|compact|history/clear|ready/wait|lease/subscribe|shellCommand, session/turn/enqueue|queue/*|turn/start|steer|interrupt, session/agent/update, runtime/read, config/read|schema|fields/read|effective/read|write|model/write|reload|proxy/read|proxy/write, setup/status|store-credential|submit-choices, workspace/prompt/prepare, workspace/trust/*, workspace/git/worktrees/remove, mcp_catalog/read|refresh|toggle|add|remove|login|logout, connector_catalog/read|refresh|toggle|connect, connectors/auth/read|refresh, plugin_catalog/read, plugin/reload, agents/install|uninstall, callback/result, feedback/*, telemetry/record, narration/summarize, diagnostics/logs/read, session/log/read, identity/read, account/read, providerAuth/read, artifacts/list.

**Nie wołane przez TUI (dostępne dla Desktop/GUI)**: session/pin, session/archive, session/markAsSeen, session/title/update, session/relocate, session/settings/update, session/turns/list, session/history/get|list, session/subagents/* (eksper.), session/spawn (request), loops/*, projectLinks/*, imports/*, review/*, vibeCode/projects/* (poza pickerem teleportu), skills/*, artifacts/markViewed|sync, client tools filesystem/terminal.
