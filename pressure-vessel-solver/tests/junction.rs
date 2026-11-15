//! Shell-to-head junction FE (`junction.rs`, #234): membrane limits, mesh convergence, thin-shell reference.
use pressure_vessel_solver::junction::*;

const E: f64 = 29.0e6;
const NU: f64 = 0.3;
const P: f64 = 100.0;

fn run(ri: f64, t: f64, head: HeadShape, refinement: usize) -> JunctionSolution {
    solve_junction(JunctionInput { inside_radius: ri, thickness: t, head, pressure: P, youngs_modulus: E, poisson: NU, refinement }).unwrap()
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs()
}

/// Membrane limit: the far cylinder carries the exact closed-end Lame stresses (axial `p Ri^2/(Ro^2-Ri^2)`, mean hoop
/// `p Ri / t`), the load enters as `-p` on the inside surface, and the crown of a hemisphere is the same biaxial state.
#[test]
fn membrane_limits_away_from_the_junction() {
    for rt in [50.0, 100.0, 200.0, 400.0] {
        let (ri, t) = (rt, 1.0);
        let ro = ri + t;
        let s = run(ri, t, HeadShape::Hemispherical, 2);
        let far = s.far_cylinder();
        let axial = P * ri * ri / (ro * ro - ri * ri);
        assert!(rel(far.membrane[0], axial) < 1e-3, "R/t {rt}: axial {} vs {axial}", far.membrane[0]);
        assert!(rel(far.membrane[1], P * ri / t) < 5e-3, "R/t {rt}: hoop {} vs {}", far.membrane[1], P * ri / t);
        // The Lame hoop stress `A + B/r^2` is not uniform through the wall: its linear (bending) part is exact, integrate it.
        let (a, b) = (P * ri * ri / (ro * ro - ri * ri), P * ri * ri * ro * ro / (ro * ro - ri * ri));
        let n = 4000;
        let lame_bending: f64 = (0..n).map(|i| {
            let z = -0.5 * t + t * (i as f64 + 0.5) / n as f64;
            (a + b / (ri + t * 0.5 + z).powi(2)) * z * (t / n as f64)
        }).sum::<f64>() * 6.0 / (t * t);
        assert!(far.bending[0].abs() < 2e-3 * axial, "R/t {rt}: far-end axial bending {}", far.bending[0]);
        assert!((far.bending[1] - lame_bending).abs() < 5e-3 * P * ri / t, "R/t {rt}: far-end hoop bending {} vs Lame {lame_bending}", far.bending[1]);
        let crown = s.crown();
        assert!(rel(crown.membrane[0], axial) < 5e-3 && rel(crown.membrane[1], axial) < 5e-3, "R/t {rt}: crown {:?} vs {axial}", crown.membrane);
        assert!(rel(s.junction().inside_normal.abs(), P) < 1e-3, "R/t {rt}: inside normal stress {}", s.junction().inside_normal);
    }
}

/// Peak discontinuity bending and junction displacement against the thin-shell reference. Tolerances shrink with `R/t`
/// because the residual is the reference's own thin-shell (Geckeler) error, which is `O(t/R)`-small, plus the sampling
/// of the peak at mesh nodes; observed errors are 2.4 %, 1.5 %, 0.8 %, 0.1 % of the peak and 0.7 %, 0.3 %, 0.2 %, 0.1 % of the
/// displacement at `R/t` = 50, 100, 200, 400.
#[test]
fn hemisphere_junction_matches_the_thin_shell_reference() {
    // (R_i / t, bending tolerance, displacement tolerance)
    for (rt, tol_bend, tol_u) in [(50.0, 0.03, 0.01), (100.0, 0.02, 0.005), (200.0, 0.01, 0.0025), (400.0, 0.01, 0.0015)] {
        let (ri, t) = (rt, 1.0);
        let reference = cylinder_hemisphere_reference(ri + 0.5 * t, t, P, E, NU);
        let s = run(ri, t, HeadShape::Hemispherical, 4);
        let (bend, at) = s.peak_bending_meridional();
        assert!(rel(bend.abs(), reference.peak_bending_stress) < tol_bend, "R/t {rt}: bending {bend} vs {}", reference.peak_bending_stress);
        assert!((at.abs() - reference.peak_distance).abs() < 0.15 * s.decay_length, "R/t {rt}: peak at {at}, reference {}", reference.peak_distance);
        assert!(rel(s.junction_radial_displacement, reference.junction_radial_displacement) < tol_u, "R/t {rt}: u {} vs {}", s.junction_radial_displacement, reference.junction_radial_displacement);
        // The moment is zero at the junction (symmetric joint): the bending stress there is small against the peak.
        assert!(s.junction().bending[0].abs() < 0.05 * bend.abs(), "R/t {rt}: junction bending {:?}", s.junction().bending);
        // The discontinuity is symmetric: the head side mirrors the cylinder side of the peak.
        let head_peak = s.stations.iter().filter(|x| x.s > 0.0).map(|x| x.bending[0].abs()).fold(0.0, f64::max);
        assert!(rel(head_peak, bend.abs()) < 0.05, "R/t {rt}: head side peak {head_peak} vs cylinder side {}", bend.abs());
    }
    // The error falls with R/t (the reference is an asymptotic result).
    let err = |rt: f64| {
        let reference = cylinder_hemisphere_reference(rt + 0.5, 1.0, P, E, NU);
        rel(run(rt, 1.0, HeadShape::Hemispherical, 4).peak_bending_meridional().0.abs(), reference.peak_bending_stress)
    };
    assert!(err(400.0) < err(50.0) / 4.0, "error does not shrink with R/t: {} vs {}", err(400.0), err(50.0));
}

/// Mesh convergence of the peak bending stress and the junction displacement (levels 1, 2, 4), hemisphere and 2:1 ellipsoid.
#[test]
fn mesh_convergence() {
    for head in [HeadShape::Hemispherical, HeadShape::Ellipsoidal { inside_depth: 50.0 }] {
        let s: Vec<_> = [1, 2, 4].iter().map(|&k| run(100.0, 1.0, head, k)).collect();
        let b: Vec<f64> = s.iter().map(|x| x.peak_bending_meridional().0.abs()).collect();
        let u: Vec<f64> = s.iter().map(|x| x.junction_radial_displacement).collect();
        assert!(rel(b[2], b[1]) < 0.01, "{head:?}: bending {b:?}");
        assert!(rel(b[2], b[0]) < 0.03, "{head:?}: bending {b:?}");
        assert!(rel(u[2], u[1]) < 5e-4 && rel(u[1], u[0]) < 2e-3, "{head:?}: displacement {u:?}");
        assert!(s[0].n_elements < s[1].n_elements && s[1].n_elements < s[2].n_elements);
    }
}

/// 2:1 ellipsoid: crown stress is the thin-shell membrane value `p a^2 / (2 b t)` (mid-surface semi-axes), the far
/// cylinder keeps its membrane state, the head is in hoop compression near the equator, and the discontinuity is larger
/// than the hemisphere's. The closed form for the cylinder-to-ellipsoid junction itself has not been sourced (see
/// docs/pressure-vessel-heads.md), so the junction values are checked by convergence and equilibrium only.
#[test]
fn ellipsoidal_head_membrane_and_discontinuity() {
    for rt in [50.0, 100.0, 200.0] {
        let (ri, t) = (rt, 1.0);
        let (a, b) = (ri + 0.5 * t, 0.5 * ri + 0.5 * t);
        let s = run(ri, t, HeadShape::Ellipsoidal { inside_depth: 0.5 * ri }, 4);
        let crown = s.crown();
        let thin = P * a * a / (2.0 * b * t);
        assert!(rel(crown.membrane[0], thin) < 0.015 && rel(crown.membrane[1], thin) < 0.015, "R/t {rt}: crown {:?} vs {thin}", crown.membrane);
        let axial = P * ri * ri / ((ri + t).powi(2) - ri * ri);
        assert!(rel(s.far_cylinder().membrane[0], axial) < 1e-3);
        let hoop_min = s.stations.iter().filter(|x| x.s > 0.0).map(|x| x.membrane[1]).fold(f64::INFINITY, f64::min);
        assert!(hoop_min < 0.0, "R/t {rt}: head hoop minimum {hoop_min}: membrane theory has the equator in compression (-p a / t)");
        let hemi = run(ri, t, HeadShape::Hemispherical, 2);
        assert!(s.peak_meridional_surface().0.abs() > 1.5 * hemi.peak_meridional_surface().0.abs());
        assert!(rel(s.junction().inside_normal.abs(), P) < 6e-3, "R/t {rt}: inside normal {}", s.junction().inside_normal);
    }
}

#[test]
fn a_half_depth_ellipsoid_of_full_depth_is_the_hemisphere_and_the_response_is_linear() {
    let h = run(80.0, 1.0, HeadShape::Hemispherical, 2);
    let e = run(80.0, 1.0, HeadShape::Ellipsoidal { inside_depth: 80.0 }, 2);
    assert_eq!(h, e);
    let double = solve_junction(JunctionInput { inside_radius: 80.0, thickness: 1.0, head: HeadShape::Hemispherical, pressure: 2.0 * P, youngs_modulus: E, poisson: NU, refinement: 2 }).unwrap();
    assert!(rel(double.peak_bending_meridional().0, 2.0 * h.peak_bending_meridional().0) < 1e-9);
}

#[test]
fn bad_input_is_refused() {
    let ok = JunctionInput { inside_radius: 50.0, thickness: 1.0, head: HeadShape::Hemispherical, pressure: P, youngs_modulus: E, poisson: NU, refinement: 1 };
    for bad in [
        JunctionInput { inside_radius: 0.0, ..ok },
        JunctionInput { thickness: -1.0, ..ok },
        JunctionInput { thickness: 40.0, ..ok },
        JunctionInput { pressure: f64::NAN, ..ok },
        JunctionInput { poisson: 0.5, ..ok },
        JunctionInput { refinement: 0, ..ok },
        JunctionInput { head: HeadShape::Ellipsoidal { inside_depth: 60.0 }, ..ok },
        JunctionInput { head: HeadShape::Ellipsoidal { inside_depth: 0.0 }, ..ok },
    ] {
        assert!(matches!(solve_junction(bad), Err(JunctionError::InvalidInput(_))), "{bad:?}");
    }
}

/// Characterisation, not a tolerance on the solver: the equal-wall Geckeler formula for an ellipsoidal head (Balicevic, Kozak,
/// Mrcela, Strojniski vestnik 54(2008)10 eqs. 14 and 17: `Q0 = p a^2 / (8 beta b^2)`, `M0 = 0`, peak surface bending
/// `6 * 0.3224 Q0 / (beta t^2)`) underestimates the head-side peak of a 2:1 head, by a margin that shrinks with `R/t`. The paper
/// claims an `O(h/R)` (< 5 %) error; the FE shows 35 / 23 / 14 / 7 % at R/t = 50 / 100 / 200 / 400.
#[test]
fn the_ellipsoid_geckeler_formula_underestimates_the_head_side_peak() {
    let ratio = |rt: f64| {
        let (a, b) = (rt + 0.5, 0.5 * rt + 0.5);
        let beta = (3.0 * (1.0 - NU * NU)).powf(0.25) / a.sqrt();
        let formula = 6.0 * 0.3224 * P * a * a / (8.0 * beta * b * b) / beta;
        let s = run(rt, 1.0, HeadShape::Ellipsoidal { inside_depth: 0.5 * rt }, 4);
        let head = s.stations.iter().filter(|x| x.s > 0.0).map(|x| x.bending[0].abs()).fold(0.0, f64::max);
        let cyl = s.stations.iter().filter(|x| x.s < 0.0).map(|x| x.bending[0].abs()).fold(0.0, f64::max);
        assert!(head > cyl, "R/t {rt}: head side {head} should carry the larger peak than the cylinder side {cyl}");
        head / formula
    };
    let r: Vec<f64> = [50.0, 100.0, 200.0, 400.0].iter().map(|&x| ratio(x)).collect();
    assert!(r.windows(2).all(|w| w[1] < w[0]), "ratio does not shrink with R/t: {r:?}");
    assert!((1.25..1.45).contains(&r[0]) && (1.15..1.30).contains(&r[1]) && (1.07..1.20).contains(&r[2]) && (1.02..1.12).contains(&r[3]), "{r:?}");
}

/// Global equilibrium: the only support force is the one axial constraint; the pressure thrust on the inside and the closed-end
/// traction at the far end must balance without it (observed ratio <= 2e-10, solver round-off), for both heads, all `R/t` and mesh levels.
#[test]
fn loads_are_in_global_equilibrium() {
    for head in [HeadShape::Hemispherical, HeadShape::Ellipsoidal { inside_depth: 50.0 }] {
        for k in [1, 2, 4] {
            let s = run(100.0, 1.0, head, k);
            assert!(s.axial_reaction.abs() < 1e-8 * s.axial_thrust, "{head:?} level {k}: reaction {} vs thrust {}", s.axial_reaction, s.axial_thrust);
        }
    }
}
