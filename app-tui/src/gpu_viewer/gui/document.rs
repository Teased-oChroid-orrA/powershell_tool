//! The page a bench without a 3D model draws where the model would be (`Authoring::document`, used by Search): a
//! search bar card, quick switches, a notice, live progress, figure tiles, a virtualised table and the selected row's
//! detail with highlighted matches. It lives in the free area the panels leave, below the tool pill and left of the
//! inspector, and adapts to that width: the bar's inputs stack on a narrow window. The table and the detail are placed
//! by `page_layout`: a card dragged by its grip to a side of the room goes there and the other makes room, the
//! splitter between them sets their shares, and both are kept between sessions (`PageLayout` in the display settings).
mod stackup;
use super::{
    page_layout,
    theme::Palette,
    with_alpha, Gui,
};
use crate::gpu_viewer::{
    controller::{Command, UiAction},
    display::PageLayout,
    editor::session::Tone,
    menu::{Entry, Outcome},
    ui::{
        DocBlock, DocDetail, DocGrid, DocHelp, DocInput, DocModal, DocPanel, DocProgress, DocTable,
        DocText, DockItem, Document,
    },
};
use egui::{
    text::LayoutJob, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Pos2, Rect,
    RichText, Sense, Shape, Stroke, TextFormat, UiBuilder, Vec2,
};

/// Height of one table row (two lines: name and folder).
const ROW_H: f32 = 42.0;
/// Height reserved for the page's bottom status strip.
const RIBBON_H: f32 = 32.0;
/// A page window shorter than this drops the header's eyebrow and subtitle so the cards keep the room.
const SHORT_PAGE_H: f32 = 720.0;/// Below this width the bar's inputs stack.
const BAR_STACK: f32 = 560.0;
/// A grip moved less than this is a click, not a move.
const DRAG_LEAST: f32 = 24.0;

/// What the page remembers between frames (scrolling and which row was selected).
#[derive(Default)]
pub(super) struct DocState {
    precision: stackup::PrecisionState,
    /// The selected row last frame: a change scrolls it into view.
    selected: Option<usize>,
    /// The most recent match navigation scroll applied to the preview area.
    last_match_scroll_request: Option<u64>,
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
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(28),
        })
}

fn tone_fill(p: &Palette, tone: Tone) -> Color32 {
    match tone {
        Tone::Warn => p.warn,
        Tone::On => p.good,
        Tone::Title => p.accent,
        _ => p.dim,
    }
}

#[cfg(test)]
mod design_d_tests {
    use super::relevance_fraction;

    #[test]
    fn only_the_relevance_column_draws_a_bar() {
        assert_eq!(relevance_fraction("Relevance", "82%"), Some(0.82));
        assert_eq!(relevance_fraction("Relevance", ""), None);
        assert_eq!(relevance_fraction("Relevance", "250%"), Some(1.0));
        assert_eq!(relevance_fraction("Matches", "82%"), None);
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn right_click_keeps_selected_text_available_for_copy() {
        let mut ctx = egui::Context::default();
        let document = DocText {
            content: "copy this text".into(),
            marks: Vec::new().into(),
        };
        let p = crate::gpu_viewer::gui::theme::palette(true);
        let draw = |ctx: &mut egui::Context, events| {
            let mut response = None;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(selectable_document(ui, &document, "test.txt", &p, None))
                });
            });
            output.textures_delta.clear();
            response.unwrap()
        };
        let first = draw(&mut ctx, Vec::new());
        let selection_id = first.id.with(("copy-selection", "test.txt"));
        let range = egui::text_selection::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(4),
        );
        let mut state = egui::text_edit::TextEditState::load(&ctx, first.id).unwrap();
        state.cursor.set_char_range(Some(range));
        state.store(&ctx, first.id);
        ctx.data_mut(|data| data.insert_temp(selection_id, (range, "copy".to_string())));
        let at = first.rect.center();
        let right_click = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        let _ = draw(
            &mut ctx,
            vec![
                egui::Event::PointerMoved(at),
                right_click(true),
                right_click(false),
            ],
        );
        assert_eq!(
            ctx.data(|data| data
                .get_temp::<(egui::text_selection::CCursorRange, String)>(selection_id)
                .unwrap()
                .1),
            "copy"
        );
        let output = ctx.run_ui(egui::RawInput::default(), |ctx| {
            assert!(copy_document_selection(ctx, selection_id))
        });
        let commands = output.platform_output.commands;
        assert!(commands.contains(&egui::OutputCommand::CopyText("copy".into())));
    }
}

fn rounded_perimeter(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(rect.width().min(rect.height()) * 0.5);
    let corners = [
        (Pos2::new(rect.right() - r, rect.top() + r), -90.0_f32),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 90.0),
        (Pos2::new(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut points = Vec::with_capacity(29);
    for (center, start) in corners {
        for step in 0..=6 {
            let angle = (start + step as f32 * 15.0).to_radians();
            points.push(center + Vec2::new(angle.cos(), angle.sin()) * r);
        }
    }
    points.push(points[0]);
    points
}

impl Gui {
    /// Draw `doc` in `area` (points). Edits come back as `Outcome::DocField`, clicks as actions.
    pub(super) fn document(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        doc: &Document,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        if area.width() < 120.0 || area.height() < 80.0 {
            return;
        }
        if doc.precision.is_some() {
            self.precision_page(ui, area, doc, p, out, hint);
            if let Some(modal) = &doc.modal {
                self.modal(ui.ctx(), modal, p, out);
            }
            return;
        }
        let search_shell = doc.sidebar.is_some();
        if search_shell {
            ui.painter().rect_filled(area, 0.0, p.bg);
        }
        let inner = area.shrink2(if search_shell {
            Vec2::new(26.0, 12.0)
        } else {
            Vec2::new(16.0, 6.0)
        });
        // A press outside the open popover (and outside the control that opened it) closes it, as a menu does.
        if self.doc.open.is_some() {
            let pressed = ui.input(|i| {
                if i.pointer.any_pressed() {
                    i.pointer.interact_pos()
                } else {
                    None
                }
            });
            if let Some(at) = pressed {
                let inside = self.doc.pop_rect.is_some_and(|r| r.contains(at))
                    || self.doc.anchor.is_some_and(|r| r.contains(at));
                if !inside {
                    self.doc.open = None;
                }
            }
        }
        if doc.modal.is_some() {
            self.doc.open = None;
        }
        ui.scope_builder(UiBuilder::new().max_rect(inner).id_salt("document"), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("document-page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                    if let Some(header) = &doc.header {
                        // A short window gives the space to the results: no eyebrow or subtitle, a smaller title.
                        let short = search_shell && area.height() < SHORT_PAGE_H;
                        ui.add_space(if short { 4.0 } else if search_shell { 16.0 } else { 8.0 });
                        let compact = search_shell && ui.available_width() < 680.0;
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                if !short {
                                    ui.label(
                                        RichText::new(&header.eyebrow)
                                            .small()
                                            .strong()
                                            .color(p.accent),
                                    );
                                }
                                ui.label(
                                    RichText::new(&header.title)
                                        .size(if short { 22.0 } else { 27.0 })
                                        .strong()
                                        .color(p.text),
                                );
                                if !short {
                                    ui.label(RichText::new(&header.subtitle).size(12.5).color(p.dim));
                                }
                            });
                            if search_shell && !compact {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    self.sidebar_toggle(ui, p);
                                    self.panel_buttons(ui, &doc.panels, p, out, hint);
                                });
                            }
                        });
                        if compact {
                            ui.horizontal_wrapped(|ui| {
                                self.panel_buttons(ui, &doc.panels, p, out, hint);
                                self.sidebar_toggle(ui, p);
                            });
                        }
                        ui.add_space(if search_shell { 8.0 } else { 4.0 });
                    }
                    self.search_bar(ui, doc, p, out, hint);
                    if let Some(n) = &doc.notice {
                        let col = tone_fill(p, n.tone);
                        egui::Frame::new()
                            .fill(with_alpha(col, 30))
                            .stroke(Stroke::new(1.0, with_alpha(col, 120)))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(12, 8))
                            .show(ui, |ui| {
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
                        self.progress_card(ui, pr, p, search_shell, out, hint);
                    } else if !doc.stats.is_empty()
                        && ui.available_width() >= BAR_STACK
                        && ui.available_height() >= 520.0
                    {
                        stat_tiles(ui, &doc.stats, p);
                    }
                    if let Some(insight) = &doc.insight {
                        self.insight_card(ui, insight, p, out, hint);
                    }
                    if let Some(policy) = &doc.policy {
                        egui::Frame::new()
                            .fill(p.panel)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&policy.title).strong());
                                egui::ScrollArea::both()
                                    .id_salt("policy-summary")
                                    .max_height(90.0)
                                    .show(ui, |ui| {
                                        egui::Grid::new("policy-grid").striped(true).show(
                                            ui,
                                            |ui| {
                                                for c in &policy.columns {
                                                    ui.label(&c.title);
                                                }
                                                ui.end_row();
                                                for row in policy.rows.iter() {
                                                    for (k, cell) in row.cells.iter().enumerate() {
                                                        if k == 0 {
                                                            if ui
                                                                .selectable_label(
                                                                    row.selected,
                                                                    cell,
                                                                )
                                                                .clicked()
                                                            {
                                                                out.push(Outcome::Action(
                                                                    row.select,
                                                                ));
                                                            }
                                                        } else {
                                                            ui.label(cell);
                                                        }
                                                    }
                                                    ui.end_row();
                                                }
                                            },
                                        );
                                    });
                            });
                    }
                    let left = ui.available_rect_before_wrap();
                    let ribbon_room = if doc.ribbon.is_some() && left.height() > 160.0 { RIBBON_H } else { 0.0 };
                    // A short window never squeezes the cards: the page scrolls to them instead.
                    let full = page_layout::room_for_cards(left, ribbon_room);
                    let mut rest = full;
                    rest.max.y -= ribbon_room;
                    if rest.height() < 60.0 {
                        return;
                    }
                    // A narrow room stacks the cards and a short one keeps only the table (widen it or make it taller to read the
                    // matches beside the list); both cards then move only while both are shown.
                    self.doc.shown = self.doc.live.unwrap_or(self.doc.layout);
                    self.doc.room = Some(rest);
                    let split = page_layout::split(rest, self.doc.shown, doc.detail.is_some());
                    self.doc.movable = !search_shell && split.detail.is_some();
                    if !self.doc.movable {
                        self.doc.dragging = None;
                    }
                    // Each card is clipped to its own rectangle: a short window never paints a card over the status bar.
                    let table_rect = split.list;
                    ui.scope_builder(
                        UiBuilder::new().max_rect(table_rect).id_salt("doc-table"),
                        |ui| {
                            ui.set_clip_rect(table_rect.intersect(ui.clip_rect()));
                            self.table_card(ui, &doc.table, p, out, hint)
                        },
                    );
                    if let (Some(r), Some(d)) = (split.detail, &doc.detail) {
                        ui.scope_builder(
                            UiBuilder::new().max_rect(r).id_salt("doc-detail"),
                            |ui| {
                                // Room for the card's shadow at the sides and the top, none below the rectangle.
                                ui.set_clip_rect(
                                    Rect::from_min_max(
                                        r.min - Vec2::splat(12.0),
                                        Pos2::new(r.right() + 12.0, r.bottom()),
                                    )
                                    .intersect(ui.clip_rect()),
                                );
                                self.detail_card(ui, d, p, out, hint)
                            },
                        );
                    }
                    if self.doc.movable {
                        if let Some(gap) = split.splitter {
                            self.splitter(ui, gap, rest, split.stacked, p, out);
                        }
                    }
                    self.drop_preview(ui.ctx(), rest, p);
                    if let (Some(r), true) = (&doc.ribbon, ribbon_room > 0.0) {
                        let strip = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - ribbon_room + 4.0), full.max);
                        ribbon(ui, strip, r, p);
                    }
                    ui.advance_cursor_after_rect(full);
                });
        });
        if let Some(m) = &doc.modal {
            self.modal(ui.ctx(), m, p, out);
        }
    }

    /// The handle a card is moved by: six dots before its title (the title is part of the handle). Dropped near a side
    /// of the room, the card goes there and the other card takes the rest.
    fn grip(
        &mut self,
        ui: &mut egui::Ui,
        handle: Rect,
        dots: Rect,
        detail: bool,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let resp = ui.interact(handle, Id::new(("page-grip", detail)), Sense::drag());
        self.targets.push((
            if detail {
                "Move the matches"
            } else {
                "Move the results"
            }
            .to_string(),
            handle,
        ));
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
        let col = if moving || resp.hovered() {
            p.accent
        } else {
            p.dim.gamma_multiply(0.8)
        };
        let c = dots.center();
        for i in 0..3 {
            for j in 0..2 {
                ui.painter().circle_filled(
                    Pos2::new(c.x - 2.5 + 5.0 * j as f32, c.y - 5.0 + 5.0 * i as f32),
                    1.5,
                    col,
                );
            }
        }
        if resp.drag_stopped() {
            if let (Some((d, from)), Some(room), Some(at)) =
                (self.doc.dragging.take(), self.doc.room, pointer)
            {
                let next = page_layout::dropped(room, at, d, self.doc.shown);
                if (at - from).length() >= DRAG_LEAST && next != self.doc.shown {
                    out.push(Outcome::Action(UiAction::Command(Command::SetPageLayout(
                        next,
                    ))));
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
        let Some((detail, from)) = self.doc.dragging else {
            return;
        };
        let Some(at) = ctx.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        if (at - from).length() < DRAG_LEAST {
            return;
        }
        let next = page_layout::dropped(room, at, detail, self.doc.shown);
        let r = page_layout::preview(room, next, detail).shrink(2.0);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::new("page-drop"),
        ));
        painter.rect(
            r,
            CornerRadius::same(12),
            with_alpha(p.accent, 34),
            Stroke::new(2.0, with_alpha(p.accent, 200)),
            egui::StrokeKind::Inside,
        );
        let side = match (next.stacked, next.detail_first == detail) {
            (false, true) => "left",
            (false, false) => "right",
            (true, true) => "top",
            (true, false) => "bottom",
        };
        let text = format!(
            "{} on the {side}",
            if detail { "Matches" } else { "Results" }
        );
        let g = painter.layout_no_wrap(text, FontId::proportional(14.0), p.on_accent);
        let pill = Rect::from_center_size(r.center(), g.size() + Vec2::new(28.0, 14.0));
        painter.rect_filled(pill, CornerRadius::same(18), p.accent);
        painter.galley(pill.center() - g.size() / 2.0, g, p.on_accent);
        ctx.request_repaint();
    }

    /// The bar between the cards: drag to share the room differently, double-click for the default shares.
    fn splitter(
        &mut self,
        ui: &mut egui::Ui,
        gap: Rect,
        room: Rect,
        stacked: bool,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let resp = ui.interact(gap, Id::new("page-splitter"), Sense::click_and_drag());
        self.targets.push(("Resize the cards".to_string(), gap));
        let active = resp.hovered() || resp.dragged();
        if active {
            ui.ctx().set_cursor_icon(if stacked {
                egui::CursorIcon::ResizeRow
            } else {
                egui::CursorIcon::ResizeColumn
            });
        }
        let shown = self.doc.shown;
        if resp.dragged() {
            if let Some(at) = ui.input(|i| i.pointer.latest_pos()) {
                self.doc.live = Some(PageLayout {
                    split: page_layout::split_at(room, at, stacked),
                    ..shown
                });
            }
        }
        if resp.drag_stopped() {
            if let Some(l) = self.doc.live.take() {
                out.push(Outcome::Action(UiAction::Command(Command::SetPageLayout(
                    l,
                ))));
            }
        } else if !resp.dragged() {
            self.doc.live = None;
        }
        if resp.double_clicked() {
            out.push(Outcome::Action(UiAction::Command(Command::SetPageLayout(
                PageLayout {
                    split: PageLayout::default().split,
                    ..shown
                },
            ))));
        }
        let c = gap.center();
        let pill = if stacked {
            Rect::from_center_size(c, Vec2::new(44.0, 4.0))
        } else {
            Rect::from_center_size(c, Vec2::new(4.0, 44.0))
        };
        ui.painter().rect_filled(
            pill,
            CornerRadius::same(2),
            if active { p.accent } else { p.line },
        );
        if !resp.dragged() {
            resp.on_hover_text("Drag to resize; double-click for the default sizes");
        }
    }

    /// A popover hanging under `anchor`: the open panel's cards or an input's guide. Rows act in place and the popover
    /// stays open (several options are often changed together); Esc or a press outside closes it.
    fn popover(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        anchor: Rect,
        width: f32,
        progress: Option<f32>,
        p: &Palette,
        body: impl FnOnce(&mut Gui, &mut egui::Ui),
    ) {
        let screen = ctx.content_rect();
        let focus = !key.starts_with("help:");
        let at = Pos2::new(
            anchor
                .left()
                .min(screen.right() - width - 12.0)
                .max(screen.left() + 8.0),
            anchor.bottom() + 6.0,
        );
        let max_h = (screen.height() - 48.0).max(120.0);
        let popup_width = width.min(screen.width() - 32.0).max(1.0);
        if focus {
            let scrim = egui::Area::new(Id::new(("doc-popover-scrim", key)))
                .order(egui::Order::Middle)
                .fixed_pos(screen.min)
                .constrain(false)
                .show(ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        with_alpha(p.side, 163),
                    );
                    (response.clicked(), ui.input(|i| i.pointer.interact_pos()))
                });
            if scrim.inner.0 && scrim.inner.1.is_some_and(|pos| !anchor.contains(pos)) {
                self.doc.open = None;
                return;
            }
        }
        let mut area = egui::Area::new(Id::new(("doc-popover", key)))
            .order(egui::Order::Foreground)
            .fade_in(false);
        if focus {
            area = area.pivot(Align2::CENTER_CENTER).fixed_pos(screen.center());
        } else {
            area = area.fixed_pos(at);
        }
        let shown = area.show(ctx, |ui| {
            let frame = super::panels::popup_frame(p)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(popup_width);
                    ui.set_max_height(max_h);
                    egui::ScrollArea::vertical()
                        .max_height(max_h)
                        .auto_shrink([true, true])
                        .show(ui, |ui| body(self, ui));
                });
            if let Some(fraction) = progress {
                super::panels::perimeter_progress(
                    ui.painter(),
                    frame.response.rect.shrink(1.0),
                    12.0,
                    fraction,
                    p.accent,
                    3.0,
                );
            }
        });
        self.doc.pop_rect = Some(shown.response.rect);
        self.doc.anchor = Some(anchor);
    }

    fn panel_popover(
        &mut self,
        ctx: &egui::Context,
        panel: &DocPanel,
        anchor: Rect,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        self.popover(ctx, &panel.label, anchor, 360.0, panel.progress, p, |gui, ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(match panel.label.as_str() { "Options" => "SEARCH PREFERENCES", "Index" => "LOCAL SEARCH INDEX", "Projects" => "SUGGESTIONS", _ => "SEARCH OUTPUT" }).size(9.0).strong().color(p.accent));
            ui.heading(&panel.label);
            ui.add(egui::Label::new(RichText::new(match panel.label.as_str() {
                "Options" => "Choose how Search matches content. Less common controls stay grouped below.",
                "Index" => "Build a local index to speed repeat searches. Results still come from the same search engine.",
                "Projects" => "Folders that look like one piece of work, found from your indexed folders.",
                _ => "Choose where Search writes reports and exports.",
            }).small().color(p.dim)).wrap());
            ui.add_space(8.0);
            for input in &panel.inputs {
                let w = ui.available_width();
                gui.labelled_input(ui, input, w, p, out, hint);
                if let Some(n) = &input.note {
                    ui.add(egui::Label::new(RichText::new(n).small().color(p.dim)).wrap());
                }
                ui.add_space(10.0);
            }
            if !panel.chips.is_empty() {
                ui.label(RichText::new("CONDITIONS · CLICK ONE TO REMOVE IT").size(9.0).strong().color(p.dim));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    for chip in &panel.chips {
                        gui.chip(ui, chip, p, out, hint);
                    }
                });
                ui.add_space(8.0);
            }
            for (k, card) in panel.cards.iter().filter(|c| !c.items.is_empty()).enumerate() {
                egui::CollapsingHeader::new(&card.title).default_open(k == 0).id_salt(("search-panel-card", &panel.label, &card.id)).show(ui, |ui| {
                    egui::Frame::new().fill(with_alpha(p.raised, 125)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        for item in &card.items {
                            let before = out.len();
                            gui.panel_item(ui, item, p, out);
                            if out.len() > before {
                                // A typed value opens its card beside the popover.
                                gui.menu_at = ui.input(|i| i.pointer.interact_pos()).map(|q| Pos2::new(anchor.left(), q.y));
                            }
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(if panel.label == "Projects" { "Nothing changes until you pick one" } else { "Changes apply to your next search" }).small().color(p.dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Done").color(p.on_accent)).fill(p.accent).stroke(Stroke::NONE).corner_radius(CornerRadius::same(7))).clicked() {
                        gui.doc.open = None;
                    }
                });
            });
        });
    }

    /// What Find found besides files: a sentence's conditions and their passages, semantic passages. One
    /// card on the page (not a popover), scrolling inside a share of the room so the results stay in view.
    fn insight_card(
        &mut self,
        ui: &mut egui::Ui,
        insight: &crate::gpu_viewer::ui::DocInsight,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let height = (ui.available_height() * 0.38).clamp(240.0, 420.0);
        egui::Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(insight.title.to_uppercase()).size(9.5).strong().color(p.accent));
                if !insight.chips.is_empty() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                        for chip in &insight.chips {
                            self.chip(ui, chip, p, out, hint);
                        }
                    });
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("find-insight")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for item in &insight.items {
                            self.panel_item(ui, item, p, out);
                        }
                    });
            });
        ui.add_space(8.0);
    }

    fn sidebar_toggle(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let label = if self.search_sidebar_visible {
            "Hide sidebar"
        } else {
            "Show sidebar"
        };
        let response = ui.add(
            egui::Button::new(RichText::new(label).size(12.0).strong().color(p.text))
                .fill(p.raised)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(8))
                .min_size(Vec2::new(0.0, 34.0)),
        );
        self.targets.push((label.into(), response.rect));
        if response.clicked() {
            self.search_sidebar_visible = !self.search_sidebar_visible;
        }
    }

    fn panel_item(
        &mut self,
        ui: &mut egui::Ui,
        item: &DockItem,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let DockItem::Field {
            label,
            value,
            tone,
            action,
            focused,
        } = item
        else {
            self.dock_item(ui, item, p, out);
            return;
        };
        let width = ui.available_width();
        let toggle = value == "On" || value == "Off";
        let label_galley = ui.painter().layout(
            label.clone(),
            FontId::proportional(13.0),
            p.text,
            (width - 82.0).max(24.0),
        );
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(width, label_galley.size().y.max(20.0) + 16.0),
            if action.is_some() {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if *focused || response.hovered() && action.is_some() {
            ui.painter().rect_filled(
                rect,
                7.0,
                if *focused {
                    with_alpha(p.accent, 35)
                } else {
                    p.panel
                },
            );
        }
        self.targets.push((label.clone(), rect));
        ui.painter().galley(
            Pos2::new(
                rect.left() + 9.0,
                rect.center().y - label_galley.size().y / 2.0,
            ),
            label_galley,
            p.text,
        );
        if toggle {
            let on = value == "On";
            let switch = Rect::from_center_size(
                Pos2::new(rect.right() - 20.0, rect.center().y),
                Vec2::new(30.0, 17.0),
            );
            ui.painter().rect_filled(
                switch,
                9.0,
                if on { p.accent } else { with_alpha(p.dim, 100) },
            );
            ui.painter().circle_filled(
                Pos2::new(
                    if on {
                        switch.right() - 8.0
                    } else {
                        switch.left() + 8.0
                    },
                    switch.center().y,
                ),
                5.5,
                p.bg,
            );
        } else {
            let color = if *tone == Tone::On { p.good } else { p.dim };
            ui.painter().text(
                Pos2::new(rect.right() - 9.0, rect.center().y),
                Align2::RIGHT_CENTER,
                value,
                FontId::monospace(11.0),
                color,
            );
        }
        let response = if label == "Index location" {
            response.on_hover_text("Click to switch between Search folder and Output folder")
        } else {
            response
        };
        if response.clicked() {
            if let Some(action) = action {
                out.push(Outcome::Action(*action));
            }
        }
    }

    fn help_popover(
        &mut self,
        ctx: &egui::Context,
        input: &DocInput,
        help: &DocHelp,
        anchor: Rect,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let width = (anchor.width() + 120.0).clamp(380.0, 560.0);
        self.popover(
            ctx,
            &format!("help:{}", input.id),
            anchor,
            width,
            None,
            p,
            |gui, ui| {
                ui.label(RichText::new(&help.title).strong().size(15.0));
                ui.label(RichText::new(&help.intro).small().color(p.dim));
                for sec in &help.sections {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(sec.title.to_uppercase())
                            .size(10.5)
                            .strong()
                            .color(p.accent),
                    );
                    ui.add_space(2.0);
                    for row in &sec.rows {
                        let r = ui.horizontal_top(|ui| {
                            let code = egui::Frame::new()
                                .fill(with_alpha(p.accent, 26))
                                .corner_radius(CornerRadius::same(5))
                                .inner_margin(Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    ui.set_min_width(150.0);
                                    ui.set_max_width(170.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&row.code)
                                                .monospace()
                                                .size(12.0)
                                                .color(p.text),
                                        )
                                        .wrap(),
                                    );
                                });
                            ui.add(
                                egui::Label::new(RichText::new(&row.text).size(12.5).color(p.dim))
                                    .wrap(),
                            );
                            code.response.rect
                        });
                        if let Some(example) = &row.example {
                            let full = r.response.rect;
                            let resp =
                                ui.interact(full, Id::new(("help-row", &row.code)), Sense::click());
                            gui.targets.push((format!("Example {}", row.code), full));
                            if resp.hovered() {
                                ui.painter().rect_stroke(
                                    full.expand(2.0),
                                    CornerRadius::same(6),
                                    Stroke::new(1.0, with_alpha(p.accent, 140)),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if resp.on_hover_text("Put this in Find").clicked() {
                                out.push(Outcome::DocField {
                                    id: input.id,
                                    text: example.clone(),
                                });
                                gui.doc.open = None;
                            }
                        }
                        ui.add_space(4.0);
                    }
                }
            },
        );
    }

    /// A card the page must answer first (the file types), centred over a dimmed page.
    fn modal(
        &mut self,
        ctx: &egui::Context,
        m: &DocModal,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let shown = egui::Modal::new(Id::new(("doc-modal", &m.card.id))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(&m.card.title).strong().size(15.0));
            ui.add_space(6.0);
            let max_h = (ctx.content_rect().height() * 0.6).max(200.0);
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .auto_shrink([true, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for item in &m.card.items {
                        self.dock_item(ui, item, p, out);
                    }
                });
        });
        if shown.should_close() {
            out.push(Outcome::Action(m.close));
        }
    }

    fn search_bar(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            let stack = width < BAR_STACK;
            // Room the buttons beside the inputs take (measured with the fonts they are drawn in).
            let text_w = |t: &str, size: f32| {
                ui.painter()
                    .layout_no_wrap(t.to_string(), FontId::proportional(size), p.text)
                    .size()
                    .x
            };
            let buttons_w = if stack {
                0.0
            } else {
                doc.buttons
                    .iter()
                    .map(|b| text_w(&b.label, 14.0) + 26.0)
                    .sum::<f32>()
                    + if doc.header.is_none() {
                        doc.panels
                            .iter()
                            .map(|x| {
                                text_w(&x.label, 14.0)
                                    + x.badge
                                        .as_deref()
                                        .map_or(0.0, |b| text_w(&format!("  {b}"), 11.0))
                                    + 26.0
                            })
                            .sum::<f32>()
                    } else {
                        0.0
                    }
                    + 8.0
                        * (doc.buttons.len()
                            + if doc.header.is_none() {
                                doc.panels.len()
                            } else {
                                0
                            }) as f32
                    + 10.0
            };
            let total_grow: f32 = doc.inputs.iter().map(|i| i.grow.max(0.1)).sum();
            let room =
                (width - buttons_w - 10.0 * doc.inputs.len().saturating_sub(1) as f32).max(160.0);
            let mut notes: Vec<(String, Color32)> = Vec::new();
            let draw_inputs = |gui: &mut Gui,
                               ui: &mut egui::Ui,
                               out: &mut Vec<Outcome>,
                               hint: &mut Option<String>,
                               notes: &mut Vec<(String, Color32)>| {
                for input in &doc.inputs {
                    let w = if stack {
                        width
                    } else {
                        room * input.grow.max(0.1) / total_grow
                    };
                    ui.allocate_ui_with_layout(
                        Vec2::new(w, 54.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(w);
                            gui.labelled_input(ui, input, w, p, out, hint);
                        },
                    );
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
                        if doc.header.is_none() {
                            self.panel_buttons(ui, &doc.panels, p, out, hint);
                        }
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
                            if doc.header.is_none() {
                                self.panel_buttons(ui, &doc.panels, p, out, hint);
                            }
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
    fn extra_inputs(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        width: f32,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        for input in &doc.extra_inputs {
            ui.add_space(2.0);
            self.labelled_input(ui, input, width, p, out, hint);
            // Just switched on and empty: the keyboard goes to it.
            if !self.doc.extra_seen.contains(&input.id) && input.text.is_empty() {
                ui.ctx()
                    .memory_mut(|m| m.request_focus(Id::new(("doc-input", input.id))));
            }
            if let Some(e) = &input.error {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}: {e}", input.label))
                            .small()
                            .color(p.warn),
                    )
                    .wrap(),
                );
            } else if let Some(n) = &input.note {
                ui.add(egui::Label::new(RichText::new(n).small().color(p.dim)).wrap());
            }
        }
        self.doc.extra_seen = doc.extra_inputs.iter().map(|i| i.id).collect();
    }

    /// The bar's popover buttons (Options, Index, Report), each with its state as a small badge.
    fn panel_buttons(
        &mut self,
        ui: &mut egui::Ui,
        panels: &[DocPanel],
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        for panel in panels {
            let open = self.doc.open.as_deref() == Some(panel.label.as_str());
            let label = ui.painter().layout_no_wrap(
                panel.label.clone(),
                FontId::proportional(14.0),
                if open { p.accent } else { p.text },
            );
            let badge = panel.badge.as_ref().map(|badge| {
                ui.painter()
                    .layout_no_wrap(badge.clone(), FontId::proportional(11.0), p.dim)
            });
            let badge_width = badge.as_ref().map_or(0.0, |g| g.size().x + 7.0);
            let size = Vec2::new(16.0 + 7.0 + label.size().x + badge_width + 20.0, 34.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let fill = if open {
                with_alpha(p.accent, 34)
            } else if response.hovered() {
                p.raised
            } else {
                p.panel
            };
            let stroke = Stroke::new(
                1.0,
                if open {
                    with_alpha(p.accent, 150)
                } else {
                    p.line
                },
            );
            ui.painter().rect(
                rect,
                CornerRadius::same(8),
                fill,
                stroke,
                egui::StrokeKind::Inside,
            );
            if let (Some(fraction), false) = (panel.progress, open) {
                super::panels::perimeter_progress(
                    ui.painter(),
                    rect.shrink(1.0),
                    8.0,
                    fraction,
                    p.accent,
                    2.0,
                );
            }
            let icon = Rect::from_min_size(
                Pos2::new(rect.left() + 9.0, rect.center().y - 8.0),
                Vec2::splat(16.0),
            );
            let glyph = match panel.label.as_str() {
                "Options" => "settings",
                "Index" => "index",
                "Report" => "report",
                _ => "",
            };
            super::panels::search_icon(ui, icon, glyph, if open { p.accent } else { p.dim });
            let mut x = icon.right() + 7.0;
            ui.painter().galley(
                Pos2::new(x, rect.center().y - label.size().y / 2.0),
                label,
                if open { p.accent } else { p.text },
            );
            x += ui
                .painter()
                .layout_no_wrap(panel.label.clone(), FontId::proportional(14.0), p.text)
                .size()
                .x;
            if let Some(badge) = badge {
                ui.painter().galley(
                    Pos2::new(x + 7.0, rect.center().y - badge.size().y / 2.0),
                    badge,
                    p.dim,
                );
            }
            let r = response.on_hover_text(&panel.label);
            self.targets
                .push((format!("{} panel", panel.label), r.rect));
            if r.clicked() {
                self.doc.open = if open {
                    None
                } else {
                    Some(panel.label.clone())
                };
            }
            if self.doc.open.as_deref() == Some(panel.label.as_str()) {
                self.panel_popover(ui.ctx(), panel, r.rect, p, out, hint);
            }
        }
    }

    /// A small caps label over a rounded text input whose border lights when it has the keyboard; an input with a guide
    /// has its link on the label's line.
    fn labelled_input(
        &mut self,
        ui: &mut egui::Ui,
        input: &DocInput,
        width: f32,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let key = format!("help:{}", input.id);
        ui.horizontal(|ui| {
            ui.set_width(width);
            ui.label(
                RichText::new(input.label.to_uppercase())
                    .size(10.5)
                    .strong()
                    .color(p.dim),
            );
            input.help.as_ref().map(|h| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = self.doc.open.as_deref() == Some(key.as_str());
                    let r = ui.add(
                        egui::Button::new(
                            RichText::new(format!("? {}", h.link))
                                .size(11.5)
                                .color(if open { p.accent } else { p.dim }),
                        )
                        .frame(false),
                    );
                    self.targets.push((h.link.clone(), r.rect));
                    if r.on_hover_text(format!("{}, with examples", h.title))
                        .clicked()
                    {
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
    pub(super) fn segmented(&mut self, ui: &mut egui::Ui, entries: &[Entry], width: f32, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
        ui.painter().rect(
            rect,
            CornerRadius::same(8),
            p.raised,
            Stroke::new(1.0, p.line),
            egui::StrokeKind::Inside,
        );
        let mut x = rect.left() + 3.0;
        for e in entries {
            let w = segment_width(ui, &e.label, p);
            let r = Rect::from_min_size(
                Pos2::new(x, rect.top() + 3.0),
                Vec2::new(w, rect.height() - 6.0),
            );
            let resp = ui.interact(r, Id::new(("doc-segment", &e.label)), Sense::click());
            self.targets.push((e.label.clone(), r));
            let on = e.checked == Some(true);
            if on {
                ui.painter().rect(
                    r,
                    CornerRadius::same(6),
                    p.panel,
                    Stroke::new(1.0, with_alpha(p.accent, 150)),
                    egui::StrokeKind::Inside,
                );
            } else if resp.hovered() {
                ui.painter()
                    .rect_filled(r, CornerRadius::same(6), with_alpha(p.panel, 150));
            }
            let col = if on {
                p.accent
            } else if resp.hovered() {
                p.text
            } else {
                p.dim
            };
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                &e.label,
                FontId::proportional(13.0),
                col,
            );
            let resp = resp.on_hover_text(e.hover_text());
            self.entry_click(&resp, e, out, hint);
            x += w;
        }
    }

    fn text_box(
        &mut self,
        ui: &mut egui::Ui,
        input: &DocInput,
        width: f32,
        height: f32,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let bg = ui.painter().add(Shape::Noop);
        let trailing_w = input
            .trailing
            .as_ref()
            .map_or(0.0, |e| e.label.len() as f32 * 7.0 + 22.0);
        let edit_rect = Rect::from_min_max(
            rect.min + Vec2::new(10.0, 0.0),
            Pos2::new(rect.right() - 8.0 - trailing_w, rect.bottom()),
        );
        let mut text = input.text.clone();
        let id = Id::new(("doc-input", input.id));
        let r = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .frame(egui::Frame::NONE)
                .hint_text(RichText::new(&input.hint).color(p.dim.gamma_multiply(0.85)))
                .font(FontId::proportional(14.0))
                .vertical_align(Align::Center)
                .desired_width(edit_rect.width()),
        );
        self.targets.push((format!("{} input", input.label), rect));
        let focused = r.has_focus();
        let stroke = if input.error.is_some() {
            Stroke::new(1.5, p.warn)
        } else if focused {
            Stroke::new(1.5, p.accent)
        } else if r.hovered() {
            Stroke::new(1.0, p.dim)
        } else {
            Stroke::new(1.0, p.line)
        };
        let mut shapes = vec![
            Shape::rect_filled(rect, CornerRadius::same(8), p.raised),
            Shape::rect_stroke(
                rect,
                CornerRadius::same(8),
                stroke,
                egui::StrokeKind::Inside,
            ),
        ];
        if focused {
            shapes.insert(
                0,
                Shape::rect_filled(
                    rect.expand(3.0),
                    CornerRadius::same(11),
                    with_alpha(p.accent, 36),
                ),
            );
        }
        ui.painter().set(bg, Shape::Vec(shapes));
        if text != input.text {
            out.push(Outcome::DocField { id: input.id, text });
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (r.lost_focus() && input.commit_on_blur) || ((r.has_focus() || r.lost_focus()) && enter)
        {
            if let Some(a) = input.submit {
                out.push(Outcome::Action(a));
            }
        }
        if let Some(e) = &input.trailing {
            let br = Rect::from_min_size(
                Pos2::new(rect.right() - 4.0 - trailing_w, rect.top() + 4.0),
                Vec2::new(trailing_w, rect.height() - 8.0),
            );
            let resp = ui.put(
                br,
                egui::Button::new(RichText::new(&e.label).size(12.5))
                    .corner_radius(CornerRadius::same(6))
                    .fill(p.panel),
            );
            self.targets.push((e.label.clone(), br));
            self.entry_click(&resp, e, out, hint);
        }
    }

    /// A button for an entry; `primary` is filled with the accent colour.
    pub(super) fn doc_button(&mut self, ui: &mut egui::Ui, e: &Entry, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>, primary: bool) {
        let label = if primary && e.enabled { RichText::new(&e.label).strong().color(p.on_accent) } else if e.enabled { RichText::new(&e.label) } else { RichText::new(&e.label).color(p.dim.gamma_multiply(0.75)) };
        let mut b = egui::Button::new(label).corner_radius(CornerRadius::same(8)).min_size(Vec2::new(0.0, 34.0));
        if primary && e.enabled {
            b = b.fill(p.accent).stroke(Stroke::NONE);
        }
        let r = ui.add(b);
        self.targets.push((e.label.clone(), r.rect));
        self.entry_click(&r, e, out, hint);
    }

    pub(super) fn entry_click(&mut self, r: &egui::Response, e: &Entry, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
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

    /// Scores behind a result as small labelled meters, the headline one first; a tooltip explains each.
    fn signal_meters(ui: &mut egui::Ui, signals: &[crate::gpu_viewer::ui::DocSignal], p: &Palette) {
        if signals.is_empty() {
            return;
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(14.0, 6.0);
            for signal in signals {
                let fraction = signal.fraction.clamp(0.0, 1.0);
                let value = format!("{:.0}%", fraction * 100.0);
                let galley = ui.painter().layout_no_wrap(
                    signal.label.clone(),
                    FontId::proportional(12.0),
                    p.dim,
                );
                let value_galley = ui.painter().layout_no_wrap(
                    value.clone(),
                    FontId::proportional(12.0),
                    p.text,
                );
                let bar = if signal.primary { 72.0 } else { 44.0 };
                let width = galley.size().x + bar + value_galley.size().x + 16.0;
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
                let painter = ui.painter();
                let mid = rect.center().y;
                painter.text(
                    Pos2::new(rect.left(), mid),
                    Align2::LEFT_CENTER,
                    &signal.label,
                    FontId::proportional(12.0),
                    if signal.primary { p.text } else { p.dim },
                );
                let track = Rect::from_min_size(
                    Pos2::new(rect.left() + galley.size().x + 8.0, mid - 3.0),
                    Vec2::new(bar, 6.0),
                );
                painter.rect_filled(track, CornerRadius::same(3), p.line);
                let fill = Rect::from_min_size(track.min, Vec2::new(bar * fraction, 6.0));
                painter.rect_filled(
                    fill,
                    CornerRadius::same(3),
                    if signal.primary { p.accent } else { with_alpha(p.accent, 150) },
                );
                painter.text(
                    Pos2::new(track.right() + 8.0, mid),
                    Align2::LEFT_CENTER,
                    value,
                    FontId::proportional(12.0),
                    p.text,
                );
                response.on_hover_text(&signal.hint);
            }
        });
    }

    /// The headline score as a chip; a click opens "Why this ranked" with every signal. Only one signal: no popover.
    fn why_ranked(&mut self, ui: &mut egui::Ui, signals: &[crate::gpu_viewer::ui::DocSignal], p: &Palette) {
        let Some(head) = signals.iter().find(|s| s.primary).or_else(|| signals.first()) else {
            return;
        };
        let more = signals.len() > 1;
        let open_id = Id::new("why-ranked-open");
        let mut open = more && ui.ctx().data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
        let label = format!("{:.0}% relevant{}", head.fraction.clamp(0.0, 1.0) * 100.0, if more { " \u{25BE}" } else { "" });
        let galley = ui.painter().layout_no_wrap(label, FontId::proportional(12.0), p.accent);
        ui.add_space(4.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 22.0, 24.0), if more { Sense::click() } else { Sense::hover() });
        ui.painter().rect(
            rect,
            CornerRadius::same(12),
            with_alpha(p.accent, if open || response.hovered() { 40 } else { 22 }),
            Stroke::new(1.0, with_alpha(p.accent, 110)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(Pos2::new(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley, p.accent);
        let response = response.on_hover_text(&head.hint);
        self.targets.push(("Why this ranked".into(), rect));
        if more && response.clicked() {
            open = !open;
        }
        if open {
            let area = egui::Area::new(Id::new("why-ranked"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.left_bottom() + Vec2::new(0.0, 6.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(14)).show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.label(RichText::new("Why this ranked").strong());
                        Self::signal_meters(ui, signals, p);
                        ui.add_space(6.0);
                        ui.label(RichText::new("Scores compare files; they are not accuracy.").small().color(p.dim));
                    });
                });
            let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|at| !area.response.rect.contains(at) && !rect.contains(at)));
            if outside {
                open = false;
            }
        }
        ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
    }

    /// Short facts about a result as quiet pills.
    fn note_pills(ui: &mut egui::Ui, notes: &[crate::gpu_viewer::ui::DocNote], p: &Palette) {
        if notes.is_empty() {
            return;
        }
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for note in notes {
                let galley = ui.painter().layout_no_wrap(
                    note.text.clone(),
                    FontId::proportional(12.0),
                    p.dim,
                );
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(galley.size().x + 18.0, 22.0),
                    Sense::hover(),
                );
                if !note.hint.is_empty() {
                    response.on_hover_text(&note.hint);
                }
                ui.painter().rect(
                    rect,
                    CornerRadius::same(11),
                    p.panel,
                    Stroke::new(1.0, p.line),
                    egui::StrokeKind::Inside,
                );
                ui.painter().galley(
                    Pos2::new(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    p.dim,
                );
            }
        });
    }

    /// A rounded switch pill: tinted when on.
    pub(super) fn chip(&mut self, ui: &mut egui::Ui, e: &Entry, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let on = e.checked == Some(true);
        let galley = ui.painter().layout_no_wrap(
            e.label.clone(),
            FontId::proportional(12.5),
            if on { p.accent } else { p.text },
        );
        let size = Vec2::new(galley.size().x + if on { 30.0 } else { 22.0 }, 26.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        self.targets.push((e.label.clone(), rect));
        let painter = ui.painter();
        let (fill, stroke) = if on {
            (
                with_alpha(p.accent, 38),
                Stroke::new(1.0, with_alpha(p.accent, 150)),
            )
        } else if resp.hovered() {
            (p.raised, Stroke::new(1.0, p.dim))
        } else {
            (p.panel, Stroke::new(1.0, p.line))
        };
        painter.rect(
            rect,
            CornerRadius::same(13),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        let mut x = rect.left() + 11.0;
        if on {
            painter.circle_filled(Pos2::new(x + 3.0, rect.center().y), 3.5, p.accent);
            x += 10.0;
        }
        painter.galley(
            Pos2::new(x, rect.center().y - galley.size().y / 2.0),
            galley,
            p.text,
        );
        let resp = resp.on_hover_text(e.hover_text());
        self.entry_click(&resp, e, out, hint);
    }

    fn progress_card(
        &mut self,
        ui: &mut egui::Ui,
        pr: &DocProgress,
        p: &Palette,
        search_shell: bool,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let running = pr.cancel.is_some();
        let fill = with_alpha(p.panel, 238);
        let line = p.line;
        let text = p.text;
        let dim = p.dim;
        let accent = p.accent;
        let response = egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, line))
            .corner_radius(CornerRadius::same(if search_shell { 10 } else { 12 }))
            .inner_margin(Margin::symmetric(15, 10))
            .shadow(egui::Shadow {
                offset: [0, 2],
                blur: 10,
                spread: 0,
                color: Color32::from_black_alpha(24),
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if search_shell && ui.available_width() < 680.0 {
                    ui.horizontal(|ui| {
                        let badge = if running { "◌" } else { "✓" };
                        ui.label(RichText::new(badge).strong().color(if running {
                            accent
                        } else {
                            p.good
                        }));
                        ui.label(RichText::new(&pr.headline).strong().size(13.5).color(text));
                        if let Some(c) = &pr.cancel {
                            self.doc_button(ui, c, p, out, hint, false);
                        }
                    });
                    if !pr.detail.is_empty() {
                        ui.label(RichText::new(&pr.detail).small().color(dim));
                    }
                    ui.horizontal_wrapped(|ui| {
                        for (label, value) in &pr.metrics {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(value).strong().size(13.0).color(text));
                                ui.label(RichText::new(label.to_lowercase()).size(9.0).color(dim));
                            });
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                if search_shell {
                                    let badge = if running { "◌" } else { "✓" };
                                    ui.label(RichText::new(badge).strong().color(if running {
                                        accent
                                    } else {
                                        p.good
                                    }));
                                }
                                ui.label(
                                    RichText::new(&pr.headline).strong().size(13.5).color(text),
                                );
                                if let Some(c) = &pr.cancel {
                                    self.doc_button(ui, c, p, out, hint, false);
                                }
                                if !search_shell {
                                    if let Some(f) = pr.fraction {
                                        ui.label(
                                            RichText::new(format!("{:.0}%", f * 100.0))
                                                .monospace()
                                                .color(accent),
                                        );
                                    }
                                }
                            });
                            if !pr.detail.is_empty() {
                                ui.label(RichText::new(&pr.detail).small().color(dim));
                            }
                        });
                        if !pr.metrics.is_empty() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for (label, value) in pr.metrics.iter().rev() {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(value).strong().size(14.0).color(text),
                                        );
                                        ui.label(
                                            RichText::new(label.to_lowercase())
                                                .size(9.5)
                                                .color(dim),
                                        );
                                    });
                                    if search_shell {
                                        ui.add_space(10.0);
                                    } else {
                                        ui.add_space(18.0);
                                    }
                                }
                            });
                        }
                    });
                }
                for (name, status) in pr.active.iter().take(4) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(10.0).color(accent));
                        ui.add(
                            egui::Label::new(
                                RichText::new(name).monospace().size(11.5).color(text),
                            )
                            .truncate(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(status).small().color(dim))
                        });
                    });
                }
                if pr.active.len() > 4 {
                    ui.label(
                        RichText::new(format!("and {} more", pr.active.len() - 4))
                            .small()
                            .color(dim),
                    );
                }
            });
        let dock = response.response.rect.shrink(2.0);
        let points = rounded_perimeter(dock, 11.0);
        let painter = ui.painter();
        let track = with_alpha(p.raised, 210);
        let lengths: Vec<f32> = points
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .collect();
        let perimeter: f32 = lengths.iter().sum();
        let perimeter_progress = !search_shell || running;
        if perimeter_progress {
            painter.add(Shape::line(points.clone(), Stroke::new(2.0, track)));
        }
        let (start, length) = if !perimeter_progress {
            (0.0, 0.0)
        } else {
            match pr.fraction {
                Some(f) => (0.0, perimeter * f.clamp(0.0, 1.0)),
                None => {
                    let t = ui.input(|i| i.time) as f32;
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(33));
                    (perimeter * (t * 0.18).fract(), perimeter * 0.18)
                }
            }
        };
        if length > 0.0 {
            let mut left = length;
            let mut skip = start;
            let mut segment = Vec::new();
            for (i, edge_len) in lengths.iter().enumerate().cycle() {
                if skip >= *edge_len {
                    skip -= edge_len;
                    continue;
                }
                let from = points[i] + (points[i + 1] - points[i]) * (skip / edge_len);
                let take = left.min(edge_len - skip);
                let to = points[i] + (points[i + 1] - points[i]) * ((skip + take) / edge_len);
                segment.extend([from, to]);
                left -= take;
                if left <= 0.0 {
                    break;
                }
                skip = 0.0;
            }
            painter.add(Shape::line(segment, Stroke::new(2.5, accent)));
        }
    }

    fn table_card(
        &mut self,
        ui: &mut egui::Ui,
        t: &DocTable,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        card_frame(p).inner_margin(Margin::same(0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(ui.available_height());
            let full = ui.max_rect();
            // Header: title on the left, the filter on the right.
            ui.scope_builder(
                UiBuilder::new().max_rect(Rect::from_min_size(
                    full.min + Vec2::new(14.0, 10.0),
                    Vec2::new(full.width() - 28.0, 32.0),
                )),
                |ui| {
                    ui.horizontal_centered(|ui| {
                        if self.doc.movable {
                            let (dots, _) =
                                ui.allocate_exact_size(Vec2::new(12.0, 24.0), Sense::hover());
                            let title = ui.add(
                                egui::Label::new(RichText::new(&t.title).strong().size(14.0))
                                    .selectable(false),
                            );
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
                },
            );
            let desired: f32 = t.columns.iter().map(|c| c.width).sum();
            if desired > full.width() && t.columns.iter().all(|c| c.width > 0.0) {
                let area = Rect::from_min_max(full.min + Vec2::new(0.0, 48.0), full.max);
                ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
                    egui::ScrollArea::both()
                        .id_salt("wide-doc-table")
                        .auto_shrink([false, false])
                        .show_rows(ui, ROW_H, t.rows.len() + 1, |ui, range| {
                            ui.set_min_width(desired + 28.0);
                            let widths: Vec<_> = t.columns.iter().map(|c| c.width).collect();
                            if range.contains(&0) {
                                let (head, _) = ui.allocate_exact_size(
                                    Vec2::new(desired + 28.0, 32.0),
                                    Sense::hover(),
                                );
                                let mut x = head.left() + 14.0;
                                for c in &t.columns {
                                    ui.painter().text(
                                        Pos2::new(x, head.center().y),
                                        Align2::LEFT_CENTER,
                                        &c.title,
                                        FontId::proportional(11.0),
                                        p.dim,
                                    );
                                    x += c.width;
                                }
                            }
                            for row in range {
                                if row > 0 {
                                    self.table_row(ui, t, row - 1, &widths, p, out);
                                }
                            }
                        });
                });
                return;
            }
            let head_top = full.top() + 48.0;
            let cols = column_widths(t, full.width() - 28.0);
            // Column headings.
            let head = Rect::from_min_size(
                Pos2::new(full.left(), head_top),
                Vec2::new(full.width(), 26.0),
            );
            ui.painter().line_segment(
                [head.left_top(), head.right_top()],
                Stroke::new(1.0, p.line),
            );
            ui.painter().line_segment(
                [head.left_bottom(), head.right_bottom()],
                Stroke::new(1.0, p.line),
            );
            let mut x = full.left() + 14.0;
            for (c, w) in t.columns.iter().zip(&cols) {
                let r = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(*w, head.height()));
                let resp = ui.interact(
                    r,
                    Id::new(("doc-col", &c.title)),
                    if c.sort.is_some() {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                self.targets.push((format!("{} column", c.title), r));
                let col = if c.sorted.is_some() || resp.hovered() {
                    p.text
                } else {
                    p.dim
                };
                let galley = ui.painter().layout_no_wrap(
                    c.title.to_uppercase(),
                    FontId::proportional(10.5),
                    col,
                );
                let gx = if c.right {
                    r.right() - galley.size().x - 4.0
                } else {
                    r.left() + 4.0
                };
                let mid = r.center().y;
                if let Some(asc) = c.sorted {
                    let ax = if c.right {
                        gx - 9.0
                    } else {
                        gx + galley.size().x + 9.0
                    };
                    let a = Pos2::new(ax, mid);
                    let tri = if asc {
                        vec![
                            a + Vec2::new(-3.5, 2.0),
                            a + Vec2::new(3.5, 2.0),
                            a + Vec2::new(0.0, -2.5),
                        ]
                    } else {
                        vec![
                            a + Vec2::new(-3.5, -2.0),
                            a + Vec2::new(3.5, -2.0),
                            a + Vec2::new(0.0, 2.5),
                        ]
                    };
                    ui.painter()
                        .add(Shape::convex_polygon(tri, p.accent, Stroke::NONE));
                }
                ui.painter()
                    .galley(Pos2::new(gx, mid - galley.size().y / 2.0), galley, col);
                if let (Some(a), true) = (c.sort, resp.clicked()) {
                    out.push(Outcome::Action(a));
                }
                x += w;
            }
            let body = Rect::from_min_max(
                Pos2::new(full.left(), head.bottom()),
                Pos2::new(full.right(), full.bottom() - 2.0),
            );
            if t.rows.is_empty() {
                let painter = ui.painter_at(body);
                let c = body.center() - Vec2::new(0.0, 12.0);
                painter.circle_stroke(
                    c - Vec2::new(0.0, 28.0),
                    16.0,
                    Stroke::new(2.0, with_alpha(p.accent, 120)),
                );
                painter.line_segment(
                    [c - Vec2::new(-11.0, 17.0), c - Vec2::new(-20.0, 8.0)],
                    Stroke::new(2.5, with_alpha(p.accent, 120)),
                );
                painter.text(
                    c + Vec2::new(0.0, 6.0),
                    Align2::CENTER_CENTER,
                    &t.empty_title,
                    FontId::proportional(15.0),
                    p.text,
                );
                let g = painter.layout(
                    t.empty_body.clone(),
                    FontId::proportional(12.5),
                    p.dim,
                    (body.width() - 60.0).clamp(100.0, 420.0),
                );
                painter.galley(Pos2::new(c.x - g.size().x / 2.0, c.y + 18.0), g, p.dim);
                return;
            }
            let selected = t.rows.iter().position(|r| r.selected);
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("doc-rows")
                .auto_shrink([false, false]);
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
                // `show_rows` places row i at i * (ROW_H + item spacing): the rows are packed, so the spacing is zero before
                // it is called (zeroed only inside, the rows drift from where egui thinks they are, ten rows by the
                // fiftieth, and a click or a scroll-to-selected lands on the wrong row).
                ui.spacing_mut().item_spacing.y = 0.0;
                scroll.show_rows(ui, ROW_H, t.rows.len(), |ui, range| {
                    for i in range {
                        self.table_row(ui, t, i, &cols, p, out);
                    }
                })
            });
            let so = shown.inner;
            self.doc.scroll = (so.state.offset.y, so.inner_rect.height());
        });
    }

    fn table_row(
        &mut self,
        ui: &mut egui::Ui,
        t: &DocTable,
        i: usize,
        cols: &[f32],
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let row = &t.rows[i];
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
        if i < 400 {
            self.targets.push((
                format!("row {}", row.cells.first().map_or("", String::as_str)),
                rect,
            ));
        }
        let painter = ui.painter();
        if row.selected {
            painter.rect_filled(
                rect.shrink2(Vec2::new(6.0, 1.0)),
                CornerRadius::same(8),
                with_alpha(p.accent, 42),
            );
            painter.rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(6.0, 9.0), Vec2::new(3.0, ROW_H - 18.0)),
                CornerRadius::same(2),
                p.accent,
            );
        } else if resp.hovered() {
            painter.rect_filled(
                rect.shrink2(Vec2::new(6.0, 1.0)),
                CornerRadius::same(8),
                p.raised,
            );
        }
        let mut x = rect.left() + 14.0;
        for (k, (cell, w)) in row.cells.iter().zip(cols).enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(*w - 8.0, ROW_H));
            let clip = painter.with_clip_rect(r.intersect(painter.clip_rect()));
            ui.interact(r, Id::new(("doc-cell", &t.title, i, k)), Sense::hover())
                .on_hover_text(format!("{}: {}", t.columns[k].title, cell));
            let right = t.columns.get(k).is_some_and(|c| c.right);
            if k == 0 {
                // The badge leads the name: a long name is cut at the cell's edge, the badge must still show.
                let mut name_x = r.left() + 4.0;
                if let Some(b) = &row.badge {
                    let g = clip.layout_no_wrap(b.clone(), FontId::monospace(10.0), p.accent);
                    let br = Rect::from_min_size(
                        Pos2::new(name_x, r.top() + 7.0),
                        g.size() + Vec2::new(10.0, 3.0),
                    );
                    clip.rect_filled(br, CornerRadius::same(5), with_alpha(p.accent, 28));
                    clip.galley(br.min + Vec2::new(5.0, 1.5), g, p.accent);
                    name_x = br.right() + 6.0;
                }
                let name = clip.layout_no_wrap(cell.clone(), FontId::proportional(13.5), p.text);
                clip.galley(Pos2::new(name_x, r.top() + 5.0), name, p.text);
                let sub = clip.layout_no_wrap(row.sub.clone(), FontId::proportional(11.5), p.dim);
                clip.galley(Pos2::new(r.left() + 4.0, r.top() + 23.0), sub, p.dim);
            } else if let Some(fraction) = relevance_fraction(&t.columns[k].title, cell) {
                // A relevance score is a short bar and its number, so a column of them reads at a glance.
                let bar = Rect::from_min_size(
                    Pos2::new(r.left() + 6.0, r.center().y - 3.0),
                    Vec2::new((r.width() - 48.0).max(16.0), 6.0),
                );
                clip.rect_filled(bar, CornerRadius::same(3), p.line);
                clip.rect_filled(
                    Rect::from_min_size(bar.min, Vec2::new(bar.width() * fraction, 6.0)),
                    CornerRadius::same(3),
                    if row.selected { p.accent } else { with_alpha(p.accent, 170) },
                );
                clip.text(
                    Pos2::new(r.right() - 4.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    cell,
                    FontId::monospace(12.0),
                    if row.selected { p.text } else { p.dim },
                );
            } else {
                let g = clip.layout_no_wrap(
                    cell.clone(),
                    FontId::monospace(12.0),
                    if row.selected { p.text } else { p.dim },
                );
                let gx = if right {
                    r.right() - g.size().x - 4.0
                } else {
                    r.left() + 4.0
                };
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

    fn section_drawing(
        &mut self,
        ui: &mut egui::Ui,
        d: &crate::gpu_viewer::ui::DocDrawing,
        p: &Palette,
        out: &mut Vec<Outcome>,
        height: f32,
    ) {
        let key = Id::new("section-zoom");
        let mut zoom = ui
            .ctx()
            .data_mut(|data| data.get_temp::<f64>(key).unwrap_or(1.0));
        ui.horizontal(|ui| {
            if ui.button("Fit section").clicked() {
                zoom = 1.0;
            }
            ui.label("Wheel to zoom; click a member");
        });

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
        if response.hovered() {
            zoom = (zoom * (ui.input(|i| i.smooth_scroll_delta.y) as f64 * 0.004).exp())
                .clamp(0.2, 30.0);
        }
        ui.ctx().data_mut(|data| data.insert_temp(key, zoom));
        let dx = (d.bounds[1][0] - d.bounds[0][0]).max(1e-9);
        let dy = (d.bounds[1][1] - d.bounds[0][1]).max(1e-9);
        let scale =
            ((rect.width() - 40.0) as f64 / dx).min((rect.height() - 40.0) as f64 / dy) * zoom;
        let middle = [
            (d.bounds[0][0] + d.bounds[1][0]) / 2.0,
            (d.bounds[0][1] + d.bounds[1][1]) / 2.0,
        ];
        let transform = |pt: [f64; 2]| {
            Pos2::new(
                rect.center().x + ((pt[0] - middle[0]) * scale) as f32,
                rect.center().y + ((pt[1] - middle[1]) * scale) as f32,
            )
        };
        let painter = ui.painter_at(rect);
        for path in &d.paths {
            let points: Vec<_> = path.points.iter().map(|pt| transform(*pt)).collect();
            let color = match path.kind {
                1 => p.warn,
                2 => p.accent,
                3 => p.good,
                _ => p.dim,
            };
            if points.len() > 2 && path.kind == 0 {
                painter.add(Shape::convex_polygon(
                    points.clone(),
                    with_alpha(color, if path.selected { 70 } else { 20 }),
                    Stroke::new(1.0, color),
                ));
            }
            painter.add(Shape::line(
                points.clone(),
                Stroke::new(if path.selected { 2.5 } else { 1.2 }, color),
            ));
            if let Some(first) = points.first() {
                let bounds = Rect::from_points(&points).expand(4.0);
                if let Some(at) = response.hover_pos() {
                    if bounds.contains(at) {
                        response.clone().on_hover_text(&path.label);
                        if response.clicked() {
                            if let Some(a) = path.select {
                                out.push(Outcome::Action(a));
                            }
                        }
                    }
                }
                if path.kind == 0 && path.selected {
                    painter.text(
                        *first - Vec2::new(6.0, 0.0),
                        Align2::RIGHT_TOP,
                        "Selected",
                        FontId::proportional(11.0),
                        color,
                    );
                }
            }
        }
    }

    fn detail_card(
        &mut self,
        ui: &mut egui::Ui,
        d: &DocDetail,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let outer = ui.available_rect_before_wrap();
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The frame's margins are outside this height: fill the rectangle, not more.
            ui.set_height((outer.height() - 28.0).max(0.0));
            ui.horizontal(|ui| {
                let navigation_width = d.match_navigation.as_ref().map_or(0.0, |_| 132.0);
                let title_width = (ui.available_width() - navigation_width).max(40.0);
                if self.doc.movable {
                    let (dots, _) = ui.allocate_exact_size(Vec2::new(12.0, 22.0), Sense::hover());
                    let title = ui.add(
                        egui::Label::new(RichText::new(&d.title).strong().size(16.0))
                            .truncate()
                            .selectable(false),
                    );
                    self.grip(ui, dots.union(title.rect), dots, true, p, out);
                } else {
                    ui.add_sized(
                        Vec2::new(title_width, 22.0),
                        egui::Label::new(RichText::new(&d.title).strong().size(16.0))
                            .truncate()
                            .selectable(false),
                    );
                }
                if let Some(nav) = &d.match_navigation {
                    ui.add_space(6.0);
                    let previous = ui.add_sized(
                        Vec2::new(26.0, 24.0),
                        egui::Button::new("‹").corner_radius(CornerRadius::same(6)),
                    );
                    self.targets.push(("Previous match".into(), previous.rect));
                    if previous.on_hover_text("Previous match").clicked() {
                        out.push(Outcome::Action(nav.previous.clone()));
                    }
                    ui.label(
                        RichText::new(format!("{} of {}", nav.current, nav.total))
                            .small()
                            .color(p.dim),
                    );
                    let next = ui.add_sized(
                        Vec2::new(26.0, 24.0),
                        egui::Button::new("›").corner_radius(CornerRadius::same(6)),
                    );
                    self.targets.push(("Next match".into(), next.rect));
                    if next.on_hover_text("Next match").clicked() {
                        out.push(Outcome::Action(nav.next.clone()));
                    }
                }
            });
            ui.add(egui::Label::new(RichText::new(&d.subtitle).small().color(p.dim)).wrap());
            self.why_ranked(ui, &d.signals, p);
            Self::note_pills(ui, &d.notes, p);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                for e in &d.actions {
                    let icon = match e.label.as_str() {
                        "Open" => "open",
                        "Show in folder" => "folder",
                        "Copy path" => "copy",
                        "Export matches" => "report",
                        _ => "",
                    };
                    let r = if icon.is_empty() {
                        ui.add(
                            egui::Button::new(RichText::new(&e.label).size(12.5))
                                .corner_radius(CornerRadius::same(7)),
                        )
                    } else {
                        let galley = ui.painter().layout_no_wrap(
                            e.label.clone(),
                            FontId::proportional(12.5),
                            p.text,
                        );
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(galley.size().x + 36.0, 32.0),
                            Sense::click(),
                        );
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(7),
                            if response.hovered() {
                                p.raised
                            } else {
                                Color32::TRANSPARENT
                            },
                            Stroke::NONE,
                            egui::StrokeKind::Inside,
                        );
                        super::panels::search_icon(
                            ui,
                            Rect::from_center_size(
                                Pos2::new(rect.left() + 14.0, rect.center().y),
                                Vec2::splat(16.0),
                            ),
                            icon,
                            p.dim,
                        );
                        ui.painter().galley(
                            Pos2::new(rect.left() + 27.0, rect.center().y - galley.size().y * 0.5),
                            galley,
                            p.text,
                        );
                        response
                    };
                    self.targets.push((e.label.clone(), r.rect));
                    let r = r.on_hover_text(e.hover_text());
                    self.entry_click(&r, e, out, hint);
                }
            });
            ui.add(egui::Separator::default().spacing(6.0));
            if let Some(drawing) = &d.drawing {
                let height = (ui.available_height() - 55.0).clamp(140.0, 450.0);
                egui::ScrollArea::vertical()
                    .id_salt("engineering-section")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.section_drawing(ui, drawing, p, out, height);
                        ui.label(RichText::new(&d.empty).small().color(p.dim));
                    });
                return;
            }
            if let Some(g) = &d.grid {
                egui::ScrollArea::both()
                    .id_salt(("doc-grid", &d.title))
                    .auto_shrink([false, false])
                    .show(ui, |ui| property_grid(ui, g, p));
                return;
            }
            if let Some(text) = &d.text {
                let scroll_to_line = if d
                    .scroll_request
                    .is_some_and(|request| Some(request) != self.doc.last_match_scroll_request)
                {
                    self.doc.last_match_scroll_request = d.scroll_request;
                    d.scroll_to_line
                } else {
                    None
                };
                let shown = egui::ScrollArea::vertical()
                    .id_salt(("full-document", &d.subtitle))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        selectable_document(ui, text, &d.title, p, scroll_to_line)
                    });
                match_gutter(ui, &shown.inner_rect, shown.state.offset.y, shown.content_size.y, text, p);
                return;
            }
            if d.blocks.is_empty() {
                ui.label(RichText::new(&d.empty).color(p.dim));
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt(("doc-detail", &d.title))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (k, b) in d.blocks.iter().enumerate() {
                        if k > 0 {
                            ui.add_space(6.0);
                            let (r, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 1.0),
                                Sense::hover(),
                            );
                            ui.painter().line_segment(
                                [r.left_center(), r.right_center()],
                                Stroke::new(1.0, with_alpha(p.line, 160)),
                            );
                            ui.add_space(6.0);
                        }
                        hit_block(ui, b, p);
                    }
                });
        });
    }
}

/// A relevance cell ("82%") as a fraction, for the bar; `None` for any other cell.
fn relevance_fraction(column: &str, cell: &str) -> Option<f32> {
    if column != "Relevance" {
        return None;
    }
    let value: f32 = cell.strip_suffix('%')?.trim().parse().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}

/// Where each match sits in the document, as ticks down a thin strip at the text's right edge, with the visible part
/// lit. The position is the byte share, which tracks the line share closely enough for orientation.
fn match_gutter(ui: &egui::Ui, view: &Rect, offset: f32, content_height: f32, text: &DocText, p: &Palette) {
    if text.marks.is_empty() || text.content.is_empty() || content_height <= view.height() + 1.0 {
        return;
    }
    let strip = Rect::from_min_max(Pos2::new(view.right() - 9.0, view.top() + 2.0), Pos2::new(view.right() - 3.0, view.bottom() - 2.0));
    let painter = ui.painter_at(*view);
    painter.rect_filled(strip, CornerRadius::same(3), with_alpha(p.line, 150));
    let lit_top = strip.top() + strip.height() * (offset / content_height).clamp(0.0, 1.0);
    let lit_height = (strip.height() * view.height() / content_height).max(6.0);
    painter.rect_filled(
        Rect::from_min_size(Pos2::new(strip.left(), lit_top), Vec2::new(strip.width(), lit_height)).intersect(strip),
        CornerRadius::same(3),
        with_alpha(p.dim, 70),
    );
    let total = text.content.len() as f32;
    for (start, _) in text.marks.iter() {
        let y = strip.top() + strip.height() * (*start as f32 / total);
        painter.rect_filled(
            Rect::from_center_size(Pos2::new(strip.center().x, y.clamp(strip.top() + 1.0, strip.bottom() - 1.0)), Vec2::new(strip.width(), 3.0)),
            CornerRadius::same(1),
            p.accent,
        );
    }
}

/// The page's bottom strip: a state dot and label, quiet facts, a segmented progress track, and a warning at the right.
fn ribbon(ui: &mut egui::Ui, strip: Rect, r: &crate::gpu_viewer::ui::DocRibbon, p: &Palette) {
    let painter = ui.painter_at(strip);
    painter.rect_filled(strip, CornerRadius::same(8), p.panel);
    painter.rect_stroke(strip, CornerRadius::same(8), Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    let mid = strip.center().y;
    let mut x = strip.left() + 14.0;
    painter.circle_filled(Pos2::new(x + 3.0, mid), 3.5, tone_fill(p, r.tone));
    x += 14.0;
    let put = |x: &mut f32, text: &str, color: Color32| {
        let g = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
        let width = g.size().x;
        painter.galley(Pos2::new(*x, mid - g.size().y / 2.0), g, color);
        *x += width + 18.0;
    };
    put(&mut x, &r.state, p.text);
    for fact in &r.facts {
        put(&mut x, fact, p.dim);
    }
    if let Some((label, fraction)) = &r.progress {
        put(&mut x, label, p.dim);
        const SEGMENTS: usize = 10;
        for k in 0..SEGMENTS {
            let seg = Rect::from_min_size(Pos2::new(x + k as f32 * 16.0, mid - 3.0), Vec2::new(14.0, 6.0));
            let filled = (k as f32 + 0.5) / SEGMENTS as f32 <= *fraction;
            painter.rect_filled(seg, CornerRadius::same(2), if filled { p.accent } else { p.line });
        }
    }
    if let Some(warning) = &r.warning {
        let g = painter.layout_no_wrap(warning.clone(), FontId::proportional(12.0), p.warn);
        painter.galley(Pos2::new(strip.right() - 14.0 - g.size().x, mid - g.size().y / 2.0), g, p.warn);
    }
}

fn selectable_document(
    ui: &mut egui::Ui,
    document: &DocText,
    title: &str,
    p: &Palette,
    scroll_to_line: Option<i32>,
) -> egui::Response {
    use egui::TextBuffer as _;
    let mut text: &str = &document.content;
    let mut layouter = |ui: &egui::Ui, value: &dyn egui::TextBuffer, width: f32| {
        let source = value.as_str();
        let mut job = LayoutJob {
            wrap: egui::text::TextWrapping {
                max_width: width,
                ..Default::default()
            },
            ..Default::default()
        };
        let normal = TextFormat {
            font_id: FontId::monospace(12.5),
            color: p.text,
            ..Default::default()
        };
        let marked = TextFormat {
            background: with_alpha(p.warn, 90),
            ..normal.clone()
        };
        let mut pos = 0;
        for &(start, end) in document.marks.iter() {
            if start < pos
                || end > source.len()
                || !source.is_char_boundary(start)
                || !source.is_char_boundary(end)
            {
                continue;
            }
            job.append(&source[pos..start], 0.0, normal.clone());
            job.append(&source[start..end], 0.0, marked.clone());
            pos = end;
        }
        job.append(&source[pos..], 0.0, normal);
        ui.painter().layout_job(job)
    };
    let output = egui::TextEdit::multiline(&mut text)
        .font(FontId::monospace(12.5))
        .desired_width(ui.available_width())
        .layouter(&mut layouter)
        .show(ui);
    if let Some(line) = scroll_to_line.filter(|line| *line > 0) {
        let line_index = (line - 1) as usize;
        let line_count = document.content.split('\n').count();
        if line_index < line_count {
            let target_char = document
                .content
                .split('\n')
                .take(line_index)
                .map(|part| part.chars().count() + 1)
                .sum::<usize>();
            let mut char_offset = 0usize;
            let target = output.galley.rows.iter().find_map(|row| {
                let start = char_offset;
                let end = start + row.glyphs.len();
                char_offset = end + usize::from(row.ends_with_newline);
                (target_char < end || (row.glyphs.is_empty() && target_char == start))
                    .then(|| row.rect().translate(output.galley_pos.to_vec2()))
            });
            if let Some(rect) = target {
                ui.scroll_to_rect(rect, Some(Align::Center));
            }
        }
    }
    let selection_id = output.response.id.with(("copy-selection", title));
    let previous = ui
        .ctx()
        .data(|data| data.get_temp::<(egui::text_selection::CCursorRange, String)>(selection_id));
    let stored_range = egui::text_edit::TextEditState::load(ui.ctx(), output.response.id)
        .and_then(|state| state.cursor.char_range());
    let selected_range = output
        .cursor_range
        .or(stored_range)
        .filter(|range| !range.is_empty());
    let selected =
        selected_range.map(|range| text.char_range(range.as_sorted_char_range()).to_string());
    let selected = selected.or_else(|| previous.as_ref().map(|(_, value)| value.clone()));
    if output.response.secondary_clicked() {
        if let (Some(range), Some(value)) = (selected_range, selected.as_ref()) {
            ui.ctx()
                .data_mut(|data| data.insert_temp(selection_id, (range, value.clone())));
        }
        if let Some((range, _)) = &previous {
            if let Some(mut state) =
                egui::text_edit::TextEditState::load(ui.ctx(), output.response.id)
            {
                state.cursor.set_char_range(Some(*range));
                state.store(ui.ctx(), output.response.id);
            }
        }
    } else if let (Some(range), Some(value)) = (selected_range, selected.as_ref()) {
        ui.ctx()
            .data_mut(|data| data.insert_temp(selection_id, (range, value.clone())));
    }
    let response = output.response.response;
    response.clone().context_menu(|ui| {
        let selected = ui.ctx().data(|data| {
            data.get_temp::<(egui::text_selection::CCursorRange, String)>(selection_id)
                .map(|(_, value)| value)
        });
        if ui
            .add_enabled(selected.is_some(), egui::Button::new("Copy selection"))
            .clicked()
        {
            if copy_document_selection(ui.ctx(), selection_id) {
                ui.close();
            }
        }
    });
    if response.clicked_by(egui::PointerButton::Primary) && !response.context_menu_opened() {
        ui.ctx().data_mut(|data| {
            data.remove::<(egui::text_selection::CCursorRange, String)>(selection_id)
        });
    }
    response
}

fn copy_document_selection(ctx: &egui::Context, selection_id: Id) -> bool {
    let selected = ctx.data(|data| {
        data.get_temp::<(egui::text_selection::CCursorRange, String)>(selection_id)
            .map(|(_, value)| value)
    });
    let Some(selected) = selected else {
        return false;
    };
    ctx.copy_text(selected);
    ctx.data_mut(|data| data.remove::<(egui::text_selection::CCursorRange, String)>(selection_id));
    true
}

fn segment_width(ui: &egui::Ui, label: &str, p: &Palette) -> f32 {
    ui.painter()
        .layout_no_wrap(label.to_string(), FontId::proportional(13.0), p.text)
        .size()
        .x
        + 26.0
}
/// The width of a segmented control (0 when there is none).
pub(super) fn segmented_width(ui: &egui::Ui, entries: &[Entry], p: &Palette) -> f32 {
    if entries.is_empty() {
        0.0
    } else {
        entries
            .iter()
            .map(|e| segment_width(ui, &e.label, p))
            .sum::<f32>()
            + 6.0
    }
}

/// Column widths: fixed ones as given, the flexible one (width 0) takes the rest.
fn column_widths(t: &DocTable, width: f32) -> Vec<f32> {
    let fixed: f32 = t.columns.iter().map(|c| c.width).sum();
    let flex = t.columns.iter().filter(|c| c.width <= 0.0).count().max(1) as f32;
    let rest = ((width - fixed) / flex).max(120.0);
    t.columns
        .iter()
        .map(|c| if c.width > 0.0 { c.width } else { rest })
        .collect()
}

fn stat_tiles(ui: &mut egui::Ui, stats: &[(String, String)], p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 8.0);
        for (label, value) in stats {
            egui::Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
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

/// Labelled values in columns (a material's properties, a comparison): dim small-caps header, striped rows, values in
/// monospace so digits line up, the best of a row in the good colour.
fn property_grid(ui: &mut egui::Ui, g: &DocGrid, p: &Palette) {
    egui::Grid::new("doc-grid")
        .striped(true)
        .spacing(Vec2::new(18.0, 7.0))
        .show(ui, |ui| {
            if !g.columns.is_empty() {
                for c in &g.columns {
                    ui.label(RichText::new(c.to_uppercase()).size(10.5).color(p.dim));
                }
                ui.end_row();
            }
            for (i, r) in g.rows.iter().enumerate() {
                if r.section {
                    // A short blank row between sections (`add_space` is not allowed inside a grid).
                    if i > 0 {
                        ui.label(RichText::new(" ").size(4.0));
                        ui.end_row();
                    }
                    ui.label(
                        RichText::new(r.cells.first().map_or("", String::as_str))
                            .strong()
                            .size(12.5)
                            .color(p.accent),
                    );
                    ui.end_row();
                    continue;
                }
                for (k, cell) in r.cells.iter().enumerate() {
                    if k == 0 {
                        ui.label(RichText::new(cell).color(p.text));
                    } else if r.best.contains(&k) {
                        ui.label(RichText::new(cell).monospace().strong().color(p.good));
                    } else {
                        ui.label(RichText::new(cell).monospace().color(
                            if k + 1 == r.cells.len()
                                && g.columns.last().is_some_and(|c| c == "Unit")
                            {
                                p.dim
                            } else {
                                p.text
                            },
                        ));
                    }
                }
                ui.end_row();
            }
        });
    if !g.note.is_empty() {
        ui.add_space(10.0);
        ui.add(egui::Label::new(RichText::new(&g.note).small().color(p.dim)).wrap());
    }
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
            egui::Frame::new()
                .fill(with_alpha(p.accent, 26))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Margin::symmetric(6, 1))
                .show(ui, |ui| {
                    ui.label(RichText::new(t).size(10.5).color(p.accent));
                });
        }
    });
    ui.add_space(3.0);
    let line = |ui: &mut egui::Ui, number: Option<i32>, job: LayoutJob| {
        let galley = ui.painter().layout_job(job);
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(GUTTER + 6.0 + width, galley.size().y + 2.0),
            Sense::hover(),
        );
        if let Some(n) = number {
            ui.painter().text(
                Pos2::new(r.left() + GUTTER - 12.0, r.top() + 1.0),
                Align2::RIGHT_TOP,
                n.to_string(),
                FontId::monospace(11.5),
                p.dim,
            );
        }
        ui.painter()
            .galley(Pos2::new(r.left() + GUTTER, r.top() + 1.0), galley, p.text);
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
    let mut job = LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: width,
            max_rows: 6,
            ..Default::default()
        },
        ..Default::default()
    };
    let normal = TextFormat {
        font_id: mono.clone(),
        color: p.text,
        ..Default::default()
    };
    let marked = TextFormat {
        font_id: mono.clone(),
        color: p.text,
        background: with_alpha(p.warn, 90),
        ..Default::default()
    };
    let mut pos = 0;
    for &(s, e) in &b.marks {
        if s < pos || e > b.text.len() || !b.text.is_char_boundary(s) || !b.text.is_char_boundary(e)
        {
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
    let (r, _) = ui.allocate_exact_size(
        Vec2::new(GUTTER + 6.0 + width, galley.size().y + 4.0),
        Sense::hover(),
    );
    ui.painter().set(
        highlight,
        Shape::rect_filled(
            Rect::from_min_max(Pos2::new(r.left() + GUTTER - 6.0, r.top()), r.max),
            CornerRadius::same(4),
            with_alpha(p.accent, 16),
        ),
    );
    ui.painter().text(
        Pos2::new(r.left() + GUTTER - 12.0, r.top() + 2.0),
        Align2::RIGHT_TOP,
        b.number.to_string(),
        FontId::monospace(11.5),
        p.accent,
    );
    ui.painter()
        .galley(Pos2::new(r.left() + GUTTER, r.top() + 2.0), galley, p.text);
    if let Some(t) = &b.after {
        line(ui, Some(b.number + 1), plain(t, p.dim));
    }
}
