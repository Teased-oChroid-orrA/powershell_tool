//! Interactive force-directed "brain map" for search results - an
//! alternate view next to the plain list, built from data the engine
//! already returns (`FileSearchResult`/`LineHit::matched_filters`), not
//! a decorative layout. Bipartite graph: one node per result file, one
//! node per distinct matched-filter string, an edge for every file/
//! filter pair that actually matched in that file. This directly
//! answers "which files matched which terms, and how" - the real
//! question a flat list leaves implicit once more than one filter is in
//! play.
//!
//! egui has no built-in graph/scene widget (confirmed against docs.rs
//! before writing this) - pan/zoom and the physics are hand-rolled, same
//! "own `Painter` + manual screen transform" shape `sketches.rs::Sketch`
//! already uses, extended with drag/scroll input and a continuous-
//! repaint simulation loop that stops once the layout settles (so it
//! isn't burning CPU sitting idle - `ctx.request_repaint()` is only
//! called while any node's velocity is still above a small epsilon).

use std::collections::HashMap;

use eframe::egui::{self, Color32, FontId, Pos2, Sense, Stroke, Vec2};

use search_core::models::FileSearchResult;

use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    File,
    Filter,
}

struct Node {
    kind: NodeKind,
    label: String,
    full_path: String,
    hit_count: usize,
    pos: Pos2,
    vel: Vec2,
    pinned: bool,
}

pub struct GraphState {
    nodes: Vec<Node>,
    edges: Vec<(usize, usize)>,
    /// Cheap fingerprint of the result set this layout was built for -
    /// results are replaced wholesale by every search run, never mutated
    /// in place, so count + total hit count is enough to detect "the
    /// results actually changed" without a full content hash.
    built_for: u64,
    pan: Vec2,
    zoom: f32,
    dragging: Option<usize>,
    settled: bool,
}

impl Default for GraphState {
    fn default() -> Self {
        Self { nodes: Vec::new(), edges: Vec::new(), built_for: u64::MAX, pan: Vec2::ZERO, zoom: 1.0, dragging: None, settled: true }
    }
}

impl GraphState {
    fn sync(&mut self, results: &[FileSearchResult]) {
        let fingerprint = results.len() as u64 ^ results.iter().map(|r| r.hits.len() as u64).sum::<u64>().wrapping_mul(2654435761);
        if fingerprint == self.built_for {
            return;
        }
        self.built_for = fingerprint;
        self.nodes.clear();
        self.edges.clear();
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;

        // Seed on a ring rather than a random pile - the physics sim
        // converges faster from an already-spread-out start, and stays
        // stable (no first-frame flash of everything piled at the
        // origin) even before it's had a chance to settle.
        let mut filter_index: HashMap<&str, usize> = HashMap::new();
        for (i, r) in results.iter().enumerate() {
            let angle = i as f32 / results.len().max(1) as f32 * std::f32::consts::TAU;
            let radius = 180.0 + (r.hits.len() as f32).sqrt() * 6.0;
            self.nodes.push(Node {
                kind: NodeKind::File,
                label: file_name(&r.full_name).to_string(),
                full_path: r.full_name.clone(),
                hit_count: r.hits.len(),
                pos: Pos2::new(angle.cos() * radius, angle.sin() * radius),
                vel: Vec2::ZERO,
                pinned: false,
            });
            let file_idx = i;
            for hit in &r.hits {
                for filt in &hit.matched_filters {
                    let filt_idx = match filter_index.get(filt.as_str()) {
                        Some(&idx) => idx,
                        None => {
                            let idx = self.nodes.len();
                            self.nodes.push(Node {
                                kind: NodeKind::Filter,
                                label: filt.clone(),
                                full_path: String::new(),
                                hit_count: 0,
                                pos: Pos2::ZERO,
                                vel: Vec2::ZERO,
                                pinned: false,
                            });
                            filter_index.insert(filt.as_str(), idx);
                            idx
                        }
                    };
                    self.nodes[filt_idx].hit_count += 1;
                    if !self.edges.contains(&(file_idx, filt_idx)) {
                        self.edges.push((file_idx, filt_idx));
                    }
                }
            }
        }
        // Filter nodes are hubs - fewer of them, placed on a small inner
        // ring so files naturally radiate outward from the terms that
        // matched them once the springs pull taut.
        let filter_positions: Vec<usize> = self.nodes.iter().enumerate().filter(|(_, n)| n.kind == NodeKind::Filter).map(|(i, _)| i).collect();
        let count = filter_positions.len().max(1);
        for (k, idx) in filter_positions.into_iter().enumerate() {
            let angle = k as f32 / count as f32 * std::f32::consts::TAU;
            self.nodes[idx].pos = Pos2::new(angle.cos() * 50.0, angle.sin() * 50.0);
        }
        self.settled = false;
    }

    fn step_physics(&mut self) {
        if self.settled {
            return;
        }
        let n = self.nodes.len();
        if n == 0 {
            self.settled = true;
            return;
        }
        let mut forces = vec![Vec2::ZERO; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let delta = self.nodes[i].pos - self.nodes[j].pos;
                let dist2 = delta.length_sq().max(4.0);
                let force = delta.normalized() * (2200.0 / dist2);
                forces[i] += force;
                forces[j] -= force;
            }
        }
        for &(a, b) in &self.edges {
            let delta = self.nodes[b].pos - self.nodes[a].pos;
            let dist = delta.length().max(0.01);
            let target = 95.0;
            let f = delta.normalized() * (dist - target) * 0.02;
            forces[a] += f;
            forces[b] -= f;
        }
        for i in 0..n {
            forces[i] += self.nodes[i].pos.to_vec2() * -0.0015;
        }
        let mut max_speed: f32 = 0.0;
        for i in 0..n {
            if self.nodes[i].pinned {
                self.nodes[i].vel = Vec2::ZERO;
                continue;
            }
            let new_vel = (self.nodes[i].vel + forces[i]) * 0.82;
            self.nodes[i].vel = new_vel;
            self.nodes[i].pos += new_vel;
            max_speed = max_speed.max(new_vel.length());
        }
        if max_speed < 0.05 {
            self.settled = true;
        }
    }
}

fn file_name(full_path: &str) -> &str {
    full_path.rsplit(['/', '\\']).next().unwrap_or(full_path)
}

/// Renders the brain map and drives its physics for one frame. Returns
/// `Some(full_path)` when a file node was clicked, so the caller can
/// sync the selection back into the list view - the two views stay
/// connected rather than being parallel dead ends.
pub fn brain_map(ui: &mut egui::Ui, tokens: &Tokens, results: &[FileSearchResult], graph: &mut GraphState, height: f32) -> Option<String> {
    graph.sync(results);
    graph.step_physics();
    if !graph.settled {
        ui.ctx().request_repaint();
    }

    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, tokens.bg_sunken);
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, tokens.border));

    if results.is_empty() {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "No results yet", FontId::proportional(12.0), tokens.fg_subtle);
        return None;
    }

    if response.hovered() {
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            graph.zoom = (graph.zoom * (1.0 + scroll * 0.001)).clamp(0.25, 3.0);
        }
    }

    let center = rect.center() + graph.pan;
    let zoom = graph.zoom;
    let to_screen = |p: Pos2| center + p.to_vec2() * zoom;

    let pointer = response.interact_pointer_pos();
    if response.drag_started() {
        if let Some(p) = pointer {
            let gp = ((p - center) / zoom).to_pos2();
            graph.dragging = graph.nodes.iter().position(|node| (node.pos - gp).length() < 16.0);
        }
    }
    if response.dragged() {
        let delta = response.drag_delta();
        if let Some(idx) = graph.dragging {
            graph.nodes[idx].pos += delta / zoom;
            graph.nodes[idx].pinned = true;
            graph.nodes[idx].vel = Vec2::ZERO;
            graph.settled = false;
        } else {
            graph.pan += delta;
        }
    }
    if response.drag_stopped() {
        graph.dragging = None;
    }

    for &(a, b) in &graph.edges {
        painter.line_segment([to_screen(graph.nodes[a].pos), to_screen(graph.nodes[b].pos)], Stroke::new(1.0, tokens.border_strong));
    }

    let culling_rect = rect.expand(40.0);
    let mut clicked_path = None;
    let mut tooltip: Option<(Pos2, String)> = None;
    for node in &graph.nodes {
        let p = to_screen(node.pos);
        if !culling_rect.contains(p) {
            continue;
        }
        let r = match node.kind {
            NodeKind::File => (6.0 + (node.hit_count as f32).sqrt() * 2.5) * zoom,
            NodeKind::Filter => 11.0 * zoom,
        };
        let color = match node.kind {
            NodeKind::File => ext_color(&node.label),
            NodeKind::Filter => tokens.accent,
        };
        painter.circle_filled(p, r, color);
        painter.circle_stroke(p, r, Stroke::new(1.0, tokens.border_strong));
        // Labels always sit BELOW the node, never centered on top of it -
        // a real bug found by screenshot: filter labels centered inside
        // their (necessarily small) circle overflowed past its edge and
        // got clipped by whichever neighboring node's circle happened to
        // be drawn afterward and overlap it, reading as truncated text
        // ("uarterly" for "quarterly"). Filter labels are bold/accent-
        // colored to stay visually distinct from file labels.
        match node.kind {
            NodeKind::Filter => {
                painter.text(p + egui::vec2(0.0, r + 9.0), egui::Align2::CENTER_TOP, &node.label, FontId::proportional(10.0), tokens.accent_strong);
            }
            NodeKind::File if r > 8.0 => {
                let label: String = node.label.chars().take(16).collect();
                painter.text(p + egui::vec2(0.0, r + 8.0), egui::Align2::CENTER_TOP, label, FontId::proportional(9.0), tokens.fg_muted);
            }
            NodeKind::File => {}
        }
        if let Some(hp) = pointer {
            if hp.distance(p) < r.max(7.0) {
                tooltip = Some((
                    p,
                    if node.kind == NodeKind::File {
                        format!("{}\n{} hit(s)", node.full_path, node.hit_count)
                    } else {
                        format!("filter \u{201c}{}\u{201d}\nmatched in {} file(s)", node.label, node.hit_count)
                    },
                ));
                if response.clicked() && node.kind == NodeKind::File {
                    clicked_path = Some(node.full_path.clone());
                }
            }
        }
    }

    if let Some((p, text)) = tooltip {
        egui::show_tooltip_at(ui.ctx(), ui.layer_id(), egui::Id::new("brain_map_tooltip"), p, |ui| {
            ui.label(text);
        });
    }

    ui.painter().text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, "Drag a node to pin \u{b7} scroll to zoom \u{b7} drag background to pan", FontId::proportional(9.0), tokens.fg_subtle);

    clicked_path
}

/// Deterministic color per file extension (same extension always gets
/// the same hue) - lets the map read at a glance which clusters share a
/// file type, without maintaining a hand-picked palette per extension.
fn ext_color(label: &str) -> Color32 {
    let ext = label.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mut hash: u32 = 2166136261;
    for b in ext.bytes() {
        hash = (hash ^ b as u32).wrapping_mul(16777619);
    }
    hsv_to_rgb((hash % 360) as f32, 0.55, 0.78)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color32 {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color32::from_rgb(((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}
