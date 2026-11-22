//! The dashboard screen: a filter box, the workbenches in three sections as glyph tiles (what is open, what was opened
//! last), and a rail of recent files (below the tiles when the window is narrow). Glyphs: `dashboard_glyphs`.
//! It fills the window; the window it lives in has no model, so nothing else is drawn.
use super::{dashboard_glyphs, theme, Gui};
use crate::gpu_viewer::{
    dashboard::{ago, unix_now, Dashboard, Group, Workbench},
    nav::NavAct,
};
use egui::{text::LayoutJob, Color32, CornerRadius, FontId, Rect, RichText, Sense, Shape, Stroke, Vec2};
use std::sync::Arc;
use std::path::PathBuf;

/// What the person asked the dashboard for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashEvent {
    /// Open workbench `.0` in this window, on `.1` when it is a recent file.
    Open(&'static str, Option<PathBuf>),
    /// As `Open`, in a window of its own (Shift-click).
    NewWindow(&'static str, Option<PathBuf>),
    /// The nav strip was used.
    Nav(NavAct),
}

const TILE_MIN: f32 = 236.0;
const TILE_H: f32 = 186.0;
const RAIL_W: f32 = 300.0;
/// The window width from which the recent files sit beside the tiles instead of below them.
const RAIL_FROM: f32 = 980.0;

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t)
}

/// `text` on one line, cut with an ellipsis at `width`.
fn one_line(ui: &egui::Ui, text: String, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::simple(text, font, color, width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

impl Gui {
    pub fn run_dashboard(&mut self, raw: egui::RawInput, dash: &mut Dashboard) -> (egui::FullOutput, Vec<DashEvent>) {
        let ctx = self.ctx.clone();
        let mut events = Vec::new();
        self.targets.clear();
        let full = ctx.run_ui(raw, |ui| self.dashboard(ui, dash, &mut events));
        (full, events)
    }

    fn dashboard(&mut self, ui: &mut egui::Ui, dash: &mut Dashboard, events: &mut Vec<DashEvent>) {
        theme::tick(ui.ctx());
        let p = theme::current(ui.ctx());
        let now = unix_now();
        events.extend(self.nav_ui(ui, &p).into_iter().map(DashEvent::Nav));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(32, 26)).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("GS Engineering Toolbench").size(26.0).strong());
                        ui.label(RichText::new("pick a workbench").color(p.dim));
                        ui.add_space(16.0);
                        theme::theme_picker(ui);
                        theme::circadian_control(ui, &p);
                    });
                    ui.add_space(12.0);
                    let search = egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(12, 9)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut dash.query).frame(egui::Frame::NONE).hint_text("Filter workbenches").desired_width(f32::INFINITY)).rect
                    });
                    self.targets.push(("Filter workbenches".into(), search.inner));
                    ui.add_space(22.0);
                    let wide = ui.available_width() >= RAIL_FROM;
                    if wide {
                        let gap = 28.0;
                        let left = ui.available_width() - RAIL_W - gap;
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.vertical(|ui| {
                                ui.set_width(left);
                                self.groups(ui, &p, dash, events, now);
                            });
                            ui.add_space(gap);
                            ui.vertical(|ui| {
                                ui.set_width(RAIL_W);
                                self.recent_rail(ui, &p, dash, events, now);
                            });
                        });
                    } else {
                        self.groups(ui, &p, dash, events, now);
                        ui.add_space(8.0);
                        self.recent_rail(ui, &p, dash, events, now);
                    }
                    if let Some(m) = &dash.message {
                        ui.add_space(14.0);
                        ui.label(RichText::new(m).color(p.dim));
                    }
                });
            });
        });
    }

    /// The tiles, one section per `Group` that has a workbench matching the filter.
    fn groups(&mut self, ui: &mut egui::Ui, p: &theme::Palette, dash: &Dashboard, events: &mut Vec<DashEvent>, now: u64) {
        let shown = dash.visible();
        if shown.is_empty() {
            ui.label(RichText::new("No workbench matches that.").color(p.dim));
        }
        let gap = 12.0;
        let width = ui.available_width();
        let cols = (((width + gap) / (TILE_MIN + gap)).floor() as usize).max(1);
        let tile_w = (width - gap * (cols as f32 - 1.0)) / cols as f32;
        for group in Group::ALL {
            let members: Vec<_> = shown.iter().copied().filter(|w| w.group == group).collect();
            if members.is_empty() {
                continue;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(group.name().to_uppercase()).size(12.0).strong().color(p.dim));
                ui.label(RichText::new(members.len().to_string()).monospace().size(11.5).color(p.muted));
                let (rule, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), Sense::hover());
                ui.painter().hline(rule.x_range(), rule.center().y, Stroke::new(1.0, p.line));
            });
            ui.add_space(8.0);
            for row in members.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for w in row {
                        if let Some(ev) = self.tile(ui, p, w, dash, Vec2::new(tile_w, TILE_H), now) {
                            events.push(ev);
                        }
                    }
                });
                ui.add_space(gap - 4.0);
            }
            ui.add_space(14.0);
        }
    }

    fn tile(&mut self, ui: &mut egui::Ui, p: &theme::Palette, w: &'static Workbench, dash: &Dashboard, size: Vec2, now: u64) -> Option<DashEvent> {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        self.targets.push((w.name.to_string(), rect));
        let hot = resp.hovered() || resp.has_focus();
        let painter = ui.painter_at(rect);
        painter.rect(rect, CornerRadius::same(14), if hot { p.raised } else { p.panel }, Stroke::new(1.0, if hot { p.accent } else { p.line }), egui::StrokeKind::Inside);
        let inner = rect.shrink(14.0);
        // The glyph box tints toward the accent while the tile is hot.
        let boxed = Rect::from_min_size(inner.min, Vec2::splat(56.0));
        let surface = if hot { mix(p.panel, p.accent, 0.14) } else { p.raised };
        painter.rect(boxed, CornerRadius::same(12), surface, Stroke::new(1.0, if hot { mix(p.line, p.accent, 0.45) } else { p.line }), egui::StrokeKind::Inside);
        let _ = dashboard_glyphs::paint(&painter, boxed.shrink(8.0), w.id, p, surface);
        if hot {
            // An arrow in the corner says "opens"; drawn, the UI font has no arrow glyph.
            let c = egui::pos2(inner.right() - 9.0, inner.top() + 9.0);
            let st = Stroke::new(1.6, p.accent);
            painter.add(Shape::line(vec![c + Vec2::new(-5.0, 5.0), c + Vec2::new(5.0, -5.0)], st));
            painter.add(Shape::line(vec![c + Vec2::new(-2.0, -5.0), c + Vec2::new(5.0, -5.0), c + Vec2::new(5.0, 2.0)], st));
        }
        let name = painter.layout_no_wrap(w.name.to_string(), FontId::proportional(15.0), p.text);
        let name_pos = egui::pos2(inner.left(), boxed.bottom() + 12.0);
        painter.galley(name_pos, name.clone(), p.text);
        let mut job = LayoutJob::simple(w.summary.to_string(), FontId::proportional(12.5), p.dim, inner.width());
        job.wrap.max_rows = 2;
        let body = painter.layout_job(job);
        painter.galley(name_pos + Vec2::new(0.0, name.size().y + 3.0), body, p.dim);
        // Footer: what is open, or what was opened last.
        let open = self.nav.as_ref().and_then(|n| n.tabs.iter().find(|t| t.id == w.id));
        let last = dash.recents.last(w.id);
        let (on, left, right) = match (open, last) {
            (Some(t), _) => (true, if t.dirty { "open now, unsaved edits" } else { "open now" }.to_string(), None),
            (None, Some(r)) => (false, r.path.as_deref().map(file_name).unwrap_or("last session").to_string(), Some(ago(now, r.opened))),
            (None, None) => (false, "not opened yet".to_string(), None),
        };
        let rule_y = inner.bottom() - 22.0;
        painter.extend(Shape::dashed_line(&[egui::pos2(inner.left(), rule_y), egui::pos2(inner.right(), rule_y)], Stroke::new(1.0, p.line), 3.0, 3.0));
        let mid = inner.bottom() - 7.0;
        let dot = egui::pos2(inner.left() + 3.0, mid);
        if on {
            painter.circle_filled(dot, 6.0, p.accent.gamma_multiply(0.22));
        }
        painter.circle_filled(dot, 3.0, if on { p.accent } else { p.line });
        let when = right.map(|t| painter.layout_no_wrap(t, FontId::monospace(11.0), p.muted));
        let room = inner.width() - 16.0 - when.as_ref().map_or(0.0, |g| g.size().x + 8.0);
        let foot = one_line(ui, left, FontId::monospace(11.5), if on { p.accent } else { p.muted }, room.max(20.0));
        painter.galley(egui::pos2(inner.left() + 14.0, mid - foot.size().y / 2.0), foot, p.muted);
        if let Some(g) = when {
            painter.galley(egui::pos2(inner.right() - g.size().x, mid - g.size().y / 2.0), g, p.muted);
        }
        let resp = resp.on_hover_text("Opens here. Shift-click opens a window of its own.");
        resp.clicked().then(|| if ui.input(|i| i.modifiers.shift) { DashEvent::NewWindow(w.id, None) } else { DashEvent::Open(w.id, None) })
    }

    /// Files opened from a workbench, newest first, one click to reopen (Shift-click: a window of its own).
    fn recent_rail(&mut self, ui: &mut egui::Ui, p: &theme::Palette, dash: &Dashboard, events: &mut Vec<DashEvent>, now: u64) {
        let items: Vec<_> = dash.recents.items.iter().filter(|r| r.path.is_some()).filter_map(|r| Some((Workbench::find(&r.workbench)?, r))).collect();
        egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(14)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("RECENT").size(11.0).strong().color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(items.len().to_string()).monospace().size(11.5).color(p.muted)));
            });
            ui.add_space(4.0);
            if items.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Files you open from a workbench appear here.").color(p.dim));
                });
                ui.add_space(6.0);
            }
            for (w, r) in items {
                let path = r.path.clone().unwrap_or_default();
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
                self.targets.push((format!("Recent {}", file_name(&path)), rect));
                let painter = ui.painter_at(rect);
                if resp.hovered() || resp.has_focus() {
                    painter.rect_filled(rect, CornerRadius::same(9), p.select);
                }
                let boxed = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 7.0), Vec2::splat(34.0));
                painter.rect(boxed, CornerRadius::same(8), p.raised, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
                let _ = dashboard_glyphs::paint(&painter, boxed.shrink(5.0), w.id, p, p.raised);
                let when = painter.layout_no_wrap(ago(now, r.opened), FontId::monospace(11.0), p.muted);
                let text_x = boxed.right() + 10.0;
                let room = (rect.right() - 8.0 - when.size().x - 8.0 - text_x).max(20.0);
                let file = one_line(ui, file_name(&path).to_string(), FontId::monospace(12.5), p.text, room);
                painter.galley(egui::pos2(text_x, rect.top() + 9.0), file, p.text);
                painter.text(egui::pos2(text_x, rect.top() + 27.0), egui::Align2::LEFT_TOP, w.name, FontId::proportional(11.5), p.dim);
                painter.galley(egui::pos2(rect.right() - 8.0 - when.size().x, rect.center().y - when.size().y / 2.0), when, p.muted);
                if resp.on_hover_text(&path).clicked() {
                    let file = Some(PathBuf::from(&path));
                    events.push(if ui.input(|i| i.modifiers.shift) { DashEvent::NewWindow(w.id, file) } else { DashEvent::Open(w.id, file) });
                }
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
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1100.0, 1700.0))), events, ..Default::default() }
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
    fn shift_click_asks_for_a_window_of_its_own_and_a_tab_marks_an_open_workbench() {
        use crate::gpu_viewer::nav::{NavStrip, NavTab};
        let (mut gui, mut dash) = (Gui::new(), Dashboard::new(Recents::default(), None));
        frames(&mut gui, &mut dash, Vec::new());
        let at = centre(&gui, "Lug Analysis");
        let mut with_shift = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT)];
        with_shift.extend(click(at));
        assert_eq!(frames(&mut gui, &mut dash, with_shift), [DashEvent::NewWindow("lug", None)]);
        frames(&mut gui, &mut dash, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        gui.set_nav(Some(NavStrip { tabs: vec![NavTab { id: "lug", name: "Lug Analysis", dirty: true, busy: false }], active: None, ask_close: None }));
        frames(&mut gui, &mut dash, Vec::new());
        assert!(gui.targets.iter().any(|(l, _)| l == "Toolbench home"), "the strip shows once a workbench is open");
        let home = centre(&gui, "Toolbench home");
        assert_eq!(frames(&mut gui, &mut dash, click(home)), [DashEvent::Nav(NavAct::Home)]);
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
        assert_eq!(names, ["Bushing", "Bushing Stackup", "Eccentric Bushing"]);
    }

    #[test]
    fn sections_list_every_workbench_once_in_order_and_the_rail_moves_below_when_narrow() {
        use crate::gpu_viewer::dashboard::{Group, WORKBENCHES};
        for g in Group::ALL {
            assert!(WORKBENCHES.iter().any(|w| w.group == g), "{} has workbenches", g.name());
        }
        let mut recents = Recents::default();
        recents.record("lug", Some("/work/lug-a.json".into()), crate::gpu_viewer::dashboard::unix_now());
        let (mut gui, mut dash) = (Gui::new(), Dashboard::new(recents, None));
        frames(&mut gui, &mut dash, Vec::new());
        // Sections draw in `Group::ALL` order, so the tiles' targets come out grouped.
        let order: Vec<_> = gui.targets.iter().filter_map(|(l, _)| WORKBENCHES.iter().find(|w| w.name == l)).map(|w| Group::ALL.iter().position(|g| *g == w.group).unwrap()).collect();
        assert_eq!(order.len(), WORKBENCHES.len());
        assert!(order.windows(2).all(|p| p[0] <= p[1]), "grouped: {order:?}");
        let wide = centre(&gui, "Recent lug-a.json");
        let tile = gui.targets.iter().find(|(l, _)| l == "Material Lookup").unwrap().1;
        assert!(wide.x > tile.right(), "beside the tiles when wide");
        // A narrow window stacks the rail under the tiles, and it still opens the file.
        let narrow = |events| egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(760.0, 2400.0))), events, ..Default::default() };
        let mut run = |gui: &mut Gui, events| {
            let (mut full, out) = gui.run_dashboard(narrow(events), &mut dash);
            full.textures_delta.clear();
            out
        };
        for _ in 0..2 {
            run(&mut gui, Vec::new());
        }
        let tile = gui.targets.iter().find(|(l, _)| l == "Material Lookup").unwrap().1;
        let at = centre(&gui, "Recent lug-a.json");
        assert!(at.y > tile.bottom(), "below the tiles when narrow");
        assert_eq!(run(&mut gui, click(at)), [DashEvent::Open("lug", Some(PathBuf::from("/work/lug-a.json")))]);
    }
}
