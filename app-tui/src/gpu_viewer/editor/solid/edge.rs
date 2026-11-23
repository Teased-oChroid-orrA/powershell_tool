//! The edge-distance estimate of a selected hole in the FEA Workbench: a pin bearing toward the nearest free edge, read
//! from the problem (the hole and its edge distance from the sketch, the plate's thickness and material, the hole's
//! bearing load) and run through the shared `edge_check::estimate`. Advisory: the workbench has no legacy edge check, so no
//! row is governing. It reads the problem and changes nothing in it.
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{authoring::{Card, Relevance}, session::Tone},
    ui::DockItem,
};
use edge_check::estimate::{self, BoreFit, EdgeEstimate, EstimateInput, EstimateOptions, LoadCase};
use edge_check::types::Strengths;
use fea_problem::{Analysis, Load, UnitSystem};

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(crate::gpu_viewer::editor::session::EditorAction::Solid(a))
}

/// A finished estimate and the input it was made for (a changed model makes it stale).
pub(super) struct EdgeOut {
    pub hole: usize,
    pub input: EstimateInput,
    pub estimate: EdgeEstimate,
}

/// The estimate input for hole `hole` of `p`, or why the model cannot be checked yet.
pub(super) fn build_input(p: &Problem, hole: usize) -> Result<(EstimateInput, Vec<String>), String> {
    if !matches!(p.analysis, Analysis::PlaneStress | Analysis::PlaneStrain) {
        return Err("the edge check reads a plane problem (plane stress or plane strain)".into());
    }
    if p.units != UnitSystem::InchPsi {
        return Err("the edge check works in inches, pounds and psi: switch the problem's units".into());
    }
    if p.bushings.iter().any(|b| b.hole == hole) {
        return Err(format!("hole {hole} has a bushing: its edge check is in the Bushing workbench (the fit pressure belongs there)"));
    }
    let h = p.hole_edge(hole)?;
    let load = p.loads.iter().find_map(|l| match l {
        Load::Bearing { hole: n, fx, fy } if *n == hole => Some(fx.hypot(*fy)),
        _ => None,
    });
    let load = load.ok_or_else(|| format!("give hole {hole} a bearing (pin) load first: the check needs the pin load"))?;
    let m = p.region_material(0);
    let sy = m.yield_stress.ok_or("the material has no yield strength: pick one from the library (Material)")?;
    let mut notes = Vec::new();
    let mut or_yield = |name: &str, v: Option<f64>| {
        v.unwrap_or_else(|| {
            notes.push(format!("{name} is not in the material: the yield strength stands in (a library material carries it)"));
            sy
        })
    };
    let (ftu, fsu, fbru) = (or_yield("Ftu", m.ftu), or_yield("Fsu", m.fsu), or_yield("Fbru", m.fbru));
    let material = Strengths { e: m.e, nu: m.nu, sy, fsu, ftu, fbru, fbru_e15: 0.0 };
    let input = EstimateInput {
        bore_radius: h.radius,
        edge: h.edge_distance,
        thickness: p.thickness,
        plane_angle_deg: 40.0,
        material,
        fit: BoreFit::Pin { friction: 0.0 },
        load: LoadCase::PinTowardEdge { load },
        fit_pressure: 0.0,
        fit_pressure_min: 0.0,
        fit_pressure_max: 0.0,
        legacy: None,
    };
    Ok((input, notes))
}

/// The hole the selection names, if it is one.
fn selected_hole(s: &SolidSession) -> Option<usize> {
    s.problem.hole_number(s.selected.as_deref()?)
}

impl SolidSession {
    /// Start the estimate for the selected hole on the worker (`deep`: with the contact FE).
    pub(super) fn start_edge_check(&mut self, deep: bool) {
        let Some(hole) = selected_hole(self) else {
            self.status = "Select a hole first.".into();
            return;
        };
        let (input, _) = match build_input(&self.problem, hole) {
            Ok(v) => v,
            Err(why) => {
                self.status = format!("Edge check: {why}.");
                return;
            }
        };
        let (edge_input, cfg) = match estimate::prepare(&input, &EstimateOptions { deep, ..Default::default() }) {
            Ok(v) => v,
            Err(why) => {
                self.status = format!("Edge check: {why}.");
                return;
            }
        };
        let job_input = input.clone();
        match self.edge_worker.start(hole as u64, move || Ok(EdgeOut { hole, input: job_input, estimate: estimate::execute(&edge_input, &cfg, None) })) {
            Ok(()) => self.status.clear(),
            Err(why) => self.status = format!("Edge check: {why}."),
        }
    }

    /// Collect a finished estimate; true when there was one.
    pub(super) fn poll_edge_check(&mut self) -> bool {
        match self.edge_worker.poll() {
            Some((_, Ok(out))) => {
                self.edge_run = Some(out);
                true
            }
            Some((_, Err(why))) => {
                self.status = format!("Edge check failed: {why}");
                true
            }
            None => false,
        }
    }

    /// The "Edge check" card of the selected hole: what it reads from the model, the run buttons and the shared matrix.
    pub(super) fn edge_card(&self) -> Option<Card> {
        let hole = selected_hole(self)?;
        let dim = |t: String| DockItem::text(t, Tone::Dim);
        let mut items = vec![dim("A pin bearing toward the nearest free edge, read from this model: the hole and its edge distance from the sketch, the plate's thickness and material, and the hole's bearing load (its size; the direction is not used). Advisory: this workbench has no legacy edge check.".into())];
        let built = build_input(&self.problem, hole);
        match &built {
            Err(why) => items.push(DockItem::text(format!("Cannot check hole {hole} yet: {why}."), Tone::Warn)),
            Ok((i, notes)) => {
                items.push(dim(format!("hole {hole}: diameter {:.4} in, edge distance {:.4} in (e/D {:.2}), thickness {:.4} in, pin load {:.0} lbf", 2.0 * i.bore_radius, i.edge, i.edge / (2.0 * i.bore_radius), i.thickness, match i.load { LoadCase::PinTowardEdge { load } => load })));
                items.extend(notes.iter().map(|n| DockItem::text(n.clone(), Tone::Warn)));
            }
        }
        if self.edge_worker.is_running() {
            items.push(DockItem::text(self.edge_worker.progress().unwrap_or_else(|| "running".into()), Tone::Warn));
        } else if built.is_ok() {
            items.push(DockItem::row("Run quick check", Tone::Title, Some(ui(SolidAction::EdgeCheck(false))), false));
            items.push(DockItem::row("Run with contact FE (about 3 s)", Tone::Title, Some(ui(SolidAction::EdgeCheck(true))), false));
        }
        let current = self.edge_run.as_ref().is_some_and(|r| r.hole == hole && built.as_ref().is_ok_and(|(i, _)| *i == r.input));
        let tag = match (&self.edge_run, self.edge_worker.is_running(), current) {
            (_, true, _) => "running",
            (None, false, _) => "not run",
            (Some(_), false, true) => "current",
            (Some(_), false, false) => "stale",
        };
        if let Some(run) = self.edge_run.as_ref().filter(|r| r.hole == hole) {
            if !current {
                items.push(DockItem::text("The model changed since this run; run it again.", Tone::Warn));
            }
            let r = &run.estimate.report;
            items.push(dim(format!("e/D {:.2}  load {:.0} lbf  bearing limit {:.0} lbf", r.edge / r.bore_diameter, r.applied_load, r.bearing_limit_load)));
            items.extend(crate::gpu_viewer::editor::edge_matrix::items(&run.estimate.matrix, r));
        }
        let card = Card::new("edge-check", "Edge check (advisory)", Relevance::Context, items).tag(tag);
        Some(if self.edge_run.as_ref().is_some_and(|r| r.hole == hole) { card } else { card.folded() })
    }
}
