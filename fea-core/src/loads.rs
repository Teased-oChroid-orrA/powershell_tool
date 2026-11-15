//! Equivalent nodal loads: point forces, surface tractions / pressure, body force, thermal.
//!
//! Boundary faces list their nodes counter-clockwise seen from outside the body (2D: walking
//! along the edge keeps the body on the left), so `(dx/dxi x dx/deta)` in 3D and `(ty, -tx)` in 2D
//! is the outward normal. `Pressure(p)` pushes into the body (force `-p n`).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::kernel::{self, Work};
use crate::mesh::{Mesh, Physics};
use std::sync::Arc;

/// Force per unit area as a function of position and outward unit normal (per unit area x thickness
/// in plane problems), integrated at the Gauss points of each face it is attached to.
#[derive(Clone)]
pub struct FaceField(pub Arc<FaceForce>);

/// `(position, outward normal) -> force per unit area`.
pub type FaceForce = dyn Fn(&[f64; 3], &[f64; 3]) -> [f64; 3] + Send + Sync;

impl FaceField {
    pub fn new(f: impl Fn(&[f64; 3], &[f64; 3]) -> [f64; 3] + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }
}

impl std::fmt::Debug for FaceField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FaceField(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceLoad {
    /// Normal pressure acting into the body.
    Pressure(f64),
    /// Force per unit area (per unit area x thickness in plane problems).
    Traction([f64; 3]),
}

#[derive(Debug, Clone, Default)]
pub struct Loads {
    /// `(node, force)`; in 2D the `z` component is ignored. Axisymmetric: total force over 360 degrees.
    pub nodal: Vec<(usize, [f64; 3])>,
    /// Boundary faces (edges in 2D) with their load.
    pub faces: Vec<(Vec<usize>, SurfaceLoad)>,
    /// Boundary faces loaded by a position-dependent force field (a bearing cosine, a parabolic shear).
    pub field_faces: Vec<(Vec<usize>, FaceField)>,
    /// Body force per unit volume (per unit area in plane problems; times thickness internally).
    pub body: Option<[f64; 3]>,
    /// Body force per unit volume acting on one block only (a pin loaded through its length inside an
    /// assembly of bodies): `(block, force)`.
    pub block_body: Vec<(usize, [f64; 3])>,
    /// Uniform temperature change from the stress-free state.
    pub delta_t: f64,
    /// Temperature change from the stress-free state at every node (a field, e.g. from `Model::solve_heat_steady`); added
    /// to the uniform `delta_t`. Linear analyses only: the nonlinear driver refuses it.
    pub temperature: Option<Vec<f64>>,
    /// Weak grounding springs `(dof, stiffness)` of the nonlinear solver (`dof = node * d + component`): they hold a
    /// body that is otherwise free to move or rotate rigidly (a pin or bushing held only by contact) and leak
    /// `k u` of force to ground, so keep them tiny. Ignored by the linear solvers.
    pub ground: Vec<(usize, f64)>,
    /// Follower pressures `(face nodes, p)`: a pressure that stays normal to the *deformed* surface (positive pushes
    /// into the body). The nonlinear driver applies them with their load stiffness; every linear analysis treats them as
    /// the dead `SurfaceLoad::Pressure` they equal for small displacements.
    pub followers: Vec<(Vec<usize>, f64)>,
}

/// 1D rule (3-point Gauss) for edges.
const GL3: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];

fn line_shape(n_nodes: usize, xi: f64) -> ([f64; 3], [f64; 3]) {
    if n_nodes == 2 {
        ([0.5 * (1.0 - xi), 0.5 * (1.0 + xi), 0.0], [-0.5, 0.5, 0.0])
    } else {
        // VTK order: the two ends, then the midside node.
        ([0.5 * xi * (xi - 1.0), 0.5 * xi * (xi + 1.0), 1.0 - xi * xi], [xi - 0.5, xi + 0.5, -2.0 * xi])
    }
}

fn face_kind(n_nodes: usize) -> Result<ElementKind, String> {
    Ok(match n_nodes {
        3 => ElementKind::Tri3,
        6 => ElementKind::Tri6,
        4 => ElementKind::Quad4,
        8 => ElementKind::Quad8,
        9 => ElementKind::Quad9,
        n => return Err(format!("a boundary face with {n} nodes is not supported")),
    })
}

/// Assemble the global load vector (`node * d + comp`).
pub fn assemble(mesh: &Mesh, loads: &Loads) -> Result<Vec<f64>, String> {
    let d = mesh.dim();
    let mut f = vec![0.0; mesh.n_dofs()];
    for &(node, force) in &loads.nodal {
        if node >= mesh.nodes.len() {
            return Err(format!("point load on missing node {node}"));
        }
        for c in 0..d {
            f[node * d + c] += force[c];
        }
    }
    for (nodes, load) in &loads.faces {
        let force = |_: &[f64; 3], n: &[f64; 3]| match *load {
            SurfaceLoad::Pressure(p) => [-p * n[0], -p * n[1], -p * n[2]],
            SurfaceLoad::Traction(t) => t,
        };
        add_face(mesh, nodes, &force, &mut f)?;
    }
    for (nodes, field) in &loads.field_faces {
        add_face(mesh, nodes, &*field.0, &mut f)?;
    }
    for (nodes, p) in &loads.followers {
        add_face(mesh, nodes, &|_: &[f64; 3], n: &[f64; 3]| [-p * n[0], -p * n[1], -p * n[2]], &mut f)?;
    }
    let mut work = Work::new();
    let mut fe = vec![0.0; crate::element::MAX_NODES * 3];
    let mut xyz = [[0.0f64; 3]; crate::element::MAX_NODES];
    if let Some(t) = &loads.temperature {
        if t.len() != mesh.nodes.len() {
            return Err(format!("the temperature field has {} values for {} nodes", t.len(), mesh.nodes.len()));
        }
    }
    if loads.body.is_some() || loads.delta_t != 0.0 || loads.temperature.is_some() || !loads.block_body.is_empty() {
        for (bi, blk) in mesh.blocks.iter().enumerate() {
            let block_body = loads.block_body.iter().filter(|(b, _)| *b == bi).fold(None, |acc: Option<[f64; 3]>, (_, v)| Some(acc.map_or(*v, |a| [a[0] + v[0], a[1] + v[1], a[2] + v[2]])));
            let nn = blk.kind.n_nodes();
            for conn in blk.conn.chunks_exact(nn) {
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = mesh.nodes[nd];
                }
                let both = match (loads.body, block_body) {
                    (Some(a), Some(b)) => Some([a[0] + b[0], a[1] + b[1], a[2] + b[2]]),
                    (a, b) => a.or(b),
                };
                if let Some(b) = both {
                    kernel::body_load(blk.kind, mesh.physics, b, &xyz[..nn], &mut work, &mut fe).map_err(|e| e.to_string())?;
                    scatter(&mut f, conn, &fe, d);
                }
                if loads.delta_t != 0.0 || loads.temperature.is_some() {
                    let gauss: Vec<f64> = match &loads.temperature {
                        Some(t) => kernel::gauss_temperatures(blk.kind, &conn.iter().map(|&n| t[n]).collect::<Vec<_>>()),
                        None => vec![0.0; blk.kind.table().ngp],
                    };
                    kernel::thermal_load_with(blk.kind, mesh.physics, &blk.material, &|g| loads.delta_t + gauss[g], &xyz[..nn], &mut work, &mut fe).map_err(|e| e.to_string())?;
                    scatter(&mut f, conn, &fe, d);
                }
            }
        }
    }
    Ok(f)
}

fn scatter(f: &mut [f64], conn: &[usize], fe: &[f64], d: usize) {
    for (a, &nd) in conn.iter().enumerate() {
        for i in 0..d {
            f[nd * d + i] += fe[a * d + i];
        }
    }
}

/// Visit the quadrature points of a boundary face (an edge in 2D): `visit(N, position, outward normal, weight)` with the
/// shape values `N` of the face's nodes (in the order given) and a weight that includes the plane thickness or the
/// axisymmetric `2 pi r`.
pub(crate) fn face_points(mesh: &Mesh, nodes: &[usize], mut visit: impl FnMut(&[f64], [f64; 3], [f64; 3], f64)) -> Result<(), String> {
    if let Some(&bad) = nodes.iter().find(|&&n| n >= mesh.nodes.len()) {
        return Err(format!("face refers to missing node {bad}"));
    }
    let d = mesh.dim();
    let nn = nodes.len();
    let scale = match mesh.physics {
        Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
        _ => 1.0,
    };
    let axisym = matches!(mesh.physics, Physics::Axisymmetric);
    let mut at = |n: &[f64], normal: [f64; 3], w: f64| {
        let mut pos = [0.0; 3];
        for a in 0..nn {
            for i in 0..3 {
                pos[i] += n[a] * mesh.nodes[nodes[a]][i];
            }
        }
        let mut wt = w * scale;
        if axisym {
            let r: f64 = (0..nn).map(|a| n[a] * mesh.nodes[nodes[a]][0]).sum();
            wt *= 2.0 * std::f64::consts::PI * r;
        }
        visit(n, pos, normal, wt);
    };
    if d == 2 {
        if nn != 2 && nn != 3 {
            return Err(format!("an edge with {nn} nodes is not supported in 2D"));
        }
        for &(xi, w) in &GL3 {
            let (n, dn) = line_shape(nn, xi);
            let (mut tx, mut ty) = (0.0, 0.0);
            for a in 0..nn {
                tx += dn[a] * mesh.nodes[nodes[a]][0];
                ty += dn[a] * mesh.nodes[nodes[a]][1];
            }
            let len = tx.hypot(ty);
            if len <= 0.0 {
                return Err("zero-length boundary edge".into());
            }
            at(&n[..nn], [ty / len, -tx / len, 0.0], w * len);
        }
    } else {
        let kind = face_kind(nn)?;
        let t = kind.table();
        for g in 0..t.ngp {
            let n = t.n_at(g);
            let dn = t.dn_at(g);
            let (mut a1, mut a2) = ([0.0f64; 3], [0.0f64; 3]);
            for a in 0..nn {
                for i in 0..3 {
                    a1[i] += dn[a * 2] * mesh.nodes[nodes[a]][i];
                    a2[i] += dn[a * 2 + 1] * mesh.nodes[nodes[a]][i];
                }
            }
            let cr = [a1[1] * a2[2] - a1[2] * a2[1], a1[2] * a2[0] - a1[0] * a2[2], a1[0] * a2[1] - a1[1] * a2[0]];
            let area = (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt();
            if area <= 0.0 {
                return Err("degenerate boundary face".into());
            }
            at(n, [cr[0] / area, cr[1] / area, cr[2] / area], t.w[g] * area);
        }
    }
    Ok(())
}

fn add_face<F: Fn(&[f64; 3], &[f64; 3]) -> [f64; 3] + ?Sized>(mesh: &Mesh, nodes: &[usize], force_at: &F, f: &mut [f64]) -> Result<(), String> {
    let d = mesh.dim();
    face_points(mesh, nodes, |n, pos, normal, wt| {
        let force = force_at(&pos, &normal);
        for a in 0..nodes.len() {
            for i in 0..d {
                f[nodes[a] * d + i] += wt * n[a] * force[i];
            }
        }
    })
}

/// The force of a follower pressure `p` on a face at displacements `u` and its derivative: `g(x) = -p int N_a (x_xi x x_eta)`
/// (3D; in 2D `-p t N_a (y_xi, -x_xi)` with the thickness `t`), so the load follows the deformed surface. Returns `(g, J)`
/// over the face's `nn * d` dofs (node-major) with `J = dg/du` row-major. Axisymmetric faces are not supported.
pub fn follower_face(mesh: &Mesh, nodes: &[usize], p: f64, u: &[f64]) -> Result<(Vec<f64>, Vec<f64>), String> {
    if let Some(&bad) = nodes.iter().find(|&&n| n >= mesh.nodes.len()) {
        return Err(format!("face refers to missing node {bad}"));
    }
    let d = mesh.dim();
    let nn = nodes.len();
    let nd = nn * d;
    let scale = match mesh.physics {
        Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
        Physics::Axisymmetric => return Err("follower pressure is not supported for axisymmetric analyses".into()),
        _ => 1.0,
    };
    let x: Vec<[f64; 3]> = nodes.iter().map(|&n| std::array::from_fn(|i| mesh.nodes[n][i] + if i < d { u[n * d + i] } else { 0.0 })).collect();
    let (mut g, mut j) = (vec![0.0; nd], vec![0.0; nd * nd]);
    if d == 2 {
        if nn != 2 && nn != 3 {
            return Err(format!("an edge with {nn} nodes is not supported in 2D"));
        }
        for &(xi, w) in &GL3 {
            let (n, dn) = line_shape(nn, xi);
            let (mut tx, mut ty) = (0.0, 0.0);
            for a in 0..nn {
                tx += dn[a] * x[a][0];
                ty += dn[a] * x[a][1];
            }
            let c = -p * w * scale;
            for a in 0..nn {
                // n da = (t_y, -t_x) dxi: g_a = c N_a (t_y, -t_x).
                g[a * 2] += c * n[a] * ty;
                g[a * 2 + 1] -= c * n[a] * tx;
                for b in 0..nn {
                    j[(a * 2) * nd + b * 2 + 1] += c * n[a] * dn[b]; // d g_x / d y_b
                    j[(a * 2 + 1) * nd + b * 2] -= c * n[a] * dn[b]; // d g_y / d x_b
                }
            }
        }
    } else {
        let kind = face_kind(nn)?;
        let t = kind.table();
        let skew = |v: [f64; 3]| [[0.0, -v[2], v[1]], [v[2], 0.0, -v[0]], [-v[1], v[0], 0.0]];
        for gp in 0..t.ngp {
            let n = t.n_at(gp);
            let dn = t.dn_at(gp);
            let (mut a1, mut a2) = ([0.0f64; 3], [0.0f64; 3]);
            for a in 0..nn {
                for i in 0..3 {
                    a1[i] += dn[a * 2] * x[a][i];
                    a2[i] += dn[a * 2 + 1] * x[a][i];
                }
            }
            let cr = [a1[1] * a2[2] - a1[2] * a2[1], a1[2] * a2[0] - a1[0] * a2[2], a1[0] * a2[1] - a1[1] * a2[0]];
            let c = -p * t.w[gp];
            let (s1, s2) = (skew(a1), skew(a2));
            for a in 0..nn {
                for i in 0..3 {
                    g[a * 3 + i] += c * n[a] * cr[i];
                }
                for b in 0..nn {
                    // d(x_xi x x_eta)/d x_b = -dNxi_b [x_eta]x + dNeta_b [x_xi]x   (d(a x b)/da = -[b]x, d/db = [a]x)
                    for i in 0..3 {
                        for k in 0..3 {
                            j[(a * 3 + i) * nd + b * 3 + k] += c * n[a] * (dn[b * 2 + 1] * s1[i][k] - dn[b * 2] * s2[i][k]);
                        }
                    }
                }
            }
        }
    }
    Ok((g, j))
}
