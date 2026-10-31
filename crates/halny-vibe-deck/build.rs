// Embed the Halny Vibe Deck icon (the same artwork as halny-vibe-deck) into
// the Windows exe as a PE resource, so Explorer / taskbar show a proper
// icon instead of the generic binary one.
//
// The icon is stored in the repo as base64 text (assets/icon.ico.b64) —
// no binary files in the repo. On Windows targets we decode it, let
// windres turn it into a COFF object (`.rsrc` section) and link it into
// the exe via cargo's link directives. All other targets: no-op.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/icon.ico.b64");
    println!("cargo:rerun-if-changed=build.rs");

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let b64 = std::fs::read_to_string("assets/icon.ico.b64").expect("read assets/icon.ico.b64");
    let ico = decode_base64(b64.trim());
    assert!(!ico.is_empty() && ico.len() > 100, "decoded icon suspiciously small");

    std::fs::write(out.join("icon.ico"), &ico).expect("write icon.ico");
    std::fs::write(out.join("halny-vibe-deck-icon.rc"), "1 ICON \"icon.ico\"\n").expect("write rc");

    let (windres, ar) = if target.contains("x86_64") {
        ("x86_64-w64-mingw32-windres", "x86_64-w64-mingw32-ar")
    } else {
        ("i686-w64-mingw32-windres", "i686-w64-mingw32-ar")
    };

    let obj = out.join("halny_vibe_deck_icon.o");
    let status = Command::new(windres)
        .current_dir(&out)
        .arg("halny-vibe-deck-icon.rc")
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("{windres} failed to start: {e}; is mingw-w64 installed?"));
    assert!(status.success(), "windres failed to compile the icon resource");

    let lib = out.join("libhalny_vibe_deck_icon.a");
    let _ = std::fs::remove_file(&lib);
    let status = Command::new(ar)
        .arg("crs")
        .arg(&lib)
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("{ar} failed to start: {e}"));
    assert!(status.success(), "ar failed to archive the icon object");

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static:+whole-archive=halny_vibe_deck_icon");
}

/// Minimal standard-alphabet base64 decoder (no dependencies).
fn decode_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        let v = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            b'\n' | b'\r' | b' ' => continue,
            b'=' => break,
            other => panic!("invalid base64 byte {other}"),
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}