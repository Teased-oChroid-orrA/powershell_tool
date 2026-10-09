//! Adjoint sensitivities against global central finite differences of the full solve (the oracle needs no theory).

use fea_core::generate::grid;
use fea_core::loads::SurfaceLoad;
use fea_core::mesh::Block;
use fea_core::*;

/// A cantilever plate of two blocks (left and right halves) so that block parameters differ.
fn plate(density: f64) -> Mesh {
    let mut mesh = grid(Physics::PlaneStress { thickness: 0.5 }, ElementKind::Quad9, Elastic::new(1.0e7, 0.3), [6, 2, 1], &|p| [3.0 * p[0], 1.0 * p[1], 0.0]).unwrap();
    let b = mesh.blocks.remove(0);
    let nn = b.kind.n_nodes();
    let half = b.conn.len() / 2;
    let mk = |conn: Vec<usize>, e: f64, name: &str| Block { kind: b.kind, conn, material: Elastic::new(e, 0.3), name: name.into(), plasticity: None, density, thermal: None };
    mesh.blocks.push(mk(b.conn[..half].to_vec(), 1.0e7, "left"));
    mesh.blocks.push(mk(b.conn[half..].to_vec(), 2.0e7, "right"));
    assert_eq!(half % nn, 0);
    mesh
}

fn setup(mesh: Mesh) -> (Model, Dirichlet, Loads) {
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let tip = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - 3.0).abs() < 1e-12 && model.mesh.nodes[i][1] < 1e-12).unwrap();
    // A point load, a pressure on the top edge (its load vector depends on the geometry) and a traction on the free end.
    let top = model.mesh.surfaces["v1"].clone();
    let end = model.mesh.surfaces["u1"].clone();
    let mut loads = Loads { nodal: vec![(tip, [0.0, -200.0, 0.0])], ..Loads::default() };
    loads.faces = top.into_iter().map(|f| (f, SurfaceLoad::Pressure(80.0))).collect();
    loads.faces.extend(end.into_iter().map(|f| (f, SurfaceLoad::Traction([30.0, 0.0, 0.0]))));
    (model, bc, loads)
}

fn perturb(mesh: &Mesh, p: Param, s: f64) -> Mesh {
    let mut m = mesh.clone();
    match p {
        Param::ModulusScale(b) => m.blocks[b].material.e *= 1.0 + s,
        Param::DensityScale(b) => m.blocks[b].density *= 1.0 + s,
        Param::Coord { node, axis } => m.nodes[node][axis] += s,
        Param::Thickness => {
            m.physics = match m.physics {
                Physics::PlaneStress { thickness } => Physics::PlaneStress { thickness: thickness + s },
                other => other,
            }
        }
    }
    m
}

fn fd(mesh: &Mesh, p: Param, step: f64, response: &dyn Fn(&Mesh) -> f64) -> f64 {
    (response(&perturb(mesh, p, step)) - response(&perturb(mesh, p, -step))) / (2.0 * step)
}

fn params(mesh: &Mesh) -> Vec<(Param, f64)> {
    let interior = (0..mesh.nodes.len()).find(|&i| (mesh.nodes[i][0] - 1.5).abs() < 1e-12 && (mesh.nodes[i][1] - 0.5).abs() < 1e-12).unwrap();
    let top_mid = (0..mesh.nodes.len()).find(|&i| (mesh.nodes[i][0] - 1.5).abs() < 1e-12 && (mesh.nodes[i][1] - 1.0).abs() < 1e-12).unwrap();
    let tip_top = (0..mesh.nodes.len()).find(|&i| (mesh.nodes[i][0] - 3.0).abs() < 1e-12 && (mesh.nodes[i][1] - 1.0).abs() < 1e-12).unwrap();
    vec![
        (Param::ModulusScale(0), 1e-5),
        (Param::ModulusScale(1), 1e-5),
        (Param::Thickness, 1e-6),
        (Param::Coord { node: interior, axis: 0 }, 1e-6),
        (Param::Coord { node: interior, axis: 1 }, 1e-6),
        (Param::Coord { node: top_mid, axis: 1 }, 1e-6), // a loaded-edge node: the pressure load vector moves with it
        (Param::Coord { node: tip_top, axis: 0 }, 1e-6),
    ]
}

fn solve(mesh: &Mesh) -> (Model, Vec<f64>) {
    let (model, bc, loads) = setup(mesh.clone());
    let u = model.solve_static(&loads, &bc).unwrap().u;
    (model, u)
}

#[test]
fn a_displacement_sensitivity_matches_the_global_finite_difference() {
    let mesh = plate(1.0);
    let (model, bc, loads) = setup(mesh.clone());
    let tip = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - 3.0).abs() < 1e-12 && (model.mesh.nodes[i][1] - 0.5).abs() < 1e-12).unwrap();
    let dof = 2 * tip + 1;
    let mut grad = vec![0.0; model.mesh.n_dofs()];
    grad[dof] = 1.0;
    let list = params(&mesh);
    let adj = model.sensitivities(&loads, &bc, &grad, &list.iter().map(|(p, _)| *p).collect::<Vec<_>>()).unwrap();
    for ((p, step), a) in list.iter().zip(&adj) {
        let num = fd(&mesh, *p, *step, &|m| solve(m).1[dof]);
        let scale = num.abs().max(1e-12);
        assert!((a - num).abs() < 2e-4 * scale, "{p:?}: adjoint {a:e} vs finite difference {num:e}");
    }
}

#[test]
fn a_compliance_sensitivity_matches_the_global_finite_difference() {
    let mesh = plate(1.0);
    let (model, bc, loads) = setup(mesh.clone());
    let list = params(&mesh);
    let adj = model.compliance_sensitivities(&loads, &bc, &list.iter().map(|(p, _)| *p).collect::<Vec<_>>()).unwrap();
    let compliance = |m: &Mesh| {
        let (model, bc, loads) = setup(m.clone());
        let sol = model.solve_static(&loads, &bc).unwrap();
        let f = fea_core::loads::assemble(&model.mesh, &loads).unwrap();
        f.iter().zip(&sol.u).map(|(a, b)| a * b).sum::<f64>()
    };
    let _ = bc;
    for ((p, step), a) in list.iter().zip(&adj) {
        let num = fd(&mesh, *p, *step, &compliance);
        assert!((a - num).abs() < 2e-4 * num.abs().max(1e-12), "{p:?}: adjoint {a:e} vs finite difference {num:e}");
    }
}

#[test]
fn a_frequency_sensitivity_matches_the_global_finite_difference() {
    let mesh = plate(2.0);
    let (model, bc, _) = setup(mesh.clone());
    let modal = model.modal(&bc, &ModalOptions { n_modes: 2, ..ModalOptions::default() }).unwrap();
    let mut list = params(&mesh);
    list.push((Param::DensityScale(0), 1e-5));
    list.push((Param::DensityScale(1), 1e-5));
    let sens = model.eigenvalue_sensitivities(&modal.modes[0], &list.iter().map(|(p, _)| *p).collect::<Vec<_>>()).unwrap();
    let lambda = |m: &Mesh| {
        let (model, bc, _) = setup(m.clone());
        model.modal(&bc, &ModalOptions { n_modes: 1, tol: 1e-12, ..ModalOptions::default() }).unwrap().modes[0].omega2
    };
    for ((p, step), a) in list.iter().zip(&sens) {
        if *p == Param::Thickness {
            // K and M both scale with the thickness: the frequencies do not depend on it (the finite difference is round-off).
            assert!(a.abs() < 1e-6 * modal.modes[0].omega2, "{a:e}");
            continue;
        }
        let num = fd(&mesh, *p, *step, &lambda);
        assert!((a - num).abs() < 5e-4 * num.abs().max(1e-12), "{p:?}: analytic {a:e} vs finite difference {num:e}");
    }
}

#[test]
fn thickness_is_refused_outside_plane_analyses_and_density_has_no_static_effect() {
    let mut mesh = grid(Physics::Solid, ElementKind::Hex8, Elastic::new(1.0e7, 0.3), [2, 2, 2], &|p| p).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("w0").unwrap() {
        bc.fix_node(n);
    }
    let grad = vec![1.0; model.mesh.n_dofs()];
    let loads = Loads { body: Some([0.0, 0.0, -1.0]), ..Loads::default() };
    assert!(model.sensitivities(&loads, &bc, &grad, &[Param::Thickness]).is_err());
    assert_eq!(model.sensitivities(&loads, &bc, &grad, &[Param::DensityScale(0)]).unwrap(), vec![0.0]);
}
