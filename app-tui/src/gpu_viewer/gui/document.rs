//! The page a bench without a 3D model draws where the model would be (`Authoring::document`, used by Search): a
//! search bar card, quick switches, a notice, live progress, figure tiles, a virtualised table and the selected row's
//! detail with highlighted matches. It lives in the free area the panels leave, below the tool pill and left of the
//! inspector, and adapts to that width: the bar's inputs stack on a narrow window. The table and the detail are placed
//! by `page_layout`: a card dragged by its grip to a side of the room goes there and the other makes room, the
//! splitter between them sets their shares, and both are kept between sessions (`PageLayout` in the display settings).
use super::{page_layout, theme::Palette, with_alpha, Gui};
use crate::gpu_viewer::{
    controller::{Command, UiAction},
    display::PageLayout,
    editor::session::Tone,
    menu::{Entry, Outcome},
    ui::{DocBlock, DocDetail, DocHelp, DocInput, DocModal, DocPanel, DocProgress, DocTable, Document},
};
use egui::{text::LayoutJob, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, TextFormat, UiBuilder, Vec2};

/// Height of one table row (two lines: name and folder).
const ROW_H: f32 = 42.0;
/// Below this width the bar's inputs stack.
const BAR_STACK: f32 = 560.0;
/// A grip moved less than this is a click, not a move.
const DRAG_LEAST: f32 = 24.0;

/// What the page remembers between frames (scrolling and which row was selected).
#[derive(Default)]
pub(super) struct DocState {
    /// The selected row last frame: a change scrolls it into view.
    selected: Option<usize>,
    /// Scroll offset and visible height of the table last frame.
    scroll: (f32, f32),
    /// A right click on a row: open the selection's menu here next frame.
    pub(super) pending_menu: Option<Pos2>,
    /// The open popover (a panel's label, or `help:<input id>`), where it was drawn and the control it hangs from.
    pub(super) open: Option<String>,
    pop_rect: Option<Rect>,
    anchor: Option<Rect>,
    /// The stored layout (the view's), set by the caller each frame.
    pub(super) layout: PageLayout,
    /// The layout drawn this frame (the stored one, or the splitter's while it moves).
    shown: PageLayout,
    /// Both cards are shown, so they can be moved.
    movable: bool,
    /// The splitter is moving: the layout it gives, stored when it is let go.
    live: Option<PageLayout>,
    /// A card is being moved by its grip (`true`: the detail), from where the press started.
    dragging: Option<(bool, Pos2)>,
    /// The room the two cards share this frame.
    room: Option<Rect>,
    /// The extra inputs drawn last frame: one that appears empty takes the keyboard.
    extra_seen: Vec<u8>,
}

fn card_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
        .shadow(egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(28) })
}

fn tone_fill(p: &Palette, tone: Tone) -> Color32 {
    match tone {
        Tone::Warn => p.warn,
        Tone::On => p.good,
        Tone::Title => p.accent,
        _ => p.dim,
    }
}

impl Gui {
    /// Draw `doc` in `area` (points). Edits come back as `Outcome::DocField`, clicks as actions.
    pub(super) fn document(&mut self, ui: &mut egui::Ui, area: Rect, doc: &Document, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        if area.width() < 120.0 || area.height() < 80.0 {
            return;
        }
        let inner = area.shrink2(Vec2::new(16.0, 6.0));
        // A press outside the open popover (and outside the control that opened it) closes it, as a menu does.
        if self.doc.open.is_some() {
            let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
            if let Some(at) = pressed {
                let inside = self.doc.pop_rect.is_some_and(|r| r.contains(at)) || self.doc.anchor.is_some_and(|r| r.contains(at));
                if !inside {
                    self.doc.open = None;
                }
            }
        }
        if doc.modal.is_some() {
            self.doc.open = None;
        }
        ui.scope_builder(UiBuilder::new().max_rect(inner).id_salt("document"), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            self.search_bar(ui, doc, p, out, hint);
            if let Some(n) = &doc.notice {
                let col = tone_fill(p, n.tone);
                egui::Frame::new().fill(with_alpha(col, 30)).stroke(Stroke::new(1.0, with_alpha(col, 120))).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&n.text).color(p.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            for e in n.actions.iter().rev() {
                                self.doc_button(ui, e, p, out, hint, false);
                            }
                        });
                    });
                });
            }
            if let Some(pr) = &doc.progress {
                self.progress_card(ui, pr, p, out, hint);
            } else if !doc.stats.is_empty() && ui.available_width() >= BAR_STACK && ui.available_height() >= 520.0 {
                stat_tiles(ui, &doc.stats, p);
            }
            let rest = ui.available_rect_before_wrap();
            if rest.height() < 60.0 {
                return;
            }
            // A narrow room stacks the cards and a short one keeps only the table (widen it or make it taller to read the
            // matches beside the list); both cards then move only while both are shown.
            self.doc.shown = self.doc.live.unwrap_or(self.doc.layout);
            self.doc.room = Some(rest);
            let split = page_layout::split(rest, self.doc.shown, doc.detail.is_some());
            self.doc.movable = split.detail.is_some();
            if !self.doc.movable {
                self.doc.dragging = None;
            }
            // Each card is clipped to its own rectangle: a short window never paints a card over the status bar.
            let table_rect = split.list;
            ui.scope_builder(UiBuilder::new().max_rect(table_rect).id_salt("doc-table"), |ui| {
                ui.set_clip_rect(table_rect.intersect(ui.clip_rect()));
                self.table_card(ui, &doc.table, p, out, hint)
            });
            if let (Some(r), Some(d)) = (split.detail, &doc.detail) {
                ui.scope_builder(UiBuilder::new().max_rect(r).id_salt("doc-detail"), |ui| {
                    // Room for the card's shadow at the sides and the top, none below the rectangle.
                    ui.set_clip_rect(Rect::from_min_max(r.min - Vec2::splat(12.0), Pos2::new(r.right() + 12.0, r.bottom())).intersect(ui.clip_rect()));
                    self.detail_card(ui, d, p, out, hint)
                });
            }
            if let Some(gap) = split.splitter {
                self.splitter(ui, gap, rest, split.stacked, p, out);
            }
            self.drop_preview(ui.ctx(), rest, p);
            ui.advance_cursor_after_rect(rest);
        });
        if let Some(m) = &doc.modal {
            self.modal(ui.ctx(), m, p, out, hint);
        }
    }

    /// The handle a card is moved by: six dots before its title (the title is part of the handle). Dropped near a side
    /// of the room, the card goes there and the other card takes the rest.
    fn grip(&mut self, ui: &mut egui::Ui, handle: Rect, dots: Rect, detail: bool, p: &Palette, out: &mut Vec<Outcome>) {
        let resp = ui.interact(handle, Id::new(("page-grip", detail)), Sense::drag());
        self.targets.push((if detail { "Move the matches" } else { "Move the results" }.to_string(), handle));
        let pointer = ui.input(|i| i.pointer.latest_pos());
        if resp.drag_started() {
            self.doc.dragging = pointer.map(|at| (detail, at));
        }
        let moving = self.doc.dragging.is_some_and(|(d, _)| d == detail);
        if moving {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let col = if moving || resp.hovered() { p.accent } else { p.dim.gamma_multiply(0.8) };
        let c = dots.center();
        for i in 0..3 {
            for j in 0..2 {
                ui.painter().circle_filled(Pos2::new(c.x - 2.5 + 5.0 * j as f32, c.y - 5.0 + 5.0 * i as f32), 1.5, col);
            }
        }
        if resp.drag_stopped() {
            if let (Some((d, from)), Some(room), Some(at)) = (self.doc.dragging.take(), self.doc.room, pointer) {
                let next = page_layout::dropped(room, at, d, self.doc.shown);
                if (at - from).length() >= DRAG_LEAST && next != self.doc.shown {
                    out.push(Outcome::Action(UiAction::Command(Command::SetPageLayout(next))));
                }
            }
        } else if !resp.dragged() && moving {
            // The press ended outside this frame's handle (released over a popover, the window lost the pointer).
            self.doc.dragging = None;
        }
        if !moving {
            resp.on_hover_text("Drag to a side to move this card; the other one makes room");
        }
    }

    /// While a card moves: where it will land, tinted, with a word on what happens.
    fn drop_preview(&mut self, ctx: &egui::Context, room: Rect, p: &Palette) {
        let Some((detail, from)) = self.doc.dragging else { return };
        let Some(at) = ctx.input(|i| i.pointer.latest_pos()) else { return };
        if (at - from).length() < DRAG_LEAST {
            return;
        }
        let next = page_layout::dropped(room, at, detail, self.doc.shown);
        let r = page_layout::preview(room, next, detail).shrink(2.0);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("page-drop")));
        painter.rect(r, CornerRadius::same(12), with_alpha(p.accent, 34), Stroke::new(2.0, with_alpha(p.accent, 200)), egui::StrokeKind::Inside);
        let side = match (next.stacked, next.detail_first == detail) {
            (false, true) => "left",
            (false, false) => "right",
            (true, true) => "top",
            (true, false) => "bottom",
        };
        let text = format!("{} on the {side}", if detail { "Matches" } else { "Results" });
        let g = painter.layout_no_wrap(text, FontId::proportional(14.0), p.on_accent);
        let pill = Rect::from_center_size(r.center(), g.size() + Vec2::new(28.0, 14.0));
        painter.rect_filled(pill, CornerRadius::same(18), p.accent);
        painter.galley(pill.center() - g.size() / 2.0, g, p.on_accent);
        ctx.request_repaint();
    }

    /// The bar between the cards: drag to share the room differently, double-click for the default shares.
    fn splitter(&mut self, ui: &mut egui::Ui, gap: Rect, room: Rect, stacked: bool, p: &Palette, out: &mut Vec<Outcome>) {
        let resp = ui.interact(gap, Id::new("page-splitter"), Sense::click_and_drag());
        self.targets.push(("Resize the cards".to_string(), gap));
        let active = resp.hovered() || resp.dragged();
        if active {
            ui.ctx().set_cursor_icon(if stacked { egui::CursorIcon::ResizeRow } else { egui::CursorIcon::ResizeColumn });
        }
        let shown = self.doc.shown;
        if resp.dragged() {
            if let Some(at) = ui.input(|i| i.pointer.latest_pos()) {
                self.doc.live = Some(PageLayout { split: page_layout::split_at(room, at, stacked), ..shown });
            }
        }
        if resp.drag_stopped() {
            if let Some(l) = self.doc.live.take() {
                out.push(Outcome::Action(UiAction::Command(Command::SetPageLayout(l))));
            }
        } else if !resp.dragged() {
            self.doc.live = None;
        }
        if resp.double_clicked() {
            out.push(Outcome::Action(UiAction::Command(Command::SetPageLayout(PageLayout { split: PageLayout::default().split, ..shown }))));
        }
        let c = gap.center();
        let pill = if stacked { Rect::from_center_size(c, Vec2::new(44.0, 4.0)) } else { Rect::from_center_size(c, Vec2::new(4.0, 44.0)) };
        ui.painter().rect_filled(pill, CornerRadius::same(2), if active { p.accent } else { p.line });
        if !resp.dragged() {
            resp.on_hover_text("Drag to resize; double-click for the default sizes");
        }
    }

    /// A popover hanging under `anchor`: the open panel's cards or an input's guide. Rows act in place and the popover
    /// stays open (several options are often changed together); Esc or a press outside closes it.
    fn popover(&mut self, ctx: &egui::Context, key: &str, anchor: Rect, width: f32, p: &Palette, body: impl FnOnce(&mut Gui, &mut egui::Ui)) {
        let screen = ctx.content_rect();
        let at = Pos2::new(anchor.left().min(screen.right() - width - 12.0).max(screen.left() + 8.0), anchor.bottom() + 6.0);
        let max_h = (screen.bottom() - at.y - 40.0).max(160.0);
        let shown = egui::Area::new(Id::new(("doc-popover", key))).order(egui::Order::Foreground).fixed_pos(at).fade_in(false).show(ctx, |ui| {
            super::panels::popup_frame(p).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(width);
                ui.set_max_height(max_h);
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([true, true]).show(ui, |ui| body(self, ui));
            });
        });
        self.doc.pop_rect = Some(shown.response.rect);
        self.doc.anchor = Some(anchor);
    }

    fn panel_popover(&mut self, ctx: &egui::Context, panel: &DocPanel, anchor: Rect, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        self.popover(ctx, &panel.label, anchor, 360.0, p, |gui, ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for input in &panel.inputs {
                let w = ui.available_width();
                gui.labelled_input(ui, input, w, p, out, hint);
                if let Some(n) = &input.note {
                    ui.add(egui::Label::new(RichText::new(n).small().color(p.dim)).wrap());
                }
                ui.add_space(10.0);
            }
            for (k, card) in panel.cards.iter().filter(|c| !c.items.is_empty()).enumerate() {
                if k > 0 {
                    ui.add_space(10.0);
                }
                ui.label(RichText::new(card.title.to_uppercase()).size(10.5).strong().color(p.accent));
                ui.add_space(2.0);
                for item in &card.items {
                    let before = out.len();
                    gui.dock_item(ui, item, p, out, hint);
                    if out.len() > before {
                        // A typed value opens its card beside the popover.
                        gui.menu_at = ui.input(|i| i.pointer.interact_pos()).map(|q| Pos2::new(anchor.left(), q.y));
                    }
                }
            }
        });
    }

    fn help_popover(&mut self, ctx: &egui::Context, input: &DocInput, help: &DocHelp, anchor: Rect, p: &Palette, out: &mut Vec<Outcome>) {
        let width = (anchor.width() + 120.0).clamp(380.0, 560.0);
        self.popover(ctx, &format!("help:{}", input.id), anchor, width, p, |gui, ui| {
            ui.label(RichText::new(&help.title).strong().size(15.0));
            ui.label(RichText::new(&help.intro).small().color(p.dim));
            for sec in &help.sections {
                ui.add_space(10.0);
                ui.label(RichText::new(sec.title.to_uppercase()).size(10.5).strong().color(p.accent));
                ui.add_space(2.0);
                for row in &sec.rows {
                    let r = ui.horizontal_top(|ui| {
                        let code = egui::Frame::new().fill(with_alpha(p.accent, 26)).corner_radius(CornerRadius::same(5)).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                            ui.set_min_width(150.0);
                            ui.set_max_width(170.0);
                            ui.add(egui::Label::new(RichText::new(&row.code).monospace().size(12.0).color(p.text)).wrap());
                        });
                        ui.add(egui::Label::new(RichText::new(&row.text).size(12.5).color(p.dim)).wrap());
                        code.response.rect
                    });
                    if let Some(example) = &row.example {
                        let full = r.response.rect;
                        let resp = ui.interact(full, Id::new(("help-row", &row.code)), Sense::click());
                        gui.targets.push((format!("Example {}", row.code), full));
                        if resp.hovered() {
                            ui.painter().rect_stroke(full.expand(2.0), CornerRadius::same(6), Stroke::new(1.0, with_alpha(p.accent, 140)), egui::StrokeKind::Outside);
                        }
                        if resp.on_hover_text("Put this in Find").clicked() {
                            out.push(Outcome::DocField { id: input.id, text: example.clone() });
                            gui.doc.open = None;
                        }
                    }
                    ui.add_space(4.0);
                }
            }
        });
    }

    /// A card the page must answer first (the file types), centred over a dimmed page.
    fn modal(&mut self, ctx: &egui::Context, m: &DocModal, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let shown = egui::Modal::new(Id::new(("doc-modal", &m.card.id))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(&m.card.title).strong().size(15.0));
            ui.add_space(6.0);
            let max_h = (ctx.content_rect().height() * 0.6).max(200.0);
            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([true, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for item in &m.card.items {
                    self.dock_item(ui, item, p, out, hint);
                }
            });
        });
        if shown.should_close() {
            out.push(Outcome::Action(m.close));
        }
    }

    fn search_bar(&mut self, ui: &mut egui::Ui, doc: &Document, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            let stack = width < BAR_STACK;
            // Room the buttons beside the inputs take (measured with the fonts they are drawn in).
            let text_w = |t: &str, size: f32| ui.painter().layout_no_wrap(t.to_string(), FontId::proportional(size), p.text).size().x;
            let buttons_w = if stack {
                0.0
            } else {
                doc.buttons.iter().map(|b| text_w(&b.label, 14.0) + 26.0).sum::<f32>()
                    + doc.panels.iter().map(|x| text_w(&x.label, 14.0) + x.badge.as_deref().map_or(0.0, |b| text_w(&format!("  {b}"), 11.0)) + 26.0).sum::<f32>()
                    + 8.0 * (doc.buttons.len() + doc.panels.len()) as f32
                    + 10.0
            };
            let total_grow: f32 = doc.inputs.iter().map(|i| i.grow.max(0.1)).sum();
            let room = (width - buttons_w - 10.0 * doc.inputs.len().saturating_sub(1) as f32).max(160.0);
            let mut notes: Vec<(String, Color32)> = Vec::new();
            let draw_inputs = |gui: &mut Gui, ui: &mut egui::Ui, out: &mut Vec<Outcome>, hint: &mut Option<String>, notes: &mut Vec<(String, Color32)>| {
                for input in &doc.inputs {
                    let w = if stack { width } else { room * input.grow.max(0.1) / total_grow };
                    ui.allocate_ui_with_layout(Vec2::new(w, 54.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(w);
                        gui.labelled_input(ui, input, w, p, out, hint);
                    });
                    if let Some(e) = &input.error {
                        notes.push((format!("{}: {e}", input.label), p.warn));
                    } else if let Some(n) = &input.note {
                        notes.push((n.clone(), p.dim));
                    }
                }
            };
            if stack {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    draw_inputs(self, ui, out, hint, &mut notes);
                    // Exclude sits under Find, above the buttons, as on a wide bar.
                    for (text, col) in notes.drain(..) {
                        ui.add(egui::Label::new(RichText::new(text).small().color(col)).wrap());
                    }
                    self.extra_inputs(ui, doc, width, p, out, hint);
                    ui.horizontal_wrapped(|ui| {
                        for (k, b) in doc.buttons.iter().enumerate() {
                            self.doc_button(ui, b, p, out, hint, k == 0);
                        }
                        self.panel_buttons(ui, &doc.panels, p, out, hint);
                    });
                });
            } else {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    draw_inputs(self, ui, out, hint, &mut notes);
                    ui.vertical(|ui| {
                        ui.add_space(19.0);
                        ui.horizontal(|ui| {
                            for (k, b) in doc.buttons.iter().enumerate() {
                                self.doc_button(ui, b, p, out, hint, k == 0);
                            }
                            self.panel_buttons(ui, &doc.panels, p, out, hint);
                        });
                    });
                });
            }
            for (text, col) in notes {
                ui.add(egui::Label::new(RichText::new(text).small().color(col)).wrap());
            }
            if !stack {
                self.extra_inputs(ui, doc, width, p, out, hint);
            }
            if !doc.chips.is_empty() {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    for c in &doc.chips {
                        self.chip(ui, c, p, out, hint);
                    }
                });
            }
        });
    }

    /// The bar's second row (Exclude), each input with its note.
    fn extra_inputs(&mut self, ui: &mut egui::Ui, doc: &Document, width: f32, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        for input in &doc.extra_inputs {
            ui.add_space(2.0);
            self.labelled_input(ui, input, width, p, out, hint);
            // Just switched on and empty: the keyboard goes to it.
            if !self.doc.extra_seen.contains(&input.id) && input.text.is_empty() {
                ui.ctx().memory_mut(|m| m.request_focus(Id::new(("doc-input", input.id))));
            }
            if let Some(e) = &input.error {
                ui.add(egui::Label::new(RichText::new(format!("{}: {e}", input.label)).small().color(p.warn)).wrap());
            } else if let Some(n) = &input.note {
                ui.add(egui::Label::new(RichText::new(n).small().color(p.dim)).wrap());
            }
        }
        self.doc.extra_seen = doc.extra_inputs.iter().map(|i| i.id).collect();
    }

    /// The bar's popover buttons (Options, Index, Report), each with its state as a small badge.
    fn panel_buttons(&mut self, ui: &mut egui::Ui, panels: &[DocPanel], p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        for panel in panels {
            let open = self.doc.open.as_deref() == Some(panel.label.as_str());
            let mut text = egui::text::LayoutJob::default();
            text.append(&panel.label, 0.0, TextFormat { font_id: FontId::proportional(14.0), color: if open { p.accent } else { p.text }, ..Default::default() });
            if let Some(b) = &panel.badge {
                text.append(&format!("  {b}"), 0.0, TextFormat { font_id: FontId::proportional(11.0), color: p.dim, ..Default::default() });
            }
            let r = ui.add(egui::Button::new(text).corner_radius(CornerRadius::same(8)).min_size(Vec2::new(0.0, 34.0)).selected(open));
            self.targets.push((format!("{} panel", panel.label), r.rect));
            if r.clicked() {
                self.doc.open = if open { None } else { Some(panel.label.clone()) };
            }
            if self.doc.open.as_deref() == Some(panel.label.as_str()) {
                self.panel_popover(ui.ctx(), panel, r.rect, p, out, hint);
            }
        }
    }

    /// A small caps label over a rounded text input whose border lights when it has the keyboard; an input with a guide
    /// has its link on the label's line.
    fn labelled_input(&mut self, ui: &mut egui::Ui, input: &DocInput, width: f32, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let key = format!("help:{}", input.id);
        ui.horizontal(|ui| {
            ui.set_width(width);
            ui.label(RichText::new(input.label.to_uppercase()).size(10.5).strong().color(p.dim));
            input.help.as_ref().map(|h| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = self.doc.open.as_deref() == Some(key.as_str());
                    let r = ui.add(egui::Button::new(RichText::new(format!("? {}", h.link)).size(11.5).color(if open { p.accent } else { p.dim })).frame(false));
                    self.targets.push((h.link.clone(), r.rect));
                    if r.on_hover_text(format!("{}, with examples", h.title)).clicked() {
                        self.doc.open = if open { None } else { Some(key.clone()) };
                    }
                })
            })
        });
        let top = ui.cursor().min;
        let choices_w = segmented_width(ui, &input.choices, p);
        if input.choices.is_empty() {
            self.text_box(ui, input, width, 34.0, p, out, hint);
        } else if width - choices_w - 8.0 >= 220.0 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                self.text_box(ui, input, width - choices_w - 8.0, 34.0, p, out, hint);
                self.segmented(ui, &input.choices, choices_w, p, out, hint);
            });
        } else {
            // Too narrow for both on one line: the choice goes under the input.
            self.text_box(ui, input, width, 34.0, p, out, hint);
            self.segmented(ui, &input.choices, choices_w.min(width), p, out, hint);
        }
        if let (Some(h), true) = (&input.help, self.doc.open.as_deref() == Some(key.as_str())) {
            let anchor = Rect::from_min_size(top, Vec2::new(width, 34.0));
            self.help_popover(ui.ctx(), input, h, anchor, p, out);
        }
    }

    /// One choice of a few, as a segmented control: the checked entry raised and tinted.
    fn segmented(&mut self, ui: &mut egui::Ui, entries: &[Entry], width: f32, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(8), p.raised, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
        let mut x = rect.left() + 3.0;
        for e in entries {
            let w = segment_width(ui, &e.label, p);
            let r = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(w, rect.height() - 6.0));
            let resp = ui.interact(r, Id::new(("doc-segment", &e.label)), Sense::click());
            self.targets.push((e.label.clone(), r));
            let on = e.checked == Some(true);
            if on {
                ui.painter().rect(r, CornerRadius::same(6), p.panel, Stroke::new(1.0, with_alpha(p.accent, 150)), egui::StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), with_alpha(p.panel, 150));
            }
            let col = if on { p.accent } else if resp.hovered() { p.text } else { p.dim };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, &e.label, FontId::proportional(13.0), col);
            let resp = resp.on_hover_text(e.hover_text());
            self.entry_click(&resp, e, out, hint);
            x += w;
        }
    }

    fn text_box(&mut self, ui: &mut egui::Ui, input: &DocInput, width: f32, height: f32, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let bg = ui.painter().add(Shape::Noop);
        let trailing_w = input.trailing.as_ref().map_or(0.0, |e| e.label.len() as f32 * 7.0 + 22.0);
        let edit_rect = Rect::from_min_max(rect.min + Vec2::new(10.0, 0.0), Pos2::new(rect.right() - 8.0 - trailing_w, rect.bottom()));
        let mut text = input.text.clone();
        let id = Id::new(("doc-input", input.id));
        let r = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut text).id(id).frame(egui::Frame::NONE).hint_text(RichText::new(&input.hint).color(p.dim.gamma_multiply(0.85))).font(FontId::proportional(14.0)).vertical_align(Align::Center).desired_width(edit_rect.width()),
        );
        self.targets.push((format!("{} input", input.label), rect));
        let focused = r.has_focus();
        let stroke = if input.error.is_some() { Stroke::new(1.5, p.warn) } else if focused { Stroke::new(1.5, p.accent) } else if r.hovered() { Stroke::new(1.0, p.dim) } else { Stroke::new(1.0, p.line) };
        let mut shapes = vec![Shape::rect_filled(rect, CornerRadius::same(8), p.raised), Shape::rect_stroke(rect, CornerRadius::same(8), stroke, egui::StrokeKind::Inside)];
        if focused {
            shapes.insert(0, Shape::rect_filled(rect.expand(3.0), CornerRadius::same(11), with_alpha(p.accent, 36)));
        }
        ui.painter().set(bg, Shape::Vec(shapes));
        if text != input.text {
            out.push(Outcome::DocField { id: input.id, text });
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(a) = input.submit {
                out.push(Outcome::Action(a));
            }
        }
        if let Some(e) = &input.trailing {
            let br = Rect::from_min_size(Pos2::new(rect.right() - 4.0 - trailing_w, rect.top() + 4.0), Vec2::new(trailing_w, rect.height() - 8.0));
            let resp = ui.put(br, egui::Button::new(RichText::new(&e.label).size(12.5)).corner_radius(CornerRadius::same(6)).fill(p.panel));
            self.targets.push((e.label.clone(), br));
            self.entry_click(&resp, e, out, hint);
        }
    }

    /// A button for an entry; `primary` is filled with the accent colour.
    fn doc_button(&mut self, ui: &mut egui::Ui, e: &Entry, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>, primary: bool) {
        let label = if primary && e.enabled { RichText::new(&e.label).strong().color(p.on_accent) } else if e.enabled { RichText::new(&e.label) } else { RichText::new(&e.label).color(p.dim.gamma_multiply(0.75)) };
        let mut b = egui::Button::new(label).corner_radius(CornerRadius::same(8)).min_size(Vec2::new(0.0, 34.0));
        if primary && e.enabled {
            b = b.fill(p.accent).stroke(Stroke::NONE);
        }
        let r = ui.add(b);
        self.targets.push((e.label.clone(), r.rect));
        self.entry_click(&r, e, out, hint);
    }

    fn entry_click(&mut self, r: &egui::Response, e: &Entry, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        if r.hovered() {
            *hint = Some(e.hover_text());
        }
        if r.clicked() {
            out.push(match (&e.reason, e.enabled) {
                (Some(reason), false) => Outcome::Blocked(reason.clone()),
                _ => Outcome::Action(e.action),
            });
        }
    }

    /// A rounded switch pill: tinted when on.
    fn chip(&mut self, ui: &mut egui::Ui, e: &Entry, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let on = e.checked == Some(true);
        let galley = ui.painter().layout_no_wrap(e.label.clone(), FontId::proportional(12.5), if on { p.accent } else { p.text });
        let size = Vec2::new(galley.size().x + if on { 30.0 } else { 22.0 }, 26.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        self.targets.push((e.label.clone(), rect));
        let painter = ui.painter();
        let (fill, stroke) = if on { (with_alpha(p.accent, 38), Stroke::new(1.0, with_alpha(p.accent, 150))) } else if resp.hovered() { (p.raised, Stroke::new(1.0, p.dim)) } else { (p.panel, Stroke::new(1.0, p.line)) };
        painter.rect(rect, CornerRadius::same(13), fill, stroke, egui::StrokeKind::Inside);
        let mut x = rect.left() + 11.0;
        if on {
            painter.circle_filled(Pos2::new(x + 3.0, rect.center().y), 3.5, p.accent);
            x += 10.0;
        }
        painter.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, p.text);
        let resp = resp.on_hover_text(e.hover_text());
        self.entry_click(&resp, e, out, hint);
    }

    fn progress_card(&mut self, ui: &mut egui::Ui, pr: &DocProgress, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&pr.headline).strong().size(14.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(c) = &pr.cancel {
                        self.doc_button(ui, c, p, out, hint, false);
                    }
                    if let Some(f) = pr.fraction {
                        ui.label(RichText::new(format!("{:.0}%", f * 100.0)).monospace().color(p.accent));
                    }
                });
            });
            let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(bar, CornerRadius::same(3), p.raised);
            match pr.fraction {
                Some(f) => {
                    let fill = Rect::from_min_size(bar.min, Vec2::new(bar.width() * f.clamp(0.0, 1.0), bar.height()));
                    painter.rect_filled(fill, CornerRadius::same(3), p.accent);
                }
                None => {
                    // Not measurable yet: a segment sweeps across.
                    let t = ui.input(|i| i.time) as f32;
                    let seg = bar.width() * 0.25;
                    let x = bar.left() + ((t * 0.6).fract() * (bar.width() + seg)) - seg;
                    let fill = Rect::from_min_max(Pos2::new(x.max(bar.left()), bar.top()), Pos2::new((x + seg).min(bar.right()), bar.bottom()));
                    painter.rect_filled(fill, CornerRadius::same(3), p.accent);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
                }
            }
            if !pr.detail.is_empty() {
                ui.label(RichText::new(&pr.detail).small().color(p.dim));
            }
            for (name, status) in pr.active.iter().take(4) {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(10.0).color(p.accent));
                    ui.add(egui::Label::new(RichText::new(name).monospace().size(11.5)).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(status).small().color(p.dim)));
                });
            }
            if pr.active.len() > 4 {
                ui.label(RichText::new(format!("and {} more", pr.active.len() - 4)).small().color(p.dim));
            }
        });
    }

    fn table_card(&mut self, ui: &mut egui::Ui, t: &DocTable, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        card_frame(p).inner_margin(Margin::same(0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(ui.available_height());
            let full = ui.max_rect();
            // Header: title on the left, the filter on the right.
            ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(full.min + Vec2::new(14.0, 10.0), Vec2::new(full.width() - 28.0, 32.0))), |ui| {
                ui.horizontal_centered(|ui| {
                    if self.doc.movable {
                        let (dots, _) = ui.allocate_exact_size(Vec2::new(12.0, 24.0), Sense::hover());
                        let title = ui.add(egui::Label::new(RichText::new(&t.title).strong().size(14.0)).selectable(false));
                        self.grip(ui, dots.union(title.rect), dots, false, p, out);
                    } else {
                        ui.label(RichText::new(&t.title).strong().size(14.0));
                    }
                    if let Some(f) = &t.filter {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let w = (ui.available_width() * 0.55).clamp(120.0, 260.0);
                            self.text_box(ui, f, w, 28.0, p, out, hint);
                        });
                    }
                });
            });
            let head_top = full.top() + 48.0;
            let cols = column_widths(t, full.width() - 28.0);
            // Column headings.
            let head = Rect::from_min_size(Pos2::new(full.left(), head_top), Vec2::new(full.width(), 26.0));
            ui.painter().line_segment([head.left_top(), head.right_top()], Stroke::new(1.0, p.line));
            ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, p.line));
            let mut x = full.left() + 14.0;
            for (c, w) in t.columns.iter().zip(&cols) {
                let r = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(*w, head.height()));
                let resp = ui.interact(r, Id::new(("doc-col", &c.title)), if c.sort.is_some() { Sense::click() } else { Sense::hover() });
                self.targets.push((format!("{} column", c.title), r));
                let col = if c.sorted.is_some() || resp.hovered() { p.text } else { p.dim };
                let galley = ui.painter().layout_no_wrap(c.title.to_uppercase(), FontId::proportional(10.5), col);
                let gx = if c.right { r.right() - galley.size().x - 4.0 } else { r.left() + 4.0 };
                let mid = r.center().y;
                if let Some(asc) = c.sorted {
                    let ax = if c.right { gx - 9.0 } else { gx + galley.size().x + 9.0 };
                    let a = Pos2::new(ax, mid);
                    let tri = if asc { vec![a + Vec2::new(-3.5, 2.0), a + Vec2::new(3.5, 2.0), a + Vec2::new(0.0, -2.5)] } else { vec![a + Vec2::new(-3.5, -2.0), a + Vec2::new(3.5, -2.0), a + Vec2::new(0.0, 2.5)] };
                    ui.painter().add(Shape::convex_polygon(tri, p.accent, Stroke::NONE));
                }
                ui.painter().galley(Pos2::new(gx, mid - galley.size().y / 2.0), galley, col);
                if let (Some(a), true) = (c.sort, resp.clicked()) {
                    out.push(Outcome::Action(a));
                }
                x += w;
            }
            let body = Rect::from_min_max(Pos2::new(full.left(), head.bottom()), Pos2::new(full.right(), full.bottom() - 2.0));
            if t.rows.is_empty() {
                let painter = ui.painter_at(body);
                let c = body.center() - Vec2::new(0.0, 12.0);
                painter.circle_stroke(c - Vec2::new(0.0, 28.0), 16.0, Stroke::new(2.0, with_alpha(p.accent, 120)));
                painter.line_segment([c - Vec2::new(-11.0, 17.0), c - Vec2::new(-20.0, 8.0)], Stroke::new(2.5, with_alpha(p.accent, 120)));
                painter.text(c + Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, &t.empty_title, FontId::proportional(15.0), p.text);
                let g = painter.layout(t.empty_body.clone(), FontId::proportional(12.5), p.dim, (body.width() - 60.0).clamp(100.0, 420.0));
                painter.galley(Pos2::new(c.x - g.size().x / 2.0, c.y + 18.0), g, p.dim);
                return;
            }
            let selected = t.rows.iter().position(|r| r.selected);
            let mut scroll = egui::ScrollArea::vertical().id_salt("doc-rows").auto_shrink([false, false]);
            if selected != self.doc.selected {
                // A selection moved by the keyboard (or a new run) comes into view.
                if let Some(i) = selected {
                    let (offset, height) = self.doc.scroll;
                    let top = i as f32 * ROW_H;
                    if top < offset {
                        scroll = scroll.vertical_scroll_offset(top);
                    } else if height > 0.0 && top + ROW_H > offset + height {
                        scroll = scroll.vertical_scroll_offset(top + ROW_H - height);
                    }
                }
                self.doc.selected = selected;
            }
            let shown = ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                scroll.show_rows(ui, ROW_H, t.rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for i in range {
                        self.table_row(ui, t, i, &cols, p, out);
                    }
                })
            });
            let so = shown.inner;
            self.doc.scroll = (so.state.offset.y, so.inner_rect.height());
        });
    }

    fn table_row(&mut self, ui: &mut egui::Ui, t: &DocTable, i: usize, cols: &[f32], p: &Palette, out: &mut Vec<Outcome>) {
        let row = &t.rows[i];
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
        if i < 400 {
            self.targets.push((format!("row {}", row.cells.first().map_or("", String::as_str)), rect));
        }
        let painter = ui.painter();
        if row.selected {
            painter.rect_filled(rect.shrink2(Vec2::new(6.0, 1.0)), CornerRadius::same(8), with_alpha(p.accent, 42));
            painter.rect_filled(Rect::from_min_size(rect.min + Vec2::new(6.0, 9.0), Vec2::new(3.0, ROW_H - 18.0)), CornerRadius::same(2), p.accent);
        } else if resp.hovered() {
            painter.rect_filled(rect.shrink2(Vec2::new(6.0, 1.0)), CornerRadius::same(8), p.raised);
        }
        let mut x = rect.left() + 14.0;
        for (k, (cell, w)) in row.cells.iter().zip(cols).enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(*w - 8.0, ROW_H));
            let clip = painter.with_clip_rect(r.intersect(painter.clip_rect()));
            let right = t.columns.get(k).is_some_and(|c| c.right);
            if k == 0 {
                let name = clip.layout_no_wrap(cell.clone(), FontId::proportional(13.5), p.text);
                let name_w = name.size().x;
                clip.galley(Pos2::new(r.left() + 4.0, r.top() + 5.0), name, p.text);
                if let Some(b) = &row.badge {
                    let g = clip.layout_no_wrap(b.clone(), FontId::monospace(10.0), p.accent);
                    let br = Rect::from_min_size(Pos2::new(r.left() + 12.0 + name_w, r.top() + 7.0), g.size() + Vec2::new(10.0, 3.0));
                    if br.right() < r.right() {
                        clip.rect_filled(br, CornerRadius::same(5), with_alpha(p.accent, 28));
                        clip.galley(br.min + Vec2::new(5.0, 1.5), g, p.accent);
                    }
                }
                let sub = clip.layout_no_wrap(row.sub.clone(), FontId::proportional(11.5), p.dim);
                clip.galley(Pos2::new(r.left() + 4.0, r.top() + 23.0), sub, p.dim);
            } else {
                let g = clip.layout_no_wrap(cell.clone(), FontId::monospace(12.0), if row.selected { p.text } else { p.dim });
                let gx = if right { r.right() - g.size().x - 4.0 } else { r.left() + 4.0 };
                clip.galley(Pos2::new(gx, r.center().y - g.size().y / 2.0), g, p.text);
            }
            x += w;
        }
        if resp.clicked() || resp.secondary_clicked() {
            out.push(Outcome::Action(row.select));
        }
        if resp.double_clicked() {
            if let Some(a) = row.open {
                out.push(Outcome::Action(a));
            }
        }
        if resp.secondary_clicked() {
            self.doc.pending_menu = resp.interact_pointer_pos();
        }
    }

    fn detail_card(&mut self, ui: &mut egui::Ui, d: &DocDetail, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let outer = ui.available_rect_before_wrap();
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The frame's margins are outside this height: fill the rectangle, not more.
            ui.set_height((outer.height() - 28.0).max(0.0));
            ui.horizontal(|ui| {
                let (dots, _) = ui.allocate_exact_size(Vec2::new(12.0, 22.0), Sense::hover());
                let title = ui.add(egui::Label::new(RichText::new(&d.title).strong().size(16.0)).truncate().selectable(false));
                self.grip(ui, dots.union(title.rect), dots, true, p, out);
            });
            ui.add(egui::Label::new(RichText::new(&d.subtitle).small().color(p.dim)).wrap());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                for e in &d.actions {
                    let r = ui.add(egui::Button::new(RichText::new(&e.label).size(12.5)).corner_radius(CornerRadius::same(7)));
                    self.targets.push((e.label.clone(), r.rect));
                    let r = r.on_hover_text(e.hover_text());
                    self.entry_click(&r, e, out, hint);
                }
            });
            ui.add(egui::Separator::default().spacing(6.0));
            if d.blocks.is_empty() {
                ui.label(RichText::new(&d.empty).color(p.dim));
                return;
            }
            egui::ScrollArea::vertical().id_salt(("doc-detail", &d.title)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (k, b) in d.blocks.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(6.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, with_alpha(p.line, 160)));
                        ui.add_space(6.0);
                    }
                    hit_block(ui, b, p);
                }
            });
        });
    }
}

fn segment_width(ui: &egui::Ui, label: &str, p: &Palette) -> f32 {
    ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.0), p.text).size().x + 26.0
}
/// The width of a segmented control (0 when there is none).
fn segmented_width(ui: &egui::Ui, entries: &[Entry], p: &Palette) -> f32 {
    if entries.is_empty() {
        0.0
    } else {
        entries.iter().map(|e| segment_width(ui, &e.label, p)).sum::<f32>() + 6.0
    }
}

/// Column widths: fixed ones as given, the flexible one (width 0) takes the rest.
fn column_widths(t: &DocTable, width: f32) -> Vec<f32> {
    let fixed: f32 = t.columns.iter().map(|c| c.width).sum();
    let flex = t.columns.iter().filter(|c| c.width <= 0.0).count().max(1) as f32;
    let rest = ((width - fixed) / flex).max(120.0);
    t.columns.iter().map(|c| if c.width > 0.0 { c.width } else { rest }).collect()
}

fn stat_tiles(ui: &mut egui::Ui, stats: &[(String, String)], p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 8.0);
        for (label, value) in stats {
            egui::Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.set_min_width(110.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(value).size(18.0).strong().color(p.text));
                    ui.label(RichText::new(label.to_uppercase()).size(10.0).color(p.dim));
                });
            });
        }
    });
}

/// One matching line with its neighbours: a line-number gutter, the matched filters as tags, the context lines dim
/// and the matches highlighted.
fn hit_block(ui: &mut egui::Ui, b: &DocBlock, p: &Palette) {
    const GUTTER: f32 = 58.0;
    let width = (ui.available_width() - GUTTER - 6.0).max(80.0);
    let mono = FontId::monospace(12.5);
    ui.horizontal(|ui| {
        ui.add_space(GUTTER - 4.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        for t in &b.tags {
            egui::Frame::new().fill(with_alpha(p.accent, 26)).corner_radius(CornerRadius::same(5)).inner_margin(Margin::symmetric(6, 1)).show(ui, |ui| {
                ui.label(RichText::new(t).size(10.5).color(p.accent));
            });
        }
    });
    ui.add_space(3.0);
    let line = |ui: &mut egui::Ui, number: Option<i32>, job: LayoutJob| {
        let galley = ui.painter().layout_job(job);
        let (r, _) = ui.allocate_exact_size(Vec2::new(GUTTER + 6.0 + width, galley.size().y + 2.0), Sense::hover());
        if let Some(n) = number {
            ui.painter().text(Pos2::new(r.left() + GUTTER - 12.0, r.top() + 1.0), Align2::RIGHT_TOP, n.to_string(), FontId::monospace(11.5), p.dim);
        }
        ui.painter().galley(Pos2::new(r.left() + GUTTER, r.top() + 1.0), galley, p.text);
    };
    let plain = |text: &str, col: Color32| {
        let mut job = LayoutJob::simple(text.to_string(), mono.clone(), col, width);
        job.wrap.max_rows = 3;
        job
    };
    if let Some(t) = &b.before {
        line(ui, Some(b.number - 1), plain(t, p.dim));
    }
    // Highlighted match line.
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: width, max_rows: 6, ..Default::default() }, ..Default::default() };
    let normal = TextFormat { font_id: mono.clone(), color: p.text, ..Default::default() };
    let marked = TextFormat { font_id: mono.clone(), color: p.text, background: with_alpha(p.warn, 90), ..Default::default() };
    let mut pos = 0;
    for &(s, e) in &b.marks {
        if s < pos || e > b.text.len() || !b.text.is_char_boundary(s) || !b.text.is_char_boundary(e) {
            continue;
        }
        if s > pos {
            job.append(&b.text[pos..s], 0.0, normal.clone());
        }
        job.append(&b.text[s..e], 0.0, marked.clone());
        pos = e;
    }
    if pos < b.text.len() {
        job.append(&b.text[pos..], 0.0, normal);
    }
    let highlight = ui.painter().add(Shape::Noop);
    let galley = ui.painter().layout_job(job);
    let (r, _) = ui.allocate_exact_size(Vec2::new(GUTTER + 6.0 + width, galley.size().y + 4.0), Sense::hover());
    ui.painter().set(highlight, Shape::rect_filled(Rect::from_min_max(Pos2::new(r.left() + GUTTER - 6.0, r.top()), r.max), CornerRadius::same(4), with_alpha(p.accent, 16)));
    ui.painter().text(Pos2::new(r.left() + GUTTER - 12.0, r.top() + 2.0), Align2::RIGHT_TOP, b.number.to_string(), FontId::monospace(11.5), p.accent);
    ui.painter().galley(Pos2::new(r.left() + GUTTER, r.top() + 2.0), galley, p.text);
    if let Some(t) = &b.after {
        line(ui, Some(b.number + 1), plain(t, p.dim));
    }
}
