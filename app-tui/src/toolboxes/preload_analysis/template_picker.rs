//! Joint-template window for Preload Analysis: pick one of the preset stack-
//! ups, or keep pressing "next random" until a generated joint looks like the
//! one you want, then apply it - the analysis is re-run immediately.
//!
//! Keys: Up/Down choose, Enter apply, `g`/Space next random (jumps to the
//! generator), `b` previous random, Esc close. Everything is also a mouse
//! target (`TemplateAction`): click a row, double-click to apply, buttons.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::theme::{StatusTone, Theme};

use super::joint_templates::{self, JointTemplate};
use super::model::PreloadModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateAction {
    Row(usize),
    Apply,
    Generate,
    Previous,
    Close,
}

#[derive(Debug, Clone, Default)]
pub struct TemplatePickerState {
    pub open: bool,
    /// `0..presets.len()` = a preset, `presets.len()` = the random generator.
    pub cursor: usize,
    /// Seeds of every candidate shown so far, newest last; empty until the
    /// first "next random".
    pub history: Vec<u64>,
    base_seed: u64,
}

fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl TemplatePickerState {
    /// Opens with a seed derived from the clock, so each session starts a
    /// different random sequence (tests use [`open_with_seed`](Self::open_with_seed)).
    pub fn open() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Self::open_with_seed(nanos)
    }

    pub fn open_with_seed(seed: u64) -> Self {
        Self { open: true, cursor: 0, history: Vec::new(), base_seed: seed }
    }

    pub fn random_index() -> usize {
        joint_templates::presets().len()
    }

    /// The template the cursor is on (`None` = random row with nothing generated yet).
    pub fn selected_template(&self) -> Option<JointTemplate> {
        let presets = joint_templates::presets();
        if self.cursor < presets.len() {
            presets.into_iter().nth(self.cursor)
        } else {
            self.history.last().map(|s| joint_templates::random(*s))
        }
    }

    pub fn generate(&mut self) {
        let next = mix(self.base_seed.wrapping_add(self.history.len() as u64 + 1));
        self.history.push(next);
        self.cursor = Self::random_index();
    }

    pub fn previous(&mut self) {
        if self.history.len() > 1 {
            self.history.pop();
        }
        self.cursor = Self::random_index();
    }

    /// Runs `action`; returns the applied template's name when one was applied
    /// (the window then closes).
    pub fn perform(&mut self, action: TemplateAction, model: &mut PreloadModel) -> Option<String> {
        match action {
            TemplateAction::Row(i) => self.cursor = i.min(Self::random_index()),
            TemplateAction::Generate => self.generate(),
            TemplateAction::Previous => self.previous(),
            TemplateAction::Close => self.open = false,
            TemplateAction::Apply => {
                if self.cursor == Self::random_index() && self.history.is_empty() {
                    self.generate();
                    return None;
                }
                if let Some(t) = self.selected_template() {
                    model.apply_template(&t);
                    self.open = false;
                    return Some(t.name);
                }
            }
        }
        None
    }
}

/// Keys while the window is open (modal). Returns the applied template name, if any.
pub fn handle_key(state: &mut TemplatePickerState, model: &mut PreloadModel, key: KeyEvent) -> (bool, Option<String>) {
    let n = TemplatePickerState::random_index() + 1;
    let applied = match key.code {
        KeyCode::Esc => state.perform(TemplateAction::Close, model),
        KeyCode::Up => {
            state.cursor = (state.cursor + n - 1) % n;
            None
        }
        KeyCode::Down => {
            state.cursor = (state.cursor + 1) % n;
            None
        }
        KeyCode::Enter | KeyCode::Char('a' | 'A') => state.perform(TemplateAction::Apply, model),
        KeyCode::Char('g' | 'G' | ' ') => state.perform(TemplateAction::Generate, model),
        KeyCode::Char('b' | 'B') => state.perform(TemplateAction::Previous, model),
        _ => None,
    };
    (true, applied)
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, state: &TemplatePickerState, model: &PreloadModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width < 50 || area.height < 14 {
        return;
    }
    let width = area.width.saturating_sub(4).min(112);
    let height = area.height.saturating_sub(2).min(34);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(true))
        .title(" Joint templates \u{b7} Up/Down choose \u{b7} Enter apply \u{b7} g next random \u{b7} b previous \u{b7} Esc close ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 6 || inner.width < 30 {
        return;
    }
    regions.template_window = Some(popup);

    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3), Constraint::Length(1)]).split(inner);
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(50.min(inner.width / 2)), Constraint::Length(2), Constraint::Min(10)]).split(rows[0]);

    // Left: preset list + generator row.
    let presets = joint_templates::presets();
    let mut items: Vec<ListItem> = presets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let sel = i == state.cursor;
            ListItem::new(Line::from(Span::styled(format!("{}{}. {}", if sel { "> " } else { "  " }, i + 1, t.name), if sel { theme.selected_row_style() } else { Style::default() })))
        })
        .collect();
    let rand_sel = state.cursor == presets.len();
    items.push(ListItem::new(Line::from(Span::styled(
        format!("{}\u{21bb} Random generator{}", if rand_sel { "> " } else { "  " }, if state.history.is_empty() { String::new() } else { format!(" (#{})", state.history.len()) }),
        if rand_sel { theme.selected_row_style().add_modifier(Modifier::BOLD) } else { theme.title_style(true) },
    ))));
    let offset = crate::widgets::scroll_list::render(frame, cols[0], items, Some(state.cursor));
    for (rect, i) in crate::mouse::list_row_regions(cols[0], offset, presets.len() + 1) {
        regions.template_actions.push((rect, TemplateAction::Row(i)));
    }

    // Right: preview of the selected template, analyzed on a scratch copy.
    let mut lines: Vec<Line> = Vec::new();
    match state.selected_template() {
        None => {
            lines.push(Line::from(Span::styled("Random generator", theme.title_style(true).add_modifier(Modifier::BOLD))));
            lines.push(Line::from("Builds a plausible bolted joint at random: 2-3 plates (sometimes a shim, doubler or fitting), washers on 0-2 sides, a bolt size, materials and friction."));
            lines.push(Line::from(""));
            lines.push(Line::from("Press g (or click Next random) until one you like appears, then Enter / Apply to analyze it. b steps back to the previous one."));
        }
        Some(t) => {
            lines.push(Line::from(Span::styled(t.name.clone(), theme.title_style(true).add_modifier(Modifier::BOLD))));
            if state.cursor == presets.len() {
                lines.push(Line::from(Span::styled(format!("Candidate #{} - seed {:016x}", state.history.len(), state.history.last().copied().unwrap_or(0)), theme.disabled_style())));
            }
            lines.push(Line::from(t.description.clone()));
            lines.push(Line::from(""));
            let mut scratch = model.clone();
            scratch.apply_template(&t);
            lines.push(Line::from(Span::styled(
                format!("Bolt {}   grip {:.3} in   friction {:.2}", scratch.matching_bolt().map(|b| b.designation).unwrap_or("custom"), scratch.members.iter().map(|m| m.thickness).sum::<f64>(), scratch.mu_thread),
                theme.title_style(false),
            )));
            for row in joint_templates::stack_diagram(&scratch) {
                let style = if row.is_washer { theme.disabled_style() } else { Style::default() };
                lines.push(Line::from(vec![Span::styled(row.bar, style), Span::raw("  "), Span::raw(row.caption)]));
            }
            lines.push(Line::from(""));
            match &scratch.output {
                Ok(sol) => {
                    lines.push(Line::from(format!("Analysis: preload {:.0} lbf at {:.0} in-lbf applied torque", sol.preload, sol.torque.applied)));
                    if !sol.warnings.is_empty() {
                        lines.push(Line::from(Span::styled(format!("{} warning(s) - see Results after applying", sol.warnings.len()), theme.status_style(StatusTone::Warning))));
                    }
                }
                Err(e) => lines.push(Line::from(Span::styled(format!("Cannot analyze: {e:?}"), theme.status_style(StatusTone::Danger)))),
            }
            lines.push(Line::from(Span::styled("Typical dimensions and torque (K=0.2, 50% of 125 ksi) - edit anything after applying.", theme.disabled_style())));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), cols[2]);

    // Buttons.
    let can_apply = state.cursor < presets.len() || !state.history.is_empty();
    let mut buttons: Vec<(&str, TemplateAction, Style)> = Vec::new();
    buttons.push((if can_apply { " Apply & analyze " } else { " Generate first " }, TemplateAction::Apply, theme.status_style(StatusTone::Success).add_modifier(Modifier::REVERSED | Modifier::BOLD)));
    buttons.push((" Next random (g) ", TemplateAction::Generate, Style::default().add_modifier(Modifier::REVERSED)));
    if state.history.len() > 1 {
        buttons.push((" Previous (b) ", TemplateAction::Previous, Style::default().add_modifier(Modifier::REVERSED)));
    }
    buttons.push((" Close ", TemplateAction::Close, Style::default().add_modifier(Modifier::REVERSED)));
    let mut spans = Vec::new();
    let mut x = rows[1].x;
    for (label, action, style) in buttons {
        let w = label.chars().count() as u16;
        if x + w > rows[1].x + rows[1].width {
            break;
        }
        regions.template_actions.push((Rect { x, y: rows[1].y, width: w, height: 1 }, action));
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("  "));
        x += w + 2;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn enter_applies_the_highlighted_preset_and_closes() {
        let mut model = PreloadModel::default();
        let mut st = TemplatePickerState::open_with_seed(1);
        let (consumed, applied) = handle_key(&mut st, &mut model, key(KeyCode::Enter));
        assert!(consumed && applied.is_some() && !st.open);
        assert_eq!(model.members.len(), 4);
    }

    #[test]
    fn g_keeps_generating_new_candidates_and_b_steps_back() {
        let mut model = PreloadModel::default();
        let mut st = TemplatePickerState::open_with_seed(99);
        handle_key(&mut st, &mut model, key(KeyCode::Char('g')));
        let first = st.selected_template().unwrap();
        handle_key(&mut st, &mut model, key(KeyCode::Char('G'))); // Caps Lock
        let second = st.selected_template().unwrap();
        assert_ne!(first.description, second.description, "each press yields a new joint");
        assert_eq!(st.cursor, TemplatePickerState::random_index());
        handle_key(&mut st, &mut model, key(KeyCode::Char('b')));
        assert_eq!(st.selected_template().unwrap(), first);
        // Applying the shown candidate analyzes exactly it.
        let (_, applied) = handle_key(&mut st, &mut model, key(KeyCode::Enter));
        assert_eq!(applied.as_deref(), Some(first.name.as_str()));
        assert!(model.output.is_ok());
    }

    #[test]
    fn apply_on_the_empty_generator_row_generates_instead_of_applying_nothing() {
        let mut model = PreloadModel::default();
        let before = model.members.len();
        let mut st = TemplatePickerState::open_with_seed(5);
        st.cursor = TemplatePickerState::random_index();
        assert!(st.selected_template().is_none());
        assert!(st.perform(TemplateAction::Apply, &mut model).is_none());
        assert!(st.open && st.selected_template().is_some());
        assert_eq!(model.members.len(), before);
    }

    #[test]
    fn up_down_wrap_and_esc_closes_without_changing_the_model() {
        let mut model = PreloadModel::default();
        let snapshot = model.members.clone();
        let mut st = TemplatePickerState::open_with_seed(1);
        handle_key(&mut st, &mut model, key(KeyCode::Up));
        assert_eq!(st.cursor, TemplatePickerState::random_index(), "wraps to the generator row");
        handle_key(&mut st, &mut model, key(KeyCode::Down));
        assert_eq!(st.cursor, 0);
        handle_key(&mut st, &mut model, key(KeyCode::Esc));
        assert!(!st.open);
        assert_eq!(model.members, snapshot);
    }

    #[test]
    fn render_shows_list_preview_diagram_and_buttons_and_publishes_regions() {
        let model = PreloadModel::default();
        let mut st = TemplatePickerState::open_with_seed(3);
        let backend = TestBackend::new(130, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &st, &model, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();
        for needle in ["Joint templates", "washers both sides", "Random generator", "bolt head", "nut", "Apply & analyze", "Analysis: preload"] {
            assert!(text.contains(needle), "missing `{needle}`:\n{text}");
        }
        assert!(regions.template_window.is_some());
        assert!(regions.template_actions.iter().any(|(_, a)| *a == TemplateAction::Row(0)));
        assert!(regions.template_actions.iter().any(|(_, a)| *a == TemplateAction::Apply));
        // Random row preview.
        st.generate();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &st, &model, &mut regions)).unwrap();
        assert!(regions.template_actions.iter().any(|(_, a)| *a == TemplateAction::Previous) || st.history.len() <= 1);
    }

    #[test]
    fn render_survives_degenerate_sizes() {
        let model = PreloadModel::default();
        let st = TemplatePickerState::open_with_seed(3);
        for (w, h) in [(0, 0), (30, 10), (60, 16), (200, 80)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &st, &model, &mut regions)).unwrap();
        }
    }
}
