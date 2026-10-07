//! Mesh, fit, load and read-out of one eccentric-bushing analysis.

use fea_core::delaunay::MeshOptions;
use fea_core::fit::{interference_contacts, start_after_fit, start_from, Tuning};
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::nonlinear::Stop;
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
}

const BINS: usize = 72;

/// The share of the pin load the rigid pressing-in stage reaches before the pin is released.
const PIN_PRESS_SHARE: f64 = 0.25;


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
    let h0 = inp.mesh_size.unwrap_or((1.2 * wall_thin).clamp(r / 8.0, r / 5.0));
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

fn capacity(pts: &[Pt], mu: f64) -> f64 {
    pts.iter().map(|q| mu * q.p * q.w * q.radius).sum()
}

/// The mesh, supports and the converged interference fit (stage 1), kept for the loaded run.
struct Fit {
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
fn solve_fit(inp: &Inputs) -> Result<Fit, String> {
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
    let sol = b.model.solve_nonlinear_contact(&loads, &bc, fit_specs(&b, eps_n, inp), &tune.options(1))?;
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
fn solve_loaded(inp: &Inputs, fit: &Fit) -> Result<(NlSolution, Model), String> {
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
    let pin_faces = mesh.surfaces.get("p/rim").cloned().ok_or("missing pin surface")?;
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
    let target = PIN_PRESS_SHARE * inp.load_lbf;
    let travel = 0.5 * inp.pin_clearance_dia + 0.05 * inp.bushing_id;
    let mut bc_a = supports(&model);
    for n in first_pin_node..n_nodes {
        bc_a.fix(n, 0, travel * dir[0]);
        bc_a.fix(n, 1, travel * dir[1]);
    }
    let loads_a = Loads { ground: fit.b.ground.clone(), ..Default::default() };
    let mut opts_a = Tuning::friction().options(200);
    opts_a.first_step = 0.05;
    opts_a.stop_at_force = target;
    opts_a.steps = 200;
    // A seating stage only: loose tolerances and few multiplier passes (the loaded stage reconverges everything).
    opts_a.tol = 1e-3;
    opts_a.max_outer = 2;
    opts_a.outer_tol = 5e-2;
    let sol_a = model.solve_nonlinear_contact_from(&loads_a, &bc_a, specs.clone(), &opts_a, Some(&start_after_fit(&fit.sol, 2)))?;
    if sol_a.stop != Stop::ForceReached {
        return Err(format!("the pin could not be pressed in to {target:.0} lbf ({:?})", sol_a.stop));
    }
    let carried = sol_a.steps.last().and_then(|s| s.master_force.first()).map_or(0.0, |f| f[0] * dir[0] + f[1] * dir[1]).abs();
    let lambda0 = (carried / inp.load_lbf).clamp(0.02, 0.9);

    // Stage B: released, loaded by the body force from `lambda0` to the full load.
    let loads_b = Loads { block_body: vec![(pin_block, b_force)], ground: fit.b.ground.iter().copied().chain(anchors).collect(), ..Default::default() };
    let bc_b = supports(&model);
    let mut opts_b = Tuning::friction().options(4);
    opts_b.first_step = 0.5;
    let nl = model.solve_nonlinear_contact_from(&loads_b, &bc_b, specs, &opts_b, Some(&start_from(&sol_a, lambda0)))?;
    if !nl.complete() {
        return Err(format!("the pin load could not be solved: {:?}", nl.stop));
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
    spin_onset_from_fit(inp, &solve_fit(inp)?)
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
    solve_fit(inp).map(|f| capacity(&f.pts, inp.friction))
}

/// Run the fit, then the pin load, and read the interface.
pub fn analyze(inp: &Inputs) -> Result<Analysis, String> {
    let fit = solve_fit(inp)?;
    let loaded = if inp.load_lbf > 0.0 { Some(solve_loaded(inp, &fit)?) } else { None };
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
    let onset_torque = if inp.direct_onset && inp.load_lbf > 0.0 { spin_onset_from_fit(inp, &fit)? } else { None };
    // The onset simulation runs on the fit pressure (no pin load): it bounds the fit-alone capacity, and the loaded one
    // is scaled by the same ratio.
    let design_capacity = match onset_torque {
        Some(onset) if torque_capacity_fit > 0.0 => integral_capacity * (onset / torque_capacity_fit).min(1.0),
        _ => integral_capacity,
    };
    let friction_torque: f64 = pts.iter().map(|q| q.shear * q.w * q.radius).sum();
    let net_force = pts.iter().fold([0.0, 0.0], |f, q| {
        let (sn, cs) = q.angle.to_radians().sin_cos();
        [f[0] + q.w * (q.p * cs - q.shear * sn), f[1] + q.w * (q.p * sn + q.shear * cs)]
    });
    let normal: f64 = pts.iter().map(|q| q.p * q.w).sum();
    // A converged run that does not transmit the pin load (the pin was never caught by the bore) is a failure, not a
    // result: the interface force must return the load. The force is a sum over the whole fit pressure, so its
    // integration noise (about 0.03 % of the fit force) bounds how small a load can be checked this way.
    if inp.load_lbf > 0.0 {
        let carried = net_force[0] * phi.cos() + net_force[1] * phi.sin();
        if (carried - inp.load_lbf).abs() > 0.05 * inp.load_lbf + 0.5 + 4e-3 * normal {
            return Err(format!("the pin load was not transmitted to the bushing ({carried:.1} of {:.1} lbf): the solution is not an equilibrium of the loaded pin", inp.load_lbf));
        }
    }
    let slip: f64 = pts.iter().filter(|q| q.slipping).map(|q| q.p * q.w).sum();
    let mean = fit_bins.iter().sum::<f64>() / BINS as f64;
    let (wall_thin, wall_thick) = inp.walls();
    Ok(Analysis {
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
    })
}
