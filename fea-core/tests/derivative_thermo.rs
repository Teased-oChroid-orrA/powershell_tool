//! Thermal loads in the exact linear sensitivities (modulus, Poisson ratio, expansion coefficient, thickness, coordinates, uniform
//! temperature) and the staggered thermal-structural chain (conduction parameters through the temperature field into the
//! displacements), direct against adjoint against Richardson differences of the full coupled solve.

use fea_core::derivative::linear_sens::{LinearParam, LinearSensitivityEngine};
use fea_core::derivative::thermo::*;
use fea_core::generate::grid;
use fea_core::thermal::{Conductivity, HeatLoads};
use fea_core::*;

#[derive(Clone, Copy, Default)]
struct Shift {
    modulus: f64,
    nu: f64,
    alpha: f64,
    thickness: f64,
    conductivity: f64,
    heat: f64,
    delta_t: f64,
    node: usize,
    dx: f64,
    dy: f64,
}

struct Case {
    model: Model,
    bc: Dirichlet,
    tbc: Dirichlet,
    loads: Loads,
    heat: HeatLoads,
    right: Vec<usize>,
}

fn case(kind: ElementKind, s: Shift) -> Case {
    let mut mesh = grid(Physics::PlaneStress { thickness: 0.5 + s.thickness }, kind, Elastic::new(2.0e5, 0.3), [5, 3, 1], &|p| [3.0 * p[0] + 0.1 * p[1], p[1] + 0.04 * p[0] * p[0], 0.0]).unwrap();
    mesh.blocks[0].material.e *= 1.0 + s.modulus;
    mesh.blocks[0].material.nu += s.nu;
    mesh.blocks[0].material.alpha = 1.2e-5 * (1.0 + s.alpha);
    mesh.set_thermal(0, Conductivity::Constant(15.0 * (1.0 + s.conductivity)), 1.0).unwrap();
    mesh.nodes[s.node][0] += s.dx;
    mesh.nodes[s.node][1] += s.dy;
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = model.mesh.node_set("u0").unwrap()[0];
    bc.fix(anchor, 1, 0.0);
    let mut tbc = model.thermal_dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        tbc.fix(n, 0, 80.0);
    }
    for &n in model.mesh.node_set("u1").unwrap() {
        tbc.fix(n, 0, 20.0);
    }
    let heat = HeatLoads { source: 40.0 * (1.0 + s.heat), ..HeatLoads::default() };
    let loads = Loads { nodal: vec![(model.mesh.node_set("u1").unwrap()[0], [0.0, -50.0, 0.0])], delta_t: s.delta_t, ..Loads::default() };
    let right = model.mesh.node_set("u1").unwrap().to_vec();
    Case { model, bc, tbc, loads, heat, right }
}

fn qoi(c: &Case) -> Vec<f64> {
    let n = c.model.mesh.n_dofs();
    let mut g = vec![0.0; n];
    for &nd in &c.right {
        g[nd * 2] = 1.0 / c.right.len() as f64;
        g[nd * 2 + 1] = 0.3 / c.right.len() as f64;
    }
    g
}

fn richardson(f: &dyn Fn(f64) -> f64, h: f64) -> f64 {
    let d = |h: f64| (f(h) - f(-h)) / (2.0 * h);
    (4.0 * d(0.5 * h) - d(h)) / 3.0
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Linear engine with a thermal load (uniform temperature rise plus a nodal force): exact derivatives against differences of the solve.
fn check_linear(kind: ElementKind) {
    let base = Shift { delta_t: 35.0, node: 8, ..Shift::default() };
    let c = case(kind, base);
    let node = (0..c.model.mesh.nodes.len()).find(|&n| c.model.mesh.nodes[n][0] > 1.0 && c.model.mesh.nodes[n][1] > 0.2 && c.model.mesh.nodes[n][1] < 0.8 && !c.model.mesh.node_set("u1").unwrap().contains(&n)).unwrap();
    let base = Shift { node, ..base };
    let c = case(kind, base);
    let g = qoi(&c);
    let eng = LinearSensitivityEngine::new(&c.model, &c.loads, &c.bc);
    let params = [LinearParam::ModulusScale(0), LinearParam::PoissonRatio(0), LinearParam::ThermalExpansion(0), LinearParam::Thickness, LinearParam::UniformTemperature, LinearParam::Coord { node, axis: 0 }, LinearParam::Coord { node, axis: 1 }];
    let (direct, _) = eng.sensitivities(&[g.clone()], &params, Some(fea_core::derivative::SensitivityMethod::Direct)).unwrap();
    let (adj, _) = eng.sensitivities(&[g.clone()], &params, Some(fea_core::derivative::SensitivityMethod::DiscreteAdjoint)).unwrap();
    let value = |s: Shift| {
        let c = case(kind, s);
        let sol = c.model.solve_static(&c.loads, &c.bc).unwrap();
        dot(&g, &sol.u)
    };
    let mk = |which: usize, h: f64| {
        let mut s = base;
        match which {
            0 => s.modulus = h,
            1 => s.nu = h,
            2 => s.alpha = h,
            3 => s.thickness = h,
            4 => s.delta_t += h,
            5 => s.dx = h,
            _ => s.dy = h,
        }
        s
    };
    let steps = [1e-2, 1e-3, 1e-2, 1e-3, 1.0, 1e-4, 1e-4];
    for (i, &h) in steps.iter().enumerate() {
        let fd = richardson(&|x| value(mk(i, x)), h);
        // the engine's modulus / alpha parameters are multipliers of the block's current values; the shifts above are about the base
        let (d, a) = (direct[0][i], adj[0][i]);
        // the thermal-expansion coefficient has base value 1.2e-5 * (1 + shift), so its multiplier derivative equals the shift derivative
        assert!((d - a).abs() < 1e-9 * d.abs().max(1e-12), "{kind:?} param {i}: direct {d:e} vs adjoint {a:e}");
        assert!((d - fd).abs() < 2e-5 * d.abs().max(fd.abs()).max(1e-12), "{kind:?} param {i}: exact {d:e} vs finite difference {fd:e}");
    }
}

#[test]
fn thermal_loads_are_differentiated_exactly_on_quadrilaterals_and_quadratic_triangles() {
    check_linear(ElementKind::Quad4);
    check_linear(ElementKind::Quad8);
    check_linear(ElementKind::Tri6);
}

fn coupled_value(kind: ElementKind, s: Shift) -> f64 {
    let c = case(kind, s);
    let eng = ThermoMechanicalSensitivity::new(&c.model, &c.loads, &c.heat, &c.bc, &c.tbc).unwrap();
    let st = eng.solve().unwrap();
    dot(&qoi(&c), &st.u)
}

fn check_coupled(kind: ElementKind) {
    let c = case(kind, Shift::default());
    let eng = ThermoMechanicalSensitivity::new(&c.model, &c.loads, &c.heat, &c.bc, &c.tbc).unwrap();
    let params = [CoupledParam::ConductivityScale(0), CoupledParam::HeatLoadScale, CoupledParam::Structural(LinearParam::ModulusScale(0)), CoupledParam::Structural(LinearParam::PoissonRatio(0)), CoupledParam::Structural(LinearParam::ThermalExpansion(0))];
    let g = qoi(&c);
    let (direct, rd) = eng.direct(&params, &[g.clone()]).unwrap();
    let (adj, ra) = eng.adjoint(&params, &[g.clone()]).unwrap();
    let mk = |which: usize, h: f64| {
        let mut s = Shift::default();
        match which {
            0 => s.conductivity = h,
            1 => s.heat = h,
            2 => s.modulus = h,
            3 => s.nu = h,
            _ => s.alpha = h,
        }
        s
    };
    let steps = [1e-2, 1e-2, 1e-2, 1e-3, 1e-2];
    for (i, &h) in steps.iter().enumerate() {
        let fd = richardson(&|x| coupled_value(kind, mk(i, x)), h);
        let (d, a) = (direct[0][i], adj[0][i]);
        assert!((d - a).abs() < 1e-9 * d.abs().max(1e-12), "{kind:?} param {i}: direct {d:e} vs adjoint {a:e}");
        assert!((d - fd).abs() < 2e-5 * d.abs().max(fd.abs()).max(1e-12), "{kind:?} param {i}: exact {d:e} vs finite difference {fd:e}");
    }
    assert!(direct[0][0].abs() > 1e-9 && direct[0][1].abs() > 1e-9, "the heat parameters must matter");
    eprintln!("{kind:?}: direct {:?} solves, adjoint {:?} solves", (rd.structural_solves, rd.thermal_solves), (ra.structural_solves, ra.thermal_solves));
    assert!(ra.thermal_solves < rd.thermal_solves || ra.thermal_solves <= 2, "the adjoint needs one thermal solve per quantity, the direct one per heat parameter (+ the primal)");
}

#[test]
fn heat_parameters_reach_the_displacements_through_the_temperature_field() {
    check_coupled(ElementKind::Quad4);
    check_coupled(ElementKind::Tri6);
}

#[test]
fn geometry_through_a_coupled_chain_and_temperature_dependent_conductivity_are_refused() {
    let c = case(ElementKind::Quad4, Shift::default());
    let eng = ThermoMechanicalSensitivity::new(&c.model, &c.loads, &c.heat, &c.bc, &c.tbc).unwrap();
    let e = eng.adjoint(&[CoupledParam::Structural(LinearParam::Thickness)], &[qoi(&c)]).unwrap_err();
    assert!(e.contains("not implemented"), "{e}");
    let mut mesh = c.model.mesh.clone();
    mesh.set_thermal(0, Conductivity::Table(vec![(0.0, 10.0), (100.0, 20.0)]), 1.0).unwrap();
    let m2 = Model::new(mesh).unwrap();
    assert!(ThermoMechanicalSensitivity::new(&m2, &c.loads, &c.heat, &c.bc, &c.tbc).is_err());
}

#[test]
fn the_transient_and_path_engines_refuse_thermal_loads_instead_of_dropping_their_parameter_dependence() {
    use fea_core::derivative::path::PathSensitivityEngine;
    use fea_core::derivative::transient::TransientSensitivity;
    use fea_core::dynamics::TransientOptions;
    let c = case(ElementKind::Quad4, Shift { delta_t: 10.0, ..Shift::default() });
    assert!(PathSensitivityEngine::new(&c.model, &c.loads, &c.bc).err().expect("refused").contains("thermal loads"));
    let mut model = c.model.clone();
    model.mesh.set_density(0, 1e-3).unwrap();
    let opt = TransientOptions { steps: 2, ..TransientOptions::default() };
    let g = |_t: f64| 1.0;
    let eng = TransientSensitivity::new(&model, &c.loads, &g, &c.bc, None, &opt);
    assert!(eng.response().unwrap_err().contains("thermal loads"));
}
