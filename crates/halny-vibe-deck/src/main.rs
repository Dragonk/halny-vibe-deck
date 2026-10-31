//! Halny Vibe Deck desktop/server entry — a thin wrapper over the
//! library (`halny_vibe_deck::run`). The headless console build lives in
//! the `halny-vibe-deck-appserver` crate instead.

// Windows: no console window — double-clicking the exe opens the GUI
// directly. Console output is only kept for debug builds.
#![cfg_attr(all(target_os = "windows", feature = "gui", not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    halny_vibe_deck::run();
}
