//! The dashboard screen: a search box, one tile per workbench (how it opens, what was opened last), the recent files.
//! It fills the window; the window it lives in has no model, so nothing else is drawn.
use super::{theme, Gui};
use crate::gpu_viewer::dashboard::{ago, unix_now, Dashboard, Workbench};
use egui::{CornerRadius, FontId, Rect, RichText, Sense, Stroke, Vec2};
use std::path::PathBuf;

/// What the person asked the dashboard for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashEvent {
    /// Open workbench `.0`, on `.1` when it is a recent file.
    Open(&'static str, Option<PathBuf>),
}

const TILE_MIN: f32 = 250.0;
const TILE_H: f32 = 118.0;

impl Gui {
    pub fn run_dashboard(&mut self, raw: egui::RawInput, dash: &mut Dashboard) -> (egui::FullOutput, Vec<DashEvent>) {
        let ctx = self.ctx.clone();
        let mut events = Vec::new();
        self.targets.clear();
        let full = ctx.run_ui(raw, |ui| self.dashboard(ui, dash, &mut events));
        (full, events)
    }

    fn dashboard(&mut self, ui: &mut egui::Ui, dash: &mut Dashboard, events: &mut Vec<DashEvent>) {
        let p = theme::current(ui.ctx());
        let now = unix_now();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let pad = 28.0;
                egui::Frame::new().inner_margin(egui::Margin::symmetric(pad as i8, 22)).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("GS Engineering Toolbench").size(22.0).strong());
                        ui.label(RichText::new("pick a workbench").color(p.dim));
                    });
                    ui.add_space(10.0);
                    let search = egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut dash.query).frame(egui::Frame::NONE).hint_text("Filter workbenches").desired_width(f32::INFINITY)).rect
                    });
                    self.targets.push(("Filter workbenches".into(), search.inner));
                    ui.add_space(14.0);
                    let shown = dash.visible();
                    if shown.is_empty() {
                        ui.label(RichText::new("No workbench matches that.").color(p.dim));
                    }
                    let gap = 12.0;
                    let width = ui.available_width();
                    let cols = (((width + gap) / (TILE_MIN + gap)).floor() as usize).max(1);
                    let tile_w = (width - gap * (cols as f32 - 1.0)) / cols as f32;
                    for row in shown.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for w in row {
                                if let Some(ev) = self.tile(ui, &p, w, dash, Vec2::new(tile_w, TILE_H), now) {
                                    events.push(ev);
                                }
                            }
                        });
                        ui.add_space(gap - 5.0);
                    }
                    ui.add_space(12.0);
                    self.recent_list(ui, &p, dash, events, now);
                    if let Some(m) = &dash.message {
                        ui.add_space(10.0);
                        ui.label(RichText::new(m).color(p.dim));
                    }
                });
            });
        });
    }

    fn tile(&mut self, ui: &mut egui::Ui, p: &theme::Palette, w: &'static Workbench, dash: &Dashboard, size: Vec2, now: u64) -> Option<DashEvent> {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        self.targets.push((w.name.to_string(), rect));
        let hot = resp.hovered() || resp.has_focus();
        let painter = ui.painter_at(rect);
        painter.rect(rect, CornerRadius::same(8), p.raised, Stroke::new(1.0, if hot { p.accent } else { p.line }), egui::StrokeKind::Inside);
        let inner = rect.shrink(14.0);
        let name = painter.layout_no_wrap(w.name.to_string(), FontId::proportional(15.0), p.text);
        painter.galley(inner.min, name.clone(), p.text);
        // How it opens, top right.
        let (tag, col) = if w.is_window() { ("window", p.good) } else { ("terminal", p.warn) };
        let t = painter.layout_no_wrap(tag.to_string(), FontId::monospace(11.0), col);
        let tr = Rect::from_min_size(egui::pos2(inner.right() - t.size().x - 12.0, inner.top()), t.size() + Vec2::new(12.0, 4.0));
        painter.rect_filled(tr, CornerRadius::same(9), col.gamma_multiply(0.18));
        painter.galley(tr.min + Vec2::new(6.0, 2.0), t, col);
        let body = painter.layout(w.summary.to_string(), FontId::proportional(12.5), p.dim, inner.width());
        painter.galley(inner.min + Vec2::new(0.0, name.size().y + 6.0), body, p.dim);
        let last = dash.recents.last(w.id).map_or_else(|| "not opened yet".to_string(), |r| format!("{}, {}", r.path.as_deref().map(file_name).unwrap_or("last session"), ago(now, r.opened)));
        let foot = painter.layout_no_wrap(format!("last: {last}"), FontId::monospace(11.0), p.dim);
        painter.galley(egui::pos2(inner.left(), inner.bottom() - foot.size().y), foot, p.dim);
        let resp = resp.on_hover_text(if w.is_window() { "Opens in its own window" } else { "Opens in a terminal window" });
        resp.clicked().then(|| DashEvent::Open(w.id, None))
    }

    fn recent_list(&mut self, ui: &mut egui::Ui, p: &theme::Palette, dash: &Dashboard, events: &mut Vec<DashEvent>, now: u64) {
        ui.label(RichText::new("RECENT").size(11.0).strong().color(p.dim));
        ui.add_space(4.0);
        let items: Vec<_> = dash.recents.items.iter().filter(|r| r.path.is_some()).filter_map(|r| Some((Workbench::find(&r.workbench)?, r))).collect();
        if items.is_empty() {
            ui.label(RichText::new("Files you open from a workbench appear here.").color(p.dim));
            return;
        }
        egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (w, r) in items {
                let path = r.path.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.set_min_height(26.0);
                    let b = ui.add(egui::Button::new(RichText::new(w.name).strong()).frame(false).min_size(Vec2::new(150.0, 0.0)));
                    self.targets.push((format!("Recent {}", file_name(&path)), b.rect));
                    ui.label(RichText::new(file_name(&path)).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(ago(now, r.opened)).color(p.dim)));
                    if b.on_hover_text(&path).clicked() {
                        events.push(DashEvent::Open(w.id, Some(PathBuf::from(&path))));
                    }
                });
            }
        });
    }
}

fn file_name(path: &str) -> &str {
    std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::dashboard::Recents;

    fn raw(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1100.0, 760.0))), events, ..Default::default() }
    }
    fn frames(gui: &mut Gui, dash: &mut Dashboard, events: Vec<egui::Event>) -> Vec<DashEvent> {
        let mut all = Vec::new();
        for events in [Vec::new(), events, Vec::new()] {
            let (mut full, output) = gui.run_dashboard(raw(events), dash);
            full.textures_delta.clear(); // no renderer in this input-level test
            all.extend(output);
        }
        all
    }
    fn centre(gui: &Gui, label: &str) -> egui::Pos2 {
        let (_, r) = gui.targets.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no {label:?} among {:?}", gui.targets.iter().map(|t| &t.0).collect::<Vec<_>>()));
        r.center()
    }
    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![egui::Event::PointerMoved(at), b(true), b(false)]
    }

    #[test]
    fn every_workbench_has_a_tile_and_a_click_asks_to_open_it() {
        let (mut gui, mut dash) = (Gui::new(), Dashboard::new(Recents::default(), None));
        frames(&mut gui, &mut dash, Vec::new());
        for w in crate::gpu_viewer::dashboard::WORKBENCHES {
            assert!(gui.targets.iter().any(|(l, _)| l == w.name), "{} has a tile", w.name);
        }
        let at = centre(&gui, "FEA Workbench");
        assert_eq!(frames(&mut gui, &mut dash, click(at)), [DashEvent::Open("fea", None)]);
    }

    #[test]
    fn the_filter_narrows_the_tiles_and_a_recent_file_reopens_in_its_workbench() {
        let mut recents = Recents::default();
        recents.record("fea", Some("/work/bracket.json".into()), crate::gpu_viewer::dashboard::unix_now());
        let (mut gui, mut dash) = (Gui::new(), Dashboard::new(recents, None));
        frames(&mut gui, &mut dash, Vec::new());
        let at = centre(&gui, "Recent bracket.json");
        assert_eq!(frames(&mut gui, &mut dash, click(at)), [DashEvent::Open("fea", Some(PathBuf::from("/work/bracket.json")))]);
        dash.query = "bushing".into();
        frames(&mut gui, &mut dash, Vec::new());
        let names: Vec<_> = gui.targets.iter().map(|t| t.0.as_str()).filter(|l| crate::gpu_viewer::dashboard::WORKBENCHES.iter().any(|w| w.name == *l)).collect();
        assert_eq!(names, ["Bushing", "Eccentric Bushing"]);
    }
}
