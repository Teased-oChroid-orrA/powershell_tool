//! Natural frequencies and buckling of problems against beam theory.

use fea_problem::dynamics::{buckling, modal};
use fea_problem::{templates::templates, Load, Problem};
use std::f64::consts::PI;

fn beam() -> Problem {
    templates().into_iter().find(|(t, _)| *t == "Cantilever beam").unwrap().1
}

#[test]
fn the_cantilever_template_vibrates_at_the_euler_bernoulli_frequencies() {
    let p = beam();
    let r = modal(&p, None, 4).unwrap();
    let (e, rho, l, h, t) = (p.material.e, p.material.density, 10.0f64, 1.0f64, p.thickness);
    let (i, a) = (t * h * h * h / 12.0, t * h);
    let omega = |c: f64| c * c * (e * i / (rho * a * l.powi(4))).sqrt();
    // Bending in the plane (the first two modes of a plane model), shear deformation lowers them by about (h / L)^2 times a constant.
    let w1 = r.modes[0].frequency_hz * 2.0 * PI;
    assert!((w1 / omega(1.875_104_07) - 1.0).abs() < 0.02, "{w1} vs {}", omega(1.875_104_07));
    assert!(r.modes.windows(2).all(|w| w[1].frequency_hz >= w[0].frequency_hz));
    assert!((r.total_mass - rho * 10.0 * 1.0 * t).abs() < 1e-9 * r.total_mass, "mass {}", r.total_mass);
    assert!(r.modes[0].effective_mass_share[1] > 0.5, "the first bending mode moves the mass in y: {:?}", r.modes[0].effective_mass_share);
    assert_eq!(r.modes[0].shape.len(), 2 * r.model.mesh.nodes.len());
    assert_eq!(r.magnitude(0).len(), r.model.mesh.nodes.len());
}

#[test]
fn a_missing_density_or_support_is_explained() {
    let mut p = beam();
    p.material.density = 0.0;
    assert!(modal(&p, None, 2).unwrap_err().contains("density"));
    let mut p = beam();
    p.supports.clear();
    let err = modal(&p, None, 2).unwrap_err();
    assert!(err.contains("support"), "{err}");
}

#[test]
fn the_cantilever_template_buckles_at_the_euler_load() {
    let mut p = beam();
    p.loads = vec![Load::Force { edge: "right".into(), fx: -1000.0, fy: 0.0, fz: 0.0 }];
    let r = buckling(&p, None, 2).unwrap();
    let pcr = PI * PI * p.material.e * (p.thickness / 12.0) / (4.0 * 100.0);
    assert!((r.modes[0].load_factor * 1000.0 / pcr - 1.0).abs() < 0.03, "{} vs {}", r.modes[0].load_factor * 1000.0, pcr);
    // A pure transverse load has no buckling load: the beam is in bending, not compression, but the stress state does compress one flange
    // (so a positive factor exists and is much larger); the axial case is the closed form.
}

#[test]
fn bushings_and_axisymmetric_problems_are_refused() {
    let mut p = templates().into_iter().find(|(t, _)| t.starts_with("Lug with an eccentric")).unwrap().1;
    assert!(modal(&p, None, 2).unwrap_err().contains("bushing"));
    p.bushings.clear();
    let cyl = templates().into_iter().find(|(t, _)| t.starts_with("Thick cylinder")).unwrap().1;
    assert!(buckling(&cyl, None, 1).unwrap_err().contains("axisymmetric"));
}

#[test]
fn a_mixed_sign_stress_state_still_gives_buckling_factors() {
    // The transverse tip load compresses one flange and stretches the other, so the geometric stiffness is indefinite: Lanczos at its
    // usual size does not isolate the positive end of the spectrum, the protocol falls back to subspace iteration and still verifies.
    let p = beam();
    let r = buckling(&p, None, 3).unwrap();
    assert!(r.modes.iter().all(|m| m.load_factor > 0.0 && m.residual < 1e-8), "{:?}", r.modes.iter().map(|m| (m.load_factor, m.residual)).collect::<Vec<_>>());
    assert!(r.modes.windows(2).all(|w| w[1].load_factor >= w[0].load_factor));
}
