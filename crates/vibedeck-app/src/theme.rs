//! 1:1 recreation of the Mistral Vibe Nexus palette (from the original
//! `globals.css`), applied to egui `Visuals`.
//!
//! **Unofficial community project — not affiliated with Mistral AI.**

use eframe::egui;
use eframe::egui::Color32;

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The Nexus color palette (dark & light variants).
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub panel: Color32,
    pub panel_hover: Color32,
    pub border: Color32,
    pub border_subtle: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub dim: Color32,
    pub accent: Color32,
    #[allow(dead_code)]
    pub accent_hover: Color32,
    pub user_msg: Color32,
    #[allow(dead_code)]
    pub ai_msg: Color32,
    #[allow(dead_code)]
    pub input_bg: Color32,
    pub session_active_bg: Color32,
    pub session_active_text: Color32,
    pub green: Color32,
    pub red: Color32,
    pub yellow: Color32,
    /// Pixel-flame colors (ChatView.tsx SVG).
    pub flame_yellowbright: Color32,
    pub flame_yellow: Color32,
    pub flame_orangebright: Color32,
    pub flame_orange: Color32,
    pub flame_red: Color32,
}

impl Palette {
    /// Dark theme — the Nexus defaults (`:root` in globals.css).
    pub fn dark() -> Self {
        Self {
            bg: c(0x11, 0x11, 0x15),
            surface: c(0x18, 0x18, 0x1b),
            panel: c(0x22, 0x22, 0x25),
            panel_hover: c(0x2a, 0x2a, 0x2e),
            border: c(0x36, 0x36, 0x3a),
            border_subtle: c(0x28, 0x28, 0x2c),
            text: c(0xf6, 0xf6, 0xf5),
            muted: c(0x8a, 0x8a, 0x8f),
            dim: c(0x55, 0x55, 0x58),
            accent: c(0xfa, 0x50, 0x0f),
            accent_hover: c(0xe8, 0x45, 0x0d),
            user_msg: c(0x2c, 0x1c, 0x17),
            ai_msg: c(0x1d, 0x1d, 0x21),
            input_bg: c(0x1a, 0x1a, 0x1e),
            session_active_bg: c(0x54, 0x2e, 0x21),
            session_active_text: c(0xff, 0xcb, 0x95),
            green: c(0x47, 0xd4, 0x7f),
            red: c(0xe1, 0x05, 0x00),
            yellow: c(0xff, 0xaf, 0x00),
            flame_yellowbright: Color32::from_rgb(0xff, 0xd7, 0x00), // `gold`
            flame_yellow: c(0xff, 0xaf, 0x00),
            flame_orangebright: c(0xff, 0x82, 0x05),
            flame_orange: c(0xfa, 0x50, 0x0f),
            flame_red: c(0xe1, 0x05, 0x00),
        }
    }

    /// Light theme — the Nexus `:root.light` overrides.
    pub fn light() -> Self {
        Self {
            bg: c(0xfa, 0xfa, 0xf9),
            surface: c(0xff, 0xff, 0xff),
            panel: c(0xf5, 0xf5, 0xf4),
            panel_hover: c(0xe7, 0xe5, 0xe4),
            border: c(0xd6, 0xd3, 0xd1),
            border_subtle: c(0xe7, 0xe5, 0xe4),
            text: c(0x1c, 0x19, 0x17),
            muted: c(0x78, 0x71, 0x6c),
            dim: c(0xa8, 0xa2, 0x9e),
            accent: c(0xfa, 0x50, 0x0f),
            accent_hover: c(0xe8, 0x45, 0x0d),
            user_msg: c(0xf6, 0xf6, 0xf5),
            ai_msg: c(0xff, 0xff, 0xff),
            input_bg: c(0xff, 0xff, 0xff),
            session_active_bg: c(0xfd, 0xe8, 0xdd),
            session_active_text: c(0xc2, 0x41, 0x0c),
            green: c(0x16, 0xa3, 0x4a),
            red: c(0xdc, 0x26, 0x26),
            yellow: c(0xd9, 0x77, 0x06),
            flame_yellowbright: Color32::from_rgb(0xff, 0xd7, 0x00),
            flame_yellow: c(0xff, 0xaf, 0x00),
            flame_orangebright: c(0xff, 0x82, 0x05),
            flame_orange: c(0xfa, 0x50, 0x0f),
            flame_red: c(0xe1, 0x05, 0x00),
        }
    }

    pub fn for_theme(theme: &str) -> Self {
        if theme == "light" {
            Self::light()
        } else {
            Self::dark()
        }
    }

    /// Status-dot color, as in the Nexus header.
    #[allow(dead_code)]
    pub fn status_color(&self, status: crate::state::AcpStatus) -> Color32 {
        match status {
            crate::state::AcpStatus::Connected => self.green,
            crate::state::AcpStatus::Connecting => self.yellow,
            crate::state::AcpStatus::Disconnected => self.muted,
            crate::state::AcpStatus::Error => self.red,
        }
    }
}

/// Apply the Nexus look (colors, 6px rounding, 14px body text) to the egui
/// context. Call once at startup and again whenever the theme setting changes.
pub fn apply(ctx: &egui::Context, theme: &str) {
    let p = Palette::for_theme(theme);
    let mut v = if theme == "light" {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };

    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = egui::Stroke::new(1.0_f32, p.border);
    v.hyperlink_color = p.accent;

    let rounding = 6.0;
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.border_subtle);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.inactive.bg_fill = p.panel;
    w.inactive.bg_stroke = egui::Stroke::new(1.0_f32, p.border_subtle);
    w.inactive.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.hovered.bg_fill = p.panel_hover;
    w.hovered.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    w.hovered.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.active.bg_fill = p.panel_hover;
    w.active.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    w.active.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.open.bg_fill = p.panel_hover;
    w.open.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    w.open.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.rounding = egui::Rounding::same(rounding);
    }

    v.selection.bg_fill = p.accent;
    v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);

    ctx.set_visuals(v);

    // Nexus uses 14px system-ui as the base font.
    let mut style = (*ctx.style()).clone();
    for (style_key, font_id) in style.text_styles.iter_mut() {
        match style_key {
            egui::TextStyle::Body | egui::TextStyle::Button => font_id.size = 14.0,
            egui::TextStyle::Small => font_id.size = 12.0,
            egui::TextStyle::Heading => font_id.size = 17.0,
            egui::TextStyle::Monospace => font_id.size = 13.0,
            _ => {}
        }
    }
    ctx.set_style(style);
}

/// Paint the pixel flame at `scale` (1.0 = 212×152 px; the Nexus renders it
/// at ~0.45). Returns the total size in pixels.
pub fn flame_size(scale: f32) -> egui::Vec2 {
    egui::Vec2::new(212.121 * scale, 151.515 * scale)
}

/// Paint the pixel-art flame into `ui` at the given scale (top-left anchored).
pub fn paint_flame(ui: &mut egui::Ui, p: &Palette, scale: f32) -> egui::Response {
    let size = flame_size(scale);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let origin = rect.left_top();
    for (color, x, y, w, h) in flame_rects(p) {
        let px = egui::Rect::from_min_size(
            origin + egui::vec2(x * scale, y * scale),
            egui::vec2(w * scale, h * scale),
        );
        painter.rect_filled(px, 0.0, color);
    }
    response
}

/// Paint a single-color (mono) pixel flame inside `rect` — the AI avatar
/// glyph in the Nexus chat view (white flame on the accent background).
pub fn paint_flame_mono_at(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    // viewBox 212.121×151.515 — keep the aspect, center vertically.
    let scale = (rect.width() / 212.121).min(rect.height() / 151.515);
    let fw = 212.121 * scale;
    let fh = 151.515 * scale;
    let origin = egui::pos2(
        rect.left() + (rect.width() - fw) / 2.0,
        rect.top() + (rect.height() - fh) / 2.0,
    );
    for (_, x, y, w, h) in flame_rects_mono() {
        let px = egui::Rect::from_min_size(
            origin + egui::vec2(x * scale, y * scale),
            egui::vec2(w * scale, h * scale),
        );
        painter.rect_filled(px, 0.0, color);
    }
}

/// The 10 pixel-flame rects (ChatView.tsx SVG), with palette colors.
fn flame_rects(p: &Palette) -> [(Color32, f32, f32, f32, f32); 10] {
    [
        (p.flame_yellowbright, 30.303, 0.0, 30.303, 30.303),
        (p.flame_yellowbright, 151.515, 0.0, 30.303, 30.303),
        (p.flame_yellow, 30.303, 30.303, 60.606, 30.303),
        (p.flame_yellow, 121.212, 30.303, 60.606, 30.303),
        (p.flame_orangebright, 30.303, 60.606, 151.515, 30.303),
        (p.flame_orange, 30.303, 90.909, 30.303, 30.303),
        (p.flame_orange, 90.909, 90.909, 30.303, 30.303),
        (p.flame_orange, 151.515, 90.909, 30.303, 30.303),
        (p.flame_red, 0.0, 121.212, 90.909, 30.303),
        (p.flame_red, 121.212, 121.212, 90.909, 30.303),
    ]
}

/// The same rects, color stripped (positions only).
fn flame_rects_mono() -> [(Color32, f32, f32, f32, f32); 10] {
    [
        (Color32::PLACEHOLDER, 30.303, 0.0, 30.303, 30.303),
        (Color32::PLACEHOLDER, 151.515, 0.0, 30.303, 30.303),
        (Color32::PLACEHOLDER, 30.303, 30.303, 60.606, 30.303),
        (Color32::PLACEHOLDER, 121.212, 30.303, 60.606, 30.303),
        (Color32::PLACEHOLDER, 30.303, 60.606, 151.515, 30.303),
        (Color32::PLACEHOLDER, 30.303, 90.909, 30.303, 30.303),
        (Color32::PLACEHOLDER, 90.909, 90.909, 30.303, 30.303),
        (Color32::PLACEHOLDER, 151.515, 90.909, 30.303, 30.303),
        (Color32::PLACEHOLDER, 0.0, 121.212, 90.909, 30.303),
        (Color32::PLACEHOLDER, 121.212, 121.212, 90.909, 30.303),
    ]
}
