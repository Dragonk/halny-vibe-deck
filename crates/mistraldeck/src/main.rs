//! Mistral Deck desktop/server entry — a thin wrapper over the
//! library (`mistraldeck::run`). The headless console build lives in
//! the `mistraldeck-appserver` crate instead.

// Windows: no console window — double-clicking the exe opens the GUI
// directly. Console output is only kept for debug builds.
#![cfg_attr(all(target_os = "windows", feature = "gui", not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    mistraldeck::run();
}
