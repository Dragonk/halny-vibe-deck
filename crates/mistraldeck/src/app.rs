//! Root component: bootstrap (first snapshot) and the app shell
//! (header + sidebar + view). L1 login gate: when `setup/status`
//! reports no API key the LoginView replaces the Shell (TUI
//! wizard parity); the remote HTTPS listener still authenticates
//! with a /login page before the app is even served.

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
    // L1: no API key (setup/status loaded, wizard-capable or not)
    // or an explicit "sign in again" from Settings / rate limit —
    // the login wizard replaces the Shell.
    let login_gate = s
        .snapshot
        .setup
        .as_ref()
        .map(|st| !st.has_api_key)
        .unwrap_or(false);
    if login_gate || s.login_requested {
        drop(s);
        return rsx! { components::login::LoginView {} };
    }
    drop(s);

    rsx! { components::shell::Shell {} }
}

/// Switch the top-level view (used by the sidebar nav).
pub fn set_view(mut state: Signal<ClientState>, view: crate::state::View) {
    state.write().view = view;
}
