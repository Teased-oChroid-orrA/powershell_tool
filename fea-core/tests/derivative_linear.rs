//! Level 3 and 5 of the derivative suite: element matrices and their exact derivatives, and linear static sensitivities through
//! the policy-selected direct or adjoint solve, on pure and on hybrid (Quad + Tri) meshes.

use fea_core::derivative::dual::Dual;
use fea_core::derivative::element;
use fea_core::derivative::linear_sens::*;
use fea_core::derivative::scalar::Scalar;
use fea_core::derivative::SensitivityMethod;
use fea_core::generate::grid;
use fea_core::kernel::{self, Work};
use fea_core::loads::SurfaceLoad;
use fea_core::mesh::Block;
use fea_core::sensitivity::Param;
use fea_core::*;

fn skewed(kind: ElementKind) -> Vec<[f64; 3]> {
    // reference coordinates of the corners, then a non-affine perturbation so that no element is a box
    let (n, d) = (kind.n_nodes(), kind.dim());
    let ref_nodes = match kind {
        ElementKind::Tri3 => vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        _ => Vec::new(),
    };
    let _ = ref_nodes;
    // use the element's own natural coordinates of its nodes via a probe: shape(xi) = e_a at node a is not exposed, so take the Gauss
    // points' physical positions instead: a map from the reference element given by a smooth distortion of the natural coordinates
    let t = kind.table();
    let _ = (n, t);
    let mut nodes = Vec::new();
    for a in 0..kind.n_nodes() {
        let xi = node_natural(kind, a);
        let x = [xi[0] * 1.3 + 0.15 * xi[1] + 0.05 * xi[0] * xi[1], if d >= 2 { xi[1] * 0.9 - 0.08 * xi[0] + 0.04 * xi[1] * xi[1] } else { 0.0 }, if d == 3 { xi[2] * 1.1 + 0.06 * xi[0] * xi[2] } else { 0.0 }];
        nodes.push(x);
    }
    nodes
}

/// Natural coordinates of the nodes of the library's elements, recovered from the shape functions (the node a where N_a = 1).
fn node_natural(kind: ElementKind, a: usize) -> [f64; 3] {
    // search a grid of natural coordinates for the point where N_a is 1 and the rest vanish
    let d = kind.dim();
    let steps: Vec<f64> = (0..=4).map(|i| i as f64 * 0.5 - 1.0).collect();
    let mut best = ([0.0; 3], f64::INFINITY);
    let range_z: Vec<f64> = if d == 3 { steps.clone() } else { vec![0.0] };
    for &x in &steps {
        for &y in &steps {
            for &z in &range_z {
                let xi = [x, y, z];
                let (n, _) = kind.shape(xi);
                let err: f64 = (0..kind.n_nodes()).map(|b| (n[b] - if b == a { 1.0 } else { 0.0 }).powi(2)).sum();
                if err < best.1 {
                    best = (xi, err);
                }
            }
        }
    }
    if best.1 > 1e-20 {
        // simplex elements have nodes at 0, 1/2, 1
        let steps: Vec<f64> = (0..=2).map(|i| i as f64 * 0.5).collect();
        for &x in &steps {
            for &y in &steps {
                for &z in &(if d == 3 { steps.clone() } else { vec![0.0] }) {
                    let xi = [x, y, z];
                    let (n, _) = kind.shape(xi);
                    let err: f64 = (0..kind.n_nodes()).map(|b| (n[b] - if b == a { 1.0 } else { 0.0 }).powi(2)).sum();
                    if err < best.1 {
                        best = (xi, err);
                    }
                }
            }
        }
    }
    assert!(best.1 < 1e-20, "{kind:?}: node {a} not found");
    best.0
}

#[test]
fn the_generic_element_stiffness_reproduces_the_kernel_for_every_element_kind() {
    let mat = Elastic::new(2.9e7, 0.31);
    let mut work = Work::new();
    for kind in [ElementKind::Tri3, ElementKind::Tri6, ElementKind::Quad4, ElementKind::Quad8, ElementKind::Quad9, ElementKind::Tet4, ElementKind::Tet10, ElementKind::Hex8, ElementKind::Hex20, ElementKind::Wedge6, ElementKind::Wedge15] {
        let physics_list: Vec<Physics> = if kind.dim() == 2 { vec![Physics::PlaneStress { thickness: 0.3 }, Physics::PlaneStrain { thickness: 0.3 }, Physics::Axisymmetric] } else { vec![Physics::Solid] };
        for physics in physics_list {
            let mut xyz = skewed(kind);
            if matches!(physics, Physics::Axisymmetric) {
                for p in xyz.iter_mut() {
                    p[0] += 2.0; // away from the axis
                }
            }
            let nn = kind.n_nodes();
            let d = kind.dim();
            let mut ke = vec![0.0; nn * nn * d * d];
            kernel::stiffness(kind, physics, &mat, &xyz, &mut work, &mut ke).unwrap();
            let coords: Vec<[f64; 3]> = xyz.clone();
            let thickness = match physics {
                Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
                _ => 1.0,
            };
            let gen = element::stiffness::<f64>(kind, physics, &coords, mat.e, mat.nu, thickness).unwrap();
            let nd = nn * d;
            let scale = ke.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            for a in 0..nn {
                for b in 0..nn {
                    for i in 0..d {
                        for j in 0..d {
                            let (want, got) = (ke[(a * nn + b) * d * d + i * d + j], gen[(a * d + i) * nd + b * d + j]);
                            assert!((want - got).abs() < 1e-11 * scale, "{kind:?} {physics:?} K[({a},{i}),({b},{j})]: kernel {want} vs generic {got}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_exact_element_derivatives_agree_with_central_differences_of_the_kernel_and_beat_them_at_a_bad_step() {
    type D = Dual<f64, 1>;
    let mat = Elastic::new(2.9e7, 0.31);
    let physics = Physics::PlaneStress { thickness: 0.3 };
    let kind = ElementKind::Quad8;
    let xyz = skewed(kind);
    let (nn, d) = (kind.n_nodes(), kind.dim());
    let nd = nn * d;
    let kernel_ke = |x: &[[f64; 3]]| -> Vec<f64> {
        let mut ke = vec![0.0; nn * nn * d * d];
        kernel::stiffness(kind, physics, &mat, x, &mut Work::new(), &mut ke).unwrap();
        // to (a d + i) (nd) + b d + j layout
        let mut out = vec![0.0; nd * nd];
        for a in 0..nn {
            for b in 0..nn {
                for i in 0..d {
                    for j in 0..d {
                        out[(a * d + i) * nd + b * d + j] = ke[(a * nn + b) * d * d + i * d + j];
                    }
                }
            }
        }
        out
    };
    for (node, axis) in [(0usize, 0usize), (3, 1), (5, 0), (7, 1)] {
        let mut coords: Vec<[D; 3]> = xyz.iter().map(|p| p.map(D::cst)).collect();
        coords[node][axis] = D::var(xyz[node][axis], 0);
        let ke = element::stiffness(kind, physics, &coords, D::cst(mat.e), D::cst(mat.nu), D::cst(0.3)).unwrap();
        let scale = ke.iter().fold(0.0f64, |m, v| m.max(v.v.abs()));
        let exact: Vec<f64> = ke.iter().map(|v| v.d[0]).collect();
        let fd = |h: f64| -> Vec<f64> {
            let (mut xp, mut xm) = (xyz.clone(), xyz.clone());
            xp[node][axis] += h;
            xm[node][axis] -= h;
            kernel_ke(&xp).iter().zip(kernel_ke(&xm)).map(|(a, b)| (a - b) / (2.0 * h)).collect()
        };
        let gap = |v: &[f64]| v.iter().zip(&exact).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / (exact.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300));
        let _ = scale;
        assert!(gap(&fd(1e-5)) < 1e-6, "node {node} axis {axis}: central difference at 1e-5 differs by {:e}", gap(&fd(1e-5)));
        // the difference quotient is limited by its step either way: too large (truncation) or too small (cancellation); the exact one is not
        assert!(gap(&fd(1e-12)) > 1e-5 && gap(&fd(1e-1)) > 1e-5, "a bad step must show: {:e} / {:e}", gap(&fd(1e-12)), gap(&fd(1e-1)));
    }
}

/// A cantilever of two blocks with a point load, a pressure on the top edge and a traction on the free end (loads that move with the
/// geometry), as in `tests/sensitivity.rs`.
fn plate() -> Mesh {
    let mut mesh = grid(Physics::PlaneStress { thickness: 0.5 }, ElementKind::Quad9, Elastic::new(1.0e7, 0.3), [6, 2, 1], &|p| [3.0 * p[0], 1.0 * p[1], 0.0]).unwrap();
    let b = mesh.blocks.remove(0);
    let half = b.conn.len() / 2;
    let mk = |conn: Vec<usize>, e: f64, name: &str| Block { kind: b.kind, conn, material: Elastic::new(e, 0.3), name: name.into(), plasticity: None, density: 0.0, thermal: None };
    mesh.blocks.push(mk(b.conn[..half].to_vec(), 1.0e7, "left"));
    mesh.blocks.push(mk(b.conn[half..].to_vec(), 2.0e7, "right"));
    mesh
}

fn setup(mesh: Mesh) -> (Model, Dirichlet, Loads) {
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let tip = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - 3.0).abs() < 1e-12 && model.mesh.nodes[i][1] < 1e-12).unwrap();
    let mut loads = Loads { nodal: vec![(tip, [0.0, -200.0, 0.0])], ..Loads::default() };
    loads.faces = model.mesh.surfaces["v1"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(80.0))).collect();
    loads.faces.extend(model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([30.0, 0.0, 0.0]))));
    (model, bc, loads)
}

fn perturb(mesh: &Mesh, p: LinearParam, s: f64) -> Mesh {
    let mut m = mesh.clone();
    match p {
        LinearParam::ModulusScale(b) => m.blocks[b].material.e *= 1.0 + s,
        LinearParam::PoissonRatio(b) => m.blocks[b].material.nu += s,
        LinearParam::Coord { node, axis } => m.nodes[node][axis] += s,
        LinearParam::Thickness => {
            if let Physics::PlaneStress { thickness } = m.physics {
                m.physics = Physics::PlaneStress { thickness: thickness + s };
            }
        }
        LinearParam::ThermalExpansion(b) => m.blocks[b].material.alpha *= 1.0 + s,
        LinearParam::UniformTemperature => unreachable!("a load parameter, not a mesh one"),
    }
    m
}

fn qois(model: &Model) -> Vec<Vec<f64>> {
    let n = model.mesh.n_dofs();
    let tip = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - 3.0).abs() < 1e-12 && model.mesh.nodes[i][1] < 1e-12).unwrap();
    let mid = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - 1.5).abs() < 1e-12 && (model.mesh.nodes[i][1] - 1.0).abs() < 1e-12).unwrap();
    let mut a = vec![0.0; n];
    a[tip * 2 + 1] = 1.0;
    let mut b = vec![0.0; n];
    b[mid * 2] = 1.0;
    b[mid * 2 + 1] = 0.5;
    vec![a, b]
}

/// Central difference with one Richardson step: `(4 D(h / 2) - D(h)) / 3` removes the `h^2` truncation term, so a moderate step is
/// accurate and far from the rounding floor of the solve.
fn richardson(f: &dyn Fn(f64) -> f64, h: f64) -> f64 {
    let d = |h: f64| (f(h) - f(-h)) / (2.0 * h);
    (4.0 * d(0.5 * h) - d(h)) / 3.0
}

fn response(mesh: &Mesh, p: LinearParam, s: f64, q: &[Vec<f64>]) -> Vec<f64> {
    let (model, bc, loads) = setup(perturb(mesh, p, s));
    let sol = model.solve_static(&loads, &bc).map(|x| x.u).unwrap_or_else(|_| model.solve_static(&loads, &bc).unwrap().u);
    let _ = &bc;
    q.iter().map(|g| g.iter().zip(&sol).map(|(a, b)| a * b).sum()).collect()
}

#[test]
fn exact_linear_sensitivities_match_finite_differences_and_direct_equals_adjoint() {
    let mesh = plate();
    let (model, bc, loads) = setup(mesh.clone());
    let q = qois(&model);
    let node_at = |x: f64, y: f64| (0..mesh.nodes.len()).find(|&i| (mesh.nodes[i][0] - x).abs() < 1e-12 && (mesh.nodes[i][1] - y).abs() < 1e-12).unwrap();
    let params = [
        LinearParam::ModulusScale(0),
        LinearParam::ModulusScale(1),
        LinearParam::PoissonRatio(0),
        LinearParam::PoissonRatio(1),
        LinearParam::Thickness,
        LinearParam::Coord { node: node_at(1.5, 0.5), axis: 0 },
        LinearParam::Coord { node: node_at(1.5, 1.0), axis: 1 },
        LinearParam::Coord { node: node_at(3.0, 1.0), axis: 1 },
        LinearParam::Coord { node: node_at(3.0, 0.5), axis: 0 },
    ];
    let engine = LinearSensitivityEngine::new(&model, &loads, &bc);
    let (direct, rd) = engine.sensitivities(&q, &params, Some(SensitivityMethod::Direct)).unwrap();
    let (adjoint, ra) = engine.sensitivities(&q, &params, Some(SensitivityMethod::DiscreteAdjoint)).unwrap();
    let (auto, rauto) = engine.sensitivities(&q, &params, None).unwrap();
    assert_eq!((rd.method, ra.method), (SensitivityMethod::Direct, SensitivityMethod::DiscreteAdjoint));
    assert!(rd.tangent_solves == params.len() + 1 && ra.tangent_solves == q.len() + 1, "{rd:?} {ra:?}");
    // two outputs, nine parameters: the policy picks the adjoint
    assert_eq!(rauto.method, SensitivityMethod::DiscreteAdjoint, "{rauto:?}");
    // existing semi-analytic engine, for the modulus / thickness / coordinate parameters it supports
    for qi in 0..q.len() {
        for pi in 0..params.len() {
            let (d, a, u) = (direct[qi][pi], adjoint[qi][pi], auto[qi][pi]);
            let scale = d.abs().max(a.abs()).max(1e-300);
            assert!((d - a).abs() < 1e-10 * scale, "q{qi} p{pi}: direct {d:e} vs adjoint {a:e}");
            assert_eq!(a, u, "auto = the adjoint it chose");
            let h = match params[pi] {
                LinearParam::Coord { .. } => 4e-3,
                LinearParam::Thickness => 4e-3,
                LinearParam::ModulusScale(_) => 4e-2,
                LinearParam::PoissonRatio(_) => 4e-3,
                LinearParam::ThermalExpansion(_) | LinearParam::UniformTemperature => unreachable!("not in this test's parameter list"),
            };
            let fd = richardson(&|s| response(&mesh, params[pi], s, &q)[qi], h);
            assert!((a - fd).abs() < 1e-6 * fd.abs().max(a.abs()).max(1e-12), "q{qi} {:?}: exact {a:e} vs finite difference {fd:e}", params[pi]);
        }
    }
    // the older semi-analytic engine agrees to the accuracy of its own coordinate difference
    let old_params: Vec<Param> = vec![Param::ModulusScale(0), Param::ModulusScale(1), Param::Thickness, Param::Coord { node: node_at(1.5, 0.5), axis: 0 }];
    let new_idx = [0usize, 1, 4, 5];
    for (qi, g) in q.iter().enumerate() {
        let old = model.sensitivities(&loads, &bc, g, &old_params).unwrap();
        for (k, &ni) in new_idx.iter().enumerate() {
            let scale = adjoint[qi][ni].abs().max(1e-12);
            let gap = (old[k] - adjoint[qi][ni]).abs() / scale;
            eprintln!("q{qi} {:?}: semi-analytic engine differs from the exact one by {gap:.2e}", old_params[k]);
            // the exact one matches Richardson finite differences to 1e-6 (above); the central difference of the element kernels in
            // `sensitivity.rs` is good to about 1e-3 for a node coordinate and exact for the linear parameters
            let allowed = if matches!(old_params[k], Param::Coord { .. }) { 1e-3 } else { 1e-9 };
            assert!(gap < allowed, "q{qi} {:?}: semi-analytic {:e} vs exact {:e}", old_params[k], old[k], adjoint[qi][ni]);
        }
    }
}

#[test]
fn shape_sensitivities_work_on_a_hybrid_quad_tri_mesh_including_the_interface_nodes() {
    use fea_core::geometry::{Loop, Region};
    use fea_core::meshing::hybrid::{mesh_hybrid_2d, ElementFamily, HybridOptions2d, Strategy};
    let region = Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()], Elastic::new(1.0e7, 0.3)).unwrap();
    let h = mesh_hybrid_2d(&region, &|_| 0.5, &HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Quadratic)).unwrap();
    assert_eq!(h.report.strategy, Strategy::Hybrid);
    let mesh = h.mesh;
    let kinds: std::collections::BTreeSet<_> = mesh.blocks.iter().map(|b| format!("{:?}", b.kind)).collect();
    assert!(kinds.len() >= 2, "a hybrid mesh has two element kinds: {kinds:?}");
    let model = Model::new(mesh.clone()).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 2.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())).unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    // nodes shared by a Quad and a Tri element (the interface), plus a hole-edge node and a free interior one
    let mut kinds_at: std::collections::HashMap<usize, std::collections::BTreeSet<usize>> = Default::default();
    for (bi, blk) in mesh.blocks.iter().enumerate() {
        for &n in &blk.conn {
            kinds_at.entry(n).or_default().insert(bi);
        }
    }
    let mut interface: Vec<usize> = kinds_at.iter().filter(|(_, b)| b.len() > 1).map(|(n, _)| *n).collect();
    interface.sort_unstable();
    assert!(interface.len() > 4);
    let hole_node = model.mesh.node_set("hole").unwrap()[0];
    let picks = [interface[0], interface[interface.len() / 2], hole_node];
    let n = model.mesh.n_dofs();
    let tip = *model.mesh.node_set("right").unwrap().iter().max_by(|&&a, &&b| model.mesh.nodes[a][1].total_cmp(&model.mesh.nodes[b][1])).unwrap();
    let mut g = vec![0.0; n];
    g[tip * 2] = 1.0;
    let params: Vec<LinearParam> = picks.iter().flat_map(|&nd| [LinearParam::Coord { node: nd, axis: 0 }, LinearParam::Coord { node: nd, axis: 1 }]).chain([LinearParam::ModulusScale(0)]).collect();
    let engine = LinearSensitivityEngine::new(&model, &loads, &bc);
    let (adj, _) = engine.sensitivities(&[g.clone()], &params, Some(SensitivityMethod::DiscreteAdjoint)).unwrap();
    for (pi, &p) in params.iter().enumerate() {
        let resp = |s: f64| {
            let (m, b, l) = (Model::new(perturb(&mesh, p, s)).unwrap(), bc.clone(), loads.clone());
            let u = m.solve_static(&l, &b).unwrap().u;
            u[tip * 2]
        };
        let fd = richardson(&resp, 4e-3);
        let scale = adj[0].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!((adj[0][pi] - fd).abs() < 1e-5 * scale, "{p:?}: exact {:e} vs finite difference {fd:e} (largest sensitivity {scale:e})", adj[0][pi]);
    }
}

#[test]
fn the_generic_geometric_stiffness_reproduces_the_kernel_and_its_coordinate_and_stress_derivatives_are_exact() {
    for kind in [ElementKind::Quad4, ElementKind::Tri6, ElementKind::Quad8] {
        let xyz = skewed(kind);
        let t = kind.table();
        let physics = Physics::PlaneStress { thickness: 0.7 };
        let stress: Vec<[f64; 6]> = (0..t.ngp).map(|g| [120.0 + 7.0 * g as f64, -40.0 + 3.0 * g as f64, 0.0, 25.0 - 2.0 * g as f64, 0.0, 0.0]).collect();
        let mut work = Work::new();
        let mut kref = vec![0.0; kind.n_nodes() * kind.n_nodes()];
        kernel::geometric_stiffness(kind, physics, &stress, &xyz, &mut work, &mut kref).unwrap();
        let plain = element::geometric_stiffness(kind, physics, &xyz, &stress, 0.7).unwrap();
        // the kernel's weights already contain the thickness
        let scale = kref.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        for (a, b) in kref.iter().zip(&plain) {
            assert!((a - b).abs() < 1e-12 * scale, "{kind:?}: kernel {a} vs generic {b}");
        }
        // derivative with respect to the x coordinate of node 1 and with respect to the xx stress of Gauss point 0
        type D = Dual<f64, 2>;
        let mut cd: Vec<[D; 3]> = xyz.iter().map(|p| p.map(D::cst)).collect();
        cd[1][0] = D::var(xyz[1][0], 0);
        let mut sd: Vec<[D; 6]> = stress.iter().map(|s| s.map(D::cst)).collect();
        sd[0][0] = D::var(stress[0][0], 1);
        let kd = element::geometric_stiffness(kind, physics, &cd, &sd, D::cst(0.7)).unwrap();
        let fd = |h: f64, which: usize| -> Vec<f64> {
            let (mut xp, mut xm) = (xyz.clone(), xyz.clone());
            let (mut sp, mut sm) = (stress.clone(), stress.clone());
            if which == 0 {
                xp[1][0] += h;
                xm[1][0] -= h;
            } else {
                sp[0][0] += h;
                sm[0][0] -= h;
            }
            let (kp, km) = (element::geometric_stiffness(kind, physics, &xp, &sp, 0.7).unwrap(), element::geometric_stiffness(kind, physics, &xm, &sm, 0.7).unwrap());
            kp.iter().zip(&km).map(|(a, b)| (a - b) / (2.0 * h)).collect()
        };
        for which in 0..2 {
            let r: Vec<f64> = fd(1e-4, which).iter().zip(fd(0.5e-4, which)).map(|(a, b)| (4.0 * b - a) / 3.0).collect();
            for (i, v) in r.iter().enumerate() {
                let exact = kd[i].d[which];
                assert!((exact - v).abs() < 1e-6 * scale, "{kind:?} direction {which} entry {i}: exact {exact} vs difference {v}");
            }
        }
    }
}
