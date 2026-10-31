# Halny Vibe Deck (branch next) — user-reported defects round 14 (1.99.21), re-verified in code

Each row was re-audited against the actual code path and the installed
`vibe 3.0.4` schema (`vibe/core/config/models.py`, `vibe_schema.py`).

| # | Report | Verified root cause | Fix |
|---|--------|--------------------|-----|
| R1 | error block clips text, part of the text renders outside the block in chat | `.chat-error-banner` was a flex row with `max-width:480px`; the bare text node is an anonymous flex item with default `min-width:auto`, so long unbroken error strings (paths, tokens) overflowed the border while the block stayed narrow | `.chat-error-banner` now `width:100%; max-width:100%; box-sizing:border-box; overflow:hidden`, and the message is wrapped in `.chat-error-text` with `flex:1; min-width:0; overflow-wrap:anywhere; word-break:break-word; white-space:pre-wrap` (applied to all three banner sites in `chat.rs`) |
| R2 | API key shown as plain text in Settings → Providers | the api_key input was `r#type: "text"` | input is now `type=password` with a Show/Hide toggle button (i18n `showKey`/`hideKey`, 6 languages), `autocomplete="off"`; resets to hidden on provider switch/add |
| R3 | Providers tab missing config fields | `ProviderRow`/`Provider` lacked `extra_headers`, `browser_auth_base_url`, `browser_auth_api_base_url`, `browser_auth_allow_origin_rewrite`, `preserve_api_base`, `emits_finish_reason` (all present in `ProviderConfig` of vibe 3.0.4) | all six added end-to-end: `mistraldeck-vibe/src/config.rs` (parse + serialize + roundtrip test), `vibeconfig.rs` `ProviderRow` (extra_headers as JSON object string, invalid JSON keeps previous value like Nexus), settings form fields with i18n labels; `upsert_provider` no longer hardcodes `extra_headers: Vec::new()` |
| R4 | Models tab vs config file | audited: form already renders/saves every `ModelConfig` field (name, provider, alias, display_name, enabled, supports_images, temperature, top_p, max_output_tokens, input/output/cached prices, thinking, thinking_levels, max_context_length, auto_compact_threshold) plus `active_model` selector | no gap found; kept |
| R5 | rate-limit/usage error from a remote session stays visible after switching to a local session | `ClientState.turn_failed` was a global `Option<(code,message)>` — set only for the current session but never keyed, so it rendered in every session after a switch | `turn_failed` is now `(code, message, agent_session_id)`; `chat.rs` renders the banner only when the id matches the visible session; sidebar switch handlers also clear it; new submit still clears it |

Build/test evidence: `cargo test -p mistraldeck --features server` (49 passed),
`-p mistraldeck-appsrv` (40), `-p mistraldeck-engine` (36), `-p mistraldeck-core`,
`-p mistraldeck-vibe` (32, incl. extended `provider_full_fields_roundtrip`);
`cargo check` clean for `--features server` and `--no-default-features --features web`.
