//! Search Files "Settings" workspace: a full scrollable form covering
//! every `SearchToolConfig` field, reached via `s` from the Run view.
//!
//! Deliberately a single flat field-index match rather than a generic
//! field/widget abstraction - only one toolbox needs a settings form in
//! this phase (see the plan's "don't abstract prematurely" rule); promote
//! to a reusable form widget once a second toolbox needs the same shape.
//! `Enter` on the Extensions field opens a modal checkbox catalog
//! (`extension_picker.rs`) instead of a plain text edit - see
//! `EXTENSIONS_FIELD` below.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph};

use search_core::models::{ExcludeScope, GroupByMode, MatchMode};

use crate::theme::{StatusTone, Theme};

use crate::app::Effect;

use super::model::{parse_list, regex_validation_error, SearchToolConfig};
use super::persistence::{PersistedSearchSettings, RecentSearch, SavedPreset};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Text,
    Number,
    Bool,
    MatchMode,
    ExcludeScope,
    GroupBy,
    /// `indexing::IndexLocation` - named distinctly from the real type to
    /// avoid confusion in this file's own `match`es.
    FastIndexLocation,
}

/// `FIELDS[EXTENSIONS_FIELD]`'s `Enter` is special-cased to open the
/// per-extension checkbox picker (a folder scan) instead of starting a
/// plain text edit - see its handling in `handle_key` below.
const EXTENSIONS_FIELD: usize = 9;

struct FieldDef {
    label: &'static str,
    kind: FieldKind,
}

const FIELDS: &[FieldDef] = &[
    FieldDef { label: "Search path", kind: FieldKind::Text },
    FieldDef { label: "Extra roots (comma-separated)", kind: FieldKind::Text },
    FieldDef { label: "Filters", kind: FieldKind::Text },
    FieldDef { label: "Exclude filters", kind: FieldKind::Text },
    FieldDef { label: "Match mode", kind: FieldKind::MatchMode },
    FieldDef { label: "Proximity lines", kind: FieldKind::Number },
    FieldDef { label: "Use regex", kind: FieldKind::Bool },
    FieldDef { label: "Whole word", kind: FieldKind::Bool },
    FieldDef { label: "Exclude scope", kind: FieldKind::ExcludeScope },
    FieldDef { label: "Extensions (comma list, blank = default)", kind: FieldKind::Text },
    FieldDef { label: "Exclude folders", kind: FieldKind::Text },
    FieldDef { label: "Include hidden", kind: FieldKind::Bool },
    FieldDef { label: "Max file size (MB)", kind: FieldKind::Number },
    FieldDef { label: "Group by", kind: FieldKind::GroupBy },
    FieldDef { label: "Output folder", kind: FieldKind::Text },
    FieldDef { label: "Output name", kind: FieldKind::Text },
    FieldDef { label: "Export HTML", kind: FieldKind::Bool },
    FieldDef { label: "Export CSV", kind: FieldKind::Bool },
    FieldDef { label: "Export JSON", kind: FieldKind::Bool },
    FieldDef { label: "Open report when done", kind: FieldKind::Bool },
    FieldDef { label: "Parallel", kind: FieldKind::Bool },
    FieldDef { label: "Throttle limit", kind: FieldKind::Number },
    FieldDef { label: "Heavy throttle limit", kind: FieldKind::Number },
    FieldDef { label: "Cache file path", kind: FieldKind::Text },
    FieldDef { label: "Dry run", kind: FieldKind::Bool },
    FieldDef { label: "PDF timeout (seconds)", kind: FieldKind::Number },
    FieldDef { label: "OCR scanned PDFs", kind: FieldKind::Bool },
    FieldDef { label: "File timeout (seconds)", kind: FieldKind::Number },
    FieldDef { label: "Max retries", kind: FieldKind::Number },
    FieldDef { label: "Fast re-search index", kind: FieldKind::Bool },
    FieldDef { label: "Index location", kind: FieldKind::FastIndexLocation },
];

/// A row in the rendered/navigable Fields list - either a non-selectable
/// section divider or a real `FIELDS` entry, same `Header`/real-row split
/// the four engineering toolboxes' own `FieldRow` enums use (see
/// `toolboxes/bushing/model.rs::FieldRow`). `FIELDS` itself stays a flat,
/// index-addressed array (unchanged - `EXTENSIONS_FIELD`/`field_value_string`/
/// `apply_text_edit`/`apply_toggle`/`display_value` all still take a plain
/// `FIELDS` index); this enum and [`settings_field_rows`] only add a
/// grouped VIEW over it for navigation/rendering, translated back to a
/// `FIELDS` index via [`SettingsView::field_index`] everywhere a real field
/// is read or edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsFieldRow {
    Header(&'static str),
    Field(usize),
}

/// `(FIELDS index a header precedes, header text)` - `FIELDS`'s own
/// ordering is already logically grouped (visible from its literal order
/// above), so grouping only needs a header inserted at these fixed
/// breakpoints, never a reorder.
const SETTINGS_HEADERS: &[(usize, &str)] = &[
    (0, "Search"),
    (2, "Filters"),
    (9, "Extensions & Exclusions"),
    (14, "Output"),
    (20, "Concurrency & Diagnostics"),
    (29, "Fast Re-search Index"),
];

fn settings_field_rows() -> Vec<SettingsFieldRow> {
    let mut rows = Vec::with_capacity(FIELDS.len() + SETTINGS_HEADERS.len());
    for i in 0..FIELDS.len() {
        if let Some((_, label)) = SETTINGS_HEADERS.iter().find(|(idx, _)| *idx == i) {
            rows.push(SettingsFieldRow::Header(label));
        }
        rows.push(SettingsFieldRow::Field(i));
    }
    rows
}

/// The `FIELDS` index for `rows[row_selected]` - `0` if that row is
/// somehow a `Header` (never happens in practice: [`SettingsView::move_selection`]
/// and click handling both nudge off `Header` rows the same way the
/// engineering toolboxes' own `clamp_selection` does).
fn resolve_field_index(rows: &[SettingsFieldRow], row_selected: usize) -> usize {
    match rows.get(row_selected) {
        Some(SettingsFieldRow::Field(i)) => *i,
        _ => 0,
    }
}

/// The inverse of [`resolve_field_index`] - which row position a `FIELDS`
/// index appears at once headers are inserted. Test-only: lets tests that
/// used to set `SettingsView { selected: <FIELDS index>, .. }` directly
/// keep expressing intent as a `FIELDS` index without duplicating the
/// header-breakpoint math themselves.
#[cfg(test)]
fn row_index_for_field(field_index: usize) -> usize {
    settings_field_rows().iter().position(|r| matches!(r, SettingsFieldRow::Field(i) if *i == field_index)).unwrap_or(0)
}

/// Which list within the Settings screen `Up`/`Down`/`Enter` currently act
/// on - `Tab`/`Shift+Tab` cycle between them (mirrors the shell's own
/// pane-focus-cycling pattern, scoped to this one screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Fields,
    Recents,
    Presets,
}

impl Section {
    fn next(self) -> Self {
        match self {
            Section::Fields => Section::Recents,
            Section::Recents => Section::Presets,
            Section::Presets => Section::Fields,
        }
    }

    fn prev(self) -> Self {
        match self {
            Section::Fields => Section::Presets,
            Section::Recents => Section::Fields,
            Section::Presets => Section::Recents,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsView {
    pub section: Section,
    /// Row position into [`settings_field_rows`] (headers included), not a
    /// `FIELDS` index - see [`SettingsView::field_index`].
    pub selected: usize,
    pub recent_selected: usize,
    pub preset_selected: usize,
    pub editing: bool,
    /// Naming a brand-new preset from the current live config (`a` in the
    /// Presets section) - a separate mode from `editing` since it isn't
    /// tied to a `FIELDS` index at all; both reuse `edit_buffer` for the
    /// in-progress text, but only one is ever true at a time.
    pub naming_preset: bool,
    /// Renaming the selected preset in place (`r` in the Presets section) -
    /// prefills `edit_buffer` with the preset's current name (unlike
    /// `naming_preset`, which starts blank) and commits by mutating that
    /// preset's `name` rather than pushing a new one. Mutually exclusive
    /// with `naming_preset`/`editing`.
    pub renaming_preset: bool,
    pub edit_buffer: String,
}

impl Default for SettingsView {
    /// Not a bare `#[derive(Default)]`: row 0 of [`settings_field_rows`] is
    /// the first `Header`, so a naive `selected: 0` would start the
    /// selection highlight on a non-selectable divider instead of the
    /// first real field - `clamp_selection` nudges it onto `Search path`
    /// the same way `FastenerHoleState`'s own `Default` impl does for its
    /// row-0-is-a-Header case.
    fn default() -> Self {
        let mut view =
            Self { section: Section::default(), selected: 0, recent_selected: 0, preset_selected: 0, editing: false, naming_preset: false, renaming_preset: false, edit_buffer: String::new() };
        view.clamp_selection();
        view
    }
}

impl SettingsView {
    /// `self.selected` is a row position into [`settings_field_rows`]
    /// (headers included), not a `FIELDS` index directly - see
    /// [`field_index`](Self::field_index) for the translation.
    pub fn move_selection(&mut self, delta: i32) {
        if self.editing {
            return;
        }
        let rows = settings_field_rows();
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        let len = rows.len() as i32;
        let mut next = self.selected as i32;
        for _ in 0..rows.len() {
            next = (next + delta).rem_euclid(len);
            if matches!(rows[next as usize], SettingsFieldRow::Field(_)) {
                break;
            }
        }
        self.selected = next as usize;
    }

    /// Nudges off a `Header` row - same technique
    /// `toolboxes/bushing/mod.rs::BushingState::clamp_selection` uses.
    /// Needed after a mouse click lands exactly on a header (arrow-key
    /// navigation via `move_selection` already never stops on one).
    pub(crate) fn clamp_selection(&mut self) {
        let rows = settings_field_rows();
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self.selected.min(rows.len() - 1);
        if matches!(rows[self.selected], SettingsFieldRow::Header(_)) {
            self.move_selection(1);
        }
    }

    /// The `FIELDS` index the current selection actually refers to.
    fn field_index(&self) -> usize {
        resolve_field_index(&settings_field_rows(), self.selected)
    }

    fn move_list_selection(current: usize, delta: i32, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        ((current as i32 + delta).rem_euclid(len as i32)) as usize
    }

    fn start_edit(&mut self, config: &SearchToolConfig) {
        self.editing = true;
        self.edit_buffer = field_value_string(config, self.field_index());
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn commit_edit(&mut self, config: &mut SearchToolConfig) {
        apply_text_edit(config, self.field_index(), &self.edit_buffer);
        self.editing = false;
        self.edit_buffer.clear();
    }
}

/// Returns `(consumed, effects)` - anything not consumed falls through to
/// `app.rs`'s global bindings (same contract as the Run view's
/// `handle_key`), so e.g. Ctrl+P still opens the command palette while
/// editing a field. Applying a recent search or preset only copies values
/// onto `config` (mirrors both existing heads' "apply preset" semantics:
/// it changes what you're about to search for, it does not itself start a
/// run) - but SAVING or DELETING a preset mutates persisted storage, so
/// those return `Effect::PersistSearchSettings` for `main.rs` to act on.
pub fn handle_key(
    view: &mut SettingsView,
    config: &mut SearchToolConfig,
    recents: &[RecentSearch],
    presets: &mut Vec<SavedPreset>,
    key: KeyEvent,
) -> (bool, Vec<Effect>) {
    if view.naming_preset {
        return match key.code {
            KeyCode::Enter => {
                let name = view.edit_buffer.trim().to_string();
                view.naming_preset = false;
                view.edit_buffer.clear();
                if name.is_empty() {
                    (true, Vec::new())
                } else {
                    presets.push(SavedPreset { name, settings: PersistedSearchSettings::from(&*config) });
                    (true, vec![Effect::PersistSearchSettings])
                }
            }
            KeyCode::Esc => {
                view.naming_preset = false;
                view.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                view.edit_buffer.pop();
                (true, Vec::new())
            }
            KeyCode::Delete => {
                view.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                view.edit_buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    if view.renaming_preset {
        return match key.code {
            KeyCode::Enter => {
                let name = view.edit_buffer.trim().to_string();
                view.renaming_preset = false;
                view.edit_buffer.clear();
                if name.is_empty() {
                    (true, Vec::new())
                } else if let Some(preset) = presets.get_mut(view.preset_selected) {
                    preset.name = name;
                    (true, vec![Effect::PersistSearchSettings])
                } else {
                    (true, Vec::new())
                }
            }
            KeyCode::Esc => {
                view.renaming_preset = false;
                view.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                view.edit_buffer.pop();
                (true, Vec::new())
            }
            KeyCode::Delete => {
                view.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                view.edit_buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    if view.editing {
        return match key.code {
            KeyCode::Enter => {
                view.commit_edit(config);
                (true, Vec::new())
            }
            KeyCode::Esc => {
                view.cancel_edit();
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                view.edit_buffer.pop();
                (true, Vec::new())
            }
            KeyCode::Delete => {
                view.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                view.edit_buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    if matches!(key.code, KeyCode::Tab) {
        view.section = view.section.next();
        return (true, Vec::new());
    }
    if matches!(key.code, KeyCode::BackTab) {
        view.section = view.section.prev();
        return (true, Vec::new());
    }

    match view.section {
        Section::Fields => {
            let field_i = view.field_index();
            let kind = FIELDS[field_i].kind;
            match key.code {
                KeyCode::Up => {
                    view.move_selection(-1);
                    (true, Vec::new())
                }
                KeyCode::Down => {
                    view.move_selection(1);
                    (true, Vec::new())
                }
                KeyCode::Char(' ') => {
                    apply_toggle(config, field_i, kind);
                    (true, Vec::new())
                }
                KeyCode::Enter if field_i == EXTENSIONS_FIELD => (
                    true,
                    vec![Effect::ScanExtensions {
                        root: config.search_path.clone(),
                        exclude_folders: parse_list(&config.exclude_folders_text),
                        include_hidden: config.include_hidden,
                    }],
                ),
                KeyCode::Enter => {
                    match kind {
                        FieldKind::Text | FieldKind::Number => view.start_edit(config),
                        FieldKind::Bool
                        | FieldKind::MatchMode
                        | FieldKind::ExcludeScope
                        | FieldKind::GroupBy
                        | FieldKind::FastIndexLocation => apply_toggle(config, field_i, kind),
                    }
                    (true, Vec::new())
                }
                _ => (false, Vec::new()),
            }
        }
        Section::Recents => match key.code {
            KeyCode::Up => {
                view.recent_selected = SettingsView::move_list_selection(view.recent_selected, -1, recents.len());
                (true, Vec::new())
            }
            KeyCode::Down => {
                view.recent_selected = SettingsView::move_list_selection(view.recent_selected, 1, recents.len());
                (true, Vec::new())
            }
            KeyCode::Enter => {
                if let Some(recent) = recents.get(view.recent_selected) {
                    config.search_path = recent.search_path.clone();
                    config.filters_text = recent.filters_text.clone();
                }
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        },
        Section::Presets => match key.code {
            KeyCode::Up => {
                view.preset_selected = SettingsView::move_list_selection(view.preset_selected, -1, presets.len());
                (true, Vec::new())
            }
            KeyCode::Down => {
                view.preset_selected = SettingsView::move_list_selection(view.preset_selected, 1, presets.len());
                (true, Vec::new())
            }
            KeyCode::Enter => {
                if let Some(preset) = presets.get(view.preset_selected) {
                    preset.settings.apply_to(config);
                }
                (true, Vec::new())
            }
            // Save the current live config as a brand-new named preset -
            // ported behavior from `app/`'s `save_current_as_preset`.
            KeyCode::Char('a' | 'A') if super::is_plain_char(key) => {
                view.naming_preset = true;
                view.edit_buffer.clear();
                (true, Vec::new())
            }
            // Rename the selected preset in place - prefills the name
            // editor with its current name (unlike `a`, which starts
            // blank), committing mutates that preset's `name` rather than
            // pushing a new one.
            KeyCode::Char('r' | 'R') if super::is_plain_char(key) && !presets.is_empty() => {
                view.renaming_preset = true;
                view.edit_buffer = presets[view.preset_selected].name.clone();
                (true, Vec::new())
            }
            // Delete the selected preset - ported behavior from `app/`'s
            // `delete_preset`.
            KeyCode::Char('d' | 'D') if super::is_plain_char(key) && !presets.is_empty() => {
                let removed = view.preset_selected.min(presets.len() - 1);
                presets.remove(removed);
                if view.preset_selected > 0 && view.preset_selected >= presets.len() {
                    view.preset_selected -= 1;
                }
                (true, vec![Effect::PersistSearchSettings])
            }
            _ => (false, Vec::new()),
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    config: &SearchToolConfig,
    recents: &[RecentSearch],
    presets: &[SavedPreset],
    view: &SettingsView,
    regions: &mut crate::mouse::MouseRegions,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(true))
        .title(" Settings (Tab: Fields / Recents / Presets) ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Recent-searches/saved-presets sections only get a slice of the area
    // when there's realistically room for them - below that, showing just
    // the field form (still fully usable) beats a cramped 3-way split.
    let show_history_sections = inner.height >= 12;

    let (fields_area, history_area) = if show_history_sections {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(5), Constraint::Length(5)])
            .split(inner);
        (rows[0], Some((rows[1], rows[2])))
    } else {
        (inner, None)
    };

    regions.settings_section_panes.push((fields_area, Section::Fields));
    draw_fields(frame, fields_area, theme, config, view, regions);

    if let Some((recents_area, presets_area)) = history_area {
        regions.settings_section_panes.push((recents_area, Section::Recents));
        regions.settings_section_panes.push((presets_area, Section::Presets));
        draw_recents(frame, recents_area, theme, recents, view, regions);
        draw_presets(frame, presets_area, theme, presets, view, regions);
    }
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, config: &SearchToolConfig, view: &SettingsView, regions: &mut crate::mouse::MouseRegions) {
    // Live regex validation takes priority over the static per-field Hint
    // below when present (it's actionable/urgent); both share the same
    // reserved bottom area, computed on the same path a real run takes -
    // shown as a reserved bottom line so it doesn't shift the field list
    // around as it appears/disappears.
    let rows = settings_field_rows();
    let validation_error = regex_validation_error(config);
    let bottom_text = match &validation_error {
        Some(error) => format!("Regex error: {error}"),
        None => field_hint(view.field_index()).to_string(),
    };
    let bottom_style = if validation_error.is_some() { theme.status_style(StatusTone::Danger) } else { theme.disabled_style() };
    let max_hint_lines = area.height.saturating_sub(4).max(1);
    let bottom_height = crate::widgets::hint_panel::hint_panel_height(&bottom_text, area.width, max_hint_lines);
    let (list_area, bottom_area) = if area.height > bottom_height + 1 {
        let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(bottom_height)]).split(area);
        (split[0], Some(split[1]))
    } else {
        (area, None)
    };

    let label_width = FIELDS.iter().map(|f| f.label.len()).max().unwrap_or(0) + 2;
    let focused = view.section == Section::Fields;

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(pos, row)| {
            if let SettingsFieldRow::Header(text) = row {
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            let SettingsFieldRow::Field(i) = row else { unreachable!() };
            let field = &FIELDS[*i];
            let selected = focused && pos == view.selected;
            let value = if selected && view.editing {
                format!("{}_", view.edit_buffer)
            } else {
                display_value(config, *i, field.kind)
            };
            let marker = if selected { "> " } else { "  " };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            ListItem::new(Line::from(Span::styled(
                format!("{marker}{:<width$}{value}", field.label, width = label_width),
                style,
            )))
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(view.selected));
    regions.settings_field_rows.extend(crate::mouse::list_row_regions(list_area, offset, rows.len()));

    if let Some(bottom_area) = bottom_area {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(bottom_text, bottom_style))).wrap(ratatui::widgets::Wrap { trim: true }), bottom_area);
    }
}

/// One-line description shown in the bottom Hint panel while this field is
/// selected (superseded by the live regex-validation error, when one is
/// present) - same purpose as `toolboxes/bushing/model.rs::field_hint`.
/// This toolbox now also groups `FIELDS` under `Header` dividers
/// ([`SettingsFieldRow`]/[`settings_field_rows`]), the same pattern the
/// four engineering toolboxes use - `FIELDS` itself stays the flat,
/// index-addressed source of truth every other call site
/// (`EXTENSIONS_FIELD`, `field_value_string`, `apply_text_edit`,
/// `apply_toggle`, `display_value`) already read by raw index; only
/// navigation and rendering go through the grouped row list, translated
/// back to a `FIELDS` index via `SettingsView::field_index`.
fn field_hint(index: usize) -> &'static str {
    match index {
        0 => "Root folder to search recursively.",
        1 => "Additional root folders to search alongside the main path, comma-separated.",
        2 => "Keywords/phrases to match, comma-separated (or one regex per Use Regex).",
        3 => "Keywords/phrases that exclude an otherwise-matching file, comma-separated.",
        4 => "How Filters are combined - match any filter, match all filters, or proximity (all filters within N lines of each other).",
        5 => "For Proximity match mode: how many lines apart matched filters may be and still count as one match.",
        6 => "Treat each filter as a regular expression instead of a literal/whole-word match.",
        7 => "Require filters to match whole words only, not substrings within a larger word.",
        8 => "Which folders Exclude filters apply to - just filenames, or the full path.",
        9 => "Which file extensions to search - blank searches the built-in default catalog; Enter opens a scanned per-extension checkbox picker.",
        10 => "Folder names to skip entirely during the recursive walk, comma-separated.",
        11 => "Include hidden files/folders (dotfiles on Unix, the Hidden attribute on Windows) in the search.",
        12 => "Skip any file larger than this size - keeps a single huge file from dominating scan time.",
        13 => "How results are organized in the report - by file, by filter, or ungrouped.",
        14 => "Folder the HTML/CSV/JSON report is written to.",
        15 => "Base filename for the generated report (before its extension).",
        16 => "Generate the HTML report.",
        17 => "Generate a CSV export alongside the report.",
        18 => "Generate a JSON export alongside the report.",
        19 => "Open the generated report automatically when the search finishes.",
        20 => "Search files concurrently rather than one at a time.",
        21 => "Maximum concurrent file-processing tasks for ordinary files.",
        22 => "Maximum concurrent tasks for heavy formats (PDF/archive extraction) - kept lower than Throttle Limit since these cost more per file.",
        23 => "Where the fast re-search index's own cache file lives on disk.",
        24 => "Preview which files would be searched without actually reading/matching their contents.",
        25 => "Give up on a single PDF after this many seconds rather than let one huge/corrupt file stall the whole run.",
        26 => "Run OCR on scanned (image-only) PDF pages that have no extractable text layer - slower, but finds text a scan alone would miss.",
        27 => "Give up on any single file after this many seconds, PDFs included (in addition to the PDF-specific timeout above).",
        28 => "How many times to retry a file after a transient read/extraction failure before giving up on it.",
        29 => "Build/use the fast Tantivy-backed re-search index for near-instant repeat searches of the same folder.",
        30 => "Where the fast re-search index itself is stored on disk.",
        _ => "",
    }
}

fn draw_recents(frame: &mut Frame, area: Rect, theme: &Theme, recents: &[RecentSearch], view: &SettingsView, regions: &mut crate::mouse::MouseRegions) {
    let focused = view.section == Section::Recents;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(format!(" Recent searches ({}) ", recents.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }
    if recents.is_empty() {
        crate::widgets::empty_state::render(frame, inner, theme, "No recent searches yet", None);
        return;
    }

    let items: Vec<ListItem> = recents
        .iter()
        .enumerate()
        .map(|(i, recent)| {
            let selected = focused && i == view.recent_selected;
            let marker = if selected { "> " } else { "  " };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{}", recent.label()), style)))
        })
        .collect();
    let offset = crate::widgets::scroll_list::render(frame, inner, items, focused.then_some(view.recent_selected));
    regions.recents_rows.extend(crate::mouse::list_row_regions(inner, offset, recents.len()));
}

fn draw_presets(frame: &mut Frame, area: Rect, theme: &Theme, presets: &[SavedPreset], view: &SettingsView, regions: &mut crate::mouse::MouseRegions) {
    let focused = view.section == Section::Presets;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(format!(" Saved presets ({}) - a: save current, r: rename, d: delete ", presets.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let (list_area, naming_area) = if (view.naming_preset || view.renaming_preset) && inner.height > 1 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        (rows[0], Some(rows[1]))
    } else {
        (inner, None)
    };

    if presets.is_empty() {
        crate::widgets::empty_state::render(frame, list_area, theme, "No saved presets yet", Some("a to save current settings"));
    } else {
        let items: Vec<ListItem> = presets
            .iter()
            .enumerate()
            .map(|(i, preset)| {
                let selected = focused && i == view.preset_selected;
                let marker = if selected { "> " } else { "  " };
                let style = if selected { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(format!("{marker}{}", preset.name), style)))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(view.preset_selected));
        regions.presets_rows.extend(crate::mouse::list_row_regions(list_area, offset, presets.len()));
    }

    if let Some(area) = naming_area {
        let label = if view.renaming_preset { "Rename preset: " } else { "New preset name: " };
        let line = Line::from(vec![
            Span::styled(label, theme.title_style(true)),
            Span::raw(view.edit_buffer.as_str()),
            Span::styled("_", theme.title_style(true)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }
}

fn field_value_string(config: &SearchToolConfig, idx: usize) -> String {
    match idx {
        0 => config.search_path.clone(),
        1 => config.search_paths_extra.join(", "),
        2 => config.filters_text.clone(),
        3 => config.exclude_filters_text.clone(),
        5 => config.proximity_lines.to_string(),
        9 => config.selected_extensions.as_ref().map(|v| v.join(", ")).unwrap_or_default(),
        10 => config.exclude_folders_text.clone(),
        12 => format!("{:.1}", config.max_file_size_mb),
        14 => config.output_folder.clone(),
        15 => config.output_name.clone(),
        21 => config.throttle_limit.to_string(),
        22 => config.heavy_throttle_limit.to_string(),
        23 => config.cache_file_path.clone(),
        25 => config.pdf_timeout_seconds.to_string(),
        27 => config.file_timeout_seconds.to_string(),
        28 => config.max_retries.to_string(),
        _ => String::new(),
    }
}

fn apply_text_edit(config: &mut SearchToolConfig, idx: usize, text: &str) {
    let trimmed = text.trim();
    match idx {
        0 => config.search_path = text.to_string(),
        1 => config.search_paths_extra = parse_list(text),
        2 => config.filters_text = text.to_string(),
        3 => config.exclude_filters_text = text.to_string(),
        5 => {
            if let Ok(v) = trimmed.parse() {
                config.proximity_lines = v;
            }
        }
        9 => {
            let list = parse_list(text);
            config.selected_extensions = if list.is_empty() { None } else { Some(list) };
        }
        10 => config.exclude_folders_text = text.to_string(),
        12 => {
            if let Ok(v) = trimmed.parse() {
                config.max_file_size_mb = v;
            }
        }
        14 => config.output_folder = text.to_string(),
        15 => config.output_name = text.to_string(),
        21 => {
            if let Ok(v) = trimmed.parse() {
                config.throttle_limit = v;
            }
        }
        22 => {
            if let Ok(v) = trimmed.parse() {
                config.heavy_throttle_limit = v;
            }
        }
        23 => config.cache_file_path = text.to_string(),
        25 => {
            if let Ok(v) = trimmed.parse() {
                config.pdf_timeout_seconds = v;
            }
        }
        27 => {
            if let Ok(v) = trimmed.parse() {
                config.file_timeout_seconds = v;
            }
        }
        28 => {
            if let Ok(v) = trimmed.parse() {
                config.max_retries = v;
            }
        }
        _ => {}
    }
    // Invalid numeric input (parse failure) is silently ignored, leaving
    // the field at its previous value - matches both existing heads'
    // "live regex validation, not a submit-time popup" philosophy of
    // failing quietly rather than blocking the whole form on one bad field.
}

fn apply_toggle(config: &mut SearchToolConfig, idx: usize, kind: FieldKind) {
    match kind {
        FieldKind::Bool => {
            if let Some(value) = bool_field_mut(config, idx) {
                *value = !*value;
            }
        }
        FieldKind::MatchMode => {
            config.match_mode = match config.match_mode {
                MatchMode::AnyLine => MatchMode::AllInFile,
                MatchMode::AllInFile => MatchMode::Proximity,
                MatchMode::Proximity => MatchMode::AnyLine,
            };
        }
        FieldKind::ExcludeScope => {
            config.exclude_scope = match config.exclude_scope {
                ExcludeScope::Line => ExcludeScope::File,
                ExcludeScope::File => ExcludeScope::Line,
            };
        }
        FieldKind::GroupBy => {
            config.group_by = match config.group_by {
                GroupByMode::Created => GroupByMode::Modified,
                GroupByMode::Modified => GroupByMode::None,
                GroupByMode::None => GroupByMode::Created,
            };
        }
        FieldKind::FastIndexLocation => {
            config.index.location = config.index.location.cycle();
        }
        FieldKind::Text | FieldKind::Number => {}
    }
}

fn bool_field_mut(config: &mut SearchToolConfig, idx: usize) -> Option<&mut bool> {
    match idx {
        6 => Some(&mut config.use_regex),
        7 => Some(&mut config.whole_word),
        11 => Some(&mut config.include_hidden),
        16 => Some(&mut config.export_html),
        17 => Some(&mut config.export_csv),
        18 => Some(&mut config.export_json),
        19 => Some(&mut config.open_report_when_done),
        20 => Some(&mut config.parallel),
        24 => Some(&mut config.dry_run),
        26 => Some(&mut config.ocr_scanned_pdfs),
        29 => Some(&mut config.index.enabled),
        _ => None,
    }
}

fn bool_field(config: &SearchToolConfig, idx: usize) -> bool {
    match idx {
        6 => config.use_regex,
        7 => config.whole_word,
        11 => config.include_hidden,
        16 => config.export_html,
        17 => config.export_csv,
        18 => config.export_json,
        19 => config.open_report_when_done,
        20 => config.parallel,
        24 => config.dry_run,
        26 => config.ocr_scanned_pdfs,
        29 => config.index.enabled,
        _ => false,
    }
}

fn display_value(config: &SearchToolConfig, idx: usize, kind: FieldKind) -> String {
    match kind {
        FieldKind::Bool => {
            if bool_field(config, idx) {
                "[x]".to_string()
            } else {
                "[ ]".to_string()
            }
        }
        FieldKind::MatchMode => match config.match_mode {
            MatchMode::AnyLine => "Any line".to_string(),
            MatchMode::AllInFile => "All in file".to_string(),
            MatchMode::Proximity => "Proximity".to_string(),
        },
        FieldKind::ExcludeScope => format!("{:?}", config.exclude_scope),
        FieldKind::GroupBy => format!("{:?}", config.group_by),
        FieldKind::FastIndexLocation => super::index_view::display_location(config.index.location).to_string(),
        FieldKind::Text | FieldKind::Number => field_value_string(config, idx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::persistence::PersistedSearchSettings;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::SHIFT, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    // Regression: crossterm reports Shift+a (or Caps Lock) as `Char('A')`
    // with `SHIFT` set, not `Char('a')` with empty modifiers - same class
    // of bug as Shift+S failing to open Settings (see search/mod.rs).
    #[test]
    fn shift_or_caps_a_starts_naming_a_new_preset_same_as_plain_a() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = Vec::new();
        handle_key(&mut view, &mut config, &[], &mut presets, shift_key(KeyCode::Char('A')));
        assert!(view.naming_preset);
    }

    #[test]
    fn move_selection_never_lands_on_a_header_row_across_a_full_lap() {
        let rows = settings_field_rows();
        let mut view = SettingsView::default();
        for _ in 0..rows.len() * 2 {
            view.move_selection(1);
            assert!(matches!(rows[view.selected], SettingsFieldRow::Field(_)), "landed on row {} which is a Header", view.selected);
        }
        for _ in 0..rows.len() * 2 {
            view.move_selection(-1);
            assert!(matches!(rows[view.selected], SettingsFieldRow::Field(_)), "landed on row {} which is a Header", view.selected);
        }
    }

    #[test]
    fn clamp_selection_nudges_off_a_header_row() {
        // Simulates a mouse click landing exactly on a header divider
        // (`app.rs::handle_settings_click` sets `selected` directly from
        // the clicked row before calling this).
        let rows = settings_field_rows();
        let header_pos = rows.iter().position(|r| matches!(r, SettingsFieldRow::Header(_))).expect("at least one header exists");
        let mut view = SettingsView { selected: header_pos, ..Default::default() };
        view.clamp_selection();
        assert!(matches!(rows[view.selected], SettingsFieldRow::Field(_)));
    }

    #[test]
    fn draw_fields_renders_every_header_divider() {
        let mut regions = crate::mouse::MouseRegions::default();
        let backend = ratatui::backend::TestBackend::new(60, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let view = SettingsView::default();
        let config = SearchToolConfig::default();
        terminal.draw(|f| draw_fields(f, f.area(), &Theme::default_palette(), &config, &view, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered: Vec<String> = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect();
        let joined = rendered.join("\n");
        for (_, label) in SETTINGS_HEADERS {
            assert!(joined.contains(&format!("-- {label} --")), "missing header `{label}`:\n{joined}");
        }
    }

    #[test]
    fn tab_cycles_through_all_three_sections_and_wraps() {
        let mut view = SettingsView::default();
        let mut config = SearchToolConfig::default();
        assert_eq!(view.section, Section::Fields);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Tab));
        assert_eq!(view.section, Section::Recents);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Tab));
        assert_eq!(view.section, Section::Presets);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Tab));
        assert_eq!(view.section, Section::Fields);
    }

    #[test]
    fn a_in_presets_section_starts_naming_a_new_preset() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = Vec::new();
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('a')));
        assert!(view.naming_preset);
    }

    #[test]
    fn committing_a_preset_name_saves_the_current_config_and_persists() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig { search_path: "/tmp/save-me".to_string(), ..Default::default() };
        let mut presets = Vec::new();
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('a')));
        for c in "My preset".chars() {
            handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char(c)));
        }
        let (consumed, effects) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Enter));
        assert!(consumed);
        assert!(!view.naming_preset);
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "My preset");
        assert_eq!(presets[0].settings.search_path, "/tmp/save-me");
        assert!(matches!(effects.as_slice(), [Effect::PersistSearchSettings]));
    }

    #[test]
    fn committing_an_empty_preset_name_saves_nothing() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = Vec::new();
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('a')));
        let (_, effects) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Enter));
        assert!(presets.is_empty());
        assert!(effects.is_empty());
    }

    #[test]
    fn esc_while_naming_a_preset_discards_without_saving() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = Vec::new();
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('a')));
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('x')));
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Esc));
        assert!(!view.naming_preset);
        assert!(presets.is_empty());
    }

    #[test]
    fn r_in_presets_section_starts_renaming_prefilled_with_current_name() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = vec![SavedPreset { name: "original".to_string(), settings: PersistedSearchSettings::from(&config) }];
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('r')));
        assert!(view.renaming_preset);
        assert_eq!(view.edit_buffer, "original");
    }

    #[test]
    fn committing_a_rename_updates_the_preset_name_in_place_and_persists() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = vec![
            SavedPreset { name: "one".to_string(), settings: PersistedSearchSettings::from(&config) },
            SavedPreset { name: "two".to_string(), settings: PersistedSearchSettings::from(&config) },
        ];
        view.preset_selected = 1;
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('r')));
        view.edit_buffer.clear();
        for c in "renamed".chars() {
            handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char(c)));
        }
        let (consumed, effects) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Enter));
        assert!(consumed);
        assert!(!view.renaming_preset);
        assert_eq!(presets.len(), 2, "rename must not add or remove presets");
        assert_eq!(presets[0].name, "one", "unrelated preset must be untouched");
        assert_eq!(presets[1].name, "renamed");
        assert!(matches!(effects.as_slice(), [Effect::PersistSearchSettings]));
    }

    #[test]
    fn committing_an_empty_rename_leaves_the_preset_name_unchanged() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = vec![SavedPreset { name: "original".to_string(), settings: PersistedSearchSettings::from(&config) }];
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('r')));
        view.edit_buffer.clear();
        let (_, effects) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Enter));
        assert_eq!(presets[0].name, "original");
        assert!(effects.is_empty());
    }

    #[test]
    fn esc_while_renaming_a_preset_discards_without_changing_the_name() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = vec![SavedPreset { name: "original".to_string(), settings: PersistedSearchSettings::from(&config) }];
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('r')));
        for c in "xxx".chars() {
            handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char(c)));
        }
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Esc));
        assert!(!view.renaming_preset);
        assert_eq!(presets[0].name, "original");
    }

    #[test]
    fn r_on_an_empty_preset_list_does_not_panic() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = Vec::new();
        let (consumed, _) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('r')));
        assert!(!consumed);
        assert!(!view.renaming_preset);
    }

    #[test]
    fn d_deletes_the_selected_preset_and_persists() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = vec![
            SavedPreset { name: "one".to_string(), settings: PersistedSearchSettings::from(&config) },
            SavedPreset { name: "two".to_string(), settings: PersistedSearchSettings::from(&config) },
        ];
        let (consumed, effects) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('d')));
        assert!(consumed);
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "two");
        assert!(matches!(effects.as_slice(), [Effect::PersistSearchSettings]));
    }

    #[test]
    fn d_on_an_empty_preset_list_does_not_panic() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut presets = Vec::new();
        let (consumed, _) = handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Char('d')));
        assert!(!consumed);
    }

    #[test]
    fn enter_on_a_recent_search_applies_it_to_config() {
        let mut view = SettingsView { section: Section::Recents, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let recents = vec![RecentSearch { search_path: "/tmp/project".to_string(), filters_text: "needle".to_string() }];
        handle_key(&mut view, &mut config, &recents, &mut Vec::new(), key(KeyCode::Enter));
        assert_eq!(config.search_path, "/tmp/project");
        assert_eq!(config.filters_text, "needle");
    }

    #[test]
    fn enter_on_a_saved_preset_applies_every_field() {
        let mut view = SettingsView { section: Section::Presets, ..Default::default() };
        let mut config = SearchToolConfig::default();
        let mut preset_settings = PersistedSearchSettings::from(&SearchToolConfig {
            search_path: "/tmp/preset-path".to_string(),
            use_regex: true,
            ..Default::default()
        });
        preset_settings.filters_text = "from-preset".to_string();
        let mut presets = vec![SavedPreset { name: "My preset".to_string(), settings: preset_settings }];
        handle_key(&mut view, &mut config, &[], &mut presets, key(KeyCode::Enter));
        assert_eq!(config.search_path, "/tmp/preset-path");
        assert_eq!(config.filters_text, "from-preset");
        assert!(config.use_regex);
    }

    #[test]
    fn navigation_within_an_empty_recents_list_does_not_panic() {
        let mut view = SettingsView { section: Section::Recents, ..Default::default() };
        let mut config = SearchToolConfig::default();
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Down));
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Up));
        assert_eq!(view.recent_selected, 0);
    }

    #[test]
    fn every_field_index_used_by_bool_or_value_helpers_is_in_bounds() {
        // Every match arm above indexes into FIELDS conceptually via a
        // literal - this test just guards against FIELDS shrinking below
        // the highest literal index used anywhere in this file without a
        // compile error (the match arms would silently become dead code
        // rather than fail to build).
        assert!(FIELDS.len() > 28, "a field index literal above assumes at least 29 fields");
    }

    #[test]
    fn space_toggles_a_boolean_field() {
        let mut view = SettingsView { selected: row_index_for_field(6), ..Default::default() }; // Use regex
        let mut config = SearchToolConfig::default();
        assert!(!config.use_regex);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Char(' ')));
        assert!(config.use_regex);
    }

    #[test]
    fn enter_cycles_match_mode() {
        let mut view = SettingsView { selected: row_index_for_field(4), ..Default::default() }; // Match mode
        let mut config = SearchToolConfig::default();
        assert_eq!(config.match_mode, MatchMode::AnyLine);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert_eq!(config.match_mode, MatchMode::AllInFile);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert_eq!(config.match_mode, MatchMode::Proximity);
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert_eq!(config.match_mode, MatchMode::AnyLine);
    }

    #[test]
    fn enter_on_a_text_field_starts_editing_prefilled_with_the_current_value() {
        let mut view = SettingsView { selected: row_index_for_field(0), ..Default::default() }; // Search path
        let mut config = SearchToolConfig { search_path: "/tmp/x".to_string(), ..Default::default() };
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert!(view.editing);
        assert_eq!(view.edit_buffer, "/tmp/x");
    }

    #[test]
    fn editing_and_committing_a_text_field_updates_config() {
        let mut view = SettingsView { selected: row_index_for_field(0), ..Default::default() };
        let mut config = SearchToolConfig::default();
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        for c in "/tmp/new".chars() {
            handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Char(c)));
        }
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert!(!view.editing);
        assert_eq!(config.search_path, "/tmp/new");
    }

    #[test]
    fn esc_cancels_an_edit_without_committing() {
        let mut view = SettingsView { selected: row_index_for_field(0), ..Default::default() };
        let mut config = SearchToolConfig { search_path: "/original".to_string(), ..Default::default() };
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Char('x')));
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Esc));
        assert!(!view.editing);
        assert_eq!(config.search_path, "/original");
    }

    #[test]
    fn invalid_numeric_input_is_ignored_not_panicking() {
        let mut view = SettingsView { selected: 5, ..Default::default() }; // Proximity lines
        let mut config = SearchToolConfig::default();
        let original = config.proximity_lines;
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        for c in "not-a-number".chars() {
            handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Char(c)));
        }
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert_eq!(config.proximity_lines, original);
    }

    #[test]
    fn match_mode_displays_friendly_labels_not_raw_debug_names() {
        // Matches `app/`'s and `app-egui/`'s own display strings exactly -
        // a prior version of this file used `format!("{:?}", ...)`, which
        // renders "AnyLine"/"AllInFile" (no space, Rust Debug format)
        // instead of "Any line"/"All in file".
        let any_line = SearchToolConfig { match_mode: MatchMode::AnyLine, ..Default::default() };
        assert_eq!(display_value(&any_line, 4, FieldKind::MatchMode), "Any line");
        let all_in_file = SearchToolConfig { match_mode: MatchMode::AllInFile, ..Default::default() };
        assert_eq!(display_value(&all_in_file, 4, FieldKind::MatchMode), "All in file");
        let proximity = SearchToolConfig { match_mode: MatchMode::Proximity, ..Default::default() };
        assert_eq!(display_value(&proximity, 4, FieldKind::MatchMode), "Proximity");
    }

    #[test]
    fn up_down_navigation_wraps() {
        let mut view = SettingsView::default();
        let start = view.selected;
        assert_eq!(start, row_index_for_field(0), "default selection must land on the first real field, not a Header row");
        view.move_selection(-1);
        assert_eq!(view.selected, row_index_for_field(FIELDS.len() - 1), "must wrap to the last real field, skipping any trailing Header");
        view.move_selection(1);
        assert_eq!(view.selected, start);
    }

    #[test]
    fn navigation_is_ignored_while_editing() {
        let mut view = SettingsView { selected: row_index_for_field(0), ..Default::default() };
        let mut config = SearchToolConfig::default();
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Down));
        assert_eq!(view.selected, row_index_for_field(0));
    }

    #[test]
    fn enter_on_the_extensions_field_requests_a_folder_scan_instead_of_text_editing() {
        // Superseded by the extension-checkbox picker: Enter here now opens
        // that picker (a folder scan) rather than starting a plain text
        // edit on a comma list. The full open-scan-select-commit round trip
        // is exercised in `mod.rs`'s own integration test, since committing
        // the picker's selection into `config.selected_extensions` happens
        // one layer up (`toolboxes::search::handle_key`), not here.
        let mut view = SettingsView { selected: row_index_for_field(EXTENSIONS_FIELD), ..Default::default() };
        let mut config = SearchToolConfig::default();
        let (consumed, effects) = handle_key(&mut view, &mut config, &[], &mut Vec::new(), key(KeyCode::Enter));
        assert!(consumed);
        assert!(!view.editing, "must not fall back to plain text editing");
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ScanExtensions { root, .. } => assert_eq!(root, &config.search_path),
            other => panic!("expected ScanExtensions, got {other:?}"),
        }
    }

    #[test]
    fn every_field_has_a_nonempty_hint() {
        for i in 0..FIELDS.len() {
            assert!(!field_hint(i).is_empty(), "field {i} (`{}`) has no hint text", FIELDS[i].label);
        }
    }

    /// Regression test for the hint-truncation bug (see `bushing/view.rs`'s
    /// twin test): selects the field with the single longest hint and
    /// asserts every word of it appears somewhere in the rendered buffer,
    /// across a range of widths including narrow ones.
    #[test]
    fn the_longest_hint_is_never_truncated_across_a_range_of_widths() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let (longest_index, longest_hint) = (0..FIELDS.len()).map(|i| (i, field_hint(i))).max_by_key(|(_, hint)| hint.len()).expect("at least one field");
        assert!(!longest_hint.is_empty());

        for width in [40u16, 50, 60, 84, 98, 140] {
            let view = SettingsView { selected: row_index_for_field(longest_index), ..Default::default() };
            let config = SearchToolConfig::default();
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &config, &[], &[], &view, &mut regions)).unwrap();

            let buffer = terminal.backend().buffer().clone();
            let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();

            for word in longest_hint.split_whitespace() {
                assert!(rendered.contains(word), "word `{word}` from the longest hint missing at width {width}:\n{rendered}");
            }
        }
    }

    #[test]
    fn draw_does_not_panic_at_degenerate_sizes() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let view = SettingsView::default();
        let config = SearchToolConfig::default();
        for (w, h) in [(0u16, 0u16), (1, 1), (40, 0), (0, 10)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| draw(f, ratatui::layout::Rect { x: 0, y: 0, width: w, height: h }, &Theme::default_palette(), &config, &[], &[], &view, &mut regions)).unwrap();
        }
    }
}
