//! Nexus-style painter charts for the Stats view — no dependencies:
//! - area chart with Y grid + labels and X date labels (min-gap ticks),
//! - stacked bar chart with Y grid + labels,
//! - GitHub-style 13-week activity heatmap with five discrete levels,
//! - donut chart with the total in the center.

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

const CHART_H: f32 = 132.0;
const PAD_L: f32 = 44.0;
const PAD_R: f32 = 8.0;
const PAD_T: f32 = 8.0;
const PAD_B: f32 = 20.0;

fn muted(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color()
}

fn grid_stroke(ui: &Ui) -> Stroke {
    Stroke::new(
        0.5_f32,
        ui.visuals()
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(0.6),
    )
}

/// Y-axis: horizontal grid lines + value labels at 0/25/50/75/100 %.
fn y_axis(p: &egui::Painter, inner: Rect, ui: &Ui, max: f64, fmt: &dyn Fn(f64) -> String) {
    for t in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let y = inner.bottom() - t * inner.height();
        p.add(egui::Shape::line(
            vec![Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)],
            grid_stroke(ui),
        ));
        let g = p.layout(
            fmt(t as f64 * max),
            FontId::proportional(8.0),
            muted(ui),
            f32::INFINITY,
        );
        p.galley(
            Pos2::new(inner.left() - g.size().x - 4.0, y - g.size().y / 2.0),
            g,
            muted(ui),
        );
    }
}

/// X-axis tick indices by minimum pixel spacing; the last point is always shown.
fn x_ticks(n: usize, x_of: &dyn Fn(usize) -> f32, min_gap: f32) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut last = -9999.0_f32;
    for i in 0..n {
        let x = x_of(i);
        if x - last >= min_gap {
            out.push(i);
            last = x;
        }
    }
    if n > 0 && !out.contains(&(n - 1)) {
        let last_px = x_of(n - 1);
        match out.last_mut() {
            Some(l) if last_px - x_of(*l) < min_gap / 2.0 => *l = n - 1,
            Some(_) => out.push(n - 1),
            None => out.push(n - 1),
        }
    }
    out
}

fn center_label(p: &egui::Painter, rect: Rect, ui: &Ui, text: &str) {
    let g = p.layout(
        text.to_string(),
        FontId::proportional(11.0),
        muted(ui),
        f32::INFINITY,
    );
    p.galley(
        Pos2::new(
            rect.center().x - g.size().x / 2.0,
            rect.center().y - g.size().y / 2.0,
        ),
        g,
        muted(ui),
    );
}

/// Cumulative area chart with axes (Nexus `AreaChart`): gradient-ish fill,
/// 1.5px outline, Y grid + labels, X date labels.
pub fn area_chart(
    ui: &mut Ui,
    id: &str,
    values: &[f64],
    x_labels: &[String],
    color: Color32,
    fmt: &dyn Fn(f64) -> String,
) {
    let width = ui.available_width().max(180.0);
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, CHART_H), Sense::hover());
    let p = ui.painter_at(rect);
    let inner = Rect::from_min_max(
        Pos2::new(rect.left() + PAD_L, rect.top() + PAD_T),
        Pos2::new(rect.right() - PAD_R, rect.bottom() - PAD_B),
    );
    if values.len() < 2 {
        center_label(
            &p,
            inner,
            ui,
            &fmt(values.first().copied().unwrap_or(0.0)),
        );
        let _ = id;
        return;
    }
    let max = values.iter().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let step = inner.width() / (values.len() - 1) as f32;
    let x_of = |i: usize| inner.left() + step * i as f32;
    let y_of = |v: f64| inner.bottom() - (v / max) as f32 * inner.height();

    y_axis(&p, inner, ui, max, fmt);

    let mut pts: Vec<Pos2> = Vec::with_capacity(values.len() + 2);
    for (i, v) in values.iter().enumerate() {
        pts.push(Pos2::new(x_of(i), y_of(*v)));
    }
    let mut area = pts.clone();
    area.push(Pos2::new(inner.right(), inner.bottom()));
    area.push(Pos2::new(inner.left(), inner.bottom()));
    p.add(egui::Shape::convex_polygon(
        area,
        color.gamma_multiply(0.18),
        Stroke::NONE,
    ));
    p.add(egui::Shape::line(pts, Stroke::new(1.5_f32, color)));

    for i in x_ticks(values.len(), &x_of, 40.0) {
        if let Some(label) = x_labels.get(i) {
            let g = p.layout(label.clone(), FontId::proportional(8.0), muted(ui), f32::INFINITY);
            let x = (x_of(i) - g.size().x / 2.0).max(rect.left());
            p.galley(Pos2::new(x, inner.bottom() + 4.0), g, muted(ui));
        }
    }
    let _ = id;
}

/// Stacked bar chart with axes (Nexus `BarChart`): two stacked series,
/// Y grid + labels, X date labels with min-gap ticks.
#[allow(clippy::too_many_arguments)]
pub fn bar_chart(
    ui: &mut Ui,
    id: &str,
    series_a: &[f64],
    series_b: &[f64],
    color_a: Color32,
    color_b: Color32,
    x_labels: &[String],
    fmt: &dyn Fn(f64) -> String,
) {
    let width = ui.available_width().max(180.0);
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, CHART_H), Sense::hover());
    let p = ui.painter_at(rect);
    let inner = Rect::from_min_max(
        Pos2::new(rect.left() + PAD_L, rect.top() + PAD_T),
        Pos2::new(rect.right() - PAD_R, rect.bottom() - PAD_B),
    );
    let n = series_a.len().max(series_b.len());
    if n == 0 {
        center_label(&p, inner, ui, "—");
        let _ = id;
        return;
    }
    let totals: Vec<f64> = (0..n)
        .map(|i| {
            series_a.get(i).copied().unwrap_or(0.0) + series_b.get(i).copied().unwrap_or(0.0)
        })
        .collect();
    let max = totals.iter().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let gap = inner.width() / n as f32;
    let bar_w = (gap * 0.7).max(4.0);

    y_axis(&p, inner, ui, max, fmt);

    for i in 0..n {
        let x = inner.left() + gap * i as f32 + (gap - bar_w) / 2.0;
        let mut y_off = 0.0_f32;
        for (series, color) in [(series_a, color_a), (series_b, color_b)] {
            let Some(v) = series.get(i) else { continue };
            let h = (v / max) as f32 * inner.height();
            if h <= 0.0 {
                continue;
            }
            p.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x, inner.bottom() - y_off - h),
                    Pos2::new(x + bar_w, inner.bottom() - y_off),
                ),
                egui::Rounding::same(1.0),
                color,
            );
            y_off += h;
        }
    }

    let x_of = |i: usize| inner.left() + gap * i as f32 + gap * 0.5;
    for i in x_ticks(n, &x_of, 40.0) {
        if let Some(label) = x_labels.get(i) {
            let g = p.layout(label.clone(), FontId::proportional(8.0), muted(ui), f32::INFINITY);
            let x = (x_of(i) - g.size().x / 2.0).max(rect.left());
            p.galley(Pos2::new(x, inner.bottom() + 4.0), g, muted(ui));
        }
    }
    let _ = id;
}

/// GitHub-style activity heatmap: `counts` (oldest week first, `weeks` × 7,
/// Monday-first rows) with five discrete intensity levels like the Nexus CSS.
pub fn heatmap(ui: &mut Ui, id: &str, counts: &[u32], weeks: usize, color: Color32) {
    let cell = 10.0;
    let gap = 2.0;
    let height = 7.0 * cell + 6.0 * gap + 2.0;
    let width = weeks as f32 * (cell + gap);
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width + 4.0, height), Sense::hover());
    let p = ui.painter_at(rect);
    let x0 = rect.left() + 2.0;
    let y0 = rect.top() + 1.0;
    for w in 0..weeks {
        for d in 0..7 {
            let idx = w * 7 + d;
            let Some(&c) = counts.get(idx) else { continue };
            let level = if c == 0 {
                0.08
            } else if c == 1 {
                0.30
            } else if c <= 3 {
                0.50
            } else if c <= 6 {
                0.70
            } else {
                1.0
            };
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(
                        x0 + w as f32 * (cell + gap),
                        y0 + d as f32 * (cell + gap),
                    ),
                    Vec2::splat(cell),
                ),
                egui::Rounding::same(2.0),
                color.gamma_multiply(level),
            );
        }
    }
    let _ = id;
}

/// Donut chart (Nexus `DonutChart`): arcs with the total in the center.
/// `entries` are `(value, color, label)`; the legend is rendered by the caller.
pub fn donut(ui: &mut Ui, id: &str, entries: &[(f64, Color32, String)], center_text: &str, center_value: &str) {
    let radius = 58.0_f32;
    let inner_r = radius * 0.62;
    let width = ui.available_width().max(radius * 2.0 + 8.0);
    let (rect, _resp) =
        ui.allocate_exact_size(Vec2::new(width, radius * 2.0 + 4.0), Sense::hover());
    let p = ui.painter_at(rect);
    let center = Pos2::new(rect.center().x, rect.top() + radius + 2.0);
    let total: f64 = entries.iter().map(|(v, _, _)| v).sum();
    if total <= 0.0 {
        center_label(&p, rect, ui, "—");
        let _ = id;
        return;
    }
    let mut angle = -90.0_f32;
    for (v, color, _) in entries {
        if *v <= 0.0 {
            continue;
        }
        let sweep = (*v / total * 360.0) as f32;
        ring_segment(&p, center, radius, inner_r, angle, angle + sweep, *color);
        angle += sweep;
    }
    let g1 = p.layout(
        center_text.to_string(),
        FontId::proportional(10.0),
        muted(ui),
        f32::INFINITY,
    );
    p.galley(
        Pos2::new(center.x - g1.size().x / 2.0, center.y - g1.size().y - 2.0),
        g1,
        muted(ui),
    );
    let g2 = p.layout(
        center_value.to_string(),
        FontId::proportional(15.0),
        ui.visuals().text_color(),
        f32::INFINITY,
    );
    p.galley(
        Pos2::new(center.x - g2.size().x / 2.0, center.y + 2.0),
        g2,
        ui.visuals().text_color(),
    );
    let _ = id;
}

/// Legend row: colored dot + `label (value)`, horizontally wrapped.
pub fn legend_row(ui: &mut Ui, entries: &[(f64, Color32, String)]) {
    ui.horizontal_wrapped(|ui| {
        for (v, color, label) in entries {
            let (_, resp) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
            ui.painter()
                .rect_filled(resp.rect, egui::Rounding::same(2.0), *color);
            ui.label(
                egui::RichText::new(format!("{label} ({})", fmt_short(*v)))
                    .size(10.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);
        }
    });
}

fn fmt_short(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("{:.1}k", v / 1_000.0)
    } else {
        format!("{}", v.round() as i64)
    }
}

fn ring_segment(
    p: &egui::Painter,
    center: Pos2,
    r_out: f32,
    r_in: f32,
    a0: f32,
    a1: f32,
    color: Color32,
) {
    let steps = (((a1 - a0).abs() / 5.0).ceil() as usize).clamp(2, 96);
    let mut pts: Vec<Pos2> = Vec::with_capacity(steps * 2 + 2);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = (a0 + (a1 - a0) * t).to_radians();
        pts.push(center + Vec2::new(a.cos(), a.sin()) * r_out);
    }
    for i in (0..=steps).rev() {
        let t = i as f32 / steps as f32;
        let a = (a0 + (a1 - a0) * t).to_radians();
        pts.push(center + Vec2::new(a.cos(), a.sin()) * r_in);
    }
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}
