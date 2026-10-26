//! Live usage bar below the input card: circular context gauge (Zed
//! style), cache hit ratio, in/out tokens, throughput and session cost.
//! Ported from Nexus UsageBar.tsx (same CSS classes, same layout).
//!
//! v0.10.2 Nexus parity: the linear context bar became a circular gauge
//! — real used context vs the model's context window (runtime value
//! from usage_update; the config `max_context_length` + auto-compact
//! marker resolution lands with the stats batch). Cost comes straight
//! from the wire (`usage_update.cost.amount`, USD).

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{lang, ClientState};

fn fmt_tokens(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n as u64)
    }
}

fn fmt_int(n: f64) -> String {
    format!("{}", n.round() as u64)
}

/// Circular gauge geometry (Nexus: r = 6, rotated -90°).
const GAUGE_R: f64 = 6.0;
const GAUGE_CIRC: f64 = 2.0 * std::f64::consts::PI * GAUGE_R;

pub fn UsageBar() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let u = state.read().snapshot.usage.clone();

    if u.updates == 0 && u.total_tokens == 0.0 {
        return VNode::empty();
    }

    let has_context = u.context_size > 0.0;
    let ctx_pct = if has_context {
        ((u.context_used / u.context_size) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    // Compact-threshold marker arrives with the stats batch; until then
    // the gauge warns at the same 85% the old linear bar used.
    let warn = ctx_pct > 85.0;
    let gauge_fill_cls = if warn {
        "usage-gauge-fill warn"
    } else {
        "usage-gauge-fill"
    };
    let dash = format!(
        "{:.2} {:.2}",
        (ctx_pct / 100.0) * GAUGE_CIRC,
        GAUGE_CIRC
    );
    let ctx_title = tf(
        &lang,
        "usageContextTooltip",
        &[
            ("used", &fmt_int(u.context_used)),
            ("total", &fmt_int(u.context_size)),
        ],
    );
    let hit = u.cache_hit_rate();
    let hit_pct = if u.prompt_tokens > 0.0 { Some((hit * 100.0).round() as u64) } else { None };
    let cache_title = tf(
        &lang,
        "usageCacheTooltip",
        &[
            ("cached", &fmt_int(u.cached_tokens)),
            ("total", &fmt_int(u.prompt_tokens)),
        ],
    );
    let tps = u.tokens_per_second;
    let prompt_title = tr(&lang, "usagePromptTooltip").to_string();
    let completion_title = tr(&lang, "usageCompletionTooltip").to_string();
    let tps_title = tr(&lang, "usageTpsTooltip").to_string();
    let cost = u.cost;
    let cost_title = tf(
        &lang,
        "usageCostTooltip",
        &[("cost", &format!("{:.4}", cost))],
    );
    let cost_text = format!("{:.2}", cost);

    rsx! {
        div { class: "usage-bar",
            if has_context {
                span { class: "usage-bar-item usage-context",
                    title: "{ctx_title}",
                    svg {
                        class: "usage-gauge",
                        width: "16",
                        height: "16",
                        view_box: "0 0 16 16",
                        "aria-hidden": "true",
                        g { transform: "rotate(-90 8 8)",
                            circle { class: "usage-gauge-track", cx: "8", cy: "8", r: "6" }
                            circle {
                                class: "{gauge_fill_cls}",
                                cx: "8",
                                cy: "8",
                                r: "6",
                                stroke_dasharray: "{dash}",
                            }
                        }
                    }
                    span { class: "usage-bar-value", "{ctx_pct.round() as u64}%" }
                }
            }
            if let Some(hit_pct) = hit_pct {
                span { class: "usage-bar-item usage-cache",
                    title: "{cache_title}",
                    span { class: "usage-bar-label", "⚡" }
                    span { class: "usage-bar-value", "{hit_pct}%" }
                }
            }
            if u.prompt_tokens > 0.0 {
                span { class: "usage-bar-item",
                    title: "{prompt_title}",
                    span { class: "usage-bar-label", "in" }
                    span { class: "usage-bar-value", "{fmt_tokens(u.prompt_tokens)}" }
                }
            }
            if u.completion_tokens > 0.0 {
                span { class: "usage-bar-item",
                    title: "{completion_title}",
                    span { class: "usage-bar-label", "out" }
                    span { class: "usage-bar-value", "{fmt_tokens(u.completion_tokens)}" }
                }
            }
            if tps > 0.0 {
                span { class: "usage-bar-item",
                    title: "{tps_title}",
                    span { class: "usage-bar-label", "tok/s" }
                    span { class: "usage-bar-value", "{tps.round() as u64}" }
                }
            }
            if cost > 0.0 {
                span { class: "usage-bar-item usage-cost",
                    title: "{cost_title}",
                    span { class: "usage-bar-label", "$" }
                    span { class: "usage-bar-value", "{cost_text}" }
                }
            }
        }
    }
}
