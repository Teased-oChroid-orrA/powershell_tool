//! The nav strip of the dashboard window (ADR-024): the way back to the dashboard, a tab per open workbench, the
//! "Open" switcher and "New window". It only reports `NavAct`s; `window/nav.rs` carries them out.
use super::{panels, theme, Gui};
use crate::gpu_viewer::{
    dashboard::{Workbench, WORKBENCHES},
    nav::{NavAct, NavStrip},
};
use egui::{Align, Align2, CornerRadius, Id, Layout, Margin, Order, RichText, Vec2};

/// The open switcher: a filter box over every workbench, the open ones first.
#[derive(Default)]
pub(super) struct SwitcherState {
    query: String,
    item: usize,
    /// Focus the filter on the next frame (just opened).
    fresh: bool,
}

impl Gui {
    /// What the strip shows this frame (`None`: standalone window, or nothing open yet).
    pub fn set_nav(&mut self, nav: Option<NavStrip>) {
        self.nav = nav;
    }
    pub fn open_switcher(&mut self) {
        self.switcher = Some(SwitcherState { fresh: true, ..Default::default() });
        self.palette = None;
        self.floating = None;
    }

    /// Draw the strip (when something is open) and the popups it raises; returns what was asked.
    pub(super) fn nav_ui(&mut self, ui: &mut egui::Ui, p: &theme::Palette) -> Vec<NavAct> {
        let mut acts = Vec::new();
        if let Some(nav) = self.nav.clone() {
            egui::Panel::top("toolbench-nav")
                .exact_size(38.0)
                .show_separator_line(false)
                .frame(panels::bar_frame(p, Margin::symmetric(8, 4)))
                .show(ui, |ui| self.strip(ui, p, &nav, &mut acts));
            self.close_question(ui.ctx(), p, &nav, &mut acts);
        }
        self.switcher_window(ui.ctx(), p, &mut acts);
        acts
    }

    fn strip(&mut self, ui: &mut egui::Ui, p: &theme::Palette, nav: &NavStrip, acts: &mut Vec<NavAct>) {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let home = ui.add(egui::Button::new(RichText::new("Toolbench").strong()).selected(nav.active.is_none()).corner_radius(CornerRadius::same(6)));
            self.targets.push(("Toolbench home".into(), home.rect));
            if home.on_hover_text("Back to the dashboard; open workbenches stay as they are").clicked() {
                acts.push(NavAct::Home);
            }
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
            for tab in &nav.tabs {
                let shown = nav.active == Some(tab.id);
                let mut name = RichText::new(tab.name);
                if shown {
                    name = name.strong();
                }
                let b = ui.add(egui::Button::new(name).selected(shown).corner_radius(CornerRadius::same(6)));
                self.targets.push((tab.name.to_string(), b.rect));
                if b.clicked() {
                    acts.push(NavAct::Open(tab.id));
                }
                if tab.dirty {
                    ui.label(RichText::new("●").color(p.warn)).on_hover_text("Unsaved edits");
                }
                if tab.busy {
                    ui.add(egui::Spinner::new().size(12.0)).on_hover_text("Solving");
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                }
                let x = ui.add(egui::Button::new(RichText::new("×").color(p.dim)).frame(false));
                self.targets.push((format!("Close {}", tab.name), x.rect));
                if x.on_hover_text(format!("Close {}", tab.name)).clicked() {
                    acts.push(NavAct::Close(tab.id));
                }
                ui.add_space(6.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(id) = nav.active {
                    let w = ui.add(egui::Button::new("New window").corner_radius(CornerRadius::same(6)));
                    self.targets.push(("New window".into(), w.rect));
                    if w.on_hover_text("Move this workbench into a window of its own").clicked() {
                        acts.push(NavAct::PopOut(id));
                    }
                }
                let o = ui.add(egui::Button::new("Open").shortcut_text(RichText::new("Ctrl/Cmd+J").small().color(p.dim)).corner_radius(CornerRadius::same(6)));
                self.targets.push(("Open workbench".into(), o.rect));
                if o.on_hover_text("Switch to or start a workbench").clicked() {
                    self.open_switcher();
                }
            });
        });
    }

    /// "Close X?" under the strip while its tab holds unsaved edits or a running solve.
    fn close_question(&mut self, ctx: &egui::Context, p: &theme::Palette, nav: &NavStrip, acts: &mut Vec<NavAct>) {
        let Some(tab) = nav.ask_close.and_then(|id| nav.tabs.iter().find(|t| t.id == id)) else { return };
        egui::Area::new(Id::new("nav-close-question")).order(Order::Foreground).anchor(Align2::CENTER_TOP, Vec2::new(0.0, 46.0)).show(ctx, |ui| {
            panels::popup_frame(p).show(ui, |ui| {
                let why = match (tab.dirty, tab.busy) {
                    (true, true) => "has unsaved edits and a solve is running",
                    (true, false) => "has unsaved edits",
                    _ => "is still solving",
                };
                ui.label(RichText::new(format!("{} {why}.", tab.name)).strong());
                ui.horizontal(|ui| {
                    let keep = ui.button("Keep open");
                    self.targets.push(("Keep open".into(), keep.rect));
                    if keep.clicked() {
                        acts.push(NavAct::KeepOpen);
                    }
                    let discard = ui.add(egui::Button::new(RichText::new("Discard and close").color(p.bad)));
                    self.targets.push(("Discard and close".into(), discard.rect));
                    if discard.clicked() {
                        acts.push(NavAct::Discard(tab.id));
                    }
                });
            });
        });
    }

    /// The switcher: type to filter, arrows and Enter pick, Esc closes. Open workbenches come first.
    fn switcher_window(&mut self, ctx: &egui::Context, p: &theme::Palette, acts: &mut Vec<NavAct>) {
        let Some(mut st) = self.switcher.take() else { return };
        let open: Vec<_> = self.nav.as_ref().map(|n| n.tabs.clone()).unwrap_or_default();
        let is_open = |w: &Workbench| open.iter().find(|t| t.id == w.id);
        let mut rows: Vec<&'static Workbench> = WORKBENCHES.iter().filter(|w| w.matches(&st.query)).collect();
        rows.sort_by_key(|w| is_open(w).is_none());
        st.item = st.item.min(rows.len().saturating_sub(1));
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if down && !rows.is_empty() {
            st.item = (st.item + 1) % rows.len();
        }
        if up && !rows.is_empty() {
            st.item = (st.item + rows.len() - 1) % rows.len();
        }
        let mut keep = !escape;
        let mut pick = (enter && !rows.is_empty()).then_some(st.item);
        let resp = egui::Area::new(Id::new("nav-switcher")).fade_in(false).order(Order::Foreground).anchor(Align2::CENTER_TOP, Vec2::new(0.0, 56.0)).show(ctx, |ui| {
            panels::popup_frame(p).show(ui, |ui| {
                ui.set_width(460.0);
                let r = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Switch to or open a workbench").desired_width(f32::INFINITY).margin(Vec2::new(8.0, 6.0)));
                if st.fresh {
                    r.request_focus();
                    st.fresh = false;
                }
                if rows.is_empty() {
                    ui.label(RichText::new("No workbench matches").color(p.dim));
                }
                for (k, w) in rows.iter().enumerate() {
                    let tab = is_open(w);
                    let b = ui.add(egui::Button::new(RichText::new(w.name)).selected(k == st.item).min_size(Vec2::new(ui.available_width(), 0.0)));
                    let badge = match tab {
                        Some(t) if t.dirty => "open, unsaved edits",
                        Some(_) => "open",
                        None => "",
                    };
                    if !badge.is_empty() {
                        ui.painter().text(b.rect.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, badge, egui::FontId::proportional(11.5), if tab.is_some_and(|t| t.dirty) { p.warn } else { p.accent });
                    }
                    self.targets.push((format!("Switch to {}", w.name), b.rect));
                    if b.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                        st.item = k;
                    }
                    if b.clicked() {
                        pick = Some(k);
                    }
                }
            });
        });
        let pressed_outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !resp.response.rect.contains(q));
        if pressed_outside {
            keep = false;
        }
        if let Some(k) = pick {
            acts.push(NavAct::Open(rows[k].id));
            keep = false;
        }
        if keep {
            self.switcher = Some(st);
        }
    }
}

