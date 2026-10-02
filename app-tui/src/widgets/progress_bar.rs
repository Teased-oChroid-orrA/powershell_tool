//! Two-row progress panel: a smooth, gradient bar (1/8-cell precision) with a
//! state glyph and percentage, over one line of live stats that sheds its
//! least important segments instead of being cut off mid-word. Used for both
//! search progress and fast-index builds. Pure function of its inputs (the
//! caller owns timing and the animation `tick`) like the other widgets.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::{StatusTone, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarState {
    Running,
    Done,
    Failed,
    Idle,
}

pub struct ProgressView {
    /// `0.0..=100.0`; `None` = indeterminate (animated sweep).
    pub percent: Option<f64>,
    pub state: BarState,
    /// Stats segments, most important first; joined with " · " while they fit.
    pub segments: Vec<String>,
    pub tick: u64,
    /// Overrides the stats tone (e.g. an error message).
    pub tone: Option<StatusTone>,
}

const PARTIAL: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

/// `(full cells, partial glyph, empty cells)` for a bar `width` cells wide.
pub fn bar_cells(percent: f64, width: usize) -> (usize, Option<char>, usize) {
    let eighths = ((percent.clamp(0.0, 100.0) / 100.0) * (width * 8) as f64).round() as usize;
    let full = (eighths / 8).min(width);
    let rem = eighths % 8;
    let partial = (rem > 0 && full < width).then(|| PARTIAL[rem]);
    let empty = width - full - usize::from(partial.is_some());
    (full, partial, empty)
}

/// Joins `segments` with " · " keeping as many leading ones as fit in `width`.
pub fn fit_segments(segments: &[String], width: usize) -> String {
    let mut out = String::new();
    for seg in segments.iter().filter(|s| !s.is_empty()) {
        let extra = if out.is_empty() { seg.chars().count() } else { 3 + seg.chars().count() };
        if out.chars().count() + extra > width {
            break;
        }
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(seg);
    }
    out
}

fn gradient(theme: &Theme, t: f64) -> Color {
    if theme.reduced_color {
        return theme.accent;
    }
    // Blue -> teal -> green along the bar.
    let (a, b) = ((64.0, 156.0, 255.0), (72.0, 214.0, 140.0));
    let l = |x: f64, y: f64| (x + (y - x) * t.clamp(0.0, 1.0)).round() as u8;
    Color::Rgb(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, view: &ProgressView) {
    if area.height == 0 || area.width < 8 {
        return;
    }
    let (glyph, glyph_style) = match view.state {
        BarState::Running => {
            let g = if theme.reduced_color { crate::widgets::spinner::ascii_frame(view.tick) } else { crate::widgets::spinner::frame(view.tick) };
            (g, Style::default().fg(theme.accent))
        }
        BarState::Done => ('✔', Style::default().fg(theme.success)),
        BarState::Failed => ('✘', Style::default().fg(theme.danger)),
        BarState::Idle => ('•', theme.disabled_style()),
    };
    let label = match view.percent {
        Some(p) => format!(" {:>3.0}%", p.clamp(0.0, 100.0)),
        None => "  …  ".to_string(),
    };
    let width = (area.width as usize).saturating_sub(2 + label.chars().count()).max(1);
    let (fill, track) = if theme.reduced_color { ('#', '-') } else { ('█', '░') };

    let mut spans = vec![Span::styled(format!("{glyph} "), glyph_style)];
    match view.percent {
        Some(p) => {
            let (full, partial, empty) = bar_cells(p, width);
            let denom = width.max(1) as f64;
            for i in 0..full {
                let color = if view.state == BarState::Failed { theme.danger } else { gradient(theme, i as f64 / denom) };
                spans.push(Span::styled(fill.to_string(), Style::default().fg(color)));
            }
            if let Some(ch) = partial {
                let ch = if theme.reduced_color { '=' } else { ch };
                spans.push(Span::styled(ch.to_string(), Style::default().fg(gradient(theme, full as f64 / denom))));
            }
            spans.push(Span::styled(track.to_string().repeat(empty), Style::default().fg(theme.fg_subtle)));
        }
        None => {
            // Indeterminate: a bright block bouncing across the track.
            let block = (width / 5).clamp(2, 12).min(width);
            let span_len = width - block;
            let cycle = (span_len * 2).max(1);
            let t = (view.tick as usize * 2) % cycle;
            let pos = if t <= span_len { t } else { cycle - t };
            spans.push(Span::styled(track.to_string().repeat(pos), Style::default().fg(theme.fg_subtle)));
            for i in 0..block {
                spans.push(Span::styled(fill.to_string(), Style::default().fg(gradient(theme, i as f64 / block as f64))));
            }
            spans.push(Span::styled(track.to_string().repeat(span_len - pos), Style::default().fg(theme.fg_subtle)));
        }
    }
    spans.push(Span::styled(label, Style::default().add_modifier(Modifier::BOLD)));
    frame.render_widget(Paragraph::new(Line::from(spans)), Rect { height: 1, ..area });

    if area.height >= 2 {
        let text = fit_segments(&view.segments, area.width as usize - 2);
        let style = match view.tone {
            Some(tone) => theme.status_style(tone),
            None => theme.disabled_style(),
        };
        let row = Rect { y: area.y + 1, height: 1, ..area };
        frame.render_widget(Paragraph::new(Line::from(vec![Span::raw("  "), Span::styled(text, style)])), row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn bar_cells_use_eighth_cell_precision_and_always_sum_to_width() {
        assert_eq!(bar_cells(0.0, 10), (0, None, 10));
        assert_eq!(bar_cells(100.0, 10), (10, None, 0));
        // 25% of 10 cells = 2.5 cells -> 2 full + a half block.
        assert_eq!(bar_cells(25.0, 10), (2, Some('▌'), 7));
        for p in [0.0, 1.0, 33.3, 50.0, 99.9, 100.0, 150.0, -5.0] {
            for w in [1usize, 7, 40] {
                let (f, part, e) = bar_cells(p, w);
                assert_eq!(f + usize::from(part.is_some()) + e, w, "p={p} w={w}");
            }
        }
    }

    #[test]
    fn fit_segments_drops_the_least_important_tail_instead_of_cutting_mid_word() {
        let segs = vec!["119 of 3001 files".to_string(), "41.2 files/s".to_string(), "ETA 12s".to_string(), "1:02".to_string()];
        assert_eq!(fit_segments(&segs, 80), "119 of 3001 files · 41.2 files/s · ETA 12s · 1:02");
        assert_eq!(fit_segments(&segs, 34), "119 of 3001 files · 41.2 files/s");
        assert_eq!(fit_segments(&segs, 5), "");
    }

    fn draw(width: u16, view: &ProgressView, reduced: bool) -> String {
        let theme = if reduced { Theme::reduced_palette() } else { Theme::default_palette() };
        let mut t = Terminal::new(TestBackend::new(width, 2)).unwrap();
        t.draw(|f| render(f, f.area(), &theme, view)).unwrap();
        let buf = t.backend().buffer();
        (0..2).map(|y| (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn view(percent: Option<f64>, state: BarState, segs: &[&str]) -> ProgressView {
        ProgressView { percent, state, segments: segs.iter().map(|s| s.to_string()).collect(), tick: 3, tone: None }
    }

    #[test]
    fn renders_percent_bar_and_stats_in_both_palettes_at_all_widths() {
        for reduced in [false, true] {
            for w in [8u16, 20, 60, 140] {
                let out = draw(w, &view(Some(42.0), BarState::Running, &["a", "b"]), reduced);
                assert!(out.contains("42%") || w < 14, "{w}: {out}");
            }
        }
        let out = draw(60, &view(Some(100.0), BarState::Done, &["done"]), false);
        assert!(out.contains('✔') && out.contains("100%") && out.contains("done"), "{out}");
        let out = draw(60, &view(None, BarState::Running, &["Scanning"]), false);
        assert!(out.contains('…') && out.contains("Scanning"), "{out}");
    }

    #[test]
    fn degenerate_areas_do_not_panic() {
        draw(1, &view(Some(50.0), BarState::Running, &["x"]), false);
        draw(8, &view(None, BarState::Idle, &[]), true);
    }
}
