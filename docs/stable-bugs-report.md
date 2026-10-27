# Raport regresji: poprawki ze stabilnej (origin/main, Electron) vs branch `next` (Rust)

Data analizy: 2026-10-27 07:52 UTC. Okno: ostatnie 24 h stabilnej ≈ v0.13.5 → **v1.0.19**
(komity `bef87ae`…`48c1738`), z naciskiem na listę priorytetową (v0.13.15, v1.0.1–v1.0.19).

Metoda: `git show <sha>` / `git show origin/main:CHANGELOG.md` dla stabilnej; czytanie
kodu `next` z lokalnego checkoutu (crates/mistraldeck*, branche semantyczne, nie nazwy plików).

Legenda: **TAK** = błąd obecny w next, **CZĘŚCIOWO** = częściowo zabezpieczone / inny kształt tej samej luki, **NIE** = nie dotyczy lub już poprawione w next.

---

## 1. Tabela: bugfixy stabilnej → status w next

| Wersja / commit (stabilna) | Bugfix | next dotknięty | Plik(i) w next | Sugerowana naprawa |
|---|---|---|---|---|
| v1.0.5 / `45d7ce8` | Refuse app/CLI updates while an ACP turn is in flight | **TAK** | `crates/mistraldeck/src/api.rs` (`api_update_app`, `api_upgrade_local_cli`), `crates/mistraldeck-engine/src/updates.rs` (`update_app`) | Dodać `Engine::any_turn_in_flight()` (iteracja `state.slots`, `slot.busy`), odrzucać update gdy true — patrz §3.1 |
| v1.0.4 / `815354b` | Reject pending ACP requests when the worker dies mid-turn | **CZĘŚCIOWO** | `crates/mistraldeck-acp/src/client.rs` (`request`, stdout reader) | Fail-fast istnieje (poll `exited()` co 50 ms); dodać drain `pending` przy `RawMsg::StdoutClosed` — patrz §3.2 |
| `5ab6b5a` | Never route a prompt into another session | **NIE** | — | next nie ma fallbacku „ostatnia aktywna sesja"; prompt zawsze trafia do slotu bieżącej sesji (`send_message_with` → `slot.pending_prompt`, wysyłka z `session_id` w `cmd_prompt`) |
| `a4bcf51` | Fix first prompt going to an invisible session on a fresh install | **NIE** | — | next tworzy i publikuje sesję serwerowo: `send_message_with` → `create_local_session()` → `BusEvent::StructureChanged` (lib.rs ~660, ~314); klient dostaje event przez stream |
| `faa86ac` | Fix dock and sidebar resizing with pointer events | **TAK** | `crates/mistraldeck/src/components/shell.rs` (`RESIZE_JS`, ~238–288), `crates/mistraldeck/assets/main.css` (`.sidebar-resize-handle`, `.dock-resize-handle`) | Zamienić mouse events na pointer events + `setPointerCapture`/`pointercancel`, `touch-action:none`, szerszy hit-area — patrz §3.3 |
| `d551ecd` | Fix tool-approval dialog layout on desktop and mobile | **CZĘŚCIOWO** | `crates/mistraldeck/assets/main.css` (`.tool-approval-*`, 3854–3935), `crates/mistraldeck/src/components/shell.rs` (`approval_el`) | Krótka nazwa + mono chip już są; brak: `overflow-wrap` zamiast `nowrap+ellipsis` dla długich komend oraz mobilny `@media` (full-width bar, zawijanie przycisków) — patrz §3.4 |
| v0.13.15 / `bef87ae` (headless/web ×10) | progress update'ów w headless, binary git diff → preview, replay guard, sendQueuedNow, thinking levels, web update, sidebar 🆕 | **CZĘŚCIOWO** (patrz §3.5) | gł. `crates/mistraldeck/src/state.rs` (`drain_queue`), reszta NIE | Większość nie dotyczy (serwerowy model snapshotów); realna resztka: kolejka przy zawieszonym `busy` — patrz §3.5 |
| v1.0.6 / `f54ff21` | Auto-approval follows the session's built-in mode (yolo) | **TAK** | `crates/mistraldeck-engine/src/lib.rs` (~1595, `AcpEvent::ToolApprovalRequest`), `crates/mistraldeck-acp/src/client.rs` | Gdy `current_mode_id` sesji == wbudowany auto-approve (`"yolo"`/lokalizowany odpowiednik — por. `lib.rs:1935` `"auto-approve"`), zatwierdzać samemu zamiast surfacingu — patrz §3.6 |
| v1.0.8 / `6f726b8` | Images actually reach the LLM (ACP image content blocks) | **TAK** | `crates/mistraldeck-acp/src/client.rs` (`cmd_prompt`, ~751), `crates/mistraldeck/src/api.rs` (`resolve_attachments`, ~219) | `cmd_prompt` wysyła wyłącznie `[{type:"text"}]`; dodać bloki obrazów base64+mime — patrz §3.7 |
| v1.0.9 / `dc3dacf` | SSH session list hang on first launch (host-key prompt) | **TAK** | `crates/mistraldeck-engine/src/remote.rs` (`ssh_run`, ~68), `remote_vibe.rs` (spawn ssh) | Dodać `-o StrictHostKeyChecking=accept-new -o ConnectTimeout=10` do każdego spawnu ssh — patrz §3.8 |
| v1.0.10 / `f3cc789` | Attachments ENOENT (`fs:write-binary` bez mkdir) | **NIE** | — | next nie zapisuje uploadów do `/tmp` przy resolve (read-only `resolve_attachments`); upload już robi `create_dir_all` (`api.rs:845`) |
| v1.0.10 / `f3cc789` | Message sent in one browser never appears on other clients | **NIE** | — | next broadcastuje serwerowo: `BusEvent::UserMessageAdded` (`lib.rs:674`), wszyscy klienci dostają przez event stream |
| v1.0.10 / `f3cc789` | PWA staleness (SW cache purge, 512px icon, `id`) | **CZĘŚCIOWO** | `crates/mistraldeck/src/pwa.rs` (MANIFEST, SW) | Zweryfikować, czy manifest ma `icons[512]` (any+maskable) i stabilne `id`; SW czyści stare cache przy activate — patrz §3.9 |
| v1.0.11 / `47c557b` | Joining a running session showed only new messages | **NIE** | — | next ładuje sesje z dysku (preview) przed dołączeniem; snapshot daje pełną historię niezależnie od streamu |
| v1.0.12 / `e486122` | Chrome could not install the PWA (manifest 401) | **NIE** | — | `serve.rs::remote_auth` (~286) zwalnia `/manifest.webmanifest`, `/sw.js`, `/icon-256.png` spod auth (komentarz „Nexus serves them pre-auth too") |
| v1.0.13 / `0b7d06a` | Reopened PWA saw running turns as idle (`busyTurns`, `acp:busy-sessions`) | **NIE** | — | next jest snapshotowy: `busy` per sesja leci w `api_snapshot` (lib.rs ~236) i po każdym reconnect klient robi pełny `refresh()` (shell.rs ~36) |
| v1.0.14 / v1.0.15 / v1.0.18 | Back gesture: LIFO nav stack, pushBack na każdym wejściu, double-back-to-exit, guard `/login` | **TAK** | `crates/mistraldeck/src/components/mobile.rs`, `shell.rs`, `state.rs` (View) | Zero `pushState`/`popstate` w kliencie web — patrz §3.10 |
| v1.0.16 / `9f86dc5` | „Resume failed: Connection closed" — auto-retry resume (4×, 3 s) | **CZĘŚCIOWO** | `crates/mistraldeck-engine/src/lib.rs` (~1385 `AcpEvent::SessionError`) | Load-failure ma fallback na świeżą sesję (respawn z `pending_prompt`), ale brak retry proper resume przy chwilowym hiccupie transportu; `pending_prompt` z `load_session` fallbackiem ginie przy błędzie innym niż load — patrz §3.11 |
| v1.0.17 / `438f57d` | Returning to the PWA shows stale data (full re-sync on reconnect + visibility) | **CZĘŚCIOWO** | `crates/mistraldeck/src/components/shell.rs` (~30–41) | Reconnect → pełny `refresh()` JEST; brak handlera `visibilitychange` i brak `drain_queue` po reconnect-refresh — patrz §3.12 |
| v1.0.19 / `48c1738` | Serving silently dies after an in-app update (systemd orphan, EADDRINUSE) | **TAK** | `crates/mistraldeck-engine/src/updates.rs` (`update_app_linux`, ~766), `crates/mistraldeck/src/serve.rs` (`bind`, ~551) | Linux relaunch przez systemd unit gdy aktywny; pidfile + recovery z EADDRINUSE w serve — patrz §3.13 |
| v1.0.3 / `f2eecd9` | Windows self-update black terminal (`detached` → brak `CREATE_NO_WINDOW`) | **TAK** | `crates/mistraldeck-engine/src/updates.rs` (~673: `creation_flags(0x0000_0008)` — tylko `DETACHED_PROCESS`) | Dodać `CREATE_NO_WINDOW (0x0800_0000)`: `creation_flags(0x0800_0008)` — patrz §3.14 |
| v1.0.3 / `f2eecd9` | „Update" from browser in headless mode | **NIE** | — | `api_update_app` w next nie ma web-blokady (tylko `check(&token)`); postęp pollingiem `api_update_progress` działa bez okna |
| v1.0.7 / `ae0aab3` | Boot prime: brak auto-otwierania sesji; chipsy z configu przed połączeniem | **NIE / design różny** | — | next nie tworzy sesji przy bootcie (sesja powstaje w `send_message_with`/`create_local_session` na żądanie); wybory model/mode aplikowane przy wake sesji (`pending_options`, lib.rs ~790) |
| `722279a`–`c865545` (v0.13.14), `f6398d0` | Send-now na zakolejkowanych promptach; always-visible usage bar; restore planu na resume | **CZĘŚCIOWO** | `crates/mistraldeck/src/state.rs` (`queue`, `drain_queue`), `components/input.rs` | Kolejka jest, ale **brak przycisku „wyślij teraz"** i brak send-now dla sesji idle — patrz §3.5 |
| v0.13.12 / `a769afd` | Resume races: replay guard, Enter guard, hung-worker probe | **CZĘŚCIOWO** | `crates/mistraldeck-acp/src/client.rs` (`replaying`), `lib.rs` (cancel-watchdog ~831, ~2142) | Replay jest serwerowy (bez wycieków między sesjami); jest watchdog cancelu; brak proby „hung worker" dla samego promptu (timeout 1 h `REQUEST_TIMEOUT`) — patrz §3.11 |
| v0.13.11 / `e2e4aac` | Duplicate sessions on failed resume; stuck busy on worker death | **NIE (gł.)** | — | „Stuck busy" rozwiązany: `AcpEvent::SessionError` czyści `busy`/`stopping` (lib.rs ~1385); duplikatów sesji nie ma (slot per session-key, dedupe po `acp_session_id`) |

---

## 2. Weryfikacja listy priorytetowej (punkt po punkcie)

- **„Refuse app/CLI updates while an ACP turn is in flight"** → **TAK**, brak w next. Szczegóły §3.1.
- **„Reject pending ACP requests when the worker dies mid-turn"** → **CZĘŚCIOWO**: semantyka fail-fast już jest (inna architektura: blocking `request()` zamiast promise'ów), ale drain pending map przy zamknięciu stdout nie istnieje. §3.2.
- **„Never route a prompt into another session"** → **NIE**: `send_message_with` (crates/mistraldeck-engine/src/lib.rs ~635) routuje wyłącznie do bieżącej sesji (`st.current_session_mut()`), prompt leci z explicite `session_id` (`cmd_prompt`, client.rs ~751); brak fallbacku „worker's last active session". Race SessionInitialized-vs-reply nie istnieje, bo `pending_prompt` trzymany jest w slocie sesji i wysyłany po `SessionReady` (lib.rs ~1328, ~1399).
- **„Fix first prompt going to an invisible session on a fresh install"** → **NIE**: przy `current.is_none()` next tworzy sesję i publikuje `StructureChanged` (lib.rs ~659), więc pierwsza wiadomość ma widoczną sesję.
- **„Fix dock and sidebar resizing with pointer events"** → **TAK**: `RESIZE_JS` w shell.rs używa dokładnie wzorca mouse-only. §3.3.
- **„Fix tool-approval dialog layout"** → **CZĘŚCIOWO**: chip `.tool-approval-input` jest, ale bez zawijania i bez mobile media query. §3.4.
- **v0.13.15 headless/web ×10** → przeważnie **NIE** (serwerowy model nextu eliminuje klasę bugów renderer-side); resztka: kolejka/stale-busy, thinking-levels picker, send-now. §3.5.
- **Kolejka promptów** → istnieje (`submit` → `queue`, `drain_queue`), ale bez „send now" i bez odporności na stale `busy`. §3.5.
- **Reconnect** → snapshot-refresh po reconnect jest; brak `visibilitychange` i drain kolejki. §3.12.
- **UI fixy (layout/mobile/scroll)** → tool-approval §3.4, back-gesture §3.10, resize §3.3.

---

## 3. Wskazówki implementacyjne dla pozycji TAK/CZĘŚCIOWO

### 3.1 Update zabijający trwający turn (v1.0.5) — **TAK, WYSOKI**

Stabilna: `updates:update-cli` ubijał workerów, `update-app` robił `app.exit(0)`/taskkills — bez sprawdzenia turna.

Next: `crates/mistraldeck/src/api.rs`:

```rust
pub async fn api_update_app(token: String) -> ... {
    check(&token)?;
    tokio::task::spawn_blocking(mistraldeck_engine::updates::update_app)  // brak checku busy
```

`api_upgrade_local_cli` (~422) — j.w. W `crates/mistraldeck-engine/src/updates.rs::update_app()` (453) jedyny guard to `UPDATE_STATE.running` (równoległy update, nie turn).

Naprawa: dodać w `crates/mistraldeck-engine/src/lib.rs` metodę `Engine::any_turn_in_flight() -> bool` (lock `state`, `st.slots.values().any(|s| s.busy)`; sloty per sesja lokalna i zdalna — pokrywa też remote). Wywołać na początku `api_update_app` i `api_upgrade_local_cli`, zwracając `AppUpdateResult { ok:false, error: Some("A turn is running — update refused") }` (i analogiczny `CliUpgradeResult::error`).

### 3.2 Worker death → pending requests (v1.0.4) — **CZĘŚCIOWO, ŚREDNI**

Next w `crates/mistraldeck-acp/src/client.rs::request()` (274–310) co 50 ms sprawdza `exited()` i natychmiast zwraca `bail!("vibe-acp exited while handling {method}")` — to daje ten sam efekt „fail fast" co poprawka stabilnej (reject zamiast czekania na timeout). Różnice:

1. Reader thread przy EOF wysyła tylko `RawMsg::StdoutClosed` (client.rs ~211) i **nie opróżnia** `pending` mapy — requesty „wiszą" w mapie do czasu pollu w `request()` — dla blokującego klienta to zawsze ktoś czeka, więc semantycznie OK; drain porządkuje jednak stan przy nieoczekiwanej śmierci w trakcie replayu (`drain_raw`).
2. Timeout promptu to `REQUEST_TIMEOUT` (do 1 h) — jeśli turn stoi, a worker żyje ale zawisł, next nie ma proby (v0.13.12 „hung-worker probe" ze stabilnej). `SessionError` czyści `busy` (lib.rs 1385), więc po faktycznej śmierci UI się odblokowuje.

Naprawa (koszt niski): w reader thread po `break` zrobić drain: `for (_, tx) in pending_reader.lock().drain() { let _ = tx.send(Err(anyhow!("ACP worker exited before responding"))); }` przed `RawMsg::StdoutClosed`. Opcjonalnie: probe „hung worker" w engine (slot `busy` > N min bez eventów → Stop + respawn, jak stabilna v0.13.12).

### 3.3 Resize handle tylko mouse events (faa86ac) — **TAK, WYSOKI (web/mobile)**

`crates/mistraldeck/src/components/shell.rs`, `RESIZE_JS` (~238–288):

```js
document.addEventListener('mousedown', ...)   // arm()
function move(e) { apply(startX - e.clientX); }        // mousemove
document.addEventListener('mouseup', up);               // brak pointercancel/pointer capture
```

Identyczne trzy symptomy co w stabilnej: dotyk/PWA nie działa (mouse events nie odpalają się dla touch), zgubione `mouseup` poza oknem → panel utkwiony w trybie resize, eventy przełapane przez iframe. Handle'y w `components/dock.rs:519` i sidebar nie mają `touch-action:none`.

Naprawa: przepisać `arm`/`drag` na `pointerdown` + `el.setPointerCapture(e.pointerId)` + `pointermove`/`pointerup`/`pointercancel`; dodać w `crates/mistraldeck/assets/main.css` do `.sidebar-resize-handle`/`.dock-resize-handle`: `touch-action:none;` i poszerzony pseudo-element hit-area (np. `::after { width:12px; margin:-6px }`). Opcjonalnie fallback domyślnej szerokości doku gdy nic nie zapisano (next ma defaulty w `load()`, więc OK).

### 3.4 Tool-approval layout (d551ecd) — **CZĘŚCIOWO, NISKI**

Next ma już split nazwa+chip: `.tool-approval-name` + `.tool-approval-input` (mono, 11px) w `assets/main.css` (3854–3935) oraz `flex-wrap:wrap` inline na akcjach (shell.rs:133). Brakujące elementy poprawki stabilnej:

- `.tool-approval-input` używa `overflow:hidden; text-overflow:ellipsis; white-space:nowrap` — stabilna celowo zamieniła to na zawijanie (`overflow-wrap:anywhere`), bo ucinanie komendy ukrywa co dokładnie aprobujesz;
- brak `min-width`/`max-width` clampów z `width:max-content` (są: `min-width:360px; max-width:640px` — OK) oraz **brak `@media (max-width:768px)`** dla `.tool-approval-bar` (full-width, mniejszy bottom offset nad klawiaturą).

Naprawa: w main.css `.tool-approval-input { white-space:normal; overflow-wrap:anywhere; }` + blok `@media (max-width:768px){ .tool-approval-bar{ left:12px; right:12px; transform:none; min-width:0; width:auto; } }`.

### 3.5 v0.13.15 (bef87ae) i kolejka promptów — **CZĘŚCIOWO, NISKI/ŚREDNI**

Rozbiór dziesięciu poprawek vs next:

| Fix v0.13.15 | next |
|---|---|
| updates:progress broadcast w headless | NIE — postęp jest polingowany (`api_update_progress`), działa bez okna |
| binary git diff → file preview (GitPanel) | do sprawdzenia w `crates/mistraldeck/src/fsgit.rs` / `components/history.rs` — jeśli next pokazuje diff binarny jako tekst, przenieść heuristicę `BINARY_EXTS`/„Binary files differ" |
| replay guard (`setPendingLoadSessionId`) | NIE — replay jest serwerowy w `client.rs` (`replaying` map → `SessionHistory`), bez wycieków między sesjami |
| user message znikający po nieudanym resume | CZĘŚCIOWO — `pending_prompt` istnieje, ale przy błędzie innym niż `load_session` jest kasowany (lib.rs ~1409); patrz §3.11 |
| attach menu bez katalogów | decyzja UX, nie bug — do oceny produktowej |
| web może update'ować apkę | NIE — brak web-blokady w `api_update_app` |
| 🆕 widoczny bez scrollowania (sidebar header) | decyzja layoutu mobile — sprawdzić `components/sidebar.rs` |
| `sendQueuedNow` dla sesji idle | **TAK-owy fragment** — next: `drain_queue` (state.rs ~515) wysyła tylko przy `!busy`; przy stale `busy` (martwy worker bez SessionError) wiadomość siedzi w kolejce wiecznie |
| thinking levels: nie nadpisuj live sesji; tylko jawne listy | do sprawdzenia w pickerze (`components/header.rs`/`settings.rs`) — ta sama zasada |

Naprawa kolejki: w `drain_queue` dodać warunek „busy dłużej niż X bez żadnego eventu → wymuś send" albo dodać przycisk „wyślij teraz" przy wpisie kolejki (UI: `components/input.rs` render kolejki), który czyści `queue` i woła `api_send_message` mimo `busy` (odpowiednik `sendQueuedNow` z v0.13.15/f6398d0).

### 3.6 Auto-approval wg trybu wbudowanego (v1.0.6) — **TAK, ŚREDNI**

Next: `AcpEvent::ToolApprovalRequest` ZAWSZE ląduje w `slot.approval` (lib.rs ~1595) i jest surfacowane w UI (shell.rs `approval_el`). Jest za to parser `current_mode_id` (client.rs `parse_new_session`, lib.rs ~1242/1280), więc dane są dostępne.

Naprawa: przy obsłudze `ToolApprovalRequest` w slocie sprawdzić zapisany `current_mode_id` (oraz wynik `set_config_option("mode")`), i gdy tryb == wbudowany auto-approve (`"yolo"`/„auto-approve" — por. `lib.rs:1935` i zlokalizowane odpowiedniki), wysłać approve automatycznie (`AcpCommand::Approve`) zamiast ustawiać `slot.approval`.

### 3.7 Obrazy nie docierają do LLM (v1.0.8) — **TAK, WYSOKI (parities funkcjonalne)**

`crates/mistraldeck-acp/src/client.rs::cmd_prompt` (~751): `prompt: [ { "type": "text", "text": text } ]` — wyłącznie tekst. `resolve_attachments` (api.rs ~219) inlinuje pliki jako tekst (MAX 256 KB); obraz odczytany jako tekst jest bezużyteczny dla modelu.

Naprawa: (1) w `mistraldeck-core::types::Attachment` dodać `mime` + `data_base64` (lub `kind: Image`); (2) w `resolve_attachments` rozpoznawać PNG/JPEG/GIF/WebP, czytać binarnie, kodować base64 (limit 8/prompt, 10 MB, jak CLI contract); (3) `cmd_prompt` budować bloki `{"type":"image","data":...,"mimeType":...}` (alias z nazwy pliku); (4) UI (input.rs) walidacja i banner gdy model bez `supports_images` (por. v1.0.8 vision-capability detection).

### 3.8 SSH: host-key prompt zawiesza pierwsze połączenie (v1.0.9) — **TAK, ŚREDNI**

`crates/mistraldeck-engine/src/remote.rs::ssh_run` (~68) buduje `Command` z tokenów `conn.ssh_cmd` i **nie dokłada żadnych `-o`**; to samo w `remote_vibe.rs` (spawn vibe-acp przez ssh). Pierwszy kontakt z nowym hostem wisi na interaktywnym pytaniu o host-key aż do timeoutu.

Naprawa: po `tokenize_cmd` dokleić `arg("-o").arg("StrictHostKeyChecking=accept-new").arg("-o").arg("ConnectTimeout=10")` przed `remote_cmd` (uwaga: tylko dla binarki ssh, nie dla plink — ewentualnie flaga w `RemoteConnection`). Bonus ze stabilnej: równoległe fetchowanie list per host (next ładuje na żądanie per conn — `api.rs:738` — więc mniejsze ryzyko, ale timeout i tak krytyczny).

### 3.9 PWA manifest (v1.0.10) — **CZĘŚCIOWO, NISKI**

`crates/mistraldeck/src/pwa.rs::MANIFEST` — sprawdzić czy ma ikonę 512×512 (`purpose: "any maskable"`) i stabilne `"id"`. SW (`SW` const) kasuje stare cache tylko jeśli ma handler `activate` z listą wersji — zweryfikować (definicja CACHE wersjonowana; bez czyszczenia starych nazw telefon może zostać na starym bundlu).

### 3.10 Back gesture / LIFO nawigacja (v1.0.14/15/18) — **TAK, ŚREDNI (tylko PWA mobile)**

W kliencie web next brak jakiegokolwiek `history.pushState`/`popstate` (grep: 0 trafień w `components/mobile.rs`, `shell.rs`, `state.rs`). Na zainstalowanej PWA back zamknie/przeładuje apkę zamiast cofnąć warstwę (dock → podgląd → lista plików → sesje).

Naprawa: w `components/shell.rs` (obok `MOBILE_JS`) zaimplementować stack warstw: każde otwarcie sesji/doku/preview/edit/settings → `history.pushState({nav:layer})`; `popstate` → zamknij dokładnie jedną warstwę (reconcyle z `event.state.nav`); ręczne zamknięcie panelu → `history.back()` (consume entry); z boot-state: pierwsza back → toast, druga w 2 s → zamknięcie. Odpowiednik `store.pushBack/removeBack` ze stabilnej.

### 3.11 Resume retry (v1.0.16) i hung worker (v0.13.12) — **CZĘŚCIOWO, NISKI**

Next: błąd `session/load` → fallback do świeżej sesji z `pending_prompt` (lib.rs ~1399, komentarz „CLI session gone from disk") — ale to inna semantyka niż stabilna: **nie ma retry** właściwego resume przy chwilowym hiccupie transportu (np. restart aplikacji w trakcie resume); next spadnie na nową sesję lub pokaże błąd. `pending_prompt` przy innych błędach jest kasowany (~1409) — wysłany tekst wisi tylko jako user message.

Naprawa: w slocie, przy `SessionError{context:"session/load"|"session/resume"}` pochodzącym od transportu (nie „session not found"), retry `LoadBackend/LoadRemote` do 4× co 3 s zanim fallback/stop.

### 3.12 Re-sync po powrocie do PWA (v1.0.17) — **CZĘŚCIOWO, ŚREDNI**

`shell.rs` (~30–41): `Err(_)` ze streamu → `sleep_ms(2000)` → `refresh(state)` — pełny snapshot, dobry odpowiednik re-syncu po reconnect. Braki vs stabilna: (1) brak `document.addEventListener('visibilitychange', ...)` → powrót na zamrożoną kartę bez zerwanego WS czeka na najbliższy event; (2) po reconnect-refresh nie wołany jest `drain_queue` (wołany tylko przy `refetch==true` z eventu) — zakolejkowany prompt wyśle się dopiero po następnym eventach strukturalnych.

Naprawa: w `shell.rs` dodać `use_future` z `visibilitychange` listenerem (JS eval → trigger refresh) i po reconnect-refresh wywołać `drain_queue`.

### 3.13 Serve umiera po update w trybie systemd (v1.0.19) — **TAK, WYSOKI (headless)**

`crates/mistraldeck-engine/src/updates.rs::update_app_linux` (~766): `Command::new(&exe).spawn(); std::process::exit(0);` — bezpośredni relaunch poza systemd unitem. Pod Linuxem w serve mode to dokładny scenariusz stabilnej: osierocony instancja trzyma port, unit dostaje EADDRINUSE, restarty nic nie dają.

Naprawa: przed relaunch sprawdzić aktywny unit (np. `systemctl is-active --quiet <unit>`, env `INVOCATION_ID`/`NOTIFY_SOCKET`), wtedy restart przez `systemctl restart <unit>` (manager przeżyje cgroup reap); direct relaunch tylko dla instalacji bez systemd. W `crates/mistraldeck/src/serve.rs` (~551): zapisać pidfile (np. `data_dir/serve.pid`), a przy błędzie `bind` z `EADDRINUSE`: odczytać pidfile + `/proc/<pid>/cmdline`, zidentyfikować własny binary (nigdy nie zabijać „w ciemno"), zabić osierocony poprzednik i ponowić bind (do 3×).

### 3.14 Windows: czarne okno terminala przy self-update (v1.0.3) — **TAK, NISKI**

`crates/mistraldeck-engine/src/updates.rs` (~673): `creation_flags(0x0000_0008)` = tylko `DETACHED_PROCESS`, bez `CREATE_NO_WINDOW`. `powershell.exe` (console-subsystem) pokaże okno i (zależnie od scenerii) zablokuje sekwencję.

Naprawa: `creation_flags(0x0800_0008)` (DETACHED_PROCESS | CREATE_NO_WINDOW) — w Rust flagi się OR-uje; wyjątku nie ma, bo helper jest PowerShell-em detached i tak.

---

## 4. Pozycje NIE dotyczące next (z krótkim uzasadnieniem)

- **`5ab6b5a` prompt routing** — bug wynikał z fallbacku „worker's last active session" w `sendPrompt`; next nigdy nie miał tego fallbacku (routing po `current_session` + slot per sesja + `pending_prompt` w slocie).
- **`a4bcf51` invisible session** — w next tworzenie sesji jest serwerowe i natychmiast broadcastowane (`StructureChanged`); klasa bugu „renderer nie zlinkował boot prime" nie istnieje.
- **v1.0.10 attachments ENOENT** — next nie zapisuje uploadów do katalogu tymczasowego w ścieżce send (read-only resolve); upload już robi `create_dir_all` (api.rs:845).
- **v1.0.10 prompt broadcast** — wbudowane w model busu (`UserMessageAdded`).
- **v1.0.11 join-session history** — snapshot/preview z dysku dostarcza pełną historię niezależnie od streamu.
- **v1.0.12 manifest 401** — `remote_auth` w serve.rs zwalnia statyki PWA spod auth (świadomie, z komentarzem).
- **v1.0.13 busyTurns** — snapshot per-sesyjny + pełny refresh po reconnect pokrywa semantykę.
- **v1.0.3 headless update** — brak web-blokady w next; postęp polingiem.
- **v1.0.7 boot prime / chipsy** — next od początku nie otwiera sesji na starcie; wybory aplikowane przy wake (design zbieżny).
- **Elektron-specyficzne**: `windowsHide`/taskkills Electron (v1.0.3), userData migration (14930bf), branding/rebranding assety (14930bf, 6c4f44f, a87db50, d697885) — dotyczą warstwy Electron/shell JS, którą next zastępuje Dioxus/egui.

---

## 5. Podsumowanie: bugi PREZENT w next (priorytety)

### WYSOKI
1. **Update app/CLI przerywa trwający turn** (v1.0.5) — `api_update_app`/`api_upgrade_local_cli` bez checku `slot.busy`; update_app relaunch/exit zabija workerów. → §3.1.
2. **Serve umiera po in-app update pod systemd** (v1.0.19) — `update_app_linux` relaunchuje exe poza unitem; brak pidfile/EADDRINUSE recovery w serve.rs. → §3.13.
3. **Resize dock/sidebar: mouse-only** (faa86ac) — dotyk/PWA nie działa, resize potrafi się zawiesić. → §3.3.
4. **Obrazy nie docierają do LLM** (v1.0.8) — `cmd_prompt` text-only; brak ACP image content blocks. → §3.7.

### ŚREDNI
5. **Back gesture w PWA zamyka aplikację** (v1.0.14/15/18) — brak LIFO stacku `pushState`/`popstate`. → §3.10.
6. **SSH first-connect hang na host-key prompcie** (v1.0.9) — brak `StrictHostKeyChecking=accept-new`/`ConnectTimeout`. → §3.8.
7. **Auto-approval ignoruje wbudowany tryb yolo** (v1.0.6) — każde narzędzie pyta o zgodę. → §3.6.
8. **Worker death: pending map nie opróżniana** (v1.0.4) — fail-fast działa przez poll `exited()`, ale drain przy `StdoutClosed` + hung-worker probe (v0.13.12) są niepełne. → §3.2.
9. **Re-sync po powrocie na kartę** (v1.0.17) — reconnect jest, brak `visibilitychange` i `drain_queue` po reconnect. → §3.12.

### NISKI
10. **Kolejka promptów: brak „send now" i odporności na stale busy** (v0.13.15/f6398d0) → §3.5.
11. **Resume bez auto-retry przy hiccupie transportu** (v1.0.16) — fallback istnieje, retry nie. → §3.11.
12. **Tool-approval: długie komendy ucięte (ellipsis), brak mobile layoutu** (d551ecd) → §3.4.
13. **Windows self-update: czarne okno terminala** (v1.0.3) — brak `CREATE_NO_WINDOW` przy `DETACHED_PROCESS`. → §3.14.
14. **PWA manifest/SW detal (ikona 512, `id`, cache purge)** (v1.0.10) → §3.9.
