//! Eccentric Bushing toolbox: spin capacity of a bushing whose bore is offset from its outer diameter, and the
//! largest offset (or load) that the interference fit still holds. A bridge over the `eccentric-bushing` solver
//! (a contact FE on `fea-core`): the bore, bushing ID, interference, friction, length, load and materials come from
//! the Bushing Workbench's live model, so nothing is entered twice; this toolbox adds only the offset, the load angle
//! and the boss size. Runs are manual (`r` / `m` / `l`) on a worker thread: they take seconds to tens of seconds.

pub mod model;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent};
use eccentric_bushing::{Analysis, Control, Inputs, OffsetLimit, SweepPoint};
use std::collections::VecDeque;

use crate::app::Effect;
use crate::toolboxes::bushing::model::BushingModel;
use model::{EccentricUi, FieldRow, Output, Task};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

/// A run on a worker thread.
pub struct Job {
    pub id: u64,
    pub task: Task,
    pub sig: Inputs,
    pub started: std::time::Instant,
    /// Stop flag (`c`), deadline and progress record shared with the worker: a stopped search reports the bracket it had,
    /// the view reads the progress.
    pub control: Control,
}

/// One finished analysis kept for comparison with the next ones.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub offset: f64,
    pub load_lbf: f64,
    pub load_angle_deg: f64,
    pub interference_dia: f64,
    pub margin: f64,
    pub design_capacity: f64,
    pub pin_peak_pressure: f64,
}

const HISTORY_LIMIT: usize = 6;

/// Runs asked for while one is going wait here (the same task is not queued twice).
const QUEUE_LIMIT: usize = 4;

pub struct EccentricState {
    pub ui: EccentricUi,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: crate::widgets::number_edit::EditBuffer,
    pub results_scroll: u16,
    /// `d` adds the interface pressure profile.
    pub show_profile: bool,
    pub analysis: Option<(Inputs, Analysis)>,
    pub max_offset: Option<(Inputs, OffsetLimit)>,
    pub max_load: Option<(Inputs, OffsetLimit)>,
    pub sweep: Option<(Inputs, Vec<SweepPoint>)>,
    /// The last analyses, newest last (re-running identical inputs replaces the newest).
    pub history: Vec<HistoryEntry>,
    pub job: Option<Job>,
    pub queue: VecDeque<Task>,
    next_job: u64,
    pub error: Option<String>,
}

impl Default for EccentricState {
    fn default() -> Self {
        Self {
            ui: EccentricUi::default(),
            selected: 1,
            editing: false,
            edit_buffer: Default::default(),
            results_scroll: 0,
            show_profile: false,
            analysis: None,
            max_offset: None,
            max_load: None,
            sweep: None,
            history: Vec::new(),
            job: None,
            queue: VecDeque::new(),
            next_job: 1,
            error: None,
        }
    }
}

impl EccentricState {
    pub fn selected_row(&self) -> Option<FieldRow> {
        model::field_rows(self.ui.advanced_open).get(self.selected).copied()
    }

    pub fn clamp_selection(&mut self) {
        let rows = model::field_rows(self.ui.advanced_open);
        self.selected = self.selected.min(rows.len() - 1);
        if matches!(rows[self.selected], FieldRow::Header(_)) {
            self.move_selection(1);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let rows = model::field_rows(self.ui.advanced_open);
        let len = rows.len() as i32;
        let mut next = self.selected as i32;
        for _ in 0..rows.len() {
            next = (next + delta).rem_euclid(len);
            if !matches!(rows[next as usize], FieldRow::Header(_)) {
                break;
            }
        }
        self.selected = next as usize;
    }

    /// Start `task` for the live Bushing Workbench model; while another run is going it waits in the queue.
    pub fn start(&mut self, bushing: &BushingModel, task: Task) -> Vec<Effect> {
        if self.job.is_some() {
            if !self.queue.contains(&task) && self.queue.len() < QUEUE_LIMIT {
                self.queue.push_back(task);
            }
            return Vec::new();
        }
        match model::build_input(bushing, &self.ui) {
            Err(why) => {
                self.error = Some(why);
                Vec::new()
            }
            Ok(input) => {
                self.error = None;
                let id = self.next_job;
                self.next_job += 1;
                let control = model::new_control(task);
                self.job = Some(Job { id, task, sig: input, started: std::time::Instant::now(), control: control.clone() });
                vec![Effect::RunEccentric { id, task, input: Box::new(input), control }]
            }
        }
    }

    fn remember(&mut self, inp: &Inputs, a: &Analysis) {
        let entry = HistoryEntry { offset: inp.offset, load_lbf: inp.load_lbf, load_angle_deg: inp.load_angle_deg, interference_dia: inp.interference_dia, margin: a.margin, design_capacity: a.design_capacity, pin_peak_pressure: a.pin_peak_pressure };
        if self.history.last().is_some_and(|h| (h.offset, h.load_lbf, h.load_angle_deg, h.interference_dia) == (entry.offset, entry.load_lbf, entry.load_angle_deg, entry.interference_dia)) {
            self.history.pop();
        }
        self.history.push(entry);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
    }

    /// `c`: stop the running job (it still reports what it has) and drop the queued ones.
    pub fn cancel(&mut self) {
        if let Some(job) = &self.job {
            if let Some(flag) = &job.control.interrupt.cancel {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.queue.clear();
    }

    /// A worker finished job `id`; returns the file to write for a field export and the next queued run.
    pub fn finish(&mut self, id: u64, result: Result<Output, String>, bushing: &BushingModel) -> Vec<Effect> {
        let Some(job) = self.job.take_if(|j| j.id == id) else { return Vec::new() };
        let mut effects = Vec::new();
        match result {
            Ok(Output::Analysis(a)) => {
                self.remember(&job.sig, &a);
                self.analysis = Some((job.sig, *a));
            }
            Ok(Output::MaxOffset(l)) => self.max_offset = Some((job.sig, l)),
            Ok(Output::MaxLoad(l)) => self.max_load = Some((job.sig, l)),
            Ok(Output::Sweep(points)) => self.sweep = Some((job.sig, points)),
            Ok(Output::Fields(a, vtu)) => {
                self.remember(&job.sig, &a);
                self.analysis = Some((job.sig, *a));
                effects.extend(crate::paths::app_data_dir().map(|dir| Effect::WriteTextFile { path: dir.join("reports").join("eccentric-bushing-fields.vtu").to_string_lossy().into_owned(), contents: vtu }));
            }
            Err(why) => self.error = Some(format!("{:?} failed: {why}", job.task)),
        }
        if self.error.as_deref().is_some_and(|e| !e.contains("failed")) {
            self.error = None;
        }
        if let Some(next) = self.queue.pop_front() {
            effects.extend(self.start(bushing, next));
        }
        effects
    }

    pub fn tick(&mut self) {}

    pub fn report_text(&self, bushing: &BushingModel) -> Option<String> {
        let input = model::build_input(bushing, &self.ui).ok()?;
        let fresh = |sig: &Inputs| *sig == input;
        let a = self.analysis.as_ref().filter(|(s, _)| fresh(s)).map(|(_, a)| a);
        let mo = self.max_offset.as_ref().filter(|(s, _)| fresh(s)).map(|(_, l)| l);
        let ml = self.max_load.as_ref().filter(|(s, _)| fresh(s)).map(|(_, l)| l);
        (a.is_some() || mo.is_some() || ml.is_some()).then(|| model::report_text(&input, a, mo, ml))
    }

    fn activate_selected(&mut self, bushing: &BushingModel) -> Vec<Effect> {
        match self.selected_row() {
            Some(FieldRow::TogglePlane) => {
                self.ui.plane_strain = !self.ui.plane_strain;
                Vec::new()
            }
            Some(FieldRow::ToggleHousing) => {
                self.ui.edge_limited = !self.ui.edge_limited;
                Vec::new()
            }
            Some(FieldRow::ToggleDirectOnset) => {
                self.ui.direct_onset = !self.ui.direct_onset;
                Vec::new()
            }
            Some(FieldRow::TogglePinCredit) => {
                self.ui.credit_pin_load = !self.ui.credit_pin_load;
                Vec::new()
            }
            Some(FieldRow::AdvancedSection) => {
                self.ui.advanced_open = !self.ui.advanced_open;
                Vec::new()
            }
            Some(FieldRow::Run(task)) => self.start(bushing, task),
            _ => Vec::new(),
        }
    }
}

fn csv_effect(state: &EccentricState, bushing: &BushingModel) -> Vec<Effect> {
    let (Ok(input), Some(dir)) = (model::build_input(bushing, &state.ui), crate::paths::app_data_dir()) else { return Vec::new() };
    let a = state.analysis.as_ref().filter(|(s, _)| *s == input).map(|(_, a)| a);
    let sweep = state.sweep.as_ref().filter(|(s, _)| *s == input).map(|(_, p)| p.as_slice());
    if a.is_none() && sweep.is_none() {
        return Vec::new();
    }
    vec![Effect::WriteTextFile { path: dir.join("reports").join("eccentric-bushing.csv").to_string_lossy().into_owned(), contents: model::csv_text(&input, a, sweep) }]
}

fn export_effect(state: &EccentricState, bushing: &BushingModel) -> Vec<Effect> {
    let (Some(contents), Some(dir)) = (state.report_text(bushing), crate::paths::app_data_dir()) else { return Vec::new() };
    vec![Effect::WriteTextFileAndOpen { path: dir.join("reports").join("eccentric-bushing-report.txt").to_string_lossy().into_owned(), contents }]
}

/// Toolbox-local key routing, same `(consumed, effects)` contract as the other toolboxes.
pub fn handle_key(state: &mut EccentricState, bushing: &BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    if state.editing {
        return match key.code {
            KeyCode::Enter => {
                commit_edit(state);
                (true, Vec::new())
            }
            KeyCode::Esc => {
                state.editing = false;
                state.edit_buffer.clear();
                (true, Vec::new())
            }
            _ if crate::widgets::number_edit::handle_buffer_key(&mut state.edit_buffer, &key) => (true, Vec::new()),
            _ => (false, Vec::new()),
        };
    }
    if let Some(c) = crate::widgets::number_edit::number_char(&key) {
        if matches!(state.selected_row(), Some(FieldRow::Number(_))) {
            state.editing = true;
            state.edit_buffer.set(c.to_string());
            return (true, Vec::new());
        }
    }
    match key.code {
        KeyCode::Up => {
            state.move_selection(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            state.move_selection(1);
            (true, Vec::new())
        }
        KeyCode::Char(' ') => (true, state.activate_selected(bushing)),
        KeyCode::Enter => match state.selected_row() {
            Some(FieldRow::Number(t)) => {
                state.editing = true;
                state.edit_buffer.set(model::format_for_edit(state.ui.number_value(t)));
                (true, Vec::new())
            }
            Some(FieldRow::Header(_)) | None => (false, Vec::new()),
            Some(_) => (true, state.activate_selected(bushing)),
        },
        KeyCode::Char('r' | 'R') => (true, state.start(bushing, Task::Analyze)),
        KeyCode::Char('m' | 'M') => (true, state.start(bushing, Task::MaxOffset)),
        KeyCode::Char('l' | 'L') => (true, state.start(bushing, Task::MaxLoad)),
        KeyCode::Char('c' | 'C') => {
            state.cancel();
            (true, Vec::new())
        }
        KeyCode::Char('s' | 'S') => (true, state.start(bushing, Task::Sweep)),
        KeyCode::Char('v' | 'V') => (true, state.start(bushing, Task::Fields)),
        KeyCode::Char('x' | 'X') => (true, csv_effect(state, bushing)),
        KeyCode::Char('d' | 'D') => {
            state.show_profile = !state.show_profile;
            (true, Vec::new())
        }
        KeyCode::Char('e' | 'E') => (true, export_effect(state, bushing)),
        KeyCode::PageUp => {
            state.results_scroll = state.results_scroll.saturating_sub(crate::widgets::scroll_paragraph::SCROLL_STEP);
            (true, Vec::new())
        }
        KeyCode::PageDown => {
            state.results_scroll = state.results_scroll.saturating_add(crate::widgets::scroll_paragraph::SCROLL_STEP);
            (true, Vec::new())
        }
        _ => (false, Vec::new()),
    }
}

fn commit_edit(state: &mut EccentricState) {
    if let Some(FieldRow::Number(t)) = state.selected_row() {
        if let Ok(raw) = state.edit_buffer.trim().parse::<f64>() {
            state.ui.commit_number(t, raw);
        }
    }
    state.editing = false;
    state.edit_buffer.clear();
}

#[cfg(test)]
mod tests;
