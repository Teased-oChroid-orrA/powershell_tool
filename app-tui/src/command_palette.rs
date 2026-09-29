//! Global command palette. Concept ported from both existing GUI heads'
//! own `command_palette.rs` (a plain `Command` enum + fuzzy-filtered list,
//! toggled by a global keychord regardless of focus) - only the rendering
//! backend differs. `app-egui/`'s own palette is an explicitly documented
//! "scoped subset" missing several commands `app/`'s has (OpenReport,
//! BrowseOutputFolder-equivalent, ClearRecentSearches); this palette
//! starts with the fuller set from day one.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SwitchToSearch,
    SwitchToFastenerHole,
    SwitchToBushing,
    SwitchToPressureVessel,
    SwitchToPreloadAnalysis,
    SwitchToDupes,
    SwitchToRename,
    SwitchToLogs,
    RunSearch,
    CancelSearch,
    ToggleTheme,
    OpenReport,
    /// TUI-adapted equivalent of a native "browse for folder" dialog -
    /// there is no OS file picker in a terminal, so the palette instead
    /// jumps focus to the path field for the user to type/paste into.
    FocusPathField,
    ClearRecentSearches,
    ToggleFastReSearchIndex,
    BuildIndex,
    RebuildIndex,
    Quit,
}

impl Command {
    pub const ALL: &'static [Command] = &[
        Command::SwitchToSearch,
        Command::SwitchToFastenerHole,
        Command::SwitchToBushing,
        Command::SwitchToPressureVessel,
        Command::SwitchToPreloadAnalysis,
        Command::SwitchToDupes,
        Command::SwitchToRename,
        Command::SwitchToLogs,
        Command::RunSearch,
        Command::CancelSearch,
        Command::ToggleTheme,
        Command::OpenReport,
        Command::FocusPathField,
        Command::ClearRecentSearches,
        Command::ToggleFastReSearchIndex,
        Command::BuildIndex,
        Command::RebuildIndex,
        Command::Quit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Command::SwitchToSearch => "Switch to: Search Files",
            Command::SwitchToFastenerHole => "Switch to: Fastener Holes",
            Command::SwitchToBushing => "Switch to: Bushing Workbench",
            Command::SwitchToPressureVessel => "Switch to: Pressure Vessel Analyzer",
            Command::SwitchToPreloadAnalysis => "Switch to: Preload Analysis",
            Command::SwitchToDupes => "Switch to: Duplicate Finder (soon)",
            Command::SwitchToRename => "Switch to: Batch Rename (soon)",
            Command::SwitchToLogs => "Switch to: Log Analyzer (soon)",
            Command::RunSearch => "Run search",
            Command::CancelSearch => "Cancel running search",
            Command::ToggleTheme => "Toggle reduced-color theme",
            Command::OpenReport => "Open last HTML report",
            Command::FocusPathField => "Focus search path field",
            Command::ClearRecentSearches => "Clear recent searches",
            Command::ToggleFastReSearchIndex => "Toggle fast re-search index",
            Command::BuildIndex => "Build fast re-search index",
            Command::RebuildIndex => "Rebuild fast re-search index from scratch",
            Command::Quit => "Quit",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommandPalette {
    pub query: String,
    pub selected: usize,
}

impl CommandPalette {
    pub fn matches(&self) -> Vec<Command> {
        let q = self.query.to_lowercase();
        Command::ALL
            .iter()
            .copied()
            .filter(|c| q.is_empty() || c.label().to_lowercase().contains(&q))
            .collect()
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: i32) {
        let len = self.matches().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected as i32;
        let next = (current + delta).rem_euclid(len as i32);
        self.selected = next as usize;
    }

    /// The command currently highlighted, if any commands match the query.
    pub fn picked(&self) -> Option<Command> {
        self.matches().get(self.selected).copied()
    }
}

/// A centered `percent_x` x `percent_y` sub-rect of `area` - the standard
/// ratatui pattern for a floating overlay (no built-in helper for this in
/// ratatui itself).
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, palette: &CommandPalette, regions: &mut crate::mouse::MouseRegions) {
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(true))
        .title(" Commands ");
    let input = Paragraph::new(Line::from(vec![
        Span::styled("> ", theme.title_style(true)),
        Span::raw(palette.query.as_str()),
    ]))
    .block(input_block);
    frame.render_widget(input, layout[0]);

    let matches = palette.matches();
    let items: Vec<ListItem> = if matches.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No matching commands",
            theme.disabled_style(),
        )))]
    } else {
        matches
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let selected = i == palette.selected;
                let marker = if selected { "> " } else { "  " };
                let style = if selected {
                    theme.selected_row_style()
                } else {
                    Style::default().fg(theme.fg)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{marker}{}", cmd.label()),
                    style.add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
                )))
            })
            .collect()
    };
    let list_block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true));
    let list_inner = list_block.inner(layout[1]);
    frame.render_widget(list_block, layout[1]);
    // A stateless `List::new(items).block(...)` (as this used to be) never
    // scrolls to keep `palette.selected` visible - same bug class as
    // `widgets/scroll_list.rs`'s own doc comment describes for every other
    // selectable list in this crate; a long enough command list (this one
    // grows every time a new toolbox/command is added) could select past
    // the popup's fixed height with no visible highlighted row. `None`
    // when there are no matches - nothing to keep in view.
    let selected = if matches.is_empty() { None } else { Some(palette.selected) };
    let offset = crate::widgets::scroll_list::render(frame, list_inner, items, selected);
    if !matches.is_empty() {
        regions.palette_rows.extend(crate::mouse::list_row_regions(list_inner, offset, matches.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn selecting_the_last_command_scrolls_it_into_view() {
        // Regression guard: this popup's list used to render via
        // `List::new(items).block(...)` directly (ratatui's stateless
        // path, which never scrolls) - the same bug class fixed elsewhere
        // in this crate via `widgets/scroll_list.rs`. A small terminal
        // can't fit all of `Command::ALL` in the popup at once, so
        // selecting the last command must still make it visible somewhere
        // in the rendered buffer, not just update `selected` invisibly.
        let mut palette = CommandPalette::default();
        let last = Command::ALL.len() - 1;
        palette.selected = last;

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &palette, &mut regions)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let rendered: String =
            (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect::<Vec<_>>().join("\n");
        let last_label = Command::ALL[last].label();
        assert!(rendered.contains(last_label), "selected last command must be scrolled into view:\n{rendered}");
    }

    #[test]
    fn render_does_not_panic_at_degenerate_sizes() {
        let palette = CommandPalette::default();
        for (w, h) in [(0, 0), (1, 1), (40, 0), (0, 10)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, w, h);
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, area, &Theme::default_palette(), &palette, &mut regions)).unwrap();
        }
    }

    #[test]
    fn empty_query_matches_every_command() {
        let palette = CommandPalette::default();
        assert_eq!(palette.matches().len(), Command::ALL.len());
    }

    #[test]
    fn query_filters_by_label_substring() {
        let mut palette = CommandPalette::default();
        "cancel".chars().for_each(|c| palette.push_char(c));
        assert_eq!(palette.matches(), vec![Command::CancelSearch]);
    }

    #[test]
    fn move_selection_wraps_both_directions() {
        let mut palette = CommandPalette::default();
        "switch to".chars().for_each(|c| palette.push_char(c));
        let match_count = palette.matches().len();
        assert_eq!(match_count, 8, "one \"Switch to: ...\" entry per ToolId variant");
        palette.move_selection(-1);
        assert_eq!(palette.selected, match_count - 1);
        palette.move_selection(1);
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn picked_reflects_current_selection() {
        let mut palette = CommandPalette::default();
        "quit".chars().for_each(|c| palette.push_char(c));
        assert_eq!(palette.picked(), Some(Command::Quit));
    }

    #[test]
    fn picked_is_none_when_nothing_matches() {
        let mut palette = CommandPalette::default();
        "zzz-no-such-command".chars().for_each(|c| palette.push_char(c));
        assert_eq!(palette.picked(), None);
    }
}
