//! Interference-fit bushing in a 3D solid (the plane profile extruded): Lamé closed form, the mid-plane against plane strain,
//! equilibrium, with and without a pin load.
use fea_problem::{templates::templates, Analysis, Geometry, Load, MeshSpec, Problem, Shape, Support};

/// Plate [0,L] x [0,H], hole r=0.25 at the centre, bushing ID 0.375, diametral interference 0.002, steel on steel.
fn problem(analysis: Analysis, load: f64, len: f64, thick: f64, layers: usize) -> Problem {
    let mut p = templates().remove(4).1;
    p.analysis = analysis;
    p.thickness = thick;
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: len, y1: len }, holes: vec![Shape::Circle { cx: 0.5 * len, cy: 0.5 * len, r: 0.25 }], depth: thick, layers, regions: Vec::new() };
    p.mesh = MeshSpec { size: len / 6.0, hole_factor: 0.25 / 6.0 / (len / 6.0), ..MeshSpec::default() };
    p.bushings[0].inner_diameter = 0.375;
    p.bushings[0].interference = 0.002;
    p.bushings[0].friction = 0.15;
    p.supports = vec![Support::fixed("right")];
    p.loads = if load > 0.0 { vec![Load::Bearing { hole: 1, fx: -load, fy: 0.0 }] } else { Vec::new() };
    p.cases.clear();
    p.active_case = 0;
    p
}

/// The mesh the 3D cases run on: Quad8 profile (hex20 solid), two layers. Measured: 1948 elements, 32k dofs; the mean fit
/// pressure is the same to 0.1 % from a mesh of 0.67 in to one of 1.0 in, so this is converged for the interface.
fn coarse(mut p: Problem) -> Problem {
    p.mesh.size = 1.0;
    p.mesh.hole_factor = 0.25 / 6.0;
    p
}

/// Lamé: a hollow bushing (a, b) pressed into a plate hole (b, c); radial interference `delta`, plane strain.
fn lame_plane_strain(p: &Problem, c: f64) -> f64 {
    let (b, a) = (0.25f64, 0.1875f64);
    let delta = 0.5 * p.bushings[0].interference;
    let (eo, vo) = (p.material.e / (1.0 - p.material.nu.powi(2)), p.material.nu / (1.0 - p.material.nu));
    let bm = &p.bushings[0].material;
    let (ei, vi) = (bm.e / (1.0 - bm.nu.powi(2)), bm.nu / (1.0 - bm.nu));
    delta / (b * ((1.0 / eo) * ((c * c + b * b) / (c * c - b * b) + vo) + (1.0 / ei) * ((b * b + a * a) / (b * b - a * a) - vi)))
}

#[test]
fn a_bushing_is_valid_in_an_extruded_solid_and_not_in_a_revolved_one() {
    let mut p = problem(Analysis::Solid, 500.0, 2.0, 0.5, 2);
    p.validate().unwrap();
    p.analysis = Analysis::Axisymmetric;
    assert!(p.validate().unwrap_err().contains("bushings are for"), "an axisymmetric model has no bushing");
}

/// ~3-8 minutes in release: `cargo test --release -p fea-problem --test bushing_solid -- --ignored --nocapture`.
#[test]
#[ignore]
fn a_long_solid_bushing_matches_lame_and_the_plane_strain_model_and_the_loaded_fit_balances() {
    // Depth 2 in on a 0.25 in hole: long enough that the middle of the bore is plane strain.
    let solid = coarse(problem(Analysis::Solid, 0.0, 2.0, 2.0, 2));
    let t = std::time::Instant::now();
    let s3 = fea_problem::solve(&solid, None).expect("the 3D fit solves");
    let t_fit = t.elapsed().as_secs_f64();
    let s2 = fea_problem::solve(&coarse(problem(Analysis::PlaneStrain, 0.0, 2.0, 2.0, 2)), None).expect("the plane-strain fit solves");
    let (p3, p2) = (s3.summary.interfaces[0].mean_pressure, s2.summary.interfaces[0].mean_pressure);
    let lame = lame_plane_strain(&solid, 1.0);
    eprintln!("3D {p3:.0} psi, plane strain {p2:.0} psi, Lame (c = 1 in) {lame:.0} psi; {} nodes, fit {t_fit:.0} s", s3.summary.nodes);
    assert!((p3 - p2).abs() / p2 < 0.06, "3D mean pressure {p3} against plane strain {p2}");
    assert!((p2 - lame).abs() / lame < 0.10, "plane strain {p2} against Lame {lame}");
    assert!(s3.summary.equilibrium_error < 1e-3, "fit equilibrium {}", s3.summary.equilibrium_error);

    let loaded = coarse(problem(Analysis::Solid, 500.0, 2.0, 2.0, 2));
    let t = std::time::Instant::now();
    let l3 = fea_problem::solve(&loaded, None).expect("the loaded 3D fit solves");
    eprintln!("loaded 3D {:.0} s", t.elapsed().as_secs_f64());
    assert!(l3.summary.equilibrium_error < 1e-3, "loaded equilibrium {}", l3.summary.equilibrium_error);
    assert!((l3.summary.applied[0] + 500.0).abs() < 1.0, "the pin load is applied: {:?}", l3.summary.applied);
}
