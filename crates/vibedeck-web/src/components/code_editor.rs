//! Editable code editor with syntax highlighting.
//!
//! Same idea as the Nexus CodeMirror editor, but dependency-free and
//! wasm-safe: a transparent `<textarea>` stacked on top of a highlighted
//! `<pre>`, plus a line-number gutter. Scroll positions are synced through
//! a tiny `document::eval` script. Tab inserts four spaces at the caret.

use dioxus::prelude::*;

use crate::highlight;

/// CodeEditor: `code` is the signal bound to the text; `id` disambiguates
/// the internal DOM ids when several editors are on screen.
#[component]
pub fn CodeEditor(mut code: Signal<String>, id: String) -> Element {
    let text = code.read().clone();
    let hl = highlight::highlight_toml(&text);
    let gut = highlight::gutter_html(&text);

    rsx! {
        div { class: "code-editor",
            div { class: "code-gutter", id: "vd-ed-gut-{id}", dangerous_inner_html: "{gut}" }
            pre { class: "code-hl", id: "vd-ed-hl-{id}", dangerous_inner_html: "{hl}" }
            textarea {
                class: "code-ta",
                id: "vd-ed-ta-{id}",
                spellcheck: "false",
                autocomplete: "off",
                value: "{text}",
                oninput: move |e| code.set(e.value()),
                onscroll: {
                    let id = id.clone();
                    move |_| {
                        let js = format!(
                            "var t=document.getElementById('vd-ed-ta-{i}');if(t){{var h=document.getElementById('vd-ed-hl-{i}');var g=document.getElementById('vd-ed-gut-{i}');if(h){{h.scrollTop=t.scrollTop;h.scrollLeft=t.scrollLeft;}}if(g){{g.scrollTop=t.scrollTop;}}}}",
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
