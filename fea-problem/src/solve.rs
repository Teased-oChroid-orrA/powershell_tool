//! Problem -> results: boundary conditions and loads from names, solve, stress recovery, summary,
//! optional adaptive remeshing.

use crate::build::{imported_mesh, sketch_mesh};
use crate::problem::{Analysis, Geometry, Load, Problem, Support};
use fea_core::adapt::{adapted_size_field, AdaptOptions};
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
}

impl Field {
    pub const ALL: [Field; 11] = [Field::VonMises, Field::Displacement, Field::MaxPrincipal, Field::MinPrincipal, Field::Sxx, Field::Syy, Field::Szz, Field::Sxy, Field::Ux, Field::Uy, Field::Uz];

    pub fn label(self) -> &'static str {
        match self {
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
    /// `|reaction + applied| / max(|applied|, |reaction|)` over the components that must balance.
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
}

pub struct Solved {
    pub problem: Problem,
    pub model: Model,
    pub u: Vec<f64>,
    /// `[xx, yy, zz, xy, yz, zx]` per node (SPR-recovered, averaged where SPR does not apply).
    pub nodal: Vec<[f64; 6]>,
    pub summary: Summary,
    pub history: Vec<PassInfo>,
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
            }
        })
        .collect()
}

impl Solved {
    /// Values of `field` at every node.
    pub fn node_values(&self, field: Field) -> Vec<f64> {
        field_values(self.model.mesh.dim(), &self.u, &self.nodal, field)
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
        write(
            &self.model.mesh,
            &[
                VtkField { name: "displacement", ncomp: 3, data: &disp },
                VtkField { name: "von_mises", ncomp: 1, data: &vm },
                VtkField { name: "max_principal", ncomp: 1, data: &p1 },
                VtkField { name: "min_principal", ncomp: 1, data: &p3 },
                VtkField { name: "stress_xx_yy_zz_xy_yz_zx", ncomp: 6, data: &stress },
            ],
            &[],
        )
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
fn named_nodes(mesh: &Mesh, name: &str) -> Result<Vec<usize>, String> {
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
fn named_faces(mesh: &mut Mesh, name: &str) -> Result<Vec<Vec<usize>>, String> {
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

fn nearest_node(mesh: &Mesh, x: [f64; 3]) -> usize {
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
                if !(total > 0.0) {
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
                if !(mag > 0.0) {
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
                if !(r > 0.0) {
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
    solve_ms: f64,
    strain_energy: f64,
    interfaces: Vec<InterfaceResult>,
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

fn solve_mesh(p: &Problem, mut mesh: Mesh) -> Result<Pass, String> {
    let bc = apply_supports(&mesh, &p.supports)?;
    let loads = build_loads(&mut mesh, p)?;
    if !p.bushings.is_empty() {
        return solve_mesh_contact(p, mesh, loads, bc);
    }
    let model = Model::new(mesh)?;
    let t0 = std::time::Instant::now();
    let sol = model.solve_static(&loads, &bc).map_err(|e| {
        if e.to_lowercase().contains("pivot") || e.to_lowercase().contains("positive") || e.to_lowercase().contains("singular") {
            format!("the model is not fully constrained (rigid-body motion remains): add supports. Solver said: {e}")
        } else {
            e
        }
    })?;
    let solve_ms = t0.elapsed().as_secs_f64() * 1e3;
    let d = model.mesh.dim();
    let f = loads::assemble(&model.mesh, &loads)?;
    let (mut reaction, mut applied) = ([0.0; 3], [0.0; 3]);
    for n in 0..model.mesh.nodes.len() {
        for c in 0..d {
            reaction[c] += sol.reactions[n * d + c];
            applied[c] += f[n * d + c];
        }
    }
    let gauss = model.gauss_stresses(&sol.u, p.delta_t)?;
    let nodal = match model.recover_spr(&gauss) {
        Ok(v) => v,
        Err(_) => model.nodal_stresses(&sol.u, p.delta_t)?,
    };
    let zz = model.zz_error(&sol.u, p.delta_t)?;
    let strain_energy = model.strain_energy(&sol.u)?;
    Ok(Pass { model, u: sol.u, nodal, zz, reaction, applied, solve_ms, strain_energy, interfaces: Vec::new() })
}

/// A plane problem with interference-fit bushings: the fit is installed first (frictional contact between each bushing
/// and its hole, the bushings held only by contact and weak springs), then the loads are applied on top of it.
fn solve_mesh_contact(p: &Problem, mesh: Mesh, loads: Loads, bc: Dirichlet) -> Result<Pass, String> {
    let d = mesh.dim();
    let hole_faces: Vec<(usize, Vec<Vec<usize>>, Vec<Vec<usize>>)> = p
        .bushings
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let plate = mesh.surfaces.get(&format!("plate_hole{}", b.hole)).cloned().ok_or_else(|| format!("bushing {}: no plate hole surface", i + 1))?;
            let od = mesh.surfaces.get(&format!("b{}/od", i + 1)).cloned().ok_or_else(|| format!("bushing {}: no outer surface", i + 1))?;
            Ok((b.hole, plate, od))
        })
        .collect::<Result<_, String>>()?;
    // Weak springs on every bushing node keep a lost or frictionless fit from leaving a free body.
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
    let model = Model::new(mesh)?;
    let r_hole: f64 = hole_faces.iter().map(|(h, ..)| match &p.geometry {
        Geometry::Sketch { holes, .. } => holes.get(h - 1).map_or(1.0, |s| s.reach()),
        _ => 1.0,
    }).fold(f64::INFINITY, f64::min);
    let e_min = p.bushings.iter().map(|b| b.material.e).fold(p.material.e, f64::min);
    let tune = Tuning { newton_tol: 1e-6, ..Tuning::friction() };
    let eps_n = tune.eps_n(e_min, r_hole);
    let specs = || -> Vec<fea_core::contact::ContactSpec> {
        let mut v = Vec::new();
        for (b, (_, plate, od)) in p.bushings.iter().zip(&hole_faces) {
            let face = plate.iter().map(|f| f.iter().map(|&n| model.mesh.nodes[n]).fold(0.0f64, |m, x| m.max((x[0] - model.mesh.nodes[f[0]][0]).hypot(x[1] - model.mesh.nodes[f[0]][1])))).fold(0.0f64, f64::max);
            v.extend(interference_contacts(&format!("fit{}", b.hole), plate.clone(), od.clone(), eps_n, b.friction, 0.5 * b.interference, 6.0 * face));
        }
        v
    };
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
    let (mut reaction, mut applied) = ([0.0; 3], [0.0; 3]);
    for n in 0..model.mesh.nodes.len() {
        for c in 0..d {
            reaction[c] += sol.reactions[n * d + c];
            applied[c] += f[n * d + c];
        }
    }
    let gauss = model.gauss_stresses(&sol.u, p.delta_t)?;
    let nodal = match model.recover_spr(&gauss) {
        Ok(v) => v,
        Err(_) => model.nodal_stresses(&sol.u, p.delta_t)?,
    };
    let zz = model.zz_error(&sol.u, p.delta_t)?;
    let strain_energy = model.strain_energy(&sol.u)?;
    let interfaces = p.bushings.iter().enumerate().map(|(i, b)| interface_result(&sol, 2 * i, b.hole, b.friction, match &p.geometry { Geometry::Sketch { holes, .. } => holes[b.hole - 1].centre(), _ => [0.0, 0.0] })).collect();
    Ok(Pass { model, u: sol.u, nodal, zz, reaction, applied, solve_ms, strain_energy, interfaces })
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

fn summarize(p: &Problem, pass: &Pass) -> Summary {
    let mesh = &pass.model.mesh;
    let val = |f| field_values(mesh.dim(), &pass.u, &pass.nodal, f);
    let (vm, disp, p1, p3) = (val(Field::VonMises), val(Field::Displacement), val(Field::MaxPrincipal), val(Field::MinPrincipal));
    let max_vm = extreme(&vm, &mesh.nodes, true);
    // Components that must balance: axisymmetric radial reaction carries the hoop, so only the axis.
    let comps: &[usize] = match mesh.physics {
        Physics::Axisymmetric => &[1],
        Physics::Solid => &[0, 1, 2],
        _ => &[0, 1],
    };
    let mut diff = 0.0f64;
    for &c in comps {
        diff += (pass.reaction[c] + pass.applied[c]).powi(2);
    }
    // A fit alone applies no load: the reactions are then judged against the force the interference fit transmits.
    let fit_scale = 1e-3 * pass.interfaces.iter().map(|f| f.normal_force).sum::<f64>();
    let scale = (0..mesh.dim()).map(|c| pass.reaction[c].abs().max(pass.applied[c].abs())).fold(fit_scale, f64::max);
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
        equilibrium_error: if scale > 0.0 { diff.sqrt() / scale } else { 0.0 },
        strain_energy: pass.strain_energy,
        zz_error: pass.zz.relative(),
        margin: p.material.yield_stress.filter(|_| max_vm.value > 0.0).map(|y| y / max_vm.value - 1.0),
        solve_ms: pass.solve_ms,
        notes: notes(p, mesh),
        interfaces: pass.interfaces.clone(),
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
/// ZZ estimate reaches `target_error`. `import_text` is the file content for an imported mesh.
pub fn solve(p: &Problem, import_text: Option<&str>) -> Result<Solved, String> {
    p.validate()?;
    let first = match &p.geometry {
        Geometry::Sketch { .. } => sketch_mesh(p, None)?,
        Geometry::Imported { .. } => imported_mesh(p, import_text.ok_or("the mesh file has not been read")?)?,
    };
    let adaptive = matches!(p.geometry, Geometry::Sketch { .. }) && p.analysis != Analysis::Solid && p.mesh.adapt_passes > 0 && p.bushings.is_empty();
    let mut history = Vec::new();
    let mut pass = solve_mesh(p, first)?;
    history.push(info(&pass));
    if adaptive {
        for _ in 0..p.mesh.adapt_passes {
            if pass.zz.relative() <= p.mesh.target_error {
                break;
            }
            let order = if p.mesh.element.is_quadratic() { 2.0 } else { 1.0 };
            let opt = AdaptOptions { target_rel_error: p.mesh.target_error, order, h_min: p.mesh.size / 16.0, h_max: p.mesh.size * 2.0, ..AdaptOptions::default() };
            let field = adapted_size_field(&pass.model.mesh, &pass.zz, p.mesh.element.is_quad(), &opt)?;
            let mesh = sketch_mesh(p, Some(&field))?;
            let next = solve_mesh(p, mesh)?;
            // A pass that did not lower the error estimate is dropped (the estimate is unreliable
            // near a singularity, and a finer mesh must never make the answer worse).
            let better = next.zz.relative() < pass.zz.relative();
            history.push(info(&next));
            if better {
                pass = next;
            } else {
                break;
            }
        }
    }
    let summary = summarize(p, &pass);
    Ok(Solved { problem: p.clone(), model: pass.model, u: pass.u, nodal: pass.nodal, summary, history })
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
    }
}

impl std::fmt::Debug for Solved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Solved({}: {} nodes, {} elements)", self.problem.name, self.summary.nodes, self.summary.elements)
    }
}
