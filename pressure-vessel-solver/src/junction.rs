//! Axisymmetric finite-element check of the shell-to-head junction under internal pressure (#234).
//!
//! A cylinder of inside radius `R_i` and wall `t` meets a hemispherical or ellipsoidal head of the same wall. Membrane
//! theory gives the two parts different radial growth, so the junction carries a bending moment and a shear that decay
//! away from it (the discontinuity stress that the UG-32 code-rule thickness in [`crate::heads`] does not see).
//! This module meshes the wall as a solid of revolution (Quad9 axisymmetric elements of `fea-core`, refined towards
//! the junction), solves it, and linearises the stresses through the wall at every station of the meridian into
//! membrane and bending parts.
//!
//! Geometry: the mid-surface is the cylinder `r = R_i + t/2` joined tangentially to a quarter ellipse of semi-axes
//! `(R_i + t/2, h + t/2)` (`h` the inside depth; a hemisphere is `h = R_i`); the wall is the constant-thickness offset of the
//! mid-surface, so the inside of an ellipsoidal head is the offset curve of the ellipse and differs from the exact inside
//! ellipse by a term of order `t^2`. The far end of the cylinder is closed-end loaded (axial traction `p R_i^2 / (R_o^2 -
//! R_i^2)`), which is the exact Lame state, so it adds no end disturbance; the cylinder is long enough (six decay lengths
//! `1/beta`) that the junction does not feel it.
//!
//! The thin-shell reference [`cylinder_hemisphere_reference`] is the textbook beam-on-elastic-foundation (Geckeler)
//! solution for the equal-wall cylinder-hemisphere junction, derived in its doc comment from the standard edge-load
//! response of a semi-infinite cylinder.

use fea_core::analysis::Model;
use fea_core::element::ElementKind;
use fea_core::generate::grid;
use fea_core::loads::{Loads, SurfaceLoad};
use fea_core::mesh::{Elastic, Physics};
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;

/// Head shape (inside dimensions; same wall as the cylinder).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeadShape {
    /// Hemispherical head: inside depth equals the inside radius.
    Hemispherical,
    /// Ellipsoidal head of the given inside depth `h` (2:1 is `h = R_i / 2`).
    Ellipsoidal { inside_depth: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JunctionInput {
    pub inside_radius: f64,
    pub thickness: f64,
    pub head: HeadShape,
    /// Internal pressure (same units as `youngs_modulus`).
    pub pressure: f64,
    pub youngs_modulus: f64,
    pub poisson: f64,
    /// Mesh level, at least 1: `2 * refinement` elements through the wall and element sizes along the meridian
    /// proportional to `1 / refinement`. Level 2 is a good default; the convergence tests run 1, 2, 4.
    pub refinement: usize,
}

/// Linearised stresses at one station of the meridian, in shell components `[meridional, hoop]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    /// Arc length along the mid-surface from the junction: negative on the cylinder, positive on the head.
    pub s: f64,
    /// Mid-surface position `(r, z)`; the junction is at `z = 0`.
    pub position: [f64; 2],
    /// Through-wall average (membrane).
    pub membrane: [f64; 2],
    /// Linear bending part at the outside surface (the inside surface is the negative).
    pub bending: [f64; 2],
    /// Inside / outside surface stress from the linearisation, `membrane -/+ bending`.
    pub inside: [f64; 2],
    pub outside: [f64; 2],
    /// Normal stress at the inside surface (`-p` there; a check of the load).
    pub inside_normal: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JunctionSolution {
    /// Stations from the far end of the cylinder to the crown, in order of `s`.
    pub stations: Vec<Station>,
    /// Decay length `1/beta` of the cylinder, `beta = (3 (1 - nu^2))^(1/4) / sqrt(R_m t)`.
    pub decay_length: f64,
    /// Radial displacement of the mid-surface at the junction.
    pub junction_radial_displacement: f64,
    pub n_elements: usize,
    pub n_dofs: usize,
}

impl JunctionSolution {
    /// The station at the junction (`s = 0`).
    pub fn junction(&self) -> &Station {
        self.stations.iter().min_by(|a, b| a.s.abs().total_cmp(&b.s.abs())).expect("a solution has stations")
    }

    /// The station at the far end of the cylinder (membrane state).
    pub fn far_cylinder(&self) -> &Station {
        &self.stations[0]
    }

    /// The station on the axis of the head.
    pub fn crown(&self) -> &Station {
        self.stations.last().expect("a solution has stations")
    }

    /// Largest magnitude of the *bending* meridional stress over the cylinder side, with its `s` (the discontinuity
    /// peak; the code-rule check compares this against its allowable as a secondary stress).
    pub fn peak_bending_meridional(&self) -> (f64, f64) {
        self.stations.iter().map(|st| (st.bending[0], st.s)).max_by(|a, b| a.0.abs().total_cmp(&b.0.abs())).expect("a solution has stations")
    }

    /// Largest meridional surface stress (inside or outside) over the whole meridian, with its `s`.
    pub fn peak_meridional_surface(&self) -> (f64, f64) {
        self.stations
            .iter()
            .flat_map(|st| [(st.inside[0], st.s), (st.outside[0], st.s)])
            .max_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
            .expect("a solution has stations")
    }
}

/// Why the check could not run.
#[derive(Debug, Clone, PartialEq)]
pub enum JunctionError {
    InvalidInput(&'static str),
    /// The finite-element kernel refused or failed.
    Solver(String),
}

impl std::fmt::Display for JunctionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JunctionError::InvalidInput(s) => write!(f, "{s}"),
            JunctionError::Solver(s) => write!(f, "finite-element solve failed: {s}"),
        }
    }
}

impl std::error::Error for JunctionError {}

fn positive(v: f64, what: &'static str) -> Result<f64, JunctionError> {
    if v.is_finite() && v > 0.0 { Ok(v) } else { Err(JunctionError::InvalidInput(what)) }
}

/// Mid-surface meridian: cylinder for `s < 0`, quarter ellipse for `s >= 0`. Returns the position `(r, z)` and the
/// outward unit normal.
struct Meridian {
    rm: f64,
    a: f64,
    b: f64,
    /// Ellipse angle `phi` against arc length, `(s, phi)`, increasing.
    table: Vec<(f64, f64)>,
}

impl Meridian {
    fn new(rm: f64, b: f64) -> Self {
        let a = rm;
        const N: usize = 20_000;
        let mut table = Vec::with_capacity(N + 1);
        let (mut s, mut prev) = (0.0, (a, 0.0));
        table.push((0.0, 0.0));
        for i in 1..=N {
            let phi = FRAC_PI_2 * i as f64 / N as f64;
            let p = (a * phi.cos(), b * phi.sin());
            s += ((p.0 - prev.0).powi(2) + (p.1 - prev.1).powi(2)).sqrt();
            table.push((s, phi));
            prev = p;
        }
        Self { rm, a, b, table }
    }

    fn head_arc(&self) -> f64 {
        self.table.last().expect("table").0
    }

    fn phi(&self, s: f64) -> f64 {
        let i = self.table.partition_point(|e| e.0 < s).clamp(1, self.table.len() - 1);
        let (s0, p0) = self.table[i - 1];
        let (s1, p1) = self.table[i];
        p0 + (p1 - p0) * ((s - s0) / (s1 - s0)).clamp(0.0, 1.0)
    }

    /// `(point, outward normal)` of the mid-surface at arc length `s`.
    fn at(&self, s: f64) -> ([f64; 2], [f64; 2]) {
        if s <= 0.0 {
            return ([self.rm, s], [1.0, 0.0]);
        }
        let phi = self.phi(s);
        let n = [phi.cos() / self.a, phi.sin() / self.b];
        let len = n[0].hypot(n[1]);
        ([self.a * phi.cos(), self.b * phi.sin()], [n[0] / len, n[1] / len])
    }
}

/// Element edge lengths growing geometrically from `h0` by `ratio` up to `hmax`, scaled to fill `length` exactly.
fn graded_sizes(length: f64, h0: f64, ratio: f64, hmax: f64) -> Vec<f64> {
    let mut sizes = Vec::new();
    let (mut h, mut total) = (h0, 0.0);
    while total + 0.5 * h < length {
        sizes.push(h);
        total += h;
        h = (h * ratio).min(hmax);
    }
    if sizes.is_empty() {
        sizes.push(length);
        return sizes;
    }
    let scale = length / total;
    sizes.iter().map(|x| x * scale).collect()
}

/// Solve the junction and linearise the stresses along the meridian.
pub fn solve_junction(inp: JunctionInput) -> Result<JunctionSolution, JunctionError> {
    let ri = positive(inp.inside_radius, "the inside radius must be positive")?;
    let t = positive(inp.thickness, "the thickness must be positive")?;
    let p = positive(inp.pressure, "the pressure must be positive")?;
    let e = positive(inp.youngs_modulus, "Young's modulus must be positive")?;
    if !(inp.poisson.is_finite() && (0.0..0.5).contains(&inp.poisson)) {
        return Err(JunctionError::InvalidInput("Poisson's ratio must be in [0, 0.5)"));
    }
    if inp.refinement == 0 {
        return Err(JunctionError::InvalidInput("the refinement level must be at least 1"));
    }
    let depth = match inp.head {
        HeadShape::Hemispherical => ri,
        HeadShape::Ellipsoidal { inside_depth } => positive(inside_depth, "the head depth must be positive")?,
    };
    if depth > ri {
        return Err(JunctionError::InvalidInput("a head deeper than a hemisphere is not supported"));
    }
    if t > 0.5 * ri {
        return Err(JunctionError::InvalidInput("the wall must be thinner than R_i / 2 (thin-shell geometry)"));
    }
    let rm = ri + 0.5 * t;
    let k = inp.refinement as f64;
    let beta = (3.0 * (1.0 - inp.poisson * inp.poisson)).powf(0.25) / (rm * t).sqrt();
    let mer = Meridian::new(rm, depth + 0.5 * t);

    // Abscissae along the meridian: junction at s = 0, graded towards both ends.
    let h0 = 0.5 * t / k;
    let hmax = 0.5 * (rm * t).sqrt() / k;
    let cyl = graded_sizes(6.0 / beta, h0, 1.2, hmax);
    let head = graded_sizes(mer.head_arc(), h0, 1.2, hmax);
    let mut s_nodes = vec![0.0];
    let mut acc = 0.0;
    for h in &head {
        acc += h;
        s_nodes.push(acc);
    }
    let mut acc = 0.0;
    for h in &cyl {
        acc -= h;
        s_nodes.insert(0, acc);
    }
    let n_mer = s_nodes.len() - 1;
    let n_thk = 2 * inp.refinement;
    let along = |q: f64| -> f64 {
        let x = (q * n_mer as f64).clamp(0.0, n_mer as f64);
        let i = (x.floor() as usize).min(n_mer - 1);
        s_nodes[i] + (s_nodes[i + 1] - s_nodes[i]) * (x - i as f64)
    };
    let place = |q0: f64, q1: f64| -> [f64; 3] {
        let (c, n) = mer.at(along(q1));
        let off = t * (q0 - 0.5);
        [c[0] + off * n[0], c[1] + off * n[1], 0.0]
    };

    let mesh = grid(Physics::Axisymmetric, ElementKind::Quad9, Elastic::new(e, inp.poisson), [n_thk, n_mer, 1], &|q| place(q[0], q[1])).map_err(JunctionError::Solver)?;
    let n_elements = mesh.n_elems();
    let model = Model::new(mesh).map_err(JunctionError::Solver)?;
    let mut bc = model.dirichlet();
    let axis = model.mesh.node_set("v1").map_err(JunctionError::Solver)?.to_vec();
    for &n in &axis {
        bc.fix(n, 0, 0.0); // r = 0 on the axis of the head
    }
    bc.fix(axis[0], 1, 0.0); // rigid axial translation; the loads are self-equilibrated so this reaction is ~0
    let ro = ri + t;
    let end_stress = p * ri * ri / (ro * ro - ri * ri);
    let mut loads = Loads::default();
    for f in &model.mesh.surfaces["u0"] {
        loads.faces.push((f.clone(), SurfaceLoad::Pressure(p)));
    }
    for f in &model.mesh.surfaces["v0"] {
        loads.faces.push((f.clone(), SurfaceLoad::Traction([0.0, -end_stress, 0.0])));
    }
    let sol = model.solve_static(&loads, &bc).map_err(JunctionError::Solver)?;
    let nodal = model.nodal_stresses(&sol.u, 0.0).map_err(JunctionError::Solver)?;

    // Locate the node of each (through-wall index, meridian index) by its mapped position.
    let key = |x: f64, y: f64| -> (i64, i64) { ((x / t * 1e7).round() as i64, (y / t * 1e7).round() as i64) };
    let mut at: HashMap<(i64, i64), usize> = HashMap::new();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        at.insert(key(x[0], x[1]), i);
    }
    let nt2 = 2 * n_thk; // quadratic: nodes per line - 1
    let nm2 = 2 * n_mer;
    let mut stations = Vec::with_capacity(nm2 + 1);
    let mut junction_u = 0.0;
    for j in 0..=nm2 {
        let q1 = j as f64 / nm2 as f64;
        let s = along(q1);
        let (c, nrm) = mer.at(s);
        let tan = [-nrm[1], nrm[0]];
        let mut line = Vec::with_capacity(nt2 + 1);
        for i in 0..=nt2 {
            let q0 = i as f64 / nt2 as f64;
            let x = place(q0, q1);
            let node = *at.get(&key(x[0], x[1])).ok_or_else(|| JunctionError::Solver("a mesh node was not found at its mapped position".into()))?;
            let st = nodal[node];
            let sm = st[0] * tan[0] * tan[0] + st[1] * tan[1] * tan[1] + 2.0 * st[3] * tan[0] * tan[1];
            let sn = st[0] * nrm[0] * nrm[0] + st[1] * nrm[1] * nrm[1] + 2.0 * st[3] * nrm[0] * nrm[1];
            line.push((t * (q0 - 0.5), sm, st[2], sn));
        }
        // Simpson over the equally spaced nodes (`nt2` is even).
        let dz = t / nt2 as f64;
        let (mut nm, mut nh, mut mm, mut mh) = (0.0, 0.0, 0.0, 0.0);
        for (i, &(z, sm, sh, _)) in line.iter().enumerate() {
            let w = if i == 0 || i == nt2 { 1.0 } else if i % 2 == 1 { 4.0 } else { 2.0 } * dz / 3.0;
            nm += w * sm;
            nh += w * sh;
            mm += w * sm * z;
            mh += w * sh * z;
        }
        let membrane = [nm / t, nh / t];
        let bending = [6.0 * mm / (t * t), 6.0 * mh / (t * t)];
        if s.abs() < 1e-9 * t {
            let mid = at[&key(place(0.5, q1)[0], place(0.5, q1)[1])];
            junction_u = sol.u[mid * 2];
        }
        stations.push(Station {
            s,
            position: c,
            membrane,
            bending,
            inside: [membrane[0] - bending[0], membrane[1] - bending[1]],
            outside: [membrane[0] + bending[0], membrane[1] + bending[1]],
            inside_normal: line[0].3,
        });
    }
    Ok(JunctionSolution { stations, decay_length: 1.0 / beta, junction_radial_displacement: junction_u, n_elements, n_dofs: model.mesh.n_dofs() })
}

/// Thin-shell closed form for a cylinder joined to a hemisphere of the same wall under internal pressure `p`, all at
/// mean radius `R` and thickness `t` (Geckeler / Timoshenko beam-on-elastic-foundation theory).
///
/// Derivation (nothing here is a tabulated coefficient). The semi-infinite cylinder under an edge shear `Q0` and moment
/// `M0` per unit circumference has the textbook edge response (`D = E t^3 / (12 (1 - nu^2))`,
/// `beta^4 = 3 (1 - nu^2) / (R^2 t^2)`): deflection `Q0/(2 beta^3 D) + M0/(2 beta^2 D)`, slope `-(Q0/(2 beta^2 D) + M0/(beta D))`,
/// moment `M(x) = e^(-beta x) (Q0 sin(beta x) / beta + M0 (cos(beta x) + sin(beta x)))`. The hemisphere is treated the same way
/// (Geckeler: the bending of a spherical cap near its edge is that of a cylinder of its radius). Membrane theory gives the
/// cylinder the radial growth `p R^2 (1 - nu/2)/(E t)` and the hemisphere `p R^2 (1 - nu)/(2 E t)`, both without rotation.
/// With identical `beta`, `D` the joint is symmetric: `M0 = 0` and the shear closes the growth mismatch
/// `delta = p R^2 / (2 E t)`, so `Q0 = beta^3 D delta`, the moment peaks at `beta x = pi/4` (`e^(-pi/4) sin(pi/4) = 0.32239`)
/// and the surface bending stress is `6 M_max / t^2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HemisphereReference {
    /// Edge shear per unit circumference at the junction.
    pub shear: f64,
    /// Peak bending moment per unit circumference (at `peak_distance` from the junction on either side).
    pub peak_moment: f64,
    pub peak_distance: f64,
    /// Meridional bending surface stress at the peak: `6 M / t^2`.
    pub peak_bending_stress: f64,
    /// Radial displacement of the junction: the mean of the two free membrane growths.
    pub junction_radial_displacement: f64,
    /// Membrane meridional stresses: cylinder `p R / (2 t)` and hemisphere `p R / (2 t)` (equal).
    pub membrane_meridional: f64,
    /// Cylinder membrane hoop stress `p R / t`.
    pub cylinder_hoop: f64,
}

pub fn cylinder_hemisphere_reference(mean_radius: f64, thickness: f64, pressure: f64, youngs_modulus: f64, poisson: f64) -> HemisphereReference {
    let (r, t, p, e, nu) = (mean_radius, thickness, pressure, youngs_modulus, poisson);
    let d = e * t.powi(3) / (12.0 * (1.0 - nu * nu));
    let beta = (3.0 * (1.0 - nu * nu)).powf(0.25) / (r * t).sqrt();
    let delta = p * r * r / (2.0 * e * t);
    let shear = beta.powi(3) * d * delta;
    let peak_moment = shear / beta * (-std::f64::consts::FRAC_PI_4).exp() * std::f64::consts::FRAC_PI_4.sin();
    HemisphereReference {
        shear,
        peak_moment,
        peak_distance: std::f64::consts::FRAC_PI_4 / beta,
        peak_bending_stress: 6.0 * peak_moment / (t * t),
        junction_radial_displacement: 0.5 * (p * r * r * (1.0 - 0.5 * nu) / (e * t) + p * r * r * (1.0 - nu) / (2.0 * e * t)),
        membrane_meridional: p * r / (2.0 * t),
        cylinder_hoop: p * r / t,
    }
}
