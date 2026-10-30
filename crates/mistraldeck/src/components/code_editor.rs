//! Editable code editor with syntax highlighting.
//!
//! Same idea as the Nexus CodeMirror editor, but dependency-free and
//! wasm-safe: a transparent `<textarea>` stacked on top of a highlighted
//! `<pre>`, plus a line-number gutter. Scroll positions are synced through
//! a tiny `document::eval` script. Tab inserts four spaces at the caret.
//!
//! Round 7 (R1): in word-wrap mode the gutter numbers are repositioned by
//! `vdEdSync` at each LOGICAL line's real pixel offset, so a wrapped line
//! never gets its own number.

use dioxus::prelude::*;

use crate::highlight;

/// Installed once per page: defines `window.vdEdSync(id)` (scroll sync +
/// wrap-aware gutter positioning) and a window-resize re-sync for every
/// visible editor.
#[cfg(any(feature = "web", feature = "desktop"))]
const EDITOR_JS: &str = r#"
(function () {
  if (window.__vdEdInit) return;
  window.__vdEdInit = true;
  window.vdEdSync = function (id) {
    var t = document.getElementById('vd-ed-ta-' + id);
    if (!t) return;
    var h = document.getElementById('vd-ed-hl-' + id);
    var g = document.getElementById('vd-ed-gut-' + id);
    if (h) { h.scrollTop = t.scrollTop; h.scrollLeft = t.scrollLeft; }
    if (!g) return;
    var wrap = t.classList.contains('wrap');
    g.classList.toggle('wrap', wrap);
    var kids = g.children;
    if (!wrap || !h) {
      g.scrollTop = t.scrollTop;
      for (var i = 0; i < kids.length; i++) kids[i].style.top = '';
      return;
    }
    // wrap mode: position every number at its logical line's offset inside
    // the highlight layer (measured with DOM ranges over the text nodes).
    var gRect = g.getBoundingClientRect();
    var range = document.createRange();
    var walker = document.createTreeWalker(h, NodeFilter.SHOW_TEXT, null);
    var nodes = [], n;
    while ((n = walker.nextNode())) nodes.push(n);
    var tops = [];
    var startNode = null, startOff = 0;
    function flushLine(endNode, endOff) {
      if (!startNode) return;
      range.setStart(startNode, startOff);
      range.setEnd(endNode, endOff);
      var r = range.getBoundingClientRect();
      tops.push(r.top - gRect.top);
    }
    for (var ni = 0; ni < nodes.length; ni++) {
      var node = nodes[ni];
      if (!startNode) { startNode = node; startOff = 0; }
      var txt = node.data;
      for (var ci = 0; ci < txt.length; ci++) {
        if (txt[ci] === '\n') {
          flushLine(node, ci);
          startNode = node; startOff = ci + 1;
        }
      }
    }
    if (startNode && nodes.length) {
      var last = nodes[nodes.length - 1];
      flushLine(last, last.data.length);
    }
    for (var k = 0; k < kids.length && k < tops.length; k++) {
      kids[k].style.top = tops[k] + 'px';
    }
    g.scrollTop = 0;
  };
  window.addEventListener('resize', function () {
    document.querySelectorAll('.code-ta').forEach(function (t) {
      if (window.vdEdSync) window.vdEdSync(t.id.replace('vd-ed-ta-', ''));
    });
  });
})();
"#;

/// CodeEditor: `code` is the signal bound to the text; `id` disambiguates
/// the internal DOM ids when several editors are on screen. `file` picks
/// the syntax highlighter by extension (round 5, R10: the same editor as
/// the preview, with highlighting — incl. Markdown — is used in Settings
/// too); `wrap` turns on word wrapping (round 5, R3; round 7 R1: logical
/// line numbers in the gutter).
#[component]
pub fn CodeEditor(
    mut code: Signal<String>,
    id: String,
    file: Option<String>,
    wrap: Option<bool>,
) -> Element {
    let text = code.read().clone();
    let fname = file.clone().unwrap_or_default();
    let hl = highlight::highlight_auto(&fname, &text);
    let gut = highlight::gutter_html(&text);
    let wrap = wrap.unwrap_or(false);
    let ed_cls = if wrap { "code-editor wrap" } else { "code-editor" };
    let ta_cls = if wrap { "code-ta wrap" } else { "code-ta" };
    let gut_cls = if wrap { "code-gutter wrap" } else { "code-gutter" };

    // Round 7 (R1): install the sync script once and re-sync after every
    // render so the gutter tracks wrapping and any content change (reading
    // `code` inside the effect makes it re-run on each keystroke).
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let code_dep = code;
        let id_dep = id.clone();
        use_effect(move || {
            let _text = code_dep.read().clone(); // dependency: re-sync on change
            let js_id = id_dep.clone();
            spawn(async move {
                let _ = dioxus::document::eval(EDITOR_JS).await;
                let _ = dioxus::document::eval(&format!(
                    "window.vdEdSync && window.vdEdSync('{js_id}');"
                ))
                .await;
            });
        });
    }

    rsx! {
        div { class: "{ed_cls}",
            div { class: "{gut_cls}", id: "vd-ed-gut-{id}", dangerous_inner_html: "{gut}" }
            pre { class: "code-hl", id: "vd-ed-hl-{id}", dangerous_inner_html: "{hl}" }
            textarea {
                class: "{ta_cls}",
                id: "vd-ed-ta-{id}",
                spellcheck: "false",
                autocomplete: "off",
                value: "{text}",
                oninput: move |e| code.set(e.value()),
                onscroll: {
                    let id = id.clone();
                    move |_| {
                        let js = format!(
                            "window.vdEdSync && window.vdEdSync('{i}');",
                            i = id
                        );
                        dioxus::document::eval(js.as_str());
                    }
                },
                onkeydown: {
                    let id = id.clone();
                    move |e| {
                        if let Key::Tab = e.key() {
                            e.prevent_default();
                            let js = format!(
                                "var t=document.getElementById('vd-ed-ta-{i}');if(t){{t.setRangeText('    ',t.selectionStart,t.selectionEnd,'end');t.dispatchEvent(new Event('input',{{bubbles:true}}));}}",
                                i = id
                            );
                            dioxus::document::eval(js.as_str());
                        }
                    }
                },
            }
        }
    }
}
