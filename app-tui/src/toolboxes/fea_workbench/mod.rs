//! FEA Workbench toolbox: define a finite-element problem (a sketched outline with holes, extruded
//! to 3D if wanted, or an imported Gmsh / Abaqus mesh; material, supports, loads, mesh settings),
//! see its mesh, solve it on the general kernel (`fea-core`, through `fea-problem`) and look at the
//! result as a colour contour. Problems save to and load from JSON; results export as a text
//! report and a `.vtu` file for ParaView.
//!
//! Single workspace pane (`PANE_MAIN`): field list on the left, mesh / contour canvas and a
//! results readout on the right. The mesh preview re-builds a moment after any input stops
//! changing and a small model re-solves by itself (`a` toggles that); every worker job carries an
//! id and a result for older inputs is dropped, like Lug Analysis.

pub mod canvas;
pub mod model;
pub mod view;

use std::cell::RefCell;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fea_problem::dynamics::{DynKind, DynSolved};
use fea_problem::raster::Raster;
use fea_problem::templates::templates;
use fea_problem::{Field, Geometry, Problem, Solved, Support};

use crate::app::Effect;
use crate::toolboxes::material_lookup::{self, MaterialLookupState};
use crate::widgets::number_edit::EditBuffer;
use model::{EditKind, FieldRow};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

/// Inputs must sit unchanged this long before the preview / auto-solve starts.
const DEBOUNCE_MS: u128 = 300;
/// Largest mesh the automatic solve takes on without being asked.
pub const AUTO_SOLVE_ELEMENTS: usize = 6000;

/// What a file read was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePurpose {
    Problem,
    Mesh,
}

/// A prompt for a file path (open a problem).
pub struct Prompt {
    pub label: &'static str,
    pub buffer: EditBuffer,
}

pub struct Preview {
    /// The inputs the mesh was built for.
    pub problem: Problem,
    pub mesh: fea_core::Mesh,
    pub names: Vec<String>,
}

struct Job {
    id: u64,
    sig: Problem,
    started: Instant,
}

/// What a cached raster was drawn from.
#[derive(Clone, PartialEq)]
pub(crate) struct RasterKey {
    pub w: usize,
    pub h: usize,
    pub field: Option<Field>,
    pub deform: bool,
    /// Mode drawn (natural-frequency / buckling run) instead of a result field.
    pub mode: Option<usize>,
    pub source: u64,
}

pub struct FeaWorkbenchState {
    pub problem: Problem,
    pub template: usize,
    pub selected: usize,
    pub edit: Option<EditKind>,
    pub edit_buffer: EditBuffer,
    pub material_browser: Option<MaterialLookupState>,
    pub prompt: Option<Prompt>,
    /// The sketch kept while the geometry source is an imported file (and back).
    saved_geometry: Option<Geometry>,
    /// The text of the imported mesh file: `(path, text)`.
    pub import: Option<(String, String)>,
    pub preview: Option<Preview>,
    pub preview_error: Option<String>,
    preview_job: Option<Job>,
    /// Inputs last seen and when they last changed (the debounce).
    seen: Option<(Problem, Instant)>,
    pub solved: Option<Box<Solved>>,
    pub solve_error: Option<String>,
    solve_job: Option<Job>,
    /// Inputs the last failed solve was for: not retried until an edit.
    failed_sig: Option<Problem>,
    /// Natural frequencies / buckling load factors of the inputs they were computed for (keys `n`, `b`).
    pub dynamic: Option<Box<DynSolved>>,
    pub dyn_error: Option<String>,
    dyn_job: Option<Job>,
    animation_job: Option<Job>,
    animation_progress: Option<crate::gpu_viewer::progress::GenerationProgress>,
    viewer_launch: Option<(u64, Instant)>,
    /// Whether the canvas and readout show the dynamic result (while it still belongs to the inputs).
    pub show_dynamic: bool,
    /// Mode drawn on the canvas.
    pub mode: usize,
    pub auto_solve: bool,
    pub field: Field,
    pub show_mesh: bool,
    pub deform: bool,
    pub details: bool,
    pub results_scroll: u16,
    next_job: u64,
    /// Bumped whenever the drawn mesh or result changes (the raster cache key).
    pub(crate) source: u64,
    pub(crate) raster: RefCell<Option<(RasterKey, Raster)>>,
}

impl Default for FeaWorkbenchState {
    fn default() -> Self {
        let (_, problem) = templates().into_iter().next().expect("a template");
        Self::with_problem(problem, 0)
    }
}

impl FeaWorkbenchState {
    fn with_problem(problem: Problem, template: usize) -> Self {
        let mut s = Self {
            problem,
            template,
            selected: 0,
            edit: None,
            edit_buffer: EditBuffer::default(),
            material_browser: None,
            prompt: None,
            saved_geometry: None,
            import: None,
            preview: None,
            preview_error: None,
            preview_job: None,
            seen: None,
            solved: None,
            solve_error: None,
            solve_job: None,
            failed_sig: None,
            dynamic: None,
            dyn_error: None,
            dyn_job: None,
            animation_job: None,
            animation_progress: None,
            viewer_launch: None,
            show_dynamic: false,
            mode: 0,
            auto_solve: true,
            field: Field::VonMises,
            show_mesh: true,
            deform: false,
            details: false,
            results_scroll: 0,
            next_job: 1,
            source: 0,
            raster: RefCell::new(None),
        };
        s.clamp_selection();
        s
    }

    /// Replace the whole problem (template, loaded file); results and preview belong to the old one.
    pub fn set_problem(&mut self, problem: Problem) {
        let keep = (self.template, self.auto_solve, self.field, self.show_mesh, self.deform);
        *self = Self::with_problem(problem, keep.0);
        (self.auto_solve, self.field, self.show_mesh, self.deform) = (keep.1, keep.2, keep.3, keep.4);
    }

    pub fn rows(&self) -> Vec<FieldRow> {
        model::field_rows(&self.problem)
    }

    /// Edge / set / surface names a support or load can use.
    pub fn names(&self) -> Vec<String> {
        match &self.problem.geometry {
            Geometry::Sketch { .. } => self.problem.edge_names(),
            Geometry::Imported { .. } => self.preview.as_ref().map(|p| p.names.clone()).unwrap_or_default(),
        }
    }

    pub fn selected_row(&self) -> Option<FieldRow> {
        self.rows().get(self.selected).copied()
    }

    pub fn clamp_selection(&mut self) {
        let rows = self.rows();
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        if matches!(rows.get(self.selected), Some(FieldRow::Header(_))) {
            self.move_selection(1);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let rows = self.rows();
        let len = rows.len() as i32;
        if len == 0 {
            return;
        }
        let mut next = self.selected as i32;
        for _ in 0..rows.len() {
            next = (next + delta).rem_euclid(len);
            if !matches!(rows[next as usize], FieldRow::Header(_)) {
                break;
            }
        }
        self.selected = next as usize;
    }

    fn import_text(&self) -> Option<String> {
        let Geometry::Imported { path } = &self.problem.geometry else { return None };
        self.import.as_ref().filter(|(p, _)| p == path).map(|(_, t)| t.clone())
    }

    // ------------------------------------------------------------------ actions

    fn activate(&mut self, row: FieldRow, forward: bool) -> Vec<Effect> {
        use FieldRow::*;
        let names = self.names();
        let first = names.first().cloned().unwrap_or_default();
        let p = &mut self.problem;
        match row {
            Template => {
                let all = templates();
                let n = all.len();
                let i = if forward { (self.template + 1) % n } else { (self.template + n - 1) % n };
                let (_, problem) = all.into_iter().nth(i).expect("template index");
                let (auto, field, mesh, deform) = (self.auto_solve, self.field, self.show_mesh, self.deform);
                *self = Self::with_problem(problem, i);
                (self.auto_solve, self.field, self.show_mesh, self.deform) = (auto, field, mesh, deform);
                return Vec::new();
            }
            Analysis => {
                let all = fea_problem::Analysis::ALL;
                let i = all.iter().position(|a| *a == p.analysis).unwrap_or(0);
                p.analysis = all[if forward { (i + 1) % all.len() } else { (i + all.len() - 1) % all.len() }];
                model::fix_element_for(p);
            }
            Material => {
                let current = crate::toolboxes::material_lookup::model::catalog().iter().position(|m| m.name == p.material.name).unwrap_or(0);
                let mut browser = MaterialLookupState { picking: true, ..MaterialLookupState::default() };
                if let Some(pos) = browser.hits.iter().position(|&i| i == current) {
                    browser.cursor = pos;
                }
                self.material_browser = Some(browser);
            }
            Source => match (&p.geometry, self.saved_geometry.take()) {
                (Geometry::Sketch { .. }, saved) => {
                    let sketch = std::mem::replace(&mut p.geometry, Geometry::Imported { path: self.import.as_ref().map(|(p, _)| p.clone()).unwrap_or_default() });
                    self.saved_geometry = Some(sketch);
                    drop(saved);
                }
                (Geometry::Imported { .. }, Some(sketch)) => p.geometry = sketch,
                (Geometry::Imported { .. }, None) => {
                    if let Some((_, pr)) = templates().into_iter().next() {
                        p.geometry = pr.geometry;
                    }
                }
            },
            OuterKind => {
                if let Geometry::Sketch { outer, .. } = &mut p.geometry {
                    *outer = outer.next_kind(true);
                }
            }
            PolyAdd => model::add_polygon_point(p),
            PolyRemove => model::remove_polygon_point(p),
            HoleKind(h) => {
                if let Geometry::Sketch { holes, .. } = &mut p.geometry {
                    if let Some(s) = holes.get_mut(h) {
                        *s = s.next_kind(false);
                    }
                }
            }
            HoleRemove(h) => {
                if let Geometry::Sketch { holes, .. } = &mut p.geometry {
                    if h < holes.len() {
                        holes.remove(h);
                    }
                }
                model::forget_hole(p, h);
            }
            HoleAdd => {
                let hole = model::new_hole(p);
                if let Geometry::Sketch { holes, .. } = &mut p.geometry {
                    holes.push(hole);
                }
            }
            Element => {
                let all: Vec<_> = fea_problem::ElementChoice::ALL.iter().copied().filter(|e| p.analysis != fea_problem::Analysis::Solid || e.is_quad()).collect();
                let i = all.iter().position(|e| *e == p.mesh.element).unwrap_or(0);
                p.mesh.element = all[if forward { (i + 1) % all.len() } else { (i + all.len() - 1) % all.len() }];
            }
            SupportKind(s) => {
                if let Some(sup) = p.supports.get_mut(s) {
                    *sup = sup.toggled(&first);
                }
            }
            SupportEdge(s) => {
                if let Some(sup) = p.supports.get_mut(s) {
                    if let Some(next) = model::cycle_name(sup.edge().unwrap_or(""), &names, forward) {
                        sup.set_edge(&next);
                    }
                }
            }
            SupportComp(s, c) => {
                if let Some(sup) = p.supports.get_mut(s) {
                    let held = sup.comps()[c].is_some();
                    sup.set_comp(c, if held { None } else { Some(0.0) });
                }
            }
            SupportRemove(s) => {
                if s < p.supports.len() {
                    p.supports.remove(s);
                }
            }
            SupportAdd => p.supports.push(Support::fixed(&first)),
            LoadKind(l) => {
                if let Some(load) = p.loads.get_mut(l) {
                    *load = load.next_kind(&first);
                }
            }
            LoadEdge(l) => {
                if let Some(load) = p.loads.get_mut(l) {
                    if let Some(next) = model::cycle_name(load.edge().unwrap_or(""), &names, forward) {
                        load.set_edge(&next);
                    }
                }
            }
            LoadRemove(l) => {
                if l < p.loads.len() {
                    p.loads.remove(l);
                }
            }
            LoadAdd => p.loads.push(model::default_load(&first)),
            BushingAdd => {
                if let Some(b) = model::new_bushing(p) {
                    p.bushings.push(b);
                }
            }
            BushingRemove(b) => {
                if b < p.bushings.len() {
                    p.bushings.remove(b);
                }
            }
            _ => {}
        }
        self.clamp_selection();
        Vec::new()
    }

    fn start_edit(&mut self, row: FieldRow, first: Option<char>) {
        let kind = model::edit_kind(row);
        let text = match (kind, first) {
            (_, Some(c)) => c.to_string(),
            (EditKind::Text, None) => match &self.problem.geometry {
                Geometry::Imported { path } => path.clone(),
                _ => String::new(),
            },
            (EditKind::Component, None) => model::number_value(&self.problem, row).map_or("free".to_string(), model::format_number),
            (_, None) => model::number_value(&self.problem, row).map_or(String::new(), model::format_number),
        };
        self.edit = Some(kind);
        self.edit_buffer.set(text);
    }

    fn commit_edit(&mut self) -> Vec<Effect> {
        let Some(kind) = self.edit.take() else { return Vec::new() };
        let text = self.edit_buffer.take();
        let Some(row) = self.selected_row() else { return Vec::new() };
        match kind {
            EditKind::Number => {
                if let Ok(v) = text.trim().parse::<f64>() {
                    model::set_number(&mut self.problem, row, v);
                }
            }
            EditKind::Component => match text.trim().parse::<f64>() {
                Ok(v) => model::set_number(&mut self.problem, row, v),
                Err(_) => model::release_component(&mut self.problem, row),
            },
            EditKind::Text => {
                let path = text.trim().trim_matches('"').to_string();
                if !path.is_empty() {
                    self.problem.geometry = Geometry::Imported { path: path.clone() };
                    return vec![Effect::ReadTextFile { purpose: FilePurpose::Mesh, path }];
                }
            }
            EditKind::Action => {}
        }
        Vec::new()
    }

    /// A file read finished.
    pub fn file_read(&mut self, purpose: FilePurpose, path: String, result: Result<String, String>) -> Option<(String, crate::theme::StatusTone)> {
        use crate::theme::StatusTone::*;
        match (purpose, result) {
            (FilePurpose::Problem, Ok(text)) => match Problem::from_json(&text) {
                Ok(problem) => {
                    let name = problem.name.clone();
                    self.set_problem(problem);
                    Some((format!("Opened problem '{name}'"), Success))
                }
                Err(e) => Some((format!("{path}: {e}"), Danger)),
            },
            (FilePurpose::Mesh, Ok(text)) => {
                self.import = Some((path.clone(), text));
                self.seen = None;
                self.failed_sig = None;
                Some((format!("Read mesh file {path}"), Info))
            }
            (_, Err(e)) => Some((format!("Could not read {path}: {e}"), Danger)),
        }
    }

    // ------------------------------------------------------------------ jobs

    /// Called every `Tick` while this toolbox is on screen: starts the mesh preview and, for a small
    /// model, the solve once the inputs have settled.
    pub fn tick(&mut self) -> Vec<Effect> {
        let sig = self.problem.clone();
        let changed = self.seen.as_ref().is_none_or(|(s, _)| *s != sig);
        if changed {
            self.seen = Some((sig.clone(), Instant::now()));
            if self.preview_job.as_ref().is_some_and(|j| j.sig != preview_key(&sig)) {
                self.preview_job = None;
            }
            if self.solve_job.as_ref().is_some_and(|j| j.sig != sig) {
                self.solve_job = None;
            }
        }
        let settled = self.seen.as_ref().is_some_and(|(_, at)| at.elapsed().as_millis() >= DEBOUNCE_MS);
        let first = self.preview.is_none() && self.preview_error.is_none();
        let mut effects = Vec::new();

        if let Err(e) = sig.validate_geometry() {
            self.preview_error = Some(e);
            self.preview_job = None;
            return effects;
        }
        let import_text = self.import_text();
        if matches!(sig.geometry, Geometry::Imported { .. }) && import_text.is_none() {
            self.preview_error = Some("read a mesh file: Enter on the Mesh File row".into());
            return effects;
        }
        let preview_current = self.preview.as_ref().is_some_and(|p| p.problem == preview_key(&sig));
        if preview_current {
            self.preview_error = None;
        }
        if !preview_current && self.preview_job.is_none() && (settled || first) {
            let id = self.take_job_id();
            self.preview_job = Some(Job { id, sig: preview_key(&sig), started: Instant::now() });
            effects.push(Effect::RunFeaPreview { id, problem: Box::new(preview_key(&sig)), import_text: import_text.clone() });
        }

        // Automatic solve for a small, current mesh.
        let solved_current = self.solved.as_ref().is_some_and(|s| s.problem == sig);
        let small = self.preview.as_ref().is_some_and(|p| p.problem == preview_key(&sig) && p.mesh.n_elems() <= AUTO_SOLVE_ELEMENTS);
        // (A bushed problem is a contact solve of seconds: on request only.)
        if self.auto_solve && sig.bushings.is_empty() && settled && preview_current && small && !solved_current && self.solve_job.is_none() && self.failed_sig.as_ref() != Some(&sig) {
            effects.extend(self.start_solve());
        }
        effects
    }

    fn take_job_id(&mut self) -> u64 {
        let id = self.next_job;
        self.next_job += 1;
        id
    }

    /// Start a solve of the current inputs (the `r` key).
    pub fn start_solve(&mut self) -> Vec<Effect> {
        if self.solve_job.is_some() {
            return Vec::new();
        }
        let sig = self.problem.clone();
        if let Err(e) = sig.validate() {
            self.solve_error = Some(e);
            return Vec::new();
        }
        let import_text = self.import_text();
        if matches!(sig.geometry, Geometry::Imported { .. }) && import_text.is_none() {
            self.solve_error = Some("read a mesh file first".into());
            return Vec::new();
        }
        let id = self.take_job_id();
        self.solve_job = Some(Job { id, sig: sig.clone(), started: Instant::now() });
        self.solve_error = None;
        vec![Effect::RunFeaSolve { id, problem: Box::new(sig), import_text }]
    }

    pub fn start_animation(&mut self) -> Vec<Effect> {
        if self.animation_job.is_some() || self.viewer_launch.is_some() {
            return Vec::new();
        }
        let id = self.take_job_id();
        self.solve_error = None;
        self.animation_progress = Some(crate::gpu_viewer::progress::GenerationProgress::stage("Preparing animation"));
        let sig = self.problem.clone();
        self.animation_job = Some(Job {
            id,
            sig: sig.clone(),
            started: Instant::now(),
        });
        vec![Effect::RunFeaAnimation {
            id,
            problem: Box::new(sig),
            import_text: self.import_text(),
        }]
    }
    pub fn finish_animation(
        &mut self,
        id: u64,
        result: Result<crate::gpu_viewer::scene::ViewerProject, String>,
    ) -> Vec<Effect> {
        let Some(job) = self.animation_job.take_if(|j| j.id == id) else {
            return Vec::new();
        };
        self.animation_progress = None;
        if job.sig != self.problem {
            return Vec::new();
        }
        match result {
            Ok(scene) => { self.solve_error = None; self.viewer_launch = Some((id, Instant::now())); vec![Effect::OpenGpuScene {
                id,
                scene: Box::new(scene),
            }] },
            Err(e) => {
                self.solve_error = Some(e);
                Vec::new()
            }
        }
    }
    pub fn open_gpu(&mut self) -> Vec<Effect> {
        if self.animation_job.is_some() || self.viewer_launch.is_some() {
            return Vec::new();
        }
        let mut sources = Vec::new();
        let selected;
        if let Some(d) = self.dynamic_shown() {
            selected = self.mode;
            if d.model().mesh.nodes.len().checked_mul(d.n_modes()).is_none_or(|n| n > 4_000_000) {
                self.solve_error = Some("computed modes exceed the native session budget; use a coarser mesh".into());
                return Vec::new();
            }
            let mesh = std::sync::Arc::new(d.model().mesh.clone());
            for mode in 0..d.n_modes() {
                let period = match d {
                    DynSolved::Modal(m) => Some(1.0 / m.modes[mode].frequency_hz as f32),
                    _ => None,
                };
                sources.push(crate::gpu_viewer::scene::SceneSource {
                    mesh: mesh.clone(), displacement: d.shape(mode).to_vec(),
                    values: d.magnitude(mode), period,
                    label: format!("{} | {}", d.title(mode), if period.is_some() { "harmonic mode; seconds; mass-normalized magnitude" } else { "static buckling shape" }),
                });
            }
        } else if let Some(s) = self.solved.as_deref().filter(|s| s.problem == self.problem) {
            selected = 0;
            sources.push(crate::gpu_viewer::scene::SceneSource {
                mesh: std::sync::Arc::new(s.model.mesh.clone()), displacement: s.u.clone(), values: s.node_values(self.field),
                period: None, label: format!("{} | static {:?}", s.problem.name, self.field),
            });
        } else {
            self.solve_error = Some("solve the current inputs before opening the GPU viewport".into());
            return Vec::new();
        }
        let source = crate::gpu_viewer::scene::ProjectSource { sources, selected, problem: self.problem.clone() };
        let id = self.take_job_id();
        self.solve_error = None;
        self.animation_progress = Some(crate::gpu_viewer::progress::GenerationProgress::stage("Preparing GPU scene"));
        self.animation_job = Some(Job {
            id,
            sig: self.problem.clone(),
            started: Instant::now(),
        });
        vec![Effect::BuildFeaGpuScene {
            id,
            source: Box::new(source),
        }]
    }

    pub fn update_animation_progress(&mut self, id: u64, progress: crate::gpu_viewer::progress::GenerationProgress) {
        if self.animation_job.as_ref().is_some_and(|j| j.id == id && j.sig == self.problem) {
            self.animation_progress = Some(progress);
        }
    }
    pub fn animation_status(&self) -> Option<String> {
        if let Some((_, started)) = &self.viewer_launch { return Some(format!("Opening native GPU viewport ({:.1} s)", started.elapsed().as_secs_f64())); }
        let job = self.animation_job.as_ref()?;
        if job.sig != self.problem {
            return Some(format!("Inputs changed; previous animation finishing ({:.1} s)", job.started.elapsed().as_secs_f64()));
        }
        Some(format!("{} ({:.1} s)", self.animation_progress.as_ref().map(|p| p.text()).unwrap_or_else(|| "Preparing animation".into()), job.started.elapsed().as_secs_f64()))
    }
    pub fn viewer_ready(&mut self, id: u64) {
        if self.viewer_launch.is_some_and(|(live, _)| live == id) { self.viewer_launch = None; }
    }

    /// Start natural frequencies (`Modal`) or buckling load factors (`Buckling`) of the current inputs (keys `n`, `b`).
    pub fn start_dynamic(&mut self, kind: DynKind) -> Vec<Effect> {
        if self.dyn_job.is_some() {
            return Vec::new();
        }
        let sig = self.problem.clone();
        if let Err(e) = sig.validate() {
            self.dyn_error = Some(e);
            return Vec::new();
        }
        let import_text = self.import_text();
        if matches!(sig.geometry, Geometry::Imported { .. }) && import_text.is_none() {
            self.dyn_error = Some("read a mesh file first".into());
            return Vec::new();
        }
        let id = self.take_job_id();
        self.dyn_job = Some(Job { id, sig: sig.clone(), started: Instant::now() });
        self.dyn_error = None;
        vec![Effect::RunFeaDynamic { id, problem: Box::new(sig), import_text, kind, n_modes: if kind == DynKind::Modal { 6 } else { 3 } }]
    }

    pub fn finish_dynamic(&mut self, id: u64, result: Result<DynSolved, String>) {
        if self.dyn_job.take_if(|j| j.id == id).is_none() {
            return;
        }
        match result {
            Ok(d) => {
                self.dynamic = Some(Box::new(d));
                self.dyn_error = None;
                self.show_dynamic = true;
                self.mode = 0;
                self.source += 1;
            }
            Err(e) => self.dyn_error = Some(e),
        }
    }

    /// Installs a dynamic result as if a worker had returned it (render tests).
    #[doc(hidden)]
    pub fn finish_dynamic_for_test(&mut self, d: DynSolved) {
        self.dynamic = Some(Box::new(d));
        self.show_dynamic = true;
        self.mode = 0;
        self.source += 1;
    }

    pub fn analysing(&self) -> Option<f64> {
        self.dyn_job.as_ref().map(|j| j.started.elapsed().as_secs_f64())
    }

    /// The dynamic result, when it belongs to the current inputs and is the one on display.
    pub fn dynamic_shown(&self) -> Option<&DynSolved> {
        self.dynamic.as_deref().filter(|d| self.show_dynamic && d.problem() == &self.problem)
    }

    pub fn finish_preview(&mut self, id: u64, result: Result<fea_core::Mesh, String>) {
        let Some(job) = self.preview_job.take_if(|j| j.id == id) else { return };
        match result {
            Ok(mesh) => {
                let names = fea_problem::build::mesh_names(&mesh);
                self.preview = Some(Preview { problem: job.sig, mesh, names });
                self.preview_error = None;
                self.source += 1;
            }
            Err(e) => self.preview_error = Some(e),
        }
    }

    pub fn finish_solve(&mut self, id: u64, result: Result<Solved, String>) {
        let Some(job) = self.solve_job.take_if(|j| j.id == id) else { return };
        match result {
            Ok(s) => {
                self.solved = Some(Box::new(s));
                self.solve_error = None;
                self.failed_sig = None;
                self.source += 1;
            }
            Err(e) => {
                self.failed_sig = Some(job.sig);
                self.solve_error = Some(e);
            }
        }
    }

    /// Installs a result as if a worker had returned it (render tests).
    #[doc(hidden)]
    pub fn finish_solve_for_test(&mut self, solved: Solved) {
        self.solved = Some(Box::new(solved));
        self.source += 1;
    }

    pub fn solving(&self) -> Option<f64> {
        self.solve_job.as_ref().map(|j| j.started.elapsed().as_secs_f64())
    }

    pub fn meshing(&self) -> bool {
        self.preview_job.is_some()
    }

    /// True while the displayed result belongs to different inputs than the editor holds.
    pub fn stale(&self) -> bool {
        self.solved.as_ref().is_some_and(|s| s.problem != self.problem)
    }

    // ------------------------------------------------------------------ files

    fn file_dir() -> Option<std::path::PathBuf> {
        crate::paths::app_data_dir().map(|d| d.join("fea-workbench"))
    }

    fn file_stem(&self) -> String {
        let s: String = self.problem.name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        s.trim_matches('-').to_string()
    }

    fn write_effect(&self, ext: &str, contents: String, open: bool) -> Vec<Effect> {
        let Some(dir) = Self::file_dir() else { return Vec::new() };
        let path = dir.join(format!("{}.{ext}", self.file_stem())).to_string_lossy().into_owned();
        vec![if open { Effect::WriteTextFileAndOpen { path, contents } } else { Effect::WriteTextFile { path, contents } }]
    }

    pub fn report_text(&self) -> Option<String> {
        self.solved.as_ref().map(|s| fea_problem::report::report(s))
    }
}

/// The part of the inputs the mesh depends on (supports, loads and material values do not).
pub(crate) fn preview_key(p: &Problem) -> Problem {
    Problem { supports: Vec::new(), loads: Vec::new(), delta_t: 0.0, name: String::new(), ..p.clone() }
}

fn is_edit_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')
}

/// Toolbox-local key routing - same `(consumed, effects)` contract as the other toolboxes.
pub fn handle_key(state: &mut FeaWorkbenchState, key: KeyEvent) -> (bool, Vec<Effect>) {
    if let Some(browser) = state.material_browser.as_mut() {
        match key.code {
            KeyCode::Enter => {
                if let Some(i) = browser.current() {
                    let m = material_lookup::model::catalog()[i];
                    let mat = &mut state.problem.material;
                    mat.name = m.name.to_string();
                    mat.e = m.e_ksi * 1000.0;
                    mat.nu = m.nu;
                    mat.alpha = m.alpha_u_f * 1e-6;
                    mat.yield_stress = (m.sy_ksi > 0.0).then_some(m.sy_ksi * 1000.0);
                }
                state.material_browser = None;
                return (true, Vec::new());
            }
            KeyCode::Esc if browser.query.text.is_empty() => {
                state.material_browser = None;
                return (true, Vec::new());
            }
            _ => {
                let (consumed, effects) = material_lookup::handle_key(browser, key);
                return (consumed || !matches!(key.code, KeyCode::Tab | KeyCode::BackTab), effects);
            }
        }
    }

    if let Some(prompt) = state.prompt.as_mut() {
        match key.code {
            KeyCode::Enter => {
                let path = prompt.buffer.trim().trim_matches('"').to_string();
                state.prompt = None;
                if path.is_empty() {
                    return (true, Vec::new());
                }
                return (true, vec![Effect::ReadTextFile { purpose: FilePurpose::Problem, path }]);
            }
            KeyCode::Esc => {
                state.prompt = None;
                return (true, Vec::new());
            }
            _ => {
                prompt.buffer.handle_key(&key, |c| !c.is_control());
                return (true, Vec::new());
            }
        }
    }

    if state.edit.is_some() {
        return match key.code {
            KeyCode::Enter => (true, state.commit_edit()),
            KeyCode::Esc => {
                state.edit = None;
                state.edit_buffer.clear();
                (true, Vec::new())
            }
            _ => {
                let kind = state.edit;
                let accept: fn(char) -> bool = match kind {
                    Some(EditKind::Number) => is_edit_char,
                    _ => |c: char| c.is_ascii_graphic() || c == ' ',
                };
                state.edit_buffer.handle_key(&key, accept);
                (true, Vec::new())
            }
        };
    }

    let row = state.selected_row();
    if let (Some(c), Some(row)) = (crate::widgets::number_edit::number_char(&key), row) {
        if matches!(model::edit_kind(row), EditKind::Number | EditKind::Component) {
            state.start_edit(row, Some(c));
            return (true, Vec::new());
        }
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => {
            state.move_selection(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            state.move_selection(1);
            (true, Vec::new())
        }
        KeyCode::Left | KeyCode::Right => {
            let forward = key.code == KeyCode::Right;
            match row {
                Some(r @ (FieldRow::Template | FieldRow::Analysis | FieldRow::Element | FieldRow::SupportEdge(_) | FieldRow::LoadEdge(_))) => (true, state.activate(r, forward)),
                _ => (false, Vec::new()),
            }
        }
        KeyCode::Char(' ') => match row {
            Some(r) if model::edit_kind(r) == EditKind::Component || model::edit_kind(r) == EditKind::Action => (true, state.activate(r, true)),
            Some(r) if model::edit_kind(r) == EditKind::Text => {
                state.start_edit(r, None);
                (true, Vec::new())
            }
            _ => (true, Vec::new()),
        },
        KeyCode::Enter => match row {
            Some(FieldRow::Header(_)) | None => (false, Vec::new()),
            Some(r) => match model::edit_kind(r) {
                EditKind::Action => (true, state.activate(r, true)),
                _ => {
                    state.start_edit(r, None);
                    (true, Vec::new())
                }
            },
        },
        KeyCode::PageUp => {
            state.results_scroll = state.results_scroll.saturating_sub(crate::widgets::scroll_paragraph::SCROLL_STEP);
            (true, Vec::new())
        }
        KeyCode::PageDown => {
            state.results_scroll = state.results_scroll.saturating_add(crate::widgets::scroll_paragraph::SCROLL_STEP);
            (true, Vec::new())
        }
        KeyCode::Char(c) if !ctrl => match c.to_ascii_lowercase() {
            'r' => {
                state.show_dynamic = false;
                state.source += 1;
                (true, state.start_solve())
            }
            't' => (true,state.start_animation()),
            'g' => (true, state.open_gpu()),
            'n' => (true, state.start_dynamic(DynKind::Modal)),
            'b' => (true, state.start_dynamic(DynKind::Buckling)),
            'a' => {
                state.auto_solve = !state.auto_solve;
                (true, Vec::new())
            }
            'v' => {
                let all = Field::ALL;
                let i = all.iter().position(|f| *f == state.field).unwrap_or(0);
                state.field = all[(i + 1) % all.len()];
                (true, Vec::new())
            }
            'm' => {
                state.show_mesh = !state.show_mesh;
                (true, Vec::new())
            }
            '[' | ']' => {
                if let Some(n) = state.dynamic.as_ref().map(|d| d.n_modes()).filter(|n| *n > 0) {
                    state.mode = if c == ']' { (state.mode + 1) % n } else { (state.mode + n - 1) % n };
                    state.show_dynamic = true;
                    state.source += 1;
                }
                (true, Vec::new())
            }
            'x' => {
                state.deform = !state.deform;
                (true, Vec::new())
            }
            'd' => {
                state.details = !state.details;
                (true, Vec::new())
            }
            'e' => match state.report_text() {
                Some(text) => (true, state.write_effect("txt", text, true)),
                None => (true, Vec::new()),
            },
            'p' => match state.solved.as_ref().map(|s| s.vtu()) {
                Some(Ok(text)) => (true, state.write_effect("vtu", text, false)),
                _ => (true, Vec::new()),
            },
            'c' => match state.solved.as_ref().map(|s| s.csv()) {
                Some(text) => (true, state.write_effect("csv", text, false)),
                None => (true, Vec::new()),
            },
            'j' => match state.problem.to_json() {
                Ok(text) => (true, state.write_effect("json", text, false)),
                Err(_) => (true, Vec::new()),
            },
            'o' => {
                let mut buffer = EditBuffer::default();
                if let Some(dir) = FeaWorkbenchState::file_dir() {
                    buffer.set(format!("{}{}{}.json", dir.display(), std::path::MAIN_SEPARATOR, state.file_stem()));
                }
                state.prompt = Some(Prompt { label: "Open problem file", buffer });
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        },
        _ => (false, Vec::new()),
    }
}

#[cfg(test)]
mod tests;
