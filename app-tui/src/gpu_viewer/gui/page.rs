//! Draws the page pieces of `page.rs`: pills, the supported line with its margin curve, the requirements table. Fixed row
//! heights and a fixed drawing height, so an edit never moves what is under it.
use super::{
    theme::{tone_color, Palette},
    with_alpha, Gui,
};
use crate::gpu_viewer::{
    menu::Outcome,
    page::{Cell, DockTable, LineDrawing, Pill},
};
use egui::{Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const ROW_H: f32 = 34.0;
const HEAD_H: f32 = 26.0;
/// Below this width the table leaves out its optional columns.
const NARROW: f32 = 640.0;
const DRAWING_H: f32 = 244.0;

fn axis(v: f64) -> String {
    let a = v.abs();
    if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

impl Gui {
    /// A scrolling page over the model's viewport. As an area it takes the pointer, so a drag on it does not turn the
    /// hidden model.
    pub(super) fn page_over_model(&mut self, ui: &mut egui::Ui, area: Rect, items: &[crate::gpu_viewer::ui::DockItem], p: &Palette, out: &mut Vec<Outcome>) {
        if area.width() < 200.0 || area.height() < 120.0 {
            return;
        }
        egui::Area::new(Id::new("model-page")).fade_in(false).order(egui::Order::Middle).fixed_pos(area.min).show(ui.ctx(), |ui| {
            let (rect, _) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, p.bg);
            let content = Rect::from_min_max(Pos2::new(rect.left() + 16.0, rect.top() + 6.0), Pos2::new(rect.right() - 16.0, rect.bottom()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                egui::ScrollArea::vertical().id_salt("model-page-scroll").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                    ui.set_max_width(content.width().min(1120.0));
                    for item in items {
                        self.dock_item(ui, item, p, out);
                    }
                    ui.add_space(24.0);
                });
            });
        });
    }

    pub(super) fn pills(&mut self, ui: &mut egui::Ui, pills: &[Pill], p: &Palette, out: &mut Vec<Outcome>) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for pill in pills {
                let color = tone_color(p, pill.tone);
                let font = FontId::proportional(12.5);
                let galley = ui.painter().layout_no_wrap(pill.text.clone(), font, if pill.selected { p.on_accent } else { color });
                let size = Vec2::new(galley.size().x + 22.0, 24.0);
                let sense = if pill.action.is_some() { Sense::click() } else { Sense::hover() };
                let (rect, resp) = ui.allocate_exact_size(size, sense);
                self.targets.push((pill.text.clone(), rect));
                let painter = ui.painter();
                let (fill, stroke) = if pill.selected {
                    (p.accent, p.accent)
                } else if resp.hovered() && pill.action.is_some() {
                    (p.raised, p.dim)
                } else {
                    (with_alpha(color, if pill.action.is_some() { 14 } else { 30 }), if pill.action.is_some() { p.line } else { with_alpha(color, 120) })
                };
                painter.rect(rect, CornerRadius::same(12), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
                painter.galley(Pos2::new(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley, if pill.selected { p.on_accent } else { color });
                if let (Some(a), true) = (pill.action, resp.clicked()) {
                    out.push(Outcome::Action(a));
                }
            }
        });
    }

    /// The tube to scale with its supports and loads, and under it the margin curve along the same x.
    pub(super) fn line_drawing(&mut self, ui: &mut egui::Ui, d: &LineDrawing, p: &Palette) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), DRAWING_H), Sense::hover());
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(10), p.panel, Stroke::new(1.0, p.line), StrokeKind::Inside);
        let (left, right) = (rect.left() + 48.0, rect.right() - 24.0);
        let length = d.length.max(1e-9);
        let x = |v: f64| left + (right - left) * (v / length).clamp(0.0, 1.0) as f32;
        // The tube.
        let tube = Rect::from_min_max(Pos2::new(left, rect.top() + 74.0), Pos2::new(right, rect.top() + 90.0));
        painter.rect(tube, 3.0, with_alpha(p.accent, 60), Stroke::new(1.5, p.accent), StrokeKind::Inside);
        // Supports: a triangle under the tube and its position.
        for (at, label) in &d.supports {
            let cx = x(*at);
            let top = tube.bottom();
            painter.add(egui::Shape::convex_polygon(vec![Pos2::new(cx, top + 1.0), Pos2::new(cx - 8.0, top + 15.0), Pos2::new(cx + 8.0, top + 15.0)], with_alpha(p.dim, 90), Stroke::new(1.2, p.dim)));
            painter.text(Pos2::new(cx, top + 17.0), Align2::CENTER_TOP, label, FontId::monospace(10.5), p.dim);
        }
        // Loads: an arrow down onto the tube, the label above it.
        for (at, label, text) in &d.loads {
            let cx = x(*at);
            let tip = Pos2::new(cx, tube.top() - 1.0);
            let tail = Pos2::new(cx, rect.top() + 34.0);
            painter.line_segment([tail, tip], Stroke::new(2.0, p.warn));
            painter.add(egui::Shape::convex_polygon(vec![tip, Pos2::new(cx - 5.0, tip.y - 9.0), Pos2::new(cx + 5.0, tip.y - 9.0)], p.warn, Stroke::NONE));
            painter.text(Pos2::new(cx, tail.y - 3.0), Align2::CENTER_BOTTOM, label, FontId::proportional(11.5), p.text);
            painter.text(Pos2::new(cx, tail.y - 15.0), Align2::CENTER_BOTTOM, text, FontId::monospace(10.0), p.dim);
        }
        // The curve's frame.
        let frame = Rect::from_min_max(Pos2::new(left, rect.top() + 142.0), Pos2::new(right, rect.bottom() - 22.0));
        painter.rect(frame, 3.0, p.bg, Stroke::new(1.0, p.line), StrokeKind::Inside);
        painter.text(Pos2::new(rect.left() + 12.0, rect.top() + 128.0), Align2::LEFT_CENTER, &d.curve_label, FontId::proportional(11.0), p.dim);
        if d.curve.len() < 2 {
            let note = if d.note.is_empty() { "No curve" } else { &d.note };
            painter.text(frame.center(), Align2::CENTER_CENTER, note, FontId::proportional(12.0), p.dim);
        } else {
            let (mut lo, mut hi) = (0.0_f64, 0.0_f64);
            for q in &d.curve {
                lo = lo.min(q[1]);
                hi = hi.max(q[1]);
            }
            let pad = 0.1 * (hi - lo).max(1e-9);
            let (y0, y1) = (lo - pad, hi + pad);
            let to = |q: &[f64; 2]| Pos2::new(x(q[0]), frame.bottom() - frame.height() * ((q[1] - y0) / (y1 - y0)) as f32);
            let zero = to(&[0.0, 0.0]).y;
            // Short of the requirement (below zero) shaded warm, met cool.
            painter.rect_filled(Rect::from_min_max(Pos2::new(frame.left() + 1.0, zero), Pos2::new(frame.right() - 1.0, frame.bottom() - 1.0)), 0.0, with_alpha(p.warn, 20));
            painter.line_segment([Pos2::new(frame.left(), zero), Pos2::new(frame.right(), zero)], Stroke::new(1.0, p.dim));
            for v in [hi, 0.0, lo] {
                if v == 0.0 || (v - 0.0).abs() > 1e-9 {
                    painter.text(Pos2::new(frame.left() - 5.0, to(&[0.0, v]).y), Align2::RIGHT_CENTER, axis(v), FontId::monospace(10.0), p.dim);
                }
            }
            let pts: Vec<Pos2> = d.curve.iter().map(to).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.0, p.accent)));
            if let Some(w) = &d.worst {
                let at = to(w);
                painter.line_segment([Pos2::new(at.x, frame.top()), Pos2::new(at.x, frame.bottom())], Stroke::new(1.0, with_alpha(p.warn, 140)));
                painter.circle(at, 4.5, p.warn, Stroke::new(1.5, p.panel));
                let right_side = at.x < frame.center().x;
                let below = at.y < frame.center().y;
                painter.text(Pos2::new(at.x + if right_side { 8.0 } else { -8.0 }, (at.y + if below { 14.0 } else { -14.0 }).clamp(frame.top() + 8.0, frame.bottom() - 8.0)), if right_side { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER }, format!("worst {} at x {}", axis(w[1]), axis(w[0])), FontId::monospace(10.5), p.warn);
            }
        }
        painter.text(Pos2::new(left, frame.bottom() + 4.0), Align2::LEFT_TOP, axis(0.0), FontId::monospace(10.0), p.dim);
        painter.text(Pos2::new(right, frame.bottom() + 4.0), Align2::RIGHT_TOP, axis(d.length), FontId::monospace(10.0), p.dim);
        painter.text(Pos2::new((left + right) / 2.0, frame.bottom() + 4.0), Align2::CENTER_TOP, "x along the run (in)", FontId::proportional(10.5), p.dim);
    }

    pub(super) fn dock_table(&mut self, ui: &mut egui::Ui, t: &DockTable, p: &Palette, out: &mut Vec<Outcome>) {
        let width = ui.available_width();
        let narrow = width < NARROW;
        let keep: Vec<usize> = (0..t.columns.len()).filter(|&c| !(narrow && t.columns[c].optional)).collect();
        let total: f32 = keep.iter().map(|&c| t.columns[c].weight).sum::<f32>().max(1e-3);
        // Column edges across the card, 10 px inside.
        let inner = width - 20.0;
        let mut edges = vec![10.0_f32];
        for &c in &keep {
            let last = *edges.last().unwrap();
            edges.push(last + inner * t.columns[c].weight / total);
        }
        let height = HEAD_H + ROW_H * t.rows.len() as f32 + if t.add.is_some() { ROW_H - 4.0 } else { 0.0 } + 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(10), p.panel, Stroke::new(1.0, p.line), StrokeKind::Inside);
        let cell_rect = |row_top: f32, k: usize| Rect::from_min_max(Pos2::new(rect.left() + edges[k] + 2.0, row_top), Pos2::new(rect.left() + edges[k + 1] - 4.0, row_top + ROW_H));
        for (k, &c) in keep.iter().enumerate() {
            let col = &t.columns[c];
            let r = Rect::from_min_max(Pos2::new(rect.left() + edges[k] + 2.0, rect.top()), Pos2::new(rect.left() + edges[k + 1] - 4.0, rect.top() + HEAD_H));
            let (align, at) = if col.right { (Align2::RIGHT_CENTER, r.right_center()) } else { (Align2::LEFT_CENTER, r.left_center()) };
            ui.painter().text(at + Vec2::new(0.0, 2.0), align, col.title.to_uppercase(), FontId::proportional(10.5), p.dim);
        }
        ui.painter().line_segment([Pos2::new(rect.left() + 1.0, rect.top() + HEAD_H), Pos2::new(rect.right() - 1.0, rect.top() + HEAD_H)], Stroke::new(1.0, p.line));
        for (ri, row) in t.rows.iter().enumerate() {
            let top = rect.top() + HEAD_H + ROW_H * ri as f32;
            if row.governing {
                ui.painter().rect_filled(Rect::from_min_max(Pos2::new(rect.left() + 1.0, top), Pos2::new(rect.right() - 1.0, top + ROW_H)), 0.0, with_alpha(p.accent, 26));
                ui.painter().rect_filled(Rect::from_min_max(Pos2::new(rect.left() + 1.0, top), Pos2::new(rect.left() + 4.0, top + ROW_H)), 0.0, p.accent);
            } else if ri > 0 {
                ui.painter().line_segment([Pos2::new(rect.left() + 10.0, top), Pos2::new(rect.right() - 10.0, top)], Stroke::new(1.0, with_alpha(p.line, 120)));
            }
            for (k, &c) in keep.iter().enumerate() {
                let Some(cell) = row.cells.get(c) else { continue };
                let r = cell_rect(top, k);
                self.table_cell(ui, cell, r, t.columns[c].right, Id::new(("vessel-table", ri, c)), p, out);
            }
        }
        if let Some((label, action)) = &t.add {
            let top = rect.top() + HEAD_H + ROW_H * t.rows.len() as f32;
            let r = Rect::from_min_max(Pos2::new(rect.left() + 10.0, top + 2.0), Pos2::new(rect.right() - 10.0, top + ROW_H - 4.0));
            let resp = ui.interact(r, Id::new("vessel-table-add"), Sense::click());
            self.targets.push((label.clone(), r));
            ui.painter().rect(r, CornerRadius::same(6), if resp.hovered() { p.raised } else { Color32::TRANSPARENT }, Stroke::new(1.0, with_alpha(p.dim, 90)), StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("+  {label}"), FontId::proportional(12.5), p.accent);
            if resp.clicked() {
                out.push(Outcome::Action(*action));
            }
        }
        if !t.footer.is_empty() {
            ui.add_space(6.0);
            let color = t.footer_tone.map_or(p.text, |tone| tone_color(p, tone));
            ui.add(egui::Label::new(egui::RichText::new(&t.footer).size(13.0).strong().color(color)).wrap());
        }
        if !t.hint.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(&t.hint).size(11.5).color(p.dim)).wrap());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn table_cell(&mut self, ui: &mut egui::Ui, cell: &Cell, r: Rect, right: bool, id: Id, p: &Palette, out: &mut Vec<Outcome>) {
        let mid = r.center().y;
        match cell {
            Cell::Text { text, tone, action, editing } => {
                let color = tone_color(p, *tone);
                let box_rect = r.shrink2(Vec2::new(0.0, 5.0));
                let resp = action.map(|_| ui.interact(box_rect, id, Sense::click()));
                self.targets.push((text.clone(), box_rect));
                let hovered = resp.as_ref().is_some_and(|r| r.hovered());
                if *editing {
                    ui.painter().rect(box_rect, CornerRadius::same(5), p.bg, Stroke::new(1.5, p.accent), StrokeKind::Inside);
                } else if hovered {
                    ui.painter().rect(box_rect, CornerRadius::same(5), p.raised, Stroke::new(1.0, p.line), StrokeKind::Inside);
                }
                let font = FontId::proportional(13.0);
                let galley = ui.painter().layout(text.clone(), font, color, (box_rect.width() - 10.0).max(10.0));
                let x = if right { box_rect.right() - 6.0 - galley.size().x } else { box_rect.left() + 6.0 };
                ui.painter().with_clip_rect(box_rect).galley(Pos2::new(x, mid - galley.size().y / 2.0), galley, color);
                if let (Some(a), Some(resp)) = (action, resp) {
                    if resp.clicked() {
                        out.push(Outcome::Action(*a));
                    }
                }
            }
            Cell::Wall { text, value, current, max, tone } => {
                let color = tone_color(p, *tone);
                let text_w = 70.0_f32.min(r.width() * 0.45);
                ui.painter().text(Pos2::new(r.left() + 4.0, mid), Align2::LEFT_CENTER, text, FontId::monospace(12.5), p.text);
                let track = Rect::from_min_max(Pos2::new(r.left() + text_w + 6.0, mid - 5.0), Pos2::new(r.right(), mid + 5.0));
                if track.width() > 12.0 && *max > 0.0 {
                    ui.painter().rect(track, 3.0, p.bg, Stroke::new(1.0, p.line), StrokeKind::Inside);
                    let at = |v: f64| track.left() + track.width() * (v / max).clamp(0.0, 1.0) as f32;
                    ui.painter().rect_filled(Rect::from_min_max(Pos2::new(track.left() + 1.0, track.top() + 1.5), Pos2::new(at(*value).max(track.left() + 2.5), track.bottom() - 1.5)), 2.0, color);
                    let tick = at(*current);
                    ui.painter().line_segment([Pos2::new(tick, track.top() - 4.0), Pos2::new(tick, track.bottom() + 4.0)], Stroke::new(1.5, p.text));
                }
            }
            Cell::Chip { text, tone } => {
                let color = tone_color(p, *tone);
                let galley = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(11.5), color);
                let size = Vec2::new((galley.size().x + 16.0).min(r.width()), 20.0);
                let x = if right { r.right() - size.x } else { r.left() };
                let chip = Rect::from_min_size(Pos2::new(x, mid - 10.0), size);
                ui.painter().rect(chip, CornerRadius::same(10), with_alpha(color, 36), Stroke::new(1.0, with_alpha(color, 140)), StrokeKind::Inside);
                ui.painter().with_clip_rect(chip).galley(Pos2::new(chip.left() + 8.0, mid - galley.size().y / 2.0), galley, color);
            }
            Cell::Button { text, action } => {
                let box_rect = Rect::from_center_size(Pos2::new(r.right() - 12.0, mid), Vec2::new(22.0, 22.0));
                let resp = ui.interact(box_rect, id, Sense::click());
                self.targets.push((format!("Remove {text}"), box_rect));
                if resp.hovered() {
                    ui.painter().rect(box_rect, CornerRadius::same(5), p.raised, Stroke::new(1.0, p.line), StrokeKind::Inside);
                }
                ui.painter().text(box_rect.center(), Align2::CENTER_CENTER, "\u{d7}", FontId::proportional(15.0), if resp.hovered() { p.warn } else { p.dim });
                if resp.clicked() {
                    out.push(Outcome::Action(*action));
                }
            }
        }
    }
}
