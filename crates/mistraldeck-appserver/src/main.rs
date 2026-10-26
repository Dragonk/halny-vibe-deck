//! Halny Vibe Deck headless app server: the same axum server + engine as
//! the desktop app, but always headless (no webview) and built as a
//! console binary (std output is kept on Windows). Useful for servers,
//! containers and remote-access-only setups.

fn main() {
    mistraldeck::set_force_headless();
    mistraldeck::run();
}
