//! Bore-diameter reamer picker popup - the primary way to set Bore
//! Diameter (see `mod.rs`'s Enter routing on that row), not a secondary
//! "nearest suggestion" alongside free numeric entry. A real installed
//! bushing bore is reamed to a real tool size, not an arbitrary decimal -
//! this picker's full, filterable catalog (`bushing_solver::reamers::all_reamers`,
//! the same sourced Pan American Tool/Rock River Tool/Omega Technologies
//! aircraft reamer data `bushing-solver/AGENTS.md` documents, plus any
//! user-added/imported entries - see `reamer_persistence.rs`) is opened
//! pre-positioned at the closest real size to whatever value is already
//! entered, mirroring `material_picker.rs`'s filter/list pattern. `m`
//! (handled one level up, in `mod.rs`, since it needs to hand control back
//! to the toolbox's own text-edit mode) is the escape hatch to a plain
//! numeric value when a non-catalog bore is genuinely needed.
//!
//! Library management (`i` import, `x` export, `n` add current bore as a
//! custom entry) is built on `crate::library`'s generic import/export,
//! duplicate-pruning, and conflict-resolution machinery - see that
//! module's own doc comment for the JSON schema. File I/O for import/
//! export goes through `Effect::ImportReamerLibraryFile`/
//! `Effect::ExportReamerLibraryFile` (never a direct `std::fs` call from
//! this reducer-adjacent code - see `app-tui/AGENTS.md`'s Contracts).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph};
use ratatui::Frame;

use bushing_solver::reamers::{self, AvailabilityTier, ReamerEntry};

use crate::library::{ConflictQueue, ConflictResolution, LibraryItem};
use crate::theme::Theme;
use crate::widgets::empty_state;

use super::model::BushingModel;
use super::reamer_persistence::{self, PersistedReamer};
use crate::app::Effect;

/// Which text buffer a `PathPrompt` is driving - the prompt UI itself
/// (prefilled path, `Enter` commits, `Esc` cancels) is identical either
/// way, only the resulting `Effect` differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathPromptKind {
    Import,
    Export,
}

#[derive(Debug, Clone)]
struct PathPrompt {
    kind: PathPromptKind,
    buffer: String,
}

pub struct ReamerPickerState {
    pub open: bool,
    /// Index into the *visible* (filtered) list, not directly into the
    /// combined built-in+custom catalog.
    pub cursor: usize,
    pub filter_text: String,
    pub filtering: bool,
    /// User-added/imported entries, merged into `visible()` after the
    /// built-in catalog - persisted via `reamer_persistence.rs`.
    pub library: Vec<LibraryItem<PersistedReamer>>,
    /// Real key conflicts (same size label, different data) from the most
    /// recent import, awaiting a keep/overwrite decision. `None` when
    /// nothing is pending.
    pub pending_conflicts: Option<ConflictQueue<PersistedReamer>>,
    path_prompt: Option<PathPrompt>,
}

impl Default for ReamerPickerState {
    fn default() -> Self {
        Self { open: false, cursor: 0, filter_text: String::new(), filtering: false, library: Vec::new(), pending_conflicts: None, path_prompt: None }
    }
}

impl ReamerPickerState {
    /// Opens with the cursor pre-positioned at the catalog entry closest to
    /// `model.bore_dia` - the picker starts where the user's current value
    /// already is, not at the top of a ~48+-entry list.
    pub fn open_near(model: &BushingModel) -> Self {
        let mut state = Self { open: true, cursor: 0, filter_text: String::new(), filtering: false, library: reamer_persistence::load(), pending_conflicts: None, path_prompt: None };
        let target = model.bore_dia;
        let all = state.visible();
        state.cursor = all.iter().enumerate().min_by(|(_, a), (_, b)| (a.0.nominal_in - target).abs().partial_cmp(&(b.0.nominal_in - target).abs()).unwrap()).map(|(i, _)| i).unwrap_or(0);
        state
    }

    /// The combined built-in + custom catalog, each entry paired with its
    /// labels, narrowed by `filter_text`. A library entry whose data
    /// exactly matches a built-in (e.g. tagging a built-in size
    /// "Preferred" via `n`, which stores it the same way an imported
    /// label-only update would) merges its labels onto that built-in's row
    /// instead of appearing as a duplicate - only a library entry with
    /// genuinely different data becomes its own new row.
    fn visible(&self) -> Vec<(ReamerEntry, Vec<String>)> {
        let needle = self.filter_text.trim().to_lowercase();
        let mut all: Vec<(ReamerEntry, Vec<String>)> = reamers::all_reamers().into_iter().map(|e| (e.clone(), Vec::new())).collect();
        for li in &self.library {
            match all.iter_mut().find(|(e, _)| PersistedReamer::from_entry(e) == li.item) {
                Some((_, labels)) => {
                    for l in &li.labels {
                        if !labels.contains(l) {
                            labels.push(l.clone());
                        }
                    }
                }
                None => all.push((li.item.to_entry(), li.labels.clone())),
            }
        }
        if !needle.is_empty() {
            all.retain(|(e, _)| e.size_label.to_lowercase().contains(&needle));
        }
        all
    }

    fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len as i32) as usize;
    }
}

pub fn handle_key(picker: &mut ReamerPickerState, model: &mut BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    if let Some(queue) = &mut picker.pending_conflicts {
        return handle_conflict_key(queue, &mut picker.library, key);
    }

    if let Some(prompt) = &mut picker.path_prompt {
        return match key.code {
            KeyCode::Enter => {
                let path = prompt.buffer.trim().to_string();
                let kind = prompt.kind;
                picker.path_prompt = None;
                if path.is_empty() {
                    return (true, Vec::new());
                }
                match kind {
                    PathPromptKind::Import => (true, vec![Effect::ImportReamerLibraryFile(path)]),
                    PathPromptKind::Export => {
                        let mut items = reamer_persistence::builtin_as_library_items();
                        items.extend(picker.library.iter().cloned());
                        let contents = crate::library::export_json(&items);
                        (true, vec![Effect::ExportReamerLibraryFile { path, contents }])
                    }
                }
            }
            KeyCode::Esc => {
                picker.path_prompt = None;
                (true, Vec::new())
            }
            KeyCode::Delete => {
                prompt.buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                prompt.buffer.pop();
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                prompt.buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    if picker.filtering {
        return match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                picker.filtering = false;
                picker.cursor = 0;
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                picker.filter_text.pop();
                picker.cursor = 0;
                (true, Vec::new())
            }
            KeyCode::Delete => {
                picker.filter_text.clear();
                picker.cursor = 0;
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                picker.filter_text.push(c);
                picker.cursor = 0;
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    match key.code {
        KeyCode::Char('/') => {
            picker.filtering = true;
            (true, Vec::new())
        }
        KeyCode::Char('i' | 'I') => {
            picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Import, buffer: reamer_persistence::default_file_path().to_string_lossy().into_owned() });
            (true, Vec::new())
        }
        KeyCode::Char('x' | 'X') => {
            picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Export, buffer: reamer_persistence::default_file_path().to_string_lossy().into_owned() });
            (true, Vec::new())
        }
        KeyCode::Char('n' | 'N') => {
            // Adds the currently-highlighted entry as a labeled custom
            // entry ("Preferred", etc.) - not a new-value form, since every
            // field a custom reamer entry needs (size, tolerances) already
            // exists on whatever row is highlighted; labeling an existing
            // built-in is a real use case ("tag this one Preferred") as
            // much as adding a genuinely new size is.
            if let Some((entry, _)) = picker.visible().get(picker.cursor).cloned() {
                let persisted = PersistedReamer::from_entry(&entry);
                if let Some(existing) = picker.library.iter_mut().find(|li| li.item == persisted) {
                    if !existing.labels.iter().any(|l| l == "Preferred") {
                        existing.labels.push("Preferred".to_string());
                    }
                } else {
                    picker.library.push(LibraryItem { item: persisted, labels: vec!["Preferred".to_string()] });
                }
                return (true, vec![Effect::PersistReamerLibrary]);
            }
            (true, Vec::new())
        }
        KeyCode::Up => {
            picker.move_cursor(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            picker.move_cursor(1);
            (true, Vec::new())
        }
        KeyCode::Enter => {
            if let Some((entry, _)) = picker.visible().get(picker.cursor) {
                model.select_reamer(entry);
            }
            picker.open = false;
            (true, Vec::new())
        }
        KeyCode::Esc => {
            if !picker.filter_text.is_empty() {
                picker.filter_text.clear();
                picker.cursor = 0;
            } else {
                picker.open = false;
            }
            (true, Vec::new())
        }
        _ => (false, Vec::new()),
    }
}

fn handle_conflict_key(queue: &mut ConflictQueue<PersistedReamer>, library: &mut Vec<LibraryItem<PersistedReamer>>, key: KeyEvent) -> (bool, Vec<Effect>) {
    // Same-action-either-case on every letter here (Windows Caps-Lock can
    // report an uppercase letter with no Shift held - see
    // `app-tui/AGENTS.md`'s Pitfalls) - each pair means one action, never
    // two different ones gated by case.
    let resolved = match key.code {
        KeyCode::Char('k' | 'K') => Some((ConflictResolution::KeepExisting, false)),
        KeyCode::Char('o' | 'O') => Some((ConflictResolution::Overwrite, false)),
        KeyCode::Char('z' | 'Z') => Some((ConflictResolution::KeepExisting, true)),
        KeyCode::Char('a' | 'A') => Some((ConflictResolution::Overwrite, true)),
        _ => None,
    };
    let Some((resolution, apply_all)) = resolved else {
        return (false, Vec::new());
    };
    queue.resolve(library, resolution, apply_all);
    (true, Vec::new())
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)])
        .split(vertical[1])[1]
}

fn tier_tag(tier: AvailabilityTier) -> &'static str {
    match tier {
        AvailabilityTier::Preferred => " [preferred]",
        AvailabilityTier::Common => "",
        AvailabilityTier::Special => " [special]",
    }
}

fn labels_tag(labels: &[String]) -> String {
    if labels.is_empty() { String::new() } else { format!(" {{{}}}", labels.join(", ")) }
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &ReamerPickerState, model: &BushingModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(70, 75, area);
    frame.render_widget(Clear, popup);

    if let Some(queue) = &picker.pending_conflicts {
        render_conflict_prompt(frame, popup, theme, queue);
        return;
    }

    let visible = picker.visible();
    let total = reamers::all_reamers().len() + picker.library.len();
    let title = if picker.filter_text.is_empty() {
        format!(" Reamer for Bore Diameter (currently {:.4} in, {total} sizes) - i: import \u{b7} x: export \u{b7} n: tag Preferred \u{b7} m: type exact value ", model.bore_dia)
    } else {
        format!(" Reamer ({} of {total}) - m: type exact value ", visible.len())
    };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (list_area, bottom_area) = if inner.height > 1 { let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner); (rows[0], Some(rows[1])) } else { (inner, None) };

    if visible.is_empty() {
        empty_state::render(frame, list_area, theme, "No reamer sizes match the filter", Some("Esc to clear the filter"));
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, (entry, labels))| {
                let selected = i == picker.cursor;
                let marker = if selected { "> " } else { "  " };
                let style = if selected { theme.selected_row_style() } else { Style::default() };
                let delta = entry.nominal_in - model.bore_dia;
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{marker}{:<12} {:.4} in (+{:.4}/-{:.4}){}{}  \u{394} {:+.4}",
                        entry.size_label,
                        entry.nominal_in,
                        entry.tool_tolerance_plus_in,
                        entry.tool_tolerance_minus_in,
                        tier_tag(entry.availability_tier),
                        labels_tag(labels),
                        delta
                    ),
                    style,
                )))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.reamer_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
    }

    if let Some(area) = bottom_area {
        if let Some(prompt) = &picker.path_prompt {
            let label = match prompt.kind {
                PathPromptKind::Import => "Import from (Enter to confirm, Esc to cancel): ",
                PathPromptKind::Export => "Export to (Enter to confirm, Esc to cancel): ",
            };
            let line = crate::widgets::input_line::line(theme, label, prompt.buffer.as_str(), "_", area.width);
            frame.render_widget(Paragraph::new(line), area);
        } else if picker.filtering || !picker.filter_text.is_empty() {
            let label = if picker.filtering { "Filter (Enter/Esc to stop): " } else { "Filter: " };
            let cursor_glyph = if picker.filtering { "_" } else { "" };
            let line = crate::widgets::input_line::line(theme, label, picker.filter_text.as_str(), cursor_glyph, area.width);
            frame.render_widget(Paragraph::new(line), area);
        }
    }
}

fn render_conflict_prompt(frame: &mut Frame, area: Rect, theme: &Theme, queue: &ConflictQueue<PersistedReamer>) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(" Reamer import conflict ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let Some((_, incoming)) = queue.current() else { return };
    let lines = vec![
        Line::from(format!("Size {} already exists with different data.", incoming.item.size_label)),
        Line::from(""),
        Line::from(format!("Imported: {:.4} in  +{:.4}/-{:.4}", incoming.item.nominal_in, incoming.item.tool_tolerance_plus_in, incoming.item.tool_tolerance_minus_in)),
        Line::from(""),
        Line::from(Span::styled("k: keep existing    o: overwrite    z: keep all remaining    a: overwrite all remaining", theme.disabled_style())),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn open_near_positions_the_cursor_at_the_closest_real_size() {
        let model = BushingModel::default(); // bore_dia = 0.5
        let picker = ReamerPickerState::open_near(&model);
        assert!(picker.open);
        let visible = picker.visible();
        let nearest = &visible[picker.cursor].0;
        for (e, _) in &visible {
            assert!((nearest.nominal_in - 0.5).abs() <= (e.nominal_in - 0.5).abs());
        }
    }

    #[test]
    fn enter_sets_bore_dia_and_tolerance_to_the_selected_reamer_and_closes() {
        let mut model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let (target, target_tol_plus) = {
            let visible = picker.visible();
            let e = &visible[picker.cursor].0;
            (e.nominal_in, e.tool_tolerance_plus_in)
        };
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        assert_eq!(model.bore_dia, target);
        assert_eq!(model.bore_tol_plus, target_tol_plus);
    }

    #[test]
    fn esc_closes_without_changing_bore_dia() {
        let mut model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let before = model.bore_dia;
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
        assert_eq!(model.bore_dia, before);
    }

    #[test]
    fn slash_then_typing_narrows_the_visible_list() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let mut model = model;
        handle_key(&mut picker, &mut model, key(KeyCode::Char('/')));
        for c in "1/4".chars() {
            handle_key(&mut picker, &mut model, key(KeyCode::Char(c)));
        }
        let visible = picker.visible();
        assert!(!visible.is_empty());
        assert!(visible.iter().all(|(e, _)| e.size_label.to_lowercase().contains("1/4")));
    }

    #[test]
    fn up_down_navigation_wraps() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState { open: true, cursor: 0, filter_text: String::new(), filtering: false, library: Vec::new(), pending_conflicts: None, path_prompt: None };
        let mut model = model;
        let len = picker.visible().len();
        handle_key(&mut picker, &mut model, key(KeyCode::Up));
        assert_eq!(picker.cursor, len - 1);
        handle_key(&mut picker, &mut model, key(KeyCode::Down));
        assert_eq!(picker.cursor, 0);
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let model = BushingModel::default();
        let picker = ReamerPickerState::open_near(&model);
        for (w, h) in [(100, 30), (0, 0), (1, 1)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, Rect { x: 0, y: 0, width: w, height: h }, &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
        }
    }

    #[test]
    fn render_with_filter_active_does_not_panic() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        picker.filtering = true;
        picker.filter_text = "3/8".to_string();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
    }

    #[test]
    fn rows_show_the_signed_delta_from_the_current_bore_diameter() {
        let mut model = BushingModel::default();
        model.bore_dia = 0.5;
        let mut picker = ReamerPickerState::open_near(&model);
        picker.filtering = true;
        picker.filter_text = "1/2".to_string();
        let entry = picker.visible()[0].0.clone();
        let expected_delta = entry.nominal_in - model.bore_dia;

        // Wide enough that the popup (70% of terminal width) doesn't clip
        // the trailing delta text - the same class of truncation this
        // crate's other row-content tests guard against.
        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered: Vec<String> = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect();
        let needle = format!("\u{394} {expected_delta:+.4}");
        assert!(rendered.iter().any(|line| line.contains(&needle)), "expected delta `{needle}` in:\n{}", rendered.join("\n"));
    }

    #[test]
    fn n_tags_the_highlighted_entry_as_preferred() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let mut model = model;
        let size_label = picker.visible()[picker.cursor].0.size_label.clone();
        let (consumed, effects) = handle_key(&mut picker, &mut model, key(KeyCode::Char('n')));
        assert!(consumed);
        assert!(matches!(effects.as_slice(), [Effect::PersistReamerLibrary]));
        let visible = picker.visible();
        let (_, labels) = visible.iter().find(|(e, _)| e.size_label == size_label).unwrap();
        assert_eq!(labels, &vec!["Preferred".to_string()]);
    }

    #[test]
    fn i_opens_an_import_prompt_prefilled_with_the_default_path() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let mut model = model;
        handle_key(&mut picker, &mut model, key(KeyCode::Char('i')));
        assert!(picker.path_prompt.is_some());
        assert!(!picker.path_prompt.as_ref().unwrap().buffer.is_empty());
    }

    #[test]
    fn enter_on_the_import_prompt_returns_an_import_effect_with_the_typed_path() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let mut model = model;
        handle_key(&mut picker, &mut model, key(KeyCode::Char('i')));
        // Replace the prefilled path with a custom one - Delete clears the
        // whole buffer in one press rather than guessing how many
        // Backspaces the platform-specific default path needs.
        handle_key(&mut picker, &mut model, key(KeyCode::Delete));
        for c in "/tmp/my-reamers.json".chars() {
            handle_key(&mut picker, &mut model, key(KeyCode::Char(c)));
        }
        let (consumed, effects) = handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(consumed);
        assert!(picker.path_prompt.is_none());
        match effects.as_slice() {
            [Effect::ImportReamerLibraryFile(path)] => assert_eq!(path, "/tmp/my-reamers.json"),
            other => panic!("expected exactly one ImportReamerLibraryFile effect, got {other:?}"),
        }
    }

    #[test]
    fn esc_cancels_the_path_prompt_without_an_effect() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let mut model = model;
        handle_key(&mut picker, &mut model, key(KeyCode::Char('x')));
        let (consumed, effects) = handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(consumed);
        assert!(effects.is_empty());
        assert!(picker.path_prompt.is_none());
    }

    #[test]
    fn conflict_resolution_keeps_existing_on_k() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let existing = PersistedReamer { size_label: "Q".to_string(), nominal_in: 0.332, tool_tolerance_plus_in: 0.0003, tool_tolerance_minus_in: 0.0, notes: String::new() };
        picker.library.push(LibraryItem::new(existing.clone()));
        let incoming = PersistedReamer { nominal_in: 999.0, ..existing.clone() };
        let outcomes = crate::library::classify_import(&picker.library, vec![LibraryItem::new(incoming)], |p| p.size_label.clone());
        let (queue, _, _) = ConflictQueue::new(outcomes, &mut picker.library);
        picker.pending_conflicts = Some(queue);
        let mut model = model;
        let (consumed, _) = handle_key(&mut picker, &mut model, key(KeyCode::Char('k')));
        assert!(consumed);
        assert!(picker.pending_conflicts.as_ref().unwrap().is_empty());
        let kept = picker.library.iter().find(|i| i.item.size_label == "Q").expect("existing entry must remain");
        assert_eq!(kept.item.nominal_in, 0.332);
    }

    #[test]
    fn render_does_not_panic_with_a_pending_conflict() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let existing = PersistedReamer { size_label: "Q".to_string(), nominal_in: 0.332, tool_tolerance_plus_in: 0.0003, tool_tolerance_minus_in: 0.0, notes: String::new() };
        picker.library.push(LibraryItem::new(existing.clone()));
        let incoming = PersistedReamer { nominal_in: 999.0, ..existing };
        let outcomes = crate::library::classify_import(&picker.library, vec![LibraryItem::new(incoming)], |p| p.size_label.clone());
        let (queue, _, _) = ConflictQueue::new(outcomes, &mut picker.library);
        picker.pending_conflicts = Some(queue);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
    }

    /// Regression test for the import-path overflow bug: a long file
    /// location typed into the Import prompt used to run past the right edge
    /// of the popup, hiding the cursor and everything just typed.
    #[test]
    fn a_long_import_path_keeps_its_tail_and_cursor_visible() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let long = format!("C:\\Users\\someone\\Documents\\{}\\reamer-library-final.json", "very-long-folder-name".repeat(4));
        picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Import, buffer: long });
        for width in [60u16, 90, 140] {
            let backend = TestBackend::new(width, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();
            assert!(rendered.contains("reamer-library-final.json_"), "tail + cursor must be visible at width {width}:\n{rendered}");
        }
    }
}
