//! The Search workbench in the native window: the terminal Search Files toolbox's own state, run, index and report
//! (`toolboxes::search`: `SearchToolState`, `run_or_notify`, `apply_event`, `exec::execute`) behind the CAD chrome.
//! It is a bench without a 3D model: its page (`Authoring::document`) is a search bar, live progress, the results
//! table and the selected file's highlighted matches, drawn where a model would be. Options are inspector rows over
//! the terminal form's own settings table (`settings_view::settings`), so labels, parsing and persistence are shared.
//!
//! Background work runs on this session's own tokio runtime and reports through the same `AppEvent`s the terminal
//! uses; `poll` folds them in with the shared reducer. Nothing here searches, matches or extracts: `search-core` and
//! `native-search` do.
mod view;
#[cfg(test)]
mod tests;

use super::{
    authoring::{Authoring, Card, StepState},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::app::{AppEvent, Effect};
use crate::gpu_viewer::{
    bench::Bench,
    menu::{Item, Menu},
    ui::{DockItem, Document, RibbonTab, TreeNode, ValueCard},
    Scene,
};
use crate::notifications::NotificationQueue;
use crate::toolboxes::search::{self as search, exec, model, persistence, settings_view, SearchToolState};
use search_core::models::FileSearchResult;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;

/// What a click in the Search workbench asks for (carried by `EditorAction::Search`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAct {
    Run,
    Cancel,
    /// Select result `.0` (an index into the run's results).
    Select(usize),
    /// Open result `.0` with the system's default program.
    Open(usize),
    OpenSelected,
    Reveal,
    CopyPath,
    ExportHits,
    OpenReport,
    SortBy(SortKey),
    /// How the filters combine (the global mode; a group's `[tag]` overrides it).
    Match(search_core::models::MatchMode),
    /// Show only results of extension `.0` (an index into `types()`); `None` shows all.
    OnlyType(Option<usize>),
    /// A setting of the terminal form (`settings_view::settings`): type a value, or flip / cycle it.
    Edit(usize),
    Step(usize),
    ApplyEdit,
    CancelEdit,
    /// Choose the folder with the operating system's folder picker.
    BrowseFolder,
    /// Choose the folder reports and exports are written to, with the same picker.
    BrowseOutput,
    BuildIndex,
    CancelIndex,
    UpdateStaleIndex,
    DismissStale,
    PickExtensions,
    ToggleExtension(usize),
    AllExtensions,
    CloseExtensions,
    ApplyRecent(usize),
    ApplyPreset(usize),
    DeletePreset(usize),
    SavePreset,
    ClearRecents,
    /// Bring the tools of a step (the tree).
    StepTab(&'static str),
    /// Show or hide the Exclude row under the bar.
    ToggleExclude,
    /// What an exclusion leaves out: the whole file or only the lines.
    ExcludeScope(search_core::models::ExcludeScope),
}

/// What the results table is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// The order the files were found in.
    Found,
    Name,
    Hits,
    Size,
    Modified,
}

/// A typed value being edited in a value card: a setting (`Some(index)`) or a new preset's name (`None`).
struct Edit {
    setting: Option<usize>,
    text: String,
}

/// Inputs of the page (`DocInput::id`).
pub(crate) const INPUT_FOLDER: u8 = 0;
pub(crate) const INPUT_FIND: u8 = 1;
pub(crate) const INPUT_FILTER: u8 = 2;
pub(crate) const INPUT_EXCLUDE: u8 = 3;
pub(crate) const INPUT_OUTPUT: u8 = 4;
/// Settings shown on the page instead of in a popover (`settings_view::settings` indices).
const SETTING_PATH: usize = 0;
const SETTING_FILTERS: usize = 2;
const SETTING_EXCLUDE: usize = 3;
const SETTING_EXCLUDE_SCOPE: usize = 8;
const SETTING_OUTPUT: usize = 14;

pub struct SearchSession {
    st: SearchToolState,
    notes: NotificationQueue,
    /// Owns the search, report, index and scan tasks; dropped with the window.
    rt: tokio::runtime::Runtime,
    tx: mpsc::UnboundedSender<AppEvent>,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    status: String,
    step: &'static str,
    /// The results filter (file name or folder text, case-insensitive).
    filter: String,
    sort: (SortKey, bool),
    /// The selected result (index into `st.run.results`).
    selected: Option<usize>,
    edit: Option<Edit>,
    /// Asks for a folder, starting in the given one (the system dialog; tests answer without one).
    pick_folder: fn(&Path) -> Option<PathBuf>,
    /// The fast index went out of date during the last run: offer an update.
    stale: Option<(usize, usize)>,
    index_started: Option<Instant>,
    /// The Exclude row is shown (it opens shown when the saved settings already exclude something).
    show_exclude: bool,
    /// Write `settings-tui.json` (off in tests, which must never touch the user's file).
    persist: bool,
    /// Where the settings live (`Authoring::path`).
    settings_file: PathBuf,
}

impl Drop for SearchSession {
    fn drop(&mut self) {
        search::request_cancel(&self.st);
        if let Some(c) = &self.st.index_cancel {
            c.cancel();
        }
        if self.persist {
            persistence::save(&self.st.to_persisted_file());
        }
    }
}

impl SearchSession {
    /// A session on the saved Search settings (shared with the terminal toolbox); `folder` replaces the saved
    /// search folder when given.
    pub fn open(folder: Option<&Path>) -> Result<Self, String> {
        let mut s = Self::with_state(SearchToolState::default(), !cfg!(test))?;
        if let Some(f) = folder.filter(|f| !f.as_os_str().is_empty()) {
            s.st.config.search_path = f.to_string_lossy().into_owned();
        }
        Ok(s)
    }

    /// A session on default settings that never reads or writes the user's settings file.
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        let mut s = Self::with_state(SearchToolState::default(), false).expect("runtime");
        // Never a real dialog in a test: Browse answers "cancelled" unless a test sets its own picker.
        s.pick_folder = |_| None;
        s
    }
    #[cfg(test)]
    pub(crate) fn set_folder_picker(&mut self, pick: fn(&Path) -> Option<PathBuf>) {
        self.pick_folder = pick;
    }

    fn with_state(st: SearchToolState, persist: bool) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("search").build().map_err(|e| format!("cannot start the search workers: {e}"))?;
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Self {
            notes: NotificationQueue::default(),
            rt,
            tx,
            rx,
            status: "Choose a folder, type what to find and press Enter.".into(),
            step: "Search",
            filter: String::new(),
            sort: (SortKey::Found, false),
            selected: None,
            edit: None,
            pick_folder: system_folder_picker,
            stale: None,
            index_started: None,
            show_exclude: !st.config.exclude_filters_text.trim().is_empty(),
            persist,
            st,
            settings_file: crate::paths::app_data_dir().map_or_else(|| PathBuf::from("settings-tui.json"), |d| d.join("settings-tui.json")),
        })
    }

    pub fn state(&self) -> &SearchToolState {
        &self.st
    }

    /// Put the session in the middle of a run (for the snapshot of the progress card): a quarter done, three files
    /// being read, one of them slow.
    #[cfg(test)]
    pub(crate) fn state_mut_for_snapshot(&mut self) {
        use search_core::models::InFlightFileStatus;
        let root = self.st.config.search_path.clone();
        let run = &mut self.st.run;
        run.is_running = true;
        run.started = Some(Instant::now() - std::time::Duration::from_secs(14));
        run.elapsed = None;
        run.files_completed = 1_204;
        run.total_files = 4_810;
        run.status_text = "1204 of 4810 file(s) - 37 hit(s) so far".into();
        run.in_flight_files = vec![
            InFlightFileStatus { file_name: format!("{root}/specs/Structural Repair Manual 51-10.pdf"), status_text: "Extracting PDF text".into(), elapsed_seconds: 6.4 },
            InFlightFileStatus { file_name: format!("{root}/notes/meeting-2026-09.docx"), status_text: "Reading".into(), elapsed_seconds: 0.3 },
            InFlightFileStatus { file_name: format!("{root}/archive/drawings.zip"), status_text: "Opening archive".into(), elapsed_seconds: 1.1 },
        ];
        self.stale = None;
    }

    // ------------------------------------------------------------------ effects and events

    fn execute(&mut self, effects: Vec<Effect>) {
        let _rt = self.rt.enter();
        for e in effects {
            if matches!(e, Effect::PersistSearchSettings) && !self.persist {
                continue;
            }
            // Every effect the Search toolbox returns is a Search effect; anything else would be a bug upstream.
            let _ = exec::execute(&self.tx, &mut self.st, e);
        }
    }
    fn save_settings(&mut self) {
        self.execute(vec![Effect::PersistSearchSettings]);
    }
    /// The newest notification becomes the status line.
    fn take_note(&mut self) {
        if let Some(t) = self.notes.visible().last() {
            self.status = t.message.clone();
        }
        self.notes = NotificationQueue::default();
    }

    pub fn run(&mut self) {
        if self.st.run.is_running {
            self.status = "A search is already running (Esc stops it).".into();
            return;
        }
        if let Some(e) = model::regex_validation_error(&self.st.config) {
            self.status = format!("Fix the pattern first: {e}");
            return;
        }
        let effects = search::run_or_notify(&mut self.st, &mut self.notes);
        self.take_note();
        if !effects.is_empty() {
            self.selected = None;
            self.stale = None;
            self.status = format!("Searching {}...", self.st.config.search_path.trim());
            self.execute(effects);
            self.save_settings();
        }
    }

    fn cancel(&mut self) {
        if self.st.run.is_running {
            search::request_cancel(&self.st);
            self.status = "Stopping the search...".into();
        } else if let Some(c) = &self.st.index_cancel {
            c.cancel();
            self.status = "Stopping the index build (what is indexed is kept)...".into();
        }
    }

    fn build_index(&mut self) {
        if self.st.run.is_running {
            self.status = "Wait for the search to finish before building the index.".into();
            return;
        }
        let effects = search::start_index_build(&mut self.st);
        if effects.is_empty() {
            self.status = if self.st.index_run.is_building { "The index is already building.".into() } else { "Choose a folder first.".into() };
            return;
        }
        self.index_started = Some(Instant::now());
        self.status = "Building the fast index...".into();
        self.execute(effects);
    }

    // ------------------------------------------------------------------ results

    /// The results in table order: filtered, then sorted.
    pub(crate) fn view(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        let results = &self.st.run.results;
        let mut v: Vec<usize> = (0..results.len()).filter(|&i| needle.is_empty() || results[i].full_name.to_lowercase().contains(&needle)).collect();
        let (key, asc) = self.sort;
        let name = |r: &FileSearchResult| file_name(&r.full_name).to_lowercase();
        match key {
            SortKey::Found => {}
            SortKey::Name => v.sort_by_key(|&i| name(&results[i])),
            SortKey::Hits => v.sort_by_key(|&i| results[i].hits.len()),
            SortKey::Size => v.sort_by_key(|&i| results[i].file_length),
            SortKey::Modified => v.sort_by_key(|&i| results[i].modified),
        }
        if key != SortKey::Found && !asc {
            v.reverse();
        }
        v
    }
    /// The selected result, or the first shown when nothing is selected.
    pub(crate) fn current(&self) -> Option<usize> {
        let view = self.view();
        self.selected.filter(|s| view.contains(s)).or_else(|| view.first().copied())
    }
    fn current_result(&self) -> Option<&FileSearchResult> {
        self.current().and_then(|i| self.st.run.results.get(i))
    }
    fn move_selection(&mut self, delta: i32) {
        let view = self.view();
        if view.is_empty() {
            return;
        }
        let at = self.current().and_then(|c| view.iter().position(|&v| v == c)).unwrap_or(0) as i32;
        self.selected = Some(view[(at + delta).clamp(0, view.len() as i32 - 1) as usize]);
    }
    /// Extensions among the results with their counts, most common first.
    pub(crate) fn types(&self) -> Vec<(String, usize)> {
        crate::format::extension_breakdown(&self.st.run.results)
    }

    fn with_current(&mut self, f: impl FnOnce(&FileSearchResult, &model::SearchToolConfig) -> Effect, what: &str) {
        match self.current_result() {
            Some(r) => {
                let effect = f(r, &self.st.config);
                self.status = format!("{what}: {}", file_name(&r.full_name));
                self.execute(vec![effect]);
            }
            None => self.status = "Select a result first.".into(),
        }
    }

    // ------------------------------------------------------------------ settings

    fn edit_setting(&mut self, idx: usize) {
        self.edit = Some(Edit { setting: Some(idx), text: settings_view::setting_text(&self.st.config, idx) });
    }
    fn apply_edit(&mut self) {
        let Some(edit) = self.edit.take() else { return };
        match edit.setting {
            Some(idx) => {
                let before = self.st.config.clone();
                settings_view::set_setting_text(&mut self.st.config, idx, &edit.text);
                let label = setting_label(idx);
                self.status = if self.st.config == before && edit.text.trim() != settings_view::setting_text(&before, idx).trim() { format!("{label}: not a valid value, kept {}.", settings_view::setting_display(&before, idx)) } else { format!("{label}: {}", settings_view::setting_display(&self.st.config, idx)) };
                self.save_settings();
            }
            None => {
                let name = edit.text.trim();
                if name.is_empty() {
                    self.status = "A preset needs a name.".into();
                    return;
                }
                self.st.saved_presets.push(persistence::SavedPreset { name: name.to_string(), settings: persistence::PersistedSearchSettings::from(&self.st.config) });
                self.status = format!("Saved preset '{name}'.");
                self.save_settings();
            }
        }
    }
    fn step_setting(&mut self, idx: usize) {
        settings_view::step_setting(&mut self.st.config, idx);
        self.status = format!("{}: {}", setting_label(idx), settings_view::setting_display(&self.st.config, idx));
        self.save_settings();
    }

    /// The system's folder picker, opened in the first of `starts` that is a folder (or the working folder).
    fn pick(&mut self, starts: &[&str]) -> Option<PathBuf> {
        let start = starts.iter().map(|s| PathBuf::from(s.trim())).find(|p| !p.as_os_str().is_empty() && p.is_dir()).unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let picked = (self.pick_folder)(&start);
        if picked.is_none() {
            self.status = "No folder chosen.".into();
        }
        picked
    }
    /// Browse: the folder to search, starting in the current one.
    fn browse(&mut self) {
        let current = self.st.config.search_path.clone();
        if let Some(dir) = self.pick(&[&current]) {
            self.st.config.search_path = dir.to_string_lossy().into_owned();
            self.status = format!("Folder: {}. Type what to find and press Enter.", dir.display());
            self.save_settings();
        }
    }
    /// Browse for the output folder, starting in it (or in the folder being searched).
    fn browse_output(&mut self) {
        let (current, searched) = (self.st.config.output_folder.clone(), self.st.config.search_path.clone());
        if let Some(dir) = self.pick(&[&current, &searched]) {
            self.st.config.output_folder = dir.to_string_lossy().into_owned();
            self.status = format!("Reports and exports go to {}.", dir.display());
            self.save_settings();
        }
    }

    fn act_search(&mut self, a: SearchAct) {
        match a {
            SearchAct::Run => self.run(),
            SearchAct::Cancel => self.cancel(),
            SearchAct::Select(i) => {
                if i < self.st.run.results.len() {
                    self.selected = Some(i);
                    self.step = "Results";
                }
            }
            SearchAct::Open(i) => {
                self.selected = Some(i);
                self.act_search(SearchAct::OpenSelected);
            }
            SearchAct::OpenSelected => self.with_current(|r, _| Effect::OpenPath(r.full_name.clone()), "Opened"),
            SearchAct::Reveal => self.with_current(|r, _| Effect::OpenPath(Path::new(&r.full_name).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()), "Showing the folder of"),
            SearchAct::CopyPath => self.with_current(|r, _| Effect::CopyToClipboard(r.full_name.clone()), "Copied the path of"),
            SearchAct::ExportHits => self.with_current(export_hits, "Exported the matches of"),
            SearchAct::OpenReport => match self.st.run.last_report_path.clone() {
                Some(p) => {
                    self.status = "Opened the report.".into();
                    self.execute(vec![Effect::OpenPath(p)]);
                }
                None => self.status = "No report yet: it is written when a search finishes (HTML export on).".into(),
            },
            SearchAct::SortBy(k) => {
                self.sort = if self.sort.0 == k { (k, !self.sort.1) } else { (k, matches!(k, SortKey::Name)) };
            }
            SearchAct::OnlyType(t) => {
                self.filter = t.and_then(|i| self.types().get(i).map(|(e, _)| e.clone())).unwrap_or_default();
                self.step = "Results";
            }
            SearchAct::Edit(idx) => match settings_view::settings().nth(idx).map(|s| s.4) {
                Some(settings_view::SettingKind::Typed) if idx == settings_view::EXTENSIONS_FIELD => self.act_search(SearchAct::PickExtensions),
                Some(settings_view::SettingKind::Typed) if idx == SETTING_OUTPUT => self.browse_output(),
                Some(settings_view::SettingKind::Typed) => self.edit_setting(idx),
                Some(_) => self.step_setting(idx),
                None => {}
            },
            SearchAct::Step(idx) => self.step_setting(idx),
            SearchAct::Match(m) => {
                self.st.config.match_mode = m;
                self.status = format!("Match: {}", model::mode_label(model::GroupMode::Match(m, None), self.st.config.proximity_lines));
                self.save_settings();
            }
            SearchAct::ApplyEdit => self.apply_edit(),
            SearchAct::CancelEdit => self.edit = None,
            SearchAct::BrowseFolder => self.browse(),
            SearchAct::BrowseOutput => self.browse_output(),
            SearchAct::BuildIndex => self.build_index(),
            SearchAct::CancelIndex => {
                if let Some(c) = &self.st.index_cancel {
                    c.cancel();
                    self.status = "Stopping the index build (what is indexed is kept)...".into();
                }
            }
            SearchAct::UpdateStaleIndex => {
                self.stale = None;
                self.build_index();
            }
            SearchAct::DismissStale => self.stale = None,
            SearchAct::PickExtensions => {
                let root = self.st.config.search_path.trim().to_string();
                if root.is_empty() {
                    self.status = "Choose a folder first: the file types are read from it.".into();
                    return;
                }
                self.status = "Reading the file types in the folder...".into();
                let effect = Effect::ScanExtensions { root, exclude_folders: model::parse_list(&self.st.config.exclude_folders_text), include_hidden: self.st.config.include_hidden };
                self.execute(vec![effect]);
            }
            SearchAct::ToggleExtension(i) => {
                let picker = &mut self.st.extension_picker;
                if let Some(ext) = picker.available.get(i).cloned() {
                    if !picker.selected.remove(&ext) {
                        picker.selected.insert(ext);
                    }
                }
            }
            SearchAct::AllExtensions => self.st.extension_picker.selected.clear(),
            SearchAct::CloseExtensions => {
                self.st.extension_picker.open = false;
                if search::commit_extension_picker(&mut self.st) {
                    self.save_settings();
                }
                self.status = match &self.st.config.selected_extensions {
                    Some(list) => format!("Searching only {}.", list.join(", ")),
                    None => "Searching every supported file type.".into(),
                };
            }
            SearchAct::ApplyRecent(i) => {
                if let Some(r) = self.st.recent_searches.get(i).cloned() {
                    self.st.config.search_path = r.search_path;
                    self.st.config.filters_text = r.filters_text;
                    self.status = "Recent search loaded: press Enter to run it.".into();
                }
            }
            SearchAct::ApplyPreset(i) => {
                if let Some(p) = self.st.saved_presets.get(i).cloned() {
                    p.settings.apply_to(&mut self.st.config);
                    self.show_exclude |= !self.st.config.exclude_filters_text.trim().is_empty();
                    self.status = format!("Preset '{}' applied.", p.name);
                    self.save_settings();
                }
            }
            SearchAct::DeletePreset(i) => {
                if i < self.st.saved_presets.len() {
                    let p = self.st.saved_presets.remove(i);
                    self.status = format!("Preset '{}' deleted.", p.name);
                    self.save_settings();
                }
            }
            SearchAct::SavePreset => {
                self.edit = Some(Edit { setting: None, text: String::new() });
            }
            SearchAct::ClearRecents => {
                self.st.recent_searches.clear();
                self.status = "Recent searches cleared.".into();
                self.save_settings();
            }
            SearchAct::StepTab(s) => self.step = s,
            SearchAct::ToggleExclude => {
                self.show_exclude = !self.show_exclude;
                let terms = model::parse_list(&self.st.config.exclude_filters_text);
                self.status = match (self.show_exclude, terms.is_empty()) {
                    (true, _) => "Type what to leave out; the switch beside it picks whole files or only the lines.".into(),
                    (false, true) => "Exclude hidden.".into(),
                    (false, false) => format!("Exclude hidden; still leaving out {}.", terms.join(", ")),
                };
            }
            SearchAct::ExcludeScope(scope) => {
                self.st.config.exclude_scope = scope;
                self.status = format!("{}: {}", setting_label(SETTING_EXCLUDE_SCOPE), settings_view::setting_display(&self.st.config, SETTING_EXCLUDE_SCOPE));
                self.save_settings();
            }
        }
    }
}

/// The operating system's folder picker (modal; on the window's thread, as macOS requires).
fn system_folder_picker(start: &Path) -> Option<PathBuf> {
    #[cfg(feature = "gpu-viewer")]
    {
        rfd::FileDialog::new().set_title("Folder to search").set_directory(start).pick_folder()
    }
    #[cfg(not(feature = "gpu-viewer"))]
    {
        let _ = start;
        None
    }
}

fn export_hits(r: &FileSearchResult, config: &model::SearchToolConfig) -> Effect {
    let stem = Path::new(&r.full_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "result".into());
    let dir = if config.output_folder.trim().is_empty() { Path::new(&r.full_name).parent().map(Path::to_path_buf).unwrap_or_default() } else { PathBuf::from(config.output_folder.trim()) };
    let path = dir.join(format!("{}_hits.txt", model::sanitize_file_name(&stem))).to_string_lossy().into_owned();
    Effect::WriteTextFileAndOpen { path, contents: model::hits_as_text(r) }
}

pub(crate) fn file_name(path: &str) -> &str {
    Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path)
}

fn setting_label(idx: usize) -> &'static str {
    settings_view::settings().nth(idx).map_or("", |s| s.2)
}

pub(crate) fn act(a: SearchAct) -> crate::gpu_viewer::controller::UiAction {
    crate::gpu_viewer::controller::UiAction::Editor(EditorAction::Search(a))
}

impl Authoring for SearchSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn path(&self) -> &Path {
        &self.settings_file
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Search(a) = action {
            self.act_search(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        match key {
            Key::Up => self.move_selection(-1),
            Key::Down => self.move_selection(1),
            Key::Enter if mods.command => self.run(),
            Key::Enter => self.act_search(SearchAct::OpenSelected),
            Key::Escape if self.edit.is_some() => self.edit = None,
            Key::Escape if self.st.extension_picker.open => self.act_search(SearchAct::CloseExtensions),
            Key::Escape if self.st.run.is_running || self.st.index_run.is_building => self.cancel(),
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'c') => self.act_search(SearchAct::CopyPath),
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'e') => self.act_search(SearchAct::ExportHits),
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        Ok(Scene { label: "search".into(), positions: vec![[0.0; 3]; 3], triangles: vec![0, 1, 2], samples: vec![[0.0; 4]; 3], times: vec![0.0], range: [0.0, 1.0], gain: 1.0, harmonic_period: None, field_label: String::new(), fields: Vec::new() })
    }
    fn needs_scene(&self) -> bool {
        false
    }
    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            let finished = matches!(event, AppEvent::SearchFinished(_));
            let index_done = matches!(event, AppEvent::IndexBuildFinished(_));
            let scanned = matches!(event, AppEvent::ExtensionsScanned(_));
            let effects = search::apply_event(&mut self.st, &mut self.notes, event);
            self.take_note();
            self.execute(effects);
            if finished {
                // Offer an index update after the run, never during it (the terminal asks with a dialog).
                if let Some(s) = self.st.index_run.stale.take().filter(|_| !self.st.index_run.is_building) {
                    self.stale = Some(s);
                }
                if !self.st.run.results.is_empty() {
                    self.step = "Results";
                }
                let n = self.st.run.results.len();
                if self.st.run.status_text.starts_with("Error") || self.st.run.status_text.starts_with("Cancelled") {
                    self.status = self.st.run.status_text.clone();
                } else {
                    self.status = format!("{} file{} with matches in {}.", n, if n == 1 { "" } else { "s" }, crate::format::format_elapsed(self.st.run.elapsed_secs().unwrap_or(0.0)));
                }
            }
            if index_done {
                self.index_started = None;
            }
            if scanned {
                self.edit = None;
                if let Some(e) = &self.st.extension_picker.scan_error {
                    self.status = format!("Cannot read the file types: {e}");
                } else {
                    self.status = "Tick the file types to search; none ticked searches every supported type.".into();
                }
            }
        }
        changed
    }
    fn prompting(&self) -> bool {
        false
    }
    fn solving(&self) -> bool {
        self.st.run.is_running || self.st.index_run.is_building
    }
    fn solving_for(&self) -> Option<std::time::Duration> {
        if self.st.run.is_running {
            self.st.run.started.map(|s| s.elapsed())
        } else if self.st.index_run.is_building {
            self.index_started.map(|s| s.elapsed())
        } else {
            None
        }
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        let folder = self.st.config.search_path.trim();
        if folder.is_empty() { "Search".into() } else { format!("Search - {}", file_name(folder)) }
    }
    fn dirty(&self) -> bool {
        false
    }
    fn geometry_view(&self) -> bool {
        false
    }
    fn tree(&self) -> Vec<TreeNode> {
        view::tree(self)
    }
    fn properties(&self) -> Vec<DockItem> {
        crate::gpu_viewer::editor::authoring::flatten(self.inspector())
    }
    fn inspector(&self) -> Vec<Card> {
        view::inspector(self)
    }
    fn tree_menu(&self, node: u32) -> Vec<Item> {
        view::tree_menu(self, node)
    }
    fn focus_step(&self) -> Option<&'static str> {
        Some(self.step)
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        view::ribbon(self)
    }
    fn step_state(&self, tab: &str) -> Option<StepState> {
        match tab {
            "Search" => Some(if self.st.run.started.is_some() && !self.st.run.is_running { StepState::Done } else { StepState::Todo }),
            "Results" => Some(if self.st.run.results.is_empty() { StepState::Todo } else { StepState::Done }),
            _ => None,
        }
    }
    fn menus(&self) -> Vec<Menu> {
        view::menus(self)
    }
    fn context_menu(&self) -> Vec<Item> {
        view::context_menu(self)
    }
    fn value_card(&self) -> Option<ValueCard> {
        view::value_card(self)
    }
    fn card_field(&mut self, _index: usize, text: String) {
        if let Some(e) = &mut self.edit {
            e.text = text;
        }
    }
    fn document(&self) -> Option<Document> {
        Some(view::document(self))
    }
    fn has_document(&self) -> bool {
        true
    }
    fn document_field(&mut self, id: u8, text: String) {
        match id {
            INPUT_FOLDER => self.st.config.search_path = text,
            INPUT_FIND => self.st.config.filters_text = text,
            INPUT_FILTER => self.filter = text,
            INPUT_EXCLUDE => self.st.config.exclude_filters_text = text,
            INPUT_OUTPUT => self.st.config.output_folder = text,
            _ => {}
        }
    }
}
