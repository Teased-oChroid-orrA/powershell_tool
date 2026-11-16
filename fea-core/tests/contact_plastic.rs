//! Deformable contact combined with plasticity: an interference fit whose hub yields, against the elastic-perfectly-plastic
//! thick-cylinder solution (Hill; plane strain, von Mises: the plastic zone carries `sigma_theta - sigma_r = Y'` with
//! `Y' = 2 sigma_y / sqrt 3`, and a pressure `p` at the bore of a cylinder `a..b` with the elastic-plastic boundary at `c` is
//! `p = Y' [ ln(c / a) + (1 - c^2 / b^2) / 2 ]`; fully plastic `p -> Y' ln(b / a)`).

use fea_core::contact::ContactSpec;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Curve, Loop, Region, Segment};
use fea_core::material::J2;
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use mechanics_core::hardening::Hardening;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const A: f64 = 1.0;
const B: f64 = 2.0;

struct Fit {
    model: Model,
    bc: Dirichlet,
    shaft_face: Vec<Vec<usize>>,
    hub_face: Vec<Vec<usize>>,
    shaft_blocks: usize,
}

/// Quarter shaft (radius `A + delta`) in a quarter hub (`A..B`), plane strain; the hub (and, with `shaft_yield`, the shaft)
/// is elastic-perfectly-plastic with yield stress `sy` when given.
fn fit(nu: f64, delta: f64, hub_yield: Option<f64>, shaft_yield: Option<f64>) -> Fit {
    let mat = Elastic::new(E, nu);
    let rs = A + delta;
    let seg = |curve, name: &str| Segment { curve, name: name.into() };
    let half_pi = std::f64::consts::FRAC_PI_2;
    let shaft = Loop::new(vec![seg(Curve::line([0.0, 0.0], [rs, 0.0]), "xaxis"), seg(Curve::arc([0.0, 0.0], rs, 0.0, half_pi), "iface"), seg(Curve::line([0.0, rs], [0.0, 0.0]), "yaxis")]).unwrap();
    let hub = Loop::new(vec![seg(Curve::line([A, 0.0], [B, 0.0]), "xaxis"), seg(Curve::arc([0.0, 0.0], B, 0.0, half_pi), "outer"), seg(Curve::line([0.0, B], [0.0, A]), "yaxis"), seg(Curve::arc([0.0, 0.0], A, half_pi, 0.0), "iface")]).unwrap();
    let physics = Physics::PlaneStrain { thickness: 1.0 };
    let mut mesh = mesh_region(&Region::new(shaft, vec![], mat).unwrap(), physics, ElementKind::Tri6, &|_| 0.1, MeshOptions::default()).unwrap();
    let m_hub = mesh_region(&Region::new(hub, vec![], mat).unwrap(), physics, ElementKind::Tri6, &|_| 0.1, MeshOptions::default()).unwrap();
    let shaft_blocks = mesh.blocks.len();
    mesh.append(&m_hub, "hub_").unwrap();
    for (b, sy) in (0..mesh.blocks.len()).map(|b| (b, if b < shaft_blocks { shaft_yield } else { hub_yield })) {
        if let Some(sy) = sy {
            mesh.set_plasticity(b, J2::small(Hardening::linear(sy, 0.0, 5.0).unwrap())).unwrap();
        }
    }
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for prefix in ["", "hub_"] {
        for &n in model.mesh.node_set(&format!("{prefix}xaxis")).unwrap() {
            bc.fix(n, 1, 0.0);
        }
        for &n in model.mesh.node_set(&format!("{prefix}yaxis")).unwrap() {
            bc.fix(n, 0, 0.0);
        }
    }
    let (shaft_face, hub_face) = (model.mesh.surfaces["iface"].clone(), model.mesh.surfaces["hub_iface"].clone());
    Fit { model, bc, shaft_face, hub_face, shaft_blocks }
}

struct Result {
    /// Physical interface pressure, from the force the two passes transmit.
    pressure: f64,
    /// Largest radius of a yielded Gauss point of the hub (the elastic-plastic boundary, within an element).
    plastic_radius: f64,
    min_gap: f64,
    sol: NlSolution,
}

fn solve(f: &Fit, steps: usize) -> Result {
    let specs: Vec<ContactSpec> = ContactSpec::two_pass("fit", f.shaft_face.clone(), f.hub_face.clone(), 1.0e9).into_iter().map(|s| s.with_margin(0.1)).collect();
    let sol = f.model.solve_nonlinear_contact(&Loads::default(), &f.bc, specs, &NlOptions { steps, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    // The physical interface force is the sum of the two passes' (each pass carries a share); the quarter arc of radius A
    // transmits `p A` in x and in y. The unweighted mean of the Gauss-point pressures is not that force once the pressure is
    // not uniform (it read 12 % high in a yielded fit while the stress field matched Hill's).
    let mf = &sol.contact.as_ref().unwrap().master_force;
    let pressure = ((mf[0][0].abs() + mf[1][0].abs()) + (mf[0][1].abs() + mf[1][1].abs())) / (2.0 * A);
    let plastic_radius = sol.state.gauss_ep(&f.model.mesh).iter().filter(|(_, ep)| *ep > 1e-12).map(|(x, _)| x[0].hypot(x[1])).fold(0.0, f64::max);
    let min_gap = sol.contact.as_ref().unwrap().min_gap;
    Result { pressure, plastic_radius, min_gap, sol }
}

/// Plane-strain Lame pressure of the elastic fit.
fn lame(nu: f64, delta: f64) -> f64 {
    delta * E / ((1.0 + nu) * A * (((1.0 - 2.0 * nu) * A * A + B * B) / (B * B - A * A) + (1.0 - 2.0 * nu)))
}

fn hill(y: f64, c: f64) -> f64 {
    y * ((c / A).ln() + 0.5 * (1.0 - c * c / (B * B)))
}

/// A yield stress far above any stress: the plastic path must reproduce the elastic contact solution.
#[test]
fn an_elastic_limit_plastic_fit_reproduces_the_elastic_contact_solution() {
    let delta = 0.002;
    let elastic = solve(&fit(NU, delta, None, None), 2);
    let plastic = solve(&fit(NU, delta, Some(1.0e12), Some(1.0e12)), 2);
    eprintln!("elastic {:.3} plastic-path {:.3} Lame {:.3}", elastic.pressure, plastic.pressure, lame(NU, delta));
    assert_eq!(plastic.sol.state.max_ep(), 0.0);
    assert!((plastic.pressure / elastic.pressure - 1.0).abs() < 1e-6, "{} vs {}", plastic.pressure, elastic.pressure);
    assert!((plastic.pressure / lame(NU, delta) - 1.0).abs() < 0.02);
    let du: f64 = elastic.sol.u.iter().zip(&plastic.sol.u).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(du < 1e-8 * delta.max(1.0), "max displacement difference {du:e}");
}

/// The hub yields from the bore outwards; the interface pressure of the converged fit matches Hill's relation at the plastic
/// radius the finite elements show, stays below the elastic Lame pressure of the same interference, and the shaft (under near
/// hydrostatic pressure) stays elastic.
#[test]
fn a_hub_yielding_past_the_elastic_limit_follows_the_thick_cylinder_solution() {
    // Pressure at first yield of the bore: Y' (1 - a^2 / b^2) / 2. Choose the yield so that the elastic Lame pressure of this
    // interference is 2x that, i.e. the plastic zone reaches well into the wall.
    let delta = 0.004;
    let sy = lame(NU, delta) / 2.0 / ((1.0 - A * A / (B * B)) / 2.0) * 3.0f64.sqrt() / 2.0;
    let y = 2.0 * sy / 3.0f64.sqrt();
    let r = solve(&fit(NU, delta, Some(sy), None), 8);
    let c = r.plastic_radius;
    eprintln!("hub yield: p {:.1} (Lame {:.1}), plastic radius {c:.3}, Hill p(c) {:.1}, max ep {:.2e}, min gap {:.1e}", r.pressure, lame(NU, delta), hill(y, c), r.sol.state.max_ep(), r.min_gap);
    assert!(r.sol.state.max_ep() > 0.0 && c > 1.15 * A && c < 0.95 * B, "plastic radius {c}");
    assert!(r.pressure < 0.9 * lame(NU, delta), "yielding must relieve the pressure: {} vs elastic {}", r.pressure, lame(NU, delta));
    // c is a Gauss-point radius, resolved to about one element (0.1): the relation is steep in c only mildly (dp/dc ~ Y'/c).
    let (lo, hi) = (hill(y, c - 0.1), hill(y, c + 0.1));
    assert!(r.pressure > lo * 0.97 && r.pressure < hi * 1.03, "p {} vs Hill {lo:.1}..{hi:.1} at c = {c}", r.pressure);
    assert!(r.min_gap > -1e-2 * delta, "penetration must be removed: {}", r.min_gap);
}

/// A very large interference drives the hub fully plastic: the pressure approaches the limit `Y' ln(b / a)` from below.
#[test]
fn a_large_interference_approaches_the_fully_plastic_limit() {
    let sy = 1.0e4;
    let y = 2.0 * sy / 3.0f64.sqrt();
    let limit = y * (B / A).ln();
    let r = solve(&fit(NU, 0.05, Some(sy), None), 8);
    eprintln!("limit {limit:.1}: p {:.1}, plastic radius {:.3}", r.pressure, r.plastic_radius);
    assert!(r.pressure < 1.03 * limit && r.pressure > 0.93 * limit, "{} vs {limit}", r.pressure);
}

/// Both bodies plastic with different yield stresses: the weaker one yields first (the right law goes to the right body).
#[test]
fn per_body_yield_stress_selects_the_body_that_yields() {
    let delta = 0.004;
    let weak_hub = solve(&fit(NU, delta, Some(2.0e3), Some(1.0e12)), 8);
    let weak_shaft = solve(&fit(NU, delta, Some(1.0e12), Some(2.0e3)), 8);
    // largest plastic strain in the shaft's blocks and in the hub's
    let split = |r: &Result, f: &Fit| -> (f64, f64) {
        let peak = |range: std::ops::Range<usize>| range.map(|b| r.sol.state.gp[b].iter().map(|g| g[6]).fold(0.0, f64::max)).fold(0.0, f64::max);
        (peak(0..f.shaft_blocks), peak(f.shaft_blocks..f.model.mesh.blocks.len()))
    };
    let f1 = fit(NU, delta, Some(2.0e3), Some(1.0e12));
    let f2 = fit(NU, delta, Some(1.0e12), Some(2.0e3));
    let (s1, h1) = split(&weak_hub, &f1);
    let (s2, h2) = split(&weak_shaft, &f2);
    eprintln!("weak hub: shaft ep {s1:.2e} hub ep {h1:.2e}; weak shaft: shaft ep {s2:.2e} hub ep {h2:.2e}");
    assert!(h1 > 1e-4 && s1 == 0.0);
    assert!(s2 > 1e-4 && h2 == 0.0);
}

/// A plastic law that never yields takes the cap-sized steps under `NlOptions::ep_step` (not the fixed 8) and lands on the
/// fixed-step solution; a fit that does yield is shortened where it yields and keeps Hill's pressure.
#[test]
fn the_plastic_increment_control_takes_long_steps_while_the_fit_stays_elastic() {
    let delta = 0.002;
    let f = fit(NU, delta, Some(1.0e12), Some(1.0e12));
    let specs = || -> Vec<ContactSpec> { ContactSpec::two_pass("fit", f.shaft_face.clone(), f.hub_face.clone(), 1.0e9).into_iter().map(|s| s.with_margin(0.1)).collect() };
    let fixed = f.model.solve_nonlinear_contact(&Loads::default(), &f.bc, specs(), &NlOptions { steps: 8, ..NlOptions::default() }).unwrap();
    let adaptive = f.model.solve_nonlinear_contact(&Loads::default(), &f.bc, specs(), &NlOptions { steps: 2, ep_step: 1e-3, ..NlOptions::default() }).unwrap();
    assert!(fixed.complete() && adaptive.complete());
    assert_eq!(fixed.steps.len(), 8);
    assert_eq!(adaptive.steps.len(), 2, "no plastic strain: the steps stay at the cap");
    let du: f64 = fixed.u.iter().zip(&adaptive.u).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(du < 1e-3 * delta, "max displacement difference {du:e}");
}
