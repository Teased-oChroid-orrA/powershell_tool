//! Shared chrome widgets ported 1:1 from the approved mockup artifact
//! (conversation record - `.step-pill`/`.nav-item`/`.card`/`.headline` in
//! the mockup's CSS) - used identically by every tool so the whole app
//! reads as one consistent UI/UX, not three differently-styled screens.
//! No deviations from the mockup's shapes/spacing without a real reason
//! (egui-specific rendering constraint) - see each function's own comment
//! where one applies.

use eframe::egui::{self, Color32, RichText, Sense, Stroke};

use crate::design::{radii, shadows, typography};
use crate::theme::Tokens;

/// `.card` - the base panel every section in the mockup sits in:
/// `background:var(--panel); border:1px solid var(--border); border-
/// radius:8px; padding:16px`.
///
/// A real bug found from the first live screenshot of this app, not a
/// cosmetic nit: `egui::Frame` shrinks to its content's natural size by
/// default - a card with sparse content (nothing running yet, no
/// results yet) collapsed to a tiny orphaned box instead of filling its
/// column, because nothing was forcing it to. Fixed the standard egui
/// way: capture the available width *before* entering the frame, then
/// `set_min_width` on the frame's own inner `Ui` as the first thing
/// inside it - `Frame` paints its background to match its content Ui's
/// final size, so this makes every card reliably fill its column
/// regardless of how much content it happens to have this frame.
pub fn card(ui: &mut egui::Ui, tokens: &Tokens, add_contents: impl FnOnce(&mut egui::Ui)) {
    let full_width = ui.available_width();
    egui::Frame::default()
        .fill(tokens.bg_raised)
        .stroke(Stroke::new(1.0, tokens.border))
        .rounding(radii::lg())
        .inner_margin(16.0)
        .shadow(shadows::raised())
        .show(ui, |ui| {
            ui.set_min_width((full_width - 32.0).max(0.0));
            add_contents(ui);
        });
}

/// `.card-title` - `font-size:1em; font-weight:700` in the mockup, i.e.
/// barely bigger than body text. `ui.heading()` is NOT this - egui's
/// heading style is roughly 1.5-2x body size, which is what made the
/// first live screenshot's "Results"/card titles look oversized and the
/// cards around them look sparser than they are. Use this for every
/// card title instead of a raw `ui.heading()` call.
pub fn card_title(ui: &mut egui::Ui, text: &str) {
    ui.label(typography::card_title_text(text).strong());
}

/// `.step-pill` row - step-number circle, label, em-dash separators,
/// accent tint on the current step. Mirrors the mockup's `.stepper`/
/// `.step-pill`/`.step-num` exactly.
pub fn stepper<S: Copy + PartialEq>(ui: &mut egui::Ui, tokens: &Tokens, steps: &[(S, &str)], current: &mut S) {
    ui.horizontal(|ui| {
        for (i, (step, label)) in steps.iter().enumerate() {
            let is_current = *step == *current;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0 + 8.0 * label.len() as f32, 26.0), Sense::click());
            let bg = if is_current { tokens.accent.gamma_multiply(0.18) } else if resp.hovered() { tokens.border } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(rect, 13.0, bg);
            let num_center = rect.left_center() + egui::vec2(15.0, 0.0);
            let num_color = if is_current { tokens.accent } else { tokens.border_strong };
            ui.painter().circle_stroke(num_center, 8.0, Stroke::new(1.5, num_color));
            if is_current {
                ui.painter().circle_filled(num_center, 8.0, tokens.accent);
            }
            ui.painter().text(
                num_center,
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                egui::FontId::monospace(9.0),
                if is_current { tokens.accent_fg } else { tokens.fg_subtle },
            );
            let text_color = if is_current { tokens.accent_strong } else { tokens.fg_muted };
            ui.painter().text(
                num_center + egui::vec2(14.0, 0.0),
                egui::Align2::LEFT_CENTER,
                *label,
                egui::FontId::proportional(12.5),
                text_color,
            );
            if resp.clicked() {
                *current = *step;
            }
            if i + 1 < steps.len() {
                ui.colored_label(tokens.border_strong, "\u{2014}");
            }
        }
    });
}

/// Minimum width the flexible (left) column of `side_by_side` is ever
/// allowed to shrink to. A bare `.max(0.0)` on the leftover-space
/// subtraction stops a *crash*, but a squashed-to-zero left column is
/// still a clash, just an invisible one - this is a real floor, not a
/// safety-only clamp.
pub const MIN_FLEX_COL: f32 = 320.0;

/// Fixed-width right column beside a flexible left column - mirrors the
/// original app's `.bushing-status-rail { flex: none; width: 240px;
/// align-self: flex-start; }` (see `app/src/main.rs`) sitting beside its
/// flexible sibling. `right_width` is reserved FIRST (matching the
/// original CSS's "one side is `flex: none`, the other flexes" shape),
/// and the left column gets whatever's left, floored at `min_left`
/// rather than the bare `.max(0.0)` that let `bushing.rs`/
/// `pressure_vessel.rs` drive a negative width into `set_min_width` on
/// any window narrower than `right_width` - the two columns fought over
/// space instead of the left one simply shrinking.
pub fn side_by_side(
    ui: &mut egui::Ui,
    right_width: f32,
    min_left: f32,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    let spacing = 14.0;
    let left_width = (ui.available_width() - right_width - spacing).max(min_left);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            // `set_width`, NOT `set_min_width` - a real, screenshot-
            // confirmed bug: `set_min_width` is a FLOOR, it doesn't cap
            // what nested content's own `ui.available_width()` calls
            // report (that's governed by the ambient `max_rect`, which a
            // `horizontal_top` doesn't shrink to a sibling's declared
            // width, only to what that sibling actually painted). Content
            // inside `left(ui)` that measures its own available width
            // (e.g. `bushing.rs`/`pressure_vessel.rs`'s sketch pane) saw
            // the FULL window width, not this column's share of it,
            // pushing the sketch pane wide enough to leave zero room for
            // - and visually hide - the persistent status rail this
            // function's `right` closure renders. Same root cause as the
            // Stat Tile width blowout fixed in `components.rs::tile`.
            ui.set_width(left_width);
            left(ui);
        });
        ui.add_space(spacing);
        ui.vertical(|ui| {
            ui.set_width(right_width);
            right(ui);
        });
    });
}

/// `.nav-item` - icon glyph, two-line title+desc, active/hover states, an
/// optional "Soon" pill for not-yet-enabled tools.
pub fn nav_item(ui: &mut egui::Ui, tokens: &Tokens, icon: &str, title: &str, desc: &str, active: bool, enabled: bool) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 40.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if active {
        tokens.accent.gamma_multiply(0.16)
    } else if resp.hovered() && enabled {
        Color32::from_white_alpha(10)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radii::MD, bg);
    if active {
        ui.painter().rect_stroke(rect, radii::MD, Stroke::new(1.0, tokens.accent.gamma_multiply(0.5)));
    }
    let icon_color = if active { tokens.accent_strong } else { tokens.fg_muted };
    ui.painter().text(rect.left_center() + egui::vec2(14.0, 0.0), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(15.0), icon_color);
    let title_color = if !enabled { tokens.fg_subtle } else if active { tokens.fg } else { tokens.fg };
    ui.painter().text(
        rect.left_top() + egui::vec2(30.0, 8.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(13.5),
        title_color,
    );
    ui.painter().text(
        rect.left_top() + egui::vec2(30.0, 23.0),
        egui::Align2::LEFT_TOP,
        desc,
        egui::FontId::proportional(11.0),
        tokens.fg_subtle,
    );
    if !enabled {
        let pill_rect = egui::Rect::from_min_size(rect.right_top() + egui::vec2(-46.0, 6.0), egui::vec2(40.0, 15.0));
        ui.painter().rect_filled(pill_rect, 8.0, tokens.bg_sunken);
        ui.painter().text(pill_rect.center(), egui::Align2::CENTER_CENTER, "SOON", egui::FontId::proportional(8.0), tokens.fg_subtle);
    }
    enabled && resp.clicked()
}

/// `.field input, .field select { font-family: var(--mono) }` from the
/// approved mockup CSS - rounding/padding are set globally in
/// `theme.rs`/`main.rs` (every widget in the app benefits, not just
/// these), but the monospace override has to stay scoped here: applying
/// it globally would make every `ui.label` in the app monospace too,
/// not just number/text inputs. Scoped via `ui.scope` so it never leaks
/// into sibling widgets (a `Grid`'s cells all share one `Ui`, so an
/// unscoped style mutation here would bleed into every later cell,
/// including header labels).
fn style_modern_input(ui: &mut egui::Ui) {
    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
}

/// A bare styled number input, no label - for spec-table cells where the
/// column header already IS the label (see `bushing.rs`'s
/// `plain_spec_row`/`cs_spec_row`).
pub fn styled_number(ui: &mut egui::Ui, value: &mut f64, speed: f64, decimals: usize) {
    ui.scope(|ui| {
        style_modern_input(ui);
        ui.add(egui::DragValue::new(value).speed(speed).max_decimals(decimals));
    });
}

/// `.field` from the approved mockup: label ABOVE the input
/// (`flex-direction: column`), `.field label { font-size: 11.5px;
/// color: var(--fg-muted) }`. Every numeric field in this app used to
/// put its label INLINE beside the input instead
/// (`ui.horizontal(|ui| { ui.label(...); ui.add(...) })`) - a real,
/// user-reported "ugly"/misaligned look, not a subjective nitpick: two
/// adjacent fields with differently-sized labels put their input boxes
/// at two different x offsets, so a stacked list of fields never lined
/// up into a clean column. Stacking label-above-input fixes that by
/// construction - every input starts at the same x regardless of its
/// label's length.
pub fn num_field(ui: &mut egui::Ui, tokens: &Tokens, label: &str, value: &mut f64, speed: f64, decimals: usize) {
    ui.vertical(|ui| {
        ui.colored_label(tokens.fg_muted, RichText::new(label).size(11.5));
        ui.add_space(3.0);
        styled_number(ui, value, speed, decimals);
    });
    ui.add_space(7.0);
}

/// Same `.field` treatment as `num_field`, for a single-line text input -
/// not yet called anywhere (Search's own text fields already put their
/// label on its own line above the input, just not through this shared
/// helper yet), kept as the reusable primitive future text fields should
/// use rather than reinventing the pattern ad hoc.
#[allow(dead_code)]
pub fn text_field(ui: &mut egui::Ui, tokens: &Tokens, label: &str, value: &mut String, hint: &str) {
    ui.vertical(|ui| {
        ui.colored_label(tokens.fg_muted, RichText::new(label).size(11.5));
        ui.add_space(3.0);
        ui.scope(|ui| {
            style_modern_input(ui);
            ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
        });
    });
    ui.add_space(7.0);
}

/// Frozen headline row: status dot + PASS/REVIEW + mini-stats, exactly
/// the mockup's `.headline`/`.mini-stats` - a plain sibling of whatever
/// scrolls beneath it, never inside a scroll area, so it stays fixed
/// while the workspace scrolls (same requirement the mockup's own
/// review round called out by name).
pub fn headline(ui: &mut egui::Ui, tokens: &Tokens, all_pass: bool, passed: usize, total: usize, mini_stats: &[(&str, String, Option<Color32>)]) {
    card(ui, tokens, |ui| {
        ui.horizontal(|ui| {
            let status_color = if all_pass { tokens.good } else { tokens.warning };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.5, status_color);
            ui.vertical(|ui| {
                ui.label(RichText::new(if all_pass { "PASS" } else { "REVIEW" }).color(status_color).strong());
                ui.colored_label(tokens.fg_subtle, format!("{passed} / {total} checks passed"));
            });
            ui.add_space(18.0);
            for (label, value, color) in mini_stats {
                ui.separator();
                ui.vertical(|ui| {
                    ui.colored_label(tokens.fg_muted, RichText::new(*label).size(10.5));
                    ui.colored_label(color.unwrap_or(tokens.fg), value);
                });
            }
        });
    });
}
