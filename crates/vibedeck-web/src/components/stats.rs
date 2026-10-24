//! Stats view: live usage cards + recent sessions (full per-session
//! analytics with costs and charts arrive with the vibe-stats engine
//! iteration). Uses the same Nexus card classes.

use dioxus::prelude::*;

use crate::i18n::tr;
use crate::state::{lang, token, ClientState, View};

fn date_str(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}.{y}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn fmt_tokens(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n as u64)
    }
}

pub fn StatsView() -> Element {
    let state = use_context::<Signal<ClientState>>();

    let s = state.read();
    let lang = lang(&state);
    let usage = s.snapshot.usage.clone();
    let sessions = s.snapshot.sessions.clone();
    let busy = s.snapshot.busy;
    drop(s);

    let mut sorted = sessions.clone();
    sorted.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    sorted.truncate(20);

    let session_count = sessions.len();
    let messages: usize = sessions.iter().map(|m| m.message_count).sum();
    let hit_pct = (usage.cache_hit_rate() * 100.0).round() as u64;

    rsx! {
        div { class: "stats-view",
            div { class: "stats-scroll",
                h2 { class: "stats-title", "{tr(&lang, \"statistics\")}" }
                div { class: "pill-row",
                    if busy {
                        span { class: "loading-indicator", "{tr(&lang, \"agentWorkingLabel\")}" }
                    }
                    button {
                        class: "refresh-btn",
                        onclick: {
                            let state = state.clone();
                            move |_| {
                                let state = state.clone();
                                async move {
                                    let _ = crate::api::api_refresh_sessions(token(&state)).await;
                                    crate::state::refresh(state).await;
                                }
                            }
                        },
                        "⟳ {tr(&lang, \"refresh\")}"
                    }
                }
                div { class: "cards-row",
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"totalTokens\")}" }
                        }
                        div { class: "card-value primary", "{fmt_tokens(usage.total_tokens)}" }
                        div { class: "card-meta",
                            "{tr(&lang, \"inputLabel\")} {fmt_tokens(usage.prompt_tokens)} · {tr(&lang, \"outputLabel\")} {fmt_tokens(usage.completion_tokens)}"
                        }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"cacheHitLabel\")}" }
                        }
                        div { class: "card-value success", "{hit_pct}%" }
                        div { class: "card-meta", "{fmt_tokens(usage.cached_tokens)} / {fmt_tokens(usage.prompt_tokens)}" }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"tokPerSecAvg\")}" }
                        }
                        div { class: "card-value accent", "{usage.tokens_per_second.round() as u64}" }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"messagesLabel\")}" }
                        }
                        div { class: "card-value", "{messages}" }
                        div { class: "card-meta", "{tr(&lang, \"recentSessions\")}: {session_count}" }
                    }
                }
                div { class: "session-list-card",
                    div { class: "section-label", "{tr(&lang, \"recentSessions\")}" }
                    div { class: "session-rows",
                        if sorted.is_empty() {
                            div { class: "empty-text", "{tr(&lang, \"noSessionsYet\")}" }
                        }
                        for (i, m) in sorted.iter().enumerate() {
                            {
                                let state = state.clone();
                                let idx = {
                                    sessions.iter().position(|x| x.id == m.id).unwrap_or(i)
                                };
                                let name = m.name.clone();
                                let date = date_str(m.last_active_ms);
                                let count = m.message_count;
                                let dir = m.working_directory.clone();
                                rsx! {
                                    div {
                                        class: "session-row",
                                        onclick: {
                                            let state = state.clone();
                                            move |_| {
                                                let state = state.clone();
                                                async move {
                                                    let _ = crate::api::api_switch_session(token(&state), idx).await;
                                                    crate::state::refresh(state).await;
                                                    crate::app::set_view(state, View::Chat);
                                                }
                                            }
                                        },
                                        div { class: "session-row-main",
                                            div { class: "session-row-title", "{name}" }
                                            div { class: "session-row-meta", "{date} · {count}" }
                                        }
                                        div { class: "session-row-tags",
                                            if !dir.is_empty() {
                                                span { class: "tag primary", "{short(&dir)}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn short(dir: &str) -> String {
    let name = dir.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("");
    if name.is_empty() { dir.to_string() } else { name.to_string() }
}
