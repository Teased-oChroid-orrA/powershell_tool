//! Heat-conduction studies on the geometry of a problem: steady conduction with fixed temperatures, heat fluxes, convection
//! and a volumetric source, optionally followed by the thermal stress of the temperature field it finds (one-way coupling,
//! `fea_core::Loads::temperature`). The mesh, the names of surfaces and the supports-and-loads vocabulary are the structural
//! problem's; the thermal boundary conditions are their own lists so a model can carry both.
use crate::problem::Problem;
use crate::solve::{named_faces, named_nodes, nearest_node, Extreme};
use fea_core::thermal::{Conductivity, HeatLoads};
use fea_core::{Dirichlet, HeatSolution, Mesh, Model};
use serde::{Deserialize, Serialize};

/// What a solve computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Study {
    /// Displacements and stresses from the supports and loads.
    #[default]
    Structural,
    /// The steady temperature field from the thermal boundary conditions.
    Thermal,
    /// The temperature field, then the stresses it causes (with the supports and loads, if any).
    ThermalStress,
}

impl Study {
    pub const ALL: [Study; 3] = [Study::Structural, Study::Thermal, Study::ThermalStress];
    pub fn label(self) -> &'static str {
        match self {
            Study::Structural => "Structural",
            Study::Thermal => "Thermal (steady)",
            Study::ThermalStress => "Thermal stress",
        }
    }
    /// Does the study conduct heat?
    pub fn is_thermal(self) -> bool {
        self != Study::Structural
    }
    /// Does the study compute displacements and stresses?
    pub fn is_structural(self) -> bool {
        self != Study::Thermal
    }
}

/// A fixed temperature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HeatSupport {
    /// Every node of the named surface (or node set) is held at `t`.
    Temperature { edge: String, t: f64 },
    /// The node nearest `(x, y, z)` is held at `t`.
    Point { x: f64, y: f64, z: f64, t: f64 },
}

/// A heat input or exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HeatLoad {
    /// Heat flowing into the body through a surface, per unit area (negative: out of it).
    Flux { edge: String, q: f64 },
    /// Convection to an ambient at `ambient` with film coefficient `h`; the heat entering is `h (ambient - T)`.
    Convection { edge: String, h: f64, ambient: f64 },
    /// Heat input at the node nearest `(x, y, z)`.
    Point { x: f64, y: f64, z: f64, q: f64 },
}

/// The thermal half of a problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalSpec {
    /// Thermal conductivity (energy per time, length and degree, in the problem's consistent units).
    pub conductivity: f64,
    /// Heat generated per unit volume and time.
    #[serde(default)]
    pub source: f64,
    /// Temperature at which the structure is stress free (thermal stress only).
    #[serde(default)]
    pub reference: f64,
    #[serde(default)]
    pub supports: Vec<HeatSupport>,
    #[serde(default)]
    pub loads: Vec<HeatLoad>,
}

impl Default for ThermalSpec {
    fn default() -> Self {
        Self { conductivity: 1.0, source: 0.0, reference: 0.0, supports: Vec::new(), loads: Vec::new() }
    }
}

impl ThermalSpec {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The numbers are usable; the names are checked against the mesh at solve time.
    pub fn validate(&self) -> Result<(), String> {
        let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
        if !(self.conductivity.is_finite() && self.conductivity > 0.0) {
            return Err("the thermal conductivity must be positive".into());
        }
        if !finite(&[self.source, self.reference]) {
            return Err("the heat source and the reference temperature must be finite".into());
        }
        for s in &self.supports {
            let ok = match s {
                HeatSupport::Temperature { t, .. } => finite(&[*t]),
                HeatSupport::Point { x, y, z, t } => finite(&[*x, *y, *z, *t]),
            };
            if !ok {
                return Err("a fixed temperature needs finite numbers".into());
            }
        }
        for l in &self.loads {
            let ok = match l {
                HeatLoad::Flux { q, .. } => finite(&[*q]),
                HeatLoad::Convection { h, ambient, .. } => finite(&[*h, *ambient]) && *h >= 0.0,
                HeatLoad::Point { x, y, z, q } => finite(&[*x, *y, *z, *q]),
            };
            if !ok {
                return Err("a heat load needs finite numbers (a convection coefficient is not negative)".into());
            }
        }
        Ok(())
    }
}

/// What the heat solve found, for the summary and report.
#[derive(Debug, Clone, PartialEq)]
pub struct HeatSummary {
    pub t_min: Extreme,
    pub t_max: Extreme,
    /// Heat entering through sources, fluxes and point inputs.
    pub heat_in: f64,
    /// Heat leaving through the fixed temperatures and the convection faces.
    pub heat_out: f64,
    /// Global energy imbalance after the solver's floating-point allowance (zero for a conducting bar between two temperatures).
    pub balance_error: f64,
    pub iterations: usize,
}

/// The thermal boundary conditions of a problem on its mesh.
pub(crate) struct Setup {
    pub bc: Dirichlet,
    pub loads: HeatLoads,
}

/// Give the blocks of `mesh` their conductivity and translate the thermal boundary conditions (surfaces used as a heat
/// surface get their faces registered, so this needs `&mut`).
pub(crate) fn prepare(p: &Problem, mesh: &mut Mesh) -> Result<Setup, String> {
    let th = &p.thermal;
    th.validate()?;
    for b in 0..mesh.blocks.len() {
        // Steady conduction does not use the capacity.
        mesh.set_thermal(b, Conductivity::Constant(th.conductivity), 0.0)?;
    }
    let mut bc = Dirichlet::new(mesh.nodes.len(), 1);
    for s in &th.supports {
        match s {
            HeatSupport::Temperature { edge, t } => {
                for n in named_nodes(mesh, edge)? {
                    bc.fix(n, 0, *t);
                }
            }
            HeatSupport::Point { x, y, z, t } => bc.fix(nearest_node(mesh, [*x, *y, *z]), 0, *t),
        }
    }
    let mut loads = HeatLoads { source: th.source, ..HeatLoads::default() };
    for l in &th.loads {
        match l {
            HeatLoad::Flux { edge, q } => {
                for f in named_faces(mesh, edge)? {
                    loads.flux.push((f, *q));
                }
            }
            HeatLoad::Convection { edge, h, ambient } => {
                for f in named_faces(mesh, edge)? {
                    loads.convection.push((f, *h, *ambient));
                }
            }
            HeatLoad::Point { x, y, z, q } => loads.nodal.push((nearest_node(mesh, [*x, *y, *z]), *q)),
        }
    }
    Ok(Setup { bc, loads })
}

/// Steady temperature field and its summary.
pub(crate) fn solve_heat(model: &Model, setup: &Setup) -> Result<(HeatSolution, HeatSummary), String> {
    let heat = model.solve_heat_steady(&setup.loads, &setup.bc, 1e-10).map_err(|e| {
        if e.contains("fixed temperature or a convection") {
            "a thermal study needs a fixed temperature or a convection surface (otherwise the temperature level is undetermined)".to_string()
        } else {
            e
        }
    })?;
    let at = |better: &dyn Fn(f64, f64) -> bool| {
        let mut best = 0;
        for (i, t) in heat.temperature.iter().enumerate() {
            if better(*t, heat.temperature[best]) {
                best = i;
            }
        }
        Extreme { value: heat.temperature.get(best).copied().unwrap_or(0.0), at: model.mesh.nodes.get(best).copied().unwrap_or([0.0; 3]) }
    };
    let summary = HeatSummary {
        t_min: at(&|a, b| a < b),
        t_max: at(&|a, b| a > b),
        heat_in: heat.heat_in,
        heat_out: heat.heat_out_fixed + heat.heat_out_convection,
        balance_error: heat.accepted_balance_error,
        iterations: heat.iterations,
    };
    Ok((heat, summary))
}

// ----------------------------------------------------------------------------- editing helpers
// By index and kind so an editor can list and change a boundary condition without knowing its variants.

impl HeatSupport {
    pub fn edge(&self) -> Option<&str> {
        match self {
            HeatSupport::Temperature { edge, .. } => Some(edge),
            HeatSupport::Point { .. } => None,
        }
    }
    pub fn set_edge(&mut self, name: &str) {
        if let HeatSupport::Temperature { edge, .. } = self {
            *edge = name.to_string();
        }
    }
    pub fn temperature(&self) -> f64 {
        match self {
            HeatSupport::Temperature { t, .. } | HeatSupport::Point { t, .. } => *t,
        }
    }
    pub fn set_temperature(&mut self, v: f64) {
        match self {
            HeatSupport::Temperature { t, .. } | HeatSupport::Point { t, .. } => *t = v,
        }
    }
    /// Point coordinates (empty for a surface).
    pub fn coords(&self) -> Vec<(&'static str, f64)> {
        match self {
            HeatSupport::Point { x, y, z, .. } => vec![("X", *x), ("Y", *y), ("Z", *z)],
            HeatSupport::Temperature { .. } => Vec::new(),
        }
    }
    pub fn set_coord(&mut self, i: usize, v: f64) {
        if let HeatSupport::Point { x, y, z, .. } = self {
            match i {
                0 => *x = v,
                1 => *y = v,
                _ => *z = v,
            }
        }
    }
    /// Toggle between a surface and a point.
    pub fn toggled(&self, first_edge: &str) -> HeatSupport {
        match self {
            HeatSupport::Temperature { t, .. } => HeatSupport::Point { x: 0.0, y: 0.0, z: 0.0, t: *t },
            HeatSupport::Point { t, .. } => HeatSupport::Temperature { edge: first_edge.into(), t: *t },
        }
    }
}

impl HeatLoad {
    pub fn label(&self) -> &'static str {
        match self {
            HeatLoad::Flux { .. } => "Heat flux",
            HeatLoad::Convection { .. } => "Convection",
            HeatLoad::Point { .. } => "Point heat input",
        }
    }
    pub fn edge(&self) -> Option<&str> {
        match self {
            HeatLoad::Flux { edge, .. } | HeatLoad::Convection { edge, .. } => Some(edge),
            HeatLoad::Point { .. } => None,
        }
    }
    pub fn set_edge(&mut self, name: &str) {
        if let HeatLoad::Flux { edge, .. } | HeatLoad::Convection { edge, .. } = self {
            *edge = name.to_string();
        }
    }
    /// Editable numbers, in a fixed order.
    pub fn params(&self) -> Vec<(&'static str, f64)> {
        match self {
            HeatLoad::Flux { q, .. } => vec![("Flux (into the body)", *q)],
            HeatLoad::Convection { h, ambient, .. } => vec![("Film coefficient h", *h), ("Ambient temperature", *ambient)],
            HeatLoad::Point { x, y, z, q } => vec![("X", *x), ("Y", *y), ("Z", *z), ("Heat input", *q)],
        }
    }
    pub fn set_param(&mut self, i: usize, v: f64) {
        match (self, i) {
            (HeatLoad::Flux { q, .. }, 0) => *q = v,
            (HeatLoad::Convection { h, .. }, 0) => *h = v.max(0.0),
            (HeatLoad::Convection { ambient, .. }, 1) => *ambient = v,
            (HeatLoad::Point { x, .. }, 0) => *x = v,
            (HeatLoad::Point { y, .. }, 1) => *y = v,
            (HeatLoad::Point { z, .. }, 2) => *z = v,
            (HeatLoad::Point { q, .. }, 3) => *q = v,
            _ => {}
        }
    }
    /// The next kind (flux, convection, point input), keeping what carries over.
    pub fn next_kind(&self, first_edge: &str) -> HeatLoad {
        let edge = self.edge().unwrap_or(first_edge).to_string();
        match self {
            HeatLoad::Flux { .. } => HeatLoad::Convection { edge, h: 10.0, ambient: 20.0 },
            HeatLoad::Convection { .. } => HeatLoad::Point { x: 0.0, y: 0.0, z: 0.0, q: 1.0 },
            HeatLoad::Point { .. } => HeatLoad::Flux { edge, q: 1.0 },
        }
    }
}
