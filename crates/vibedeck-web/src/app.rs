//! Root component: bootstrap (auth state + first snapshot), the login
//! gate and the app shell (header + sidebar + view).

use dioxus::prelude::*;

use crate::components;
use crate::state::{refresh, ClientState};

pub fn App() -> Element {
    use_context_provider(|| Signal::new(ClientState::default()));
    let mut state = use_context::<Signal<ClientState>>();

    // Bootstrap: auth state + first snapshot.
    use_future(move || async move {
        let auth_required = crate::api::api_auth_state().await.unwrap_or(false);
        {
            let mut s = state.write();
            s.auth_required = auth_required;
        }
        if !auth_required {
            refresh(state).await;
            state.write().loaded = true;
        }
    });

    let s = state.read();

    if s.auth_required && s.token.is_none() {
        drop(s);
        return rsx! { components::login::Login {} };
    }
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
