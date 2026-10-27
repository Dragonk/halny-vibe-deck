# Plan parity z Rust TUI Mistral Vibe — wdrożenie w GUI Halny Vibe Deck (branch `next`)

> Plan wdrożenia (tylko plan, bez zmian w kodzie). Źródła prawdy:
> [tui-capabilities-report.md](tui-capabilities-report.md) — **badanie TUI v3.0.3, w razie
> rozbieżności z [appserver-protocol.md](appserver-protocol.md) raport WYGRYWA**
> (referencja protokołu opisuje głównie Pythonową warstwę, raport — to, co klient
> realnie wysyła). Status transportu: [migration-appserver.md](migration-appserver.md)
> (F0–F7 done: transport `mistraldeck-appsrv` działa E2E, flaga `transport=appserver|acp`).
>
> Kod: `crates/mistraldeck` (UI Dioxus fullstack: `src/api.rs` = server-fn,
> `src/components/*` = widoki), `mistraldeck-engine` (maszyna stanu, sloty,
> `TransportHandle { Aps | Acp }`), `mistraldeck-appsrv` (worker: conn/proto/patch/
> reducer), `mistraldeck-core` (typy współdzielone: `proto.rs` = `AcpCommand`/
> `AcpEvent`, `snapshot.rs` = `Snapshot`), `mistraldeck-acp` (legacy).

## 0. Zasady nadrzędne (z raportu TUI)

1. **Kwoty / „ile zostało z Pro" NIE ISTNIEJĄ na wire.** Parity z TUI = dokładnie:
   - `plan.title` w bannerze (`post_ready.rs:37-43` → banner `ui/banner/mod.rs`),
   - `planOffer.{kind,url}` — CTA w bannerze „what's new" (`startup/banners.rs:120-131`),
   - `rateLimitAction` — zmiana tekstu błędu rate_limit + CTA (`event_handler.rs:1061-1070`),
   - `apiKey.preview/scope/accessScope` + `identity/read` — `/whoami` (`commands/simple.rs:232-280`).
   Wszystko z jednego źródła: **`account/read` → `AccountView`** (`models.py:119-126`).
   Żadnych liczników, window resetu, zużycia Pro — nie projektować UI, którego dane nie ma.
2. **Login przez przeglądarkę robi KLIENT (PKCE), nie serwer** (`setup/auth/sign_in_flow.rs:30`):
   - `code_verifier` (64 B random), `code_challenge` = BASE64URL(SHA256(verifier)),
   - `POST {browserAuthApiBaseUrl}/vibe/sign-in` `{code_challenge, code_challenge_method:"S256"}`
     → `{process_id, sign_in_url, poll_url, expires_at}`,
   - otwarcie `sign_in_url` (u nas: `window.open` po stronie przeglądarki klienta),
   - **poll `poll_url` co 3 s** do `expires_at`: `pending | completed(→exchange_token) |
     expired | denied | error` (3 kolejne błędy sieci = porażka),
   - exchange `(process_id, exchange_token, code_verifier)` → `{api_key}` (403 = workspace
     nieeligible, 410 = wygasło, 9004 = brak dostępu),
   - potem `setup/store-credential` → `setup/submit-choices`. Bez localhost-callbacku.
3. **Login kluczem**: paste → `setup/store-credential {provider, apiKey, customDomain?}`
   → `{outcome: completed|env_var_error|save_error, detail?}` → `setup/submit-choices`
   → `{outcome: completed|provider_config_error, failures[]}`.
4. **Seed**: `setup/status` → `supportsBrowserSignIn`, `hasApiKey`, `provider.browserAuth*`,
   `consoleBaseUrl`, `pinnedConsoleUrl` (blokada własnej domeny logowania).
5. **Feature detection przez `method_not_found` (-32601)** — starsze CLI bez `setup/*` /
   `account/read`: UI konto degradowane, nie zepsute (TUI tak samo: `setup/auth/rpc.rs:183,193`).
6. Klient JSON-RPC ma już wszystkie pułapki ogarnięte (string IDs, `append`-patch,
   watermark, handoff) — L1-L4 **nie zmieniają** conn/reduktora streamingu, tylko
   **dokładają metody request/response i notyfikacje**.

### Rozstrzygnięcia architektoniczne dla L1 (wcześniej niż pliki)

| Decyzja | Wybór | Uzasadnienie |
|---|---|---|
| Gdzie PKCE + HTTP? | **Serwer (engine)**, nie wasm | Klient wasm nie ma sieci (brak `fetch` w sandboksie, CORS na `api.mistral.ai`); klucz i tak zapisuje serwer (`store-credential` pisze do keyringu hosta). Desktop/webview idzie przez te same server-fn. HTTP: rozszerzenie istniejącego wzorca `curl` (`remote.rs::http_get_json`) o POST z JSON body — zero nowych ciężkich zależności. |
| Źródło losowości do verifiera | dep **`getrandom`** (0.2) w `mistraldeck-engine` | W grafie nie ma żadnego RNG (`rand`/`getrandom` brak; `sha2` nie jest RNG). 64 B z CSPRNG systemowego. Alternatywa odrzucona: `/dev/urandom` ręcznie (Windows). |
| BASE64URL | **własny encoder** (~15 linii, bez paddingu) w `auth_browser.rs` | `base64` nie ma w grafie; base64url-no-pad to trywialny kod z testem wektorowym. (Alternatywa: dep `base64` — dopuszczalna, jeśli wolisz zależność.) |
| SHA256 | **jest** (`sha2 = "0.10"` w `mistraldeck-engine`) | nic nie dodajemy. |
| Kto trzyma stan flow logowania | `Engine: Mutex<Option<BrowserSignInFlow>>` | poll idzie osobnymi server-fn (klient co 3 s, `gloo-timers`); verifier MUSI przeżyć między wywołaniami → stan na serwerze; `expires_at` wymusza timeout. |
| Które połączenie woła `setup/*` / `account/read` / `identity/read` | **lister** (istniejące połączenie z `vibe-app-server`) | to metody globalne, nie sesyjne — TUI woła je przed/obok sesji. Engine i tak robi `Start` na liście (`start_agent`); app-server przyjmuje `setup/status` bez sesji. |
| Otwarcie przeglądarki | klient wasm (`window.open(sign_in_url)`) | przeglądarka jest po stronie użytkownika; TUI robi `external_url::open`, my mamy okno już otwarte. `CopySignInUrl` jako fallback (parity z TUI). |
| Rate limit | `turn/completed.turn.error.code == "rate_limit"` + `rateLimitAction` z `account/read` | raport §2/§1.1; wymaga tolerancyjnej zmiany `Turn.error` na `Value` (obecnie `Option<String>` — obiekt `{code,message}` nie sparsuje się). |
| whoami bezpośrednio (`GET {console}/api/vibe/whoami`) | **NIE w L1**; opisane w §Ryzyka jako ewentualny dodatek | `plan.name` ("Pro") i `plan.title` i tak przychodzą z `account/read`; whoami dodałby tylko `plan_type`/`prompt_switching_to_pro_plan` przy koszcie ryzyka (patrz §6). |

---

## 1. Etap L1 — must-have (do buildu v1.99.2): Login & Account

**Cel:** użytkownik GUI może zalogować się (przeglądarka PKCE lub klucz API), zobaczyć
plan w bannerze, dostać CTA przy rate limicie i podgląd klucza w ustawieniach —
1:1 z tym, co pokazuje Rust TUI. Start aplikacji z-gatingiem, gdy brak klucza.

Kolejność prac (zależności kompilacji): **core → appsrv → engine → api.rs → UI → i18n → testy**.

### 1.0 `crates/mistraldeck-core` — typy (krok 1)

**`src/proto.rs`** — nowe warianty `AcpCommand` (na końcu enuma, bez łamania kolejności):

```rust
    // --- L1: setup / konto (metody globalne app-servera, nie sesyjne) ---
    /// setup/status — seed wizarda (supportsBrowserSignIn, hasApiKey, browserAuth*).
    SetupStatus,
    /// setup/store-credential — zapis klucza (keyring / $VIBE_HOME/.env).
    SetupStoreCredential { provider: Option<String>, api_key: String, custom_domain: Option<String> },
    /// setup/submit-choices — domknięcie onboarding (theme/console pozostają = "zostaw").
    SetupSubmitChoices,
    /// identity/read — /whoami (login, org).
    IdentityRead,
    /// account/read — AccountView (plan.title, planOffer, rateLimitAction, apiKey.preview).
    AccountRead,
```

**`src/proto.rs`** — nowe warianty `AcpEvent`:

```rust
    /// L1: odpowiedź setup/status; `unsupported` = method_not_found (stare CLI).
    SetupStatusReady { status: crate::snapshot::SetupStatusView },
    /// L1: wynik zapisu klucza: "completed" | "env_var_error" | "save_error" (+detail).
    SetupStoreOutcome { outcome: String, detail: Option<String> },
    /// L1: wynik submit-choices: "completed" | "provider_config_error" (+failures).
    SetupChoicesOutcome { outcome: String, failures: Vec<String> },
    /// L1: odpowiedź identity/read (tolerancyjnie: surowy JSON → string).
    IdentityReady { identity: crate::snapshot::IdentityView },
    /// L1: odpowiedź account/read.
    AccountReady { account: crate::snapshot::AccountView },
    /// L1: tura zakończona błędem (rate_limit i in.) — z wire `turn.error`.
    TurnFailed { session_id: String, code: String, message: String },
```

**`src/snapshot.rs`** — nowe struktury (wszystkie `Serialize+Deserialize`, `#[serde(default)]`
na polach opcjonalnych — parity z tolerancyjnym stylem proto) + pola w `Snapshot`:

```rust
/// `provider` z setup/status (raport §3.0; models/_setup.py).
pub struct SetupProviderView { pub name: String, pub api_base: String,
    pub api_key_env_var: String, pub browser_auth_base_url: String,
    pub browser_auth_api_base_url: String, pub browser_auth_allow_origin_rewrite: bool }
/// Seed wizarda: `setup/status`.
pub struct SetupStatusView {
    pub provider: Option<SetupProviderView>,
    pub console_base_url: String, pub vibe_base_url: String,
    pub supports_browser_sign_in: bool, pub has_api_key: bool,
    pub pinned_console_url: Option<String>,
    /// true gdy serwer nie zna metody (method_not_found) — UI degraduje się.
    pub unsupported: bool,
}
/// `AccountPlanView` (models.py:101): kind api|chat|mistral_code.
pub struct AccountPlanView { pub kind: String, pub name: String, pub title: String }
/// `AccountActionView` (models.py:96): kind switch_api_key|upgrade_to_pro + url.
pub struct AccountActionView { pub kind: String, pub url: String }
/// `AccountApiKeyView` (models.py:112); accessScope: None na starszych backendach.
pub struct AccountApiKeyView { pub preview: String, pub scope: String,
    pub access_scope: Option<String> }
/// `AccountView` (models.py:119-126) — JEDYNE źródło danych konta.
pub struct AccountView {
    pub status: String,                       // ready|missing_key|unauthorized|unavailable
    pub plan: Option<AccountPlanView>,
    pub plan_offer: Option<AccountActionView>,
    pub rate_limit_action: Option<AccountActionView>,
    pub teleport_eligible: bool,
    pub api_key: Option<AccountApiKeyView>,
    pub unsupported: bool,                    // method_not_found na starym CLI
}
/// identity/read → /whoami (commands/simple.rs:232-280); pola tolerancyjne.
pub struct IdentityView { pub raw_json: String }  // UI czyta name/email/org z surowego JSONa
```

w `Snapshot`:

```rust
    /// L1: konto (account/read) — None = jeszcze nie odpytane.
    #[serde(default)] pub account: Option<AccountView>,
    /// L1: seed logowania (setup/status) — None = nie odpytane / stary CLI.
    #[serde(default)] pub setup: Option<SetupStatusView>,
    /// L1: whoami (identity/read) — opcjonalne, do ustawień.
    #[serde(default)] pub identity: Option<IdentityView>,
```

(+ `Default` uzupełnione `None`.)

### 1.1 `crates/mistraldeck-appsrv` — transport (krok 2)

**`src/proto.rs`** — tolerant modele wire (żadnych `deny_unknown_fields`):

```rust
pub struct SetupStatusWire { pub provider: Option<Value>, pub console_base_url: Option<String>,
    pub vibe_base_url: Option<String>, pub supports_browser_sign_in: Option<bool>,
    pub has_api_key: Option<bool>, pub pinned_console_url: Option<String> }
// AccountViewWire: status, plan{kind,name,title}, planOffer{kind,url},
// rateLimitAction{kind,url}, teleportEligible, apiKey{preview,scope,accessScope}
pub struct AccountViewWire { /* jak wyżej, wszystko Option */ }
```

**`src/worker.rs`** — nowe gałęzie w `Worker::handle()` i metody (wzorzec jak
`list_sessions`: `self.call(...)` → mapowanie wire → `AcpEvent` → `self.emit`):

```rust
fn setup_status(&mut self)          // "setup/status" params {} → SetupStatusReady{unsupported gdy -32601}
fn store_credential(&mut self, provider: Option<&str>, api_key: &str, custom_domain: Option<&str>)
                                    // "setup/store-credential" → SetupStoreOutcome
fn submit_choices(&mut self)        // "setup/submit-choices" params {} → SetupChoicesOutcome
fn read_identity(&mut self)         // "identity/read" params {} → IdentityReady (raw JSON)
fn read_account(&mut self)          // "account/read" params {sessionId?} → AccountReady
```

- `read_account`: param `{sessionId}` wg raportu (`AccountView` request ma `sessionId`) —
  wysyłać `{}` gdy brak sesji; serwer przyjmuje (TUI woła po starcie sesji, my też
  odświeżamy po `SessionReady`).
- **Detekcja method_not_found**: `Worker::call` zwraca `Err` z `(code, message)`;
  rozpoznać `code == "method_not_found"` lub `-32601` (por. `is_method_not_found`,
  raport §5) → wypełnić `unsupported: true` zamiast emitować `SessionError`.
- **`reducer.rs`** — `turn/completed`: `Turn.error` zmienić z `Option<String>` na
  `Option<Value>` (wire to obiekt `{code, message}`; obecny parse po cichu daje `None`)
  i przy `error.code` niepustym emitować dodatkowo `AcpEvent::TurnFailed { code, message }`
  (istniejący `PromptCompleted` zostaje — UI nie traci stop_reason).

### 1.2 `crates/mistraldeck-engine` — flow logowania + snapshot (krok 3)

**`src/auth_browser.rs`** (NOWY plik) — PKCE po stronie serwera:

```rust
/// Stan flow przeglądowego logowania; żyje w Engine: Mutex<Option<..>>.
pub struct BrowserSignInFlow {
    pub process_id: String,
    pub sign_in_url: String,
    pub poll_url: String,
    pub expires_at_ms: i64,
    pub verifier: String,          // nigdy nie wychodzi do UI
}
/// Wynik jednego polla (klient woła co 3 s).
pub enum PollOutcome {
    Pending,
    Completed { api_key: String }, // exchange zrobiony wewnątrz
    Expired, Denied, Failed(String),
}
/// Start: verifier(64B z getrandom) → challenge=S256 → POST {base}/vibe/sign-in.
pub fn browser_sign_in_begin(browser_auth_api_base_url: &str) -> Result<BrowserSignInFlow, String>;
/// Jeden krok polla: GET poll_url (curl); "completed" → natychmiastowy exchange.
pub fn browser_sign_in_poll(flow: &mut BrowserSignInFlow) -> PollOutcome;
fn gen_verifier() -> String;                       // getrandom 64B → hex
fn base64url_no_pad(bytes: &[u8]) -> String;       // + test wektorowy
fn http_post_json(url: &str, body: &serde_json::Value, timeout_s: u64) -> Result<String, String>;
//  ← uzupełnia http_get_json z remote.rs (curl -fsSL -X POST -H "Content-Type: application/json" -d …)
```

**`src/lib.rs` (`Engine`)**:

```rust
// stan
signin_flow: Mutex<Option<BrowserSignInFlow>>,
last_setup: Mutex<Option<SetupStatusView>>,   // seed dla snapshotów
last_account: Mutex<Option<AccountView>>,

// API (wołane z server-fn; wysyłają AcpCommand do listera / operują na flow)
pub fn setup_status(&self);                       // → AcpCommand::SetupStatus
pub fn login_browser_start(&self) -> Result<(sign_in_url, expires_at_ms), String>;
    // 1. jeśli last_setup brak → SetupStatus najpierw; 2. begin() na endpointach z provider
pub fn login_browser_poll(&self) -> PollOutcome;  // poll co wywołanie; po Completed → store_credential + submit_choices automatycznie
pub fn login_with_api_key(&self, key: &str);      // → AcpCommand::SetupStoreCredential + (po outcome completed) SetupSubmitChoices
pub fn refresh_account(&self);                    // → IdentityRead + AccountRead (po SessionReady też)
```

- pompa eventów (`drain_*` istniejącej pętli) obsługuje nowe `AcpEvent::SetupStatusReady |
  SetupStoreOutcome | SetupChoicesOutcome | IdentityReady | AccountReady | TurnFailed`:
  zapis do `last_setup/last_account`, `StructureChanged`/`Acp` na bus; przy
  `SetupStoreOutcome.completed` → automatyczny `SetupSubmitChoices`.
- `snapshot()` dokłada `account: last_account.clone()`, `setup: last_setup.clone()`,
  `identity`.
- **Gating**: przy `snapshot()` gdy `setup.has_api_key == false` (i `!unsupported`) →
  `Snapshot.status` zostaje, ale UI decyduje na podstawie `setup`/`account.status` —
  bez wymuszania w engine (logika widoku w `app.rs`, wzorzec jak `setup_needed`).
- `start_agent()` bez zmian — lister i tak startuje; `setup/status` wołać zaraz po
  `Initialized` (pierwszy snapshot wywoła `setup_status()` gdy `last_setup.is_none()`).

**`Cargo.toml`**: `getrandom = "0.2"` (+ ewent. `base64 = "0.22"` zamiast ręcznego encodera).

### 1.3 `crates/mistraldeck/src/api.rs` — server-fn (krok 4)

```rust
#[server] pub async fn api_setup_account_status(token: String)
    -> Result<(), ServerFnError>;            // trigger: SetupStatus + (po załadowaniu) Account/Identity
#[server] pub async fn api_login_start(token: String)
    -> Result<LoginStartView, ServerFnError>; // {sign_in_url, expires_at_ms}
#[server] pub async fn api_login_poll(token: String)
    -> Result<String, ServerFnError>;        // "pending"|"completed"|"expired"|"denied"|error
#[server] pub async fn api_login_with_api_key(token: String, api_key: String, custom_domain: Option<String>)
    -> Result<(), ServerFnError>;
```

UI obserwuje wyniki przez `Snapshot` (`setup`, `account`, `identity`) — zgodnie z
istniejącym modelem „snapshot + StructureChanged", bez nowych kanałów.

### 1.4 `crates/mistraldeck/src/components` — UI (krok 5)

**`src/components/login.rs`** (NOWY) — wizard logowania, ekrany jak TUI
(`setup/wizard/mod.rs:29-37`):

```rust
pub fn LoginView() -> Element;      // faza: method → browser | apikey (+ error)
// wewnętrznie:
//  - ekran wyboru metody: przeglądarka (jeśli supports_browser_sign_in) / klucz API
//  - BrowserSignIn: stepper OpenBrowser → Waiting → Exchanging → Complete;
//    poll w use_future z gloo-timers co 3 s aż expires_at; window.open(sign_in_url);
//    fallback "skopiuj URL" (parity CopySignInUrl); expired/denied → ekran błędu z retry
//  - ApiKey: input (type=password) + walidacja pustości; outcome env_var_error/
//    save_error → komunikat z detail; completed → refresh_snapshot
//  - stary CLI (setup.unsupported) → informacja + instrukcja ręczna (env var) — brak wizarda
```

**`src/app.rs`** — gating startowy:

```rust
// po !s.loaded i setup_needed, PRZED Shell:
if let Some(setup) = &s.snapshot.setup {
    if !setup.unsupported && !setup.has_api_key { return rsx! { LoginView {} }; }
}
```

**`src/components/header.rs`** — banner konta (parity `ui/banner/mod.rs`):
linia pod/obok statusu z `account.plan.title` (np. „[Subscription] Pro") gdy
`account.status == "ready"`; `planOffer` → drobny link-CTA (otwiera `planOffer.url`).

**`src/components/chat.rs`** (+ ewent. `components/ratelimit.rs`) — obsługa rate limit:

- `AcpEvent::TurnFailed` → czerwony baner nad inputem: `message` z błędu tury;
  gdy `code == "rate_limit"` **i** `account.rate_limit_action` obecne → tekst
  podstawiony pod kąt akcji (`upgrade_to_pro` → link CTA do
  `rateLimitAction.url`, `switch_api_key` → przejście do LoginView/klucz).
  Parity: `rateLimitAction != null` = serwer uznał upgrade za sensowny (plan API /
  Mistral Code) — planu CHAT Pro to nie dotyczy (nie pokazywać CTA Pro posiadaczom Pro).
- banyer retry (`turn/retrying` / `snapshot.retrying`) — **L2** (patrz §2), L1 zostawia.

**`src/components/settings.rs`** — nowa sekcja „Konto": `account.status`, plan,
`apiKey.preview` (+ `scope`/`accessScope` gdy obecne), przycisk „Odśwież"
(`api_setup_account_status`), whoami z `identity` (login/e-mail z raw JSON).
`customDomain` tylko za polem zaawansowanym (pinnedConsoleUrl blokuje).

### 1.5 `crates/mistraldeck/src/i18n.rs` — stringi PL/EN (krok 6)

Nowe klucze: login method/browser stepper/api-key outcomes, account banner, rate
limit CTA, settings-account. (Wzorzec `tr/tf` istnieje.)

### 1.6 Testy L1

- **Jednostkowe (`mistraldeck-appsrv`)**: parse `setup/status` i `AccountView` z
  fixture'ów JSON (warianty: planOffer/rateLimitAction null; `accessScope` brak —
  „None on older backends"); `method_not_found` → `unsupported: true`;
  `turn/completed` z `error: {code:"rate_limit", message}` → `TurnFailed`.
- **Jednostkowe (`mistraldeck-engine`)**: `base64url_no_pad` (wektory RFC),
  challenge = SHA256(verifier) zgodność z TUI (`sign_in_flow.rs:137-150`),
  mapowanie statusów polla, expired po `expires_at`.
- **E2E (ręcznie / skrypt markerowy, patrz §7)**: pełny wizard, rate-limit, banner.

**Szacunek L1: 3–5 dni.** Ryzyko najwyższe w flow przeglądowym (endpoints live) —
sonda `curl`-owa przed pisaniem UI (jak F0).

---

## 2. Etap L2 — następny build: kolejka + steer + retry

- **Transport** (`appsrv`): `AcpCommand::QueueReplace|QueueRemove|QueueSteer|QueueResume|
  QueueEditStart|QueueEditSave|QueueEditCancel|QueueEditAbandon` + `AcpCommand::Steer` →
  metody `session/turn/queue/*` i `turn/steer` (expectedTurnId — walidacja `STALE_TURN`/
  `NOT_STEERABLE`); reduktor: notyfikacja `turn/queueUpdated` → `AcpEvent::QueueUpdated`
  (obecnie `turnQueue` trzymany jako surowy `Value` — dodać model `PublicTurnQueue`),
  `turn/retrying` + `session/snapshot.retrying` (`PublicRetryState`) →
  `AcpEvent::Retrying { attempt, message }`.
- **Core**: `Snapshot.queue: Vec<QueueItemView>` (prompt, pozycja, stan edycji).
- **UI**: `components/queue.rs` — lista oczekujących promptów pod inputem
  (edycja inline przez edit/start→save, usuwanie, „wyślij teraz" = steer); baner
  „retrying…" nad transkryptem; kompozytor przechwytuje retry-owy prompt (jak TUI
  przy rewind — tu: po `retrying` nie blokować inputu).
- **AC**: prompt dodany w trakcie tury widoczny na liście; edit/remove/steer działa;
  przerwany connection → po reconnect kolejka odtworzona z snapshotu.

## 3. Etap L3: checkpointy + subagenci + lease

- **Rewind/fork**: `AcpCommand::RewindRead|Rewind|Fork` → `session/rewind/read`
  (`hasFileChanges`, `paths`), `session/rewind` (`entryId`, `restoreFiles`, `inplace` —
  wynik: przywrócony prompt wraca do kompozytora), `session/fork` (`entryId`/`afterTurnId`,
  `attach`) → nowa sesja w sidebarze. UI: menu na wpisie historii („cofnij tutaj",
  „przywróć pliki", „utwórz gałąź"); wpisy `checkpoint` w historii (raport §1.6).
- **Subagenci**: `PublicSessionState.childSessions` + `session/childSessionUpdated`
  → `AcpEvent::ChildSessionUpdated`; UI: docked lista z postępem/token_usage, wejście
  w transkrypt dziecka (resume po `session_id`). `session/subagents/*` zostawiamy
  (headless-only, experimental). Nie deklarujemy `sessionSpawn` capability (GUI v1
  nie obsługuje server-request `session/spawn`).
- **Lease**: `session/lease/subscribe` + `session/lease/updated|ended` → badge
  „sesja otwarta gdzie indziej" w sidebarze (blokada resume edycji).

## 4. Etap L4 (opcjonalne)

- **Loops**: `loops/list|create|delete|clear` (TUI tego nie woła — GUI może pierwszy;
  `ScheduledLoop`: `id`, `prompt`, `intervalSeconds`|`cron`, `nextFireAt`); fajerwerki
  i tak wpływają jako notice `ScheduledLoopFiredNoticeDetail`.
- **Worktrees**: `AgentConfig.worktree` w `session/start` (create|auto|existing) +
  `workspace/git/worktrees/remove` (outcome/holders) — dialog przy starcie i sprzątaniu.
- **MCP/connectors UI**: `mcp_catalog/*`, `connector_catalog/*`, `connectors/auth/read`;
  wymaga dopisania capability `connector_auth` + handlera notyfikacji `mcp_catalog/authUrl`.
- **Teleport**: `vibeCode/projects/*` + `vibeCode/teleport/start` + eventy.
- **Artifacts**: `artifacts/list` + `artifacts/created|updated` (prStatus, checks).

## 5. Ryzyka i feature detection

| Ryzyko | Mitygacja |
|---|---|
| **app-server = API wewnętrzne** (`setup/*`, `account/read` mogą się zmienić bez deprecation) | tolerancyjne modele wire (wszystko `Option`), pin wersji CLI w testach, sonda curl przed każdym release'em |
| **Starsze CLI**: `method_not_found` na `setup/*`, `account/read`, `identity/read` | flagi `unsupported` w widokach → wizard ukryty, banner „konto: nieznane", instrukcja ręczna (env var). Wzorzec TUI: `is_method_not_found` (raport §5) |
| **`apiKey.accessScope` absent** na starszych backendach | pole `Option`, nie blokuje renderu preview |
| **Flow przeglądowy**: expires_at, denied, 403/410/9004, 3 błędy sieci | mapowanie 1:1 z `sign_in_flow.rs`; każdy status ma ekran; timeout klienta = `expires_at` z serwera |
| **Zamknięcie GUI w trakcie flow** — verifier ginie | flow jednorazowe: ponowny start wizarda robi nowy POST `/vibe/sign-in` (stary process_id wygasa sam) |
| **Zdalne sesje SSH**: klucz zapisywany na hoście serwera, nie klienta | `store-credential` przez HTTPS listener dotyczy hosta z agentem — jasno opisać w UI („klucz zostanie zapisany na …"); rozważyć `require_local()` jak w `api_set_remote` |
| **`pinnedConsoleUrl`** blokuje własną domenę | wizard czyta pole z `setup/status` i nie oferuje CustomDomain, gdy pin ustawiony |
| **whoami bezpośrednio (`GET {console}/api/vibe/whoami`)** jako dodatek do „stanu Pro" | patrz niżej — **nie robić w L1** |
| Rate limit: `Turn.error` to obiekt, nie string | zmiana typu w `proto.rs` + test (cicha utrata błędu przy obecnym kodzie) |
| Wallet creep: pokazanie kwot Pro, których nie ma na wire | zakazane zasadą §0.1; UI tylko z pól `AccountView` |

**O ryzyku bezpośredniego whoami (opis — na życzenie, niewdrażane w L1):**
serwer i tak robi `GET {console}/api/vibe/whoami` z 6-godzinnym cache per klucz
(`whoami.py:117`), a jedyne dane ponad `account/read` to `plan_type`,
`prompt_switching_to_pro_plan`, `organization_kind`, `api_base`/`vibe_base`.
Klient musiałby: (1) znać `consoleBaseUrl` (z `setup/status`, albo honourować
`pinnedConsoleUrl`), (2) mieć klucz w czystej postaci — a ten po zapisie siedzi w
keyringu / `$VIBE_HOME/.env` hosta serwera, GUI by musiało czytać `.env` z dysku
(fragilne, omija abstrakcję), (3) polegać na wewnętrznym endpointcie, który nie
jest częścią protokołu i może zmienić kształt bez powiadomienia. Korzyść marginalna
(niczego nie ma w whoami, czego nie da `account/read` poza sygnałem
`prompt_switching_to_pro_plan`). Decyzja: **preflow przez `account/read`**; whoami
bezpośrednio dopiero, gdy serwer doda pola do `AccountView` albo powstanie realna
potrzeba `plan_type` w UI.

## 6. Co testować — scenariusze E2E (na realnym `vibe-app-server`)

**L1:**
1. **Cold start bez klucza**: start GUI → `setup/status` (`hasApiKey=false`) →
   LoginView zamiast Shellu; wybór „przeglądarka".
2. **Login przeglądarkowy happy path**: POST sign-in → `sign_in_url` otwarty →
   poll `pending`×N → `completed` → exchange → `store-credential completed` →
   `submit-choices completed` → Shell, banner z `plan.title`.
3. **Login przeglądarkowy — denied / expired**: poll zwraca `denied`/`expired`
   (oczekiwanie > expires_at) → ekran błędu, retry tworzy nowy proces.
4. **Login kluczem**: paste ważnego klucza → `completed`; nieważnego →
   `save_error`/`env_var_error` z detail, bez crasha; stary CLI (symulacja
   `method_not_found`) → wizard ukryty, komunikat + instrukcja env.
5. **Rate limit**: wymuszony `turn.error.code=="rate_limit"` (plan API pod limitem)
   → baner z CTA wg `rateLimitAction`; brak `rateLimitAction` (plan chat Pro) →
   sam komunikat, bez CTA.
6. **Banner + settings**: po zalogowaniu `plan.title` w headerze; ustawienia
   pokazują `apiKey.preview` (maskowanie), scope, whoami; „Odśwież" działa.
7. **Reconnect/restart**: po restarcie GUI (proces nowy) account/status odtworzony
   z `account/read`; snapshot nie traci sesji.

**L2**: enqueue w trakcie tury → widoczny w kolejce; edit-save zmienia treść;
remove usuwa; steer w trakcie tury (błąd `NOT_STEERABLE` przy quiescent — obsłużony);
`turn/retrying` → baner; reconnect w trakcie retry → spójny stan.

**L3**: rewind z `restoreFiles=true` przywraca pliki + prompt wraca do inputu;
fork tworzy sesję siostrę; childSessions live-update; lease badge przy otwarciu
tej samej sesji w oficjalnym TUI.

## 7. Procedura weryfikacji (wspólna)

1. Sonda curl-owa `setup/status` + `account/read` na zainstalowanym CLI (jak F0)
   przed startem prac L1 — potwierdzenie kształtu wire na używanej wersji.
2. `cargo test -p mistraldeck-appsrv -p mistraldeck-engine` (fixtury JSON z sondu).
3. Smoke GUI: login → sesja → prompt → banner/rate-limit na buildzie dev (tag next).
4. Porównanie z oficjalnym TUI: te same dane konta w `/whoami` co w naszym settings.
