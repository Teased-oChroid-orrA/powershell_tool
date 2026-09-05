//! Design System Epic Phase 2: shared component primitives (Button
//! variants, Segmented control, Select, Tooltip, EmptyState, Toast).
//! Adapted from `NATIVE_PREMIUM_UI_SYSTEM_EPIC.md`'s component catalog,
//! scoped to what this 3-tool app actually needs - see the approved
//! planning artifact's "Design system" tier for the deferred rest
//! (DataTable, FileBrowser, Dock, NotificationCenter, etc. don't fit a
//! single-window engineering-calculator app).

// `ButtonVariant::Secondary`/`Ghost` and `ToastKind::Info`/`Error` have no
// real call site yet (Danger/Success do - see `search.rs`) - kept as
// reusable variants for Phase 3+ call sites and other tools to reach for,
// same treatment as Phase 1's not-yet-consumed typography/spacing scale.
#![allow(dead_code)]

use eframe::egui::{self, Color32, RichText, Stroke};

use super::{radii, shadows, typography};
use crate::theme::Tokens;

/// Semantic intent for `button` - drives fill/stroke, not the widget's
/// shape (every variant keeps the same size/rounding so they read as one
/// family, not four different-looking controls).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// The one primary action per view (e.g. "Run search") - filled with
    /// the accent color, matching the mockup's `.btn-primary`.
    Primary,
    /// Any other ordinary action - the existing app-wide default look
    /// (`bg_sunken` fill, `border_strong` stroke) egui's theme already
    /// gives a bare `ui.button()`, named here so call sites can be
    /// explicit about which variant they mean.
    Secondary,
    /// A low-emphasis action that shouldn't compete for attention (e.g. a
    /// row's inline "remove" control) - no fill/stroke until hovered.
    Ghost,
    /// A destructive/irreversible action - tinted with `tokens.danger`.
    Danger,
}

/// A styled `egui::Button` for one of the four semantic variants. Returns
/// the `Response` (same as `ui.add_enabled(...)`) so callers use
/// `.clicked()`/`.on_hover_text()` exactly as before - this only changes
/// what gets painted, not the calling convention. `enabled` matches
/// `ui.add_enabled`'s own parameter (egui dims disabled widgets via its
/// global `visuals.disabled` handling regardless of custom fill/stroke).
pub fn button(ui: &mut egui::Ui, tokens: &Tokens, variant: ButtonVariant, label: &str, enabled: bool) -> egui::Response {
    button_sized(ui, tokens, variant, label, enabled, egui::vec2(0.0, 0.0))
}

/// Same as `button`, with an explicit minimum size - for the one
/// full-width "primary action" button per view (e.g. Search's "Run
/// search", sized to 60% of its row's available width).
pub fn button_sized(ui: &mut egui::Ui, tokens: &Tokens, variant: ButtonVariant, label: &str, enabled: bool, min_size: egui::Vec2) -> egui::Response {
    let (fill, stroke, text_color) = match variant {
        ButtonVariant::Primary => (tokens.accent, Stroke::NONE, tokens.accent_fg),
        ButtonVariant::Secondary => (tokens.bg_sunken, Stroke::new(1.0, tokens.border_strong), tokens.fg),
        ButtonVariant::Ghost => (Color32::TRANSPARENT, Stroke::NONE, tokens.fg_muted),
        ButtonVariant::Danger => (tokens.danger_bg, Stroke::new(1.0, tokens.danger), tokens.danger),
    };
    ui.add_enabled(enabled, egui::Button::new(RichText::new(label).color(text_color)).fill(fill).stroke(stroke).rounding(radii::md()).min_size(min_size))
}

/// `.segmented` - a bordered pill container divided into N mutually-
/// exclusive options, the current one tinted with the accent color. This
/// is the one real *new* visual treatment Phase 2 introduces (every prior
/// mutually-exclusive chip row in this app - head type, countersink mode,
/// match mode, etc. - used bare `ui.selectable_label`/`selectable_value`
/// calls, each option its own independently-rounded rect with no shared
/// container, not a connected segmented group). Returns whether the
/// selection changed this frame.
pub fn segmented<T: Copy + PartialEq>(ui: &mut egui::Ui, tokens: &Tokens, current: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::default().fill(tokens.bg_sunken).stroke(Stroke::new(1.0, tokens.border)).rounding(radii::md()).inner_margin(2.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (value, label) in options {
                let is_current = *current == *value;
                let text = RichText::new(*label).font(typography::body_small()).color(if is_current { tokens.accent_strong } else { tokens.fg_muted });
                let btn = egui::Button::new(text)
                    .fill(if is_current { tokens.accent.gamma_multiply(0.20) } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .rounding(radii::sm());
                if ui.add(btn).clicked() && !is_current {
                    *current = *value;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// `.field select` - the same label-above-input `.field` treatment
/// `widgets::num_field`/`text_field` already give every other input,
/// applied to `egui::ComboBox` for the first time (every existing
/// dropdown in this app used `ComboBox::from_label`, which puts its label
/// to the LEFT of the box - the odd one out next to every numeric/text
/// field's label-above layout). `add_contents` renders the dropdown's
/// options exactly as a raw `ComboBox::show_ui` closure would (typically
/// a run of `ui.selectable_value(...)` calls) - this wrapper only
/// restyles the surrounding label/spacing, not the option list itself.
pub fn select_field(ui: &mut egui::Ui, tokens: &Tokens, label: &str, selected_text: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.colored_label(tokens.fg_muted, RichText::new(label).font(typography::label()));
        ui.add_space(3.0);
        egui::ComboBox::from_id_salt(label).selected_text(selected_text).width(220.0).show_ui(ui, add_contents);
    });
    ui.add_space(7.0);
}

/// A styled hover tooltip - `response.on_hover_ui` already exists in egui,
/// this just gives it this app's card-like chrome (border/rounding/bg)
/// instead of egui's default plain popup styling, so a tooltip reads as
/// part of this app rather than a generic library default.
pub fn tooltip(response: egui::Response, tokens: &Tokens, text: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        egui::Frame::default().fill(tokens.bg_raised).stroke(Stroke::new(1.0, tokens.border)).rounding(radii::sm()).inner_margin(8.0).show(ui, |ui| {
            ui.colored_label(tokens.fg, RichText::new(text).font(typography::caption()));
        });
    })
}

/// `.empty-state` - the "nothing here yet" block every tool's Results/
/// output area shows before it has real content. Search's own results
/// column already had this exact text as a bare `ui.colored_label` call
/// before this component existed (see `search.rs::results_column`) -
/// componentized so Bushing/PV or a future tool can reuse the same
/// treatment (icon + title + subtitle) instead of a copy-pasted label.
pub fn empty_state(ui: &mut egui::Ui, tokens: &Tokens, icon: &str, title: &str, subtitle: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(18.0);
        ui.colored_label(tokens.fg_subtle, RichText::new(icon).size(28.0));
        ui.add_space(6.0);
        ui.colored_label(tokens.fg_muted, RichText::new(title).font(typography::h3()));
        ui.add_space(2.0);
        ui.colored_label(tokens.fg_subtle, RichText::new(subtitle).font(typography::body_small()));
        ui.add_space(18.0);
    });
}

/// Semantic intent for a `Toast` - drives color only.
#[derive(Clone, Copy)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast {
    kind: ToastKind,
    text: String,
    expires_at: std::time::Instant,
}

/// Transient notification queue - owned once by `ToolbenchApp`, rendered
/// every frame via `show`, pushed into by any tool that has a real
/// completion event worth surfacing (see `search.rs`'s index-build-
/// complete call site, the first real usage). Auto-expires each toast 4s
/// after it was pushed; `show` both draws and prunes, so callers never
/// need to remember to clean up.
#[derive(Default)]
pub struct ToastQueue {
    items: Vec<Toast>,
}

impl ToastQueue {
    pub fn push(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.items.push(Toast { kind, text: text.into(), expires_at: std::time::Instant::now() + std::time::Duration::from_secs(4) });
    }

    /// Draws every live toast bottom-right and prunes expired ones. Call
    /// once per frame from `ToolbenchApp::update` regardless of whether
    /// any toast is active - cheap (a `Vec::retain` over what's usually
    /// zero or one item) and it's the only place expiry is checked.
    pub fn show(&mut self, ctx: &egui::Context, tokens: &Tokens) {
        let now = std::time::Instant::now();
        self.items.retain(|t| t.expires_at > now);
        if self.items.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toast_area")).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0)).show(ctx, |ui| {
            for t in self.items.iter().rev() {
                let (bg, fg) = match t.kind {
                    ToastKind::Info => (tokens.bg_raised, tokens.fg),
                    ToastKind::Success => (tokens.good_bg, tokens.good),
                    ToastKind::Error => (tokens.danger_bg, tokens.danger),
                };
                egui::Frame::default().fill(bg).stroke(Stroke::new(1.0, tokens.border)).rounding(radii::lg()).inner_margin(12.0).shadow(shadows::overlay()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    ui.colored_label(fg, &t.text);
                });
                ui.add_space(6.0);
            }
        });
        // A toast expiring is a timer event, not an input event - without
        // this, the app would only repaint on the next real input and a
        // "done" toast could sit onscreen indefinitely past its 4s life.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
