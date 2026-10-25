//! Root component: bootstrap (first snapshot) and the app shell
//! (header + sidebar + view). No in-app login gate: the local server is
//! trusted and the remote HTTPS listener authenticates with a /login page
//! before the app is even served.

use dioxus::prelude::*;

use crate::components;
use crate::state::{refresh, ClientState};

pub fn App() -> Element {
    use_context_provider(|| Signal::new(ClientState::default()));
    let mut state = use_context::<Signal<ClientState>>();

    // Bootstrap: first snapshot.
    use_future(move || async move {
        refresh(state).await;
        state.write().loaded = true;
    });

    let s = state.read();

    if !s.loaded {
        drop(s);
        return rsx! { div { class: "loading", "Ładowanie…" } };
    }
    if s.setup_needed && !s.setup_skipped {
        drop(s);
        return rsx! { components::setup::SetupView {} };
    }
    drop(s);

    rsx! { components::shell::Shell {} }
}

/// Switch the top-level view (used by the sidebar nav).
pub fn set_view(mut state: Signal<ClientState>, view: crate::state::View) {
    state.write().view = view;
}
