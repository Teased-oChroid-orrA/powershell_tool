//! Interactive force-directed "brain map" for search results - an
//! alternate view next to the plain list, built from data the engine
//! already returns (`FileSearchResult`/`LineHit::matched_filters`/
//! `match_line`/`modified`), not a decorative layout. Bipartite graph
//! (file <-> matched-filter), optionally with a third folder tier: an
//! edge for every file/filter pair that actually matched, and - when
//! folder clustering is on - an edge from every file to its immediate
//! parent folder. This directly answers "which files matched which
//! terms, and how" - the real question a flat list leaves implicit once
//! more than one filter is in play.
//!
//! egui has no built-in graph/scene widget (confirmed against docs.rs
//! before writing this) - pan/zoom and the physics are hand-rolled, same
//! "own `Painter` + manual screen transform" shape `sketches.rs::Sketch`
//! already uses, extended with drag/scroll input and a continuous-
//! repaint simulation loop that stops once the layout settles (so it
//! isn't burning CPU sitting idle - `ctx.request_repaint()` is only
//! called while any node's velocity is still above a small epsilon).
//!
//! Roadmap features added on top of the original list/filter graph (all
//! approved via the "Toolbench Overhaul" planning artifact's Enhancements
//! tier): click-to-open + right-click context menu, hover preview of the
//! real matched line, type-to-highlight, fit-to-view, folder clustering,
//! color-by toggle (extension/density/recency), export-as-image, and
//! pinned layouts that survive a search re-run or app restart.

use std::collections::HashMap;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use search_core::models::FileSearchResult;

use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    File,
    Filter,
    Folder,
}

/// How file nodes are colored - the "color-by" roadmap toggle. Filter/
/// Folder nodes keep a fixed color regardless of mode (a real, bounded
/// scope choice: those two node kinds don't have a meaningful "density"/
/// "recency" of their own beyond the file count they already show via
/// size, so re-coloring them per mode would add a control with no new
/// information behind it).
#[derive(Clone, Copy, PartialEq)]
enum ColorMode {
    Extension,
    Density,
    Recency,
}

struct Node {
    kind: NodeKind,
    label: String,
    full_path: String,
    hit_count: usize,
    modified: Option<chrono::DateTime<chrono::Local>>,
    /// First matched line's text (File nodes only) - real hover-preview
    /// content, not a placeholder. `None` for Filter/Folder nodes.
    preview: Option<String>,
    pos: Pos2,
    vel: Vec2,
    pinned: bool,
}

impl Node {
    /// Stable identity across a graph rebuild (new search, app restart) -
    /// what pinned-layout persistence keys on. File nodes use their full
    /// path (unique by construction); Filter/Folder nodes prefix their
    /// label so a file that happens to share text with a filter/folder
    /// name can never collide with it.
    fn stable_id(&self) -> String {
        match self.kind {
            NodeKind::File => self.full_path.clone(),
            NodeKind::Filter => format!("filter:{}", self.label),
            NodeKind::Folder => format!("folder:{}", self.full_path),
        }
    }
}

pub struct GraphState {
    nodes: Vec<Node>,
    edges: Vec<(usize, usize)>,
    /// Cheap fingerprint of the result set this layout was built for -
    /// results are replaced wholesale by every search run, never mutated
    /// in place, so count + total hit count is enough to detect "the
    /// results actually changed" without a full content hash. Also
    /// changes when `cluster_folders` toggles, since that changes the
    /// graph's own shape (extra tier of nodes/edges), not just styling.
    built_for: u64,
    pan: Vec2,
    zoom: f32,
    dragging: Option<usize>,
    settled: bool,

    /// Type-to-highlight query - matching file/filter/folder labels stay
    /// at full opacity, everything else dims. Empty = no dimming.
    filter_query: String,
    color_mode: ColorMode,
    cluster_folders: bool,
    /// Right-click context menu target: node index + the screen position
    /// it was opened at (menu content doesn't move if the graph pans
    /// while it's open - closing on any outside click keeps that from
    /// mattering in practice).
    context_menu: Option<(usize, Pos2)>,
    /// Manually-pinned node positions, keyed by `Node::stable_id` -
    /// restored on every `sync()` (new search, or the very first sync
    /// after loading a persisted session) so a layout a user arranged by
    /// hand survives both a re-search and an app restart. `main.rs` reads
    ////writes this via `pinned_layout_snapshot`/`apply_pinned_layout`,
    /// the same snapshot/restore shape `SearchTool::to_snapshot` already
    /// established for other per-session state.
    pinned_positions: HashMap<String, (f32, f32)>,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            built_for: u64::MAX,
            pan: Vec2::ZERO,
            zoom: 1.0,
            dragging: None,
            settled: true,
            filter_query: String::new(),
            color_mode: ColorMode::Extension,
            cluster_folders: false,
            context_menu: None,
            pinned_positions: HashMap::new(),
        }
    }
}

impl GraphState {
    pub fn pinned_layout_snapshot(&self) -> HashMap<String, (f32, f32)> {
        self.pinned_positions.clone()
    }

    pub fn apply_pinned_layout(&mut self, layout: HashMap<String, (f32, f32)>) {
        self.pinned_positions = layout;
    }

    fn sync(&mut self, results: &[FileSearchResult]) {
        let fingerprint = (results.len() as u64 ^ results.iter().map(|r| r.hits.len() as u64).sum::<u64>().wrapping_mul(2654435761))
            ^ (self.cluster_folders as u64).wrapping_mul(0x9E3779B97F4A7C15);
        if fingerprint == self.built_for {
            return;
        }
        self.built_for = fingerprint;
        self.nodes.clear();
        self.edges.clear();
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.context_menu = None;

        // Seed on a ring rather than a random pile - the physics sim
        // converges faster from an already-spread-out start, and stays
        // stable (no first-frame flash of everything piled at the
        // origin) even before it's had a chance to settle.
        let mut filter_index: HashMap<&str, usize> = HashMap::new();
        let mut folder_index: HashMap<String, usize> = HashMap::new();
        for (i, r) in results.iter().enumerate() {
            let angle = i as f32 / results.len().max(1) as f32 * std::f32::consts::TAU;
            let radius = 180.0 + (r.hits.len() as f32).sqrt() * 6.0;
            let preview = r.hits.first().map(|h| {
                let line = h.match_line.trim();
                if line.chars().count() > 90 {
                    format!("{}\u{2026}", line.chars().take(90).collect::<String>())
                } else {
                    line.to_string()
                }
            });
            self.nodes.push(Node {
                kind: NodeKind::File,
                label: file_name(&r.full_name).to_string(),
                full_path: r.full_name.clone(),
                hit_count: r.hits.len(),
                modified: Some(r.modified),
                preview,
                pos: Pos2::new(angle.cos() * radius, angle.sin() * radius),
                vel: Vec2::ZERO,
                pinned: false,
            });
            let file_idx = i;

            if self.cluster_folders {
                let folder = parent_folder(&r.full_name);
                let folder_idx = match folder_index.get(&folder) {
                    Some(&idx) => idx,
                    None => {
                        let idx = self.nodes.len();
                        self.nodes.push(Node {
                            kind: NodeKind::Folder,
                            label: folder.clone(),
                            full_path: folder.clone(),
                            hit_count: 0,
                            modified: None,
                            preview: None,
                            pos: Pos2::ZERO,
                            vel: Vec2::ZERO,
                            pinned: false,
                        });
                        folder_index.insert(folder.clone(), idx);
                        idx
                    }
                };
                self.nodes[folder_idx].hit_count += 1;
                self.edges.push((file_idx, folder_idx));
            }

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
                                modified: None,
                                preview: None,
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
        // matched them once the springs pull taut. Folder nodes (when
        // present) get a slightly larger ring so they visually sit
        // between the filter hub ring and the file ring.
        let filter_positions: Vec<usize> = self.nodes.iter().enumerate().filter(|(_, n)| n.kind == NodeKind::Filter).map(|(i, _)| i).collect();
        let count = filter_positions.len().max(1);
        for (k, idx) in filter_positions.into_iter().enumerate() {
            let angle = k as f32 / count as f32 * std::f32::consts::TAU;
            self.nodes[idx].pos = Pos2::new(angle.cos() * 50.0, angle.sin() * 50.0);
        }
        let folder_positions: Vec<usize> = self.nodes.iter().enumerate().filter(|(_, n)| n.kind == NodeKind::Folder).map(|(i, _)| i).collect();
        let fcount = folder_positions.len().max(1);
        for (k, idx) in folder_positions.into_iter().enumerate() {
            let angle = k as f32 / fcount as f32 * std::f32::consts::TAU;
            self.nodes[idx].pos = Pos2::new(angle.cos() * 105.0, angle.sin() * 105.0);
        }

        // Restore any previously-pinned position for a node that still
        // exists in this new result set - a real layout a user arranged
        // by hand shouldn't reset just because they re-ran the search or
        // restarted the app.
        for node in &mut self.nodes {
            if let Some(&(x, y)) = self.pinned_positions.get(&node.stable_id()) {
                node.pos = Pos2::new(x, y);
                node.pinned = true;
            }
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

    /// Fit-to-view: computes the bounding box of every node's
    /// graph-space position and picks pan/zoom so the whole graph is
    /// centered and visible in `rect` with a little breathing room - a
    /// real recentering, not just a reset to the default zoom=1/pan=0
    /// (which would leave an off-center or partly-offscreen layout after
    /// the user has dragged/panned/zoomed around).
    fn fit_to_view(&mut self, rect: Rect) {
        if self.nodes.is_empty() {
            self.pan = Vec2::ZERO;
            self.zoom = 1.0;
            return;
        }
        let mut min = Pos2::new(f32::MAX, f32::MAX);
        let mut max = Pos2::new(f32::MIN, f32::MIN);
        for node in &self.nodes {
            min.x = min.x.min(node.pos.x);
            min.y = min.y.min(node.pos.y);
            max.x = max.x.max(node.pos.x);
            max.y = max.y.max(node.pos.y);
        }
        let bbox_size = (max - min).max(Vec2::new(1.0, 1.0));
        let bbox_center = min + (max - min) * 0.5;
        let padding = 0.85; // leave ~15% margin so edge nodes/labels aren't clipped
        let zoom = (rect.width() / bbox_size.x).min(rect.height() / bbox_size.y) * padding;
        self.zoom = zoom.clamp(0.1, 3.0);
        self.pan = -bbox_center.to_vec2() * self.zoom;
    }

    /// Renders the graph's own node/edge topology onto an offscreen
    /// `tiny_skia::Pixmap` and returns PNG bytes - the "export as image"
    /// roadmap feature. Deliberately does NOT render text labels: `tiny-
    /// skia` has no font/text shaping of its own (it's a 2D raster
    /// backend, not a text-layout engine), and pulling in a second text
    /// stack just for this export isn't worth it - a topology + color
    /// snapshot is still the real, useful part of "export as image" for
    /// sharing what clustered with what, disclosed here rather than
    /// silently shipping unlabeled nodes as if that were the whole
    /// feature.
    fn export_png(&self, tokens: &Tokens, width: u32, height: u32) -> Option<Vec<u8>> {
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        let bg = tokens.bg_sunken;
        pixmap.fill(tiny_skia::Color::from_rgba8(bg.r(), bg.g(), bg.b(), 255));

        if self.nodes.is_empty() {
            return pixmap.encode_png().ok();
        }
        let mut min = Pos2::new(f32::MAX, f32::MAX);
        let mut max = Pos2::new(f32::MIN, f32::MIN);
        for node in &self.nodes {
            min.x = min.x.min(node.pos.x);
            min.y = min.y.min(node.pos.y);
            max.x = max.x.max(node.pos.x);
            max.y = max.y.max(node.pos.y);
        }
        let bbox_size = (max - min).max(Vec2::new(1.0, 1.0));
        let bbox_center = min + (max - min) * 0.5;
        let scale = (width as f32 / bbox_size.x).min(height as f32 / bbox_size.y) * 0.85;
        let center = Pos2::new(width as f32 / 2.0, height as f32 / 2.0);
        let to_canvas = |p: Pos2| center + (p - bbox_center) * scale;

        let edge_color = tokens.border_strong;
        let mut edge_paint = tiny_skia::Paint::default();
        edge_paint.set_color(tiny_skia::Color::from_rgba8(edge_color.r(), edge_color.g(), edge_color.b(), 255));
        let edge_stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
        for &(a, b) in &self.edges {
            let pa = to_canvas(self.nodes[a].pos);
            let pb = to_canvas(self.nodes[b].pos);
            let mut pb_builder = tiny_skia::PathBuilder::new();
            pb_builder.move_to(pa.x, pa.y);
            pb_builder.line_to(pb.x, pb.y);
            if let Some(path) = pb_builder.finish() {
                pixmap.stroke_path(&path, &edge_paint, &edge_stroke, tiny_skia::Transform::identity(), None);
            }
        }

        for node in &self.nodes {
            let p = to_canvas(node.pos);
            let r = (match node.kind {
                NodeKind::File => 6.0 + (node.hit_count as f32).sqrt() * 2.5,
                NodeKind::Filter => 11.0,
                NodeKind::Folder => 9.0,
            }) * scale.max(0.3).min(1.4);
            let color = self.node_color(tokens, node);
            let mut fill_paint = tiny_skia::Paint::default();
            fill_paint.set_color(tiny_skia::Color::from_rgba8(color.r(), color.g(), color.b(), 255));
            if let Some(path) = tiny_skia::PathBuilder::from_circle(p.x, p.y, r.max(1.0)) {
                pixmap.fill_path(&path, &fill_paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            }
        }
        pixmap.encode_png().ok()
    }

    fn node_color(&self, tokens: &Tokens, node: &Node) -> Color32 {
        match node.kind {
            NodeKind::Filter => tokens.accent,
            NodeKind::Folder => tokens.warning,
            NodeKind::File => match self.color_mode {
                ColorMode::Extension => ext_color(&node.label),
                ColorMode::Density => {
                    let max_hits = self.nodes.iter().filter(|n| n.kind == NodeKind::File).map(|n| n.hit_count).max().unwrap_or(1).max(1);
                    let t = (node.hit_count as f32 / max_hits as f32).clamp(0.0, 1.0);
                    lerp_color(tokens.fg_subtle, tokens.danger, t)
                }
                ColorMode::Recency => {
                    let times: Vec<_> = self.nodes.iter().filter(|n| n.kind == NodeKind::File).filter_map(|n| n.modified).collect();
                    let (oldest, newest) = match (times.iter().min(), times.iter().max()) {
                        (Some(a), Some(b)) => (*a, *b),
                        _ => return tokens.fg_muted,
                    };
                    let span = (newest - oldest).num_seconds().max(1) as f32;
                    let t = node.modified.map(|m| (m - oldest).num_seconds() as f32 / span).unwrap_or(0.0).clamp(0.0, 1.0);
                    lerp_color(tokens.fg_subtle, tokens.good, t)
                }
            },
        }
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn file_name(full_path: &str) -> &str {
    full_path.rsplit(['/', '\\']).next().unwrap_or(full_path)
}

fn parent_folder(full_path: &str) -> String {
    match full_path.rfind(['/', '\\']) {
        Some(idx) if idx > 0 => full_path[..idx].to_string(),
        _ => "(root)".to_string(),
    }
}

/// Renders the brain map (toolbar + canvas) and drives its physics for
/// one frame. Returns `Some(full_path)` when a file node was clicked, so
/// the caller can sync the selection back into the list view - the two
/// views stay connected rather than being parallel dead ends. Left-click
/// on a file node also opens it directly (`open::that`) - the roadmap's
/// "click-to-open", not just a selection sync.
pub fn brain_map(ui: &mut egui::Ui, tokens: &Tokens, results: &[FileSearchResult], graph: &mut GraphState, height: f32) -> Option<String> {
    graph.sync(results);

    // Toolbar: type-to-highlight, color-by, folder clustering, fit-to-
    // view, export - all real controls wired to the state above, not
    // decoration.
    let canvas_rect_estimate = Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut graph.filter_query).hint_text("Highlight\u{2026}").desired_width(140.0));
        ui.separator();
        crate::design::components::segmented(
            ui,
            tokens,
            &mut graph.color_mode,
            &[(ColorMode::Extension, "By type"), (ColorMode::Density, "By hits"), (ColorMode::Recency, "By recency")],
        );
        ui.separator();
        if ui.checkbox(&mut graph.cluster_folders, "Cluster by folder").changed() {
            graph.built_for = u64::MAX; // force a resync - clustering changes the graph's shape, not just its styling
        }
        ui.separator();
        if ui.button("Fit to view").clicked() {
            graph.fit_to_view(canvas_rect_estimate);
        }
        if ui.button("\u{1F4E4} Export PNG").clicked() {
            if let Some(bytes) = graph.export_png(tokens, 1600, 1200) {
                if let Some(path) = rfd::FileDialog::new().set_file_name("brain_map.png").add_filter("PNG image", &["png"]).save_file() {
                    let _ = std::fs::write(path, bytes);
                }
            }
        }
    });
    ui.add_space(6.0);

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
        if let Some(idx) = graph.dragging {
            let id = graph.nodes[idx].stable_id();
            let pos = graph.nodes[idx].pos;
            graph.pinned_positions.insert(id, (pos.x, pos.y));
        }
        graph.dragging = None;
    }

    for &(a, b) in &graph.edges {
        painter.line_segment([to_screen(graph.nodes[a].pos), to_screen(graph.nodes[b].pos)], Stroke::new(1.0, tokens.border_strong));
    }

    let query = graph.filter_query.trim().to_lowercase();
    let culling_rect = rect.expand(40.0);
    let mut clicked_path = None;
    let mut tooltip: Option<(Pos2, String)> = None;
    let mut right_clicked: Option<(usize, Pos2)> = None;
    for (i, node) in graph.nodes.iter().enumerate() {
        let p = to_screen(node.pos);
        if !culling_rect.contains(p) {
            continue;
        }
        let base_r = match node.kind {
            NodeKind::File => 6.0 + (node.hit_count as f32).sqrt() * 2.5,
            NodeKind::Filter => 11.0,
            NodeKind::Folder => 9.0,
        };
        let r = base_r * zoom;
        let dim = !query.is_empty() && !node.label.to_lowercase().contains(&query) && !node.full_path.to_lowercase().contains(&query);
        let mut color = graph.node_color(tokens, node);
        if dim {
            color = color.gamma_multiply(0.25);
        }
        painter.circle_filled(p, r, color);
        painter.circle_stroke(p, r, Stroke::new(1.0, tokens.border_strong));
        // Labels always sit BELOW the node, never centered on top of it -
        // a real bug found by screenshot: filter labels centered inside
        // their (necessarily small) circle overflowed past its edge and
        // got clipped by whichever neighboring node's circle happened to
        // be drawn afterward and overlap it, reading as truncated text
        // ("uarterly" for "quarterly"). Filter labels are bold/accent-
        // colored to stay visually distinct from file labels.
        let label_color = if dim { tokens.fg_subtle.gamma_multiply(0.5) } else { tokens.accent_strong };
        match node.kind {
            NodeKind::Filter => {
                painter.text(p + egui::vec2(0.0, r + 9.0), egui::Align2::CENTER_TOP, &node.label, FontId::proportional(10.0), label_color);
            }
            NodeKind::Folder => {
                let label: String = node.label.chars().rev().take(20).collect::<String>().chars().rev().collect();
                painter.text(p + egui::vec2(0.0, r + 9.0), egui::Align2::CENTER_TOP, format!("\u{1F4C1} {label}"), FontId::proportional(9.5), if dim { tokens.fg_subtle.gamma_multiply(0.5) } else { tokens.warning });
            }
            NodeKind::File if r > 8.0 => {
                let label: String = node.label.chars().take(16).collect();
                let color = if dim { tokens.fg_subtle.gamma_multiply(0.5) } else { tokens.fg_muted };
                painter.text(p + egui::vec2(0.0, r + 8.0), egui::Align2::CENTER_TOP, label, FontId::proportional(9.0), color);
            }
            NodeKind::File => {}
        }
        if let Some(hp) = pointer {
            if hp.distance(p) < r.max(7.0) {
                tooltip = Some((
                    p,
                    match node.kind {
                        NodeKind::File => {
                            let mut t = format!("{}\n{} hit(s)", node.full_path, node.hit_count);
                            if let Some(preview) = &node.preview {
                                t.push_str(&format!("\n\u{201c}{preview}\u{201d}"));
                            }
                            t
                        }
                        NodeKind::Filter => format!("filter \u{201c}{}\u{201d}\nmatched in {} file(s)", node.label, node.hit_count),
                        NodeKind::Folder => format!("{}\n{} file(s)", node.full_path, node.hit_count),
                    },
                ));
                if response.clicked() && node.kind == NodeKind::File {
                    let _ = open::that(&node.full_path);
                    clicked_path = Some(node.full_path.clone());
                }
                if response.secondary_clicked() {
                    right_clicked = Some((i, hp));
                }
            }
        }
    }

    if let Some(rc) = right_clicked {
        graph.context_menu = Some(rc);
    }

    if let Some((p, text)) = tooltip {
        egui::show_tooltip_at(ui.ctx(), ui.layer_id(), egui::Id::new("brain_map_tooltip"), p, |ui| {
            ui.set_max_width(360.0);
            ui.label(text);
        });
    }

    if let Some((idx, pos)) = graph.context_menu {
        let node_kind = graph.nodes[idx].kind;
        let node_path = graph.nodes[idx].full_path.clone();
        let mut close = false;
        let area_resp = egui::Area::new(egui::Id::new("brain_map_context_menu")).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
            egui::Frame::default().fill(tokens.bg_raised).stroke(Stroke::new(1.0, tokens.border)).rounding(crate::design::radii::md()).inner_margin(6.0).shadow(crate::design::shadows::overlay()).show(ui, |ui| {
                ui.set_min_width(160.0);
                if node_kind == NodeKind::File {
                    if ui.button("Open").clicked() {
                        let _ = open::that(&node_path);
                        close = true;
                    }
                    if ui.button("Reveal containing folder").clicked() {
                        let _ = open::that(parent_folder(&node_path));
                        close = true;
                    }
                    if ui.button("Copy path").clicked() {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(node_path.clone());
                        }
                        close = true;
                    }
                } else {
                    ui.colored_label(tokens.fg_muted, &graph.nodes[idx].label);
                }
            });
        });
        if close || area_resp.response.clicked_elsewhere() {
            graph.context_menu = None;
        }
    }

    ui.painter().text(
        rect.left_bottom() + egui::vec2(8.0, -8.0),
        egui::Align2::LEFT_BOTTOM,
        "Click to open \u{b7} right-click for more \u{b7} drag a node to pin \u{b7} scroll to zoom \u{b7} drag background to pan",
        FontId::proportional(9.0),
        tokens.fg_subtle,
    );

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
