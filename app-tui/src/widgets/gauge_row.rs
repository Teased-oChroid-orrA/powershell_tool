//! Themed wrapper over `ratatui::widgets::Gauge`, used for search progress
//! (and reusable by any future toolbox's own long-running progress). The
//! caller decides run state (running/success/warning/danger); this widget
//! only renders it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Gauge;

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaugeTone {
    Running,
    Success,
    Warning,
    Danger,
}

fn tone_color(theme: &Theme, tone: GaugeTone) -> Color {
    match tone {
        GaugeTone::Running => theme.accent,
        GaugeTone::Success => theme.success,
        GaugeTone::Warning => theme.warning,
        GaugeTone::Danger => theme.danger,
    }
}

/// Converts an arbitrary `percent` into the `0.0..=1.0` ratio
/// `ratatui::widgets::Gauge::ratio` requires, clamping out-of-range input
/// rather than panicking - `Gauge::ratio` itself panics (via an internal
/// assert) on a value outside `0.0..=1.0`, and progress data arriving from
/// a background task should never be trusted to already be in range.
fn ratio(percent: f64) -> f64 {
    percent.clamp(0.0, 100.0) / 100.0
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, percent: f64, label: &str, tone: GaugeTone) {
    let gauge = Gauge::default()
        .ratio(ratio(percent))
        .label(label.to_string())
        .gauge_style(Style::default().fg(tone_color(theme, tone)));
    frame.render_widget(gauge, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn ratio_clamps_below_zero_and_above_hundred() {
        assert_eq!(ratio(-5.0), 0.0);
        assert_eq!(ratio(150.0), 1.0);
        assert_eq!(ratio(54.0), 0.54);
    }

    #[test]
    fn tone_color_maps_to_expected_theme_field() {
        let theme = Theme::default_palette();
        assert_eq!(tone_color(&theme, GaugeTone::Running), theme.accent);
        assert_eq!(tone_color(&theme, GaugeTone::Success), theme.success);
        assert_eq!(tone_color(&theme, GaugeTone::Warning), theme.warning);
        assert_eq!(tone_color(&theme, GaugeTone::Danger), theme.danger);
    }

    #[test]
    fn renders_without_panicking_for_out_of_range_percent() {
        let backend = TestBackend::new(30, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &Theme::default_palette(), 999.0, "over 100", GaugeTone::Danger))
            .unwrap();
        terminal
            .draw(|f| render(f, f.area(), &Theme::default_palette(), -20.0, "negative", GaugeTone::Warning))
            .unwrap();
    }
}
