//! Dock: Files / Git panel on the right side of the chat view
//! (file tree, git status and previews arrive with the fs/git server
//! functions iteration). Same CSS classes as the Nexus dock.

use dioxus::prelude::*;

use crate::i18n::tr;
use crate::state::{lang, ClientState, View};

pub fn Dock() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut tab = use_signal(|| "files".to_string());

    let s = state.read();
    let lang = lang(&state);
    let open = s.snapshot.config.dock_open;
    let view = s.view;
    drop(s);

    if !open || view != View::Chat {
        return VNode::empty();
    }

    let files_cls = if *tab.read() == "files" { "dock-tab active" } else { "dock-tab" };
    let git_cls = if *tab.read() == "git" { "dock-tab active" } else { "dock-tab" };

    rsx! {
        aside { class: "dock", style: "width:320px;",
            div { class: "dock-tabs",
                button { class: "{files_cls}", onclick: move |_| tab.set("files".into()), "{tr(&lang, \"filesTab\")}" }
                button { class: "{git_cls}", onclick: move |_| tab.set("git".into()), "{tr(&lang, \"gitTab\")}" }
            }
            div { class: "dock-body",
                div { class: "empty-text", "…in progress" }
            }
        }
    }
}
