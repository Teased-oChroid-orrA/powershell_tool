//! Public entry point: build the condensed lug once, then solve any number
//! of pin cases (load, direction, clearance/interference, friction) against it.

use crate::contact::{ContactModel, ContactParams, ContactState, IfacePoint, InterfaceSpec, PointResult, StepResult};
use crate::fe::{Condensed, FarEnd, FactorStats, Material};
use crate::geometry::LugGeometry;
use crate::mesh::{BushingMesh, MeshSpec, MeshStats, Refinement};
use crate::plastic::{Hardening, PlaneMode, PlasticModel, PlasticOptions};
use crate::stress::{error_estimate, geometric_energy, nodal_stresses, strain_energy, Stress};
use std::time::Instant;

/// The rigid pin. `diameter` smaller than the hole is clearance, larger is interference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinSpec {
    pub diameter: f64,
    pub friction: f64,
    /// Normal penalty as a multiple of `E / a`.
    pub penalty_factor: f64,
    /// Uniform temperature change from the fit's reference temperature (default none).
    pub thermal: Thermal,
    /// Rigid by default; an elastic pin ovalises under the bearing load and spreads it.
    pub body: PinBody,
}

/// What the pin is made of.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PinBody {
    /// Rigid analytic circle: nothing is meshed.
    #[default]
    Rigid,
    /// Elastic disc of this material (true constants; plane stress or strain follows the model).
    Elastic(Material),
}

impl PinSpec {
    pub fn new(diameter: f64, friction: f64) -> Self {
        Self { diameter, friction, penalty_factor: ContactParams::default().penalty_factor, thermal: Thermal::default(), body: PinBody::Rigid }
    }
}

/// Uniform temperature change of an unrestrained assembly (free thermal expansion of the hole,
/// the bushing and the pin). It changes only the fit: the pin's size against the surface that
/// carries it, and the bushing's interference. Thermal stress from structural restraint (the
/// far end of a real lug) is not modelled.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Thermal {
    /// Temperature change (any unit, consistent with the coefficients).
    pub delta_t: f64,
    /// Linear expansion coefficients (per unit of `delta_t`).
    pub alpha_lug: f64,
    pub alpha_bushing: f64,
    pub alpha_pin: f64,
}

impl Thermal {
    /// Diametral interference after the change: `d + dT D (alpha_bushing - alpha_lug)`.
    pub fn interference(&self, interference_dia: f64, hole_dia: f64) -> f64 {
        interference_dia + self.delta_t * hole_dia * (self.alpha_bushing - self.alpha_lug)
    }

    /// Pin diameter measured against the (nominal-size) surface that carries it: the pin grows
    /// as `alpha_pin dT` and that surface (bushing bore, or the hole) as `alpha_ref dT`.
    pub fn pin_diameter(&self, diameter: f64, has_bushing: bool) -> f64 {
        let reference = if has_bushing { self.alpha_bushing } else { self.alpha_lug };
        diameter * (1.0 + self.alpha_pin * self.delta_t) / (1.0 + reference * self.delta_t)
    }
}

/// Pin load. `angle_deg` is measured from the lug axis: 0 pulls the pin
/// away from the shank (axial tension), 90 is transverse (+y), 180 pushes
/// toward the shank (axial compression).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadCase {
    pub load_lbf: f64,
    pub angle_deg: f64,
}

impl LoadCase {
    pub fn direction(&self) -> [f64; 2] {
        let a = self.angle_deg.to_radians();
        [-a.cos(), a.sin()]
    }

    /// Load along the lug axis, which makes the problem symmetric about it.
    pub fn is_axial(&self) -> bool {
        self.angle_deg.to_radians().sin().abs() < 1e-9
    }
}

/// A bushing pressed into the lug hole. The pin then bears on the bushing's inner surface and
/// the bushing's outer surface meets the hole in a contact with the interference fit and
/// friction (so the fit pressure is solved, and can be lost).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BushingSpec {
    /// Inner diameter (the pin-contact surface).
    pub inner_dia: f64,
    pub material: Material,
    /// Diametral interference: bushing outer diameter minus hole diameter (negative = clearance).
    pub interference_dia: f64,
    /// Coulomb friction between bushing and hole.
    pub friction: f64,
}

/// Bushing and fit results.
#[derive(Debug, Clone)]
pub struct BushingResult {
    /// Interface contact points at the final state.
    pub interface: Vec<IfacePoint>,
    /// Mean fit pressure with no external load (the pin at its force-free position).
    pub fit_pressure_unloaded: f64,
    /// Mean / peak interface pressure at the applied load.
    pub interface_mean_pressure: f64,
    pub interface_peak_pressure: f64,
    /// Fraction of the interface (by length) that has lost contact at the applied load.
    pub separated_fraction: f64,
    /// Largest tensile hoop stress in the bushing (on its inner surface) and where.
    pub peak_hoop: f64,
    pub peak_hoop_angle_deg: f64,
    pub peak_von_mises: f64,
    /// `P / (inner diameter x t)`.
    pub bearing_stress: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct BoreStress {
    /// Polar angle about the hole centre, degrees `[0, 360)`.
    pub angle_deg: f64,
    pub hoop: f64,
    pub radial: f64,
    pub shear: f64,
    pub von_mises: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub build: FactorStats,
    pub contact_ms: f64,
    pub recover_ms: f64,
}

/// Self-checks of one converged solution. Every number is dimensionless and should be tiny;
/// `Verification::ok` applies the acceptance limits and the toolbox shows a note when it fails.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Verification {
    /// `|sum of contact forces - target load| / target` (0 for a zero load).
    pub force_balance: f64,
    /// `|U_strain - 1/2 f.u| / U_strain`: the discrete energy identity of the linear solve.
    pub energy_balance: f64,
    /// Deepest penetration over the contact radius (the augmented Lagrangian removes it).
    pub max_penetration: f64,
    /// Largest friction excess `(|tau| - mu p)` over the peak pressure (0 = inside the cone).
    pub friction_excess: f64,
    /// Largest tensile contact pressure over the peak pressure (always 0: pressure is clamped).
    pub tension: f64,
    /// Force the bushing's artificial grounding springs absorb, over the target load.
    pub ground_leak: f64,
    /// ZZ relative energy-norm error estimate of the stress field (discretisation error).
    pub discretisation_error: f64,
}

impl Verification {
    /// Solver consistency (not discretisation) within limits.
    pub fn ok(&self) -> bool {
        self.force_balance < 1e-3 && self.energy_balance < 1e-2 && self.max_penetration < 1e-3 && self.friction_excess < 1e-2 && self.ground_leak < 1e-3
    }
}

#[derive(Debug, Clone)]
pub struct LugSolution {
    /// Applied pin load vector (lbf), x along the lug axis toward the shank.
    pub load: [f64; 2],
    /// Pin centre relative to the hole centre (in).
    pub pin_centre: [f64; 2],
    /// Pin travel along the load direction after the gap closes (in).
    pub bearing_deflection: f64,
    /// Contact pressure / friction traction at the Gauss points.
    pub contact: Vec<PointResult>,
    pub peak_pressure: f64,
    /// Angular extent of the contact patch (degrees); 360 when the pin is held all round.
    pub contact_arc_deg: f64,
    /// Centre of the contact patch (degrees about the hole centre).
    pub contact_centre_deg: f64,
    /// Stress on the pin-contact surface (the bushing's inner surface when there is a bushing).
    pub bore: Vec<BoreStress>,
    /// Stress on the lug's own hole surface (equal to `bore` without a bushing).
    pub lug_bore: Vec<BoreStress>,
    pub bushing: Option<BushingResult>,
    /// Largest tensile hoop stress on the lug's hole and where it is.
    pub peak_hoop: f64,
    pub peak_hoop_angle_deg: f64,
    /// Largest von Mises stress anywhere in the lug (psi) and its position.
    pub peak_von_mises: f64,
    pub peak_von_mises_at: [f64; 2],
    /// `P / ((W - D) t)`.
    pub net_section_stress: f64,
    /// Bearing stress `P / (D t)` on the pin's diameter.
    pub bearing_stress: f64,
    /// `peak_hoop / net_section_stress` (0 when unloaded).
    pub kt_net: f64,
    pub mesh: MeshStats,
    pub dofs: usize,
    pub bandwidth: usize,
    pub contact_iterations: usize,
    pub timings: Timings,
    pub verification: Verification,
}

/// One converged point of the elastic-plastic load-travel curve.
#[derive(Debug, Clone, Copy)]
pub struct CurvePoint {
    /// Pin travel from its force-free position (in).
    pub travel: f64,
    pub load_lbf: f64,
    /// Fraction of Gauss points with plastic strain.
    pub plastic_fraction: f64,
    /// Largest equivalent plastic strain anywhere.
    pub max_equivalent_strain: f64,
}

/// Controls of [`LugModel::limit_load_with`].
#[derive(Debug, Clone, Copy)]
pub struct LimitOptions {
    /// Largest pin travel from its force-free position, as a multiple of the bore radius.
    pub travel_cap_over_a: f64,
    /// Stop as soon as the load has flattened.
    pub stop_on_plateau: bool,
    /// Strain-hardening law (plane strain). Without one the lug is perfectly plastic.
    pub hardening: Option<Hardening>,
    /// With hardening the load never plateaus: the ultimate capacity is the load at which the
    /// largest equivalent plastic strain anywhere reaches this value (the material's ductility).
    pub strain_limit: Option<f64>,
}

impl Default for LimitOptions {
    fn default() -> Self {
        Self { travel_cap_over_a: 0.5, stop_on_plateau: true, hardening: None, strain_limit: None }
    }
}

/// Elastic-perfectly-plastic collapse of the lug.
#[derive(Debug, Clone)]
pub struct LimitLoad {
    /// Largest load on the curve (lbf).
    pub limit_load_lbf: f64,
    /// The curve flattened (a true limit); `false` means the travel cap or a convergence
    /// failure ended it first, so the value is a lower bound.
    pub plateau: bool,
    /// The hardening run reached `strain_limit` (a defined end, like a plateau).
    pub strain_limited: bool,
    pub curve: Vec<CurvePoint>,
    pub flow_stress: f64,
    pub plastic_iterations: usize,
    pub max_plastic_strain: f64,
    pub elapsed_ms: f64,
    pub note: Option<String>,
}

pub struct LugModel {
    cond: Condensed,
    geom: LugGeometry,
    symmetric: bool,
    /// True material constants (the condensed model holds the effective ones in plane strain).
    true_material: Material,
    mode: PlaneMode,
    bushing: Option<BushingSpec>,
    /// Elastic-pin compliances already built, by (E, nu, radius) of the pin: each costs one
    /// inversion of the bore matrix.
    pin_cache: std::sync::Mutex<Vec<([u64; 3], std::sync::Arc<crate::pin::CombinedCompliance>)>>,
}

impl LugModel {
    /// `symmetric` meshes half the lug (valid only for axial loads); otherwise the full lug.
    pub fn build(geom: &LugGeometry, material: Material, mesh: MeshSpec, symmetric: bool) -> Result<Self, String> {
        Self::build_with(geom, material, mesh, symmetric, PlaneMode::Stress)
    }

    /// As [`build`](Self::build) for the given plane idealisation. `PlaneMode::Strain` models
    /// the confined bearing zone and is meant for [`limit_load`](Self::limit_load); its
    /// elastic `solve` results are plane-strain, not the thin-plate stresses.
    pub fn build_with(geom: &LugGeometry, material: Material, mesh: MeshSpec, symmetric: bool, mode: PlaneMode) -> Result<Self, String> {
        Self::build_bushed(geom, material, mesh, symmetric, mode, None)
    }

    /// As [`build_with`](Self::build_with), optionally with a bushing in the hole.
    pub fn build_bushed(geom: &LugGeometry, material: Material, mesh: MeshSpec, symmetric: bool, mode: PlaneMode, bushing: Option<BushingSpec>) -> Result<Self, String> {
        let (e, nu) = mode.effective(material.e_psi, material.nu);
        let bush = match bushing {
            Some(b) => {
                let bm = bushing_mesh(geom, mesh, &b)?;
                let (be, bn) = mode.effective(b.material.e_psi, b.material.nu);
                Some((bm, Material { e_psi: be, nu: bn }))
            }
            None => None,
        };
        let cond = Condensed::build_with(geom, mesh, Material { e_psi: e, nu }, symmetric, FarEnd::Clamped, bush)?;
        Ok(Self { cond, geom: *geom, symmetric, true_material: material, mode, bushing, pin_cache: std::sync::Mutex::new(Vec::new()) })
    }

    pub fn bushing(&self) -> Option<&BushingSpec> {
        self.bushing.as_ref()
    }

    /// Radius of the surface the pin bears on.
    pub fn contact_radius(&self) -> f64 {
        self.bushing.map_or(self.geom.bore_radius(), |b| b.inner_dia / 2.0)
    }

    fn contact_model(&self, pin: PinSpec, dir: [f64; 2]) -> Result<ContactModel<'_>, String> {
        let params = ContactParams { penalty_factor: pin.penalty_factor, friction: pin.friction };
        let th = pin.thermal;
        let iface = self.bushing.map(|b| InterfaceSpec { interference: th.interference(b.interference_dia, self.geom.hole_dia) / 2.0, friction: b.friction });
        let radius = th.pin_diameter(pin.diameter, self.bushing.is_some()) / 2.0;
        let cm = ContactModel::with_interface(&self.cond, radius, params, dir, iface);
        match pin.body {
            PinBody::Rigid => Ok(cm),
            PinBody::Elastic(material) => Ok(cm.with_pin_compliance(self.pin_compliance(material, radius)?)),
        }
    }

    /// Bore compliance with an elastic pin of `material` and `radius` (cached).
    fn pin_compliance(&self, material: Material, radius: f64) -> Result<std::sync::Arc<crate::pin::CombinedCompliance>, String> {
        if !(material.e_psi.is_finite() && material.e_psi > 0.0 && (0.0..0.5).contains(&material.nu)) {
            return Err("the pin material needs a positive modulus and 0 <= nu < 0.5".into());
        }
        let key = [material.e_psi.to_bits(), material.nu.to_bits(), radius.to_bits()];
        let mut cache = self.pin_cache.lock().map_err(|_| "pin cache poisoned".to_string())?;
        if let Some((_, c)) = cache.iter().find(|(k, _)| *k == key) {
            return Ok(c.clone());
        }
        let (e, nu) = self.mode.effective(material.e_psi, material.nu);
        let disc = crate::pin::PinDisc::build(&self.cond.mesh.theta, self.cond.mesh.periodic, radius, Material { e_psi: e, nu }, 6)?;
        let combined = std::sync::Arc::new(disc.combine(&self.cond)?);
        if cache.len() >= 4 {
            cache.remove(0);
        }
        cache.push((key, combined.clone()));
        Ok(combined)
    }

    /// Beyond this `x` the clamped far end disturbs the field (its corners are singular): peak
    /// stresses and the error estimate ignore it. Half a width from the clamp.
    fn clamp_zone_x(&self) -> f64 {
        (self.geom.length - 0.5 * self.geom.width).max(0.6 * self.geom.length)
    }

    /// The error estimate only looks at the region around the pin (one and a half widths along
    /// the shank), where the answers are read; the far shank is a bending field the clamp shapes.
    fn evaluation_zone_x(&self) -> f64 {
        (1.5 * self.geom.width).min(self.clamp_zone_x())
    }

    pub fn plane_mode(&self) -> PlaneMode {
        self.mode
    }

    /// Build the cheapest model that is valid for `case`.
    pub fn build_for(geom: &LugGeometry, material: Material, mesh: MeshSpec, case: LoadCase) -> Result<Self, String> {
        Self::build(geom, material, mesh, case.is_axial())
    }

    /// As [`build_for`](Self::build_for), also refining the bore mesh around the
    /// loaded sector when the pin's clearance makes the contact patch narrow
    /// (see [`auto_refinement`]). A refinement already in `mesh` is kept.
    pub fn build_for_pin(geom: &LugGeometry, material: Material, mut mesh: MeshSpec, pin: PinSpec, case: LoadCase) -> Result<Self, String> {
        if mesh.refine.is_none() {
            mesh.refine = auto_refinement(geom, material, pin, case, mesh.elements_around);
        }
        Self::build(geom, material, mesh, case.is_axial())
    }

    pub fn is_symmetric(&self) -> bool {
        self.symmetric
    }

    pub fn geometry(&self) -> &LugGeometry {
        &self.geom
    }

    pub fn stats(&self) -> (MeshStats, FactorStats) {
        (self.cond.mesh.stats, self.cond.stats)
    }

    pub fn solve(&self, pin: PinSpec, case: LoadCase) -> Result<LugSolution, String> {
        self.solve_field(pin, case, None).map(|(s, _)| s)
    }

    /// Second-order (P-delta) elastic solution: the geometric stiffness of the converged stress
    /// is added to the tangent and the contact solved again, to a fixed point. Tension in the
    /// lug stiffens it and compression softens it, which matters for slender or highly stressed
    /// lugs at oblique or transverse loads; the elastic buckling load is a limit (error). Small
    /// strains and moderate rotations; the mesh itself is not moved. Each pass costs one
    /// factorisation and condensation of the lug (the model build time).
    pub fn solve_second_order(&self, pin: PinSpec, case: LoadCase) -> Result<LugSolution, String> {
        let (mut sol, mut u) = self.solve_field(pin, case, None)?;
        for _ in 0..6 {
            let gs = crate::stress::gauss_stresses(&self.cond, &u);
            let sigma: Vec<[[f64; 3]; 9]> = gs.iter().map(|e| std::array::from_fn(|k| [e[k].stress.sx, e[k].stress.sy, e[k].stress.txy])).collect();
            let cond = Condensed::from_mesh_geo(self.cond.mesh.clone(), &self.geom, self.cond.material, self.cond.bushing_material, FarEnd::Clamped, Some(&sigma))
                .map_err(|e| format!("second-order analysis failed ({e}): the lug may be at its elastic buckling load"))?;
            let tangent = LugModel { cond, geom: self.geom, symmetric: self.symmetric, true_material: self.true_material, mode: self.mode, bushing: self.bushing, pin_cache: std::sync::Mutex::new(Vec::new()) };
            let (next, un) = tangent.solve_field(pin, case, Some(&sigma))?;
            let change = (next.peak_hoop - sol.peak_hoop).abs() / sol.peak_hoop.abs().max(1e-12);
            sol = next;
            u = un;
            if change < 2e-3 {
                break;
            }
        }
        Ok(sol)
    }

    /// [`solve`](Self::solve) also returning the displacement field of the lug (`2 * node + comp`).
    fn solve_field(&self, pin: PinSpec, case: LoadCase, geo: Option<&[[[f64; 3]; 9]]>) -> Result<(LugSolution, Vec<f64>), String> {
        if !(pin.diameter.is_finite() && pin.diameter > 0.0) {
            return Err("pin diameter must be positive".into());
        }
        if !(case.load_lbf.is_finite() && case.load_lbf >= 0.0) {
            return Err("load must be zero or positive".into());
        }
        if self.symmetric && !case.is_axial() {
            return Err("a symmetric (half) model only supports axial loads".into());
        }
        if pin.diameter / 2.0 > 1.2 * self.contact_radius() {
            return Err("pin is far larger than the hole".into());
        }
        let dir = case.direction();
        let t = self.geom.thickness;
        let pt = case.load_lbf / t;

        let t0 = Instant::now();
        let cm = self.contact_model(pin, dir)?;
        let mut st0 = cm.fresh_state();
        let r0 = cm.advance(&mut st0, 0.0)?;
        // Settle the pin at its force-free position (an interference fit leaves a net
        // force at the centred position), then ramp to the load.
        let (mut state, mut result, s_eq) = self.solve_for_load(&cm, st0, r0, 0.0, dir)?;
        let fit_unloaded = mean_pressure(&result.iface);
        let mut s_final = s_eq;
        if pt > 0.0 {
            let (st, res, s) = self.solve_for_load(&cm, state, result, pt, dir)?;
            state = st;
            result = res;
            s_final = s;
        }
        let contact_ms = t0.elapsed().as_secs_f64() * 1e3;

        let t1 = Instant::now();
        let forces = cm.bore_forces(&state);
        let u = self.cond.displacements(&forces);
        let stresses = nodal_stresses(&self.cond, &u);
        let verification = {
            let target = if self.symmetric { pt / 2.0 } else { pt };
            let total = [result.force[0], result.force[1]];
            let along = total[0] * dir[0] + total[1] * dir[1];
            // The half model carries only its own share of the axial load; the cross component is the
            // mirror image's and is not balanced by the half alone.
            let across = if self.symmetric { 0.0 } else { (total[0] * dir[1] - total[1] * dir[0]).abs() };
            let force_balance = if target > 0.0 { ((along - target).abs().max(across)) / target } else { 0.0 };
            let work: f64 = forces.iter().map(|&(n, f)| 0.5 * (f[0] * u[2 * n] + f[1] * u[2 * n + 1])).sum();
            let (ground_e, ground_f) = self.cond.ground_leak(&u);
            let energy = strain_energy(&self.cond, &u) + ground_e + geo.map_or(0.0, |sg| geometric_energy(&self.cond, &u, sg));
            let energy_balance = if energy > 0.0 { (energy - work).abs() / energy } else { 0.0 };
            let peak_p = result.points.iter().fold(0.0f64, |m, p| m.max(p.pressure)).max(1e-300);
            let radius = self.contact_radius();
            let max_penetration = result.points.iter().map(|p| (-p.gap).max(0.0)).chain(result.iface.iter().map(|p| (-p.gap).max(0.0))).fold(0.0f64, f64::max) / radius;
            let mu_i = self.bushing.map_or(0.0, |b| b.friction);
            let ex = result.points.iter().map(|p| (p.shear.abs() - pin.friction * p.pressure) / peak_p).fold(0.0f64, f64::max);
            let ei = result.iface.iter().map(|p| (p.shear.abs() - mu_i * p.pressure) / peak_p).fold(0.0f64, f64::max);
            let tension = result.points.iter().map(|p| (-p.pressure).max(0.0) / peak_p).fold(0.0f64, f64::max);
            Verification { force_balance, energy_balance, max_penetration, friction_excess: ex.max(ei), tension, ground_leak: if target > 0.0 { ground_f[0].hypot(ground_f[1]) / target } else { 0.0 }, discretisation_error: error_estimate(&self.cond, &u, self.evaluation_zone_x()).relative }
        };
        let mesh = &self.cond.mesh;

        let row_stress = |row: usize| -> Vec<BoreStress> {
            (0..mesh.n_cols)
                .map(|c| {
                    let th = mesh.theta[c];
                    let s = stresses[mesh.node_id(c, row)];
                    let (hoop, radial, shear) = s.polar(th);
                    BoreStress { angle_deg: th.to_degrees().rem_euclid(360.0), hoop, radial, shear, von_mises: s.von_mises() }
                })
                .collect()
        };
        let bore = row_stress(0);
        let lug_bore = if mesh.bush_rows > 0 { row_stress(mesh.bush_rows) } else { bore.clone() };
        let peak_of = |v: &[BoreStress]| v.iter().fold((f64::MIN, 0.0), |(h, a), b| if b.hoop > h { (b.hoop, b.angle_deg) } else { (h, a) });
        let (peak_hoop, peak_hoop_angle) = peak_of(&lug_bore);
        // Bushing nodes are those of group-1 elements; the lug peak ignores them.
        let mut in_bushing = vec![false; mesh.nodes.len()];
        for (ei, e) in mesh.elems.iter().enumerate() {
            if mesh.elem_group[ei] == 1 {
                for &n in e {
                    in_bushing[n] = true;
                }
            }
        }
        let (mut peak_vm, mut peak_at, mut bush_vm) = (0.0f64, [0.0, 0.0], 0.0f64);
        let clamp_x = self.clamp_zone_x();
        for (id, s) in stresses.iter().enumerate() {
            let vm = s.von_mises();
            if in_bushing[id] {
                bush_vm = bush_vm.max(vm);
            } else if mesh.nodes[id][0] > clamp_x {
                // The clamped far end (a corner singularity at the support) is a model boundary.
                continue;
            } else if vm > peak_vm {
                peak_vm = vm;
                peak_at = mesh.nodes[id];
            }
        }
        let bushing_result = self.bushing.map(|b| {
            let (bh, ba) = peak_of(&bore);
            let total: f64 = result.iface.iter().map(|p| p.w).sum::<f64>().max(1e-300);
            let separated: f64 = result.iface.iter().filter(|p| p.pressure <= 1e-6 * result.iface.iter().fold(0.0f64, |m, q| m.max(q.pressure)).max(1e-300)).map(|p| p.w).sum();
            BushingResult {
                interface: result.iface.clone(),
                fit_pressure_unloaded: fit_unloaded,
                interface_mean_pressure: mean_pressure(&result.iface),
                interface_peak_pressure: result.iface.iter().fold(0.0f64, |m, p| m.max(p.pressure)),
                separated_fraction: separated / total,
                peak_hoop: bh,
                peak_hoop_angle_deg: ba,
                peak_von_mises: bush_vm,
                bearing_stress: case.load_lbf / (b.inner_dia * t),
            }
        });
        let recover_ms = t1.elapsed().as_secs_f64() * 1e3;

        // Contact patch extent about the load direction.
        let dir_deg = dir[1].atan2(dir[0]).to_degrees();
        let peak_pressure = result.points.iter().fold(0.0f64, |m, p| m.max(p.pressure));
        let (mut lo, mut hi, mut any) = (f64::MAX, f64::MIN, false);
        let mut all_round = true;
        for p in &result.points {
            if p.pressure > 1e-3 * peak_pressure && peak_pressure > 0.0 {
                let mut d = (p.angle_deg - dir_deg).rem_euclid(360.0);
                if d > 180.0 {
                    d -= 360.0;
                }
                if self.symmetric {
                    // The half model sees one side of the axis; the patch is symmetric about it.
                    d = d.abs();
                    lo = lo.min(-d);
                }
                lo = lo.min(d);
                hi = hi.max(d);
                any = true;
            } else {
                all_round = false;
            }
        }
        let (contact_arc_deg, contact_centre_deg) = if !any {
            (0.0, dir_deg)
        } else if all_round {
            (360.0, dir_deg)
        } else {
            (hi - lo, dir_deg + 0.5 * (hi + lo))
        };

        let net_section_stress = if self.geom.width > self.geom.hole_dia { case.load_lbf / ((self.geom.width - self.geom.hole_dia) * t) } else { 0.0 };
        let kt_net = if net_section_stress > 0.0 { peak_hoop / net_section_stress } else { 0.0 };
        Ok((LugSolution {
            load: [case.load_lbf * dir[0], case.load_lbf * dir[1]],
            pin_centre: state.centre,
            bearing_deflection: (s_final - s_eq).max(0.0),
            contact: result.points.clone(),
            peak_pressure,
            contact_arc_deg,
            contact_centre_deg,
            bore,
            lug_bore,
            bushing: bushing_result,
            peak_hoop,
            peak_hoop_angle_deg: peak_hoop_angle,
            peak_von_mises: peak_vm,
            peak_von_mises_at: peak_at,
            net_section_stress,
            bearing_stress: case.load_lbf / (self.geom.hole_dia * t),
            kt_net,
            mesh: mesh.stats,
            dofs: self.cond.stats.dofs,
            bandwidth: self.cond.stats.bandwidth,
            contact_iterations: result.iterations,
            timings: Timings { build: self.cond.stats, contact_ms, recover_ms },
            verification,
        }, u))
    }

    /// Elastic solutions for several load directions at one load, in parallel. The condensed
    /// model (factor, `G`, `S`) is shared read-only, so each angle costs only its contact Newton.
    /// Needs the full model unless every angle is axial.
    pub fn solve_sweep(&self, pin: PinSpec, load_lbf: f64, angles_deg: &[f64]) -> Vec<Result<LugSolution, String>> {
        parallel_map(angles_deg, |&angle_deg| self.solve(pin, LoadCase { load_lbf, angle_deg }))
    }

    /// Plastic collapse load for several load directions, in parallel (the capacity envelope).
    /// `self` must be the plane-strain model (see `limit_load`).
    pub fn collapse_sweep(&self, pin: PinSpec, flow_stress: f64, angles_deg: &[f64]) -> Vec<Result<LimitLoad, String>> {
        parallel_map(angles_deg, |&angle_deg| self.limit_load(pin, LoadCase { load_lbf: 1000.0, angle_deg }, flow_stress))
    }

    /// Collapse load of the elastic-perfectly-plastic lug (von Mises, plane stress) at
    /// `flow_stress`: the pin is settled at its force-free position, then driven along the
    /// load direction with converged plasticity at every step until the load plateaus.
    /// Small-displacement kinematics; the travel is capped at half the bore radius.
    pub fn limit_load(&self, pin: PinSpec, case: LoadCase, flow_stress: f64) -> Result<LimitLoad, String> {
        self.limit_load_with(pin, case, flow_stress, LimitOptions::default())
    }

    pub fn limit_load_with(&self, pin: PinSpec, case: LoadCase, flow_stress: f64, options: LimitOptions) -> Result<LimitLoad, String> {
        if !(flow_stress.is_finite() && flow_stress > 0.0) {
            return Err("the flow stress must be positive".into());
        }
        if self.symmetric && !case.is_axial() {
            return Err("a symmetric (half) model only supports axial loads".into());
        }
        let t0 = Instant::now();
        let a = self.contact_radius();
        let dir = case.direction();
        let t = self.geom.thickness;
        let share = if self.symmetric { 2.0 } else { 1.0 };
        let cm = self.contact_model(pin, dir)?;
        let mut st = cm.fresh_state();
        let r0 = cm.advance(&mut st, 0.0)?;
        let (mut st, _, s_eq) = self.solve_for_load(&cm, st, r0, 0.0, dir)?;
        let pm = PlasticModel::new(&self.cond, self.mode, self.true_material.e_psi, self.true_material.nu)?;
        let mut ps = pm.fresh_state();
        let opts = PlasticOptions { hardening: options.hardening, ..PlasticOptions::new(flow_stress) };
        let mut strain_limited = false;
        let load_of = |f: [f64; 2]| share * (f[0] * dir[0] + f[1] * dir[1]) * t;

        let mut curve: Vec<CurvePoint> = Vec::new();
        let mut iterations = 0usize;
        let mut max_ep = 0.0f64;
        let mut note = None;
        let mut plateau = false;
        // Fit stresses can already yield an interference-fitted bore.
        if let Ok(eq) = pm.equilibrate(&cm, &mut st, &mut ps, s_eq, opts) {
            iterations += eq.iterations;
            max_ep = max_ep.max(eq.max_plastic_strain);
        }
        let s_cap = s_eq + options.travel_cap_over_a * a;
        let mut step = 0.01 * a;
        let mut s = s_eq;
        let mut retries = 0;
        while s < s_cap {
            let s_try = (s + step).min(s_cap);
            // Snapshot: a step that fails, or whose load collapses implausibly (a spurious
            // zero-stress fixed point of the plastic iteration), is redone with half the step.
            let (st_before, ps_before) = (st.clone(), ps.clone());
            let last_load = curve.last().map_or(0.0, |c| c.load_lbf);
            let outcome = pm.equilibrate(&cm, &mut st, &mut ps, s_try, opts).and_then(|eq| {
                let p = load_of(eq.force);
                if last_load > 0.0 && p < 0.6 * last_load {
                    Err(format!("the load fell from {last_load:.0} to {p:.0} lbf in one step (a non-physical state)"))
                } else {
                    Ok(eq)
                }
            });
            let eq = match outcome {
                Ok(eq) => eq,
                Err(e) => {
                    st = st_before;
                    ps = ps_before;
                    retries += 1;
                    step *= 0.5;
                    if retries > 6 || step < 1e-4 * a {
                        note = Some(format!("stopped at {:.4} in of pin travel: {e}", s - s_eq));
                        break;
                    }
                    continue;
                }
            };
            s = s_try;
            iterations += eq.iterations;
            max_ep = max_ep.max(eq.max_plastic_strain);
            let point = CurvePoint { travel: s - s_eq, load_lbf: load_of(eq.force), plastic_fraction: eq.plastic_fraction, max_equivalent_strain: eq.max_equivalent_strain };
            if let (Some(limit), Some(prev)) = (options.strain_limit, curve.last().copied().or(Some(CurvePoint { travel: 0.0, load_lbf: 0.0, plastic_fraction: 0.0, max_equivalent_strain: 0.0 }))) {
                if point.max_equivalent_strain >= limit {
                    // Load at the ductility limit, interpolated on the strain between the last two points.
                    let f = ((limit - prev.max_equivalent_strain) / (point.max_equivalent_strain - prev.max_equivalent_strain).max(1e-300)).clamp(0.0, 1.0);
                    curve.push(CurvePoint { travel: prev.travel + f * (point.travel - prev.travel), load_lbf: prev.load_lbf + f * (point.load_lbf - prev.load_lbf), plastic_fraction: point.plastic_fraction, max_equivalent_strain: limit });
                    strain_limited = true;
                    plateau = true;
                    break;
                }
            }
            curve.push(point);
            step = (step * 1.3).min(0.06 * a);
            let n = curve.len();
            if n >= 4 && options.stop_on_plateau && options.hardening.is_none() {
                let (p1, p2, p3) = (curve[n - 1].load_lbf, curve[n - 2].load_lbf, curve[n - 3].load_lbf);
                let top = p1.max(p2).max(p3).max(1e-12);
                // Flat for two steps in a row (or falling): the collapse load is reached.
                // Flat for two steps in a row, or the last two steps added under 1.2 % in all
                // (a plastic hinge forming slowly), or falling: the collapse load is reached.
                if ((p1 - p2) < 0.005 * top && (p2 - p3) < 0.01 * top) || (p1 - p3) < 0.012 * top {
                    plateau = true;
                    break;
                }
            }
        }
        let limit = curve.iter().map(|c| c.load_lbf).fold(0.0, f64::max);
        if limit <= 0.0 {
            return Err(note.unwrap_or_else(|| "no load was carried".into()));
        }
        if !plateau && note.is_none() {
            note = Some("the pin travel cap (half the bore radius) ended the run before the load flattened: a lower bound".into());
        }
        Ok(LimitLoad { limit_load_lbf: limit, plateau, strain_limited, curve, flow_stress, plastic_iterations: iterations, max_plastic_strain: max_ep, elapsed_ms: t0.elapsed().as_secs_f64() * 1e3, note })
    }

    /// Move the pin along `dir` until the load it carries is `pt` (per unit
    /// thickness): a bracketing (Illinois) root solve on the pin travel, which
    /// is monotone in the load. Each trial restarts from the nearest solved
    /// state so friction history stays consistent. Works both ways, so it can
    /// also settle an interference fit that leaves a net force at the start.
    fn solve_for_load(&self, cm: &ContactModel, st0: ContactState, r0: StepResult, pt: f64, dir: [f64; 2]) -> Result<(ContactState, StepResult, f64), String> {
        // A half model carries half the pin load.
        let share = if self.symmetric { 2.0 } else { 1.0 };
        let dot = |f: [f64; 2]| share * (f[0] * dir[0] + f[1] * dir[1]);
        let e = self.cond.material.e_psi;
        let a = self.contact_radius();
        let tol = (1e-4 * pt.abs()).max(1e-9 * e * a);
        let k0 = 4.0 * e; // typical pin bearing stiffness per unit thickness

        struct Known {
            s: f64,
            p: f64,
            state: ContactState,
            result: StepResult,
        }
        let p0 = dot(r0.force);
        let s_start = st0.centre[0] * dir[0] + st0.centre[1] * dir[1];
        let mut known = vec![Known { s: s_start, p: p0, state: st0, result: r0 }];
        if (p0 - pt).abs() <= tol {
            let k = known.pop().unwrap();
            return Ok((k.state, k.result, k.s));
        }
        let clearance = (a - cm.pin_radius).max(0.0);
        // One pin step never exceeds a tenth of the bore radius: past that the
        // small-displacement model is no longer meaningful (and contact can be lost).
        let max_step = 0.1 * a;
        let mut last_step: f64;
        let mut s_try = if p0 < pt { s_start + ((pt - p0) / k0).min(max_step) + if p0 == 0.0 { clearance } else { 0.0 } } else { s_start - ((p0 - pt) / k0).min(max_step) };
        for _ in 0..80 {
            // Nearest solved state to start from.
            let from = known.iter().enumerate().min_by(|x, y| (x.1.s - s_try).abs().total_cmp(&(y.1.s - s_try).abs())).map(|(i, _)| i).unwrap();
            let mut trial = known[from].state.clone();
            // Predictor: between two solved states carry the bore displacements and multipliers
            // along their secant so Newton starts close. Interpolation only: an extrapolated start
            // can converge to a spurious no-contact state (found at 45 degrees).
            let predicted = known.iter().enumerate().filter(|(i, k)| *i != from && (k.s - known[from].s).abs() > 1e-12 && (k.s - s_try) * (known[from].s - s_try) < 0.0).min_by(|x, y| (x.1.s - s_try).abs().total_cmp(&(y.1.s - s_try).abs())).map(|(i, _)| i);
            if let Some(second) = predicted {
                let (ka, kb) = (&known[from], &known[second]);
                let ratio = ((s_try - ka.s) / (kb.s - ka.s)).clamp(0.0, 1.0);
                for (u, (ua, ub)) in trial.u.iter_mut().zip(ka.state.u.iter().zip(&kb.state.u)) {
                    *u = ua + ratio * (ub - ua);
                }
                for (l, (la, lb)) in trial.lambda.iter_mut().zip(ka.state.lambda.iter().zip(&kb.state.lambda)) {
                    *l = (la + ratio * (lb - la)).max(0.0);
                }
            }
            // The load must not fall as the pin goes further in (it can only do so by landing
            // on a spurious state); if it does, redo the step cold from the nearest state.
            let floor = known.iter().filter(|k| k.s < s_try).map(|k| k.p).fold(f64::NEG_INFINITY, f64::max);
            let plausible = |p: f64| !(s_try > s_start && floor.is_finite() && floor > 0.0 && p < 0.9 * floor);
            // Far from the target a single multiplier pass is enough to place the next trial; the
            // full augmented-Lagrangian solve is only paid for near it.
            let coarse = known.iter().all(|k| (k.p - pt).abs() > 0.05 * pt.abs().max(1e-12)) && pt != 0.0;
            let step = |state: &mut ContactState| if coarse { cm.advance_coarse(state, s_try) } else { cm.advance(state, s_try) };
            let mut res = step(&mut trial);
            if predicted.is_some() && res.as_ref().map_or(true, |r| !plausible(dot(r.force))) {
                trial = known[from].state.clone();
                res = step(&mut trial);
            }
            let res = res?;
            let mut p = dot(res.force);
            let mut res = res;
            if coarse && (p - pt).abs() <= 0.05 * pt.abs() {
                // Close enough: converge the multipliers here and measure again.
                res = cm.advance(&mut trial, s_try)?;
                p = dot(res.force);
            }
            if (p - pt).abs() <= tol {
                return Ok((trial, res, s_try));
            }
            last_step = (s_try - known[from].s).abs();
            known.push(Known { s: s_try, p, state: trial, result: res });
            // Bracket: largest P below the target, smallest P above it.
            let lo = known.iter().filter(|k| k.p < pt).max_by(|x, y| x.p.total_cmp(&y.p));
            let hi = known.iter().filter(|k| k.p > pt).min_by(|x, y| x.p.total_cmp(&y.p));
            s_try = match (lo, hi) {
                (Some(l), Some(h)) if h.p > l.p => {
                    let s = l.s + (pt - l.p) * (h.s - l.s) / (h.p - l.p);
                    let (a, b) = (l.s.min(h.s), l.s.max(h.s));
                    // Stay strictly inside the bracket; fall back to bisection when the
                    // interpolation stalls against an end (flat clearance region).
                    if s <= a + 1e-3 * (b - a) || s >= b - 1e-3 * (b - a) {
                        0.5 * (l.s + h.s)
                    } else {
                        s
                    }
                }
                (Some(l), None) => {
                    // Not bracketed yet: the slope estimate was too stiff, so never
                    // step less than last time (a soft Hertz start stiffens as it loads).
                    let (s2, p2) = known.iter().filter(|k| k.p < l.p && k.p > 0.0 && k.s != l.s).max_by(|x, y| x.p.total_cmp(&y.p)).map_or((l.s - 1.0, l.p - k0), |k| (k.s, k.p));
                    let slope = ((l.p - p2) / (l.s - s2)).max(1e-6 * k0);
                    // Early in the ramp the slope is still soft Hertz and understates the stiffness, so never
                    // step less than last time; closer to the target a stiffening contact would overshoot.
                    let floor_step = if l.p < 0.25 * pt { last_step } else { 0.0 };
                    let step = (1.05 * (pt - l.p) / slope).max(floor_step).max((pt - l.p) / k0).min(max_step);
                    l.s + step
                }
                (None, Some(h)) => {
                    let (s2, p2) = known.iter().filter(|k| k.p > h.p && k.s != h.s).min_by(|x, y| x.p.total_cmp(&y.p)).map_or((h.s + 1.0, h.p + k0), |k| (k.s, k.p));
                    let slope = ((p2 - h.p) / (s2 - h.s)).max(1e-6 * k0);
                    let step = (1.05 * (h.p - pt) / slope).max(last_step).max((h.p - pt) / k0).min(max_step);
                    h.s - step
                }
                _ => s_try + 1e-4,
            };
        }
        Err("the pin load could not be reached (contact did not stiffen)".into())
    }
}

/// The bushing's mesh stack (inner radius and element layers through the wall) for a lug hole of
/// `geom`, validating the spec.
pub(crate) fn bushing_mesh(geom: &LugGeometry, mesh: MeshSpec, b: &BushingSpec) -> Result<BushingMesh, String> {
    if !(b.inner_dia.is_finite() && b.inner_dia > 0.0 && b.inner_dia < geom.hole_dia * (1.0 - 1e-3)) {
        return Err("the bushing's inner diameter must be positive and smaller than the hole".into());
    }
    let material_ok = b.material.e_psi > 0.0 && b.material.nu > 0.0 && b.material.nu < 0.5;
    if !material_ok || !b.interference_dia.is_finite() || !(0.0..=2.0).contains(&b.friction) {
        return Err("the bushing needs a valid material, interference and friction".into());
    }
    let wall = geom.bore_radius() - b.inner_dia / 2.0;
    let tangential = (b.inner_dia / 2.0) * 2.0 * std::f64::consts::PI / mesh.elements_around as f64;
    let layers = ((wall / tangential).round() as usize).clamp(2, 6);
    Ok(BushingMesh { inner_radius: b.inner_dia / 2.0, layers })
}

/// Length-weighted mean pressure over interface points (0 for none).
fn mean_pressure(points: &[IfacePoint]) -> f64 {
    let w: f64 = points.iter().map(|p| p.w).sum();
    if w > 0.0 {
        points.iter().map(|p| p.pressure * p.w).sum::<f64>() / w
    } else {
        0.0
    }
}

/// Convenience: build the cheapest valid model (refined for the pin) and solve one case.
pub fn solve_once(geom: &LugGeometry, material: Material, mesh: MeshSpec, pin: PinSpec, case: LoadCase) -> Result<LugSolution, String> {
    LugModel::build_for_pin(geom, material, mesh, pin, case)?.solve(pin, case)
}

/// Angular refinement around the loaded sector for a clearance pin.
///
/// A loose pin touches a narrow patch whose half-width follows 2D Hertz
/// contact: `b = sqrt(4 P' R* / (pi E))` with `1/R* = 1/r_pin - 1/a` and
/// `P'` the load per unit thickness. Resolving it takes about eight elements
/// across the half patch; refining only the loaded sector keeps that cheap.
/// A tight or interference fit loads a wide arc that the coarse mesh already
/// resolves, so `None` is returned.
pub fn auto_refinement(geom: &LugGeometry, material: Material, pin: PinSpec, case: LoadCase, elements_around: usize) -> Option<Refinement> {
    auto_refinement_for(geom, material, pin, case, elements_around, geom.bore_radius())
}

/// [`auto_refinement`] for a pin bearing on a surface of radius `contact_radius` (a bushing's
/// inner surface, smaller than the hole).
pub fn auto_refinement_for(geom: &LugGeometry, material: Material, pin: PinSpec, case: LoadCase, elements_around: usize, contact_radius: f64) -> Option<Refinement> {
    let a = contact_radius;
    let rp = pin.diameter / 2.0;
    if !(rp > 0.0 && rp < a && case.load_lbf > 0.0) {
        return None;
    }
    let r_star = 1.0 / (1.0 / rp - 1.0 / a);
    let b = (4.0 * (case.load_lbf / geom.thickness) * r_star / (std::f64::consts::PI * material.e_psi)).sqrt();
    let half = (b / a).min(std::f64::consts::FRAC_PI_2);
    let coarse = 2.0 * std::f64::consts::PI / elements_around as f64;
    // A patch wider than about 40 degrees is resolved by the coarse mesh.
    if half > 0.7 {
        return None;
    }
    // Quantised (fine step to 0.25 degree, zone width up to 5 degrees) so that small load
    // or clearance edits keep the same mesh and the condensed model can be reused.
    let q = |v: f64, step: f64, up: bool| (if up { (v / step).ceil() } else { (v / step).round() }) * step;
    let fine = q((half / 8.0).to_degrees().max(0.25), 0.25, false).to_radians();
    if fine >= coarse {
        return None;
    }
    let width = q((1.5 * half).to_degrees() + 2.0 * fine.to_degrees(), 5.0, true).to_radians();
    let dir = case.direction();
    Some(Refinement { centre: dir[1].atan2(dir[0]).rem_euclid(2.0 * std::f64::consts::PI), half_width: width.min(std::f64::consts::PI), fine_step: fine })
}

/// Von Mises of an arbitrary stress, re-exported for callers that post-process.
pub fn von_mises(s: &Stress) -> f64 {
    s.von_mises()
}

/// Map `f` over `items` on scoped threads (one per available core at most), keeping the order.
pub(crate) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut slots: Vec<Option<R>> = items.iter().map(|_| None).collect();
    let results = std::sync::Mutex::new(&mut slots);
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    slots.into_iter().map(|r| r.expect("every item was processed")).collect()
}
