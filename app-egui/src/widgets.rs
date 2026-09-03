//! Shared chrome widgets ported 1:1 from the approved mockup artifact
//! (conversation record - `.step-pill`/`.nav-item`/`.card`/`.headline` in
//! the mockup's CSS) - used identically by every tool so the whole app
//! reads as one consistent UI/UX, not three differently-styled screens.
//! No deviations from the mockup's shapes/spacing without a real reason
//! (egui-specific rendering constraint) - see each function's own comment
//! where one applies.

use eframe::egui::{self, Color32, RichText, Sense, Stroke};

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
    egui::Frame::default().fill(tokens.bg_raised).stroke(Stroke::new(1.0, tokens.border)).rounding(8.0).inner_margin(16.0).show(ui, |ui| {
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
    ui.label(RichText::new(text).size(15.0).strong());
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
    ui.painter().rect_filled(rect, 6.0, bg);
    if active {
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, tokens.accent.gamma_multiply(0.5)));
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
