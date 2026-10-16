//! Dynamic studies run from the solid editor: natural frequencies, buckling and the load-release
//! transient. They use the same solvers as the terminal Workbench (`fea_problem::dynamics`,
//! `gpu_viewer::transient`) and produce scenes the window cycles through with `[` / `]`
//! (harmonic modes and the transient animate), so no separate viewer window is needed.
use super::*;
use crate::gpu_viewer::{scene::SceneSource, transient};
use fea_problem::dynamics::{self, DynKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudyKind {
    Modal,
    Buckling,
    Transient,
}
impl StudyKind {
    pub const ALL: [StudyKind; 3] = [Self::Modal, Self::Buckling, Self::Transient];
    pub fn label(self) -> &'static str {
        match self {
            Self::Modal => "Natural frequencies",
            Self::Buckling => "Buckling",
            Self::Transient => "Load-release transient",
        }
    }
}

/// Computed result scenes of one study, valid for exactly one problem.
pub struct Study {
    pub kind: StudyKind,
    pub problem: Problem,
    pub scenes: Vec<Scene>,
}

impl SolidSession {
    pub fn study_running(&self) -> bool {
        self.study_worker.is_running()
    }
    /// The study result when it was computed for the current inputs.
    pub fn current_study(&self) -> Option<&Study> {
        self.study.as_ref().filter(|s| s.problem == self.problem)
    }
    pub fn study_availability(&self, kind: StudyKind) -> Result<(), String> {
        if self.sketch_mode() {
            return Err("finish the sketch first: the model is rebuilt from it when you do".into());
        }
        if self.solving() {
            return Err("a computation is already running (Esc cancels it)".into());
        }
        self.problem.validate()?;
        if matches!(self.problem.geometry, Geometry::Imported { .. }) && self.import_text().is_none() {
            return Err("read the mesh file first (Enter on the Mesh File row)".into());
        }
        if kind == StudyKind::Transient && (!self.problem.bushings.is_empty() || self.problem.delta_t != 0.0) {
            return Err("the transient needs mechanical loads and no contact bushings or thermal load".into());
        }
        if kind != StudyKind::Buckling && !(self.problem.material.density.is_finite() && self.problem.material.density > 0.0) {
            return Err("set a positive mass density in the material first".into());
        }
        Ok(())
    }
    pub fn show_study_availability(&self) -> Result<(), String> {
        self.current_study().map(|_| ()).ok_or_else(|| "run a study first (Simulation menu); results of an older model are discarded".into())
    }
    pub fn run_study(&mut self, kind: StudyKind) {
        if let Err(e) = self.study_availability(kind) {
            self.status = e;
            return;
        }
        let (problem, import) = (self.problem.clone(), self.import_text());
        let tag = self.take_tag();
        let started = self.study_worker.start_observed(tag, move |progress| {
            progress.stage("Meshing and solving study");
            let scenes = match kind {
                StudyKind::Modal | StudyKind::Buckling => {
                    let (dk, n) = if kind == StudyKind::Modal { (DynKind::Modal, 6) } else { (DynKind::Buckling, 3) };
                    let solved = dynamics::run(dk, &problem, import.as_deref(), n)?;
                    SceneSource::from_dynamic(&solved).into_iter().map(SceneSource::build).collect::<Result<Vec<_>, _>>()?
                }
                StudyKind::Transient => vec![transient::run_cancellable(&problem, import.as_deref(),
                    &|p| progress.stage(p.text()),
                    &|| progress.cancelled(),
                )?],
            };
            if scenes.is_empty() {
                return Err("the study produced no result".into());
            }
            Ok(Study { kind, problem, scenes,
            })
        });
        self.status = match started {
            Ok(()) => format!("{}: computing...", kind.label()),
            Err(e) => e,
        };
    }
    /// Collect a finished study; true when something changed.
    pub(super) fn poll_study(&mut self) -> bool {
        let Some((_, answer)) = self.study_worker.poll() else { return false };
        match answer {
            Ok(study) => {
                let current = study.problem == self.problem;
                self.status = format!("{}: {} result(s){}", study.kind.label(), study.scenes.len(), if current { "" } else { " (model changed meanwhile: discarded)" });
                if current {
                    self.study_index = 0;
                    self.show_study = true;
                    self.show_results = false;
                    self.study = Some(study);
                }
            }
            Err(e) => self.status = format!("Study failed: {e}"),
        }
        self.touch();
        true
    }
    pub fn toggle_study(&mut self) {
        match self.show_study_availability() {
            Ok(()) => {
                self.show_study = !self.show_study;
                if self.show_study {
                    self.show_results = false;
                }
            }
            Err(e) => self.status = e,
        }
        self.touch();
    }
    /// Next / previous mode of the shown study.
    pub fn study_step(&mut self, forward: bool) {
        match self.current_study().map(|s| s.scenes.len()) {
            Some(n) if n > 0 => {
                self.study_index = if forward { (self.study_index + 1) % n } else { (self.study_index + n - 1) % n };
                self.show_study = true;
                self.show_results = false;
                self.status = format!("Result {} of {n}", self.study_index + 1);
            }
            _ => self.status = "run a study first (Simulation menu)".into(),
        }
        self.touch();
    }
    /// Scene of the study to display, if it is being shown.
    pub(super) fn study_scene(&self) -> Option<Scene> {
        let s = self.current_study().filter(|_| self.show_study)?;
        s.scenes.get(self.study_index.min(s.scenes.len() - 1)).cloned()
    }
}
