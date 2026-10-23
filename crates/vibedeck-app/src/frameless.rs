//! Frameless window chrome: the native titlebar is disabled in `main.rs`
//! (`.with_decorations(false)`, like Photocraft / the Electron original),
//! so this module reimplements the missing OS behaviour on top of egui:
//!
//! - **dragging** the header moves the window (`ViewportCommand::StartDrag`),
//! - **double-clicking** the header toggles maximize,
//! - **edge resize**: the pointer near a window edge switches the cursor and
//!   dragging starts a native OS resize via `ViewportCommand::BeginResize`
//!   (sides 6px, corners 14px).

use eframe::egui::{self, Context, CursorIcon, ResizeDirection, ViewportCommand};

const EDGE: f32 = 6.0;
const CORNER: f32 = 14.0;

/// Which window edge(s) a drag is resizing from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    N,
    S,
    W,
    E,
    Nw,
    Ne,
    Sw,
    Se,
}

impl ResizeEdge {
    fn direction(self) -> ResizeDirection {
        match self {
            ResizeEdge::N => ResizeDirection::North,
            ResizeEdge::S => ResizeDirection::South,
            ResizeEdge::W => ResizeDirection::West,
            ResizeEdge::E => ResizeDirection::East,
            ResizeEdge::Nw => ResizeDirection::NorthWest,
            ResizeEdge::Ne => ResizeDirection::NorthEast,
            ResizeEdge::Sw => ResizeDirection::SouthWest,
            ResizeEdge::Se => ResizeDirection::SouthEast,
        }
    }

    fn cursor(self) -> CursorIcon {
        match self {
            ResizeEdge::N | ResizeEdge::S => CursorIcon::ResizeVertical,
            ResizeEdge::W | ResizeEdge::E => CursorIcon::ResizeHorizontal,
            ResizeEdge::Nw | ResizeEdge::Se => CursorIcon::ResizeNwSe,
            ResizeEdge::Ne | ResizeEdge::Sw => CursorIcon::ResizeNeSw,
        }
    }
}

/// Detect the pointer over a window edge: set the resize cursor, and when a
/// drag starts hand the resize over to the OS (`BeginResize`).
/// Call once per frame, before building the panels, when the window
/// is not maximized.
pub fn window_edges(ctx: &Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if maximized {
        return;
    }

    let Some(pointer) = ctx.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    let Some(win) = ctx.input(|i| i.viewport().inner_rect) else {
        return;
    };
    if !win.contains(pointer) {
        return;
    };

    let near_left = pointer.x - win.left() < EDGE;
    let near_right = win.right() - pointer.x < EDGE;
    let near_top = pointer.y - win.top() < EDGE;
    let near_bottom = win.bottom() - pointer.y < EDGE;
    // Corners are a bit bigger so they're easy to grab.
    let corner_left = pointer.x - win.left() < CORNER;
    let corner_right = win.right() - pointer.x < CORNER;
    let corner_top = pointer.y - win.top() < CORNER;
    let corner_bottom = win.bottom() - pointer.y < CORNER;

    let edge = match (corner_left, corner_right, corner_top, corner_bottom) {
        (true, _, true, _) => Some(ResizeEdge::Nw),
        (_, true, true, _) => Some(ResizeEdge::Ne),
        (true, _, _, true) => Some(ResizeEdge::Sw),
        (_, true, _, true) => Some(ResizeEdge::Se),
        _ => match (near_left, near_right, near_top, near_bottom) {
            (true, _, _, _) => Some(ResizeEdge::W),
            (_, true, _, _) => Some(ResizeEdge::E),
            (_, _, true, _) => Some(ResizeEdge::N),
            (_, _, _, true) => Some(ResizeEdge::S),
            _ => None,
        },
    };

    let Some(edge) = edge else { return };
    ctx.set_cursor_icon(edge.cursor());

    let dragging = ctx.input(|i| i.pointer.primary_down());
    if dragging && ctx.input(|i| i.pointer.primary_pressed()) {
        // Hand the resize to the OS; it tracks the pointer itself.
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(edge.direction()));
    }
    ctx.request_repaint();
}

/// ─ □ ✕ window buttons (Nexus `.header-btn`: 32×28, radius 4;
/// close hovers `#e02020` with a white glyph).
pub fn window_buttons(ctx: &Context, ui: &mut egui::Ui, p: &crate::theme::Palette) {
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
    let maximize_glyph = if maximized { "❐" } else { "□" };
    if btn(ui, maximize_glyph, p.panel_hover, p.text).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if btn(ui, "─", p.panel_hover, p.text).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}
