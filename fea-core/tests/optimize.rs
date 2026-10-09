//! Sizing optimisation against the closed form of bars in series.

use fea_core::generate::grid;
use fea_core::loads::SurfaceLoad;
use fea_core::mesh::Block;
use fea_core::*;

/// Three bars in series (block moduli `e`, lengths `len`), clamped at x = 0, loaded axially at the free end (nu = 0: one-dimensional).
fn series(e: [f64; 3], len: [f64; 3]) -> (Model, Dirichlet, Loads) {
    let total: f64 = len.iter().sum();
    let mut mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.0), [12, 1, 1], &move |p| [total * p[0], p[1], 0.0]).unwrap();
    let b = mesh.blocks.remove(0);
    let nn = b.kind.n_nodes();
    // 12 elements: split by x position into the three bars.
    let bounds = [len[0], len[0] + len[1], total];
    let mut conns: [Vec<usize>; 3] = Default::default();
    for c in b.conn.chunks_exact(nn) {
        let xc = c.iter().map(|&n| mesh.nodes[n][0]).sum::<f64>() / nn as f64;
        let k = if xc < bounds[0] { 0 } else if xc < bounds[1] { 1 } else { 2 };
        conns[k].extend_from_slice(c);
    }
    for (k, conn) in conns.iter_mut().enumerate() {
        mesh.blocks.push(Block { kind: b.kind, conn: std::mem::take(conn), material: Elastic::new(e[k], 0.0), name: format!("bar{k}"), plasticity: None, density: 0.0, thermal: None });
    }
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let end = model.mesh.surfaces["u1"].clone();
    let loads = Loads { faces: end.into_iter().map(|f| (f, SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    (model, bc, loads)
}

#[test]
fn bars_in_series_get_a_stiffness_inversely_proportional_to_the_root_of_their_modulus() {
    // Minimise sum L_b / (E_b s_b) subject to sum L_b s_b = V: L_b / (E_b s_b^2) = mu L_b, so s_b = c / sqrt(E_b).
    let (e, len) = ([1.0e7, 4.0e7, 9.0e7], [1.0, 1.0, 2.0]);
    let (model, bc, loads) = series(e, len);
    let volume: f64 = len.iter().sum();
    let r = model.optimize_compliance(&loads, &bc, &[0, 1, 2], volume, &OptOptions::default()).unwrap();
    assert!(r.converged, "{} iterations", r.iterations);
    let c = volume / len.iter().zip(&e).map(|(l, e)| l / e.sqrt()).sum::<f64>();
    for k in 0..3 {
        let exact = c / e[k].sqrt();
        assert!((r.scales[k] / exact - 1.0).abs() < 1e-5, "bar {k}: {} vs {exact}", r.scales[k]);
    }
    // The compliance fell monotonically and the budget is met.
    assert!(r.compliance.windows(2).all(|w| w[1] <= w[0] * (1.0 + 1e-9)), "{:?}", r.compliance);
    assert!((r.scales.iter().zip(&len).map(|(s, l)| s * l).sum::<f64>() - volume).abs() < 1e-8 * volume);
    assert!(r.free_sensitivity_spread < 1e-4, "stationary: {}", r.free_sensitivity_spread);
    // And better than the uniform design.
    assert!(r.compliance.last().unwrap() < &(r.compliance[0] * 0.999));
}

#[test]
fn a_bound_that_binds_moves_the_rest_of_the_budget_to_the_other_bars() {
    let (e, len) = ([1.0e7, 4.0e7, 9.0e7], [1.0, 1.0, 2.0]);
    let (model, bc, loads) = series(e, len);
    let volume: f64 = len.iter().sum();
    let r = model.optimize_compliance(&loads, &bc, &[0, 1, 2], volume, &OptOptions { s_max: 1.2, ..OptOptions::default() }).unwrap();
    assert!(r.converged);
    assert!((r.scales[0] - 1.2).abs() < 1e-9, "the softest bar wants more than the cap: {:?}", r.scales);
    assert!((r.scales.iter().zip(&len).map(|(s, l)| s * l).sum::<f64>() - volume).abs() < 1e-8 * volume);
    // The two free bars still satisfy the equal-sensitivity condition: s_b E_b^0.5 equal.
    assert!(((r.scales[1] * e[1].sqrt()) / (r.scales[2] * e[2].sqrt()) - 1.0).abs() < 1e-5, "{:?}", r.scales);
}

#[test]
fn an_unreachable_budget_is_an_error() {
    let (model, bc, loads) = series([1.0e7; 3], [1.0; 3]);
    assert!(model.optimize_compliance(&loads, &bc, &[0, 1, 2], 1e9, &OptOptions::default()).is_err());
}
