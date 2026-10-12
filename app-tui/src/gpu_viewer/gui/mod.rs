//! The window chrome, built with egui (ADR-017): a thin top bar (menu, step strip, search, theme), a
//! contextual tool pill floating over the model, the feature tree and the property manager docked on the
//! sides, a status bar, the command palette (Ctrl/Cmd+K) and right-click menus. The model fills the space
//! between the docks; `Gui::viewport_px` is where the window draws it.
//!
//! The front end only reads what the editor and the viewer expose (`Authoring::tree/properties/ribbon/menus`,
//! `menus::bar`) and reports clicks as `Outcome`s; availability rules stay with the model that owns them.
mod panels;
pub mod theme;

use super::{
    controller::{Panel, Visible},
    editor::{adapt::Adapt, authoring::StepState},
    results::GuiInput,
    menu::{palette_entries, Entry, Item, Outcome},
    menus::{self, MenuContext},
    ui::RibbonTab,
};
use egui::{Align2, Color32, Id, Order, Pos2, Rect, Vec2};
use std::collections::{HashMap, HashSet};
use theme::Palette;

/// A pixel rectangle in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}
impl PxRect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && y >= self.y as f64 && x < (self.x + self.w) as f64 && y < (self.y + self.h) as f64
    }
}

#[derive(Default)]
struct PaletteState {
    query: String,
    item: usize,
    /// Focus the text field on the next frame (just opened).
    fresh: bool,
}

/// A list of entries floating at a point (right-click menu, applicable supports of a clicked entity).
struct Floating {
    at: Pos2,
    items: Vec<Item>,
}

pub struct Gui {
    ctx: egui::Context,
    pub visible: Visible,
    tab: usize,
    collapsed: HashSet<u32>,
    palette: Option<PaletteState>,
    floating: Option<Floating>,
    hint: Option<String>,
    viewport: Option<Rect>,
    /// Interactive controls of the last frame by label, in points (tests click them).
    targets: Vec<(String, Rect)>,
    /// The row under the pointer that previews its effect this frame, and the one that did last frame.
    hover_now: Option<(crate::gpu_viewer::controller::UiAction, crate::gpu_viewer::controller::UiAction)>,
    hover_prev: Option<(crate::gpu_viewer::controller::UiAction, crate::gpu_viewer::controller::UiAction)>,
    /// What the layout has learned from use (ADR-018); decides orders, collapsed cards and the tree's default.
    adapt: Adapt,
    /// Cards the person opened or folded this session (overrides what was learned).
    card_open: HashMap<String, bool>,
    /// The inspector was closed for this selection (its cards' ids and titles); it returns when they change.
    inspector_dismissed: Option<String>,
    inspector_pinned: bool,
    /// The person opened or folded the tree; `None` follows what was learned.
    tree_user: Option<bool>,
    /// A right click on a tree row: select it now, open its menu next frame.
    pending_tree_menu: Option<(Pos2, u32)>,
    /// Last known state of each step, and when it became done (egui time).
    prev_steps: HashMap<String, StepState>,
    done_at: HashMap<String, f64>,
    /// The step the person is probably about to take, drawn as a ring on its chip.
    nudge: Option<String>,
    now: f64,
    /// Where the tool pill is this frame; the inspector sits below it instead of covering its buttons.
    pill_rect: Option<Rect>,
    /// The step the selection last belonged to (a change brings that step's tools).
    last_focus_step: Option<&'static str>,
}

impl Default for Gui {
    fn default() -> Self {
        Self::new()
    }
}

impl Gui {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        Self {
            ctx,
            visible: Visible::default(),
            tab: 0,
            collapsed: HashSet::new(),
            palette: None,
            floating: None,
            hint: None,
            viewport: None,
            targets: Vec::new(),
            hover_now: None,
            hover_prev: None,
            adapt: Adapt::in_memory(),
            card_open: HashMap::new(),
            inspector_dismissed: None,
            inspector_pinned: false,
            tree_user: None,
            pending_tree_menu: None,
            prev_steps: HashMap::new(),
            done_at: HashMap::new(),
            nudge: None,
            now: 0.0,
            pill_rect: None,
            last_focus_step: None,
        }
    }

    /// Keep what the layout learns between sessions (`<app data>/fea-editor/layout`).
    pub fn attach_layout_store(&mut self) {
        if let Some(dir) = Adapt::default_location() {
            self.adapt = Adapt::load(&dir);
        }
    }
    /// As `attach_layout_store`, in `dir` (tests).
    pub fn attach_layout_dir(&mut self, dir: &std::path::Path) {
        self.adapt = Adapt::load(dir);
    }
    pub fn flush_layout(&mut self) {
        self.adapt.flush();
    }
    pub fn adapt_enabled(&self) -> bool {
        self.adapt.enabled
    }
    pub fn set_adaptive(&mut self, on: bool) {
        self.adapt.enabled = on;
    }
    pub fn reset_layout(&mut self) {
        self.adapt.reset();
        self.card_open.clear();
        self.tree_user = None;
    }
    pub fn adapt_summary(&self) -> String {
        self.adapt.summary()
    }
    /// Open the tree when it is a rail, fold it when it is open.
    pub fn toggle_tree_open(&mut self) {
        let open = self.tree_user.unwrap_or_else(|| self.adapt.tree_open_by_default());
        self.tree_user = Some(!open);
    }
    pub(crate) fn tree_is_open(&self) -> bool {
        self.tree_user.unwrap_or_else(|| self.adapt.tree_open_by_default())
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }
    pub fn toggle(&mut self, panel: Panel) {
        match panel {
            Panel::Ribbon => self.visible.ribbon = !self.visible.ribbon,
            Panel::Tree => self.visible.tree = !self.visible.tree,
            Panel::Properties => self.visible.properties = !self.visible.properties,
            Panel::Status => self.visible.status = !self.visible.status,
        }
    }
    pub fn open_palette(&mut self) {
        self.palette = Some(PaletteState { fresh: true, ..Default::default() });
        self.floating = None;
    }
    /// Show `items` at a window pixel (right click, or the list a click on the model asked for).
    pub fn open_context(&mut self, at_px: (f64, f64), items: Vec<Item>) {
        let ppp = self.ctx.pixels_per_point().max(0.1);
        self.floating = Some(Floating { at: Pos2::new(at_px.0 as f32 / ppp, at_px.1 as f32 / ppp), items });
        self.palette = None;
    }
    /// Close whatever floats (palette, context list); true when something was open.
    pub fn close_floating(&mut self) -> bool {
        self.palette.take().is_some() | self.floating.take().is_some()
    }
    pub fn floating_open(&self) -> bool {
        self.palette.is_some() || self.floating.is_some()
    }
    /// A text field owns the keyboard (the palette's query).
    pub fn wants_keyboard(&self) -> bool {
        self.palette.is_some() || self.ctx.egui_wants_keyboard_input()
    }
    pub fn wants_pointer(&self) -> bool {
        self.ctx.egui_wants_pointer_input()
    }
    /// Background colour of the viewport for the theme in use (sRGB).
    pub fn background_rgb(&self) -> [u8; 3] {
        let c = theme::current(&self.ctx).bg;
        [c.r(), c.g(), c.b()]
    }
    /// The rectangle the model is drawn into, in window pixels (the whole window before the first frame).
    pub fn viewport_px(&self, window: (u32, u32)) -> PxRect {
        let ppp = self.ctx.pixels_per_point();
        match self.viewport {
            Some(r) => {
                let (x, y) = ((r.min.x * ppp).round().max(0.0) as u32, (r.min.y * ppp).round().max(0.0) as u32);
                let w = ((r.max.x * ppp).round() as u32).saturating_sub(x).min(window.0.saturating_sub(x)).max(1);
                let h = ((r.max.y * ppp).round() as u32).saturating_sub(y).min(window.1.saturating_sub(y)).max(1);
                PxRect { x, y, w, h }
            }
            None => PxRect { x: 0, y: 0, w: window.0, h: window.1 },
        }
    }
    /// Centre (window pixels) of the first control of the last frame whose label starts with `prefix`.
    #[cfg(test)]
    pub(crate) fn find_target(&self, prefix: &str) -> Option<(f64, f64)> {
        let ppp = self.ctx.pixels_per_point();
        let (_, r) = self.targets.iter().find(|(l, _)| l.starts_with(prefix))?;
        Some(((r.center().x * ppp) as f64, (r.center().y * ppp) as f64))
    }
    #[cfg(test)]
    pub(crate) fn target_labels(&self) -> Vec<String> {
        self.targets.iter().map(|(l, _)| l.clone()).collect()
    }
    #[cfg(test)]
    pub(crate) fn adapt_mut(&mut self) -> &mut Adapt {
        &mut self.adapt
    }
    #[cfg(test)]
    pub(crate) fn card_open(&self, id: &str) -> Option<bool> {
        self.card_open.get(id).copied()
    }
    #[cfg(test)]
    pub(crate) fn nudge(&self) -> Option<&str> {
        self.nudge.as_deref()
    }
    #[cfg(test)]
    pub(crate) fn palette_open(&self) -> bool {
        self.palette.is_some()
    }
    #[cfg(test)]
    pub(crate) fn context_open(&self) -> bool {
        self.floating.is_some()
    }
    #[cfg(test)]
    pub(crate) fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Run one frame: build the chrome from `input`, return what egui must paint and the clicks.
    pub fn run(&mut self, raw: egui::RawInput, input: &GuiInput) -> (egui::FullOutput, Vec<Outcome>) {
        let ctx = self.ctx.clone();
        let mut out = Vec::new();
        self.targets.clear();
        self.hover_now = None;
        let mut hint = None;
        let full = ctx.run_ui(raw, |ui| self.build(ui, input, &mut out, &mut hint));
        self.hint = hint;
        // A previewing row: `on` when the pointer arrives (or moves to another such row), `off` when it leaves them all.
        let now = self.hover_now.take();
        if now != self.hover_prev {
            match (&now, &self.hover_prev) {
                (Some((on, _)), _) => out.push(Outcome::Action(*on)),
                (None, Some((_, off))) => out.push(Outcome::Action(*off)),
                (None, None) => {}
            }
            self.hover_prev = now;
        }
        (full, out)
    }

    fn build(&mut self, ui: &mut egui::Ui, input: &GuiInput, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let p = theme::current(ui.ctx());
        self.now = ui.ctx().input(|i| i.time);
        self.pill_rect = None;
        let mctx = MenuContext { view: input.view, scene: input.scene(), results: if input.editor.is_some() { 1 } else { input.scenes.len() }, editor: input.editor, visible: self.visible, adaptive: self.adapt.enabled };
        let menus = menus::bar(&mctx);
        let tabs = menus::ribbon(&mctx);
        self.tab = self.tab.min(tabs.len().saturating_sub(1));
        // A right click on a tree row selected it last frame; its menu is built from the selected state.
        if let Some((at, node)) = self.pending_tree_menu.take() {
            let items = input.editor.map(|e| e.tree_menu(node)).unwrap_or_default();
            if !items.is_empty() {
                self.floating = Some(Floating { at, items });
                self.palette = None;
            }
        }
        self.track_steps(input, &tabs);
        // Selecting something in the tree brings the tools of the step it belongs to.
        let focus = input.editor.and_then(|e| e.focus_step());
        if focus != self.last_focus_step {
            self.last_focus_step = focus;
            if let Some(i) = focus.and_then(|f| tabs.iter().position(|t| t.title == f)) {
                self.tab = i;
            }
        }

        self.top_bar(ui, input, &menus, &tabs, &p, out, hint);
        if self.visible.status {
            self.status_bar(ui, input, &p, out, hint);
        }
        let tree_open = self.tree_is_open();
        if self.visible.tree && tree_open {
            self.tree_panel(ui, input, &p, out);
        }
        // What the panels leave is the model's viewport, less the room the inspector's cards take when they are shown
        // (the model is never drawn under them). It is deliberately not a CentralPanel: that would use the space up, and
        // egui would then count the pointer over the model as being over the interface.
        let plan = if self.visible.properties { self.inspector_plan(input, &tabs) } else { None };
        let mut free = ui.available_rect_before_wrap();
        if plan.is_some() {
            free.max.x = (free.max.x - panels::INSPECTOR_WIDTH - 36.0).max(free.min.x + 240.0);
        }
        self.viewport = Some(free);
        if self.visible.tree && !tree_open {
            self.tree_chip(ui.ctx(), &p);
        }
        if self.visible.ribbon {
            self.tool_pill(ui.ctx(), &tabs, &p, out, hint);
        }
        if let Some(plan) = &plan {
            self.inspector(ui.ctx(), plan, &p, out, hint);
        }
        self.model_labels(ui.ctx(), input, &p);
        self.start_card(ui.ctx(), input, &p, out, hint);
        self.selection_box(ui.ctx(), input, &p);
        self.floating_list(ui.ctx(), &p, out, hint);
        self.palette_window(ui.ctx(), &menus, &p, out);
        self.adapt.frame_end();
    }

    /// Watch the steps: a finished step either moves on to the one this person usually takes next (once that is a
    /// clear habit) or marks it with a ring; going to another step soon after one finished teaches the habit.
    fn track_steps(&mut self, input: &GuiInput, tabs: &[RibbonTab]) {
        let Some(ed) = input.editor else { return };
        let active = tabs.get(self.tab).map(|t| t.title.clone());
        for (i, t) in tabs.iter().enumerate() {
            let Some(now) = ed.step_state(&t.title) else { continue };
            let before = self.prev_steps.insert(t.title.clone(), now);
            if before == Some(StepState::Todo) && now == StepState::Done {
                self.done_at.insert(t.title.clone(), self.now);
                if active.as_deref() == Some(t.title.as_str()) {
                    let following = tabs.iter().skip(i + 1).find(|n| ed.step_state(&n.title) == Some(StepState::Todo)).map(|n| n.title.clone());
                    match self.adapt.predicted_next(&t.title) {
                        Some((next, _)) if tabs.iter().any(|n| n.title == next) => {
                            self.tab = tabs.iter().position(|n| n.title == next).unwrap_or(self.tab);
                            self.nudge = None;
                        }
                        _ => self.nudge = following,
                    }
                }
            }
        }
    }

    /// The person chose step `to` (index into `tabs`).
    pub(super) fn select_tab(&mut self, to: usize, tabs: &[RibbonTab]) {
        let (Some(from), Some(next)) = (tabs.get(self.tab), tabs.get(to)) else { return };
        if from.title != next.title && self.done_at.get(&from.title).is_some_and(|t| self.now - t < 90.0) {
            self.adapt.step_taken(&from.title, &next.title);
        }
        if self.nudge.as_deref() == Some(next.title.as_str()) {
            self.nudge = None;
        }
        self.tab = to;
    }

    /// Dimension values and constraint glyphs the editor placed on the model, clipped to the viewport.
    fn model_labels(&self, ctx: &egui::Context, input: &GuiInput, p: &Palette) {
        use crate::gpu_viewer::editor::authoring::OverlayKind;
        let Some(vp) = self.viewport else { return };
        if input.overlay.is_empty() {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Middle, Id::new("model-labels"))).with_clip_rect(vp);
        // The window's viewport pixels are physical; egui draws in points.
        let origin = vp.min;
        for o in input.overlay {
            let at = Pos2::new(origin.x + o.px[0] / ppp, origin.y + o.px[1] / ppp);
            match o.kind {
                OverlayKind::Dimension => {
                    let galley = painter.layout_no_wrap(o.text.clone(), egui::FontId::monospace(12.0), p.text);
                    let rect = Rect::from_center_size(at, galley.size() + Vec2::new(10.0, 4.0));
                    painter.rect(rect, 4.0, with_alpha(p.panel, 235), egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
                    painter.galley(rect.center() - galley.size() / 2.0, galley, p.text);
                }
                OverlayKind::Constraint => {
                    let galley = painter.layout_no_wrap(o.text.clone(), egui::FontId::monospace(10.0), p.dim);
                    let rect = Rect::from_center_size(at, galley.size() + Vec2::new(6.0, 2.0));
                    painter.rect(rect, 3.0, with_alpha(p.panel, 215), egui::Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
                    painter.galley(rect.center() - galley.size() / 2.0, galley, p.dim);
                }
            }
        }
    }

    fn selection_box(&self, ctx: &egui::Context, input: &GuiInput, p: &Palette) {
        let Some((a, b)) = input.box_drag else { return };
        let ppp = ctx.pixels_per_point();
        let pt = |q: (f64, f64)| Pos2::new(q.0 as f32 / ppp, q.1 as f32 / ppp);
        let r = Rect::from_two_pos(pt(a), pt(b));
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("selection-box")));
        painter.rect(r, 2.0, p.accent.gamma_multiply(0.16), egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
    }

    /// Right-click menu and the lists a click on the model asks for.
    fn floating_list(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) {
        let Some(f) = self.floating.take() else { return };
        let mut chosen = false;
        let resp = egui::Area::new(Id::new("floating-list")).fade_in(false).order(Order::Foreground).fixed_pos(f.at).constrain(true).show(ctx, |ui| {
            panels::popup_frame(p).show(ui, |ui| {
                ui.set_min_width(210.0);
                chosen = self.items_ui(ui, &f.items, p, out, hint);
            });
        });
        let pressed_outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !resp.response.rect.contains(q));
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if !(chosen || pressed_outside || escape) {
            self.floating = Some(f);
        }
    }

    /// One list of entries: entries, separators and headings. True when an entry was chosen.
    fn items_ui(&mut self, ui: &mut egui::Ui, items: &[Item], p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>) -> bool {
        let mut chosen = false;
        for item in items {
            match item {
                Item::Entry(e) => chosen |= self.entry_button(ui, e, p, out, hint, true),
                Item::Separator => {
                    ui.separator();
                }
                Item::Heading(h) => {
                    ui.label(egui::RichText::new(h).small().color(p.dim));
                }
            }
        }
        chosen
    }

    /// A button for `e`: checked entries are highlighted, unavailable ones dim but still explain themselves when
    /// clicked. True when clicked.
    fn entry_button(&mut self, ui: &mut egui::Ui, e: &Entry, p: &Palette, out: &mut Vec<Outcome>, hint: &mut Option<String>, wide: bool) -> bool {
        let mut label = egui::RichText::new(&e.label);
        if !e.enabled {
            label = label.color(p.dim.gamma_multiply(0.75));
        }
        let mut b = egui::Button::new(label).selected(e.checked == Some(true));
        if !e.shortcut.is_empty() && wide {
            b = b.shortcut_text(egui::RichText::new(&e.shortcut).small().color(p.dim));
        }
        if wide {
            b = b.min_size(Vec2::new(ui.available_width().max(190.0), 0.0));
        }
        let r = ui.add(b);
        self.targets.push((e.label.clone(), r.rect));
        if r.hovered() {
            *hint = Some(e.hover_text());
        }
        let r = r.on_hover_text(e.hover_text());
        if r.clicked() {
            out.push(match (&e.reason, e.enabled) {
                (Some(reason), false) => Outcome::Blocked(reason.clone()),
                _ => Outcome::Action(e.action),
            });
            ui.close();
            return true;
        }
        false
    }

    fn palette_window(&mut self, ctx: &egui::Context, menus: &[crate::gpu_viewer::menu::Menu], p: &Palette, out: &mut Vec<Outcome>) {
        let Some(mut st) = self.palette.take() else { return };
        let rows = palette_entries(menus, &st.query);
        st.item = st.item.min(rows.len().saturating_sub(1));
        let mut keep = true;
        let mut run: Option<usize> = None;
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
        if escape {
            keep = false;
        }
        if enter && !rows.is_empty() {
            run = Some(st.item);
        }
        let resp = egui::Area::new(Id::new("palette")).fade_in(false).order(Order::Foreground).anchor(Align2::CENTER_TOP, Vec2::new(0.0, 64.0)).show(ctx, |ui| {
            panels::popup_frame(p).show(ui, |ui| {
                ui.set_width(520.0);
                let edit = egui::TextEdit::singleline(&mut st.query).hint_text("Search commands").desired_width(f32::INFINITY).margin(Vec2::new(8.0, 6.0));
                let r = ui.add(edit);
                if st.fresh {
                    r.request_focus();
                    st.fresh = false;
                }
                ui.add_space(2.0);
                if rows.is_empty() {
                    ui.label(egui::RichText::new("No command matches").color(p.dim));
                }
                for (k, e) in rows.iter().enumerate() {
                    let mut label = egui::RichText::new(&e.label);
                    if !e.enabled {
                        label = label.color(p.dim);
                    }
                    let mut b = egui::Button::new(label).selected(k == st.item).min_size(Vec2::new(ui.available_width(), 0.0));
                    if !e.shortcut.is_empty() {
                        b = b.shortcut_text(egui::RichText::new(&e.shortcut).small().color(p.dim));
                    }
                    let r = ui.add(b);
                    self.targets.push((format!("palette {}", e.label), r.rect));
                    if r.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                        st.item = k;
                    }
                    if r.clicked() {
                        run = Some(k);
                    }
                    if let Some(reason) = e.reason.as_ref().filter(|_| k == st.item && !e.enabled) {
                        ui.label(egui::RichText::new(reason).small().color(p.warn));
                    }
                }
            });
        });
        let pressed_outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !resp.response.rect.contains(q));
        if pressed_outside {
            keep = false;
        }
        if let Some(k) = run {
            let e = &rows[k];
            match (&e.reason, e.enabled) {
                (Some(reason), false) => out.push(Outcome::Blocked(reason.clone())),
                _ => {
                    out.push(Outcome::Action(e.action));
                    keep = false;
                }
            }
        }
        if keep {
            self.palette = Some(st);
        }
    }

    fn selected_tab<'a>(&self, tabs: &'a [RibbonTab]) -> Option<&'a RibbonTab> {
        tabs.get(self.tab)
    }
}

/// `Color32` with its alpha replaced.
pub(crate) fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

#[cfg(test)]
impl Gui {
    /// Select the ribbon tab called `title` (tests and snapshots).
    pub(crate) fn select_tab_named(&mut self, input: &GuiInput, title: &str) {
        let mctx = MenuContext { view: input.view, scene: input.scene(), results: if input.editor.is_some() { 1 } else { input.scenes.len() }, editor: input.editor, visible: self.visible, adaptive: self.adapt.enabled };
        if let Some(i) = menus::ribbon(&mctx).iter().position(|t| t.title == title) {
            self.tab = i;
        }
    }
}
