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

use crate::nav::ToolId;
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
    CancelIndexBuild,
    /// Opens `toolbench-debug.log` (written next to where the app was launched).
    OpenDebugLog,
    /// Fastener Holes has no Numbers panel (see `app-tui/AGENTS.md`'s
    /// Pitfalls for why) - `e` export is its only palette-worthy action.
    ExportFastenerHoleReport,
    ToggleBushingNumbersPanel,
    ExportBushingReport,
    OpenReamerPicker,
    OpenHousingMaterialPicker,
    OpenBushingMaterialPicker,
    TogglePressureVesselNumbersPanel,
    ExportPressureVesselReport,
    OpenPressureVesselMaterialPicker,
    TogglePreloadAnalysisNumbersPanel,
    ExportPreloadAnalysisReport,
    OpenBoltPicker,
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
        Command::CancelIndexBuild,
        Command::OpenDebugLog,
        Command::ExportFastenerHoleReport,
        Command::ToggleBushingNumbersPanel,
        Command::ExportBushingReport,
        Command::OpenReamerPicker,
        Command::OpenHousingMaterialPicker,
        Command::OpenBushingMaterialPicker,
        Command::TogglePressureVesselNumbersPanel,
        Command::ExportPressureVesselReport,
        Command::OpenPressureVesselMaterialPicker,
        Command::TogglePreloadAnalysisNumbersPanel,
        Command::ExportPreloadAnalysisReport,
        Command::OpenBoltPicker,
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
            Command::CancelIndexBuild => "Stop running index build",
            Command::OpenDebugLog => "Open debug log (toolbench-debug.log)",
            Command::ExportFastenerHoleReport => "Export report",
            Command::ToggleBushingNumbersPanel => "Toggle Numbers panel",
            Command::ExportBushingReport => "Export report",
            Command::OpenReamerPicker => "Open reamer catalog (Bore Diameter)",
            Command::OpenHousingMaterialPicker => "Open Housing Material picker",
            Command::OpenBushingMaterialPicker => "Open Bushing Material picker",
            Command::TogglePressureVesselNumbersPanel => "Toggle Numbers panel",
            Command::ExportPressureVesselReport => "Export report",
            Command::OpenPressureVesselMaterialPicker => "Open Material picker",
            Command::TogglePreloadAnalysisNumbersPanel => "Toggle Numbers panel",
            Command::ExportPreloadAnalysisReport => "Export report",
            Command::OpenBoltPicker => "Open Bolt (AN Standard) catalog",
            Command::Quit => "Quit",
        }
    }

    /// Which toolbox this command is scoped to - `None` means always
    /// visible regardless of the active tool (global navigation/app-level
    /// actions), `Some(tool)` means it only appears in the palette while
    /// that toolbox is active. See `CommandPalette::matches`.
    pub fn scope(self) -> Option<ToolId> {
        match self {
            Command::SwitchToSearch
            | Command::SwitchToFastenerHole
            | Command::SwitchToBushing
            | Command::SwitchToPressureVessel
            | Command::SwitchToPreloadAnalysis
            | Command::SwitchToDupes
            | Command::SwitchToRename
            | Command::SwitchToLogs
            | Command::ToggleTheme
            | Command::OpenDebugLog
            | Command::Quit => None,
            Command::RunSearch
            | Command::CancelSearch
            | Command::OpenReport
            | Command::FocusPathField
            | Command::ClearRecentSearches
            | Command::ToggleFastReSearchIndex
            | Command::BuildIndex
            | Command::RebuildIndex
            | Command::CancelIndexBuild => Some(ToolId::Search),
            Command::ExportFastenerHoleReport => Some(ToolId::FastenerHole),
            Command::ToggleBushingNumbersPanel | Command::ExportBushingReport | Command::OpenReamerPicker | Command::OpenHousingMaterialPicker | Command::OpenBushingMaterialPicker => {
                Some(ToolId::Bushing)
            }
            Command::TogglePressureVesselNumbersPanel | Command::ExportPressureVesselReport | Command::OpenPressureVesselMaterialPicker => Some(ToolId::PressureVessel),
            Command::TogglePreloadAnalysisNumbersPanel | Command::ExportPreloadAnalysisReport | Command::OpenBoltPicker => Some(ToolId::PreloadAnalysis),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommandPalette {
    pub query: String,
    pub selected: usize,
}

impl CommandPalette {
    /// Commands visible right now: always-global ones (`Command::scope() ==
    /// None`) plus whichever toolbox-scoped ones belong to `active_tool`,
    /// narrowed further by the typed query - so switching tabs never shows
    /// a Search-only command while on the Bushing tab, and vice versa.
    pub fn matches(&self, active_tool: ToolId) -> Vec<Command> {
        let q = self.query.to_lowercase();
        Command::ALL
            .iter()
            .copied()
            .filter(|c| c.scope().is_none() || c.scope() == Some(active_tool))
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

    pub fn move_selection(&mut self, active_tool: ToolId, delta: i32) {
        let len = self.matches(active_tool).len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected as i32;
        let next = (current + delta).rem_euclid(len as i32);
        self.selected = next as usize;
    }

    /// The command currently highlighted, if any commands match the query.
    pub fn picked(&self, active_tool: ToolId) -> Option<Command> {
        self.matches(active_tool).get(self.selected).copied()
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

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, palette: &CommandPalette, active_tool: ToolId, regions: &mut crate::mouse::MouseRegions) {
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

    let matches = palette.matches(active_tool);
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
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &palette, ToolId::Search, &mut regions)).unwrap();

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
            terminal.draw(|f| render(f, area, &Theme::default_palette(), &palette, ToolId::Search, &mut regions)).unwrap();
        }
    }

    #[test]
    fn empty_query_matches_every_global_command_plus_the_active_tools_own() {
        let palette = CommandPalette::default();
        let global_count = Command::ALL.iter().filter(|c| c.scope().is_none()).count();
        let bushing_count = Command::ALL.iter().filter(|c| c.scope() == Some(ToolId::Bushing)).count();
        assert_eq!(palette.matches(ToolId::Bushing).len(), global_count + bushing_count);
    }

    #[test]
    fn a_toolbox_scoped_command_is_absent_while_a_different_tool_is_active() {
        let palette = CommandPalette::default();
        assert!(!palette.matches(ToolId::Search).contains(&Command::ExportBushingReport), "Bushing-only command must not show while Search is active");
        assert!(palette.matches(ToolId::Bushing).contains(&Command::ExportBushingReport), "Bushing-only command must show while Bushing is active");
    }

    #[test]
    fn a_global_command_is_present_regardless_of_active_tool() {
        let palette = CommandPalette::default();
        assert!(palette.matches(ToolId::Search).contains(&Command::Quit));
        assert!(palette.matches(ToolId::Bushing).contains(&Command::Quit));
    }

    #[test]
    fn query_filters_by_label_substring() {
        let mut palette = CommandPalette::default();
        "cancel".chars().for_each(|c| palette.push_char(c));
        assert_eq!(palette.matches(ToolId::Search), vec![Command::CancelSearch]);
    }

    #[test]
    fn move_selection_wraps_both_directions() {
        let mut palette = CommandPalette::default();
        "switch to".chars().for_each(|c| palette.push_char(c));
        let match_count = palette.matches(ToolId::Search).len();
        assert_eq!(match_count, 8, "one \"Switch to: ...\" entry per ToolId variant");
        palette.move_selection(ToolId::Search, -1);
        assert_eq!(palette.selected, match_count - 1);
        palette.move_selection(ToolId::Search, 1);
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn picked_reflects_current_selection() {
        let mut palette = CommandPalette::default();
        "quit".chars().for_each(|c| palette.push_char(c));
        assert_eq!(palette.picked(ToolId::Search), Some(Command::Quit));
    }

    #[test]
    fn picked_is_none_when_nothing_matches() {
        let mut palette = CommandPalette::default();
        "zzz-no-such-command".chars().for_each(|c| palette.push_char(c));
        assert_eq!(palette.picked(ToolId::Search), None);
    }
}
