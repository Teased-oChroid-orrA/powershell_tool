//! The edge-strength estimate of the Fastener Hole window, in two load cases: a pin or fastener bearing on the first hole
//! toward the free edge (the shared `edge_check::estimate`), or the plate itself under an axial load (`edge_check::plate`). The inputs (plate material, thickness, pin load, pin friction)
//! belong to the window and its case file, not to the terminal toolbox's model; the hole diameter and the edge distance
//! are the toolbox's own (nominal values). Fastener Hole has no legacy edge check, so the result is advisory: no row is
//! governing.
use crate::toolboxes::fastener_hole::model::{FastenerHoleModel, HoleType};
use edge_check::estimate::{self, BoreFit, EdgeEstimate, EstimateInput, EstimateOptions, LoadCase};
use edge_check::plate::{self, PlateEstimate, PlateInput};
use edge_check::types::Strengths;
use mechanics_core::materials::{Material, MATERIALS};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

/// Which load case the card computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadKind {
    /// A pin or fastener bearing on the hole, pulling toward the nearest free edge.
    #[default]
    PinBearing,
    /// The plate under an axial load along a strip of a given width.
    PlateAxial,
}

impl LoadKind {
    pub fn label(self) -> &'static str {
        match self {
            LoadKind::PinBearing => "Pin bearing toward the edge",
            LoadKind::PlateAxial => "Plate under axial load",
        }
    }
    pub fn next(self) -> Self {
        match self {
            LoadKind::PinBearing => LoadKind::PlateAxial,
            LoadKind::PlateAxial => LoadKind::PinBearing,
        }
    }
}

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
    pub case: LoadKind,
    /// Plate case: the strip's width, edge to edge, in.
    pub width: f64,
    /// Plate case: the load direction against the strip axis, degrees (0 along the strip, 90 all shear).
    pub angle_deg: f64,
    /// Plate case: distance from the hole centre to the loaded end of the strip, in (at least the width).
    pub arm: f64,
}

impl Default for EdgeInputs {
    fn default() -> Self {
        Self { material: "al7075".into(), thickness: 0.125, load: 1000.0, friction: 0.0, case: LoadKind::PinBearing, width: 4.0, angle_deg: 0.0, arm: 4.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeField {
    Thickness,
    Load,
    Friction,
    Width,
    Angle,
    Arm,
}

impl EdgeField {
    /// The inputs of a load case, in order.
    pub fn of(case: LoadKind) -> &'static [EdgeField] {
        match case {
            LoadKind::PinBearing => &[EdgeField::Thickness, EdgeField::Load, EdgeField::Friction],
            LoadKind::PlateAxial => &[EdgeField::Thickness, EdgeField::Load, EdgeField::Width, EdgeField::Angle, EdgeField::Arm],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EdgeField::Thickness => "Plate thickness",
            EdgeField::Load => "Load",
            EdgeField::Friction => "Pin-hole friction",
            EdgeField::Width => "Plate width",
            EdgeField::Angle => "Load angle to the strip axis",
            EdgeField::Arm => "Load distance from the hole",
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            EdgeField::Thickness => " in",
            EdgeField::Load => " lbf",
            EdgeField::Friction => "",
            EdgeField::Width => " in",
            EdgeField::Angle => " deg",
            EdgeField::Arm => " in",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            EdgeField::Thickness => "Plate thickness at the hole. Must be positive.",
            EdgeField::Load => "Pin case: the load the pin puts on the hole, pulling toward the nearest free edge (the worst case). Plate case: the axial load on the strip. Must not be negative.",
            EdgeField::Friction => "Coulomb coefficient between pin and hole, 0 to 1. The validation tests agree best with 0.",
            EdgeField::Width => "Strip width, edge to edge. The Edge Distance must not exceed half of it (the edge named is the nearer one).",
            EdgeField::Angle => "Direction of the plate load against the strip axis: 0 is along the strip (parallel to the free edges), 90 is all shear. The load is an inclined end load: its axial part is uniform, its transverse part parabolic, and the far end reacts both with the shear and the end moment. Both transverse directions are computed and the worse is shown.",
            EdgeField::Arm => "Distance from the hole centre to the loaded end of the strip (the far, reacting end is as far on the other side). At least the plate width; it sets the moment at the hole, shear times this distance.",
        }
    }
    pub fn get(self, e: &EdgeInputs) -> f64 {
        match self {
            EdgeField::Thickness => e.thickness,
            EdgeField::Load => e.load,
            EdgeField::Friction => e.friction,
            EdgeField::Width => e.width,
            EdgeField::Angle => e.angle_deg,
            EdgeField::Arm => e.arm,
        }
    }
    /// Sets the value, or says why it is not one.
    pub fn set(self, e: &mut EdgeInputs, v: f64) -> Result<(), String> {
        let ok = match self {
            EdgeField::Thickness => v.is_finite() && v > 0.0,
            EdgeField::Load => v.is_finite() && v >= 0.0,
            EdgeField::Friction => v.is_finite() && (0.0..=1.0).contains(&v),
            EdgeField::Width => v.is_finite() && v > 0.0,
            EdgeField::Angle => v.is_finite() && (0.0..=90.0).contains(&v),
            EdgeField::Arm => v.is_finite() && v > 0.0,
        };
        if !ok {
            return Err(format!("{} not changed: {v} is out of range. {}", self.label(), self.hint()));
        }
        match self {
            EdgeField::Thickness => e.thickness = v,
            EdgeField::Load => e.load = v,
            EdgeField::Friction => e.friction = v,
            EdgeField::Width => e.width = v,
            EdgeField::Angle => e.angle_deg = v,
            EdgeField::Arm => e.arm = v,
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

/// The input of the live model for the chosen load case: the first hole's nominal diameter and the nominal edge distance.
pub fn estimate_input(m: &FastenerHoleModel, e: &EdgeInputs) -> Result<EdgeCase, String> {
    let diameter = match m.hole_type {
        HoleType::Regular => m.regular.hole1.nominal,
        HoleType::Countersunk => m.countersink.hole.nominal,
    };
    let edge = m.edge_distance.ok_or("enter an Edge Distance under Hole Layout first")?.nominal;
    Ok(match e.case {
        LoadKind::PinBearing => EdgeCase::Pin(EstimateInput {
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
        }),
        LoadKind::PlateAxial => EdgeCase::Plate(PlateInput { bore_radius: diameter / 2.0, edge, width: e.width, thickness: e.thickness, material: Strengths::from_material(e.plate()), load: e.load, angle_deg: e.angle_deg, arm: e.arm }),
    })
}

/// What an estimate was made for.
#[derive(Clone, Debug, PartialEq)]
pub enum EdgeCase {
    Pin(EstimateInput),
    Plate(PlateInput),
}

/// Its result.
pub enum EdgeResult {
    Pin(EdgeEstimate),
    Plate(PlateEstimate),
}

/// A finished run and the input it was made for (a changed input makes it stale).
pub struct EdgeRun {
    pub input: EdgeCase,
    pub result: EdgeResult,
}

/// A run on a worker thread.
pub struct EdgeJob {
    pub started: Instant,
    rx: Receiver<Option<EdgeRun>>,
}

/// Validates and starts the run on a worker thread (the pin case: about 0.3 s, about 3 s with the contact FE; the plate
/// case: a few seconds for the minimum edge distances).
pub fn start(m: &FastenerHoleModel, e: &EdgeInputs, deep: bool) -> Result<EdgeJob, String> {
    let input = estimate_input(m, e)?;
    let (tx, rx) = channel();
    let work: Box<dyn FnOnce() -> Result<EdgeResult, String> + Send> = match input.clone() {
        EdgeCase::Pin(i) => {
            let (edge_input, cfg) = estimate::prepare(&i, &EstimateOptions { deep, ..Default::default() })?;
            Box::new(move || Ok(EdgeResult::Pin(estimate::execute(&edge_input, &cfg, None))))
        }
        EdgeCase::Plate(i) => {
            plate::validate(&i)?;
            Box::new(move || plate::estimate(&i).map(EdgeResult::Plate))
        }
    };
    std::thread::Builder::new()
        .name("fastener-edge-estimate".into())
        .spawn(move || {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).ok().and_then(|r| r.ok()).map(|result| EdgeRun { input, result });
            let _ = tx.send(run);
        })
        .map_err(|e| format!("could not start the edge estimate: {e}"))?;
    Ok(EdgeJob { started: Instant::now(), rx })
}

/// What a job has produced: `None` while it runs, `Some(None)` if it failed.
pub fn poll(job: &EdgeJob) -> Option<Option<EdgeRun>> {
    match job.rx.try_recv() {
        Ok(run) => Some(run),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(None),
    }
}
