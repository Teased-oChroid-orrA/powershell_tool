//! A workbench drawn as three fixed panes (Bushing, `ui::Workbench`): the inputs on the left, the views of the model in the
//! middle (the 3D view over two dimensioned drawings, or one of them alone) and the results on the right. The case's
//! actions sit in the top bar (`panels::top_bar`) and a confirmation (`ui::Dialog`) opens over everything. Plain painting
//! of the editor's data; clicks come back as `Outcome`s. The drawings' geometry and the places of their labels are
//! `sketch::layout`'s, so the no-clash rule is tested without a window.
use super::{panels::bar_frame, theme::Palette, with_alpha, Gui};
use crate::gpu_viewer::{
    editor::{authoring::Card, session::Tone},
    menu::Outcome,
    sketch::{self, Fill, Ink, Sketch},
    menu::Entry,
    ui::{Compare, Dialog, DockItem, Look, Workbench, WorkbenchView},
};
use egui::{Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Order, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, Vec2};

/// Height of the views' toolbar.
const TOOLBAR: f32 = 44.0;
/// Share of the centre the 3D view takes over the drawings.
const MODEL_SHARE: f32 = 0.56;
/// Below this centre width the two drawings stack instead of sitting side by side.
const STACK_BELOW: f32 = 620.0;
/// Below this window width the results are a drawer over the views instead of a docked pane.
const DRAWER_BELOW: f32 = 1100.0;
/// Below this window width the inputs are a drawer too.
const INPUTS_DRAWER_BELOW: f32 = 700.0;
/// Width of an open drawer (at most 86 % of the window).
const DRAWER_WIDTH: f32 = 320.0;
/// Below this window width the top bar drops the status chips and folds the secondary actions into a menu.
const COMPACT_BAR_BELOW: f32 = 900.0;
/// Height of the strip under the views that a boxed workbench keeps for its status line and plot (at most 42 % of the body).
const UNDER: f32 = 292.0;
/// Height of the status line over the cards under the views.
const STATUS: f32 = 28.0;

/// A workbench pane that a narrow window folds into a drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Drawer {
    Inputs,
    Results,
}

impl Gui {
    pub(super) fn workbench(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let window = ui.available_rect_before_wrap();
        // Narrow windows give the views the room: the results (then also the inputs) become drawers over them.
        let results_drawer = window.width() < DRAWER_BELOW;
        let inputs_drawer = window.width() < INPUTS_DRAWER_BELOW;
        let boxed = wb.look == Look::Boxed;
        // Boxed cards sit on the page colour, so they read as raised from it.
        let pane = |margin: Margin| if boxed { bar_frame(p, margin).fill(p.bg) } else { bar_frame(p, margin) };
        if !inputs_drawer {
            egui::Panel::left("wb-inputs").default_size(if boxed { 330.0 } else { 300.0 }).size_range(250.0..=440.0).frame(pane(Margin::symmetric(12, 10))).show(ui, |ui| self.inputs_pane(ui, wb, p, out));
        }
        if !results_drawer {
            egui::Panel::right("wb-results").default_size(if boxed { 400.0 } else { 350.0 }).size_range(290.0..=540.0).frame(pane(Margin::symmetric(12, 10))).show(ui, |ui| self.results_pane(ui, wb, p, out));
        }
        let free = ui.available_rect_before_wrap();
        let narrow = free.width() < STACK_BELOW;
        let bar_height = if narrow { 92.0 } else { TOOLBAR };
        let bar = Rect::from_min_size(free.min, Vec2::new(free.width(), bar_height));
        ui.painter().rect_filled(bar, 0.0, p.panel);
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, p.line));
        ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(Vec2::new(10.0, 5.0))).id_salt("wb-toolbar"), |ui| {
            if boxed {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let k = wb.views.len().max(1) as f32;
                    self.segmented_equal(ui, &wb.views, (k * 96.0).min(ui.max_rect().width() * 0.4), p, out);
                    let k = wb.fields.len().max(1) as f32;
                    self.segmented_equal(ui, &wb.fields, (k * 104.0).min(ui.max_rect().width() * 0.5), p, out);
                    self.switches(ui, wb, p, out, hint);
                });
            } else if narrow {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let width = super::document::segmented_width(ui, &wb.views, p);
                        self.segmented(ui, &wb.views, width, p, out, hint);
                    });
                    ui.horizontal(|ui| self.views_toolbar(ui, wb, p, out, hint));
                });
            } else {
                ui.horizontal(|ui| {
                    let width = super::document::segmented_width(ui, &wb.views, p);
                    self.segmented(ui, &wb.views, width, p, out, hint);
                    self.views_toolbar(ui, wb, p, out, hint);
                });
            }
        });
        let whole = Rect::from_min_max(Pos2::new(free.left(), bar.bottom()), free.max);
        // What a boxed workbench keeps under the views: a status line and its cards (the plot).
        let under_h = match (wb.under.is_empty(), wb.view_status.is_some()) {
            (true, false) => 0.0,
            // A status line alone needs no room for cards.
            (true, true) => STATUS + 4.0,
            _ => UNDER.min(whole.height() * 0.42),
        };
        let body = Rect::from_min_max(whole.min, Pos2::new(whole.right(), whole.bottom() - under_h));
        if under_h > 0.0 {
            self.under_pane(ui, Rect::from_min_max(Pos2::new(whole.left(), body.bottom()), whole.max), wb, p, out);
        }
        match wb.show {
            WorkbenchView::Model => {
                self.viewport = Some(body);
                self.model_caption(ui, body, wb, p);
            }
            WorkbenchView::Split => {
                let share = if narrow { 0.36 } else { MODEL_SHARE };
                let model_h = (body.height() * share).max(160.0).min(body.height());
                let model = Rect::from_min_size(body.min, Vec2::new(body.width(), model_h));
                self.viewport = Some(model);
                self.model_caption(ui, model, wb, p);
                let rest = Rect::from_min_max(Pos2::new(body.left(), model.bottom()), body.max);
                let n = wb.sketches.len().max(1);
                let stack = body.width() < STACK_BELOW;
                for (k, s) in wb.sketches.iter().enumerate() {
                    let f = |i: usize| i as f32 / n as f32;
                    let r = if stack {
                        Rect::from_min_max(Pos2::new(rest.left(), rest.top() + rest.height() * f(k)), Pos2::new(rest.right(), rest.top() + rest.height() * f(k + 1)))
                    } else {
                        Rect::from_min_max(Pos2::new(rest.left() + rest.width() * f(k), rest.top()), Pos2::new(rest.left() + rest.width() * f(k + 1), rest.bottom()))
                    };
                    if wb.dialog.is_none() { paint_sketch(ui.painter(), r, s, p); }
                }
            }
            WorkbenchView::Sketch(i) => {
                // The 3D view keeps its rectangle (the camera stays as it was); the drawing covers it, and as an area it takes
                // the pointer so a drag over it does not turn the hidden model.
                self.viewport = Some(body);
                if let Some(s) = wb.sketches.get(i) {
                    egui::Area::new(Id::new("wb-sketch")).fade_in(false).order(Order::Middle).fixed_pos(body.min).show(ui.ctx(), |ui| {
                        let (r, _) = ui.allocate_exact_size(body.size(), Sense::click_and_drag());
                        paint_sketch(ui.painter(), r, s, p);
                    });
                }
            }
        }
        self.room = self.viewport;
        self.drawers(ui, window, wb, results_drawer, inputs_drawer, p, out);
    }

    fn inputs_pane(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>) {
        pane_title(ui, "Inputs", p);
        egui::ScrollArea::vertical().id_salt("wb-inputs-scroll").auto_shrink([false, false]).show(ui, |ui| {
            for c in &wb.inputs {
                self.pane_card(ui, c, true, wb.look, p, out);
            }
            if !wb.inputs_note.is_empty() {
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(&wb.inputs_note).size(11.5).color(p.dim)).wrap());
            }
        });
    }

    fn results_pane(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>) {
        pane_title(ui, "Results", p);
        egui::ScrollArea::vertical().id_salt("wb-results-scroll").auto_shrink([false, false]).show(ui, |ui| {
            for (k, c) in wb.results.iter().enumerate() {
                if k > 0 && wb.look == Look::Ruled {
                    ui.add_space(4.0);
                }
                self.pane_card(ui, c, false, wb.look, p, out);
            }
        });
    }

    /// The panes a narrow window folds: an edge tab for each (the results' one carries the verdict), and the open one slid
    /// over the views with a scrim; a click outside it or Esc closes it. The same cards, only placed differently.
    #[allow(clippy::too_many_arguments)]
    fn drawers(&mut self, ui: &mut egui::Ui, window: Rect, wb: &Workbench, results: bool, inputs: bool, p: &Palette, out: &mut Vec<Outcome>) {
        let ctx = ui.ctx().clone();
        if wb.dialog.is_some() || !(results || inputs) {
            self.drawer = None;
            return;
        }
        // A pane that is docked again (window grown) cannot stay open as a drawer.
        match self.drawer {
            Some(Drawer::Results) if !results => self.drawer = None,
            Some(Drawer::Inputs) if !inputs => self.drawer = None,
            _ => {}
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drawer = None;
        }
        let verdict = wb.chips.iter().find(|(_, t)| matches!(t, Tone::Warn | Tone::On));
        let mut tabs: Vec<(Drawer, String)> = Vec::new();
        if inputs {
            tabs.push((Drawer::Inputs, "Inputs".into()));
        }
        if results {
            tabs.push((Drawer::Results, verdict.map_or("Results".into(), |(v, _)| format!("Results  \u{b7}  {v}"))));
        }
        if self.drawer.is_none() {
            for (d, label) in &tabs {
                self.drawer_tab(&ctx, window, *d, label, verdict.filter(|_| *d == Drawer::Results).map(|(_, t)| *t), p);
            }
        }
        let Some(open) = self.drawer else { return };
        let width = DRAWER_WIDTH.min(window.width() * 0.86);
        let rect = match open {
            Drawer::Results => Rect::from_min_max(Pos2::new(window.right() - width, window.top()), window.max),
            Drawer::Inputs => Rect::from_min_max(window.min, Pos2::new(window.left() + width, window.bottom())),
        };
        let scrim = egui::Area::new(Id::new("wb-scrim")).order(Order::Foreground).fixed_pos(window.min).show(&ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(window.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(115));
            resp
        });
        if scrim.inner.clicked() {
            self.drawer = None;
        }
        egui::Area::new(Id::new("wb-drawer")).order(Order::Tooltip).fixed_pos(rect.min).show(&ctx, |ui| {
            ui.set_clip_rect(rect);
            ui.painter().rect_filled(rect, 0.0, p.panel);
            let (shadow_x, edge_x) = if open == Drawer::Results { (rect.left() - 10.0, rect.left()) } else { (rect.right(), rect.right()) };
            ui.painter().rect_filled(Rect::from_min_size(Pos2::new(shadow_x, rect.top()), Vec2::new(10.0, rect.height())), 0.0, Color32::from_black_alpha(60));
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                bar_frame(p, Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_min_size(rect.size() - Vec2::new(26.0, 22.0));
                    ui.set_max_size(rect.size() - Vec2::new(26.0, 22.0));
                    match open {
                        Drawer::Results => self.results_pane(ui, wb, p, out),
                        Drawer::Inputs => self.inputs_pane(ui, wb, p, out),
                    }
                });
            });
            ui.painter().line_segment([Pos2::new(edge_x, rect.top()), Pos2::new(edge_x, rect.bottom())], Stroke::new(1.0, p.line));
        });
    }

    /// A vertical tab on the window's edge that opens a drawer.
    fn drawer_tab(&mut self, ctx: &egui::Context, window: Rect, which: Drawer, label: &str, tone: Option<Tone>, p: &Palette) {
        let colour = match tone {
            Some(Tone::On) => p.good,
            Some(Tone::Warn) => p.warn,
            _ => p.accent,
        };
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(12.0), colour));
        let size = Vec2::new(galley.size().y + 14.0, galley.size().x + 24.0);
        // Results on the right edge, Inputs on the left; the inputs tab sits above the middle so two tabs never overlap.
        let (x, y) = match which {
            Drawer::Results => (window.right() - size.x, window.center().y - size.y / 2.0 + 0.0),
            Drawer::Inputs => (window.left(), window.center().y - size.y / 2.0),
        };
        let rect = Rect::from_min_size(Pos2::new(x, y), size);
        egui::Area::new(Id::new(("wb-drawer-tab", which as u8))).order(Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(rect.size(), Sense::click());
            let round = if which == Drawer::Results { CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 } } else { CornerRadius { nw: 0, sw: 0, ne: 8, se: 8 } };
            ui.painter().rect(r, round, if resp.hovered() { p.raised } else { p.panel }, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
            // Text runs downward: rotated a quarter turn clockwise about its top-left corner, which then sits at the right.
            let origin = Pos2::new(r.center().x + galley.size().y / 2.0, r.top() + 12.0);
            ui.painter().add(egui::epaint::TextShape::new(origin, galley, colour).with_angle(std::f32::consts::FRAC_PI_2));
            self.targets.push((format!("{label} tab"), r));
            if resp.clicked() {
                self.drawer = Some(which);
            }
        });
    }

    /// The view switch, what the 3D view is coloured by, its switches, and a note at the right.
    fn views_toolbar(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        ui.spacing_mut().item_spacing.x = 8.0;
        if !wb.fields.is_empty() {
            let current = wb.fields.iter().find(|e| e.checked == Some(true)).map_or("", |e| e.label.as_str()).to_string();
            let r = egui::ComboBox::from_id_salt("wb-field").selected_text(current).width(if ui.max_rect().width() < STACK_BELOW { 120.0 } else { 150.0 }).show_ui(ui, |ui| {
                for e in &wb.fields {
                    let r = ui.selectable_label(e.checked == Some(true), &e.label);
                    self.targets.push((e.label.clone(), r.rect));
                    if r.clicked() {
                        out.push(Outcome::Action(e.action));
                    }
                }
            });
            self.targets.push(("Colour by".into(), r.response.rect));
            r.response.on_hover_text("What the 3D view is coloured by");
        }
        self.switches(ui, wb, p, out, hint);
        if !wb.view_note.is_empty() && ui.max_rect().width() > 850.0 {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&wb.view_note).size(11.5).color(p.dim)).truncate());
            });
        }
    }

    /// The views' switches and buttons (Quarter cut, Reset view).
    fn switches(&mut self, ui: &mut egui::Ui, wb: &Workbench, _p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        for e in &wb.switches {
            match e.checked {
                Some(on) => {
                    let mut v = on;
                    let r = ui.checkbox(&mut v, RichText::new(&e.label).size(13.0));
                    self.targets.push((e.label.clone(), r.rect));
                    self.entry_click(&r, e, out, hint);
                }
                None => {
                    let r = ui.add(egui::Button::new(RichText::new(&e.label).size(13.0)).frame_when_inactive(false));
                    self.targets.push((e.label.clone(), r.rect));
                    self.entry_click(&r, e, out, hint);
                }
            }
        }
    }

    /// The 3D view's title at its top left and the hint at its bottom left, over the model.
    fn model_caption(&self, ui: &egui::Ui, r: Rect, wb: &Workbench, p: &Palette) {
        let painter = ui.painter();
        let at = r.left_top() + Vec2::new(12.0, 10.0);
        // A boxed workbench's title names the model whole (it may be a 2D one).
        let x = if wb.look == Look::Boxed { at.x } else { painter.text(at, Align2::LEFT_TOP, "3D", FontId::proportional(11.0), p.dim).right() + 6.0 };
        if wb.look == Look::Boxed {
            // A chip behind the title: the model's own edge runs under it.
            let g = painter.layout_no_wrap(wb.model_title.clone(), FontId::proportional(11.5), p.text);
            painter.rect_filled(Rect::from_min_size(at - Vec2::new(8.0, 4.0), g.size() + Vec2::new(16.0, 8.0)), 6.0, with_alpha(p.panel, 225));
        }
        painter.text(Pos2::new(x, at.y), Align2::LEFT_TOP, &wb.model_title, FontId::proportional(11.5), p.text);
        painter.text(r.left_bottom() + Vec2::new(12.0, -8.0), Align2::LEFT_BOTTOM, &wb.model_hint, FontId::proportional(11.0), p.dim);
    }

    /// One section of a pane: a header that folds it (title, a tag at the right), then its rows. Inputs draw their values
    /// as boxes.
    fn pane_card(&mut self, ui: &mut egui::Ui, card: &Card, inputs: bool, look: Look, p: &Palette, out: &mut Vec<Outcome>) {
        if look == Look::Boxed {
            return self.boxed_card(ui, card, inputs, p, out);
        }
        let open = *self.card_open.get(&card.id).unwrap_or(&!card.folded);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
        self.targets.push((format!("{} card", card.title), rect));
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, 5.0, p.raised);
        }
        let mid = rect.center().y;
        let c = Pos2::new(rect.left() + 7.0, mid);
        let tri = if open { vec![c + Vec2::new(-3.5, -2.0), c + Vec2::new(3.5, -2.0), c + Vec2::new(0.0, 3.0)] } else { vec![c + Vec2::new(-2.0, -3.5), c + Vec2::new(3.0, 0.0), c + Vec2::new(-2.0, 3.5)] };
        painter.add(egui::Shape::convex_polygon(tri, p.dim, Stroke::NONE));
        let title = painter.text(Pos2::new(rect.left() + 18.0, mid), Align2::LEFT_CENTER, card.title.to_uppercase(), FontId::proportional(11.0), p.dim);
        if !card.tag.is_empty() {
            let room = rect.right() - title.right() - 16.0;
            let tag = painter.layout(card.tag.clone(), FontId::proportional(11.0), p.dim, room.max(40.0));
            painter.galley(Pos2::new(rect.right() - 4.0 - tag.size().x, mid - tag.size().y / 2.0), tag, p.dim);
        }
        if resp.clicked() {
            self.card_open.insert(card.id.clone(), !open);
        }
        if open {
            ui.spacing_mut().item_spacing.y = 3.0;
            for item in &card.items {
                match item {
                    DockItem::Field { label, value, tone, action, focused } if inputs => self.input_row(ui, label, value, *tone, *action, *focused, false, p, out),
                    _ => self.dock_item(ui, item, p, out),
                }
            }
        }
        ui.add_space(4.0);
        let y = ui.cursor().top();
        ui.painter().line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, with_alpha(p.line, 160)));
        ui.add_space(4.0);
    }

    /// A boxed workbench's card: a rounded box on the page, its title and a state chip in a header that folds it.
    fn boxed_card(&mut self, ui: &mut egui::Ui, card: &Card, inputs: bool, p: &Palette, out: &mut Vec<Outcome>) {
        let open = *self.card_open.get(&card.id).unwrap_or(&!card.folded);
        egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::click());
            self.targets.push((format!("{} card", card.title), rect));
            let painter = ui.painter();
            let mid = rect.center().y;
            // A chevron says the card folds: down when open, right when folded.
            let c = Pos2::new(rect.left() + 4.0, mid);
            let tri = if open { vec![c + Vec2::new(-3.5, -2.0), c + Vec2::new(3.5, -2.0), c + Vec2::new(0.0, 3.0)] } else { vec![c + Vec2::new(-2.0, -3.5), c + Vec2::new(3.0, 0.0), c + Vec2::new(-2.0, 3.5)] };
            painter.add(egui::Shape::convex_polygon(tri, p.dim, Stroke::NONE));
            let title = painter.text(Pos2::new(rect.left() + 16.0, mid), Align2::LEFT_CENTER, card.title.to_uppercase(), FontId::proportional(11.0), if resp.hovered() { p.text } else { p.dim });
            if !card.tag.is_empty() {
                let colour = tone_colour(p, card.tag_tone);
                let room = (rect.right() - title.right() - 12.0).max(40.0);
                // A tag longer than the room is cut with an ellipsis, never clipped mid-letter.
                let mut text = card.tag.clone();
                let mut g = painter.layout_no_wrap(text.clone(), FontId::proportional(11.0), colour);
                while g.size().x + 16.0 > room && text.chars().count() > 2 {
                    text.pop();
                    g = painter.layout_no_wrap(format!("{}\u{2026}", text.trim_end()), FontId::proportional(11.0), colour);
                }
                let w = g.size().x + 16.0;
                let chip = Rect::from_min_size(Pos2::new(rect.right() - w, mid - 10.0), Vec2::new(w, 20.0));
                painter.rect(chip, CornerRadius::same(10), if card.tag_tone == Tone::Dim { Color32::TRANSPARENT } else { with_alpha(colour, 30) }, Stroke::new(1.0, with_alpha(colour, 120)), StrokeKind::Inside);
                painter.galley(Pos2::new(chip.left() + 8.0, mid - g.size().y / 2.0), g, colour);
            }
            if resp.clicked() {
                self.card_open.insert(card.id.clone(), !open);
            }
            if open {
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                for item in &card.items {
                    match item {
                        DockItem::Field { label, value, tone, action, focused } if inputs => self.input_row(ui, label, value, *tone, *action, *focused, true, p, out),
                        _ => self.dock_item(ui, item, p, out),
                    }
                }
            }
        });
        ui.add_space(10.0);
    }

    /// The strip under the views: the last run's status line, then the cards (the load-travel plot).
    fn under_pane(&mut self, ui: &mut egui::Ui, r: Rect, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>) {
        ui.painter().rect_filled(r, 0.0, p.bg);
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, p.line));
        if let Some((text, tone)) = &wb.view_status {
            let colour = tone_colour(p, *tone);
            let at = Pos2::new(r.left() + 16.0, r.top() + STATUS / 2.0 + 2.0);
            ui.painter().circle_filled(at, 4.0, if *tone == Tone::Dim { p.dim } else { colour });
            let g = ui.painter().layout(text.clone(), FontId::proportional(12.0), p.dim, (r.width() - 44.0).max(60.0));
            ui.painter().galley(Pos2::new(at.x + 12.0, at.y - 7.0), g, p.dim);
        }
        let cards = Rect::from_min_max(Pos2::new(r.left() + 10.0, r.top() + STATUS + 2.0), r.max - Vec2::new(10.0, 0.0));
        if wb.dialog.is_some() || wb.under.is_empty() || cards.height() < 40.0 {
            return;
        }
        ui.scope_builder(UiBuilder::new().max_rect(cards).id_salt("wb-under"), |ui| {
            egui::ScrollArea::vertical().id_salt("wb-under-scroll").auto_shrink([false, false]).show(ui, |ui| {
                for c in &wb.under {
                    self.pane_card(ui, c, false, wb.look, p, out);
                }
            });
        });
    }

    /// One choice of a few as a segmented control across `width`: equal segments, the chosen one filled with the accent.
    pub(super) fn segmented_equal(&mut self, ui: &mut egui::Ui, entries: &[Entry], width: f32, p: &Palette, out: &mut Vec<Outcome>) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(40.0), 30.0), Sense::click());
        ui.painter().rect(rect, CornerRadius::same(9), p.raised, Stroke::new(1.0, p.line), StrokeKind::Inside);
        let w = (rect.width() - 4.0) / entries.len().max(1) as f32;
        let at = |q: Pos2| (((q.x - rect.left() - 2.0) / w).floor().max(0.0) as usize).min(entries.len().saturating_sub(1));
        let hovered = resp.hover_pos().map(at);
        for (i, e) in entries.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(rect.left() + 2.0 + w * i as f32, rect.top() + 2.0), Vec2::new(w, rect.height() - 4.0));
            self.targets.push((e.label.clone(), r));
            let on = e.checked == Some(true);
            let colour = if on {
                ui.painter().rect_filled(r, CornerRadius::same(7), p.accent);
                p.on_accent
            } else if !e.enabled {
                with_alpha(p.dim, 120)
            } else if hovered == Some(i) {
                ui.painter().rect_filled(r, CornerRadius::same(7), p.panel);
                p.text
            } else {
                p.dim
            };
            ui.painter().with_clip_rect(r).text(r.center(), Align2::CENTER_CENTER, &e.label, FontId::proportional(12.0), colour);
        }
        if let Some(i) = hovered {
            let tip = entries[i].hover_text();
            if !tip.is_empty() {
                resp.clone().on_hover_text(tip);
            }
        }
        if let (true, Some(q)) = (resp.clicked(), resp.interact_pointer_pos()) {
            let e = &entries[at(q)];
            out.push(match (&e.reason, e.enabled) {
                (Some(reason), false) => Outcome::Blocked(reason.clone()),
                _ => Outcome::Action(e.action),
            });
        }
    }

    /// Named rows with a detail line and a figure per column; the governing row has an accent bar and a tint.
    pub(super) fn compare_ui(&mut self, ui: &mut egui::Ui, c: &Compare, p: &Palette) {
        let width = ui.available_width();
        let figures = c.heads.len().saturating_sub(1);
        let figure_font = FontId::monospace(12.5);
        let head_font = FontId::proportional(10.5);
        let measure = |t: &str, f: &FontId| ui.painter().layout_no_wrap(t.to_string(), f.clone(), p.text).size().x;
        // Each figure column is as wide as its widest figure or its head, plus a gap, so a head sits over its figures.
        let col_w: Vec<f32> = (0..figures)
            .map(|j| {
                let figs = c.rows.iter().filter_map(|r| r.values.get(j)).map(|v| measure(v, &figure_font)).fold(0.0_f32, f32::max);
                figs.max(measure(&c.heads[j + 1].to_uppercase(), &head_font)).max(30.0) + 14.0
            })
            .collect();
        let chip_w = if c.rows.iter().any(|r| r.chip.is_some()) { 70.0 } else { 0.0 };
        let name_w = (width - col_w.iter().sum::<f32>() - chip_w - 6.0).max(80.0);
        // Right edge of figure column `k`, from the left of the card.
        let col_right = |k: usize| width - chip_w - col_w[k + 1..].iter().sum::<f32>() - 2.0;
        let (head, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
        for (k, h) in c.heads.iter().enumerate() {
            let (align, x) = if k == 0 { (Align2::LEFT_CENTER, head.left()) } else { (Align2::RIGHT_CENTER, head.left() + col_right(k - 1)) };
            ui.painter().text(Pos2::new(x, head.center().y), align, h.to_uppercase(), head_font.clone(), p.dim);
        }
        let n = c.rows.len();
        for (k, row) in c.rows.iter().enumerate() {
            let text_col = if row.dim { p.dim } else { p.text };
            let title = ui.painter().layout(row.title.clone(), FontId::proportional(13.0), text_col, name_w - 4.0);
            let detail = (!row.detail.is_empty()).then(|| ui.painter().layout(row.detail.clone(), FontId::proportional(11.0), p.dim, name_w - 4.0));
            let bar_h = if row.bar.is_some() { 12.0 } else { 0.0 };
            let height = title.size().y + detail.as_ref().map_or(0.0, |d| d.size().y + 1.0) + bar_h + 14.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            self.targets.push((row.title.clone(), rect));
            let painter = ui.painter();
            if row.governing {
                // To the card's edges, as the mockup's row runs under its padding.
                let band = Rect::from_min_max(Pos2::new(rect.left() - 12.0, rect.top()), Pos2::new(rect.right() + 12.0, rect.bottom()));
                painter.rect_filled(band, 0.0, with_alpha(p.accent, 24));
                painter.rect_filled(Rect::from_min_size(band.left_top(), Vec2::new(3.0, band.height())), 0.0, p.accent);
            } else if k + 1 < n && !c.rows[k + 1].governing {
                painter.line_segment([Pos2::new(rect.left(), rect.bottom()), Pos2::new(rect.right(), rect.bottom())], Stroke::new(1.0, with_alpha(p.line, 170)));
            }
            let mut y = rect.top() + 7.0;
            let title_h = title.size().y;
            painter.galley(Pos2::new(rect.left(), y), title, text_col);
            y += title_h;
            if let Some(d) = detail {
                let h = d.size().y;
                painter.galley(Pos2::new(rect.left(), y + 1.0), d, p.dim);
                y += h + 1.0;
            }
            if let Some(share) = row.bar {
                let track = Rect::from_min_size(Pos2::new(rect.left(), y + 4.0), Vec2::new(name_w * 0.9, 6.0));
                painter.rect_filled(track, 3.0, p.raised);
                painter.rect_filled(Rect::from_min_size(track.min, Vec2::new((track.width() * share.clamp(0.0, 1.0)).max(4.0), track.height())), 3.0, p.accent);
            }
            let mid = rect.top() + 7.0 + 8.0;
            for (j, v) in row.values.iter().enumerate() {
                let colour = if v == "-" || row.dim { p.dim } else if row.governing { p.text } else { p.text };
                painter.text(Pos2::new(rect.left() + col_right(j), mid), Align2::RIGHT_CENTER, v, figure_font.clone(), colour);
            }
            if let Some((text, tone)) = &row.chip {
                let colour = tone_colour(p, *tone);
                let g = painter.layout_no_wrap(text.clone(), FontId::proportional(11.0), colour);
                let w = (g.size().x + 14.0).min(chip_w);
                let chip = Rect::from_min_size(Pos2::new(rect.right() - w, mid - 9.0), Vec2::new(w, 18.0));
                painter.rect(chip, CornerRadius::same(9), with_alpha(colour, 30), Stroke::new(1.0, with_alpha(colour, 120)), StrokeKind::Inside);
                painter.with_clip_rect(chip).galley(Pos2::new(chip.left() + 7.0, mid - g.size().y / 2.0), g, colour);
            }
        }
        if !c.footer.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&c.footer).size(11.5).color(p.dim)).wrap());
        }
    }

    /// An input: its label, and its value in a box at the right (the edit box while it is being typed).
    #[allow(clippy::too_many_arguments)]
    fn input_row(&mut self, ui: &mut egui::Ui, label: &str, value: &str, tone: Tone, action: Option<crate::gpu_viewer::controller::UiAction>, focused: bool, boxed: bool, p: &Palette, out: &mut Vec<Outcome>) {
        let width = ui.available_width();
        let value_w = ui.painter().layout_no_wrap(value.to_string(), FontId::monospace(12.0), p.text).size().x + 18.0;
        // A long value (a material's name) gets more of the row, at most two thirds; past that it is cut with an ellipsis.
        let box_w = (width * 0.46).clamp(96.0, 170.0).max(value_w.min(width * 0.66));
        let (flag, label) = match label.strip_prefix("! ") {
            Some(rest) => (true, rest),
            None => (false, label),
        };
        let label_w = width - box_w - if flag { 26.0 } else { 8.0 };
        let galley = ui.painter().layout(label.to_string(), FontId::proportional(13.0), p.text, label_w.max(40.0));
        let sense = if action.is_some() { Sense::click() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, (galley.size().y + 8.0).max(28.0)), sense);
        self.targets.push((label.to_string(), rect));
        let painter = ui.painter();
        let mid = rect.center().y;
        let mut x = rect.left() + 2.0;
        if flag {
            let r = Rect::from_center_size(Pos2::new(x + 7.0, mid), Vec2::splat(14.0));
            painter.rect_filled(r, 3.0, with_alpha(p.warn, 56));
            painter.text(r.center(), Align2::CENTER_CENTER, "!", FontId::proportional(10.5), p.warn);
            x += 20.0;
        }
        painter.galley(Pos2::new(x, mid - galley.size().y / 2.0), galley, p.text);
        let b = Rect::from_min_max(Pos2::new(rect.right() - box_w, rect.top() + 2.0), Pos2::new(rect.right(), rect.bottom() - 2.0));
        let editing = focused || tone == Tone::Warn;
        let stroke = if editing { Stroke::new(1.4, p.accent) } else if resp.hovered() && action.is_some() { Stroke::new(1.0, p.dim) } else { Stroke::new(1.0, p.line) };
        if boxed && action.is_none() {
            // A figure the window works out: no box to type in.
            painter.rect_filled(b, 7.0, with_alpha(p.raised, 150));
        } else {
            painter.rect(b, if boxed { 7.0 } else { 5.0 }, if boxed { p.raised } else { p.bg }, stroke, egui::StrokeKind::Inside);
        }
        let vcol = if editing { p.accent } else if tone == Tone::On { p.good } else if boxed && action.is_none() { p.dim } else { p.text };
        let clip = b.shrink2(Vec2::new(8.0, 0.0));
        let mut shown = value.to_string();
        let mut value_g = painter.layout_no_wrap(shown.clone(), FontId::monospace(12.0), vcol);
        while boxed && value_g.size().x > clip.width() && shown.chars().count() > 2 {
            shown.pop();
            value_g = painter.layout_no_wrap(format!("{}\u{2026}", shown.trim_end()), FontId::monospace(12.0), vcol);
        }
        // A boxed workbench's figures sit at the right of their box, as typed numbers do.
        let x = if boxed { (clip.right() - value_g.size().x).max(clip.left()) } else { clip.left() };
        painter.with_clip_rect(clip).galley(Pos2::new(x, mid - value_g.size().y / 2.0), value_g, vcol);
        if let (Some(a), true) = (action, resp.clicked()) {
            out.push(Outcome::Action(a));
        }
    }

    /// The confirmation over the window: what will be sent, the choices, a note, Cancel and the action.
    pub(super) fn workbench_dialog(&mut self, ctx: &egui::Context, d: &Dialog, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let shown = egui::Modal::new(Id::new("wb-dialog")).frame(egui::Frame::popup(&ctx.global_style()).fill(p.panel).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(20))).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new(&d.title).size(17.0).strong());
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&d.intro).size(13.0).color(p.dim)).wrap());
            ui.add_space(10.0);
            egui::Frame::new().fill(p.raised).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("wb-dialog-rows").num_columns(2).spacing(Vec2::new(14.0, 5.0)).show(ui, |ui| {
                    for (k, v) in &d.rows {
                        ui.label(RichText::new(k).size(12.5).color(p.dim));
                        ui.add(egui::Label::new(RichText::new(v).size(12.5)).wrap());
                        ui.end_row();
                    }
                });
            });
            ui.add_space(10.0);
            for o in &d.options {
                let width = ui.available_width();
                let label_col = if o.enabled { p.text } else { p.dim };
                let detail = ui.painter().layout(o.detail.clone(), FontId::proportional(11.5), p.dim, width - 34.0);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0 + detail.size().y), if o.enabled && o.action.is_some() { Sense::click() } else { Sense::hover() });
                self.targets.push((o.label.clone(), rect));
                let painter = ui.painter();
                if resp.hovered() && o.enabled && o.action.is_some() {
                    painter.rect_filled(rect, 6.0, p.raised);
                }
                let bx = Rect::from_min_size(rect.left_top() + Vec2::new(4.0, 4.0), Vec2::splat(15.0));
                if o.on {
                    painter.rect_filled(bx, 4.0, if o.enabled { p.accent } else { p.dim });
                    let c = bx.center();
                    painter.add(egui::Shape::line(vec![c + Vec2::new(-4.0, 0.0), c + Vec2::new(-1.0, 3.0), c + Vec2::new(4.0, -3.0)], Stroke::new(1.8, p.on_accent)));
                } else {
                    painter.rect_stroke(bx, 4.0, Stroke::new(1.2, if o.enabled { p.dim } else { with_alpha(p.dim, 120) }), egui::StrokeKind::Inside);
                }
                painter.text(Pos2::new(bx.right() + 10.0, rect.top() + 4.0), Align2::LEFT_TOP, &o.label, FontId::proportional(13.0), label_col);
                painter.galley(Pos2::new(bx.right() + 10.0, rect.top() + 21.0), detail, p.dim);
                if let (Some(a), true, true) = (o.action, o.enabled, resp.clicked()) {
                    out.push(Outcome::Action(a));
                }
                ui.add_space(4.0);
            }
            if !d.note.is_empty() {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(&d.note).size(12.0).color(p.dim)).wrap());
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                self.doc_button(ui, &d.cancel, p, out, hint, false);
                self.doc_button(ui, &d.confirm, p, out, hint, true);
            });
        });
        if shown.should_close() {
            out.push(Outcome::Action(d.cancel.action));
        }
    }

    /// The workbench's chips after the title in the top bar (the units, the verdict).
    pub(super) fn workbench_bar(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette) {
        // The chips come second to the title and the actions when the window is small (the verdict stays on its drawer tab).
        if ui.ctx().content_rect().width() < COMPACT_BAR_BELOW {
            return;
        }
        for (text, tone) in &wb.chips {
            let c = match tone {
                Tone::Warn => p.warn,
                Tone::On => p.good,
                _ => p.dim,
            };
            let fill = if matches!(tone, Tone::Warn | Tone::On) { with_alpha(c, 30) } else { Color32::TRANSPARENT };
            egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, with_alpha(c, 130))).corner_radius(CornerRadius::same(13)).inner_margin(Margin::symmetric(10, 3)).show(ui, |ui| {
                ui.label(RichText::new(text).size(12.0).color(if matches!(tone, Tone::Warn | Tone::On) { c } else { p.text }));
            });
        }
    }

    /// The case's actions, inside the bar's right-to-left part: the last (the main one) at the right in the accent colour.
    pub(super) fn workbench_actions(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let n = wb.actions.len();
        let compact = ui.ctx().content_rect().width() < COMPACT_BAR_BELOW;
        if compact && n > 1 {
            // Small window: only the main action stays in the bar; the others fold into one menu so nothing overlaps.
            self.doc_button(ui, &wb.actions[n - 1], p, out, hint, true);
            let r = ui.menu_button("More", |ui| {
                for e in &wb.actions[..n - 1] {
                    let r = ui.add_enabled(e.enabled, egui::Button::new(&e.label));
                    self.targets.push((e.label.clone(), r.rect));
                    let r = if e.shortcut.is_empty() { r } else { r.on_hover_text(&e.shortcut) };
                    if r.clicked() {
                        ui.close();
                    }
                    self.entry_click(&r, e, out, hint);
                }
            });
            self.targets.push(("More".into(), r.response.rect));
            return;
        }
        for (k, e) in wb.actions.iter().enumerate().rev() {
            if k + 1 == n {
                self.doc_button(ui, e, p, out, hint, true);
            } else {
                let r = ui.add(egui::Button::new(RichText::new(&e.label).color(if e.enabled { p.text } else { p.dim.gamma_multiply(0.75) })).frame_when_inactive(false).min_size(Vec2::new(0.0, 28.0)));
                self.targets.push((e.label.clone(), r.rect));
                let r = if e.shortcut.is_empty() { r } else { r.on_hover_text(&e.shortcut) };
                self.entry_click(&r, e, out, hint);
            }
        }
    }
}

fn tone_colour(p: &Palette, tone: Tone) -> Color32 {
    super::theme::tone_color(p, tone)
}

fn pane_title(ui: &mut egui::Ui, title: &str, p: &Palette) {
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(p.dim));
    ui.add_space(4.0);
}

/// Paint a drawing in `r`: its panel, title and caption, then the layout's fills, lines, arrowheads and labels.
pub(super) fn paint_sketch(painter: &egui::Painter, r: Rect, s: &Sketch, p: &Palette) {
    painter.rect(r, 0.0, p.panel, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    let at = r.left_top() + Vec2::new(12.0, 10.0);
    let title = painter.text(at, Align2::LEFT_TOP, s.title.to_uppercase(), FontId::proportional(11.0), p.dim);
    painter.text(Pos2::new(title.right() + 6.0, at.y), Align2::LEFT_TOP, &s.caption, FontId::proportional(11.5), p.text);
    let area = sketch::Rect { min: [r.left() + 8.0, r.top() + 30.0], max: [r.right() - 8.0, r.bottom() - 8.0] };
    if area.width() < 40.0 || area.height() < 40.0 {
        return;
    }
    let font = FontId::proportional(11.0);
    let measure = |t: &str| {
        let g = painter.layout_no_wrap(t.to_string(), font.clone(), p.text);
        [g.size().x, g.size().y]
    };
    let l = sketch::layout(s, area, &measure);
    let pt = |q: &sketch::Q| Pos2::new(q[0], q[1]);
    let painter = painter.with_clip_rect(r.shrink(1.0));
    for g in &l.regions {
        let fill = match g.fill {
            Fill::Housing => p.raised,
            Fill::Bushing => with_alpha(p.accent, 46),
            Fill::Hole => p.panel,
        };
        painter.add(egui::Shape::convex_polygon(g.points.iter().map(pt).collect(), fill, Stroke::NONE));
    }
    for s in &l.strokes {
        let pts: Vec<Pos2> = s.points.iter().map(pt).collect();
        match s.ink {
            Ink::Center => painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(0.8, p.dim), 9.0, 3.0)),
            Ink::Ghost => painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.3, p.warn), 5.0, 3.5)),
            ink => {
                painter.add(egui::Shape::line(pts, ink_stroke(ink, p)));
            }
        }
    }
    for h in &l.heads {
        let (tip, d) = (pt(&h.tip), Vec2::new(h.dir[0], h.dir[1]));
        let n = Vec2::new(-d.y, d.x);
        let base = tip - d * sketch::ARROW;
        painter.add(egui::Shape::convex_polygon(vec![tip, base + n * sketch::ARROW * 0.36, base - n * sketch::ARROW * 0.36], ink_stroke(h.ink, p).color, Stroke::NONE));
    }
    for label in &l.labels {
        let col = match label.ink {
            Ink::Ghost => p.warn,
            Ink::Load => p.accent,
            Ink::Center => p.dim,
            _ => p.text,
        };
        painter.text(pt(&label.rect.min), Align2::LEFT_TOP, &label.text, font.clone(), col);
    }
}

fn ink_stroke(ink: Ink, p: &Palette) -> Stroke {
    match ink {
        Ink::Outline => Stroke::new(1.4, p.text),
        Ink::Thin => Stroke::new(0.8, p.dim),
        Ink::Center => Stroke::new(0.8, p.dim),
        Ink::Ghost => Stroke::new(1.3, p.warn),
        Ink::Load => Stroke::new(1.8, p.accent),
    }
}
