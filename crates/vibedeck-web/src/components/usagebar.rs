//! Live usage bar below the input card: context fill, cache hit ratio,
//! in/out tokens and throughput (ported from Nexus UsageBar.tsx).

use dioxus::prelude::*;

use crate::state::ClientState;

fn fmt_tokens(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n as u64)
    }
}

pub fn UsageBar() -> Element {
    let state = use_context::<Signal<ClientState>>();
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
    let ctx_fill_cls = if ctx_pct > 85.0 {
        "usage-context-fill warn"
    } else {
        "usage-context-fill"
    };
    let ctx_width = format!("width: {}%;", ctx_pct);
    let hit = u.cache_hit_rate();
    let hit_pct = if u.prompt_tokens > 0.0 { Some((hit * 100.0).round() as u64) } else { None };
    let tps = u.tokens_per_second;

    rsx! {
        div { class: "usage-bar",
            if has_context {
                span { class: "usage-bar-item usage-context",
                    span { class: "usage-bar-label", "ctx" }
                    span { class: "usage-context-track",
                        span { class: "{ctx_fill_cls}", style: "{ctx_width}" }
                    }
                    span { class: "usage-bar-value", "{ctx_pct.round() as u64}%" }
                }
            }
            if let Some(hit_pct) = hit_pct {
                span { class: "usage-bar-item usage-cache",
                    span { class: "usage-bar-label", "⚡" }
                    span { class: "usage-bar-value", "{hit_pct}%" }
                }
            }
            if u.prompt_tokens > 0.0 {
                span { class: "usage-bar-item",
                    span { class: "usage-bar-label", "in" }
                    span { class: "usage-bar-value", "{fmt_tokens(u.prompt_tokens)}" }
                }
            }
            if u.completion_tokens > 0.0 {
                span { class: "usage-bar-item",
                    span { class: "usage-bar-label", "out" }
                    span { class: "usage-bar-value", "{fmt_tokens(u.completion_tokens)}" }
                }
            }
            if tps > 0.0 {
                span { class: "usage-bar-item",
                    span { class: "usage-bar-label", "tok/s" }
                    span { class: "usage-bar-value", "{tps.round() as u64}" }
                }
            }
        }
    }
}
