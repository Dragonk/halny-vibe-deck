//! Small cross-crate process helpers.

/// Hide the console window of a spawned child on Windows
/// (`CREATE_NO_WINDOW`). Round 5 (R8): the desktop app must never flash a
/// black terminal window — apply to EVERY `Command` spawn (git, ssh,
/// curl, update helpers, the agent process itself).
#[cfg(windows)]
pub fn no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// No-op on non-Windows platforms.
#[cfg(not(windows))]
pub fn no_window(_cmd: &mut std::process::Command) {}
