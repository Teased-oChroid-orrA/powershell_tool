//! Problem -> results: boundary conditions and loads from names, solve, stress recovery, summary,
//! optional adaptive remeshing.

use crate::build::{imported_mesh, sketch_mesh, sketch_mesh_noted, sketch_mesh_sized};
use crate::problem::{AdaptMode, MeshStrategy, Analysis, Geometry, Load, Problem, Support};
use crate::thermal::Study;
use fea_core::adapt::{refine, AdaptOptions};
use fea_core::kernel::von_mises;
use fea_core::loads::{self, FaceField, Loads, SurfaceLoad};
use fea_core::fit::{interference_contacts, start_from, Tuning};
use fea_core::{Dirichlet, ElementKind, Mesh, Model, Physics};

/// A scalar result field, per node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    VonMises,
    Displacement,
    Ux,
    Uy,
    Uz,
    Sxx,
    Syy,
    Szz,
    Sxy,
    MaxPrincipal,
    MinPrincipal,
    /// Steady temperature (thermal studies).
    Temperature,
    /// Equivalent plastic strain (elastic-plastic material).
    PlasticStrain,
}

impl Field {
    pub const ALL: [Field; 11] = [Field::VonMises, Field::Displacement, Field::MaxPrincipal, Field::MinPrincipal, Field::Sxx, Field::Syy, Field::Szz, Field::Sxy, Field::Ux, Field::Uy, Field::Uz];

    pub fn label(self) -> &'static str {
        match self {
            Field::Temperature => "Temperature",
            Field::PlasticStrain => "Equivalent plastic strain",
            Field::VonMises => "von Mises stress",
            Field::Displacement => "Displacement magnitude",
            Field::Ux => "Displacement x",
            Field::Uy => "Displacement y",
            Field::Uz => "Displacement z",
            Field::Sxx => "Stress xx",
            Field::Syy => "Stress yy",
            Field::Szz => "Stress zz",
            Field::Sxy => "Shear stress xy",
            Field::MaxPrincipal => "Max principal stress",
            Field::MinPrincipal => "Min principal stress",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extreme {
    pub value: f64,
    pub at: [f64; 3],
}

/// What one pass of mesh + solve produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassInfo {
    pub nodes: usize,
    pub elements: usize,
    /// ZZ relative energy-norm error estimate.
    pub zz_error: f64,
    pub max_von_mises: f64,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub nodes: usize,
    pub elements: usize,
    pub dofs: usize,
    pub max_displacement: Extreme,
    pub max_von_mises: Extreme,
    pub max_principal: Extreme,
    pub min_principal: Extreme,
    /// Sum of the support reactions (force components).
    pub reaction: [f64; 3],
    /// Sum of the applied loads (including thermal and body loads).
    pub applied: [f64; 3],
    /// `|reaction + applied|` over the total magnitude of the forces, for the components that must balance.
    pub equilibrium_error: f64,
    pub strain_energy: f64,
    pub zz_error: f64,
    /// `yield / max von Mises - 1` when the material has a yield stress.
    pub margin: Option<f64>,
    pub solve_ms: f64,
    /// Things the results should not be trusted without knowing (an element type the kernel documents as poor).
    pub notes: Vec<String>,
    /// The interference fit of every bushing (empty without bushings).
    pub interfaces: Vec<InterfaceResult>,
    /// What the heat solve found (thermal studies).
    pub heat: Option<crate::thermal::HeatSummary>,
    /// The plastic state of an elastic-plastic result.
    pub plastic: Option<PlasticSummary>,
}

/// What yielding did in an elastic-plastic solve.
#[derive(Debug, Clone, PartialEq)]
pub struct PlasticSummary {
    /// Largest equivalent plastic strain at a Gauss point.
    pub max_strain: f64,
    /// Share of the Gauss points that have yielded.
    pub yielded_fraction: f64,
    pub load_steps: usize,
    pub newton_iterations: usize,
}

pub struct Solved {
    pub problem: Problem,
    pub model: Model,
    /// Displacements (all zero in a thermal-only study).
    pub u: Vec<f64>,
    /// Steady temperature at every node (thermal studies).
    pub temperature: Option<Vec<f64>>,
    /// Equivalent plastic strain at every node (elastic-plastic material).
    pub plastic: Option<Vec<f64>>,
    /// `[xx, yy, zz, xy, yz, zx]` per node (SPR-recovered, averaged where SPR does not apply).
    pub nodal: Vec<[f64; 6]>,
    pub summary: Summary,
    pub history: Vec<PassInfo>,
    /// ZZ error estimate of every element of the final mesh (block order); drives `advice::error_zones`.
    pub zz_eta: Vec<f64>,
}

/// Sorted (descending) principal values of a symmetric stress in the library order.
pub fn principal(s: &[f64; 6]) -> [f64; 3] {
    let (xx, yy, zz, xy, yz, zx) = (s[0], s[1], s[2], s[3], s[4], s[5]);
    let i1 = (xx + yy + zz) / 3.0;
    let (a, b, c) = (xx - i1, yy - i1, zz - i1);
    let p2 = 0.5 * (a * a + b * b + c * c) + xy * xy + yz * yz + zx * zx;
    if p2 <= 1e-300 {
        return [i1; 3];
    }
    let p = (p2 / 3.0).sqrt();
    // B = (S - i1 I) / p ; det(B) / 2 = cos(3 phi)
    let det = a * (b * c - yz * yz) - xy * (xy * c - yz * zx) + zx * (xy * yz - b * zx);
    let r = (det / (p * p * p) * 0.5).clamp(-1.0, 1.0);
    let phi = r.acos() / 3.0;
    let e1 = i1 + 2.0 * p * phi.cos();
    let e3 = i1 + 2.0 * p * (phi + 2.0 * std::f64::consts::FRAC_PI_3).cos();
    [e1, 3.0 * i1 - e1 - e3, e3]
}

/// Values of `field` at every node from displacements `u` (`d` per node) and nodal stresses.
fn field_values(d: usize, u: &[f64], nodal: &[[f64; 6]], field: Field) -> Vec<f64> {
    nodal
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let comp = |c: usize| if c < d { u[i * d + c] } else { 0.0 };
            match field {
                Field::VonMises => von_mises(s),
                Field::Displacement => (0..3).map(|c| comp(c).powi(2)).sum::<f64>().sqrt(),
                Field::Ux => comp(0),
                Field::Uy => comp(1),
                Field::Uz => comp(2),
                Field::Sxx => s[0],
                Field::Syy => s[1],
                Field::Szz => s[2],
                Field::Sxy => s[3],
                Field::MaxPrincipal => principal(s)[0],
                Field::MinPrincipal => principal(s)[2],
                // A temperature is not in these arrays: `Solved::node_values` answers it from the heat solution.
                Field::Temperature | Field::PlasticStrain => 0.0,
            }
        })
        .collect()
}

impl Solved {
    /// Values of `field` at every node.
    pub fn node_values(&self, field: Field) -> Vec<f64> {
        if field == Field::Temperature {
            return self.temperature.clone().unwrap_or_else(|| vec![0.0; self.model.mesh.nodes.len()]);
        }
        if field == Field::PlasticStrain {
            return self.plastic.clone().unwrap_or_else(|| vec![0.0; self.model.mesh.nodes.len()]);
        }
        field_values(self.model.mesh.dim(), &self.u, &self.nodal, field)
    }

    /// The field a viewer opens on: the temperature of a thermal-only study, else the von Mises stress.
    pub fn default_field(&self) -> Field {
        if self.problem.study.is_structural() { Field::VonMises } else { Field::Temperature }
    }

    /// The fields this result has, the one to show first leading: the temperature of a thermal study, the stresses and
    /// displacements of a structural one.
    pub fn fields(&self) -> Vec<Field> {
        let mut v = Vec::new();
        if self.temperature.is_some() {
            v.push(Field::Temperature);
        }
        if self.problem.study.is_structural() {
            v.extend(Field::ALL);
        }
        if self.plastic.is_some() {
            v.insert(1, Field::PlasticStrain);
        }
        v
    }

    /// CSV of the nodal results, one row per node: position, displacement, the six stress components, von Mises and the
    /// principal stresses (a spreadsheet or plotting script reads it where ParaView is not at hand).
    pub fn csv(&self) -> String {
        use std::fmt::Write;
        let d = self.model.mesh.dim();
        let axes = ["x", "y", "z"];
        let mut o = String::from("node");
        for a in &axes[..d] {
            let _ = write!(o, ",{a}");
        }
        for a in &axes[..d] {
            let _ = write!(o, ",u{a}");
        }
        o.push_str(",sxx,syy,szz,sxy,syz,szx,von_mises,max_principal,min_principal");
        if self.temperature.is_some() {
            o.push_str(",temperature");
        }
        o.push('\n');
        let (vm, p1, p3) = (self.node_values(Field::VonMises), self.node_values(Field::MaxPrincipal), self.node_values(Field::MinPrincipal));
        for (i, x) in self.model.mesh.nodes.iter().enumerate() {
            let _ = write!(o, "{i}");
            for c in &x[..d] {
                let _ = write!(o, ",{c:.6e}");
            }
            for c in &self.u[i * d..(i + 1) * d] {
                let _ = write!(o, ",{c:.6e}");
            }
            for c in &self.nodal[i] {
                let _ = write!(o, ",{c:.6e}");
            }
            let _ = write!(o, ",{:.6e},{:.6e},{:.6e}", vm[i], p1[i], p3[i]);
            if let Some(t) = &self.temperature {
                let _ = write!(o, ",{:.6e}", t[i]);
            }
            o.push('\n');
        }
        o
    }

    /// `.vtu` text with the displacement vector and every stress field.
    pub fn vtu(&self) -> Result<String, String> {
        use fea_core::vtu::{pad3, write, Field as VtkField};
        let d = self.model.mesh.dim();
        let disp = pad3(&self.u, d);
        let vm = self.node_values(Field::VonMises);
        let p1 = self.node_values(Field::MaxPrincipal);
        let p3 = self.node_values(Field::MinPrincipal);
        let stress: Vec<f64> = self.nodal.iter().flat_map(|s| s.iter().copied()).collect();
        let temperature = self.temperature.clone().unwrap_or_default();
        let mut fields = vec![
            VtkField { name: "displacement", ncomp: 3, data: &disp },
            VtkField { name: "von_mises", ncomp: 1, data: &vm },
            VtkField { name: "max_principal", ncomp: 1, data: &p1 },
            VtkField { name: "min_principal", ncomp: 1, data: &p3 },
            VtkField { name: "stress_xx_yy_zz_xy_yz_zx", ncomp: 6, data: &stress },
        ];
        if self.temperature.is_some() {
            fields.push(VtkField { name: "temperature", ncomp: 1, data: &temperature });
        }
        write(&self.model.mesh, &fields, &[])
    }
}

// ----------------------------------------------------------------------------- name resolution

fn available(mesh: &Mesh) -> String {
    let v = crate::build::mesh_names(mesh);
    if v.is_empty() {
        "the mesh defines no names".into()
    } else {
        format!("available: {}", v.join(", "))
    }
}

/// Nodes of a named node set, or of the faces of a named surface.
pub(crate) fn named_nodes(mesh: &Mesh, name: &str) -> Result<Vec<usize>, String> {
    if let Some(s) = mesh.node_sets.get(name) {
        return Ok(s.clone());
    }
    if let Some(f) = mesh.surfaces.get(name) {
        let mut v: Vec<usize> = f.iter().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        return Ok(v);
    }
    Err(format!("no edge or set named '{name}' ({})", available(mesh)))
}

/// Boundary faces of a named surface; a node set without a surface gets the boundary faces whose
/// corner nodes all belong to it.
pub(crate) fn named_faces(mesh: &mut Mesh, name: &str) -> Result<Vec<Vec<usize>>, String> {
    if let Some(f) = mesh.surfaces.get(name) {
        return Ok(f.clone());
    }
    if mesh.node_sets.contains_key(name) {
        let faces = mesh.select_faces_by_nodes(name, name)?.to_vec();
        if faces.is_empty() {
            return Err(format!("set '{name}' contains no boundary face to load"));
        }
        return Ok(faces);
    }
    Err(format!("no edge or surface named '{name}' ({})", available(mesh)))
}

pub(crate) fn nearest_node(mesh: &Mesh, x: [f64; 3]) -> usize {
    let dim = mesh.dim();
    (0..mesh.nodes.len())
        .min_by(|&a, &b| {
            let d = |i: usize| (0..dim).map(|c| (mesh.nodes[i][c] - x[c]).powi(2)).sum::<f64>();
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(0)
}

/// Measure of a loaded face: length x thickness (plane), `2 pi int r ds` (axisymmetric), area (3D).
fn face_measure(mesh: &Mesh, face: &[usize]) -> f64 {
    match mesh.physics {
        Physics::Solid => {
            let nc = if matches!(face.len(), 3 | 6) { 3 } else { 4 };
            let p: Vec<[f64; 3]> = face[..nc].iter().map(|&n| mesh.nodes[n]).collect();
            let tri = |a: usize, b: usize, c: usize| {
                let (u, v) = (std::array::from_fn::<f64, 3, _>(|i| p[b][i] - p[a][i]), std::array::from_fn::<f64, 3, _>(|i| p[c][i] - p[a][i]));
                0.5 * ((u[1] * v[2] - u[2] * v[1]).powi(2) + (u[2] * v[0] - u[0] * v[2]).powi(2) + (u[0] * v[1] - u[1] * v[0]).powi(2)).sqrt()
            };
            if nc == 3 {
                tri(0, 1, 2)
            } else {
                tri(0, 1, 2) + tri(0, 2, 3)
            }
        }
        phys => {
            let (a, b) = (mesh.nodes[face[0]], mesh.nodes[face[1]]);
            let mid = face.get(2).map(|&m| mesh.nodes[m]);
            // Gauss-Legendre 3 points on the (possibly quadratic) edge.
            const G: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];
            let mut s = 0.0;
            for (xi, w) in G {
                let (pos, dx): ([f64; 2], [f64; 2]) = match mid {
                    Some(m) => {
                        let (n, dn) = ([0.5 * xi * (xi - 1.0), 0.5 * xi * (xi + 1.0), 1.0 - xi * xi], [xi - 0.5, xi + 0.5, -2.0 * xi]);
                        (std::array::from_fn(|i| n[0] * a[i] + n[1] * b[i] + n[2] * m[i]), std::array::from_fn(|i| dn[0] * a[i] + dn[1] * b[i] + dn[2] * m[i]))
                    }
                    None => (std::array::from_fn(|i| 0.5 * (a[i] + b[i])), std::array::from_fn(|i| 0.5 * (b[i] - a[i]))),
                };
                let weight = match phys {
                    Physics::Axisymmetric => 2.0 * std::f64::consts::PI * pos[0],
                    Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
                    Physics::Solid => 1.0,
                };
                s += w * dx[0].hypot(dx[1]) * weight;
            }
            s
        }
    }
}

// ----------------------------------------------------------------------------- conditions

/// Apply the supports to a Dirichlet set.
pub fn apply_supports(mesh: &Mesh, supports: &[Support]) -> Result<Dirichlet, String> {
    let d = mesh.dim();
    let mut bc = Dirichlet::new(mesh.nodes.len(), d);
    for s in supports {
        let (nodes, comps) = match s {
            Support::Edge { edge, ux, uy, uz } => (named_nodes(mesh, edge)?, [*ux, *uy, *uz]),
            Support::Point { x, y, z, ux, uy, uz } => (vec![nearest_node(mesh, [*x, *y, *z])], [*ux, *uy, *uz]),
        };
        if comps.iter().take(d).all(Option::is_none) {
            return Err("a support fixes no displacement component".into());
        }
        for n in nodes {
            for (c, v) in comps.iter().take(d).enumerate() {
                if let Some(v) = v {
                    bc.fix(n, c, *v);
                }
            }
        }
    }
    Ok(bc)
}

/// Translate the problem's loads. Needs `&mut` because a node set used as a loaded surface gets its
/// boundary faces registered.
pub fn build_loads(mesh: &mut Mesh, p: &Problem) -> Result<Loads, String> {
    let mut out = Loads { delta_t: p.delta_t, ..Loads::default() };
    let d = mesh.dim();
    let mut body = [0.0f64; 3];
    let mut has_body = false;
    for l in &p.loads {
        match l {
            Load::Pressure { edge, p: pr } => {
                for f in named_faces(mesh, edge)? {
                    out.faces.push((f, SurfaceLoad::Pressure(*pr)));
                }
            }
            Load::Traction { edge, tx, ty, tz } => {
                for f in named_faces(mesh, edge)? {
                    out.faces.push((f, SurfaceLoad::Traction([*tx, *ty, *tz])));
                }
            }
            Load::Force { edge, fx, fy, fz } => {
                let faces = named_faces(mesh, edge)?;
                let total: f64 = faces.iter().map(|f| face_measure(mesh, f)).sum();
                if total.is_nan() || total <= 0.0 {
                    return Err(format!("edge '{edge}' has no extent to spread a force over"));
                }
                let t = [fx / total, fy / total, fz / total];
                for f in faces {
                    out.faces.push((f, SurfaceLoad::Traction(t)));
                }
            }
            Load::Bearing { hole, fx, fy } => {
                if d != 2 || p.analysis == Analysis::Axisymmetric {
                    return Err("a bearing load is for plane problems".into());
                }
                let Geometry::Sketch { holes, .. } = &p.geometry else { return Err("a bearing load needs a sketch hole".into()) };
                let shape = holes.get(hole.wrapping_sub(1)).ok_or_else(|| format!("bearing load: there is no hole {hole}"))?;
                let mag = fx.hypot(*fy);
                if mag.is_nan() || mag <= 0.0 {
                    continue;
                }
                // The pin bears on the bore: with a bushing that is the bushing's (possibly offset) bore centre.
                let off = p.bushings.iter().find(|b| b.hole == *hole).map_or([0.0, 0.0], |b| b.offset);
                let (dir, c) = ([fx / mag, fy / mag], [shape.centre()[0] + off[0], shape.centre()[1] + off[1]]);
                let unit = FaceField::new(move |x, _| {
                    let (rx, ry) = (x[0] - c[0], x[1] - c[1]);
                    let r = rx.hypot(ry).max(1e-300);
                    let cos = (rx * dir[0] + ry * dir[1]) / r;
                    let w = cos.max(0.0);
                    [w * dir[0], w * dir[1], 0.0]
                });
                let faces = named_faces(mesh, &format!("hole{hole}"))?;
                // Scale the unit field so its resultant along the load direction is |F|.
                let probe = Loads { field_faces: faces.iter().map(|f| (f.clone(), unit.clone())).collect(), ..Loads::default() };
                let fv = loads::assemble(mesh, &probe)?;
                let r: f64 = (0..fv.len() / d).map(|n| fv[n * d] * dir[0] + fv[n * d + 1] * dir[1]).sum();
                if r.is_nan() || r <= 0.0 {
                    return Err(format!("hole {hole}: the bearing load has nothing to bear on"));
                }
                let s = mag / r;
                let scaled = FaceField::new(move |x, n| {
                    let v = (unit.0)(x, n);
                    [s * v[0], s * v[1], 0.0]
                });
                for f in faces {
                    out.field_faces.push((f, scaled.clone()));
                }
            }
            Load::Point { x, y, z, fx, fy, fz } => {
                out.nodal.push((nearest_node(mesh, [*x, *y, *z]), [*fx, *fy, *fz]));
            }
            Load::Body { bx, by, bz } => {
                body[0] += bx;
                body[1] += by;
                body[2] += bz;
                has_body = true;
            }
        }
    }
    if has_body {
        out.body = Some(body);
    }
    Ok(out)
}

// ----------------------------------------------------------------------------- solve

struct Pass {
    model: Model,
    u: Vec<f64>,
    nodal: Vec<[f64; 6]>,
    zz: fea_core::recover::ZzEstimate,
    reaction: [f64; 3],
    applied: [f64; 3],
    /// Total magnitude of the loads / reactions: the yardstick of the force balance.
    force_scale: f64,
    solve_ms: f64,
    strain_energy: f64,
    interfaces: Vec<InterfaceResult>,
    /// Model-check warnings (the solve ran, but the result needs a caveat), shown with the summary's notes.
    warnings: Vec<String>,
    /// Temperature at every node and the heat balance (thermal studies).
    heat: Option<(Vec<f64>, crate::thermal::HeatSummary)>,
    /// Equivalent plastic strain per node and its summary (elastic-plastic material).
    plastic: Option<PlasticPass>,
}

struct PlasticPass {
    nodal: Vec<f64>,
    summary: PlasticSummary,
}

/// What the interference fit of one bushing did, from the converged contact tractions (integrals, which are reliable:
/// pointwise pressures scatter on non-matching meshes).
#[derive(Debug, Clone)]
pub struct InterfaceResult {
    /// Hole number.
    pub hole: usize,
    /// Mean contact pressure over the interface, and the peak over 5-degree bins.
    pub mean_pressure: f64,
    pub peak_pressure: f64,
    /// Arc over which the interface carries no pressure, degrees.
    pub open_arc_deg: f64,
    /// Torque the friction of the interface can carry at its pressure, `sum mu p r w`.
    pub torque_capacity: f64,
    /// Share of the normal force on points at their friction limit.
    pub slip_share: f64,
    /// Total normal force the interface transmits (`sum p w`, both passes).
    pub normal_force: f64,
}

impl fea_core::adapt::Pass for Pass {
    fn mesh(&self) -> &Mesh {
        &self.model.mesh
    }

    fn zz(&self) -> &fea_core::recover::ZzEstimate {
        &self.zz
    }
}

/// A thermal-only study: the temperature field, with zero displacements and stresses so every consumer of a result keeps
/// working.
fn solve_thermal_only(mesh: Mesh, setup: &crate::thermal::Setup) -> Result<Pass, String> {
    let t0 = std::time::Instant::now();
    let model = Model::new(mesh)?;
    let (heat, summary) = crate::thermal::solve_heat(&model, setup)?;
    let (n, d) = (model.mesh.nodes.len(), model.mesh.dim());
    let zz = fea_core::recover::ZzEstimate { eta: Vec::new(), total: 0.0, norm: 0.0 };
    let solve_ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok(Pass { model, u: vec![0.0; n * d], nodal: vec![[0.0; 6]; n], zz, reaction: [0.0; 3], applied: [0.0; 3], force_scale: 0.0, solve_ms, strain_energy: 0.0, interfaces: Vec::new(), warnings: Vec::new(), heat: Some((heat.temperature, summary)), plastic: None })
}

fn solve_mesh(p: &Problem, mut mesh: Mesh) -> Result<Pass, String> {
    if p.study.is_thermal() && !p.bushings.is_empty() {
        return Err("a thermal study does not support bushings".into());
    }
    let setup = if p.study.is_thermal() { Some(crate::thermal::prepare(p, &mut mesh)?) } else { None };
    if p.study == Study::Thermal {
        return solve_thermal_only(mesh, setup.as_ref().expect("a thermal study has its setup"));
    }
    let bc = apply_supports(&mesh, &p.supports)?;
    let mut loads = build_loads(&mut mesh, p)?;
    if !p.bushings.is_empty() {
        return solve_mesh_contact(p, mesh, loads, bc);
    }
    if p.material.plastic {
        attach_plasticity(&mut mesh, p)?;
        let model = Model::new(mesh)?;
        return solve_plastic_model(p, model, loads, bc);
    }
    let model = Model::new(mesh)?;
    // Thermal stress: the steady temperature on this mesh, as a temperature change from the stress-free state.
    let heat = match &setup {
        Some(setup) => {
            let (heat, summary) = crate::thermal::solve_heat(&model, setup)?;
            loads.temperature = Some(heat.temperature.iter().map(|t| t - p.thermal.reference).collect());
            Some((heat.temperature, summary))
        }
        None => None,
    };
    // The kernel's model check (inverted elements, loose nodes, rigid-body motion, non-finite loads, nothing applied):
    // an error stops the solve with its reason, a warning travels with the result.
    let issues = model.validate(&loads, &bc);
    if let Some(first) = issues.iter().find(|i| i.severity == fea_core::Severity::Error) {
        let all: Vec<&str> = issues.iter().filter(|i| i.severity == fea_core::Severity::Error).map(|i| i.message.as_str()).collect();
        return Err(if all.len() == 1 { first.message.clone() } else { all.join("; ") });
    }
    let warnings: Vec<String> = issues.iter().filter(|i| i.severity == fea_core::Severity::Warning && i.code != "low_order_element").map(|i| i.message.clone()).collect();
    let t0 = std::time::Instant::now();
    let solve = |model: &Model| {
        model.solve_static(&loads, &bc).map_err(|e| {
            if e.to_lowercase().contains("pivot") || e.to_lowercase().contains("positive") || e.to_lowercase().contains("singular") {
                format!("the model is not fully constrained (rigid-body motion remains): add supports. Solver said: {e}")
            } else {
                e
            }
        })
    };
    let mut sol = solve(&model)?;
    let d = model.mesh.dim();
    let f = loads::assemble(&model.mesh, &loads)?;
    let (mut reaction, mut applied) = totals(d, &sol.reactions, &f);
    // The result must pass two checks that do not depend on how it was computed: the equations are satisfied and the
    // reactions balance the loads. A result that fails them is recomputed on one thread, and refused if it still fails:
    // a wrong displacement field must never be presented as a result.
    // The imbalance is judged against the total magnitude of the forces, not against the net force (a thermal load, or a
    // self-equilibrated one, has a net force of zero).
    let mut scale = force_scale(&f, &sol.reactions);
    let imbalance = |reaction: &[f64; 3], applied: &[f64; 3], scale: f64| balance_error(&model.mesh.physics, reaction, applied, scale);
    let mut warnings = warnings;
    if let Some(why) = &sol.fallback {
        warnings.push(format!("The linear solve was repeated: {why}."));
    }
    if !(sol.rel_residual < 1e-6 && imbalance(&reaction, &applied, scale) < 1e-6) {
        let first = (sol.rel_residual, imbalance(&reaction, &applied, scale));
        sol = fea_core::sequentially(|| solve(&model))?;
        (reaction, applied) = totals(d, &sol.reactions, &f);
        scale = force_scale(&f, &sol.reactions);
        let second = (sol.rel_residual, imbalance(&reaction, &applied, scale));
        if !(second.0 < 1e-6 && second.1 < 1e-6) {
            return Err(format!(
                "the solve did not produce a consistent result (residual {:.1e}, force imbalance {:.1e}); the model may be badly conditioned: check units, supports and element shapes",
                second.0, second.1
            ));
        }
        warnings.push(format!("A solve failed its own checks (residual {:.1e}, force imbalance {:.1e}) and was recomputed on one thread; the result shown passed them.", first.0, first.1));
    }
    let solve_ms = t0.elapsed().as_secs_f64() * 1e3;
    let field = loads.temperature.as_deref();
    let gauss = model.gauss_stresses_field(&sol.u, p.delta_t, field)?;
    let nodal = match recover_nodal(&model, &gauss) {
        Ok(v) => v,
        Err(_) => model.nodal_stresses_field(&sol.u, p.delta_t, field)?,
    };
    let mut zz = model.zz_error_field(&sol.u, p.delta_t, field)?;
    if p.mesh.adapt_mode == AdaptMode::Equilibrated && d == 2 {
        zz = equilibrated_estimate(&model, &loads, &bc, &sol.u, &zz, &mut warnings)?;
    }
    let strain_energy = model.strain_energy(&sol.u)?;
    Ok(Pass { model, u: sol.u, nodal, zz, reaction, applied, force_scale: scale, solve_ms, strain_energy, interfaces: Vec::new(), warnings, heat, plastic: None })
}

/// Nodal stresses by SPR recovery with patches that never span two materials. A node on a material interface has a stress from
/// each side; the one with the larger von Mises stress is kept, because the peak stress is what gets checked.
fn recover_nodal(model: &Model, gauss: &[Vec<fea_core::analysis::GaussStress>]) -> Result<Vec<[f64; 6]>, String> {
    let rec = model.recover_spr_by_material(gauss)?;
    let mut out = vec![[0.0; 6]; model.mesh.nodes.len()];
    let mut best = vec![f64::NEG_INFINITY; out.len()];
    for (b, blk) in model.mesh.blocks.iter().enumerate() {
        for &n in &blk.conn {
            let v = rec.at(b, n);
            let vm = von_mises(&v);
            if vm > best[n] {
                best[n] = vm;
                out[n] = v;
            }
        }
    }
    Ok(out)
}

/// The error estimate of a pass from the equilibrated stress reconstruction, in the shape the adaptive loop reads (element
/// energy-norm errors against the energy norm of the recovered stress). A model the reconstruction does not cover is refused
/// with the reason, never estimated some other way; a degraded estimate travels as a caveat.
fn equilibrated_estimate(model: &Model, loads: &Loads, bc: &Dirichlet, u: &[f64], zz: &fea_core::recover::ZzEstimate, warnings: &mut Vec<String>) -> Result<fea_core::recover::ZzEstimate, String> {
    use fea_core::adaptivity::{EquilibratedEstimator, ErrorEstimator, EstimatorContext, Reliability};
    use fea_core::meshing::adjacency::Adjacency;
    let adjacency = Adjacency::new(&model.mesh);
    let ctx = EstimatorContext::new(model, loads, bc, u, &adjacency).map_err(|e| e.to_string())?;
    let field = EquilibratedEstimator.estimate(&ctx).map_err(|e| e.to_string())?;
    match &field.reliability {
        Reliability::Unsupported(why) => Err(format!("the equilibrated error estimate does not cover this model: {why}; use the ZZ error estimate")),
        Reliability::Failed(why) => Err(format!("the equilibrated error estimate failed: {why}")),
        Reliability::Degraded(why) => {
            warnings.push(format!("The equilibrated error estimate is degraded: {why}."));
            Ok(fea_core::recover::ZzEstimate { eta: field.values, total: field.total, norm: zz.norm })
        }
        Reliability::Reliable => Ok(fea_core::recover::ZzEstimate { eta: field.values, total: field.total, norm: zz.norm }),
    }
}

/// A plane problem with interference-fit bushings: the fit is installed first (frictional contact between each bushing
/// and its hole, the bushings held only by contact and weak springs), then the loads are applied on top of it.
/// A bushing's hole index with its bore and outer faces (node lists per face).
type HoleFaces = (usize, Vec<Vec<usize>>, Vec<Vec<usize>>);

/// What the interference fits of a bushed mesh need to be solved: the faces that touch, the weak springs, the penalty.
struct FitSetup {
    hole_faces: Vec<HoleFaces>,
    /// Weak springs on every bushing node keep a lost or frictionless fit from leaving a free body.
    ground: Vec<(usize, f64)>,
    tune: Tuning,
    eps_n: f64,
}

fn fit_setup(p: &Problem, mesh: &Mesh) -> Result<FitSetup, String> {
    let d = mesh.dim();
    let hole_faces: Vec<HoleFaces> = p
        .bushings
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let plate = mesh.surfaces.get(&format!("plate_hole{}", b.hole)).cloned().ok_or_else(|| format!("bushing {}: no plate hole surface", i + 1))?;
            let od = mesh.surfaces.get(&format!("b{}/od", i + 1)).cloned().ok_or_else(|| format!("bushing {}: no outer surface", i + 1))?;
            Ok((b.hole, plate, od))
        })
        .collect::<Result<_, String>>()?;
    let mut ground = Vec::new();
    for (bi, blk) in mesh.blocks.iter().enumerate().skip(1) {
        let k = 1e-9 * p.bushings.get(bi - 1).map_or(p.material.e, |b| b.material.e) * if let Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } = mesh.physics { thickness } else { 1.0 };
        let mut seen = std::collections::BTreeSet::new();
        for &n in &blk.conn {
            if seen.insert(n) {
                ground.extend((0..d).map(|c| (n * d + c, k)));
            }
        }
    }
    let r_hole: f64 = hole_faces.iter().map(|(h, ..)| match &p.geometry {
        Geometry::Sketch { holes, .. } => holes.get(h - 1).map_or(1.0, |s| s.reach()),
        _ => 1.0,
    }).fold(f64::INFINITY, f64::min);
    let e_min = p.bushings.iter().map(|b| b.material.e).fold(p.material.e, f64::min);
    let tune = Tuning { newton_tol: 1e-6, ..Tuning::friction() };
    let eps_n = tune.eps_n(e_min, r_hole);
    Ok(FitSetup { hole_faces, ground, tune, eps_n })
}

/// The contact interfaces of the fits on `mesh`.
fn fit_specs(p: &Problem, mesh: &Mesh, setup: &FitSetup) -> Vec<fea_core::contact::ContactSpec> {
    let mut v = Vec::new();
    for (b, (_, plate, od)) in p.bushings.iter().zip(&setup.hole_faces) {
        let face = plate.iter().map(|f| f.iter().map(|&n| mesh.nodes[n]).fold(0.0f64, |m, x| m.max((x[0] - mesh.nodes[f[0]][0]).hypot(x[1] - mesh.nodes[f[0]][1])))).fold(0.0f64, f64::max);
        v.extend(interference_contacts(&format!("fit{}", b.hole), plate.clone(), od.clone(), setup.eps_n, b.friction, 0.5 * b.interference, 6.0 * face));
    }
    v
}

fn solve_mesh_contact(p: &Problem, mesh: Mesh, loads: Loads, bc: Dirichlet) -> Result<Pass, String> {
    let d = mesh.dim();
    let setup = fit_setup(p, &mesh)?;
    let (ground, tune) = (setup.ground.clone(), setup.tune);
    let model = Model::new(mesh)?;
    // The kernel's model check, minus what a bushed model fails on purpose: the bushing is held by contact alone (a rigid-body
    // finding), and a fit with no applied load is a real case (no-load). Inverted elements, non-finite loads and values and
    // bad materials stop here with their reason instead of minutes into the contact iteration.
    let issues = model.validate(&loads, &bc);
    let blocking: Vec<&str> = issues.iter().filter(|i| i.severity == fea_core::Severity::Error && i.code != "rigid_body").map(|i| i.message.as_str()).collect();
    if !blocking.is_empty() {
        return Err(blocking.join("; "));
    }
    let warnings: Vec<String> = issues.iter().filter(|i| i.severity == fea_core::Severity::Warning && !matches!(i.code, "low_order_element" | "no_load")).map(|i| i.message.clone()).collect();
    let specs = || fit_specs(p, &model.mesh, &setup);
    let t0 = std::time::Instant::now();
    let fit_loads = Loads { ground: ground.clone(), ..Loads::default() };
    let mut sol = model.solve_nonlinear_contact(&fit_loads, &bc, specs(), &tune.options(1)).map_err(|e| format!("the interference fit could not be solved: {e}"))?;
    if !sol.complete() {
        return Err(format!("the interference fit could not be solved: {:?}", sol.stop));
    }
    let has_loads = !loads.nodal.is_empty() || !loads.faces.is_empty() || !loads.field_faces.is_empty() || loads.body.is_some() || loads.delta_t != 0.0;
    if has_loads {
        let full = Loads { ground: ground.clone(), ..loads.clone() };
        let mut opts = Tuning::friction().options(4);
        opts.first_step = 0.5;
        sol = model.solve_nonlinear_contact_from(&full, &bc, specs(), &opts, Some(&start_from(&sol, 0.0))).map_err(|e| format!("the loaded bushing could not be solved: {e}"))?;
        if !sol.complete() {
            return Err(format!("the loaded bushing could not be solved: {:?}", sol.stop));
        }
    }
    let solve_ms = t0.elapsed().as_secs_f64() * 1e3;
    let f = loads::assemble(&model.mesh, &loads)?;
    let (reaction, applied) = totals(d, &sol.reactions, &f);
    let gauss = model.gauss_stresses(&sol.u, p.delta_t)?;
    let nodal = match model.recover_spr(&gauss) {
        Ok(v) => v,
        Err(_) => model.nodal_stresses(&sol.u, p.delta_t)?,
    };
    let zz = model.zz_error(&sol.u, p.delta_t)?;
    let strain_energy = model.strain_energy(&sol.u)?;
    let interfaces = p.bushings.iter().enumerate().map(|(i, b)| interface_result(&sol, 2 * i, b.hole, b.friction, match &p.geometry { Geometry::Sketch { holes, .. } => holes[b.hole - 1].centre(), _ => [0.0, 0.0] })).collect();
    let scale = force_scale(&f, &sol.reactions);
    Ok(Pass { model, u: sol.u, nodal, zz, reaction, applied, force_scale: scale, solve_ms, strain_energy, interfaces, warnings, heat: None, plastic: None })
}

/// The fit interface of one bushing (its two contact passes, indices `first` and `first + 1`) from the tractions.
fn interface_result(sol: &fea_core::NlSolution, first: usize, hole: usize, mu: f64, centre: [f64; 2]) -> InterfaceResult {
    const BINS: usize = 72;
    let (mut force, mut wa, mut wb) = ([0.0f64; BINS], [0.0f64; BINS], [0.0f64; BINS]);
    let (mut normal, mut slip, mut torque, mut length) = (0.0, 0.0, 0.0, 0.0);
    for pass in 0..2 {
        for t in sol.contact_tractions(first + pass, mu) {
            let (dx, dy) = (t.x[0] - centre[0], t.x[1] - centre[1]);
            let bin = (((dy.atan2(dx).to_degrees().rem_euclid(360.0)) / 360.0 * BINS as f64) as usize).min(BINS - 1);
            force[bin] += t.pressure * t.weight;
            if pass == 0 { wa[bin] += t.weight } else { wb[bin] += t.weight }
            normal += t.pressure * t.weight;
            if t.slipping {
                slip += t.pressure * t.weight;
            }
            torque += mu * t.pressure * t.weight * dx.hypot(dy);
            length += t.weight;
        }
    }
    // Both passes' pressures add up to the physical one; per bin that is their force over the mean slave length.
    let p: Vec<f64> = (0..BINS).map(|b| { let w = 0.5 * (wa[b] + wb[b]); if w > 0.0 { force[b] / w } else { 0.0 } }).collect();
    let peak = p.iter().cloned().fold(0.0, f64::max);
    InterfaceResult {
        hole,
        mean_pressure: if length > 0.0 { normal / (0.5 * length) } else { 0.0 },
        peak_pressure: peak,
        open_arc_deg: p.iter().filter(|&&v| v < 1e-3 * peak.max(1e-300)).count() as f64 * 360.0 / BINS as f64,
        torque_capacity: torque,
        slip_share: if normal > 0.0 { slip / normal } else { 0.0 },
        normal_force: normal,
    }
}

fn extreme(values: &[f64], nodes: &[[f64; 3]], pick_max: bool) -> Extreme {
    let mut best = (0usize, if pick_max { f64::NEG_INFINITY } else { f64::INFINITY });
    for (i, &v) in values.iter().enumerate() {
        if v.is_finite() && (if pick_max { v > best.1 } else { v < best.1 }) {
            best = (i, v);
        }
    }
    Extreme { value: if best.1.is_finite() { best.1 } else { 0.0 }, at: nodes.get(best.0).copied().unwrap_or([0.0; 3]) }
}

/// Sum of the reaction and of the load vector over the nodes, per component.
fn totals(d: usize, reactions: &[f64], loads: &[f64]) -> ([f64; 3], [f64; 3]) {
    let (mut reaction, mut applied) = ([0.0; 3], [0.0; 3]);
    for n in 0..reactions.len() / d {
        for c in 0..d {
            reaction[c] += reactions[n * d + c];
            applied[c] += loads[n * d + c];
        }
    }
    (reaction, applied)
}

/// Total magnitude of the forces of a solve (loads and reactions over every dof, whichever is larger): the yardstick of
/// the force balance. The net force cannot serve: a thermal or otherwise self-equilibrated load has a net force of zero.
fn force_scale(loads: &[f64], reactions: &[f64]) -> f64 {
    loads.iter().map(|v| v.abs()).sum::<f64>().max(reactions.iter().map(|v| v.abs()).sum::<f64>())
}

/// `|reaction + applied| / scale` over the force components that must balance: the axial one only for an axisymmetric
/// model (its radial reaction carries the hoop).
fn balance_error(physics: &Physics, reaction: &[f64; 3], applied: &[f64; 3], scale: f64) -> f64 {
    let comps: &[usize] = match physics {
        Physics::Axisymmetric => &[1],
        Physics::Solid => &[0, 1, 2],
        _ => &[0, 1],
    };
    let diff: f64 = comps.iter().map(|&c| (reaction[c] + applied[c]).powi(2)).sum();
    if scale > 0.0 { diff.sqrt() / scale } else { 0.0 }
}

fn summarize(p: &Problem, pass: &Pass) -> Summary {
    let mesh = &pass.model.mesh;
    let val = |f| field_values(mesh.dim(), &pass.u, &pass.nodal, f);
    let (vm, disp, p1, p3) = (val(Field::VonMises), val(Field::Displacement), val(Field::MaxPrincipal), val(Field::MinPrincipal));
    let max_vm = extreme(&vm, &mesh.nodes, true);
    // A fit alone applies no load: the reactions are then judged against the force the interference fit transmits.
    let fit_scale = 1e-3 * pass.interfaces.iter().map(|f| f.normal_force).sum::<f64>();
    let equilibrium_error = balance_error(&mesh.physics, &pass.reaction, &pass.applied, pass.force_scale.max(fit_scale));
    Summary {
        nodes: mesh.nodes.len(),
        elements: mesh.n_elems(),
        dofs: mesh.n_dofs(),
        max_displacement: extreme(&disp, &mesh.nodes, true),
        max_von_mises: max_vm,
        max_principal: extreme(&p1, &mesh.nodes, true),
        min_principal: extreme(&p3, &mesh.nodes, false),
        reaction: pass.reaction,
        applied: pass.applied,
        equilibrium_error,
        strain_energy: pass.strain_energy,
        zz_error: pass.zz.relative(),
        margin: p.material.yield_stress.filter(|_| max_vm.value > 0.0 && !p.has_region_materials()).map(|y| y / max_vm.value - 1.0),
        solve_ms: pass.solve_ms,
        notes: notes(p, mesh).into_iter().chain(pass.warnings.iter().cloned()).collect(),
        interfaces: pass.interfaces.clone(),
        heat: pass.heat.as_ref().map(|h| h.1.clone()),
        plastic: pass.plastic.as_ref().map(|q| q.summary.clone()),
    }
}

/// Caveats the kernel documents for the elements in use.
fn notes(_p: &Problem, mesh: &Mesh) -> Vec<String> {
    let mut out = Vec::new();
    if mesh.blocks.iter().any(|b| matches!(b.kind, ElementKind::Tri3 | ElementKind::Tet4)) {
        out.push("Linear triangles / tetrahedra (constant strain) are poor on pressure and bending problems (the Lame cylinder at 16 x 16 is 2 % off in displacement, and stresses are discontinuous between elements): use a quadratic element for results.".to_string());
    }
    out
}

/// Build, solve and (for 2D sketches with `adapt_passes > 0`) refine until the passes run out or the
/// ZZ estimate meets `target_error` with the fewest unknowns the passes find. `import_text` is the file content for an imported mesh.
pub fn solve(p: &Problem, import_text: Option<&str>) -> Result<Solved, String> {
    if p.cases.is_empty() {
        return solve_resolved(p, import_text);
    }
    // A load case solves its own supports and loads; the result stays tied to the full problem (stale checks, saved files).
    let mut solved = solve_resolved(&p.resolved(), import_text)?;
    solved.problem = p.clone();
    Ok(solved)
}

fn solve_resolved(p: &Problem, import_text: Option<&str>) -> Result<Solved, String> {
    p.validate()?;
    let mut mesh_notes = Vec::new();
    let first = match &p.geometry {
        Geometry::Sketch { .. } => sketch_mesh_noted(p, None, &mut mesh_notes)?,
        Geometry::Imported { .. } => imported_mesh(p, import_text.ok_or("the mesh file has not been read")?)?,
        Geometry::Solid { .. } => crate::solid::solid_mesh(p)?.0,
    };
    let adaptive = matches!(p.geometry, Geometry::Sketch { .. }) && p.analysis != Analysis::Solid && p.mesh.adapt_passes > 0 && p.study.is_structural();
    let goal_oriented = adaptive && p.mesh.adapt_mode == AdaptMode::GoalOriented;
    if goal_oriented && !p.bushings.is_empty() {
        return Err("goal-oriented adaptivity does not support bushings (contact); use the ZZ error estimate".into());
    }
    let mut history = Vec::new();
    if p.material.plastic && goal_oriented {
        return Err("goal-oriented adaptivity is for elastic materials; an elastic-plastic problem adapts on its own recovery-based indicator".into());
    }
    let pass = if p.material.plastic && adaptive {
        solve_plastic_adaptive(p, first, &mut history, &mut mesh_notes)?
    } else if goal_oriented {
        solve_goal_oriented(p, first, &mut history, &mut mesh_notes)?
    } else {
        let first = solve_mesh(p, first)?;
        if adaptive {
            let order = if p.mesh.element.is_quadratic() { 2.0 } else { 1.0 };
            let opt = AdaptOptions { target_rel_error: p.mesh.target_error, order, h_min: p.mesh.size / 16.0, h_max: p.mesh.size * 2.0, ..AdaptOptions::default() };
            refine(first, p.mesh.adapt_passes as usize, p.mesh.element.is_quad(), &opt, |field| solve_mesh(p, sketch_mesh(p, Some(field))?), |pass| history.push(info(pass)))?
        } else {
            history.push(info(&first));
            first
        }
    };
    let mut summary = summarize(p, &pass);
    if p.mesh.bond_regions && p.has_region_materials() {
        mesh_notes.push("Several materials: the yield margin is not given, and at a material interface the stress shown is that of the side with the larger von Mises stress.".into());
    }
    summary.notes.extend(mesh_notes);
    let zz_eta = pass.zz.eta.clone();
    Ok(Solved { problem: p.clone(), model: pass.model, u: pass.u, temperature: pass.heat.map(|h| h.0), plastic: pass.plastic.map(|q| q.nodal), nodal: pass.nodal, summary, history, zz_eta })
}

// ----------------------------------------------------------------------------- elastic-plastic

/// Load steps of an elastic-plastic solve (the loads grow from zero to their full value in this many increments, more where
/// the solver has to cut them).
const PLASTIC_STEPS: usize = 8;

/// J2 plasticity with linear hardening on every block of `mesh`.
fn attach_plasticity(mesh: &mut Mesh, p: &Problem) -> Result<(), String> {
    use fea_core::material::J2;
    let y = p.material.yield_stress.ok_or("an elastic-plastic material needs a yield stress")?;
    // the table runs to a plastic strain of 1, far beyond what a small-strain analysis means
    let law = mechanics_core::hardening::Hardening::linear(y, p.material.hardening, 1.0)?;
    for b in 0..mesh.blocks.len() {
        mesh.set_plasticity(b, J2::small(law))?;
    }
    Ok(())
}

fn plastic_options() -> fea_core::nonlinear::NlOptions {
    fea_core::nonlinear::NlOptions { steps: PLASTIC_STEPS, tol: 1e-8, ..Default::default() }
}

/// Equivalent plastic strain at the nodes: the mean of an element's Gauss values, averaged over the elements at the node.
fn nodal_plastic_strain(mesh: &Mesh, state: &fea_core::nonlinear::NlState) -> Vec<f64> {
    let (mut sum, mut count) = (vec![0.0; mesh.nodes.len()], vec![0.0; mesh.nodes.len()]);
    for (b, blk) in mesh.blocks.iter().enumerate() {
        let ngp = blk.kind.table().ngp;
        if state.gp[b].is_empty() {
            continue;
        }
        for e in 0..blk.n_elems() {
            let mean = (0..ngp).map(|g| state.gp[b][e * ngp + g][6]).sum::<f64>() / ngp as f64;
            for &n in blk.elem(e) {
                sum[n] += mean;
                count[n] += 1.0;
            }
        }
    }
    sum.iter().zip(&count).map(|(s, c)| if *c > 0.0 { s / c } else { 0.0 }).collect()
}

/// Solve the elastic-plastic model (small strain, load control) and read it out like any other pass.
fn solve_plastic_model(p: &Problem, model: Model, loads: Loads, bc: Dirichlet) -> Result<Pass, String> {
    let issues = model.validate(&loads, &bc);
    if let Some(e) = issues.iter().find(|i| i.severity == fea_core::Severity::Error) {
        return Err(e.message.clone());
    }
    let t0 = std::time::Instant::now();
    let sol = model.solve_nonlinear(&loads, &bc, &plastic_options()).map_err(|e| format!("the elastic-plastic solve failed: {e}"))?;
    plastic_pass(p, model, &loads, sol, t0.elapsed().as_secs_f64() * 1e3)
}

fn plastic_pass(p: &Problem, model: Model, loads: &Loads, sol: fea_core::nonlinear::NlSolution, solve_ms: f64) -> Result<Pass, String> {
    use fea_core::nonlinear::{NlState, Stop};
    if sol.stop != Stop::Completed {
        return Err(format!("the elastic-plastic solve stopped at {:.0} % of the load ({:?}): the structure may be at its collapse load, or the load steps could not be completed", 100.0 * sol.lambda, sol.stop));
    }
    let d = model.mesh.dim();
    let f = loads::assemble(&model.mesh, loads)?;
    let (reaction, applied) = totals(d, &sol.reactions, &f);
    let virgin = NlState::new(&model.mesh);
    let old = sol.prev_state.as_ref().unwrap_or(&virgin);
    let gauss = model.nl_gauss_stresses(&sol.u, old)?;
    let nodal = recover_nodal(&model, &gauss)?;
    let zz = fea_core::adaptivity::nl_zz_estimate(&model, &sol).map_err(|e| e.to_string())?;
    let work: f64 = f.iter().zip(&sol.u).map(|(a, b)| a * b).sum();
    let summary = PlasticSummary { max_strain: sol.state.max_ep(), yielded_fraction: sol.state.plastic_fraction(), load_steps: sol.steps.len(), newton_iterations: sol.steps.iter().map(|s| s.iterations).sum() };
    let ep = nodal_plastic_strain(&model.mesh, &sol.state);
    let scale = force_scale(&f, &sol.reactions);
    let warnings = vec!["Elastic-plastic result: small strain, J2 yield with linear hardening; the strain energy shown is the external work f.u / 2, and the nodal stresses are recovered from the converged stress state.".to_string()];
    let _ = p;
    Ok(Pass { model, u: sol.u, nodal, zz, reaction, applied, force_scale: scale, solve_ms, strain_energy: 0.5 * work, interfaces: Vec::new(), warnings, heat: None, plastic: Some(PlasticPass { nodal: ep, summary }) })
}

/// Adaptive remeshing of an elastic-plastic problem (`adaptivity::run_amr_nonlinear`): a recovery-based indicator of the
/// converged plastic state, the external work as the quantity watched from mesh to mesh. `history` gets one entry per mesh.
fn solve_plastic_adaptive(p: &Problem, first: Mesh, history: &mut Vec<PassInfo>, notes: &mut Vec<String>) -> Result<Pass, String> {
    use fea_core::adaptivity::{run_amr_nonlinear, AdaptSize, NlAmrConfig, NlGoal, NlProblem};
    let quad_scale = if p.mesh.element.is_quad() && p.mesh.strategy == MeshStrategy::Unstructured { 0.5 } else { 1.0 };
    let meshed = |mut mesh: Mesh| -> Result<NlProblem, String> {
        attach_plasticity(&mut mesh, p)?;
        let bc = apply_supports(&mesh, &p.supports)?;
        let loads = build_loads(&mut mesh, p)?;
        Ok(NlProblem { model: Model::new(mesh)?, loads, bc, contacts: Vec::new(), labels: None, quad_scale })
    };
    let mut cfg = NlAmrConfig { nl: plastic_options(), max_cycles: p.mesh.adapt_passes as usize, physics_tolerance: Some(p.mesh.target_error), ..NlAmrConfig::default() };
    cfg.adapt.order = if p.mesh.element.is_quadratic() { 2.0 } else { 1.0 };
    cfg.adapt.h_min = p.mesh.size / 16.0;
    cfg.adapt.h_max = p.mesh.size * 2.0;
    let work = NlGoal::new("external work", p.mesh.target_error, |prob, sol| loads::assemble(&prob.model.mesh, &prob.loads).map(|f| f.iter().zip(&sol.u).map(|(a, b)| a * b).sum()).unwrap_or(f64::NAN));
    let mut mesher = |s: &AdaptSize| meshed(sketch_mesh_sized(p, Some(&|x| s.at([x[0], x[1], 0.0])))?);
    let t0 = std::time::Instant::now();
    let result = run_amr_nonlinear(meshed(first)?, &[work], &mut mesher, &cfg).map_err(|e| e.to_string())?;
    for rec in &result.history {
        history.push(PassInfo { nodes: rec.dofs / p.analysis_dim(), elements: rec.elements, zz_error: rec.physics_error, max_von_mises: 0.0 });
    }
    notes.push(format!(
        "Elastic-plastic adaptivity: {} cycle(s), stopped: {:?}; peak plastic strain {:.3e}, {:.0} % of the Gauss points yielded.",
        result.history.len().saturating_sub(1),
        result.stop,
        result.solution.state.max_ep(),
        100.0 * result.solution.state.plastic_fraction()
    ));
    let pass = plastic_pass(p, result.problem.model, &result.problem.loads, result.solution, t0.elapsed().as_secs_f64() * 1e3)?;
    if let Some(last) = history.last_mut() {
        *last = info(&pass);
    }
    Ok(pass)
}

/// Goal-oriented adaptivity (`fea_core::adaptivity::run_amr`) on the compliance of a structural sketch problem: the adjoint
/// (enriched-dual) estimate fused with the ZZ physics criterion at `target_error`, Dorfler marking, a remesh per cycle at the
/// size field the controller asks for. The final mesh is solved again through the ordinary path (`solve_mesh`) so that the
/// result carries every check and summary of any other solve. `history` gets one entry per cycle: the physics error is the
/// controller's; the maximum von Mises stress is known only for the final mesh (0 for the others).
fn solve_goal_oriented(p: &Problem, first: Mesh, history: &mut Vec<PassInfo>, notes: &mut Vec<String>) -> Result<Pass, String> {
    use fea_core::adaptivity::{run_amr, AdaptSize, AmrConfig, GoalSpec, MeshedProblem};
    // every mesh the controller builds, before the loads touch it: the accepted one is solved again through `solve_mesh`
    let raws = std::cell::RefCell::new(Vec::<Mesh>::new());
    // The unstructured quadrilateral mesh is the triangle mesh with every triangle split into quadrilaterals: its edges are about
    // half the size the mesher was asked for. The hybrid mesher keeps quadrilaterals at the requested size.
    let quad_scale = if p.mesh.element.is_quad() && p.mesh.strategy == MeshStrategy::Unstructured { 0.5 } else { 1.0 };
    let meshed = |mut mesh: Mesh| -> Result<MeshedProblem, String> {
        raws.borrow_mut().push(mesh.clone());
        let bc = apply_supports(&mesh, &p.supports)?;
        let loads = build_loads(&mut mesh, p)?;
        Ok(MeshedProblem { model: Model::new(mesh)?, loads, bc, labels: None, quad_scale })
    };
    let order = if p.mesh.element.is_quadratic() { 2.0 } else { 1.0 };
    let mut cfg = AmrConfig { max_cycles: p.mesh.adapt_passes as usize, physics_tolerance: Some(p.mesh.target_error), ..AmrConfig::default() };
    cfg.adapt.order = order;
    cfg.adapt.h_min = p.mesh.size / 16.0;
    cfg.adapt.h_max = p.mesh.size * 2.0;
    // The compliance `f.u = 2 W` is wanted to a relative error of `target_error^2` (the energy-norm error `e` of the physics
    // criterion enters the compliance as `e^2`), judged against the first mesh.
    let compliance = 2.0 * solve_mesh(p, first.clone())?.strain_energy;
    let goals = [GoalSpec::compliance(Some(p.mesh.target_error * p.mesh.target_error * compliance))];
    let mut mesher = |s: &AdaptSize| -> Result<MeshedProblem, String> { meshed(sketch_mesh_sized(p, Some(&|x| s.at([x[0], x[1], 0.0])))?) };
    let result = run_amr(meshed(first)?, &goals, &mut mesher, &cfg).map_err(|e| e.to_string())?;
    if let Some(last) = result.history.last() {
        notes.push(format!(
            "Goal-oriented adaptivity: {} cycle(s), goal estimator {}, physics estimator {}, stopped: {:?}.",
            result.history.len() - 1,
            last.goal_estimator,
            last.physics_estimator,
            result.stop
        ));
    }
    for rec in &result.history {
        // nodes follow from the dofs of a plane problem (2 per node)
        history.push(PassInfo { nodes: rec.dofs / 2, elements: rec.elements, zz_error: rec.physics_error, max_von_mises: 0.0 });
    }
    let accepted = &result.problem.model.mesh.nodes;
    let raw = raws.into_inner().into_iter().rev().find(|m| &m.nodes == accepted).ok_or("the accepted adaptive mesh was not found among the built ones")?;
    let pass = solve_mesh(p, raw)?;
    if let Some(last) = history.last_mut() {
        *last = PassInfo { nodes: pass.model.mesh.nodes.len(), elements: pass.model.mesh.n_elems(), zz_error: pass.zz.relative(), max_von_mises: pass.nodal.iter().map(von_mises).fold(0.0f64, f64::max) };
    }
    Ok(pass)
}

fn info(pass: &Pass) -> PassInfo {
    let vm = pass.nodal.iter().map(von_mises).fold(0.0f64, f64::max);
    PassInfo { nodes: pass.model.mesh.nodes.len(), elements: pass.model.mesh.n_elems(), zz_error: pass.zz.relative(), max_von_mises: vm }
}

/// Element kinds of a mesh, for display.
pub fn kinds(mesh: &Mesh) -> Vec<ElementKind> {
    let mut v: Vec<ElementKind> = mesh.blocks.iter().map(|b| b.kind).collect();
    v.dedup();
    v
}

/// Mesh only (no solve): what the workbench shows while the problem is being set up. Returns the
/// mesh with its names so the editor can offer them.
pub fn preview(p: &Problem, import_text: Option<&str>) -> Result<Mesh, String> {
    p.validate_geometry()?;
    match &p.geometry {
        Geometry::Sketch { .. } => sketch_mesh(p, None),
        Geometry::Imported { .. } => imported_mesh(p, import_text.ok_or("the mesh file has not been read")?),
        Geometry::Solid { .. } => crate::solid::solid_mesh(p).map(|(m, _)| m),
    }
}

/// The body of an extruded, revolved or swept model as a coarse surface mesh, without its analysis mesh: what the viewer
/// shows before anyone has asked for the real mesh. Only the geometry matters (supports, loads, element choice, adaptive
/// passes, refinement zones and the extrusion's layers are dropped); the size follows the part, fine enough that a
/// circle is a smooth polygon. Needs a 3D model built from a sketch.
pub fn body_preview(p: &Problem) -> Result<Mesh, String> {
    use crate::history::Body;
    let Geometry::Sketch { outer, depth, regions, holes, layers } = &p.geometry else { return Err("only a model built from a sketch has a body to show".into()) };
    if p.analysis != Analysis::Solid {
        return Err("a plane model has no body".into());
    }
    let extruded = p.history.as_ref().map_or(true, |h| h.rebuild().is_ok_and(|b| matches!(b.body, Body::Extrude { .. })));
    let (lo, hi) = outer.bounds();
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]).max(*depth).max(1e-12);
    let mut last = String::new();
    for divisions in [36.0, 90.0, 220.0] {
        let mut body = p.clone();
        body.supports.clear();
        body.loads.clear();
        body.bushings.clear();
        body.geometry = Geometry::Sketch { outer: outer.clone(), holes: holes.clone(), depth: *depth, layers: if extruded { 1 } else { *layers }, regions: regions.clone() };
        body.mesh = crate::problem::MeshSpec {
            size: diag / divisions,
            hole_factor: 1.0,
            adapt_passes: 0,
            predict_sizes: false,
            refine: Vec::new(),
            element: crate::problem::ElementChoice::Quad4,
            ..p.mesh.clone()
        };
        match preview(&body, None) {
            Ok(mesh) => return Ok(mesh),
            Err(e) => last = e,
        }
    }
    Err(last)
}

impl std::fmt::Debug for Solved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Solved({}: {} nodes, {} elements)", self.problem.name, self.summary.nodes, self.summary.elements)
    }
}
