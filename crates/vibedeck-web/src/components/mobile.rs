//! Mobile / touch UX (Nexus v0.10.4 parity).
//!
//! One injected script (client only — the server SSR build never evaluates
//! JS) handles the JS half; the Rust half is a pair of hidden bridge
//! buttons the script "clicks" (the same pattern as the markdown path
//! links in chat.rs):
//!
//! 1. **Full-area swipe gestures**: swipe right reveals the sidebar,
//!    swipe left reveals the dock; a swipe in the opposite direction hides
//!    the visible panel first. Gated to phones (≤768 px) and skipping text
//!    controls / horizontally scrollable strips.
//! 2. **Long-press** on a session row opens the same context menu as
//!    right-click, and the synthesized click after the finger lift is
//!    suppressed (it would immediately close the menu).
//! 3. **`--vv-h`** tracks the visual viewport height so the input card
//!    stays above the on-screen keyboard (the viewport meta
//!    `interactive-widget=resizes-content` covers the rest).

use dioxus::prelude::*;

use crate::state::{save_prefs, ClientState, View};

pub const MOBILE_JS: &str = r#"(function(){
  if (window.__vdMobileBridge) return; window.__vdMobileBridge = true;

  // ---- v0.10.4: full-area swipe gestures ------------------------------
  var sx = 0, sy = 0, gestureOk = false;
  document.addEventListener('touchstart', function(e){
    gestureOk = false;
    if (e.touches.length !== 1) return;
    if (!window.matchMedia('(max-width: 768px)').matches) return;
    var t = e.touches[0];
    sx = t.clientX; sy = t.clientY;
    var el = e.target;
    gestureOk = !!el && el.closest &&
      !el.closest('input, textarea, [contenteditable="true"], pre, code, table, .dock-tabs');
  }, { passive: true });
  document.addEventListener('touchend', function(e){
    if (!gestureOk || e.changedTouches.length !== 1) return;
    gestureOk = false;
    var t = e.changedTouches[0];
    var dx = t.clientX - sx, dy = t.clientY - sy;
    if (Math.abs(dx) < 60 || Math.abs(dx) < Math.abs(dy) * 1.5) return;
    var btn = document.getElementById(dx > 0 ? 'vd-swipe-right' : 'vd-swipe-left');
    if (btn) btn.click();
  }, { passive: true });

  // ---- v0.6.0: long-press on session rows -> context menu -------------
  var lpTimer = null, lpFired = false;
  document.addEventListener('touchstart', function(e){
    var el = e.target && e.target.closest ? e.target.closest('.session-item') : null;
    if (!el || e.touches.length !== 1) return;
    var t = e.touches[0];
    var x = t.clientX, y = t.clientY;
    lpFired = false;
    if (lpTimer) clearTimeout(lpTimer);
    lpTimer = setTimeout(function(){
      lpTimer = null;
      lpFired = true;
      el.dispatchEvent(new MouseEvent('contextmenu', {
        bubbles: true, cancelable: true, clientX: x, clientY: y
      }));
    }, 550);
  }, { passive: true });
  document.addEventListener('touchmove', function(){
    if (lpTimer) { clearTimeout(lpTimer); lpTimer = null; }
  }, { passive: true });
  document.addEventListener('touchend', function(e){
    if (lpTimer) { clearTimeout(lpTimer); lpTimer = null; }
    if (lpFired) {
      // prevent the synthesized mousedown/mouseup/click after the finger
      // lift — it would immediately close the menu the long-press opened.
      lpFired = false;
      e.preventDefault();
    }
  }, { passive: false });

  // ---- keyboard: --vv-h tracks the visual viewport ---------------------
  var vv = window.visualViewport;
  if (vv) {
    var onVv = function(){
      document.documentElement.style.setProperty('--vv-h', Math.round(vv.height) + 'px');
    };
    vv.addEventListener('resize', onVv);
    vv.addEventListener('scroll', onVv);
    onVv();
  }
})();
"#;

/// Toggle the dock (persisted through prefs — same path as the header
/// button, so the state survives a reload).
pub async fn set_dock_open(state: Signal<ClientState>, open: bool) {
    let (language, theme, limit) = {
        let s = state.read();
        (
            s.snapshot.config.language.clone(),
            s.snapshot.config.theme.clone(),
            s.snapshot.config.sidebar_sessions_limit.unwrap_or(0),
        )
    };
    save_prefs(state, language, theme, open, limit).await;
}

/// The mobile scrim + the hidden bridge buttons the injected JS clicks.
/// Rendered between the header and the app body (the scrim is fixed, the
/// buttons are display:none — neither affects the desktop layout).
pub fn MobileBridges() -> Element {
    let state = use_context::<Signal<ClientState>>();

    rsx! {
        if state.read().mobile_sidebar_open {
            div {
                class: "mobile-scrim",
                onclick: {
                    let mut state = state.clone();
                    move |_| { state.write().mobile_sidebar_open = false; }
                },
            }
        }
        button {
            id: "vd-swipe-right",
            style: "display:none;",
            onclick: {
                let mut state = state.clone();
                move |_| {
                    // Swipe right: push the dock away, otherwise reveal the sidebar.
                    let dock_visible = {
                        let s = state.read();
                        s.snapshot.config.dock_open && s.view == View::Chat
                    };
                    if dock_visible {
                        let state = state.clone();
                        spawn(async move { set_dock_open(state, false).await; });
                    } else {
                        state.write().mobile_sidebar_open = true;
                    }
                }
            },
        }
        button {
            id: "vd-swipe-left",
            style: "display:none;",
            onclick: {
                let mut state = state.clone();
                move |_| {
                    // Swipe left: hide the session list, otherwise reveal the dock.
                    if state.read().mobile_sidebar_open {
                        state.write().mobile_sidebar_open = false;
                    } else if state.read().view == View::Chat {
                        let state = state.clone();
                        spawn(async move { set_dock_open(state, true).await; });
                    }
                }
            },
        }
    }
}
