//! Long pressure LINE (hydraulic / fuel / pneumatic tube) stress check: a straight tube run supported
//! at `x` positions, loaded by internal/external pressure, distributed weight with load factors,
//! point loads (valves, actuators, flow thrust at bends/tees) and thermal growth between axial
//! anchors, checked with the standard failure criterion for the material class. Docs and sources:
//! `docs/pressure-vessel-line-run.md`.
//!
//! Pipeline per load case: (1) member actions along the run from the 1-D beam / bar solvers in
//! `line_beam.rs` (two orthogonal bending planes plus the axial bar; equilibrium self-checked);
//! (2) at every station, on the inner and outer surface and around the circumference, the stress
//! tensor `sigma_x` (pressure end load + axial force + bending), hoop and radial (exact Lame via
//! `stress.rs`) and the thin-wall transverse shear `tau_x,theta`; (3) the criteria of the material class
//! (`failure.rs` von Mises / Tresca / maximum-principal for ductile metals; Rankine and Coulomb-Mohr
//! for brittle ones). Composite or other classes are refused. Allowables and factors of safety are
//! INPUTS: nothing here has a default allowable.
//!
//! Units are the caller's, consistent (e.g. in, lbf, psi, lbf/in^3, 1/degF, degF). Coordinates: `x`
//! along the line, `y` and `z` transverse; the circumferential angle `theta` runs from `+y` toward
//! `+z`. A load factor vector `n` multiplies weight: the inertial/gravity force on weight `W` is `W n`
//! (one g with `z` up is `n = [0, 0, -1]`).
//!
//! Not modelled (refused or stated, never guessed): torsion (a torque about the line axis needs a
//! torsional-restraint model the owner has not specified, so such loads are refused); fatigue,
//! vibration and buckling; partial (spring) axial anchors; local stresses at bends, tees and
//! clamps (stress intensification); radial thermal gradient through the wall.

use crate::failure::{max_abs_principal_stress, tresca_stress, von_mises_stress};
use crate::geometry::CylinderGeometry;
use crate::line_beam::{solve_bar, solve_beam, NodeBc};
use crate::pressure::{EndCondition, PressureLoading};
use crate::stress::{stress_at_inner_surface, stress_at_outer_surface, StressState};
use std::f64::consts::PI;

#[derive(Debug, Clone, PartialEq)]
pub enum LineRunError {
    /// Input is non-finite, non-positive where it must be positive, or inconsistent.
    InvalidInput(String),
    /// The request is outside what this check supports (composites, torsion): a reason, not a number.
    Refused(String),
    /// The supports do not hold the line (mechanism) or an anchor is missing for an axial load.
    Unstable(String),
    /// A solve or equilibrium self-check failed its acceptance limit.
    Rejected(String),
}
impl std::fmt::Display for LineRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (k, s) = match self {
            Self::InvalidInput(s) => ("invalid input", s),
            Self::Refused(s) => ("refused", s),
            Self::Unstable(s) => ("unstable", s),
            Self::Rejected(s) => ("rejected", s),
        };
        write!(f, "{k}: {s}")
    }
}
impl std::error::Error for LineRunError {}

/// Equilibrium / backward-error acceptance limit of every solve (relative).
pub const BALANCE_LIMIT: f64 = 1e-9;
/// Each segment between consecutive key points is divided into this many stations unless
/// `LineRun::subdivisions` says otherwise (peak moments are exact either way: see `stations`).
pub const DEFAULT_SUBDIVISIONS: usize = 8;
/// Coarse circumferential samples before the golden-section refinement of the worst angle.
const THETA_SAMPLES: usize = 360;

/// Tube section and material constants. The tube weight is `weight_density * area` per length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tube {
    pub outer_diameter: f64,
    pub wall: f64,
    pub e: f64,
    /// Coefficient of thermal expansion, strain per degree.
    pub alpha: f64,
    /// Weight per volume (force/length^3); 0 for a weightless tube.
    pub weight_density: f64,
}
impl Tube {
    pub fn geometry(&self) -> Result<CylinderGeometry, LineRunError> {
        let ro = self.outer_diameter / 2.0;
        CylinderGeometry::new(ro - self.wall, ro).map_err(|e| LineRunError::InvalidInput(format!("tube geometry: {e:?}")))
    }
    fn validate(&self) -> Result<(), LineRunError> {
        for (n, v) in [("outer_diameter", self.outer_diameter), ("wall", self.wall), ("e", self.e)] {
            if !(v.is_finite() && v > 0.0) {
                return Err(LineRunError::InvalidInput(format!("tube {n} must be positive and finite")));
            }
        }
        if !(self.alpha.is_finite() && self.weight_density.is_finite() && self.weight_density >= 0.0) {
            return Err(LineRunError::InvalidInput("tube alpha / weight_density must be finite (weight_density >= 0)".into()));
        }
        self.geometry().map(|_| ())
    }
    pub fn area(&self) -> f64 {
        let g = self.geometry().expect("validated");
        PI * (g.outer_radius.powi(2) - g.inner_radius.powi(2))
    }
    /// Second moment of area about a diameter.
    pub fn inertia(&self) -> f64 {
        let g = self.geometry().expect("validated");
        PI / 4.0 * (g.outer_radius.powi(4) - g.inner_radius.powi(4))
    }
    /// Bore flow area `pi r_i^2` (for flow thrust).
    pub fn flow_area(&self) -> f64 {
        PI * self.geometry().expect("validated").inner_radius.powi(2)
    }
}

/// A support at station `x`: transversely pinned in `y` and `z`. `axial_anchor` also holds `x`;
/// `rotation_fixed` clamps the slope (a rigid fixture, not a clip or saddle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Support {
    pub x: f64,
    pub axial_anchor: bool,
    pub rotation_fixed: bool,
}

/// A point load at `x`. `weight` is a lumped weight scaled by the case's load factors; `force` is
/// applied as given. `offset` is the arm from the line axis point to the load's line of action
/// (valve centre of gravity), giving the moment `offset x (force + weight n)` added to `moment`.
/// The resulting moment about the line axis must vanish (no torsion model).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointLoad {
    pub x: f64,
    pub force: [f64; 3],
    pub weight: f64,
    pub moment: [f64; 3],
    pub offset: [f64; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineRun {
    pub tube: Tube,
    pub x_start: f64,
    pub x_end: f64,
    pub supports: Vec<Support>,
    /// Contents (fluid) weight per length and insulation / fittings weight per length (force/length).
    pub contents_weight_per_length: f64,
    pub insulation_weight_per_length: f64,
    /// Hardware loads present in every case (valves, actuators).
    pub point_loads: Vec<PointLoad>,
    /// Stations per segment (peak moments are exact regardless); `DEFAULT_SUBDIVISIONS` is typical.
    pub subdivisions: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadCase {
    pub name: String,
    pub internal_pressure: f64,
    pub external_pressure: f64,
    /// `true`: the pressure end load is carried by the tube wall (capped / blind-ended line:
    /// `sigma_x = PD/4t` through Lame, `EndCondition::Closed`). `false`: no pressure axial stress, e.g.
    /// a line whose thrust is reacted at anchors and applied explicitly through `point_loads`
    /// ([`flow_thrust`] at the bends); never both for the same thrust.
    pub pressure_end_load: bool,
    /// Operating minus stress-free (rigging) temperature, uniform.
    pub delta_t: f64,
    pub load_factors: [f64; 3],
    /// Extra loads for this case (flow thrust, valve actuation, ...).
    pub point_loads: Vec<PointLoad>,
}

/// Material class selects the failure theory. Allowables and factors of safety are inputs.
#[derive(Debug, Clone, PartialEq)]
pub enum MaterialClass {
    /// Ductile metal: von Mises and Tresca against yield, maximum principal stress against ultimate.
    Ductile { yield_strength: f64, ultimate_strength: f64, yield_factor_of_safety: f64, ultimate_factor_of_safety: f64 },
    /// Brittle material: maximum principal stress (Rankine, tension and compression) and Coulomb-Mohr
    /// against the tension and compression ultimates (both positive numbers).
    Brittle { ultimate_tension: f64, ultimate_compression: f64, factor_of_safety: f64 },
    /// Composite laminates need ply-level criteria (Tsai-Wu, Hashin...) and layup data: refused.
    Composite,
    /// Anything else (polymers, elastomers, ...): refused with this name in the reason.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criterion {
    VonMisesYield,
    TrescaYield,
    UltimateMaxPrincipal,
    Rankine,
    CoulombMohr,
}
impl Criterion {
    pub fn label(self) -> &'static str {
        match self {
            Self::VonMisesYield => "Von Mises yield",
            Self::TrescaYield => "Tresca (maximum shear) yield",
            Self::UltimateMaxPrincipal => "Ultimate (maximum principal)",
            Self::Rankine => "Rankine (maximum normal stress)",
            Self::CoulombMohr => "Coulomb-Mohr",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Inner,
    Outer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub case: String,
    pub x: f64,
    pub surface: Surface,
    /// Circumferential angle (radians) from `+y` toward `+z`.
    pub theta: f64,
}

/// Stress components at the governing point (`x`, hoop, radial normal; `tau` = `tau_x,theta`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointStress {
    pub sigma_x: f64,
    pub hoop: f64,
    pub radial: f64,
    pub tau: f64,
}
impl PointStress {
    /// Principal stresses, descending. `radial` is principal (no shear on the surface); the
    /// `x`-`theta` pair is reduced by its Mohr circle.
    pub fn principals(&self) -> [f64; 3] {
        let c = 0.5 * (self.sigma_x + self.hoop);
        let r = (0.25 * (self.sigma_x - self.hoop).powi(2) + self.tau * self.tau).sqrt();
        let mut p = [c + r, c - r, self.radial];
        p.sort_by(|a, b| b.partial_cmp(a).expect("finite"));
        p
    }
    /// The principal triple carried in the `failure.rs` stress carrier so the shared criteria run on it.
    fn carrier(&self, radius: f64) -> StressState {
        let [a, b, c] = self.principals();
        StressState { radial: c, hoop: a, axial: b, radius }
    }
}

/// A margin of safety `1 / utilization - 1` with `utilization = factor_of_safety * demand / allowable`.
#[derive(Debug, Clone, PartialEq)]
pub struct LineMargin {
    pub criterion: Criterion,
    pub margin: f64,
    /// Equivalent stress of the criterion (for Rankine and Coulomb-Mohr: utilization times the tension ultimate).
    pub applied: f64,
    /// The allowable the criterion is compared with (tension ultimate for Rankine / Coulomb-Mohr).
    pub allowable: f64,
    pub factor_of_safety: f64,
    pub location: Location,
    pub stress: PointStress,
}

/// Member actions at a station (a node appears twice, once per side, where actions jump).
/// `bend_y` is the moment of the bending plane that deflects the line in `y` (stress `-bend_y y / I`),
/// `bend_z` that of the plane deflecting in `z`; `shear_y`, `shear_z` are their derivatives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    pub x: f64,
    pub bend_y: f64,
    pub bend_z: f64,
    pub shear_y: f64,
    pub shear_z: f64,
    /// Tension positive.
    pub axial_force: f64,
}
impl Station {
    pub fn resultant_moment(&self) -> f64 {
        self.bend_y.hypot(self.bend_z)
    }
}

/// Support reaction (force and moment on the line by the support, global axes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reaction {
    pub x: f64,
    pub force: [f64; 3],
    pub moment: [f64; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseResult {
    pub name: String,
    pub stations: Vec<Station>,
    pub reactions: Vec<Reaction>,
    pub margins: Vec<LineMargin>,
    /// Largest relative equilibrium / backward-error residual of the case's solves.
    pub residual: f64,
}
impl CaseResult {
    pub fn governing(&self) -> &LineMargin {
        self.margins.iter().min_by(|a, b| a.margin.partial_cmp(&b.margin).expect("margins are not NaN")).expect("margins are never empty")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineResult {
    pub cases: Vec<CaseResult>,
}
impl LineResult {
    /// The lowest margin over all cases and criteria.
    pub fn governing(&self) -> &LineMargin {
        self.cases.iter().map(|c| c.governing()).min_by(|a, b| a.margin.partial_cmp(&b.margin).expect("margins are not NaN")).expect("at least one case")
    }
}

/// One leg of a flow junction for [`flow_thrust`]: unit direction OF FLOW in that leg (into the
/// junction for an inlet, away from it for an outlet), gauge pressure, flow area, mass flow, velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowLeg {
    pub direction: [f64; 3],
    pub inflow: bool,
    pub gauge_pressure: f64,
    pub area: f64,
    pub mass_flow: f64,
    pub velocity: f64,
}

/// Steady-state force of the fluid on a bend, tee or other junction: the control-volume momentum
/// balance `F = sum_in (p A + m_dot v) d - sum_out (p A + m_dot v) d` (gauge pressures; atmospheric
/// pressure cancels over the closed surface). A 90 degree bend with equal legs gives
/// `(p A + m_dot v)(d_in - d_out)`. Apply the result as a [`PointLoad::force`] at the junction.
pub fn flow_thrust(legs: &[FlowLeg]) -> Result<[f64; 3], LineRunError> {
    let mut f = [0.0; 3];
    for l in legs {
        let n = l.direction.iter().map(|c| c * c).sum::<f64>().sqrt();
        let ok = [l.gauge_pressure, l.area, l.mass_flow, l.velocity].iter().all(|v| v.is_finite()) && l.area > 0.0 && l.mass_flow >= 0.0;
        if !ok || (n - 1.0).abs() > 1e-9 {
            return Err(LineRunError::InvalidInput("flow leg needs a unit direction, finite gauge pressure, area > 0, mass flow >= 0".into()));
        }
        let mag = (l.gauge_pressure * l.area + l.mass_flow * l.velocity) * if l.inflow { 1.0 } else { -1.0 };
        for k in 0..3 {
            f[k] += mag * l.direction[k];
        }
    }
    Ok(f)
}

/// Thin-wall transverse shear stress `tau_x,theta` at angle `theta` for shear force components
/// `(vy, vz)`: `(-vy sin(theta) + vz cos(theta)) / (pi r_m t)`. Integrating `tau t r_m` against
/// `(-sin, cos)` over the circumference returns `(vy, vz)` (tested); the peak `2 V / A` at the neutral
/// axis is the classical thin-tube value. Thin-wall: for thick walls it is an approximation.
pub fn wall_shear_stress(vy: f64, vz: f64, theta: f64, mean_radius: f64, wall: f64) -> f64 {
    (-vy * theta.sin() + vz * theta.cos()) / (PI * mean_radius * wall)
}

fn validate_class(class: &MaterialClass) -> Result<(), LineRunError> {
    let pos = |v: f64| v.is_finite() && v > 0.0;
    match class {
        MaterialClass::Ductile { yield_strength, ultimate_strength, yield_factor_of_safety, ultimate_factor_of_safety } => {
            if ![*yield_strength, *ultimate_strength, *yield_factor_of_safety, *ultimate_factor_of_safety].into_iter().all(pos) {
                return Err(LineRunError::InvalidInput("ductile allowables and factors of safety must be positive and finite (supply them from MMPDS / the spec; there are no defaults)".into()));
            }
            if ultimate_strength < yield_strength {
                return Err(LineRunError::InvalidInput("ultimate strength below yield strength".into()));
            }
            Ok(())
        }
        MaterialClass::Brittle { ultimate_tension, ultimate_compression, factor_of_safety } => {
            if ![*ultimate_tension, *ultimate_compression, *factor_of_safety].into_iter().all(pos) {
                return Err(LineRunError::InvalidInput("brittle ultimates (tension and compression, both positive) and factor of safety must be positive and finite".into()));
            }
            Ok(())
        }
        MaterialClass::Composite => Err(LineRunError::Refused("composite tube: needs ply-level failure criteria (e.g. Tsai-Wu / Hashin) and layup data; no criterion is guessed".into())),
        MaterialClass::Other(name) => Err(LineRunError::Refused(format!("material class '{name}' has no supported failure criterion"))),
    }
}

fn utilization_of(class: &MaterialClass, criterion: Criterion, s: &PointStress, radius: f64) -> (f64, f64) {
    // Returns (utilization including the factor of safety, equivalent applied stress).
    let st = s.carrier(radius);
    match (class, criterion) {
        (MaterialClass::Ductile { yield_strength, yield_factor_of_safety, .. }, Criterion::VonMisesYield) => {
            let v = von_mises_stress(&st);
            (yield_factor_of_safety * v / yield_strength, v)
        }
        (MaterialClass::Ductile { yield_strength, yield_factor_of_safety, .. }, Criterion::TrescaYield) => {
            let v = tresca_stress(&st);
            (yield_factor_of_safety * v / yield_strength, v)
        }
        (MaterialClass::Ductile { ultimate_strength, ultimate_factor_of_safety, .. }, Criterion::UltimateMaxPrincipal) => {
            let v = max_abs_principal_stress(&st);
            (ultimate_factor_of_safety * v / ultimate_strength, v)
        }
        (MaterialClass::Brittle { ultimate_tension: ft, ultimate_compression: fc, factor_of_safety: fs }, Criterion::Rankine) => {
            let [s1, _, s3] = s.principals();
            let u = (s1 / ft).max(-s3 / fc).max(0.0);
            (fs * u, u * ft)
        }
        (MaterialClass::Brittle { ultimate_tension: ft, ultimate_compression: fc, factor_of_safety: fs }, Criterion::CoulombMohr) => {
            // Coulomb-Mohr (Shigley): s3 >= 0: s1 = Sut; s1 >= 0 >= s3: s1/Sut - s3/Suc = 1; 0 >= s1: s3 = -Suc.
            // Linear in the stresses, so utilization u scales proportional loading: margin = 1/u - 1.
            let [s1, _, s3] = s.principals();
            let u = if s3 >= 0.0 {
                s1 / ft
            } else if s1 <= 0.0 {
                -s3 / fc
            } else {
                s1 / ft - s3 / fc
            };
            (fs * u, u * ft)
        }
        _ => unreachable!("criteria are selected from the class"),
    }
}

fn criteria_of(class: &MaterialClass) -> &'static [Criterion] {
    match class {
        MaterialClass::Ductile { .. } => &[Criterion::VonMisesYield, Criterion::TrescaYield, Criterion::UltimateMaxPrincipal],
        MaterialClass::Brittle { .. } => &[Criterion::Rankine, Criterion::CoulombMohr],
        _ => unreachable!("validated"),
    }
}

fn allowable_and_fs(class: &MaterialClass, c: Criterion) -> (f64, f64) {
    match (class, c) {
        (MaterialClass::Ductile { yield_strength, yield_factor_of_safety, .. }, Criterion::VonMisesYield | Criterion::TrescaYield) => (*yield_strength, *yield_factor_of_safety),
        (MaterialClass::Ductile { ultimate_strength, ultimate_factor_of_safety, .. }, _) => (*ultimate_strength, *ultimate_factor_of_safety),
        (MaterialClass::Brittle { ultimate_tension, factor_of_safety, .. }, _) => (*ultimate_tension, *factor_of_safety),
        _ => unreachable!("validated"),
    }
}

/// Maximizes `f` over the circumference: coarse scan, then golden-section refinement of the best bracket.
fn worst_theta(f: impl Fn(f64) -> f64) -> (f64, f64) {
    let step = 2.0 * PI / THETA_SAMPLES as f64;
    let (mut bi, mut bv) = (0, f64::NEG_INFINITY);
    for i in 0..THETA_SAMPLES {
        let v = f(i as f64 * step);
        if v > bv {
            (bi, bv) = (i, v);
        }
    }
    let (mut lo, mut hi) = ((bi as f64 - 1.0) * step, (bi as f64 + 1.0) * step);
    let g = (5.0_f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (hi - g * (hi - lo), lo + g * (hi - lo));
    let (mut fa, mut fb) = (f(a), f(b));
    for _ in 0..60 {
        if fa < fb {
            lo = a;
            (a, fa) = (b, fb);
            b = lo + g * (hi - lo);
            fb = f(b);
        } else {
            hi = b;
            (b, fb) = (a, fa);
            a = hi - g * (hi - lo);
            fa = f(a);
        }
    }
    let (t, v) = if fa > fb { (a, fa) } else { (b, fb) };
    if v > bv {
        (t, v)
    } else {
        (bi as f64 * step, bv)
    }
}

/// Total point load actions at a node: force and moment about the line axis point, global axes.
fn resolve_point(pl: &PointLoad, n: [f64; 3]) -> Result<([f64; 3], [f64; 3]), LineRunError> {
    let all = pl.force.iter().chain(&pl.moment).chain(&pl.offset).chain([&pl.weight, &pl.x]);
    if !all.into_iter().all(|v| v.is_finite()) {
        return Err(LineRunError::InvalidInput("point load has a non-finite value".into()));
    }
    let f = [pl.force[0] + pl.weight * n[0], pl.force[1] + pl.weight * n[1], pl.force[2] + pl.weight * n[2]];
    let o = pl.offset;
    let m = [pl.moment[0] + o[1] * f[2] - o[2] * f[1], pl.moment[1] + o[2] * f[0] - o[0] * f[2], pl.moment[2] + o[0] * f[1] - o[1] * f[0]];
    let scale = pl.moment[0].abs() + (o[1] * f[2]).abs() + (o[2] * f[1]).abs();
    if m[0].abs() > 1e-12 * scale.max(f64::MIN_POSITIVE) && m[0] != 0.0 {
        return Err(LineRunError::Refused(format!("point load at x = {} has a torque {} about the line axis: no torsion model (give the torsional restraint, then it can be added)", pl.x, m[0])));
    }
    Ok((f, m))
}

struct Mesh {
    x: Vec<f64>,
}

fn build_mesh(run: &LineRun, cases: &[LoadCase]) -> Result<Mesh, LineRunError> {
    let mut key = vec![run.x_start, run.x_end];
    key.extend(run.supports.iter().map(|s| s.x));
    key.extend(run.point_loads.iter().map(|p| p.x));
    for c in cases {
        key.extend(c.point_loads.iter().map(|p| p.x));
    }
    if key.iter().any(|v| !v.is_finite()) {
        return Err(LineRunError::InvalidInput("non-finite station coordinate".into()));
    }
    let len = run.x_end - run.x_start;
    let tol = 1e-9 * len;
    if key.iter().any(|&v| v < run.x_start - tol || v > run.x_end + tol) {
        return Err(LineRunError::InvalidInput("a support or load lies outside [x_start, x_end]".into()));
    }
    key.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    key.dedup_by(|a, b| (*a - *b).abs() <= tol);
    let mut x = Vec::new();
    for w in key.windows(2) {
        for i in 0..run.subdivisions {
            x.push(w[0] + (w[1] - w[0]) * i as f64 / run.subdivisions as f64);
        }
    }
    x.push(*key.last().expect("two keys"));
    if x.len() > 4000 {
        return Err(LineRunError::InvalidInput("more than 4000 stations".into()));
    }
    Ok(Mesh { x })
}

fn node_of(mesh: &Mesh, x: f64, tol: f64) -> usize {
    mesh.x.iter().position(|v| (v - x).abs() <= tol).expect("key points are mesh nodes")
}

fn evaluate_case(run: &LineRun, class: &MaterialClass, mesh: &Mesh, case: &LoadCase) -> Result<CaseResult, LineRunError> {
    let nn = mesh.x.len();
    let ne = nn - 1;
    let tol = 1e-9 * (run.x_end - run.x_start);
    let a = run.tube.area();
    let inertia = run.tube.inertia();
    let n = case.load_factors;
    if !(n.iter().all(|v| v.is_finite()) && case.delta_t.is_finite() && case.internal_pressure.is_finite() && case.external_pressure.is_finite()) {
        return Err(LineRunError::InvalidInput(format!("case '{}': non-finite load factor, temperature or pressure", case.name)));
    }
    let pressure = PressureLoading::new(case.internal_pressure, case.external_pressure, if case.pressure_end_load { EndCondition::Closed } else { EndCondition::Open })
        .map_err(|_| LineRunError::InvalidInput(format!("case '{}': pressures must be non-negative magnitudes", case.name)))?;
    let w = run.tube.weight_density * a + run.contents_weight_per_length + run.insulation_weight_per_length;
    if !w.is_finite() || run.contents_weight_per_length < 0.0 || run.insulation_weight_per_length < 0.0 {
        return Err(LineRunError::InvalidInput("weights per length must be finite and non-negative".into()));
    }
    // Nodal loads per plane. Plane y: force Fy, slope-conjugate moment Mz. Plane z: Fz, -My.
    let (mut px, mut py, mut pz) = (vec![0.0; nn], vec![0.0; nn], vec![0.0; nn]);
    let (mut mzn, mut myn) = (vec![0.0; nn], vec![0.0; nn]);
    let mut applied = Vec::new();
    for pl in run.point_loads.iter().chain(&case.point_loads) {
        let (f, m) = resolve_point(pl, n)?;
        let i = node_of(mesh, pl.x, tol);
        px[i] += f[0];
        py[i] += f[1];
        pz[i] += f[2];
        mzn[i] += m[2];
        myn[i] += m[1];
        applied.push((pl.x, f, m));
    }
    let (qx, qy, qz) = (w * n[0], w * n[1], w * n[2]);
    // Boundary conditions.
    let mut bc = vec![NodeBc { fix_w: false, fix_theta: false }; nn];
    let mut fix_u = vec![false; nn];
    for s in &run.supports {
        let i = node_of(mesh, s.x, tol);
        bc[i].fix_w = true;
        bc[i].fix_theta |= s.rotation_fixed;
        fix_u[i] |= s.axial_anchor;
    }
    let ei = run.tube.e * inertia;
    let qy_e = vec![qy; ne];
    let qz_e = vec![qz; ne];
    let unstable = |s: String| LineRunError::Unstable(format!("case '{}': {s}", case.name));
    let by = solve_beam(&mesh.x, ei, &bc, &qy_e, &py, &mzn).map_err(&unstable)?;
    let neg_myn: Vec<f64> = myn.iter().map(|v| -v).collect();
    let bz = solve_beam(&mesh.x, ei, &bc, &qz_e, &pz, &neg_myn).map_err(&unstable)?;
    let axial_loaded = qx != 0.0 || px.iter().any(|v| *v != 0.0);
    let bar = if fix_u.iter().any(|f| *f) {
        Some(solve_bar(&mesh.x, run.tube.e * a, &fix_u, run.tube.alpha * case.delta_t, &vec![qx; ne], &px).map_err(&unstable)?)
    } else if axial_loaded {
        return Err(unstable("axial loads (load factor along x, point force Fx) need at least one axial anchor".into()));
    } else {
        None // no anchor, no axial load: free thermal growth, no axial force.
    };
    // Equilibrium and backward-error acceptance.
    let mut residual = by.residual.max(bz.residual);
    if let Some(b) = &bar {
        residual = residual.max(b.residual);
    }
    let len = run.x_end - run.x_start;
    let x0 = run.x_start;
    let balance = |reaction: &[f64], reaction_m: &[f64], p: &[f64], m: &[f64], q: f64| -> f64 {
        let mut f = q * len;
        let mut mo = q * len * len / 2.0;
        let mut scale_f = (q * len).abs();
        let mut scale_m = (q * len * len / 2.0).abs();
        for i in 0..nn {
            let r = mesh.x[i] - x0;
            f += p[i] + reaction[i];
            mo += p[i] * r + reaction[i] * r + m[i] + reaction_m[i];
            scale_f += p[i].abs() + reaction[i].abs();
            scale_m += (p[i] * r).abs() + (reaction[i] * r).abs() + m[i].abs() + reaction_m[i].abs();
        }
        (f.abs() / scale_f.max(1e-300)).max(mo.abs() / scale_m.max(1e-300))
    };
    residual = residual.max(balance(&by.reaction_w, &by.reaction_m, &py, &mzn, qy));
    residual = residual.max(balance(&bz.reaction_w, &bz.reaction_m, &pz, &neg_myn, qz));
    if let Some(b) = &bar {
        let mut f = qx * len;
        let mut sc = (qx * len).abs();
        for i in 0..nn {
            f += px[i] + b.reaction[i];
            sc += px[i].abs() + b.reaction[i].abs();
        }
        residual = residual.max(f.abs() / sc.max(1e-300));
    }
    if !(residual <= BALANCE_LIMIT) {
        return Err(LineRunError::Rejected(format!("case '{}': equilibrium / solve residual {residual:.2e} exceeds {BALANCE_LIMIT:.0e}", case.name)));
    }
    // Reactions at support nodes (global axes; a clamped support's moment).
    let mut reactions: Vec<Reaction> = Vec::new();
    for s in &run.supports {
        let i = node_of(mesh, s.x, tol);
        if reactions.iter().any(|r| (r.x - mesh.x[i]).abs() <= tol) {
            continue;
        }
        let fx = bar.as_ref().map_or(0.0, |b| b.reaction[i]);
        reactions.push(Reaction { x: mesh.x[i], force: [fx, by.reaction_w[i], bz.reaction_w[i]], moment: [0.0, -bz.reaction_m[i], by.reaction_m[i]] });
    }
    // Stations: both ends of every element, plus the interior extremum of each moment.
    let mut stations = Vec::new();
    for e in 0..ne {
        let (ey, ez) = (by.elems[e], bz.elems[e]);
        let l = ey.l;
        let nl = bar.as_ref().map_or(0.0, |b| b.n_left[e]);
        let mut s_list = vec![0.0, l];
        for el in [&ey, &ez] {
            if el.q != 0.0 {
                let s = -el.v_left / el.q;
                if s > 0.0 && s < l {
                    s_list.push(s);
                }
            }
        }
        for s in s_list {
            stations.push(Station { x: mesh.x[e] + s, bend_y: ey.moment(s), bend_z: ez.moment(s), shear_y: ey.shear(s), shear_z: ez.shear(s), axial_force: nl - qx * s });
        }
    }
    // Margins.
    let g = run.tube.geometry()?;
    let inner = stress_at_inner_surface(&g, &pressure);
    let outer = stress_at_outer_surface(&g, &pressure);
    let criteria = criteria_of(class);
    let r_m = g.mean_radius();
    let mut margins: Vec<LineMargin> = Vec::new();
    for &crit in criteria {
        let (allowable, fs) = allowable_and_fs(class, crit);
        let mut best: Option<(f64, f64, Location, PointStress)> = None; // utilization, applied, loc, stress
        for st in &stations {
            for (surface, lame, r) in [(Surface::Inner, &inner, g.inner_radius), (Surface::Outer, &outer, g.outer_radius)] {
                let point = |theta: f64| PointStress {
                    sigma_x: lame.axial + st.axial_force / a - (st.bend_y * r * theta.cos() + st.bend_z * r * theta.sin()) / inertia,
                    hoop: lame.hoop,
                    radial: lame.radial,
                    tau: wall_shear_stress(st.shear_y, st.shear_z, theta, r_m, g.wall_thickness()),
                };
                let (theta, u) = worst_theta(|t| utilization_of(class, crit, &point(t), r).0);
                if best.as_ref().map_or(true, |b| u > b.0) {
                    let ps = point(theta);
                    let applied = utilization_of(class, crit, &ps, r).1;
                    best = Some((u, applied, Location { case: case.name.clone(), x: st.x, surface, theta }, ps));
                }
            }
        }
        let (u, applied, location, stress) = best.expect("at least one station");
        let margin = if u > 0.0 { 1.0 / u - 1.0 } else { f64::INFINITY };
        margins.push(LineMargin { criterion: crit, margin, applied, allowable, factor_of_safety: fs, location, stress });
    }
    Ok(CaseResult { name: case.name.clone(), stations, reactions, margins, residual })
}

/// Runs every load case and returns the member actions, reactions and margins per case.
pub fn analyze(run: &LineRun, class: &MaterialClass, cases: &[LoadCase]) -> Result<LineResult, LineRunError> {
    run.tube.validate()?;
    validate_class(class)?;
    if cases.is_empty() {
        return Err(LineRunError::InvalidInput("no load case".into()));
    }
    if !(run.x_end > run.x_start) || !run.x_start.is_finite() || !run.x_end.is_finite() {
        return Err(LineRunError::InvalidInput("x_end must exceed x_start".into()));
    }
    if run.subdivisions == 0 {
        return Err(LineRunError::InvalidInput("subdivisions must be at least 1".into()));
    }
    if run.supports.is_empty() {
        return Err(LineRunError::Unstable("no supports".into()));
    }
    let mesh = build_mesh(run, cases)?;
    let mut out = Vec::new();
    for c in cases {
        out.push(evaluate_case(run, class, &mesh, c)?);
    }
    Ok(LineResult { cases: out })
}

#[cfg(test)]
mod tests;
