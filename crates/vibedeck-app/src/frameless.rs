//! Frameless window chrome: the native titlebar is disabled in `main.rs`
//! (`.with_decorations(false)`), so this module reimplements the missing
//! OS behaviour on top of egui — the way PhotoCraft does it
//! (storytold/photocraft, crates/ui-egui/src/titlebar.rs):
//!
//! - **dragging** the header moves the window (`ViewportCommand::StartDrag`),
//! - **double-clicking** the header toggles maximize,
//! - **edge resize**: invisible zones along the window edges (sides 8pt,
//!   corners 12pt) live on a foreground layer, so they win every hit. A drag
//!   that starts in one hands the resize to the OS
//!   (`ViewportCommand::BeginResize`) — the compositor then tracks the
//!   pointer itself, like a native window.
//! - **hand-off**: the OS keeps the button release of a move/resize it owns,
//!   so egui would think the button is still down (moves/resizes would stop
//!   working until the next click). `release_after_os_resize` ends the press
//!   synthetically on the next input — call it from `raw_input_hook`.

use eframe::egui::{
    self, Context, CursorIcon, Id, LayerId, Order, PointerButton, RawInput, Rect, ResizeDirection,
    Sense, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};

use crate::theme::Palette;

const EDGE: f32 = 8.0;
const CORNER: f32 = 12.0;

/// The eight edge/corner zones of the window.
fn zones(w: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    let (e, c) = (EDGE, CORNER);
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeVertical),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeVertical),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeHorizontal),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeHorizontal),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

fn handed_off_id() -> Id {
    Id::new("vd-resize-handed-off")
}

/// An OS resize or move started at `pass`; `released` is the pass whose input
/// ended the press ([`release_after_os_resize`]).
#[derive(Clone, Copy, Debug)]
struct HandOff {
    pass: u64,
    released: Option<u64>,
}

/// The OS took the press at this pass: [`release_after_os_resize`] ends it on
/// the next input.
fn hand_off(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(handed_off_id(), HandOff { pass, released: None }));
}

/// Move the window from this press (header drag). The window manager keeps
/// the button release of the move, so the same hand-off ends the press.
pub fn start_os_move(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    hand_off(ctx);
}

/// The compositor takes the button release of an OS resize or move, so egui
/// keeps the edge pressed: the zones stop hovering until the next click, and
/// every other resize/move fails. End the press ourselves on the input after
/// the hand-off, but only while egui still has it down and this input brings
/// no release of its own: where the OS does send it, nothing is added, so no
/// second release can turn into a click. Call from `raw_input_hook`.
pub fn release_after_os_resize(ctx: &Context, raw: &mut RawInput) {
    let Some(h) = ctx.data(|d| d.get_temp::<HandOff>(handed_off_id())) else {
        return;
    };
    if h.released.is_some() || h.pass >= ctx.cumulative_pass_nr() {
        return;
    }
    let os_released = raw
        .events
        .iter()
        .any(|ev| matches!(ev, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. }));
    if let Some(pos) = ctx
        .input(|i| i.pointer.latest_pos())
        .filter(|_| !os_released && ctx.input(|i| i.pointer.primary_down()))
    {
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: ctx.input(|i| i.modifiers),
        });
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(handed_off_id(), HandOff { released: Some(pass), ..h }));
}

/// The compositor owns the pointer during an OS resize and sets its own
/// cursor; egui-winit only re-sends a cursor that changed, so afterwards the
/// window would keep the compositor's arrow until the next press. On the
/// first pointer motion after the hand-off, show the default cursor for one
/// frame so the resize cursor is sent again on the next.
fn reshow_cursor_after_os_resize(ctx: &Context) {
    let Some(HandOff { released: Some(pass), .. }) =
        ctx.data(|d| d.get_temp::<HandOff>(handed_off_id()))
    else {
        return;
    };
    if pass < ctx.cumulative_pass_nr()
        && ctx.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::PointerMoved(_))))
    {
        ctx.data_mut(|d| d.remove::<HandOff>(handed_off_id()));
        ctx.set_cursor_icon(CursorIcon::Default);
        ctx.request_repaint();
    }
}

/// Invisible resize zones along the window's edges, above everything else; a
/// press in one starts an OS resize in that direction. None while the window
/// is maximized or full screen. Call once per frame, before building panels.
pub fn resize_zones(ctx: &Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false))
        || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
    {
        reshow_cursor_after_os_resize(ctx);
        return;
    }
    let w = ctx.screen_rect();
    let layer = LayerId::new(Order::Foreground, Id::new("vd-resize"));
    let zui = Ui::new(
        ctx.clone(),
        layer,
        Id::new("vd-resize-zones"),
        UiBuilder::new().max_rect(w),
    );
    for (i, (rect, dir, cursor)) in zones(w).into_iter().enumerate() {
        let resp = zui
            .interact(rect, Id::new(("vd-resize", i)), Sense::drag())
            .on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            hand_off(ctx);
        }
    }
    // After the zones, so its one-frame cursor wins over theirs.
    reshow_cursor_after_os_resize(ctx);
}

/// ─ □ ✕ window buttons (Nexus `.header-btn`: 32×28, radius 4;
/// close hovers `#e02020` with a white glyph).
pub fn window_buttons(ctx: &Context, ui: &mut egui::Ui, p: &Palette) {
    let btn = |ui: &mut egui::Ui, glyph: &str, hover_fill: egui::Color32, hover_text: egui::Color32| {
        let size = egui::vec2(32.0, 28.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let base_text = ui.visuals().widgets.inactive.text_color();
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(4.0), {
                if response.hovered() {
                    hover_fill
                } else {
                    egui::Color32::TRANSPARENT
                }
            });
        let color = if response.hovered() {
            hover_text
        } else {
            base_text
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(13.0),
            color,
        );
        response
    };

    let close_red = egui::Color32::from_rgb(0xe0, 0x20, 0x20);
    if btn(ui, "✕", close_red, egui::Color32::WHITE).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let maximize_glyph = if maximized { "▣" } else { "□" };
    if btn(ui, maximize_glyph, p.panel_hover, p.text).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if btn(ui, "—", p.panel_hover, p.text).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}
