//! Built-in offline Mermaid renderer — no network, no external services.
//! Supports the diagram types agent answers actually use:
//! - flowcharts (`graph TD|LR`, `flowchart TD|LR`) with node shapes
//!   `[rect] (round) ([stadium]) {diamond} [(cylinder)] ((circle)) >flag]`
//!   and edges `--> --- -.-> ==> -->|label| A --> B & C`,
//! - sequence diagrams (participants, `->>` `-->>` messages, notes),
//! - pie charts (donut + legend).
//!
//! Anything else (`erDiagram`, `gantt`, …) returns `false` so the caller
//! can fall back to showing the source code.

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

/// Theme-dependent colors used by the diagrams.
pub struct Colors {
    pub bg: Color32,
    pub node: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub green: Color32,
    pub blue: Color32,
    pub yellow: Color32,
    pub red: Color32,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render a mermaid diagram. Returns `false` when the diagram type (or the
/// source) is not supported — the caller should then show the raw code.
pub fn render(ui: &mut Ui, code: &str) -> bool {
    let colors = crate::theme::theme_palette(ui);
    let Some(diagram) = parse(code) else {
        return false;
    };
    match diagram {
        Diagram::Flow(fc) => draw_flow(ui, &fc, &colors),
        Diagram::Seq(seq) => draw_sequence(ui, &seq, &colors),
        Diagram::Pie(pie) => draw_pie(ui, &pie, &colors),
    }
    true
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

enum Diagram {
    Flow(FlowChart),
    Seq(Sequence),
    Pie(Pie),
}

struct FlowChart {
    lr: bool,
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
}

struct FlowNode {
    id: String,
    label: String,
    shape: Shape,
}

#[derive(Clone, Copy, PartialEq)]
#[derive(Debug)]
enum Shape {
    Rect,
    Round,
    Stadium,
    Diamond,
    Cylinder,
    Circle,
    Flag,
}

struct FlowEdge {
    from: usize,
    to: usize,
    label: Option<String>,
    dashed: bool,
    thick: bool,
}

struct Sequence {
    participants: Vec<String>,
    steps: Vec<SeqStep>,
}

enum SeqStep {
    Message {
        from: usize,
        to: usize,
        text: String,
        dashed: bool,
    },
    Note {
        over: (usize, usize),
        text: String,
    },
}

struct Pie {
    title: Option<String>,
    slices: Vec<(String, f64)>,
}

fn parse(code: &str) -> Option<Diagram> {
    let mut lines = code
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let first = lines.next()?;
    let rest: Vec<&str> = lines.collect();

    let header: Vec<String> = first
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    match header.first().map(String::as_str) {
        Some("graph") | Some("flowchart") => parse_flow(&header, &rest),
        Some("sequencediagram") => parse_sequence(&rest),
        Some("pie") => {
            let after = first.split_once(' ').map(|(_, r)| r).unwrap_or("");
            let mut lines: Vec<&str> = Vec::new();
            if !after.is_empty() {
                lines.push(after);
            }
            lines.extend(rest);
            parse_pie(&lines)
        }
        _ => None,
    }
}

// -- flowchart ----------------------------------------------------------------

fn parse_flow(header: &[String], lines: &[&str]) -> Option<Diagram> {
    let lr = matches!(header.get(1).map(String::as_str), Some("lr"));
    let mut fc = FlowChart {
        lr,
        nodes: Vec::new(),
        edges: Vec::new(),
    };

    let node_index = |fc: &mut FlowChart, id: String, label: String, shape: Shape| -> usize {
        if let Some(i) = fc.nodes.iter().position(|n| n.id == id) {
            // refine an existing bare node if it gets a label now
            if !label.is_empty() && fc.nodes[i].label == fc.nodes[i].id {
                fc.nodes[i].label = label;
                fc.nodes[i].shape = shape;
            }
            i
        } else {
            fc.nodes.push(FlowNode {
                label: if label.is_empty() { id.clone() } else { label },
                id,
                shape,
            });
            fc.nodes.len() - 1
        }
    };

    for line in lines {
        let line = line.trim();
        if line.starts_with("subgraph")
            || line.starts_with("end")
            || line.starts_with("classDef")
            || line.starts_with("class ")
            || line.starts_with("style ")
            || line.starts_with("click")
        {
            continue; // container/styling directives — skip gracefully
        }
        // edge?
        if let Some((from_tok, to_toks, edge)) = parse_flow_edge(line) {
            let (fid, flabel, fshape) = parse_flow_node(&from_tok);
            let from = node_index(&mut fc, fid, flabel, fshape);
            for to_tok in to_toks {
                let (tid, tlabel, tshape) = parse_flow_node(&to_tok);
                let to = node_index(&mut fc, tid, tlabel, tshape);
                fc.edges.push(FlowEdge {
                    from,
                    to,
                    label: edge.0.clone(),
                    dashed: edge.1,
                    thick: edge.2,
                });
            }
            continue;
        }
        // standalone node definition
        if let Some((id, label, shape)) = parse_flow_node(line).valid() {
            node_index(&mut fc, id, label, shape);
        }
    }

    if fc.nodes.is_empty() {
        return None;
    }
    Some(Diagram::Flow(fc))
}

/// A parsed flow node token: `id`, `id[Label]`, `id(Label)`, …
fn parse_flow_node(tok: &str) -> (String, String, Shape) {
    let tok = tok.trim();
    let id_end = tok
        .find(|c: char| "([{<".contains(c))
        .unwrap_or(tok.len());
    let id: String = tok[..id_end].trim().to_string();
    if id_end >= tok.len() {
        return (id, String::new(), Shape::Round);
    }
    let open = tok.as_bytes()[id_end] as char;
    let (shape, close) = match open {
        '[' => {
            // `[(…)]` cylinder
            if tok[id_end + 1..].starts_with('(') {
                let rest = &tok[id_end + 1..];
                if let Some(ic) = find_close(rest, ')') {
                    if rest[ic + 1..].starts_with(']') {
                        let label = rest[1..ic].to_string();
                        return (id, label, Shape::Cylinder);
                    }
                }
            }
            (Shape::Rect, ']')
        }
        '(' => {
            let rest = &tok[id_end..];
            if rest.starts_with("((") {
                if let Some(ic) = find_close(&rest[2..], ')') {
                    if rest[2 + ic + 1..].starts_with(')') {
                        return (id, rest[2..2 + ic].to_string(), Shape::Circle);
                    }
                }
            }
            // `([…])` stadium
            if rest.starts_with("([") {
                if let Some(ic) = find_close(&rest[2..], ']') {
                    if rest[2 + ic + 1..].starts_with(')') {
                        return (id, rest[2..2 + ic].to_string(), Shape::Stadium);
                    }
                }
            }
            (Shape::Round, ')')
        }
        '{' => (Shape::Diamond, '}'),
        '<' => (Shape::Flag, ']'),
        _ => (Shape::Round, ']'),
    };
    let start = id_end + 1;
    let label = tok[start..].trim_end_matches(close).to_string();
    (id, label, shape)
}

fn find_close(s: &str, close: char) -> Option<usize> {
    s.find(close)
}

/// `A --> B`, `A -->|label| B`, `A -.-> B`, `A ==> B`, `A --> B & C`
/// → (from_token, vec of to_tokens, (label, dashed, thick))
fn parse_flow_edge(line: &str) -> Option<(String, Vec<String>, (Option<String>, bool, bool))> {
    // longest tokens first
    let (token, dashed, thick) = [
        ("==>", false, true),
        ("-.->", true, false),
        ("-->", false, false),
        ("---", false, false),
        ("-.", true, false),
    ]
    .iter()
    .find_map(|(t, d, k)| line.contains(t).then_some((*t, *d, *k)))?;

    let (left, right) = line.split_once(token)?;
    let left = left.trim().trim_end_matches('-').trim();
    let mut right = right.trim();

    let mut label = None;
    // trailing `|label|` comes AFTER the edge: A -->|text| B — but we split on
    // the edge token, so the label is at the start of `right`.
    if let Some(rest) = right.strip_prefix('|') {
        if let Some(end) = rest.find('|') {
            label = Some(rest[..end].to_string());
            right = rest[end + 1..].trim();
        }
    }

    let to_toks: Vec<String> = right
        .split(|c| c == '&' || c == ',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if left.is_empty() || to_toks.is_empty() {
        return None;
    }
    Some((left.to_string(), to_toks, (label, dashed, thick)))
}

trait FlowNodeParsed {
    fn valid(self) -> Option<(String, String, Shape)>;
}
impl FlowNodeParsed for (String, String, Shape) {
    fn valid(self) -> Option<(String, String, Shape)> {
        (!self.0.is_empty()).then_some(self)
    }
}

// -- sequence ------------------------------------------------------------------

fn parse_sequence(lines: &[&str]) -> Option<Diagram> {
    let mut seq = Sequence {
        participants: Vec::new(),
        steps: Vec::new(),
    };
    let participant = |seq: &mut Sequence, name: &str| -> usize {
        if let Some(i) = seq.participants.iter().position(|p| p == name) {
            i
        } else {
            seq.participants.push(name.to_string());
            seq.participants.len() - 1
        }
    };
    for line in lines {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("participant ")
            .or_else(|| line.strip_prefix("actor "))
        {
            participant(&mut seq, rest.trim().trim_matches('"'));
            continue;
        }
        // Note over A,B: text / Note over A: text / Note left of A: …
        if let Some(rest) = line.strip_prefix("Note ") {
            let (kind, after) = match rest.split_once(" of ").map(|(k, a)| {
                (k.trim(), a)
            }) {
                Some(x) => x,
                None => match rest.strip_prefix("over ") {
                    Some(a) => ("over", a),
                    None => continue,
                },
            };
            let _ = kind;
            let Some((names, text)) = after.split_once(':') else {
                continue;
            };
            let text = text.trim().to_string();
            if kind == "over" {
                let ids: Vec<&str> = names.split(',').map(str::trim).collect();
                if ids.is_empty() {
                    continue;
                }
                let a = participant(&mut seq, ids[0]);
                let b = ids
                    .get(1)
                    .map(|n| participant(&mut seq, n))
                    .unwrap_or(a);
                seq.steps.push(SeqStep::Note {
                    over: (a.min(b), a.max(b)),
                    text,
                });
            } else {
                // left/right of — attach a note spanning one participant
                let Some(name) = names.split(',').next() else { continue };
                let a = participant(&mut seq, name.trim());
                seq.steps.push(SeqStep::Note {
                    over: (a, a),
                    text,
                });
            }
            continue;
        }
        // messages: A->>B: text, A-->>B: text, A->B, A--)B …
        let (arrow_pos, dashed) = find_seq_arrow(line)?;
        let left = line[..arrow_pos.0].trim();
        let right_raw = &line[arrow_pos.1..];
        let (target, text) = match right_raw.split_once(':') {
            Some((t, txt)) => (t.trim(), txt.trim().to_string()),
            None => (right_raw.trim(), String::new()),
        };
        if left.is_empty() || target.is_empty() {
            continue;
        }
        let from = participant(&mut seq, left);
        let to = participant(&mut seq, target.trim_end_matches(')'));
        seq.steps.push(SeqStep::Message {
            from,
            to,
            text,
            dashed,
        });
    }
    if seq.participants.is_empty() {
        return None;
    }
    Some(Diagram::Seq(seq))
}

/// Find `->>`, `-->>`, `->`, `-->`, `--` in a line.
/// Returns (byte_range of the arrow, dashed).
fn find_seq_arrow(line: &str) -> Option<((usize, usize), bool)> {
    for (tok, dashed) in [("-->>", true), ("->>", false), ("-->", true), ("->", false), ("--", true)] {
        if let Some(p) = line.find(tok) {
            // don't confuse `--` in front of a `>` … keep simple matching order
            return Some(((p, p + tok.len()), dashed));
        }
    }
    None
}

// -- pie -----------------------------------------------------------------------

fn parse_pie(lines: &[&str]) -> Option<Diagram> {
    let mut pie = Pie {
        title: None,
        slices: Vec::new(),
    };
    for line in lines {
        if let Some(rest) = line.strip_prefix("title") {
            pie.title = Some(rest.trim().to_string());
            continue;
        }
        let Some((label, value)) = line.split_once(':') else {
            continue;
        };
        let Ok(v) = value.trim().parse::<f64>() else {
            continue;
        };
        pie.slices.push((label.trim().trim_matches('"').to_string(), v));
    }
    if pie.slices.is_empty() {
        return None;
    }
    Some(Diagram::Pie(pie))
}

// ---------------------------------------------------------------------------
// Shared painter helpers
// ---------------------------------------------------------------------------

fn lerp_pos(a: Pos2, b: Pos2, t: f32) -> Pos2 {
    Pos2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

struct Painter {
    p: egui::Painter,
    off: Vec2,
}

impl Painter {
    fn line(&self, a: Pos2, b: Pos2, stroke: Stroke) {
        self.p.line_segment([a + self.off, b + self.off], stroke);
    }

    fn dashed_line(&self, a: Pos2, b: Pos2, stroke: Stroke) {
        let a = a + self.off;
        let b = b + self.off;
        let len = (b - a).length();
        if len <= 0.0 {
            return;
        }
        let dash = 5.0;
        let n = (len / dash).ceil() as i32;
        for i in 0..n {
            let t0 = (i as f32 * dash) / len;
            let t1 = ((i as f32 + 0.6) * dash) / len;
            if t0 > 1.0 {
                break;
            }
            self.p
                .line_segment([lerp_pos(a, b, t0), lerp_pos(a, b, t1.min(1.0))], stroke);
        }
    }

    fn rect(&self, r: Rect, rounding: f32, fill: Color32, stroke: Color32) {
        self.p.rect(
            r.translate(self.off),
            egui::Rounding::same(rounding),
            fill,
            egui::Stroke::new(1.0_f32, stroke),
        );
    }

    fn text_size(&self, text: &str, font: FontId) -> Vec2 {
        self.p
            .layout(text.to_string(), font, Color32::WHITE, f32::INFINITY)
            .size()
    }

    fn text(&self, pos: Pos2, text: &str, font: FontId, color: Color32) {
        let galley = self.p.layout(text.to_string(), font, color, f32::INFINITY);
        self.p.galley(pos + self.off, galley, color);
    }

    fn arrow(&self, tip: Pos2, dir: Vec2, color: Color32) {
        let dir = dir.normalized();
        let perp = Vec2::new(-dir.y, dir.x);
        let a = tip - dir * 9.0 + perp * 4.0;
        let b = tip - dir * 9.0 - perp * 4.0;
        self.p.add(egui::Shape::convex_polygon(
            vec![tip + self.off, a + self.off, b + self.off],
            color,
            Stroke::NONE,
        ));
    }
}

const NODE_FONT: f32 = 12.0;
const COL_GAP: f32 = 40.0;

fn node_size(p: &Painter, label: &str, shape: Shape) -> Vec2 {
    let ts = p.text_size(label, FontId::proportional(NODE_FONT));
    let pad = match shape {
        Shape::Diamond => 18.0,
        Shape::Circle => 14.0,
        _ => 10.0,
    };
    Vec2::new(ts.x + pad * 2.0, ts.y + 12.0)
}

/// Draw one node shape, returning its rect.
fn draw_node(p: &Painter, center: Pos2, size: Vec2, label: &str, shape: Shape, c: &Colors) {
    let half = size / 2.0;
    let rect = Rect::from_center_size(center, size);
    let border = c.border;
    match shape {
        Shape::Diamond => {
            let pts = [
                center + Vec2::new(half.x, 0.0),
                center + Vec2::new(0.0, half.y),
                center - half,
                center + Vec2::new(-half.x, 0.0),
            ];
            p.p.add(egui::Shape::convex_polygon(
                pts.iter().map(|x| *x + p.off).collect(),
                c.node,
                egui::Stroke::new(1.0_f32, border),
            ));
        }
        Shape::Circle => {
            p.p.circle_filled(center + p.off, half.x.min(half.y), c.node);
            p.p.circle_stroke(
                center + p.off,
                half.x.min(half.y),
                egui::Stroke::new(1.0_f32, border),
            );
        }
        Shape::Cylinder => {
            p.rect(rect, 4.0, c.node, border);
            let ry = 4.0;
            p.p.add(egui::Shape::ellipse_filled(
                Pos2::new(center.x, rect.top() + p.off.y + ry) + Vec2::new(p.off.x, 0.0),
                Vec2::new(half.x, ry),
                c.node,
            ));
            p.p.add(egui::Shape::ellipse_stroke(
                Pos2::new(rect.left() + half.x + p.off.x, rect.bottom() - ry + p.off.y),
                Vec2::new(half.x, ry),
                egui::Stroke::new(1.0_f32, border),
            ));
        }
        Shape::Flag => {
            // asymmetric flag: right side pointed
            let pts = [
                rect.left_top(),
                Pos2::new(rect.right(), rect.top() + half.y),
                rect.left_bottom(),
            ];
            p.p.add(egui::Shape::convex_polygon(
                pts.iter().map(|x| *x + p.off).collect(),
                c.node,
                egui::Stroke::new(1.0_f32, border),
            ));
        }
        Shape::Stadium => {
            p.rect(rect, half.y.min(half.x), c.node, border);
        }
        Shape::Round => {
            p.rect(rect, 7.0, c.node, border);
        }
        Shape::Rect => {
            p.rect(rect, 2.0, c.node, border);
        }
    }
    p.text(
        Pos2::new(center.x - half.x, center.y - half.y * 0.6),
        label,
        FontId::proportional(NODE_FONT),
        c.text,
    );
}

// ---------------------------------------------------------------------------
// Flowchart drawing
// ---------------------------------------------------------------------------

fn draw_flow(ui: &mut Ui, fc: &FlowChart, c: &Colors) {
    let measure = Painter {
        p: ui.painter().clone(),
        off: Vec2::ZERO,
    };
    let sizes: Vec<Vec2> = fc
        .nodes
        .iter()
        .map(|n| node_size(&measure, &n.label, n.shape))
        .collect();

    // layer assignment: longest path from roots
    let n = fc.nodes.len();
    let mut layer = vec![0usize; n];
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &fc.edges {
        if !incoming[e.to].contains(&e.from) {
            incoming[e.to].push(e.from);
        }
    }
    // iterate until stable (max n passes)
    for _ in 0..n {
        let mut changed = false;
        for i in 0..n {
            let want = incoming
                .get(i)
                .map(|ins| ins.iter().map(|&p| layer[p] + 1).max().unwrap_or(0))
                .unwrap_or(0);
            if want > layer[i] {
                layer[i] = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let layers = layer.iter().copied().max().unwrap_or(0) + 1;
    // group node indices by layer, in first-seen order
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for (i, l) in layer.iter().enumerate() {
        rows[*l].push(i);
    }

    // positions: sequential with a gap, then centered per row
    let mut centers = vec![Pos2::ZERO; n];
    let row_gap = 84.0;
    let mut max_extent = 0.0f32;
    for (r, idxs) in rows.iter().enumerate() {
        let total: f32 = idxs
            .iter()
            .map(|&i| sizes[i].x)
            .sum::<f32>()
            + COL_GAP * idxs.len().saturating_sub(1) as f32;
        max_extent = max_extent.max(total);
        let mut x = 0.0f32;
        let y = 20.0 + r as f32 * row_gap + sizes[idxs[0]].y * 0.5;
        for &i in idxs {
            centers[i] = Pos2::new(x + sizes[i].x * 0.5, y);
            x += sizes[i].x + COL_GAP;
        }
    }
    let content_w = max_extent;
    let content_h = 20.0 + rows.len() as f32 * row_gap;

    let width = ui.available_width().min(content_w + 16.0).max(120.0);
    let size = Vec2::new(width, content_h + 12.0);
    let (rect, _resp) = ui.allocate_exact_size(size, Sense::hover());
    let off = Vec2::new((width - content_w) / 2.0, 0.0) + rect.min.to_vec2();
    let p = Painter {
        p: ui.painter_at(rect),
        off,
    };

    // edges first (under the nodes)
    for e in &fc.edges {
        let (a, b) = (centers[e.from], centers[e.to]);
        let (sa, sb) = (sizes[e.from], sizes[e.to]);
        let (start, end, dir) = if fc.lr {
            let s = Pos2::new(a.x + sa.x / 2.0, a.y);
            let t = Pos2::new(b.x - sb.x / 2.0, b.y);
            let dir = (t - s).normalized();
            (s, t, dir)
        } else {
            let s = Pos2::new(a.x, a.y + sa.y / 2.0);
            let t = Pos2::new(b.x, b.y - sb.y / 2.0);
            let dir = (t - s).normalized();
            (s, t, dir)
        };
        let stroke = egui::Stroke::new(
            if e.thick { 2.5_f32 } else { 1.4_f32 },
            if e.label.is_some() { c.accent } else { c.muted },
        );
        if e.dashed {
            p.dashed_line(start, end, stroke);
        } else {
            p.line(start, end, stroke);
        }
        p.arrow(end, dir, stroke.color);
        if let Some(label) = &e.label {
            let mid = lerp_pos(start, end, 0.5);
            let ts = p.text_size(label, FontId::proportional(11.0));
            let r = Rect::from_center_size(mid, ts + Vec2::new(10.0, 6.0));
            p.rect(r, 4.0, c.bg, c.border);
            p.text(
                Pos2::new(r.left() + 5.0, r.top() + 3.0),
                label,
                FontId::proportional(11.0),
                c.text,
            );
        }
    }

    // nodes
    for (i, node) in fc.nodes.iter().enumerate() {
        draw_node(&p, centers[i], sizes[i], &node.label, node.shape, c);
    }
}

// ---------------------------------------------------------------------------
// Sequence drawing
// ---------------------------------------------------------------------------

fn draw_sequence(ui: &mut Ui, seq: &Sequence, c: &Colors) {
    let measure = Painter {
        p: ui.painter().clone(),
        off: Vec2::ZERO,
    };
    let n = seq.participants.len().max(1);
    let heads: Vec<Vec2> = seq
        .participants
        .iter()
        .map(|name| measure.text_size(name, FontId::proportional(NODE_FONT)))
        .collect();
    let head_w: f32 = heads
        .iter()
        .map(|s| s.x + 22.0)
        .fold(70.0_f32, f32::max);
    let col_w = head_w + 46.0;
    let total_w = col_w * n as f32 + 20.0;
    let step_h = 34.0;
    let total_h = 64.0 + seq.steps.len() as f32 * step_h + 8.0;

    let width = ui.available_width().min(total_w + 8.0).max(160.0);
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, total_h), Sense::hover());
    let off = Vec2::new(10.0 + (width - total_w).max(0.0) / 2.0, 0.0) + rect.min.to_vec2();
    let p = Painter {
        p: ui.painter_at(rect),
        off,
    };

    let x_at = |i: usize| -> f32 { 10.0 + col_w * i as f32 + col_w / 2.0 };
    let top = 8.0;

    // participant headers
    for (i, name) in seq.participants.iter().enumerate() {
        let cx = x_at(i);
        let r = Rect::from_center_size(
            Pos2::new(cx, top + 16.0),
            Vec2::new(head_w, 30.0),
        );
        p.rect(r, 5.0, c.node, c.border);
        p.text(
            Pos2::new(r.left() + 11.0, r.top() + 8.0),
            name,
            FontId::proportional(NODE_FONT),
            c.text,
        );
    }
    // lifelines
    let line_stroke = egui::Stroke::new(1.0_f32, c.border);
    for i in 0..n {
        p.dashed_line(
            Pos2::new(x_at(i), top + 32.0),
            Pos2::new(x_at(i), total_h - 6.0),
            line_stroke,
        );
    }

    // steps
    let mut y = top + 52.0;
    for step in &seq.steps {
        match step {
            SeqStep::Message {
                from,
                to,
                text,
                dashed,
            } => {
                let (x0, x1) = (x_at(*from), x_at(*to));
                let color = if *dashed { c.muted } else { c.accent };
                let stroke = egui::Stroke::new(1.4_f32, color);
                let start = Pos2::new(x0.min(x1), y);
                let end = Pos2::new(x0.max(x1), y);
                if *dashed {
                    p.dashed_line(start, end, stroke);
                } else {
                    p.line(start, end, stroke);
                }
                let dir = if x1 >= x0 {
                    Vec2::new(1.0, 0.0)
                } else {
                    Vec2::new(-1.0, 0.0)
                };
                p.arrow(end, dir, color);
                if !text.is_empty() {
                    let ts = p.text_size(text, FontId::proportional(11.0));
                    let cx = (start.x + end.x) / 2.0 - ts.x / 2.0;
                    p.text(
                        Pos2::new(cx, y - 18.0),
                        text,
                        FontId::proportional(11.0),
                        c.text,
                    );
                }
            }
            SeqStep::Note { over, text } => {
                let (x0, x1) = (x_at(over.0), x_at(over.1));
                let left = x0.min(x1) - col_w / 2.0 + 8.0;
                let right = x0.max(x1) + col_w / 2.0 - 8.0;
                let ts = p.text_size(text, FontId::proportional(11.0));
                let r = Rect::from_min_max(
                    Pos2::new(left, y - 4.0),
                    Pos2::new(right.max(left + ts.x + 12.0), y + 16.0),
                );
                p.rect(r, 4.0, c.yellow.gamma_multiply(0.25), c.yellow);
                p.text(
                    Pos2::new(r.left() + 6.0, r.top() + 3.0),
                    text,
                    FontId::proportional(11.0),
                    c.text,
                );
            }
        }
        y += step_h;
    }
}

// ---------------------------------------------------------------------------
// Pie drawing
// ---------------------------------------------------------------------------

fn draw_pie(ui: &mut Ui, pie: &Pie, c: &Colors) {
    let measure = Painter {
        p: ui.painter().clone(),
        off: Vec2::ZERO,
    };
    let palette = [c.accent, c.green, c.blue, c.yellow, c.red, c.muted];
    let total: f64 = pie.slices.iter().map(|(_, v)| *v).sum();
    let legend_w: f32 = pie
        .slices
        .iter()
        .map(|(label, v)| {
            measure
                .text_size(
                    &format!("{label} {:.0}%", v / total.max(1.0) * 100.0),
                    FontId::proportional(11.0),
                )
                .x
        })
        .fold(120.0_f32, f32::max)
        + 24.0;
    let title_h = pie.title.is_some() as u32 as f32 * 22.0;
    let radius = 52.0f32;
    let height = radius * 2.0 + 16.0 + title_h;

    let width = ui.available_width().min(radius * 2.0 + legend_w + 24.0).max(180.0);
    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let p = Painter {
        p: ui.painter_at(rect),
        off: rect.min.to_vec2(),
    };

    let center = Pos2::new(12.0 + radius, 10.0 + title_h + radius);
    if let Some(title) = &pie.title {
        p.text(
            Pos2::new(12.0, 2.0),
            title,
            FontId::proportional(13.0),
            c.text,
        );
    }

    let mut angle: f32 = -90.0;
    let mut legend_y = 10.0 + title_h;
    for (i, (label, v)) in pie.slices.iter().enumerate() {
        let share = v / total.max(1.0);
        let sweep = (share * 360.0) as f32;
        let color = palette[i % palette.len()];
        donut_segment(&p, center, radius, radius * 0.55, angle, angle + sweep, color);
        // legend
        let pct = share * 100.0;
        p.p.rect_filled(
            Rect::from_min_size(
                Pos2::new(center.x + radius + 18.0, legend_y + 2.0) + p.off,
                Vec2::new(9.0, 9.0),
            ),
            egui::Rounding::same(2.0),
            color,
        );
        p.text(
            Pos2::new(center.x + radius + 32.0, legend_y),
            &format!("{label} {pct:.0}%"),
            FontId::proportional(11.0),
            c.text,
        );
        legend_y += 17.0;
        angle += sweep;
    }
}

/// One donut ring segment, drawn as a convex polygon (outer + inner arc).
fn donut_segment(
    p: &Painter,
    center: Pos2,
    r_out: f32,
    r_in: f32,
    a0: f32,
    a1: f32,
    color: Color32,
) {
    let steps = (((a1 - a0).abs() / 6.0).ceil() as usize).clamp(2, 64);
    let mut pts: Vec<Pos2> = Vec::with_capacity(steps * 2 + 2);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = (a0 + (a1 - a0) * t).to_radians();
        pts.push(
            center
                + Vec2::new(a.cos(), a.sin()) * r_out
                + p.off,
        );
    }
    for i in (0..=steps).rev() {
        let t = i as f32 / steps as f32;
        let a = (a0 + (a1 - a0) * t).to_radians();
        pts.push(
            center
                + Vec2::new(a.cos(), a.sin()) * r_in
                + p.off,
        );
    }
    p.p
        .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flow_basic() {
        let d = parse("graph TD\nA[Start] --> B{Choice}\nB -->|yes| C[OK]").unwrap();
        match d {
            Diagram::Flow(fc) => {
                assert_eq!(fc.nodes.len(), 3);
                assert_eq!(fc.edges.len(), 2);
                assert_eq!(fc.edges[1].label.as_deref(), Some("yes"));
                assert_eq!(fc.nodes[1].shape, Shape::Diamond);
            }
            _ => panic!("expected flowchart"),
        }
    }

    #[test]
    fn parse_flow_multi_target() {
        let d = parse("flowchart LR\nA --> B & C").unwrap();
        match d {
            Diagram::Flow(fc) => {
                assert_eq!(fc.edges.len(), 2);
                assert!(fc.lr);
            }
            _ => panic!("expected flowchart"),
        }
    }

    #[test]
    fn parse_sequence_basic() {
        let d = parse("sequenceDiagram\nAlice->>Bob: Hi\nBob-->>Alice: Hello!").unwrap();
        match d {
            Diagram::Seq(seq) => {
                assert_eq!(seq.participants.len(), 2);
                assert_eq!(seq.steps.len(), 2);
            }
            _ => panic!("expected sequence"),
        }
    }

    #[test]
    fn parse_pie_basic() {
        let d = parse("pie title Pets\n\"Dogs\": 40\n\"Cats\": 60").unwrap();
        match d {
            Diagram::Pie(pie) => {
                assert_eq!(pie.slices.len(), 2);
                assert_eq!(pie.title.as_deref(), Some("Pets"));
            }
            _ => panic!("expected pie"),
        }
    }

    #[test]
    fn unsupported_returns_none() {
        assert!(parse("erDiagram\nUSER ||--o{ POST : owns").is_none());
        assert!(parse("gantt\n title x").is_none());
    }

    #[test]
    fn node_shapes() {
        let (_, label, shape) = parse_flow_node("A[(DB)]");
        assert_eq!(label, "DB");
        assert_eq!(shape, Shape::Cylinder);
        let (_, label, shape) = parse_flow_node("A((O))");
        assert_eq!(label, "O");
        assert_eq!(shape, Shape::Circle);
        let (id, label, _) = parse_flow_node("Bare");
        assert_eq!(id, "Bare");
        assert!(label.is_empty());
    }
}
