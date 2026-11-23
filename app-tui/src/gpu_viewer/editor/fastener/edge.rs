//! The edge-strength estimate of the Fastener Hole window: a pin or fastener bearing on the first hole toward the free
//! edge, run through the shared `edge_check::estimate`. The inputs (plate material, thickness, pin load, pin friction)
//! belong to the window and its case file, not to the terminal toolbox's model; the hole diameter and the edge distance
//! are the toolbox's own (nominal values). Fastener Hole has no legacy edge check, so the result is advisory: no row is
//! governing.
use crate::toolboxes::fastener_hole::model::{FastenerHoleModel, HoleType};
use edge_check::estimate::{self, BoreFit, EdgeEstimate, EstimateInput, EstimateOptions, LoadCase};
use edge_check::types::Strengths;
use mechanics_core::materials::{Material, MATERIALS};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

/// What the estimate needs beyond the hole: saved with the case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeInputs {
    /// `Material::id` of the plate.
    pub material: String,
    /// Plate thickness at the hole, in.
    pub thickness: f64,
    /// Pin load toward the free edge, lbf.
    pub load: f64,
    /// Pin-hole friction coefficient; 0 agrees best with the validation tests.
    pub friction: f64,
}

impl Default for EdgeInputs {
    fn default() -> Self {
        Self { material: "al7075".into(), thickness: 0.125, load: 1000.0, friction: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeField {
    Thickness,
    Load,
    Friction,
}

impl EdgeField {
    pub const ALL: [EdgeField; 3] = [EdgeField::Thickness, EdgeField::Load, EdgeField::Friction];

    pub fn label(self) -> &'static str {
        match self {
            EdgeField::Thickness => "Plate thickness",
            EdgeField::Load => "Pin load toward the edge",
            EdgeField::Friction => "Pin-hole friction",
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            EdgeField::Thickness => " in",
            EdgeField::Load => " lbf",
            EdgeField::Friction => "",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            EdgeField::Thickness => "Plate thickness at the hole. Must be positive.",
            EdgeField::Load => "The load the pin puts on the hole, pulling toward the nearest free edge (the worst case). Must not be negative.",
            EdgeField::Friction => "Coulomb coefficient between pin and hole, 0 to 1. The validation tests agree best with 0.",
        }
    }
    pub fn get(self, e: &EdgeInputs) -> f64 {
        match self {
            EdgeField::Thickness => e.thickness,
            EdgeField::Load => e.load,
            EdgeField::Friction => e.friction,
        }
    }
    /// Sets the value, or says why it is not one.
    pub fn set(self, e: &mut EdgeInputs, v: f64) -> Result<(), String> {
        let ok = match self {
            EdgeField::Thickness => v.is_finite() && v > 0.0,
            EdgeField::Load => v.is_finite() && v >= 0.0,
            EdgeField::Friction => v.is_finite() && (0.0..=1.0).contains(&v),
        };
        if !ok {
            return Err(format!("{} not changed: {v} is out of range. {}", self.label(), self.hint()));
        }
        match self {
            EdgeField::Thickness => e.thickness = v,
            EdgeField::Load => e.load = v,
            EdgeField::Friction => e.friction = v,
        }
        Ok(())
    }
}

impl EdgeInputs {
    /// The plate material (the default aluminium if the file names one this build does not have).
    pub fn plate(&self) -> &'static Material {
        MATERIALS.iter().find(|m| m.id == self.material).unwrap_or_else(|| mechanics_core::materials::get_material("al7075"))
    }

    /// The next material of the table.
    pub fn cycle_material(&mut self) {
        let i = MATERIALS.iter().position(|m| m.id == self.plate().id).unwrap_or(0);
        self.material = MATERIALS[(i + 1) % MATERIALS.len()].id.to_string();
    }
}

/// The estimate input for the live model: the first hole's nominal diameter, the nominal edge distance, a pin in a bare
/// hole, no legacy check.
pub fn estimate_input(m: &FastenerHoleModel, e: &EdgeInputs) -> Result<EstimateInput, String> {
    let diameter = match m.hole_type {
        HoleType::Regular => m.regular.hole1.nominal,
        HoleType::Countersunk => m.countersink.hole.nominal,
    };
    let edge = m.edge_distance.ok_or("enter an Edge Distance under Hole Layout first")?.nominal;
    Ok(EstimateInput {
        bore_radius: diameter / 2.0,
        edge,
        thickness: e.thickness,
        plane_angle_deg: 40.0,
        material: Strengths::from_material(e.plate()),
        fit: BoreFit::Pin { friction: e.friction },
        load: LoadCase::PinTowardEdge { load: e.load },
        fit_pressure: 0.0,
        fit_pressure_min: 0.0,
        fit_pressure_max: 0.0,
        legacy: None,
    })
}

/// A finished run and the input it was made for (a changed input makes it stale).
pub struct EdgeRun {
    pub input: EstimateInput,
    pub estimate: EdgeEstimate,
}

/// A run on a worker thread.
pub struct EdgeJob {
    pub deep: bool,
    pub started: Instant,
    rx: Receiver<Option<EdgeRun>>,
}

/// Validates and starts the run on a worker thread (about 0.3 s, about 3 s with the contact FE).
pub fn start(m: &FastenerHoleModel, e: &EdgeInputs, deep: bool) -> Result<EdgeJob, String> {
    let input = estimate_input(m, e)?;
    let (edge_input, cfg) = estimate::prepare(&input, &EstimateOptions { deep, ..Default::default() })?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("fastener-edge-estimate".into())
        .spawn(move || {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| EdgeRun { estimate: estimate::execute(&edge_input, &cfg, None), input })).ok();
            let _ = tx.send(run);
        })
        .map_err(|e| format!("could not start the edge estimate: {e}"))?;
    Ok(EdgeJob { deep, started: Instant::now(), rx })
}

/// What a job has produced: `None` while it runs, `Some(None)` if it failed.
pub fn poll(job: &EdgeJob) -> Option<Option<EdgeRun>> {
    match job.rx.try_recv() {
        Ok(run) => Some(run),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(None),
    }
}
