//! Live usage bar below the input card: circular context gauge (Zed
//! style), cache hit ratio, in/out tokens, throughput and session cost.
//! Ported from Nexus UsageBar.tsx (same CSS classes, same layout).
//!
//! v0.13.5 parity: the gauge denominator (model context window) comes
//! ONLY from the config `max_context_length` — the runtime
//! `usage_update.size` is the CLI-resolved auto-compact threshold, not
//! the window, so a model that declares no window simply has no gauge
//! (tokens still show). The 1px marker sits at the model's auto-compact
//! threshold (per-model cap > global cap > 80% of window > 200k) and the
//! fill turns red once the used context crosses it. Cost comes from the
//! wire (`usage_update.cost.amount`); previewed sessions get theirs
//! computed from the local price table (engine, Nexus parity).

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{lang, token, ClientState};
use crate::vibestats::UsageModelInfoView;

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
    let mut last_model = use_signal(String::new);
    let mut model_info =
        use_signal(|| UsageModelInfoView { window: None, threshold: None });

    let (u, cur_model) = {
        let s = state.read();
        let u = s.snapshot.usage.clone();
        let cm = s
            .snapshot
            .config_options
            .iter()
            .find(|o| o.id == "model")
            .map(|o| o.current_value.clone())
            .unwrap_or_default();
        (u, cm)
    };

    // Refetch the model window/threshold whenever the model chip changes
    // (Nexus: useEffect on [currentModel, sessWd, sessRid]).
    if cur_model != *last_model.read() {
        last_model.set(cur_model.clone());
        if cur_model.is_empty() {
            model_info.set(UsageModelInfoView {
                window: None,
                threshold: None,
            });
        } else {
            let st = state.clone();
            spawn(async move {
                if let Ok(info) =
                    crate::vibestats::api_usage_model_info(token(&st), cur_model.clone()).await
                {
                    model_info.set(info);
                }
            });
        }
    }
    let info = model_info.read().clone();

    if u.updates == 0 && u.total_tokens == 0.0 {
        return VNode::empty();
    }

    // v0.13.5: window ONLY from the model config; no window = no gauge.
    let ctx_window = info.window.filter(|w| *w > 0.0);
    let has_context = ctx_window.is_some();
    let ctx_pct = match ctx_window {
        Some(w) => ((u.context_used / w) * 100.0).clamp(0.0, 100.0),
        None => 0.0,
    };
    // Auto-compact marker: hidden when disabled (0/-1) or >= 99.5%.
    let threshold = info.threshold.filter(|t| *t > 0.0);
    let compact_pct: Option<f64> = match (ctx_window, threshold) {
        (Some(w), Some(t)) => Some(((t / w) * 100.0).clamp(0.0, 100.0)),
        _ => None,
    };
    let over_compact = compact_pct.map(|c| ctx_pct >= c).unwrap_or(false);
    let gauge_fill_cls = if over_compact {
        "usage-gauge-fill warn"
    } else {
        "usage-gauge-fill"
    };
    let dash = format!(
        "{:.2} {:.2}",
        (ctx_pct / 100.0) * GAUGE_CIRC,
        GAUGE_CIRC
    );
    // Marker circle: 1px dash at the compact threshold position.
    let marker_node: Element = match compact_pct.filter(|c| *c < 99.5) {
        Some(c) => {
            let mdash = format!("1 {:.2}", GAUGE_CIRC);
            let moffset = format!("-{:.2}", (c / 100.0) * GAUGE_CIRC);
            rsx! {
                circle {
                    class: "usage-gauge-mark",
                    cx: "8",
                    cy: "8",
                    r: "6",
                    stroke_dasharray: "{mdash}",
                    stroke_dashoffset: "{moffset}",
                }
            }
        }
        None => VNode::empty(),
    };
    let ctx_title: String = match (ctx_window, threshold) {
        (Some(w), Some(t)) => tf(
            &lang,
            "usageGaugeTooltip",
            &[
                ("used", &fmt_int(u.context_used)),
                ("window", &fmt_int(w)),
                ("compact", &fmt_int(t)),
            ],
        ),
        (Some(w), None) => format!(
            "{} - {}",
            tf(
                &lang,
                "usageContextTooltip",
                &[("used", &fmt_int(u.context_used)), ("total", &fmt_int(w))],
            ),
            tr(&lang, "usageCompactOff")
        ),
        _ => String::new(),
    };
    let hit = u.cache_hit_rate();
    let hit_pct = if u.prompt_tokens > 0.0 {
        Some((hit * 100.0).round() as u64)
    } else {
        None
    };
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
                            {marker_node}
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
