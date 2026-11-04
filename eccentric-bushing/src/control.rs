//! How a long run is steered and watched: a stop (deadline or cancel flag) and a progress record the caller can poll.

use fea_core::{Interrupt, StepObserver};
use crate::model::FitCache;
use std::sync::{Arc, Mutex};

/// What a run is doing now, for a live display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressState {
    /// The stage of the solve in flight: "fit", "press-in", "pin load" (empty before the first solve).
    pub stage: String,
    /// The last converged step of that solve ("e 0.0200 in, 8856 lbf: step 12, pin carries 1180 lbf").
    pub detail: String,
    /// Finite-element solves finished (a search counts every candidate).
    pub solves_done: usize,
    /// The interval a search has narrowed the answer to so far, in the caller's inches or lbf:
    /// `(verified to hold, not holding or unresolved)`.
    pub bracket: Option<(f64, f64)>,
    /// True only when the bracket's lower bound was tested on the selected capacity basis.
    pub bracket_design_basis: bool,
}

/// Shared progress record: the worker writes, the display reads a snapshot.
#[derive(Debug, Default)]
pub struct Progress(Mutex<ProgressState>);

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> ProgressState {
        self.0.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Replace the record (a worker with its own solver loop, a test).
    pub fn set(&self, state: ProgressState) {
        self.update(|s| *s = state);
    }

    fn update(&self, f: impl FnOnce(&mut ProgressState)) {
        if let Ok(mut s) = self.0.lock() {
            f(&mut s);
        }
    }
}

/// A stop and an optional progress record for one run (`Default`: neither).
#[derive(Debug, Clone)]
pub struct Control {
    pub interrupt: Interrupt,
    pub progress: Option<Arc<Progress>>,
    pub(crate) fit_cache: Option<Arc<FitCache>>,
    progress_scale: f64,
    bracket_design_basis: bool,
}

impl Default for Control {
    fn default() -> Self {
        Self::new(Interrupt::default(), None)
    }
}

impl From<Interrupt> for Control {
    fn from(interrupt: Interrupt) -> Self {
        Self { interrupt, progress: None, fit_cache: None, progress_scale: 1.0, bracket_design_basis: false }
    }
}

impl Control {
    pub fn new(interrupt: Interrupt, progress: Option<Arc<Progress>>) -> Self {
        Self { interrupt, progress, fit_cache: None, progress_scale: 1.0, bracket_design_basis: false }
    }

    pub fn with_fit_cache(mut self, cache: Arc<FitCache>) -> Self {
        self.fit_cache = Some(cache);
        self
    }

    pub(crate) fn with_progress_scale(mut self, scale: f64) -> Self {
        self.progress_scale = scale;
        self
    }

    pub(crate) fn with_bracket_basis(mut self, design_basis: bool) -> Self {
        self.bracket_design_basis = design_basis;
        self
    }

    pub(crate) fn stage(&self, name: &str) {
        if let Some(p) = &self.progress {
            p.update(|s| name.clone_into(&mut s.stage));
        }
    }

    pub(crate) fn solve_done(&self) {
        if let Some(p) = &self.progress {
            p.update(|s| s.solves_done += 1);
        }
    }

    pub(crate) fn bracket(&self, lo: f64, hi: f64) {
        if let Some(p) = &self.progress {
            p.update(|s| {
                s.bracket = Some((lo * self.progress_scale, hi * self.progress_scale));
                s.bracket_design_basis = self.bracket_design_basis;
            });
        }
    }

    /// The step observer of a solve labelled `label` (`None` without a progress record).
    pub(crate) fn observer(&self, label: String) -> Option<StepObserver> {
        let p = self.progress.clone()?;
        Some(StepObserver(Arc::new(move |e| {
            p.update(|s| {
                s.detail = match e.master_force {
                    Some(f) => format!("{label}: step {}, pin carries {f:.0} lbf", e.step),
                    None => format!("{label}: step {}", e.step),
                }
            })
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_brackets_report_physical_units() {
        let progress = Progress::new();
        let ctl = Control::new(Interrupt::default(), Some(progress.clone())).with_progress_scale(0.25).with_bracket_basis(true);
        ctl.bracket(0.4, 0.8);
        assert_eq!(progress.snapshot().bracket, Some((0.1, 0.2)));
        assert!(progress.snapshot().bracket_design_basis);
    }
}
