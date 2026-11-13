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
    ui::{Dialog, DockItem, Workbench, WorkbenchView},
};
use egui::{Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Order, Pos2, Rect, RichText, Sense, Stroke, UiBuilder, Vec2};

/// Height of the views' toolbar.
const TOOLBAR: f32 = 44.0;
/// Share of the centre the 3D view takes over the drawings.
const MODEL_SHARE: f32 = 0.56;
/// Below this centre width the two drawings stack instead of sitting side by side.
const STACK_BELOW: f32 = 620.0;

impl Gui {
    pub(super) fn workbench(&mut self, ui: &mut egui::Ui, wb: &Workbench, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        egui::Panel::left("wb-inputs").default_size(300.0).size_range(250.0..=440.0).frame(bar_frame(p, Margin::symmetric(12, 10))).show(ui, |ui| {
            pane_title(ui, "Inputs", p);
            egui::ScrollArea::vertical().id_salt("wb-inputs-scroll").auto_shrink([false, false]).show(ui, |ui| {
                for c in &wb.inputs {
                    self.pane_card(ui, c, true, p, out, hint);
                }
                if !wb.inputs_note.is_empty() {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(&wb.inputs_note).size(11.5).color(p.dim)).wrap());
                }
            });
        });
        egui::Panel::right("wb-results").default_size(350.0).size_range(290.0..=540.0).frame(bar_frame(p, Margin::symmetric(12, 10))).show(ui, |ui| {
            pane_title(ui, "Results", p);
            egui::ScrollArea::vertical().id_salt("wb-results-scroll").auto_shrink([false, false]).show(ui, |ui| {
                for (k, c) in wb.results.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(4.0);
                    }
                    self.pane_card(ui, c, false, p, out, hint);
                }
            });
        });
        let free = ui.available_rect_before_wrap();
        let narrow = free.width() < STACK_BELOW;
        let bar_height = if narrow { 92.0 } else { TOOLBAR };
        let bar = Rect::from_min_size(free.min, Vec2::new(free.width(), bar_height));
        ui.painter().rect_filled(bar, 0.0, p.panel);
        ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, p.line));
        ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(Vec2::new(10.0, 5.0))).id_salt("wb-toolbar"), |ui| {
            if narrow {
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
        let body = Rect::from_min_max(Pos2::new(free.left(), bar.bottom()), free.max);
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
        if !wb.view_note.is_empty() && ui.max_rect().width() > 850.0 {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&wb.view_note).size(11.5).color(p.dim)).truncate());
            });
        }
    }

    /// The 3D view's title at its top left and the hint at its bottom left, over the model.
    fn model_caption(&self, ui: &egui::Ui, r: Rect, wb: &Workbench, p: &Palette) {
        let painter = ui.painter();
        let at = r.left_top() + Vec2::new(12.0, 10.0);
        let cap = painter.text(at, Align2::LEFT_TOP, "3D", FontId::proportional(11.0), p.dim);
        painter.text(Pos2::new(cap.right() + 6.0, at.y), Align2::LEFT_TOP, &wb.model_title, FontId::proportional(11.5), p.text);
        painter.text(r.left_bottom() + Vec2::new(12.0, -8.0), Align2::LEFT_BOTTOM, &wb.model_hint, FontId::proportional(11.0), p.dim);
    }

    /// One section of a pane: a header that folds it (title, a tag at the right), then its rows. Inputs draw their values
    /// as boxes.
    fn pane_card(&mut self, ui: &mut egui::Ui, card: &Card, inputs: bool, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
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
                    DockItem::Field { label, value, tone, action, focused } if inputs => self.input_row(ui, label, value, *tone, *action, *focused, p, out),
                    _ => self.dock_item(ui, item, p, out, hint),
                }
            }
        }
        ui.add_space(4.0);
        let y = ui.cursor().top();
        ui.painter().line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, with_alpha(p.line, 160)));
        ui.add_space(4.0);
    }

    /// An input: its label, and its value in a box at the right (the edit box while it is being typed).
    #[allow(clippy::too_many_arguments)]
    fn input_row(&mut self, ui: &mut egui::Ui, label: &str, value: &str, tone: Tone, action: Option<crate::gpu_viewer::controller::UiAction>, focused: bool, p: &Palette, out: &mut Vec<Outcome>) {
        let width = ui.available_width();
        let box_w = (width * 0.46).clamp(96.0, 170.0);
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
        painter.rect(b, 5.0, p.bg, stroke, egui::StrokeKind::Inside);
        let vcol = if editing { p.accent } else if tone == Tone::On { p.good } else { p.text };
        let value_g = painter.layout_no_wrap(value.to_string(), FontId::monospace(12.0), vcol);
        let clip = b.shrink2(Vec2::new(8.0, 0.0));
        painter.with_clip_rect(clip).galley(Pos2::new(clip.left(), mid - value_g.size().y / 2.0), value_g, vcol);
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
