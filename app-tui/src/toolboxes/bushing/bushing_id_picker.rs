//! Bushing ID picker - same selection/library functionality as the reamer
//! picker (`reamer_picker.rs`), minus "nearest real size" preseeding: there
//! is no industry catalog for a finished bushing ID the way there is for
//! reamer tool sizes, so this list is purely user-defined/imported entries.
//! `n` adds the current Bushing ID value as a new labeled entry (tagged
//! "Preferred"), `i`/`x` import/export a library file, `m` drops to plain
//! numeric entry. Import/export/duplicate-pruning/conflict-resolution/
//! labeling all reuse `crate::library`'s generic machinery - see that
//! module's own doc comment for the JSON schema.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::library::{ConflictQueue, ConflictResolution, LibraryItem};
use crate::theme::Theme;
use crate::widgets::empty_state;

use super::bushing_id_persistence::{self, PersistedBushingId};
use super::model::{self, BushingModel};
use crate::app::Effect;

enum PathPromptKind {
    Import,
    Export,
}

struct PathPrompt {
    kind: PathPromptKind,
    buffer: String,
}

pub struct BushingIdPickerState {
    pub open: bool,
    pub cursor: usize,
    pub filter_text: String,
    pub filtering: bool,
    pub library: Vec<LibraryItem<PersistedBushingId>>,
    pub pending_conflicts: Option<ConflictQueue<PersistedBushingId>>,
    path_prompt: Option<PathPrompt>,
}

impl Default for BushingIdPickerState {
    fn default() -> Self {
        Self { open: false, cursor: 0, filter_text: String::new(), filtering: false, library: Vec::new(), pending_conflicts: None, path_prompt: None }
    }
}

impl BushingIdPickerState {
    /// Opens at the closest *saved* entry to the current Bushing ID, if any
    /// exist - otherwise an empty state (there's no built-in catalog to
    /// fall back to).
    pub fn open_near(model: &BushingModel) -> Self {
        let library = bushing_id_persistence::load();
        let mut state = Self { open: true, library, ..Default::default() };
        if !state.library.is_empty() {
            let target = model.id_bushing;
            state.cursor = state.library.iter().enumerate().min_by(|(_, a), (_, b)| (a.item.id_in - target).abs().partial_cmp(&(b.item.id_in - target).abs()).unwrap()).map(|(i, _)| i).unwrap_or(0);
        }
        state
    }

    fn visible(&self) -> Vec<&LibraryItem<PersistedBushingId>> {
        let needle = self.filter_text.trim().to_lowercase();
        self.library.iter().filter(|li| needle.is_empty() || li.item.label.to_lowercase().contains(&needle)).collect()
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

pub fn handle_key(picker: &mut BushingIdPickerState, model: &mut BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    if let Some(queue) = &mut picker.pending_conflicts {
        return handle_conflict_key(queue, &mut picker.library, key);
    }

    if let Some(prompt) = &mut picker.path_prompt {
        return match key.code {
            KeyCode::Enter => {
                let path = prompt.buffer.trim().to_string();
                let kind_is_export = matches!(prompt.kind, PathPromptKind::Export);
                picker.path_prompt = None;
                if path.is_empty() {
                    return (true, Vec::new());
                }
                if kind_is_export {
                    let contents = crate::library::export_json(&picker.library);
                    (true, vec![Effect::ExportBushingIdLibraryFile { path, contents }])
                } else {
                    (true, vec![Effect::ImportBushingIdLibraryFile(path)])
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
            picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Import, buffer: default_path_string() });
            (true, Vec::new())
        }
        KeyCode::Char('x' | 'X') => {
            picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Export, buffer: default_path_string() });
            (true, Vec::new())
        }
        KeyCode::Char('n' | 'N') => {
            let value = model.id_bushing;
            let label = model::format_for_edit(value);
            let persisted = PersistedBushingId { label: label.clone(), id_in: value };
            if let Some(existing) = picker.library.iter_mut().find(|li| li.item == persisted) {
                if !existing.labels.iter().any(|l| l == "Preferred") {
                    existing.labels.push("Preferred".to_string());
                }
            } else {
                picker.library.push(LibraryItem { item: persisted, labels: vec!["Preferred".to_string()] });
            }
            (true, vec![Effect::PersistBushingIdLibrary])
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
            if let Some(entry) = picker.visible().get(picker.cursor) {
                model.id_bushing = entry.item.id_in;
                model.recompute();
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

fn default_path_string() -> String {
    crate::paths::app_data_dir().unwrap_or_default().join("bushing-id-library.json").to_string_lossy().into_owned()
}

fn handle_conflict_key(queue: &mut ConflictQueue<PersistedBushingId>, library: &mut Vec<LibraryItem<PersistedBushingId>>, key: KeyEvent) -> (bool, Vec<Effect>) {
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
    (true, vec![Effect::PersistBushingIdLibrary])
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

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &BushingIdPickerState, model: &BushingModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(64, 60, area);
    frame.render_widget(Clear, popup);

    if let Some(queue) = &picker.pending_conflicts {
        render_conflict_prompt(frame, popup, theme, queue);
        return;
    }

    let visible = picker.visible();
    let title = format!(" Bushing ID library ({} of {}) - n: save current \u{b7} i: import \u{b7} x: export \u{b7} m: type exact value ", visible.len(), picker.library.len());
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (list_area, bottom_area) = if inner.height > 1 { let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner); (rows[0], Some(rows[1])) } else { (inner, None) };

    if visible.is_empty() {
        empty_state::render(frame, list_area, theme, "No saved Bushing ID entries yet", Some("n saves the current value here \u{b7} m types an exact value directly"));
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, li)| {
                let selected = i == picker.cursor;
                let marker = if selected { "> " } else { "  " };
                let style = if selected { theme.selected_row_style() } else { Style::default() };
                let labels_tag = if li.labels.is_empty() { String::new() } else { format!(" {{{}}}", li.labels.join(", ")) };
                let delta = li.item.id_in - model.id_bushing;
                ListItem::new(Line::from(Span::styled(format!("{marker}{:<16} {:.4} in{}  \u{394} {:+.4}", li.item.label, li.item.id_in, labels_tag, delta), style)))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.bushing_id_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
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

fn render_conflict_prompt(frame: &mut Frame, area: Rect, theme: &Theme, queue: &ConflictQueue<PersistedBushingId>) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(" Bushing ID import conflict ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let Some((_, incoming)) = queue.current() else { return };
    let lines = vec![
        Line::from(format!("\"{}\" already exists with a different value.", incoming.item.label)),
        Line::from(""),
        Line::from(format!("Imported: {:.4} in", incoming.item.id_in)),
        Line::from(""),
        Line::from(Span::styled("k: keep existing    o: overwrite    z: keep all remaining    a: overwrite all remaining", theme.disabled_style())),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
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
    fn empty_library_opens_with_no_entries() {
        let model = BushingModel::default();
        let picker = BushingIdPickerState::open_near(&model);
        assert!(picker.open);
        assert!(picker.library.is_empty());
    }

    #[test]
    fn n_saves_the_current_value_as_a_preferred_entry() {
        let mut picker = BushingIdPickerState::open_near(&BushingModel::default());
        let mut model = BushingModel::default();
        model.id_bushing = 0.375;
        let (consumed, effects) = handle_key(&mut picker, &mut model, key(KeyCode::Char('n')));
        assert!(consumed);
        assert!(matches!(effects.as_slice(), [Effect::PersistBushingIdLibrary]));
        assert_eq!(picker.library.len(), 1);
        assert_eq!(picker.library[0].item.id_in, 0.375);
        assert_eq!(picker.library[0].labels, vec!["Preferred".to_string()]);
    }

    #[test]
    fn enter_selects_the_highlighted_entry_and_closes() {
        let mut picker = BushingIdPickerState::open_near(&BushingModel::default());
        picker.library.push(LibraryItem::new(PersistedBushingId { label: "0.4000".to_string(), id_in: 0.4 }));
        let mut model = BushingModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        assert_eq!(model.id_bushing, 0.4);
    }

    #[test]
    fn esc_closes_without_changing_id_bushing() {
        let mut picker = BushingIdPickerState::open_near(&BushingModel::default());
        let mut model = BushingModel::default();
        let before = model.id_bushing;
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
        assert_eq!(model.id_bushing, before);
    }

    #[test]
    fn render_does_not_panic_when_empty_or_populated() {
        let mut picker = BushingIdPickerState::open_near(&BushingModel::default());
        let model = BushingModel::default();
        for (w, h) in [(100, 30), (0, 0), (1, 1)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, Rect { x: 0, y: 0, width: w, height: h }, &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
        }
        picker.library.push(LibraryItem::new(PersistedBushingId { label: "0.4000".to_string(), id_in: 0.4 }));
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, Rect { x: 0, y: 0, width: 100, height: 30 }, &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
    }
}
