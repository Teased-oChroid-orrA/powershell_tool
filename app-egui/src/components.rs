//! Shared UI pieces for both engineering tools (Bushing Workbench,
//! Pressure Vessel Analyzer) - egui port of `app/src/components.rs`.
//!
//! Design choices ported from the approved artifact mockup
//! (conversation record, not a file in this repo): Checks render as Stat
//! Tiles (a grid of cards: name, margin-of-safety, a thin fill bar, applied
//! vs. allowable), and the status rail renders as a Ladder/Center-spine -
//! a vertical axis with each check's margin plotted at a rank-evenly-
//! spaced position (not raw-value position, which the mockup's first pass
//! showed clashes badly when two margins are numerically close) with a
//! short tick back to its true position on the spine.

use eframe::egui::{self, Color32, RichText};

use crate::theme::Tokens;

pub struct CheckRow {
    pub name: &'static str,
    pub applied: f64,
    pub allowable: f64,
    pub margin: f64,
    pub unit: &'static str,
}

fn margin_color(tokens: &Tokens, margin: f64) -> Color32 {
    if !margin.is_finite() {
        tokens.fg_subtle
    } else if margin < 0.0 {
        tokens.danger
    } else if margin < 0.15 {
        tokens.warning
    } else {
        tokens.good
    }
}

/// `.tile-chip` - the small Pass/Watch/Fail pill in each tile's top-right
/// corner. A real gap found while wiring `good_bg`/`danger_bg` into
/// actual use: this chip existed in the approved mockup and was missing
/// entirely from the first working build.
fn status_chip(ui: &mut egui::Ui, tokens: &Tokens, margin: f64) {
    let (text, bg, fg) = if !margin.is_finite() {
        ("\u{2014}", tokens.bg, tokens.fg_subtle)
    } else if margin < 0.0 {
        ("Fail", tokens.danger_bg, tokens.danger)
    } else if margin < 0.15 {
        ("Watch", tokens.warning.gamma_multiply(0.18), tokens.warning)
    } else {
        ("Pass", tokens.good_bg, tokens.good)
    };
    egui::Frame::default().fill(bg).rounding(10.0).inner_margin(egui::Margin::symmetric(7.0, 2.0)).show(ui, |ui| {
        ui.colored_label(fg, RichText::new(text).size(9.0).strong());
    });
}

fn fmt_margin(margin: f64) -> String {
    if margin.is_infinite() {
        "\u{2014}".to_string()
    } else {
        format!("{margin:+.2}")
    }
}

/// Stat-tile Checks grid - the chosen design from the mockup review.
pub fn checks_tiles(ui: &mut egui::Ui, tokens: &Tokens, rows: &[CheckRow]) {
    let cols = 3usize.min(rows.len().max(1));
    egui::Grid::new("checks_tiles").num_columns(cols).spacing([10.0, 10.0]).show(ui, |ui| {
        for (i, r) in rows.iter().enumerate() {
            tile(ui, tokens, r);
            if (i + 1) % cols == 0 {
                ui.end_row();
            }
        }
    });
}

fn tile(ui: &mut egui::Ui, tokens: &Tokens, r: &CheckRow) {
    egui::Frame::default()
        .fill(tokens.bg_sunken)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .rounding(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(170.0);
            let color = margin_color(tokens, r.margin);
            ui.horizontal(|ui| {
                ui.colored_label(tokens.fg_muted, r.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    status_chip(ui, tokens, r.margin);
                });
            });
            ui.label(RichText::new(fmt_margin(r.margin)).monospace().size(22.0).color(color).strong());
            let frac = if r.allowable != 0.0 { (r.applied.abs() / r.allowable.abs()).clamp(0.0, 1.0) } else { 0.0 };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 5.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, tokens.bg);
            let mut filled = rect;
            filled.set_width(rect.width() * frac as f32);
            ui.painter().rect_filled(filled, 3.0, color);
            ui.horizontal(|ui| {
                ui.colored_label(tokens.fg_subtle, format!("{:.0} {}", r.applied, r.unit));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(tokens.fg_subtle, format!("limit {:.0}", r.allowable));
                });
            });
        });
}

/// Status rail: PASS/REVIEW header + Ladder/Center-spine.
pub fn status_rail(ui: &mut egui::Ui, tokens: &Tokens, rows: &[CheckRow]) {
    let passed = rows.iter().filter(|r| r.margin.is_finite() && r.margin >= 0.0).count();
    let all_pass = passed == rows.len();
    egui::Frame::default()
        .fill(tokens.bg_raised)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .rounding(8.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(220.0);
            ui.horizontal(|ui| {
                let dot_color = if all_pass { tokens.good } else { tokens.warning };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, dot_color);
                ui.vertical(|ui| {
                    ui.strong(if all_pass { "PASS" } else { "REVIEW" });
                    ui.colored_label(tokens.fg_subtle, format!("{passed} / {} checks passed", rows.len()));
                });
            });
            ui.add_space(10.0);
            ladder(ui, tokens, rows);
        });
}

fn ladder(ui: &mut egui::Ui, tokens: &Tokens, rows: &[CheckRow]) {
    let n = rows.len().max(1);
    let height = 26.0 * n as f32 + 20.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    let cx = rect.center().x;
    let top = rect.top() + 10.0;
    let bottom = rect.bottom() - 10.0;
    painter.line_segment([egui::pos2(cx, top), egui::pos2(cx, bottom)], egui::Stroke::new(2.0, tokens.border_strong));

    // Rank-evenly-spaced (not raw-value-positioned) - the mockup's first
    // pass plotted nodes at their literal margin fraction, which visibly
    // collided whenever two checks had close margins. Ranking by margin
    // and spacing evenly by rank guarantees no overlap regardless of how
    // close the real values are; the number itself still carries the
    // exact magnitude.
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| rows[b].margin.partial_cmp(&rows[a].margin).unwrap_or(std::cmp::Ordering::Equal));

    for (rank, &idx) in order.iter().enumerate() {
        let r = &rows[idx];
        let y = top + (bottom - top) * (rank as f32 / (n.max(2) - 1) as f32).max(0.0);
        let color = margin_color(tokens, r.margin);
        painter.circle_filled(egui::pos2(cx, y), 4.5, color);
        let right_side = rank % 2 == 0;
        let tick_end_x = if right_side { cx + 14.0 } else { cx - 14.0 };
        painter.line_segment([egui::pos2(cx, y), egui::pos2(tick_end_x, y)], egui::Stroke::new(1.0, tokens.border_strong));
        let text = format!("{}  {}", fmt_margin(r.margin), r.name);
        let anchor = if right_side { egui::Align2::LEFT_CENTER } else { egui::Align2::RIGHT_CENTER };
        let label_x = if right_side { tick_end_x + 4.0 } else { tick_end_x - 4.0 };
        painter.text(egui::pos2(label_x, y), anchor, text, egui::FontId::monospace(10.5), tokens.fg_muted);
    }
}
