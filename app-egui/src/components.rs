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

use eframe::egui::{self, Color32};

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

/// Fixed tile width - a real, screenshot-confirmed bug, not a cosmetic
/// nit: this used to be `ui.set_min_width(170.0)` (a floor, not a cap),
/// and the progress-bar rect below sizes itself off `ui.available_width()`.
/// That's a problem inside `checks_tiles`'s `egui::Grid`, nested in
/// `card()` (which itself forces its content `Ui`'s `min_width` to the
/// FULL card width so an empty card doesn't collapse, per `widgets::card`'s
/// own doc comment): `available_width()` there reports the *entire
/// card's* width, not a sane per-tile share of it. Every tile stretched
/// to fill nearly the whole card, wrapping into a "list of wide rows"
/// instead of a tile grid and overflowing past the window's own edge
/// onto the desktop (a real screenshot showed exactly this, and
/// separately showed the persistent status rail entirely missing - it
/// was being pushed off/painted over by the oversized tiles, not failing
/// to render at all). A fixed `set_width`, not `set_min_width`, caps the
/// tile's own `Ui`, so everything inside it that reads `available_width()`
/// gets the real tile width back, not whatever its ancestors report.
const TILE_WIDTH: f32 = 170.0;

fn chip_text_bg_fg(tokens: &Tokens, margin: f64) -> (&'static str, Color32, Color32) {
    if !margin.is_finite() {
        ("\u{2014}", tokens.bg, tokens.fg_subtle)
    } else if margin < 0.0 {
        ("Fail", tokens.danger_bg, tokens.danger)
    } else if margin < 0.15 {
        ("Watch", tokens.warning_bg, tokens.warning)
    } else {
        ("Pass", tokens.good_bg, tokens.good)
    }
}

const CHIP_PAD: egui::Vec2 = egui::vec2(7.0, 2.0);

/// Measures the pill's total width WITHOUT painting - callers reserve
/// room for it (e.g. truncating a name that would otherwise run under
/// it) before calling [`chip_at`] to actually draw.
fn chip_width(ctx: &egui::Context, tokens: &Tokens, margin: f64) -> f32 {
    let (text, _, fg) = chip_text_bg_fg(tokens, margin);
    let galley = ctx.fonts(|f| f.layout_no_wrap(text.to_string(), egui::FontId::proportional(9.0), fg));
    galley.size().x + CHIP_PAD.x * 2.0
}

/// Right-aligned pill ending at `right_x`, vertically centered on `cy`.
/// Draws its own background, sized to the actual measured text (via
/// `Fonts::layout_no_wrap`) - the same measure-then-paint approach
/// `ladder()` already uses successfully, not `ui.label`/`Frame` widget
/// calls.
fn chip_at(painter: &egui::Painter, ctx: &egui::Context, tokens: &Tokens, margin: f64, right_x: f32, cy: f32) {
    let (text, bg, fg) = chip_text_bg_fg(tokens, margin);
    let galley = ctx.fonts(|f| f.layout_no_wrap(text.to_string(), egui::FontId::proportional(9.0), fg));
    let size = galley.size() + CHIP_PAD * 2.0;
    let rect = egui::Rect::from_min_size(egui::pos2(right_x - size.x, cy - size.y / 2.0), size);
    painter.rect_filled(rect, 10.0, bg);
    painter.galley(rect.min + CHIP_PAD, galley, fg);
}

/// Shortens `text` with a trailing "…" until it fits `max_width` at
/// `font`, measuring via the same `Fonts::layout_no_wrap` `chip_at` uses
/// (memoized, so repeated calls across frames are cheap) - a real fix,
/// not a guess: the tile name and its right-aligned chip previously both
/// painted unconditionally at full width with nothing reserving room for
/// the other, so a longer check name visibly ran under/over the chip in
/// a real screenshot.
fn truncate_to_width(ctx: &egui::Context, text: &str, font: egui::FontId, max_width: f32) -> String {
    let measure = |s: &str| ctx.fonts(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)).size().x;
    if measure(text) <= max_width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
        if measure(&candidate) <= max_width {
            return candidate;
        }
    }
    "\u{2026}".to_string()
}

fn fmt_margin(margin: f64) -> String {
    if margin.is_infinite() {
        "\u{2014}".to_string()
    } else {
        format!("{margin:+.2}")
    }
}

/// Fixed content height, matching the fixed layout offsets below -
/// header row, big margin number, fill bar, applied/limit row, each at
/// an explicit y from the tile's top.
const TILE_HEIGHT: f32 = 84.0;

/// Every element painted directly via `ui.painter_at(rect)` at explicit,
/// pre-computed y-offsets within ONE `allocate_exact_size`'d rect - not a
/// sequence of `ui.label`/`ui.horizontal`/`with_layout` widget calls.
///
/// A real, screenshot-confirmed bug forced this rewrite, not a style
/// preference: this tile used to be built from ordinary sequential widget
/// calls (a `ui.horizontal` name+chip row, then `ui.label` for the big
/// number, then a manually-painted bar, then another row). In an actual
/// running build - not just `cargo check` - the name+chip row, the
/// number, and the applied/limit row all painted overlapping each other
/// at nearly the same y-coordinate, inside `checks_tiles`' `egui::Grid`
/// nested in `card()`. Multiple attempts to fix the automatic-layout
/// version (capping widths, splitting each row into explicit `child_ui`s
/// over a pre-allocated rect) each fixed one overlap while leaving
/// another, which is itself the signal that automatic cursor-based
/// layout inside this specific nesting (Grid cell inside a `card()`-
/// forced-width Frame) isn't reliably advancing the way `ui.label` et al.
/// assume on this renderer/version. `ladder()` never had this problem
/// because it was ALREADY built this way - one allocation, everything
/// else placed at explicit offsets read back off that one rect. Matching
/// that proven approach here instead of continuing to patch the
/// automatic-layout version.
fn tile(ui: &mut egui::Ui, tokens: &Tokens, r: &CheckRow) {
    egui::Frame::default()
        .fill(tokens.bg_sunken)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .rounding(10.0)
        .inner_margin(12.0)
        .shadow(crate::design::shadows::raised())
        .show(ui, |ui| {
            ui.set_width(TILE_WIDTH);
            let content_width = TILE_WIDTH - 24.0; // minus inner_margin(12.0) both sides
            let (rect, _) = ui.allocate_exact_size(egui::vec2(content_width, TILE_HEIGHT), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            let ctx = ui.ctx().clone();
            let color = margin_color(tokens, r.margin);

            let header_cy = rect.top() + 9.0;
            let header_font = egui::FontId::proportional(12.5);
            // Reserve room for the chip BEFORE drawing the name - a real
            // screenshot showed a longer check name running straight
            // under/through the chip when both painted at full width
            // with nothing to stop them overlapping.
            let chip_w = chip_width(&ctx, tokens, r.margin);
            let name_budget = (content_width - chip_w - 6.0).max(0.0);
            let name = truncate_to_width(&ctx, r.name, header_font.clone(), name_budget);
            painter.text(egui::pos2(rect.left(), header_cy), egui::Align2::LEFT_CENTER, name, header_font, tokens.fg_muted);
            chip_at(&painter, &ctx, tokens, r.margin, rect.right(), header_cy);

            let number_cy = rect.top() + 34.0;
            painter.text(
                egui::pos2(rect.left(), number_cy),
                egui::Align2::LEFT_CENTER,
                fmt_margin(r.margin),
                egui::FontId::monospace(22.0),
                color,
            );

            // `applied`/`allowable` are both a real, honest `0.0` sentinel
            // for check kinds that only ever carry a margin (Bushing's
            // `bushing_solver::solve::MarginCandidate` has no stress
            // fields at all - see that struct). Showing "0 / limit 0" and
            // an always-empty bar for those would look like real
            // (wrong) data, not "not applicable" - so this row and the
            // bar only draw when there's something real to show.
            if r.applied != 0.0 || r.allowable != 0.0 {
                let bar_top = rect.top() + 54.0;
                let bar_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), bar_top), egui::vec2(content_width, 5.0));
                painter.rect_filled(bar_rect, 3.0, tokens.bg);
                let frac = if r.allowable != 0.0 { (r.applied.abs() / r.allowable.abs()).clamp(0.0, 1.0) } else { 0.0 };
                let mut filled = bar_rect;
                filled.set_width(bar_rect.width() * frac as f32);
                painter.rect_filled(filled, 3.0, color);
            }

            if r.applied != 0.0 || r.allowable != 0.0 {
                let applied_cy = rect.top() + 72.0;
                painter.text(
                    egui::pos2(rect.left(), applied_cy),
                    egui::Align2::LEFT_CENTER,
                    format!("{:.0} {}", r.applied, r.unit),
                    egui::FontId::proportional(10.5),
                    tokens.fg_subtle,
                );
                painter.text(
                    egui::pos2(rect.right(), applied_cy),
                    egui::Align2::RIGHT_CENTER,
                    format!("limit {:.0}", r.allowable),
                    egui::FontId::proportional(10.5),
                    tokens.fg_subtle,
                );
            }
        });
}

/// Status rail: PASS/REVIEW header + Ladder/Center-spine. Persistent -
/// callers render this as a fixed-width sibling of the step content on
/// every step, not just Results (see `widgets::side_by_side`), matching
/// the original `DesignStatusRail`/`PvStatusRail`'s "visible regardless
/// of which step is open" behavior (`app/src/main.rs`'s
/// `.bushing-status-rail`).
pub fn status_rail(ui: &mut egui::Ui, tokens: &Tokens, rows: &[CheckRow]) {
    let passed = rows.iter().filter(|r| r.margin.is_finite() && r.margin >= 0.0).count();
    let all_pass = passed == rows.len();
    egui::Frame::default()
        .fill(tokens.bg_raised)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .rounding(8.0)
        .inner_margin(12.0)
        .shadow(crate::design::shadows::raised())
        .show(ui, |ui| {
            ui.set_width(208.0); // 232px rail - 2*12px inner_margin; `set_width`, not `set_min_width` - see `tile`'s doc comment for why a floor isn't enough here either
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
            // Bounded, matching the original's `max-height:100%;
            // overflow-y:auto` - `ladder()`'s height grows unbounded with
            // check count (`26px * n`), and without a cap it clashed with
            // whatever rendered below/beside it once a config had enough
            // checks to push the rail taller than its column.
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                ladder(ui, tokens, rows);
            });
        });
}

/// Ladder/center-spine. Pulled directly from the approved artifact's own
/// CSS (fetched and read, not guessed from a screenshot crop) - this
/// project's `egui-shell-preview` artifact's `.r5a-*`/`.status-rail`
/// rules:
/// ```css
/// .status-rail { width: 232px; ... }
/// .r5a-spine { background: linear-gradient(to top, var(--danger), var(--border-strong) 15%, var(--border-strong) 85%, var(--good)); }
/// .r5a-dot { width: 8px; height: 8px; box-shadow: 0 0 0 3px var(--panel); }
/// .r5a-tick-r/-l { width: 14px; height: 1px; }
/// .r5a-label-r/-l { left/right: calc(50% + 18px); font-size: 10.5px; }
/// .r5a-label-r b, .r5a-label-l b { font-family: mono; display: block; }
/// .r5a-tag { font-size: 8.5px; uppercase; color: var(--fg-subtle); }
/// ```
/// `<b>` as `display: block` with a sibling name text after it means the
/// label is genuinely TWO LINES (bold mono value, then the name below) -
/// not "value  name" side by side on one line, which an earlier pass
/// guessed from a cropped screenshot where tight `line-height:1.2` made
/// them look close together. The spine's color comes entirely from its
/// gradient (danger at the very bottom, good at the very top) - there's
/// no separate flat-color end-tick in the source, so this doesn't draw
/// one either. `white-space: nowrap` on the label classes means the
/// mockup's own short illustrative names ("Yield", "Ultimate") were never
/// truncated - but this app's REAL check names (Bushing's "Edge distance
/// (sequencing)") are longer than anything the mockup exercised, so
/// truncating (`truncate_to_width`, same helper `tile()` uses) is a
/// necessary addition for real data, not a deviation from the design.
fn ladder(ui: &mut egui::Ui, tokens: &Tokens, rows: &[CheckRow]) {
    let n = rows.len().max(1);
    let row_h = 34.0; // two text lines + spacing, not one
    let axis_label_h = 14.0;
    let height = row_h * n as f32 + 20.0 + axis_label_h * 2.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let ctx = ui.ctx().clone();
    let cx = rect.center().x;
    let top = rect.top() + 6.0 + axis_label_h;
    let bottom = rect.bottom() - 6.0 - axis_label_h;

    let axis_font = egui::FontId::proportional(8.5);
    painter.text(egui::pos2(cx, top - 10.0), egui::Align2::CENTER_BOTTOM, "SAFE", axis_font.clone(), tokens.fg_subtle);
    painter.text(egui::pos2(cx, bottom + 10.0), egui::Align2::CENTER_TOP, "LIMIT", axis_font, tokens.fg_subtle);

    // `linear-gradient(to top, danger, border-strong 15%, border-strong
    // 85%, good)` - approximated with short flat segments since egui has
    // no gradient-stroke primitive; dense enough (24 segments) to read as
    // smooth, matching the CSS breakpoints exactly.
    const SEGMENTS: i32 = 24;
    for i in 0..SEGMENTS {
        let t0 = i as f32 / SEGMENTS as f32;
        let t1 = (i + 1) as f32 / SEGMENTS as f32;
        let mix_at = |t: f32| -> egui::Color32 {
            if t <= 0.15 {
                lerp_color(tokens.danger, tokens.border_strong, t / 0.15)
            } else if t >= 0.85 {
                lerp_color(tokens.border_strong, tokens.good, (t - 0.85) / 0.15)
            } else {
                tokens.border_strong
            }
        };
        // t=0 is the bottom of the gradient (danger) per `to top`.
        let y0 = bottom - (bottom - top) * t0;
        let y1 = bottom - (bottom - top) * t1;
        painter.line_segment([egui::pos2(cx, y0), egui::pos2(cx, y1)], egui::Stroke::new(2.0, mix_at((t0 + t1) / 2.0)));
    }

    // Rank-evenly-spaced (not raw-value-positioned) - the mockup's first
    // pass plotted nodes at their literal margin fraction, which visibly
    // collided whenever two checks had close margins. Ranking by margin
    // and spacing evenly by rank guarantees no overlap regardless of how
    // close the real values are; the number itself still carries the
    // exact magnitude.
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| rows[b].margin.partial_cmp(&rows[a].margin).unwrap_or(std::cmp::Ordering::Equal));

    let tick_len = 14.0; // `.r5a-tick-r/-l { width: 14px }`
    let label_gap = 18.0; // `.r5a-label-r { left: calc(50% + 18px) }`
    let half_budget = (rect.width() / 2.0 - label_gap - 4.0).max(20.0);
    let value_font = egui::FontId::monospace(11.0);
    let name_font = egui::FontId::proportional(10.5);
    let line_gap = 2.0;

    for (rank, &idx) in order.iter().enumerate() {
        let r = &rows[idx];
        let y = top + (bottom - top) * (rank as f32 / (n.max(2) - 1) as f32).max(0.0);
        let color = margin_color(tokens, r.margin);
        painter.circle_filled(egui::pos2(cx, y), 4.0, color);
        let right_side = rank % 2 == 0;
        let tick_end_x = if right_side { cx + tick_len } else { cx - tick_len };
        painter.line_segment([egui::pos2(cx, y), egui::pos2(tick_end_x, y)], egui::Stroke::new(1.0, tokens.border_strong));

        let value_text = fmt_margin(r.margin);
        let name = truncate_to_width(&ctx, r.name, name_font.clone(), half_budget);
        let label_x = if right_side { cx + label_gap } else { cx - label_gap };
        let align_top = if right_side { egui::Align2::LEFT_TOP } else { egui::Align2::RIGHT_TOP };
        let value_h = ctx.fonts(|f| f.row_height(&value_font));
        painter.text(egui::pos2(label_x, y - value_h - line_gap / 2.0), align_top, &value_text, value_font.clone(), color);
        painter.text(egui::pos2(label_x, y + line_gap / 2.0), align_top, &name, name_font.clone(), tokens.fg);

        let row_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), y - row_h / 2.0), egui::vec2(rect.width(), row_h));
        ui.interact(row_rect, ui.id().with(("ladder_row", idx)), egui::Sense::hover()).on_hover_text(format!("{value_text}  {}", r.name));
    }
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}
