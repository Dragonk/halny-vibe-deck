//! Chat view: empty state with the pixel flame, message list with user
//! bubbles / AI rows (thinking, tool calls, markdown), the agent
//! busy indicator. Ported from Nexus ChatView + MessageBubble (same
//! CSS classes).

use dioxus::prelude::*;
use vibedeck_core::types::{Message, MessageSender, ToolStatus};

use crate::i18n::tr;
use crate::state::{lang, ClientState};

pub fn ChatView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let s = state.read();
    let messages = s.snapshot.current_messages.clone();
    let busy = s.snapshot.busy;
    let status = s.snapshot.status.clone();
    let acp_error = s.snapshot.acp_error.clone();
    let queue_len = s.queue.len();
    drop(s);

    // one-shot: install the mermaid MutationObserver renderer (client only;
    // the server SSR build never evaluates JS)
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let mut mermaid_init = use_signal(|| false);
        if !*mermaid_init.read() {
            mermaid_init.set(true);
            spawn(async move {
                dioxus::document::eval(MERMAID_JS);
            });
        }
    }

    let subtitle = match status.as_str() {
        "connected" => tr(&lang, "agentReady").to_string(),
        "connecting" => tr(&lang, "statusConnecting").to_string(),
        "error" => tr(&lang, "agentConnectionError").to_string(),
        _ => tr(&lang, "configureAcpInSettings").to_string(),
    };

    rsx! {
        div { class: "chat-view-wrap",
            if queue_len > 0 {
                div { class: "tool-approval-queue-hint", style: "position:static;margin:0 16px 4px;", "⏳ {queue_len}" }
            }
            div { class: "chat-messages",
                div { class: "chat-inner",
                    if messages.is_empty() {
                        div { class: "chat-empty",
                            Flame {}
                            div { class: "chat-empty-title", "{tr(&lang, \"appName\")}" }
                            div { class: "chat-empty-subtitle", "{subtitle}" }
                            if status == "error" && !acp_error.is_empty() {
                                div { class: "chat-error-banner", span { "⚠" } " {acp_error}" }
                            }
                        }
                    } else {
                        div { class: "message-list",
                            for (idx, msg) in messages.iter().enumerate() {
                                MessageBubble { msg: msg.clone(), streaming: !msg.is_complete && busy && idx == messages.len() - 1 }
                            }
                            if busy {
                                div { class: "agent-busy-row",
                                    span { class: "agent-busy-dots", span {}, span {}, span {} }
                                    "{tr(&lang, \"agentWorkingLabel\")}"
                                }
                            }
                        }
                    }
                }
            }
            crate::components::input::InputBar {}
        }
    }
}

/// The pixel-art flame from the Nexus empty state.
#[component]
fn Flame() -> Element {
    rsx! {
        svg {
            class: "chat-empty-icon",
            xmlns: "http://www.w3.org/2000/svg",
            width: "95", height: "68",
            view_box: "0 0 212.121 151.515",
            version: "1.1",
            rect { fill: "var(--yellowbright-500)", x: "30.303001", y: "0", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--yellowbright-500)", x: "151.515", y: "0", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--yellow-500)", x: "30.303001", y: "30.303001", width: "60.605999", height: "30.302999" }
            rect { fill: "var(--yellow-500)", x: "121.21201", y: "30.303001", width: "60.605999", height: "30.302999" }
            rect { fill: "var(--orangebright-500)", x: "30.303001", y: "60.606003", width: "151.515", height: "30.302999" }
            rect { fill: "var(--orange-500)", x: "30.303001", y: "90.908997", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--orange-500)", x: "90.908997", y: "90.908997", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--orange-500)", x: "151.515", y: "90.908997", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--red-700)", x: "0", y: "121.21201", width: "90.908997", height: "30.302999" }
            rect { fill: "var(--red-700)", x: "121.21201", y: "121.21201", width: "90.908997", height: "30.302999" }
        }
    }
}

#[component]
pub fn MessageBubble(msg: Message, streaming: bool) -> Element {
    match msg.sender {
        MessageSender::User => rsx! {
            div { class: "msg-row msg-user",
                div { class: "msg-bubble",
                    div { class: "msg-text", "{msg.content}" }
                }
            }
        },
        MessageSender::Ai => rsx! {
            div { class: "msg-row msg-ai",
                span { class: "msg-avatar", "V" }
                div { class: "msg-content",
                    for block in &msg.thinking {
                        ThinkingSection { block: block.clone() }
                    }
                    if !msg.tool_calls.is_empty() {
                        div { class: "tool-calls-list",
                            for tc in &msg.tool_calls {
                                ToolCallBlock { tc: tc.clone() }
                            }
                        }
                    }
                    MarkdownLite { text: msg.content.clone() }
                    if streaming {
                        span { class: "cursor-blink" }
                    }
                }
            }
        },
        MessageSender::System => rsx! {
            div { class: "msg-row msg-notice",
                div { class: "msg-content", "{msg.content}" }
            }
        },
    }
}

#[component]
fn ThinkingSection(block: vibedeck_core::types::ThinkingBlock) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let label = if block.is_complete {
        tr(&lang, "thinking").to_string()
    } else {
        tr(&lang, "thinkingThinking").to_string()
    };
    let paragraphs: Vec<&str> = block.content.split("\n\n").collect();
    rsx! {
        details { class: "thinking-section",
            summary { class: "thinking-toggle",
                span { class: "thinking-dot" }
                span { class: "thinking-label", "{label}" }
                span { class: "thinking-chevron" }
            }
            div { class: "thinking-content",
                div { class: "thinking-paragraphs",
                    for para in paragraphs {
                        p { class: "thinking-paragraph", "{para}" }
                    }
                }
            }
        }
    }
}

#[component]
fn ToolCallBlock(tc: vibedeck_core::types::ToolCall) -> Element {
    let (icon, done_cls) = match tc.status {
        ToolStatus::Done => ("✔", "tool-done"),
        ToolStatus::Error => ("✖", "tool-error"),
        ToolStatus::Running => ("⏳", "tool-running"),
    };
    let kind = tc.kind.clone();
    let title = tc.title.clone();
    let input = tc.input_json.clone().unwrap_or_default();
    let output = tc.output.clone().unwrap_or_default();
    let has_changes = !tc.file_changes.is_empty();

    rsx! {
        details { class: "tool-rich-bash",
            summary { class: "tool-call-header",
                span { class: "tool-status {done_cls}", "{icon}" }
                span { class: "tool-call-title", "{title}" }
                span { class: "tool-call-chevron" }
            }
            div { class: "tool-call-content",
                if !kind.is_empty() {
                    span { class: "tool-kind-chip", "{kind}" }
                }
                if !input.is_empty() {
                    div { class: "tool-call-input", "{input}" }
                }
                if has_changes {
                    div { class: "tool-call-path",
                        for ch in &tc.file_changes {
                            span { class: "tool-file-path", "{ch.path}" }
                        }
                    }
                }
                if !output.is_empty() {
                    div { class: "tool-call-output", "{output}" }
                }
            }
        }
    }
}

/// Full markdown renderer (see `crate::md`): blocks are rendered to HTML and
/// mounted through `dangerous_inner_html`. The HTML is fully escaped by the
/// renderer, so model output can never inject raw markup. Mermaid fences are
/// emitted as `.mermaid-container[data-src]` divs and rendered to SVG by
/// MERMAID_JS below (mermaid@10 from CDN, loaded lazily once per page).
#[component]
fn MarkdownLite(text: String) -> Element {
    if text.is_empty() {
        return VNode::empty();
    }
    let html = crate::md::to_html(&text);
    rsx! { div { class: "markdown-content", dangerous_inner_html: html } }
}

/// Client-side mermaid bootstrap. Runs once per page load: lazily injects the
/// mermaid script, then a MutationObserver renders every new
/// `.mermaid-container` (attribute `data-src` holds the escaped diagram
/// source). Dioxus never manages children of these divs, so hydration and
/// re-renders stay safe.
#[cfg(any(feature = "web", feature = "desktop"))]
const MERMAID_JS: &str = r#"
(function(){
  if (window.__vdMermaid) return; window.__vdMermaid = true;
  function load(cb){
    if (window.mermaid) return cb();
    if (window.__vdMermaidLoading) return window.__vdMermaidLoading.then(cb);
    window.__vdMermaidLoading = new Promise(function(res){
      var s = document.createElement('script');
      s.src = 'https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js';
      s.onload = res; s.onerror = res;
      document.head.appendChild(s);
    });
    window.__vdMermaidLoading.then(cb);
  }
  function renderEl(el){
    load(function(){
      if (!window.mermaid) {
        el.classList.add('mermaid-error');
        el.innerHTML = '<div class="mermaid-error-title">mermaid.js unavailable</div>';
        return;
      }
      try {
        window.mermaid.initialize({ startOnLoad: false, securityLevel: 'loose', theme: 'dark' });
        var id = 'vdGraph' + (el.__vdId || (el.__vdId = Math.random().toString(36).slice(2)));
        window.mermaid.render(id, el.getAttribute('data-src') || '', function(svg){
          el.innerHTML = svg; el.classList.add('mermaid-ready');
        }, function(err){
          el.classList.add('mermaid-error');
          el.innerHTML = '<div class="mermaid-error-title">Mermaid error</div><pre></pre>';
          el.querySelector('pre').textContent = String(err && err.message ? err.message : err);
        });
      } catch (e) {
        el.classList.add('mermaid-error');
        el.innerHTML = '<div class="mermaid-error-title">Mermaid error</div><pre></pre>';
        el.querySelector('pre').textContent = String(e);
      }
    });
  }
  function scan(){
    document.querySelectorAll('.mermaid-container:not([data-vd-done])').forEach(function(el){
      el.setAttribute('data-vd-done', '1');
      renderEl(el);
    });
  }
  var mo = new MutationObserver(function(){ scan(); });
  mo.observe(document.body, { childList: true, subtree: true });
  scan();
})();
"#;
