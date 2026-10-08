//! Mesh, fit, load and read-out of one eccentric-bushing analysis.

use fea_core::delaunay::MeshOptions;
use fea_core::fit::{interference_contacts, start_after_fit, start_from, Tuning};
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::nonlinear::Stop;
use crate::control::Control;
use fea_core::{Elastic, ElementKind, Loads, Model, NlSolution, Physics};
use mechanics_core::materials::Material;

/// In-plane orthotropy of a body: the constants along its own axes `1, 2` (`e_psi` of [`Elasticity`] is `E1`), with the
/// axes turned `angle_deg` from the global `x` axis. The out-of-plane constants are taken equal to the in-plane ones
/// (`E3 = E2`, `nu13 = nu23 = nu12`, `G13 = G23 = G12`), which only matters in plane strain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orthotropy {
    pub e2_psi: f64,
    pub g12_psi: f64,
    pub nu12: f64,
    pub angle_deg: f64,
}

/// Elastic constants, psi: isotropic (`e_psi`, `nu`) unless `ortho` is given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elasticity {
    pub e_psi: f64,
    pub nu: f64,
    pub ortho: Option<Orthotropy>,
}

impl Elasticity {
    pub const fn iso(e_psi: f64, nu: f64) -> Self {
        Self { e_psi, nu, ortho: None }
    }

    /// Smallest Young's modulus over the axes (the penalty of a contact is scaled by it).
    pub fn e_min(&self) -> f64 {
        self.ortho.map_or(self.e_psi, |o| self.e_psi.min(o.e2_psi))
    }

    /// The material the kernel uses.
    fn elastic(&self) -> Result<Elastic, String> {
        let Some(o) = self.ortho else { return Ok(Elastic::new(self.e_psi, self.nu)) };
        let m = Elastic::orthotropic(self.e_psi, o.e2_psi, o.e2_psi, o.nu12, o.nu12, o.nu12, o.g12_psi, o.g12_psi, o.g12_psi)?;
        if o.angle_deg == 0.0 { Ok(m) } else { m.rotated_z(o.angle_deg.to_radians()) }
    }
}

impl From<&Material> for Elasticity {
    fn from(m: &Material) -> Self {
        Self { e_psi: m.e_ksi * 1000.0, nu: m.nu, ortho: None }
    }
}

/// Everything one analysis needs. Lengths in inches, forces in lbf, psi. The bore centre of the bushing sits
/// at `(offset, 0)` for a bushing centred on the housing bore at the origin; the pin load acts at `load_angle_deg`
/// from `+x`, i.e. `90` is transverse to the offset line (the worst case for spin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs {
    /// Housing bore diameter (the bushing's outer diameter before the fit is `bore_dia + interference_dia`).
    pub bore_dia: f64,
    /// Outer diameter of the round boss around the bore.
    pub housing_od: f64,
    /// Edge-limited housing: a rectangular plate whose free edge is this far from the bore centre (on the side opposite
    /// the offset, `-x`); the other sides are `max(3 e, 5 D)` away. `None`: the round boss of diameter `housing_od`.
    pub edge_distance: Option<f64>,
    /// Bushing inner (pin) diameter.
    pub bushing_id: f64,
    /// Distance between the bushing's bore centre and its outer-diameter centre.
    pub offset: f64,
    /// Diametral interference: bushing outer diameter minus housing bore diameter.
    pub interference_dia: f64,
    /// Length along the bore (the thickness of the plane model).
    pub thickness: f64,
    pub housing: Elasticity,
    pub bushing: Elasticity,
    /// Coulomb friction between bushing and housing.
    pub friction: f64,
    /// The pin: a meshed elastic disc in frictional contact with the bushing bore, loaded through its length (the body
    /// force of the lug's elastic-pin model), free to rotate. `pin_clearance_dia` is the diametral clearance to the bore
    /// *as fitted* (the fit closes the bore; a pin sized to the nominal ID would start with interference).
    pub pin: Elasticity,
    pub pin_friction: f64,
    pub pin_clearance_dia: f64,
    /// Judge the margin on the capacity with the pin loaded (the interface pressure the pin's real contact produces)
    /// instead of the fit alone. The pin's pressure has a non-zero mean that squeezes the bushing: the loaded capacity
    /// is usually higher, so the fit-alone basis is the conservative one.
    pub credit_pin_load: bool,
    pub load_lbf: f64,
    pub load_angle_deg: f64,
    /// Also run the direct spin simulation ([`spin_onset_torque`], 15-30 s more) and judge the margin on the smaller of
    /// that and the integral capacity: the all-slip integral overstates the true onset by 3-4 % when the wall varies 3:1.
    pub direct_onset: bool,
    /// Smallest acceptable wall at the thin side (the Bushing Workbench's minimum wall): the analysis reports against
    /// it, the offset search reports the offset it allows; the hard numerical floor is `MIN_WALL_FRACTION` of the bore.
    pub min_wall: f64,
    /// Plane strain (axially constrained bushing) instead of plane stress (free ends, the Lame assumption of
    /// `bushing-solver`: the default keeps the fit pressure equal to the Bushing toolbox's).
    pub plane_strain: bool,
    /// Target element size at the interface (`None`: from the thinnest wall).
    pub mesh_size: Option<f64>,
}

/// Thinnest wall the model accepts, as a fraction of the bore diameter (verified down to 2.5 %; the contact solve does
/// not converge reliably at 2 %).
pub const MIN_WALL_FRACTION: f64 = 0.03;

impl Inputs {
    /// Bore radius (the interface radius).
    pub fn bore_radius(&self) -> f64 {
        self.bore_dia / 2.0
    }

    /// Wall of the bushing at its thinnest and thickest side.
    pub fn walls(&self) -> (f64, f64) {
        let wall = self.bore_radius() - self.bushing_id / 2.0;
        (wall - self.offset, wall + self.offset)
    }

    pub fn validate(&self) -> Result<(), String> {
        let positive = [self.bore_dia, self.housing_od, self.bushing_id, self.thickness, self.housing.e_psi, self.bushing.e_psi];
        if positive.iter().any(|v| !(v.is_finite() && *v > 0.0)) {
            return Err("diameters, thickness and moduli must be positive".into());
        }
        if !(self.interference_dia.is_finite() && self.interference_dia > 0.0) {
            return Err("the bushing needs a positive interference to hold (no fit, no friction)".into());
        }
        if !(self.offset.is_finite() && self.offset >= 0.0 && self.load_lbf.is_finite() && self.load_lbf >= 0.0 && self.load_angle_deg.is_finite()) {
            return Err("offset and load must be finite and not negative".into());
        }
        if !(self.bushing_id < self.bore_dia && self.housing_od > 1.5 * self.bore_dia) {
            return Err("the bushing ID must be smaller than the bore and the boss at least 1.5 bore diameters".into());
        }
        if let Some(e) = self.edge_distance {
            if !(e.is_finite() && e > 0.6 * self.bore_dia) {
                return Err(format!("the edge distance {e:.4} in must leave a ligament beside the bore (more than 0.6 bore diameters)"));
            }
        }
        if self.walls().0 < MIN_WALL_FRACTION * self.bore_dia * (1.0 - 1e-9) {
            return Err(format!("the offset leaves a {:.4} in wall at the thin side (the model needs at least {:.0} % of the bore)", self.walls().0, MIN_WALL_FRACTION * 100.0));
        }
        if !(self.pin.e_psi.is_finite() && self.pin.e_psi > 0.0 && (0.0..0.5).contains(&self.pin.nu) && (0.0..=2.0).contains(&self.pin_friction) && self.pin_clearance_dia.is_finite() && self.pin_clearance_dia >= 0.0 && self.pin_clearance_dia < 0.1 * self.bushing_id) {
            return Err("the pin needs a positive modulus, 0 <= nu < 0.5, friction 0..2 and a diametral clearance of at most 10 % of the ID".into());
        }
        if !(0.01..=2.0).contains(&self.friction) || !(0.0..0.5).contains(&self.housing.nu) || !(0.0..0.5).contains(&self.bushing.nu) {
            return Err("friction must be in 0.01..2 (a nearly frictionless fit carries no torque and its contact does not solve) and Poisson's ratios in 0..0.5".into());
        }
        Ok(())
    }
}

/// One angular bin of the interface pressure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileBin {
    /// Bin centre, degrees from `+x`, counter-clockwise: the bore is offset towards `+x`, so the wall is thinnest at 0 and thickest at 180.
    pub angle_deg: f64,
    /// Interface pressure after the fit alone, psi.
    pub fit: f64,
    /// Interface pressure with the pin load, psi.
    pub loaded: f64,
}

/// Result of one analysis.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub offset: f64,
    /// `F e sin(phi)`: the torque the interface must return, lbf in.
    pub torque_required: f64,
    /// `sum mu p |x| w` of the fit alone, lbf in.
    pub torque_capacity_fit: f64,
    /// The same with the pin load applied, lbf in.
    pub torque_capacity: f64,
    /// The capacity the margin is judged on (`Inputs::credit_pin_load`: the loaded one, else the fit alone), lbf in.
    pub design_capacity: f64,
    /// Peak contact pressure of the pin on the bushing bore and the arc it touches, psi / degrees.
    pub pin_peak_pressure: f64,
    pub pin_arc_deg: f64,
    /// The direct simulation's onset torque (`Inputs::direct_onset`), lbf in.
    pub onset_torque: Option<f64>,
    /// Net force the weak springs that keep the bushing and the pin from floating carry (leak), lbf: it must be a negligible
    /// share of the pin load.
    pub ground_leak: f64,
    /// Moment of the interface friction about the bushing's centre, from the FE: it equals `torque_required`
    /// when the interface holds (global equilibrium; a check on the solution, independent of any formula).
    pub friction_torque: f64,
    /// Net force of the interface on the housing (the pin load, by global equilibrium), lbf.
    pub net_force: [f64; 2],
    /// `design_capacity / T_req - 1` (positive = holds); infinite when there is no demand.
    pub margin: f64,
    /// Share of the transmitted normal force that sits on points at their friction limit (1 = all slipping).
    pub slip_share: f64,
    pub fit_pressure_mean: f64,
    pub fit_pressure_min: f64,
    pub fit_pressure_max: f64,
    /// Arc of the interface with no contact pressure under the load, degrees.
    pub contact_lost_deg: f64,
    pub wall_thin: f64,
    pub wall_thick: f64,
    /// The thin wall meets `Inputs::min_wall`.
    pub wall_ok: bool,
    pub profile: Vec<ProfileBin>,
    pub factorisations: usize,
    pub dofs: usize,
    /// Newton solves of the loaded stage accepted on a stalled residual (below 0.3 % of the force scale) instead of the
    /// tolerance; `0` when every one converged.
    pub stalled_solves: usize,
    /// Why the pin load could not be solved, when it could not: the pin fields are zero and the capacity, margin and
    /// profile are those of the fit alone (the conservative basis).
    pub loaded_failure: Option<String>,
}

const BINS: usize = 72;

/// The share of the pin load the rigid pressing-in stage reaches before the pin is released.
const PIN_PRESS_SHARE: f64 = 0.25;

/// The share tried when the loaded stage does not converge from the first one. The friction contact can fall into a
/// stick-slip limit cycle that depends on where the pin is released: the default Bushing Workbench case at 1500 lbf and
/// 0.04 in offset does at 0.25 and 0.4, converges at 0.12.
const PIN_PRESS_RETRY_SHARE: f64 = 0.12;

/// Default element size of the housing and bushing: about 1.2 x the thin wall, between `r / MESH_COARSE` and `r / MESH_FINE`
/// of the bore radius `r` (the pin is a sixth of its own radius, or the bushing's size if coarser: a pin finer than the bushing mesh made the contact converge worse). The capacity integrals are
/// mesh independent far beyond this (the displayed pressure range is not: its bin-to-bin scatter is under 0.5 % at `r / 5` and 10 % at `r / 4`, so the element stays at or below `r / 5`): the margin of the Bushing Workbench defaults at 1500 lbf is 5.921 / 5.920 / 5.921 at
/// elements of 0.1 / 0.07 / 0.05 in, 5.924 at the earlier default (0.031) and the fit capacity of a 0.016 in thin wall is
/// 209.237 / 209.2185 / 209.2214 at 0.1 / 0.05 / 0.03 in (0.01 %), while the solve gets 5-20 times cheaper.
const MESH_FINE: f64 = 6.0;
const MESH_COARSE: f64 = 5.0;

/// The loaded stage accepts a Newton solve whose residual has stalled (no 3 % gain in 8 iterations: a friction contact
/// chattering between stick and slip) below this fraction of the force scale. Case 1 of the random sweep (bore 0.25 in,
/// 113 lbf) crawled through 24 steps and 657 factorisations (103 s) with the strict tolerance and gave a margin of
/// 51.361; accepting the stall it takes 4 steps (7 s) and gives 51.358 (0.006 %). `Analysis::stalled_solves` reports it.
const LOADED_STALL_TOL: f64 = 3e-3;

/// Load steps of the pressing-in stage over its whole travel (a seating stage: the loaded stage reconverges everything,
/// and the force target halves a step that overshoots). 200 -> 50 cut the user's 8856 lbf case from 107 s to 19 s at the
/// same margin (1.443 against 1.441 at 25, 1.442 at 100).
const PRESS_STEPS: usize = 50;

/// When the prescribed pressing-in travel is used up before the pin carries its share (soft parts, large loads), it is
/// extended by this factor, at most `MAX_PRESS_EXTENSIONS` times, continuing from the state reached.
const PRESS_EXTENSION: f64 = 3.0;
const MAX_PRESS_EXTENSIONS: usize = 3;

/// Diagnostics on stderr when `ECCENTRIC_TRACE` is set: one line per solved stage (stop reason, steps, factorisations,
/// wall time), the evidence for a slow or failed run.
pub(crate) fn trace(line: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("ECCENTRIC_TRACE").is_some()) {
        eprintln!("[eccentric] {}", line());
    }
}

fn trace_stage(name: &str, inp: &Inputs, sol: &NlSolution) {
    trace(|| format!("{name}: offset {:.5} load {:.1} -> {:?}, {} steps, {} factorisations, {:.1} s", inp.offset, inp.load_lbf, sol.stop, sol.steps.len(), sol.factorisations, sol.elapsed_ms / 1e3));
}

/// The error a solve returns when its [`Interrupt`] fired (a deadline or a cancel request), not a failure of the model.
pub const INTERRUPTED: &str = "interrupted";


struct Built {
    model: Model,
    housing_bore: Vec<Vec<usize>>,
    bushing_od: Vec<Vec<usize>>,
    bushing_id: Vec<Vec<usize>>,
    supports: [usize; 2],
    ground: Vec<(usize, f64)>,
    h: f64,
}

fn build(inp: &Inputs) -> Result<Built, String> {
    let (r, ri, ro, e) = (inp.bore_radius(), inp.bushing_id / 2.0, inp.housing_od / 2.0, inp.offset);
    let (wall_thin, _) = inp.walls();
    // Element size: uniform, about the thin wall (between an eighth and a fifth of the bore: the pointwise pressure scatter on the non-matching meshes grows with the element size, ~+-10 % at a quarter; the capacity is converged with
    // elements twice the thin wall, and a graded mesh was tried: fine on the thin side, coarse on the thick one, it solved
    // the same model three times slower, the contact converging worse across mismatched element sizes). The housing
    // matches it near the interface.
    let h0 = inp.mesh_size.unwrap_or((1.2 * wall_thin).clamp(r / MESH_FINE, r / MESH_COARSE));
    let size_at = move |_: [f64; 2]| h0;
    let physics = if inp.plane_strain { Physics::PlaneStrain { thickness: inp.thickness } } else { Physics::PlaneStress { thickness: inp.thickness } };
    let opt = MeshOptions::default();

    let outer = match inp.edge_distance {
        None => Loop::circle([0.0, 0.0], ro, "outer")?,
        Some(ed) => {
            let far = (3.0 * ed).max(5.0 * inp.bore_dia);
            Loop::rectangle(-ed, -far, far, far)?
        }
    };
    let housing = Region::new(outer, vec![Loop::circle([0.0, 0.0], r, "bore")?], inp.housing.elastic()?)?;
    let near = move |x: [f64; 2]| (size_at(x) + 0.35 * (x[0].hypot(x[1]) - r).abs()).min(8.0 * r / 4.0);
    let mut mesh = mesh_region(&housing, physics, ElementKind::Quad9, &near, opt)?;
    let supports = {
        // Two point supports on the free boundary (the far side both dofs, the near side one): the fit and the pin load
        // reach them only as the (small) reaction of the pin load.
        let (a_set, b_set, xa, xb) = match inp.edge_distance {
            None => ("outer", "outer", ro, -ro),
            Some(ed) => ("right", "left", (3.0 * ed).max(5.0 * inp.bore_dia), -ed),
        };
        let nearest = |set: &str, tx: f64| -> Result<usize, String> {
            let nodes = mesh.node_set(set)?;
            nodes.iter().copied().min_by(|&a, &b| (mesh.nodes[a][0] - tx).hypot(mesh.nodes[a][1]).total_cmp(&(mesh.nodes[b][0] - tx).hypot(mesh.nodes[b][1]))).ok_or_else(|| "housing mesh has no boundary nodes".to_string())
        };
        [nearest(a_set, xa)?, nearest(b_set, xb)?]
    };

    let bushing = Region::new(Loop::circle([0.0, 0.0], r, "od")?, vec![Loop::circle([e, 0.0], ri, "id")?], inp.bushing.elastic()?)?;
    let bmesh = mesh_region(&bushing, physics, ElementKind::Quad9, &size_at, opt)?;
    let off = mesh.append(&bmesh, "b/")?;

    let surface = |name: &str| mesh.surfaces.get(name).cloned().ok_or_else(|| format!("missing surface {name}"));
    let (housing_bore, bushing_od, bushing_id) = (surface("bore")?, surface("b/od")?, surface("b/id")?);
    // The bushing is held only by its contact: weak springs keep a lost or frictionless fit from leaving a free body.
    let k = 1e-9 * inp.bushing.e_psi * inp.thickness;
    let ground = (off..mesh.nodes.len()).flat_map(|n| [(2 * n, k), (2 * n + 1, k)]).collect();
    let model = Model::new(mesh)?;
    Ok(Built { model, housing_bore, bushing_od, bushing_id, supports, ground, h: h0 })
}

/// Net force the interface returns on the housing, `[x, y]`.
fn interface_force(pts: &[Pt]) -> [f64; 2] {
    pts.iter().fold([0.0, 0.0], |f, q| {
        let (sn, cs) = q.angle.to_radians().sin_cos();
        [f[0] + q.w * (q.p * cs - q.shear * sn), f[1] + q.w * (q.p * sn + q.shear * cs)]
    })
}

/// A converged run that does not transmit the pin load (the pin was never caught by the bore, or a stalled residual
/// measured against the large fit forces hid a pin carrying a fraction of it) is a failure, not a result: the interface
/// force must return the load. The force is a sum over the whole fit pressure, so its integration noise (about 0.03 % of
/// the fit force) bounds how small a load can be checked this way.
fn transmitted(inp: &Inputs, pts: &[Pt]) -> Result<(), String> {
    let net = interface_force(pts);
    let phi = inp.load_angle_deg.to_radians();
    let normal: f64 = pts.iter().map(|q| q.p * q.w).sum();
    let carried = net[0] * phi.cos() + net[1] * phi.sin();
    if (carried - inp.load_lbf).abs() > 0.05 * inp.load_lbf + 0.5 + 4e-3 * normal {
        return Err(format!("the pin load was not transmitted to the bushing ({carried:.1} of {:.1} lbf): the solution is not an equilibrium of the loaded pin", inp.load_lbf));
    }
    Ok(())
}

/// One interface point of the fit contacts (two passes: their pressures add up to the physical pressure).
struct Pt {
    angle: f64,
    radius: f64,
    p: f64,
    /// Force along `+theta` on the housing per unit weight.
    shear: f64,
    w: f64,
    slipping: bool,
    first_pass: bool,
}

fn interface_points(nl: &NlSolution, first: usize, mu: f64) -> Vec<Pt> {
    let mut out = Vec::new();
    for pass in 0..2 {
        for t in nl.contact_tractions(first + pass, mu) {
            let th = t.x[1].atan2(t.x[0]);
            let (sn, cs) = th.sin_cos();
            let f_t = t.friction[0] * -sn + t.friction[1] * cs;
            // The housing is the slave of pass 0 (force on it: minus the friction traction), the bushing of pass 1
            // (the traction is the force on the housing).
            let shear = if pass == 0 { -f_t } else { f_t };
            out.push(Pt { angle: th.to_degrees().rem_euclid(360.0), radius: t.x[0].hypot(t.x[1]), p: t.pressure, shear, w: t.weight, slipping: t.slipping, first_pass: pass == 0 });
        }
    }
    out
}

/// Interface pressure per angular bin: both passes' force over the mean slave length of the bin.
fn pressure_bins(pts: &[Pt]) -> [f64; BINS] {
    let (mut f, mut wa, mut wb) = ([0.0; BINS], [0.0; BINS], [0.0; BINS]);
    for q in pts {
        let b = ((q.angle / 360.0 * BINS as f64) as usize).min(BINS - 1);
        f[b] += q.p * q.w;
        if q.first_pass { wa[b] += q.w } else { wb[b] += q.w }
    }
    std::array::from_fn(|b| {
        let w = 0.5 * (wa[b] + wb[b]);
        if w > 0.0 { f[b] / w } else { 0.0 }
    })
}

impl Fit {
    /// Torque capacity of the fit alone, lbf in.
    pub(crate) fn capacity(&self, mu: f64) -> f64 {
        capacity(&self.pts, mu)
    }
}

fn capacity(pts: &[Pt], mu: f64) -> f64 {
    pts.iter().map(|q| mu * q.p * q.w * q.radius).sum()
}

/// The mesh, supports and the converged interference fit (stage 1), kept for the loaded run.
pub(crate) struct Fit {
    b: Built,
    tune: Tuning,
    eps_n: f64,
    sol: NlSolution,
    pts: Vec<Pt>,
}

fn fit_specs(b: &Built, eps_n: f64, inp: &Inputs) -> Vec<fea_core::contact::ContactSpec> {
    interference_contacts("fit", b.housing_bore.clone(), b.bushing_od.clone(), eps_n, inp.friction, 0.5 * inp.interference_dia, 6.0 * b.h).to_vec()
}

/// Stage 1: install the interference fit with no pin load.
pub(crate) fn solve_fit(inp: &Inputs, ctl: &Control) -> Result<Fit, String> {
    inp.validate()?;
    let b = build(inp)?;
    // The fit is the start of the loaded stage: converge it tighter than that stage's own tolerance, or its leftover
    // residual is larger than the small first load steps can resolve.
    let tune = Tuning { newton_tol: 1e-6, ..Tuning::friction() };
    let eps_n = tune.eps_n(inp.housing.e_min().min(inp.bushing.e_min()), inp.bore_radius());
    let mut bc = b.model.dirichlet();
    bc.fix_node(b.supports[0]);
    bc.fix(b.supports[1], 1, 0.0);
    let loads = Loads { ground: b.ground.clone(), ..Default::default() };
    let mut opts = tune.options(1);
    opts.interrupt = ctl.interrupt.clone();
    opts.stick_slip_guard = true;
    ctl.stage("fit");
    opts.observer = ctl.observer(format!("e {:.4} in, fit", inp.offset));
    let sol = b.model.solve_nonlinear_contact(&loads, &bc, fit_specs(&b, eps_n, inp), &opts)?;
    trace_stage("fit", inp, &sol);
    if sol.stop == Stop::Interrupted {
        return Err(INTERRUPTED.into());
    }
    if !sol.complete() {
        return Err(format!("the interference fit could not be solved: {:?}", sol.stop));
    }
    let pts = interface_points(&sol, 0, inp.friction);
    Ok(Fit { b, tune, eps_n, sol, pts })
}

/// The bushing's bore after the fit: the displaced nodes of the bore surface, and the circle that fits them. A
/// least-squares circle (Kasa) is exact for points on a circle however they are spaced: the mean of the nodes is not
/// (the mesh is finer on the thin side, which pulled the mean 1.5 mm off the centre).
fn fitted_bore(b: &Built, fit: &NlSolution) -> (Vec<[f64; 2]>, [f64; 2], f64) {
    let mut nodes: Vec<usize> = b.bushing_id.iter().flatten().copied().collect();
    nodes.sort_unstable();
    nodes.dedup();
    let pos: Vec<[f64; 2]> = nodes.iter().map(|&n| [b.model.mesh.nodes[n][0] + fit.u[2 * n], b.model.mesh.nodes[n][1] + fit.u[2 * n + 1]]).collect();
    // x^2 + y^2 = 2 a x + 2 b y + c  (centre (a, b), radius^2 = c + a^2 + b^2): normal equations.
    let (mut sxx, mut sxy, mut syy, mut sx, mut sy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut sxz, mut syz, mut sz) = (0.0, 0.0, 0.0);
    let m = pos.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / pos.len() as f64, a[1] + p[1] / pos.len() as f64]);
    for p in &pos {
        let (x, y) = (p[0] - m[0], p[1] - m[1]);
        let z = x * x + y * y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sx += x;
        sy += y;
        sxz += x * z;
        syz += y * z;
        sz += z;
        n += 1.0;
    }
    // Solve [sxx sxy sx; sxy syy sy; sx sy n] [2a 2b c]^T = [sxz syz sz]^T by Cramer's rule.
    let det = |a: [[f64; 3]; 3]| a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    let a = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]];
    let rhs = [sxz, syz, sz];
    let d = det(a);
    let col = |k: usize| {
        let mut t = a;
        for r in 0..3 {
            t[r][k] = rhs[r];
        }
        det(t) / d
    };
    let (a2, b2, c) = (col(0), col(1), col(2));
    let centre = [m[0] + 0.5 * a2, m[1] + 0.5 * b2];
    let radius = (c + 0.25 * (a2 * a2 + b2 * b2)).max(0.0).sqrt();
    (pos, centre, radius)
}

/// The loaded stage: a meshed elastic pin in frictional deformable contact with the bore, loaded through its length by
/// a body force of resultant `F` (the lug's elastic-pin model) and free to rotate. The pin's pressure distribution is
/// whatever its stiffness, the clearance and the friction make it.
///
/// A free pin has nothing to push against while it crosses its clearance, which load control cannot do (every step is
/// an enormous Newton move) and which arc-length steps can leap over. So it is done in two stages: first the pin is
/// pressed in by a prescribed rigid translation along the load until it carries a quarter of `F` (displacement control
/// is robust at first touch), then it is released and the body force is ramped from that level to `F` (the contact patch
/// already exists, and the stage continues from it: `Start::lambda`). Returns the solution and the model it ran on.
fn solve_loaded(inp: &Inputs, fit: &Fit, ctl: &Control) -> Result<(NlSolution, Model), String> {
    // The attempts, in order: a friction contact that cycles between stick and slip is cured by a different route, not
    // a stronger version of the same (see `LoadAttempt`).
    let mut last = String::new();
    for (i, attempt) in LOAD_ATTEMPTS.iter().enumerate() {
        match solve_loaded_from(inp, fit, ctl, attempt) {
            Ok(r) => return Ok(r),
            Err(LoadFailure::NoConvergence(m)) if !ctl.interrupt.triggered() => {
                trace(|| format!("attempt {} ({}) failed: {m}", i + 1, attempt.name));
                last = m;
            }
            Err(LoadFailure::NoConvergence(m) | LoadFailure::Other(m)) => return Err(m),
        }
    }
    Err(format!("{last} (after {} attempts: {})", LOAD_ATTEMPTS.len(), LOAD_ATTEMPTS.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")))
}

/// One way of running the loaded stage.
struct LoadAttempt {
    name: &'static str,
    /// The share of the load the rigid pressing-in stage reaches before the pin is released.
    share: f64,
    /// `NlOptions::step_memory`.
    step_memory: bool,
}

/// The default case at 0.031 in elements and 1500 lbf cycles at share 0.25 and 0.4 (and with anchor stiffness 1e-5 / 1e-4)
/// and converges at share 0.12 and with step memory; the reported case (8856 lbf, 0.1875 in bushing) converges only
/// without step memory (4 s, against failing with it). Hence the order: the plain run first.
const LOAD_ATTEMPTS: [LoadAttempt; 3] = [
    LoadAttempt { name: "release at 25 %", share: PIN_PRESS_SHARE, step_memory: false },
    LoadAttempt { name: "release at 25 %, step memory", share: PIN_PRESS_SHARE, step_memory: true },
    LoadAttempt { name: "release at 12 %", share: PIN_PRESS_RETRY_SHARE, step_memory: false },
];

/// Why a loaded run failed: `NoConvergence` is the one a different release point can cure.
enum LoadFailure {
    NoConvergence(String),
    Other(String),
}

impl From<String> for LoadFailure {
    fn from(m: String) -> Self {
        LoadFailure::Other(m)
    }
}

fn solve_loaded_from(inp: &Inputs, fit: &Fit, ctl: &Control, attempt: &LoadAttempt) -> Result<(NlSolution, Model), LoadFailure> {
    let share = attempt.share;
    let (bore_pts, centre, _bore) = fitted_bore(&fit.b, &fit.sol);
    let phi = inp.load_angle_deg.to_radians();
    let dir = [phi.cos(), phi.sin()];
    // The fitted bore is not exactly round (the thin side gives way more): the pin is sized to its tightest point.
    let tightest = |c: [f64; 2]| bore_pts.iter().map(|p| (p[0] - c[0]).hypot(p[1] - c[1])).fold(f64::INFINITY, f64::min);
    let rp = tightest(centre) - 0.5 * inp.pin_clearance_dia;
    let physics = fit.b.model.mesh.physics;
    // The pin's own element size: a sixth of its radius (independent of the thin wall: tying it to the wall made the loaded
    // stage fail to converge on the default toolbox case); the capacity does not depend on it, the pin's peak pressure does.
    let hp = inp.mesh_size.unwrap_or((inp.bushing_id / 2.0 / 6.0).max(fit.b.h));
    let region = Region::new(Loop::circle(centre, rp, "rim")?, Vec::new(), inp.pin.elastic()?)?;
    let pmesh = mesh_region(&region, physics, ElementKind::Quad9, &move |_| hp, MeshOptions::default())?;
    let mut mesh = fit.b.model.mesh.clone();
    let first_pin_node = mesh.nodes.len();
    mesh.append(&pmesh, "p/")?;
    let n_nodes = mesh.nodes.len();
    let pin_block = mesh.blocks.len() - 1;
    let pin_faces = mesh.surfaces.get("p/rim").cloned().ok_or_else(|| "missing pin surface".to_string())?;
    let area = std::f64::consts::PI * rp * rp;
    let b_force = [inp.load_lbf * dir[0] / (area * inp.thickness), inp.load_lbf * dir[1] / (area * inp.thickness), 0.0];
    // Weak springs remove the pin's three rigid modes without leaking load: two rim nodes perpendicular to the load
    // (node A both translations, node B the motion along the load, which stops the rotation).
    // Strong enough to make the pin's soft rigid modes determinate (with 1e-6 the friction contact stagnated on the default
    // toolbox case), weak enough to leak ~0.1 % of the load (`Analysis::ground_leak`).
    let k = 3e-5 * inp.pin.e_psi * inp.thickness;
    let rim_node = |ang: f64| -> usize {
        let t = [centre[0] + rp * ang.cos(), centre[1] + rp * ang.sin()];
        (first_pin_node..n_nodes).min_by(|&a, &b| (mesh.nodes[a][0] - t[0]).hypot(mesh.nodes[a][1] - t[1]).total_cmp(&(mesh.nodes[b][0] - t[0]).hypot(mesh.nodes[b][1] - t[1]))).expect("pin nodes")
    };
    let (na, nb) = (rim_node(phi + std::f64::consts::FRAC_PI_2), rim_node(phi - std::f64::consts::FRAC_PI_2));
    let along = |n: usize| if dir[0].abs() >= dir[1].abs() { 2 * n } else { 2 * n + 1 };
    let anchors = [(2 * na, k), (2 * na + 1, k), (along(nb), k)];
    let model = Model::new(mesh)?;
    let supports = |model: &Model| {
        let mut bc = model.dirichlet();
        bc.fix_node(fit.b.supports[0]);
        bc.fix(fit.b.supports[1], 1, 0.0);
        bc
    };
    let eps_n = fit.tune.eps_n(inp.housing.e_min().min(inp.bushing.e_min()).min(inp.pin.e_min()), inp.bore_radius());
    let mut specs = interference_contacts("pin", fit.b.bushing_id.clone(), pin_faces, eps_n, inp.pin_friction, 0.0, 6.0 * fit.b.h).to_vec();
    specs.extend(fit_specs(&fit.b, fit.eps_n, inp));

    // Stage A: the pin pressed in as a rigid body (prescribed displacement scales with the load factor) until it carries
    // a quarter of the load.
    let target = share * inp.load_lbf;
    let mut travel = 0.5 * inp.pin_clearance_dia + 0.05 * inp.bushing_id;
    let loads_a = Loads { ground: fit.b.ground.clone(), ..Default::default() };
    let mut opts_a = Tuning::friction().options(200);
    opts_a.first_step = 0.05;
    opts_a.stop_at_force = target;
    opts_a.steps = PRESS_STEPS;
    // A seating stage only: loose tolerances and few multiplier passes (the loaded stage reconverges everything).
    opts_a.tol = 1e-3;
    opts_a.max_outer = 2;
    opts_a.outer_tol = 5e-2;
    opts_a.interrupt = ctl.interrupt.clone();
    opts_a.stick_slip_guard = true;
    ctl.stage("press-in");
    opts_a.observer = ctl.observer(format!("e {:.4} in, {:.0} lbf, pin press-in", inp.offset, inp.load_lbf));
    let mut start_a = start_after_fit(&fit.sol, 2);
    let mut extensions = 0;
    let sol_a = loop {
        let mut bc_a = supports(&model);
        for n in first_pin_node..n_nodes {
            bc_a.fix(n, 0, travel * dir[0]);
            bc_a.fix(n, 1, travel * dir[1]);
        }
        let sol = model.solve_nonlinear_contact_from(&loads_a, &bc_a, specs.clone(), &opts_a, Some(&start_a))?;
        trace_stage(&format!("press-in (share {share}, travel {travel:.4})"), inp, &sol);
        match sol.stop {
            Stop::ForceReached => break sol,
            Stop::Interrupted => return Err(LoadFailure::Other(INTERRUPTED.into())),
            // The travel was used up before the pin carried its share: press further from where it stands (the travel so
            // far is the fraction `1 / PRESS_EXTENSION` of the longer one).
            Stop::Completed if extensions < MAX_PRESS_EXTENSIONS => {
                extensions += 1;
                start_a = start_from(&sol, 1.0 / PRESS_EXTENSION);
                travel *= PRESS_EXTENSION;
            }
            ref stop => return Err(LoadFailure::Other(format!("the pin could not be pressed in to {target:.0} lbf ({stop:?}, travel {travel:.4} in)"))),
        }
    };
    let carried = sol_a.steps.last().and_then(|s| s.master_force.first()).map_or(0.0, |f| f[0] * dir[0] + f[1] * dir[1]).abs();
    let lambda0 = (carried / inp.load_lbf).clamp(0.02, 0.9);

    // Stage B: released, loaded by the body force from `lambda0` to the full load.
    let loads_b = Loads { block_body: vec![(pin_block, b_force)], ground: fit.b.ground.iter().copied().chain(anchors).collect(), ..Default::default() };
    let bc_b = supports(&model);
    let mut opts_b = Tuning::friction().options(4);
    opts_b.first_step = 0.5;
    opts_b.interrupt = ctl.interrupt.clone();
    ctl.stage("pin load");
    opts_b.observer = ctl.observer(format!("e {:.4} in, {:.0} lbf, pin load", inp.offset, inp.load_lbf));
    opts_b.stall_tol = LOADED_STALL_TOL;
    opts_b.stick_slip_guard = true;
    opts_b.step_memory = attempt.step_memory;
    let nl = model.solve_nonlinear_contact_from(&loads_b, &bc_b, specs, &opts_b, Some(&start_from(&sol_a, lambda0)))?;
    trace_stage(&format!("pin load (from {lambda0:.3})"), inp, &nl);
    if nl.stop == Stop::Interrupted {
        return Err(LoadFailure::Other(INTERRUPTED.into()));
    }
    if !nl.complete() {
        return Err(LoadFailure::NoConvergence(format!("the pin load could not be solved: {:?}", nl.stop)));
    }
    // A stalled residual is measured against the whole force scale (the fit forces dwarf a light pin load): accept it only
    // when the interface really returns the pin load, else the next attempt runs.
    if nl.stalled_solves > 0 {
        transmitted(inp, &interface_points(&nl, 2, inp.friction)).map_err(|e| LoadFailure::NoConvergence(format!("a stalled residual was accepted but {e}")))?;
    }
    Ok((nl, model))
}

/// Peak pressure and angular extent of the pin's contact with the bore (both passes' pressures added per angle bin).
fn pin_contact(nl: &NlSolution) -> (f64, f64) {
    let mut bins = [0.0f64; BINS];
    let mut w = [0.0f64; BINS];
    for pass in 0..2 {
        for ((s, x), wt) in nl.state.contact[pass].iter().zip(&nl.contact_points[pass]).zip(&nl.contact_weights[pass]) {
            let a = x[1].atan2(x[0]).to_degrees().rem_euclid(360.0);
            let b = ((a / 360.0 * BINS as f64) as usize).min(BINS - 1);
            if s.active {
                bins[b] += s.p * wt;
            }
            if pass == 0 {
                w[b] += wt;
            }
        }
    }
    let p: Vec<f64> = (0..BINS).map(|b| if w[b] > 0.0 { bins[b] / w[b] } else { 0.0 }).collect();
    let peak = p.iter().cloned().fold(0.0, f64::max);
    (peak, p.iter().filter(|&&v| v > 1e-3 * peak.max(1e-300)).count() as f64 * 360.0 / BINS as f64)
}

/// The spin-onset torque by direct simulation, independent of the capacity integral `sum mu p |x| w`: a pure torque is
/// applied to the bushing's bore as a uniform tangential traction (no pin, no force) on top of the installed fit and
/// raised under arc-length control until the interface slips all round. The load factor climbs steeply, then its
/// increments collapse (a knee) and it creeps on: the onset is the load at the knee (the creep after it is the slipping
/// interface re-seating, not capacity). `None` when no knee is reached within `2.5 x` the integral capacity.
pub fn spin_onset_torque(inp: &Inputs) -> Result<Option<f64>, String> {
    spin_onset_from_fit(inp, &solve_fit(inp, &Control::default())?)
}

fn spin_onset_from_fit(inp: &Inputs, fit: &Fit) -> Result<Option<f64>, String> {
    let t_ref = 2.5 * capacity(&fit.pts, inp.friction);
    let ri = inp.bushing_id / 2.0;
    let (c0, t) = (inp.offset, inp.thickness);
    // Tangential traction about the bore centre: torque = tau * (2 pi ri) * t * ri.
    let tau = t_ref / (2.0 * std::f64::consts::PI * ri * ri * t);
    let field = fea_core::loads::FaceField::new(move |x: &[f64; 3], _n: &[f64; 3]| {
        let psi = x[1].atan2(x[0] - c0);
        [-tau * psi.sin(), tau * psi.cos(), 0.0]
    });
    let loads = Loads { field_faces: fit.b.bushing_id.iter().map(|f| (f.clone(), field.clone())).collect(), ground: fit.b.ground.clone(), ..Default::default() };
    let mut bc = fit.b.model.dirichlet();
    bc.fix_node(fit.b.supports[0]);
    bc.fix(fit.b.supports[1], 1, 0.0);
    let nodes = fit.b.model.mesh.nodes.len() as f64;
    let mut opts = Tuning::friction().options(1);
    opts.control = fea_core::Control::Arc { ds: 1e-5 * nodes.sqrt(), lambda_max: 1.0, max_steps: 30 };
    let sol = fit.b.model.solve_nonlinear_contact_from(&loads, &bc, fit_specs(&fit.b, fit.eps_n, inp), &opts, Some(&start_from(&fit.sol, 0.0)))?;
    let lam: Vec<f64> = std::iter::once(0.0).chain(sol.steps.iter().map(|s| s.lambda)).collect();
    let inc: Vec<f64> = lam.windows(2).map(|w| w[1] - w[0]).collect();
    let max_inc = inc.iter().cloned().fold(0.0f64, f64::max);
    let peak = inc.iter().position(|&v| v == max_inc).unwrap_or(0);
    // The first step after the largest one whose increment has collapsed to a few percent of it.
    Ok(inc.iter().enumerate().skip(peak + 1).find(|(_, &v)| v < 0.06 * max_inc).map(|(k, _)| lam[k] * t_ref))
}

/// Torque capacity of the fit alone (stage 1 only: the cheap, conservative bound the offset search starts from).
pub fn fit_capacity(inp: &Inputs) -> Result<f64, String> {
    fit_capacity_with(inp, &Control::default())
}

/// [`fit_capacity`] that gives up with `Err(INTERRUPTED)` when `ctl`'s interrupt fires.
pub fn fit_capacity_with(inp: &Inputs, ctl: &Control) -> Result<f64, String> {
    solve_fit(inp, ctl).map(|f| capacity(&f.pts, inp.friction))
}

/// Run the fit, then the pin load, and read the interface.
pub fn analyze(inp: &Inputs) -> Result<Analysis, String> {
    analyze_with(inp, &Control::default())
}

/// [`analyze`] that gives up with `Err(INTERRUPTED)` when `ctl`'s interrupt fires.
pub fn analyze_with(inp: &Inputs, ctl: &Control) -> Result<Analysis, String> {
    analyze_inner(inp, &solve_fit(inp, ctl)?, ctl, true).map(|r| r.0)
}

/// The analysis on an already solved fit (stage 1 does not depend on the pin load: a search over the load solves it once).
pub(crate) fn analyze_from_fit(inp: &Inputs, fit: &Fit, ctl: &Control) -> Result<Analysis, String> {
    analyze_inner(inp, fit, ctl, false).map(|r| r.0)
}

/// The analysis and, with a pin load, the loaded solution and its model (the fields of `analyze_fields`).
fn analyze_inner(inp: &Inputs, fit: &Fit, ctl: &Control, fall_back: bool) -> Result<(Analysis, Option<(NlSolution, Model)>), String> {
    let mut loaded_failure = None;
    let loaded = if inp.load_lbf > 0.0 {
        match solve_loaded(inp, fit, ctl) {
            Ok(l) => Some(l),
            // A stand-alone analysis still reports the fit and its capacity, with the failure stated (the conservative
            // basis does not credit the pin's squeeze); a search must not mix the two bases, it stops instead.
            Err(e) if fall_back && e != INTERRUPTED => {
                loaded_failure = Some(e);
                None
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    let factorisations = fit.sol.factorisations + loaded.as_ref().map_or(0, |l| l.0.factorisations);
    let pts = loaded.as_ref().map_or_else(|| interface_points(&fit.sol, 0, inp.friction), |(nl, _)| interface_points(nl, 2, inp.friction));
    // The pin's contact on the bore: its two passes' pressures add up (as at the fit interface).
    let (pin_peak_pressure, pin_arc_deg) = loaded.as_ref().map_or((0.0, 0.0), |(nl, _)| pin_contact(nl));
    let phi = inp.load_angle_deg.to_radians();
    let fit_pts = &fit.pts;

    let (fit_bins, bins) = (pressure_bins(fit_pts), pressure_bins(&pts));
    let torque_required = inp.load_lbf * inp.offset * phi.sin().abs();
    let torque_capacity_fit = capacity(fit_pts, inp.friction);
    let torque_capacity = capacity(&pts, inp.friction);
    let integral_capacity = if inp.credit_pin_load { torque_capacity } else { torque_capacity_fit };
    let onset_torque = if inp.direct_onset && inp.load_lbf > 0.0 { spin_onset_from_fit(inp, fit)? } else { None };
    // The onset simulation runs on the fit pressure (no pin load): it bounds the fit-alone capacity, and the loaded one
    // is scaled by the same ratio.
    let design_capacity = match onset_torque {
        Some(onset) if torque_capacity_fit > 0.0 => integral_capacity * (onset / torque_capacity_fit).min(1.0),
        _ => integral_capacity,
    };
    let friction_torque: f64 = pts.iter().map(|q| q.shear * q.w * q.radius).sum();
    let net_force = interface_force(&pts);
    let normal: f64 = pts.iter().map(|q| q.p * q.w).sum();
    if inp.load_lbf > 0.0 && loaded.is_some() {
        transmitted(inp, &pts)?;
    }
    let slip: f64 = pts.iter().filter(|q| q.slipping).map(|q| q.p * q.w).sum();
    let mean = fit_bins.iter().sum::<f64>() / BINS as f64;
    let (wall_thin, wall_thick) = inp.walls();
    let analysis = Analysis {
        offset: inp.offset,
        torque_required,
        torque_capacity_fit,
        torque_capacity,
        design_capacity,
        pin_peak_pressure,
        pin_arc_deg,
        onset_torque,
        ground_leak: loaded.as_ref().map_or(fit.sol.ground_leak, |(nl, _)| nl.ground_leak),
        friction_torque,
        net_force,
        margin: if torque_required > 0.0 { design_capacity / torque_required - 1.0 } else { f64::INFINITY },
        slip_share: if normal > 0.0 { slip / normal } else { 0.0 },
        fit_pressure_mean: mean,
        fit_pressure_min: fit_bins.iter().cloned().fold(f64::INFINITY, f64::min),
        fit_pressure_max: fit_bins.iter().cloned().fold(0.0, f64::max),
        contact_lost_deg: bins.iter().filter(|&&p| p < 1e-3 * mean.max(1e-300)).count() as f64 * 360.0 / BINS as f64,
        wall_thin,
        wall_thick,
        wall_ok: wall_thin >= inp.min_wall,
        profile: (0..BINS).map(|i| ProfileBin { angle_deg: (i as f64 + 0.5) * 360.0 / BINS as f64, fit: fit_bins[i], loaded: bins[i] }).collect(),
        factorisations,
        dofs: fit.b.model.mesh.n_dofs(),
        stalled_solves: loaded.as_ref().map_or(0, |(nl, _)| nl.stalled_solves),
        loaded_failure,
    };
    Ok((analysis, loaded))
}

/// [`analyze`] and the finite-element fields of the solved model as `.vtu` text (ParaView): the displacement vector, von
/// Mises and the stress components at the nodes, over the housing, bushing and pin.
pub fn analyze_fields(inp: &Inputs, ctl: &Control) -> Result<(Analysis, String), String> {
    use fea_core::kernel::von_mises;
    use fea_core::vtu::{pad3, write, Field};
    let fit = solve_fit(inp, ctl)?;
    let (analysis, loaded) = analyze_inner(inp, &fit, ctl, true)?;
    let (u, model) = match &loaded {
        Some((nl, model)) => (&nl.u, model),
        None => (&fit.sol.u, &fit.b.model),
    };
    let gauss = model.gauss_stresses(u, 0.0)?;
    let nodal = model.recover_spr(&gauss).or_else(|_| model.nodal_stresses(u, 0.0))?;
    let disp = pad3(u, model.mesh.dim());
    let vm: Vec<f64> = nodal.iter().map(von_mises).collect();
    let stress: Vec<f64> = nodal.iter().flat_map(|s| s.iter().copied()).collect();
    let text = write(&model.mesh, &[Field { name: "displacement", ncomp: 3, data: &disp }, Field { name: "von_mises", ncomp: 1, data: &vm }, Field { name: "stress_xx_yy_zz_xy_yz_zx", ncomp: 6, data: &stress }], &[])?;
    Ok((analysis, text))
}
