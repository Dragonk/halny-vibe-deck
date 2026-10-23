//! Tiny painter-based charts for the Stats view — no dependencies:
//! cumulative area chart, bar chart, 12-week activity heatmap and a
//! donut chart with a legend.

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

const H: f32 = 110.0;

/// Cumulative area chart: filled area under the polyline + 2px outline +
/// the last value labelled in the top-right corner.
pub fn area_chart(
    ui: &mut Ui,
    id: &str,
    values: &[f64],
    color: Color32,
    fmt: &dyn Fn(f64) -> String,
) {
    let width = ui.available_width();
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, H), Sense::hover());
    let p = ui.painter_at(rect);
    let inner = rect.shrink(4.0);

    p.rect_filled(inner, egui::Rounding::same(4.0), color.gamma_multiply(0.06));

    if values.len() < 2 {
        label_center(&p, inner, &fmt(values.first().copied().unwrap_or(0.0)), color);
        return;
    }
    let max = values.iter().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let step = inner.width() / (values.len() - 1) as f32;
    let y_of = |v: f64| inner.bottom() - (v / max) as f32 * inner.height();

    // filled area
    let mut pts: Vec<Pos2> = Vec::with_capacity(values.len() + 2);
    for (i, v) in values.iter().enumerate() {
        pts.push(Pos2::new(inner.left() + step * i as f32, y_of(*v)));
    }
    let mut area = pts.clone();
    area.push(Pos2::new(inner.right(), inner.bottom()));
    area.push(Pos2::new(inner.left(), inner.bottom()));
    p.add(egui::Shape::convex_polygon(
        area,
        color.gamma_multiply(0.25),
        Stroke::NONE,
    ));
    // outline
    p.add(egui::Shape::line(pts, egui::Stroke::new(2.0_f32, color)));

    // last value label
    let last = values.last().copied().unwrap_or(0.0);
    let text = fmt(last);
    let galley = p.layout(text.clone(), FontId::proportional(11.0), color, f32::INFINITY);
    p.galley(
        Pos2::new(
            (inner.right() - galley.size().x - 2.0).max(inner.left()),
            inner.top(),
        ),
        galley,
        color,
    );
    let _ = id;
}

/// Vertical bars with x labels; `values` are summed per bucket by the caller.
pub fn bar_chart(
    ui: &mut Ui,
    id: &str,
    values: &[f64],
    color: Color32,
    fmt: &dyn Fn(f64) -> String,
) {
    let width = ui.available_width();
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, H), Sense::hover());
    let p = ui.painter_at(rect);
    let inner = rect.shrink(4.0);

    if values.is_empty() {
        label_center(&p, inner, "—", color);
        return;
    }
    let max = values.iter().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let n = values.len().max(1) as f32;
    let bar_w = (inner.width() / n * 0.7).max(2.0);
    let gap = inner.width() / n;
    let total: f64 = values.iter().sum();
    for (i, v) in values.iter().enumerate() {
        let x = inner.left() + gap * i as f32 + (gap - bar_w) / 2.0;
        let h = (*v / max) as f32 * inner.height();
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(x, inner.bottom() - h),
                Pos2::new(x + bar_w, inner.bottom()),
            ),
            egui::Rounding::same(2.0),
            color.gamma_multiply(if *v > 0.0 { 0.85 } else { 0.25 }),
        );
    }
    // total label
    let text = fmt(total);
    let galley = p.layout(text, FontId::proportional(11.0), color, f32::INFINITY);
    p.galley(Pos2::new(inner.right() - galley.size().x - 2.0, inner.top()), galley, color);
    let _ = id;
}

/// GitHub-style activity heatmap: `counts` (oldest first, `weeks` × 7),
/// 12 weeks by default. Monday-first rows.
pub fn heatmap(ui: &mut Ui, id: &str, counts: &[u32], weeks: usize, color: Color32) {
    let cell = 9.0;
    let gap = 2.5;
    let width = ui.available_width();
    let height = 7.0 * cell + 6.0 * gap + 14.0;
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let p = ui.painter_at(rect);

    let max = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    let x0 = rect.left() + 2.0;
    let y0 = rect.top() + 2.0;
    for w in 0..weeks {
        for d in 0..7 {
            let idx = w * 7 + d;
            let Some(c) = counts.get(idx) else { break };
            let intensity = 0.12 + 0.88 * (*c as f32 / max);
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(
                        x0 + w as f32 * (cell + gap),
                        y0 + d as f32 * (cell + gap),
                    ),
                    Vec2::splat(cell),
                ),
                egui::Rounding::same(2.0),
                color.gamma_multiply(intensity),
            );
        }
    }
    let _ = id;
}

/// Donut chart with a legend: `entries` are (value, color, label).
pub fn donut(ui: &mut Ui, id: &str, entries: &[(f64, Color32, String)]) {
    let measure = ui.painter().clone();
    let legend_w = entries
        .iter()
        .map(|(_, _, label)| {
            measure
                .layout(label.clone(), FontId::proportional(11.0), Color32::WHITE, f32::INFINITY)
                .size()
                .x
        })
        .fold(80.0_f32, f32::max)
        + 30.0;
    let radius = 40.0f32;
    let width = ui.available_width().min(radius * 2.0 + legend_w + 12.0);
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, radius * 2.0 + 8.0), Sense::hover());
    let p = ui.painter_at(rect);

    let total: f64 = entries.iter().map(|(v, _, _)| *v).sum();
    let center = Pos2::new(rect.left() + 6.0 + radius, rect.top() + 4.0 + radius);
    let mut angle = -90.0_f32;
    let mut legend_y = rect.top() + 6.0;
    for (v, color, label) in entries {
        let share = v / total.max(1e-9);
        let sweep = (share * 360.0) as f32;
        ring_segment(&p, center, radius, radius * 0.55, angle, angle + sweep, *color);
        let pct = share * 100.0;
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(center.x + radius + 12.0, legend_y + 1.0),
                Vec2::new(8.0, 8.0),
            ),
            egui::Rounding::same(2.0),
            *color,
        );
        let galley = p.layout(
            format!("{label} {pct:.0}%"),
            FontId::proportional(11.0),
            Color32::WHITE,
            f32::INFINITY,
        );
        p.galley(
            Pos2::new(center.x + radius + 25.0, legend_y),
            galley,
            ui.visuals().text_color(),
        );
        legend_y += 15.0;
        angle += sweep;
    }
    let _ = id;
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

fn label_center(p: &egui::Painter, rect: Rect, text: &str, color: Color32) {
    let galley = p.layout(text.to_string(), FontId::proportional(12.0), color, f32::INFINITY);
    p.galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}
