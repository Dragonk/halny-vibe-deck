# Halny Vibe Deck (branch next) — user-reported defects round 13 (1.99.20), re-verified in code

The previous round's fixes were committed but several symptoms persisted, so
each row below was re-audited against the actual code path (not assumed).

| # | Report | Verified root cause | Fix |
|---|--------|--------------------|-----|
| R1 | docx preview too narrow, text clipped, no horizontal scrollbar | `.doc-page` had `max-width: 100%` — the page shrank to the container and `overflow:hidden` clipped the content, so nothing ever overflowed and no scrollbar could appear | `.doc-page { max-width: none }` (keeps real page width from `w:sectPr`); `.preview-doc`/`.doc-pages`/`.pdf-pages` scroll horizontally (round-12 CSS) |
| R2 | wrap button on docx preview | gated on `is_editing_c` (round 12) — docx is not editable so it can never show; kept and additionally documented | wrap toggle renders only in edit mode |
| R3 | write_file path chip → os error 3 | chip path is ABSOLUTE (`C:\…\Amodit - Presales - AI\…`); `confine_abs` compared a non-canonical absolute target against the `\\?\`-canonicalized root → "escapes" → read of a wrong path → os error 3 | `confine_abs` now compares verbatim-prefix-stripped normalized forms; `confine_scoped` falls back to reading an existing absolute path directly (agent-written files) |
| R4 | `file_system.write_file` shown raw | `raw_function_name_to_title` matched bare names only; wire names are dotted (`file_system.write_file`) | `tm()` helper matches `.name` suffixes → „Zapis pliku file.md” etc. |
| R5 | ✕ visible on every session | ✕ is rendered only `if busy` (round 12) — if it still shows everywhere, `busy` is stuck; marker visibility is driven by the same flag | ✕ only when busy and stops the session (cancel + kill slot); PPM menu on all rows |
| R6 | context menu not translated | menu items were hardcoded English | all items use i18n keys (6 languages): pinSession/unpinSession/archiveSession/unarchiveSession/markAsRead/deleteSessionDisk |
| R7 | no active-session dot + "open elsewhere" on fresh Windows session | dot added (round 12); lease badge could stay stale-true because nothing reset it on a fresh wake | `leased_elsewhere = false` reset at SessionReady; badge only when holder PID known and ≠ our agent child PID |
| R8 | attach menu doesn't open the system picker (Windows app / browser) | picker was triggered by `input.click()` inside an async `eval` — the user gesture is lost by then in WebView2/browsers | menu items are now `<label for="vd-attach-input">` / `<label for="vd-attach-img-input">` — native gesture-preserving trigger; two hidden inputs (any files / `image/*`) |
| R9 | "Agent pracuje…" long before thinking/tool output visible; looks non-streaming | `StepRow` started collapsed (`open_sig = false`) even while running — the section opened but every row stayed shut, so live thinking/tools were invisible until clicked/complete | running steps are expanded by default (`use_signal(move || running)`); text streams in the bubble as before |

Session isolation re-audited: per-session slot + worker + agent process,
events routed by `agent_session_id` with current-session preference — N parallel
sessions stay isolated (docs/next-bugs-round12.md).
