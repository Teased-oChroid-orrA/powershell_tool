use solid_kernel::{
    plc_conform::{conform, Options, Phase},
    step::ErrorKind,
    V3,
};
use std::collections::BTreeSet;

/// Outward-oriented triangulation of an extruded square (optionally with a
/// concentric square through-hole): a box or a thick square ring.
fn plate(side: f64, hole: Option<f64>, thickness: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let corners = |s: f64, z: f64| {
        let o = (side - s) / 2.;
        [[o, o, z], [o + s, o, z], [o + s, o + s, z], [o, o + s, z]]
    };
    let mut v = Vec::new();
    v.extend(corners(side, 0.)); // 0..4 outer bottom
    v.extend(corners(side, thickness)); // 4..8 outer top
    let mut t = Vec::new();
    for i in 0..4 {
        let j = (i + 1) % 4;
        t.push([i, j, 4 + j]);
        t.push([i, 4 + j, 4 + i]);
    }
    match hole {
        None => {
            // top and bottom as two triangles each
            t.push([4, 5, 6]);
            t.push([4, 6, 7]);
            t.push([0, 2, 1]);
            t.push([0, 3, 2]);
        }
        Some(h) => {
            v.extend(corners(h, 0.)); // 8..12 inner bottom
            v.extend(corners(h, thickness)); // 12..16 inner top
            for i in 0..4 {
                let j = (i + 1) % 4;
                // inner walls face the hole
                t.push([8 + i, 12 + j, 8 + j]);
                t.push([8 + i, 12 + i, 12 + j]);
                // top ring (normal +z) and bottom ring (normal -z)
                t.push([4 + i, 4 + j, 12 + j]);
                t.push([4 + i, 12 + j, 12 + i]);
                t.push([i, 8 + j, j]);
                t.push([i, 8 + i, 8 + j]);
            }
        }
    }
    (v, t)
}

fn volume_of(points: &[V3], cells: &[[usize; 4]]) -> f64 {
    cells
        .iter()
        .map(|c| {
            let [a, b, cc, d] = c.map(|i| points[i]);
            let u: V3 = std::array::from_fn(|i| a[i] - d[i]);
            let v: V3 = std::array::from_fn(|i| b[i] - d[i]);
            let w: V3 = std::array::from_fn(|i| cc[i] - d[i]);
            (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
                + u[2] * (v[0] * w[1] - v[1] * w[0]))
                .abs()
                / 6.
        })
        .sum()
}
fn face_key(mut f: [usize; 3]) -> [usize; 3] {
    f.sort_unstable();
    f
}

#[test]
fn box_with_conflicting_diagonals_is_recovered_with_exact_volume() {
    let (v, t) = plate(2., None, 1.);
    let mut phases = BTreeSet::new();
    let mut last = 0;
    let m = conform(&v, &t, Options::default(), |p| {
        assert!(p.steiner >= last);
        last = p.steiner;
        phases.insert(format!("{:?}", p.phase));
        true
    })
    .unwrap();
    assert!((volume_of(&m.points, &m.cells) - 4.).abs() < 1e-12);
    assert!(m.report.volume_error_relative < 1e-12);
    assert!(phases.contains(&format!("{:?}", Phase::Extraction)));
    // every sub-facet is a face of some extracted cell, exactly once
    let faces: Vec<[usize; 3]> = m
        .cells
        .iter()
        .flat_map(|&[a, b, c, d]| [[a, b, c], [a, b, d], [a, c, d], [b, c, d]])
        .map(face_key)
        .collect();
    for (f, parent) in &m.facets {
        assert!(*parent < t.len());
        assert_eq!(faces.iter().filter(|k| **k == face_key(*f)).count(), 1);
    }
    // sub-facets cover the input facets: total area is unchanged
    let area = |tri: [usize; 3]| {
        let [a, b, c] = tri.map(|i| m.points[i]);
        let u: V3 = std::array::from_fn(|i| b[i] - a[i]);
        let w: V3 = std::array::from_fn(|i| c[i] - a[i]);
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.
    };
    let total: f64 = m.facets.iter().map(|(f, _)| area(*f)).sum();
    assert!((total - 16.).abs() < 1e-9, "area {total}");
    assert!(m.report.max_steiner_offset_relative < 1e-14);
}

#[test]
fn ring_plate_with_a_through_hole_excludes_the_hole_from_the_cells() {
    let (v, t) = plate(4., Some(2.), 1.);
    let m = conform(&v, &t, Options::default(), |_| true).unwrap();
    assert!((volume_of(&m.points, &m.cells) - 12.).abs() < 1e-10);
    for c in &m.cells {
        let centroid: V3 =
            std::array::from_fn(|i| c.iter().map(|&k| m.points[k][i]).sum::<f64>() / 4.);
        assert!(
            !((centroid[0] - 2.).abs() < 1. - 1e-9 && (centroid[1] - 2.).abs() < 1. - 1e-9),
            "cell inside the hole"
        );
    }
    assert!(m.report.interior_cells < m.report.hull_cells);
    assert!(m.report.segment_steiner + m.report.facet_steiner > 0);
}

#[test]
fn invalid_shells_budgets_and_cancellation_refuse() {
    let (v, t) = plate(4., Some(2.), 1.);
    // open shell
    let open = &t[..t.len() - 1];
    assert_eq!(
        conform(&v, open, Options::default(), |_| true)
            .unwrap_err()
            .kind,
        ErrorKind::Topology
    );
    // dangling vertex index
    let mut bad = t.clone();
    bad[0][0] = 99;
    assert!(conform(&v, &bad, Options::default(), |_| true).is_err());
    // inward-oriented shell has negative enclosed volume
    let flipped: Vec<_> = t.iter().map(|f| [f[0], f[2], f[1]]).collect();
    assert!(conform(&v, &flipped, Options::default(), |_| true).is_err());
    // Steiner budget
    let tiny = Options {
        max_steiner: 0,
        ..Options::default()
    };
    assert_eq!(
        conform(&v, &t, tiny, |_| true).unwrap_err().kind,
        ErrorKind::Limit
    );
    // cancellation at the start and mid-way
    assert_eq!(
        conform(&v, &t, Options::default(), |_| false)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    let mut n = 0;
    assert_eq!(
        conform(&v, &t, Options::default(), |_| {
            n += 1;
            n < 3
        })
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
}

#[test]
fn missing_only_mode_also_recovers_the_ring_and_extracts_the_same_volume() {
    let (v, t) = plate(4., Some(2.), 1.);
    let options = Options {
        protect_encroached: false,
        ..Options::default()
    };
    let m = conform(&v, &t, options, |_| true).unwrap();
    assert!((volume_of(&m.points, &m.cells) - 12.).abs() < 1e-10);
    let full = conform(&v, &t, Options::default(), |_| true).unwrap();
    assert!(
        m.report.segment_steiner + m.report.facet_steiner
            <= full.report.segment_steiner + full.report.facet_steiner
    );
}

#[test]
fn a_strongly_graded_input_exceeds_a_small_steiner_budget_instead_of_looping() {
    // 400:1 thickness-to-width grading needs far more than a handful of points.
    let (v, t) = plate(4., Some(2.), 0.01);
    let small = Options {
        max_steiner: 20,
        unprotected_fallback: false,
        ..Options::default()
    };
    assert_eq!(
        conform(&v, &t, small, |_| true).unwrap_err().kind,
        ErrorKind::Limit
    );
}

/// The protect-everything recovery that runs out of budget is repeated once splitting only what is missing; on this input
/// that needs 6 points, and the result is verified like any other (exact volume, every sub-facet bounds the cells once).
#[test]
fn the_unprotected_fallback_recovers_what_the_protected_recovery_gives_up_on() {
    let (v, t) = plate(4., Some(2.), 0.01);
    let small = Options { max_steiner: 20, ..Options::default() };
    assert!(small.unprotected_fallback);
    let m = conform(&v, &t, small, |_| true).expect("the fallback recovers the plate");
    assert!((volume_of(&m.points, &m.cells) - (16. - 4.) * 0.01).abs() < 1e-12);
    assert!(m.report.segment_steiner + m.report.facet_steiner <= 20);
    // opting out restores the refusal; a recovery that succeeds first time is identical with and without the option
    let off = Options { unprotected_fallback: false, ..small };
    assert_eq!(conform(&v, &t, off, |_| true).unwrap_err().kind, ErrorKind::Limit);
    let (v, t) = plate(3., None, 1.);
    let a = conform(&v, &t, Options::default(), |_| true).unwrap();
    let b = conform(&v, &t, Options { unprotected_fallback: false, ..Options::default() }, |_| true).unwrap();
    assert_eq!((a.points, a.cells), (b.points, b.cells));
}

/// The protected recovery gives up at `protected_max_steiner` and the unprotected one takes over: the result is exactly the
/// unprotected recovery of the same input.
#[test]
fn the_protected_step_limit_hands_over_to_the_unprotected_recovery() {
    let (v, t) = plate(4., Some(2.), 0.01);
    let protected = conform(&v, &t, Options { unprotected_fallback: false, ..Options::default() }, |_| true).unwrap();
    let used = protected.report.segment_steiner + protected.report.facet_steiner;
    assert!(used > 6, "the protected recovery of this plate needs more than the unprotected one: {used}");
    let unprotected = conform(&v, &t, Options { protect_encroached: false, ..Options::default() }, |_| true).unwrap();
    let limited = Options { protected_max_steiner: used - 1, ..Options::default() };
    let handed_over = conform(&v, &t, limited, |_| true).unwrap();
    assert_eq!((&handed_over.points, &handed_over.cells), (&unprotected.points, &unprotected.cells));
    // at the exact count it is still the protected recovery
    let exact = conform(&v, &t, Options { protected_max_steiner: used, ..Options::default() }, |_| true).unwrap();
    assert_eq!((&exact.points, &exact.cells), (&protected.points, &protected.cells));
    // the limit does not apply without the fallback
    let off = Options { unprotected_fallback: false, protected_max_steiner: 1, ..Options::default() };
    assert!(conform(&v, &t, off, |_| true).is_ok());
}

/// A coordinate that is the rounding residue of a placement (-6e-31 instead of 0) is below the exact predicates' range;
/// it used to refuse the whole shell with "3D predicate coordinates must be zero or have magnitude in [1e-30, 1e30]".
#[test]
fn a_coordinate_below_the_predicate_range_is_flushed_to_zero() {
    let (mut v, t) = plate(3., None, 1.);
    v[0][2] = -5.9e-31;
    v[1][2] = 3e-40;
    let m = conform(&v, &t, Options::default(), |_| true).expect("a residue of 1e-31 must not refuse the shell");
    assert!((volume_of(&m.points, &m.cells) - 9.).abs() < 1e-10);
    assert!(m.points.iter().flatten().all(|x| *x == 0. || x.abs() >= 1e-30));
}

#[test]
fn radius_edge_refinement_bounds_the_slab_cells_and_keeps_the_volume() {
    let (v, t) = plate(3., None, 1.);
    let base = conform(&v, &t, Options::default(), |_| true).unwrap();
    let options = Options {
        max_radius_edge: Some(2.),
        ..Options::default()
    };
    let m = conform(&v, &t, options, |_| true).unwrap();
    assert!((volume_of(&m.points, &m.cells) - 9.).abs() < 1e-10);
    assert!(m.report.interior_cells > base.report.interior_cells);
    assert_eq!(m.report.unresolved_bad_cells, 0);
    assert!(
        m.report.max_radius_edge_ratio <= 2. + 1e-9,
        "ratio {}",
        m.report.max_radius_edge_ratio
    );
    assert!(m.report.max_radius_edge_ratio < base.report.max_radius_edge_ratio);
    // the boundary is unchanged: every sub-facet still bounds the cells once
    let faces: Vec<[usize; 3]> = m
        .cells
        .iter()
        .flat_map(|&[a, b, c, d]| [[a, b, c], [a, b, d], [a, c, d], [b, c, d]])
        .map(face_key)
        .collect();
    for (f, _) in &m.facets {
        assert_eq!(faces.iter().filter(|k| **k == face_key(*f)).count(), 1);
    }
}

#[test]
fn a_thin_slab_with_seven_degree_face_diagonals_refuses_instead_of_descending_forever() {
    // The 4:0.5 side walls are triangulated with diagonals at 7 degrees to the
    // box edges. Plain recovery succeeds, but quality refinement toward those
    // corners does not terminate (known limit): it must hit the budget.
    let (v, t) = plate(4., None, 0.5);
    assert!(conform(&v, &t, Options::default(), |_| true).is_ok());
    let options = Options {
        max_steiner: 300,
        max_radius_edge: Some(2.),
        ..Options::default()
    };
    // It stops either at the Steiner budget or at the feature-size floor, whichever the cascade meets first.
    let e = conform(&v, &t, options, |_| true).unwrap_err();
    assert!(
        e.kind == ErrorKind::Limit || e.detail.contains("feature-size floor"),
        "{e:?}"
    );
}

#[test]
fn reconnection_keeps_the_boundary_and_volume_and_never_worsens_quality() {
    use fea_core::meshing::tet_flip::FlipOptions;
    let (v, t) = plate(4., Some(2.), 1.);
    let plain = conform(&v, &t, Options::default(), |_| true).unwrap();
    let options = Options {
        reconnect: Some(FlipOptions::default()),
        ..Options::default()
    };
    let m = conform(&v, &t, options, |_| true).unwrap();
    let report = m.report.reconnection.as_ref().expect("reconnection ran");
    assert!(report.min_radius_ratio_after >= report.min_radius_ratio_before);
    assert!(report.cells_below_target_after <= report.cells_below_target_before);
    assert_eq!(m.points, plain.points);
    assert!((volume_of(&m.points, &m.cells) - 12.).abs() < 1e-10);
    assert_eq!(m.facets, plain.facets);
    let faces: Vec<[usize; 3]> = m
        .cells
        .iter()
        .flat_map(|&[a, b, c, d]| [[a, b, c], [a, b, d], [a, c, d], [b, c, d]])
        .map(face_key)
        .collect();
    for (f, _) in &m.facets {
        assert_eq!(faces.iter().filter(|k| **k == face_key(*f)).count(), 1);
    }
}

#[test]
fn reconnection_lets_the_recovered_slab_pass_the_solver_gate_and_a_tension_patch() {
    use fea_core::{meshing::tet_flip::FlipOptions, Elastic, Loads, Model, SurfaceLoad};
    let (v, t) = plate(3., None, 1.);
    let base = Options {
        max_radius_edge: Some(2.),
        ..Options::default()
    };
    // Without reconnection a sliver (scaled Jacobian 0.049) fails the solver gate.
    let raw = conform(&v, &t, base, |_| true).unwrap();
    assert!(raw.to_solver_mesh(Elastic::new(1000., 0.)).is_err());
    let options = Options {
        reconnect: Some(FlipOptions::default()),
        ..base
    };
    let m = conform(&v, &t, options, |_| true).unwrap();
    // E = 1000, nu = 0, uniform traction 10 on the top: u_z = 0.01 z everywhere.
    let mesh = m.to_solver_mesh(Elastic::new(1000., 0.)).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for (i, p) in model.mesh.nodes.iter().enumerate() {
        if p[2] == 0. {
            bc.fix_node(i);
        }
    }
    let mut loads = Loads::default();
    for face in &model.mesh.surfaces["boundary"] {
        if face.iter().all(|&i| model.mesh.nodes[i][2] == 1.) {
            loads
                .faces
                .push((face.clone(), SurfaceLoad::Traction([0., 0., 10.])));
        }
    }
    assert!(!loads.faces.is_empty());
    let solved = model.solve_static(&loads, &bc).unwrap();
    for (i, p) in model.mesh.nodes.iter().enumerate() {
        assert!(
            (solved.u[3 * i + 2] - 0.01 * p[2]).abs() < 1e-9,
            "node {i} at {p:?}: {}",
            solved.u[3 * i + 2]
        );
    }
}

#[test]
fn the_shell_angle_measure_matches_the_known_inputs() {
    use solid_kernel::plc_conform::min_shell_angle_deg;
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let (v, t) = plate(2., None, 2.);
    assert!(near(min_shell_angle_deg(&v, &t), 45.), "cube");
    // 3 x 3 x 1: side-wall diagonals sit at atan(1/3) to the box edges.
    let (v, t) = plate(3., None, 1.);
    assert!(near(
        min_shell_angle_deg(&v, &t),
        (1f64 / 3.).atan().to_degrees()
    ));
    // 4 x 4 x 0.5: atan(0.5/4) = 7.1 degrees.
    let (v, t) = plate(4., None, 0.5);
    assert!(near(
        min_shell_angle_deg(&v, &t),
        (0.5f64 / 4.).atan().to_degrees()
    ));
    // The ring plate: 18.4 degree trapezoid corners, but its 4 x 1 outer walls
    // put diagonals at atan(1/4); the hole walls are 90 degree dihedrals.
    let (v, t) = plate(4., Some(2.), 1.);
    assert!(near(
        min_shell_angle_deg(&v, &t),
        0.25f64.atan().to_degrees()
    ));
}

/// Outward-oriented triangulated prism over a regular `n`-gon (circumradius `r`, height `h`): all its points are cospherical
/// pairs on the hull of its vertices, the case a cylinder is made of.
fn n_gon_prism(n: usize, r: f64, h: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let mut v = Vec::new();
    for z in [0., h] {
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            v.push([r * a.cos(), r * a.sin(), z]);
        }
    }
    let mut t = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        t.push([i, j, n + j]);
        t.push([i, n + j, n + i]);
    }
    for i in 1..n - 1 {
        t.push([n, n + i, n + i + 1]); // top, +z
        t.push([0, i + 1, i]); // bottom, -z
    }
    (v, t)
}

#[test]
fn a_prism_over_a_regular_polygon_recovers_although_every_point_is_on_the_hull() {
    // Segment splits of rim chords and lateral facets are rounded a few ulps outside the hull of the vertices; the engine used
    // to dent its hull on each and refuse a later point that was a whole millimetre outside the dented hull.
    for (n, r, h) in [(22, 2., 60.), (24, 2., 60.), (40, 5., 12.)] {
        let (v, t) = n_gon_prism(n, r, h);
        let options = Options {
            max_steiner: 20_000,
            ..Options::default()
        };
        let m = conform(&v, &t, options, |_| true)
            .unwrap_or_else(|e| panic!("{n}-gon prism of height {h}: {e}"));
        let exact = 0.5 * n as f64 * r * r * (std::f64::consts::TAU / n as f64).sin() * h;
        let got = volume_of(&m.points, &m.cells);
        assert!((got / exact - 1.).abs() < 1e-9, "{got} vs {exact}");
        // The far points that make every shell point interior are not part of the result.
        assert!(m.cells.iter().flatten().all(|&i| i < m.points.len()));
        assert_eq!(
            m.points.len(),
            v.len() + m.report.segment_steiner + m.report.facet_steiner
        );
    }
}

/// A cube [lo, hi]^3 as twelve triangles, outward (`outward = true`) or turned inside out (a cavity wall).
fn cube_shell(lo: f64, hi: f64, outward: bool, v: &mut Vec<V3>, f: &mut Vec<[usize; 3]>) {
    let base = v.len();
    for k in 0..8 {
        v.push([
            if k & 1 == 0 { lo } else { hi },
            if k & 2 == 0 { lo } else { hi },
            if k & 4 == 0 { lo } else { hi },
        ]);
    }
    for q in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
        let [a, b, c, d] = q.map(|i| base + i);
        if outward {
            f.extend([[a, b, c], [a, c, d]]);
        } else {
            f.extend([[a, c, b], [a, d, c]]);
        }
    }
}

#[test]
fn a_closed_cavity_inside_the_shell_is_not_filled() {
    // the region between an outer cube and an inner cubic cavity (the cavity wall oriented into the cavity): the cells between
    // the two shells only, so the volume is 4^3 - 2^3 = 56 exactly and no cell lies in the cavity
    let (mut v, mut f) = (Vec::new(), Vec::new());
    cube_shell(0., 4., true, &mut v, &mut f);
    cube_shell(1., 3., false, &mut v, &mut f);
    let c = conform(&v, &f, Options::default(), |_| true).unwrap();
    let volume: f64 = c
        .cells
        .iter()
        .map(|t| {
            let p = t.map(|i| c.points[i]);
            let d = |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            let (a, b, e) = (d(p[1], p[0]), d(p[2], p[0]), d(p[3], p[0]));
            (a[0] * (b[1] * e[2] - b[2] * e[1]) - a[1] * (b[0] * e[2] - b[2] * e[0]) + a[2] * (b[0] * e[1] - b[1] * e[0])) / 6.
        })
        .sum();
    assert!((volume - 56.).abs() < 1e-9, "{volume}");
    assert!(c.cells.iter().all(|t| {
        // no cell centroid inside the cavity
        let m: V3 = std::array::from_fn(|k| t.iter().map(|&i| c.points[i][k]).sum::<f64>() / 4.);
        !(m.iter().all(|x| *x > 1. && *x < 3.))
    }));
}

#[test]
fn every_returned_point_belongs_to_a_cell_a_facet_or_a_segment() {
    // an input vertex keeps its index; a Steiner point that nothing uses is not returned (it would be a node of no element)
    let (mut v, mut f) = (Vec::new(), Vec::new());
    cube_shell(0., 4., true, &mut v, &mut f);
    cube_shell(1., 3., false, &mut v, &mut f);
    let options = Options { max_radius_edge: Some(2.0), protect_encroached: false, ..Options::default() };
    let c = conform(&v, &f, options, |_| true).unwrap();
    assert_eq!(&c.points[..v.len()], &v[..], "input vertices keep their numbers");
    let mut used = vec![false; c.points.len()];
    c.cells.iter().flatten().for_each(|&i| used[i] = true);
    c.facets.iter().flat_map(|(f, _)| f).for_each(|&i| used[i] = true);
    c.segments.iter().flatten().for_each(|&i| used[i] = true);
    let unused: Vec<usize> = (v.len()..c.points.len()).filter(|&i| !used[i]).collect();
    assert!(unused.is_empty(), "unused Steiner points {unused:?}");
}

fn solver_gate(c: &solid_kernel::plc_conform::Conforming) -> (f64, usize, Result<(), String>) {
    use fea_core::{Elastic, ElementKind, Mesh, Physics};
    let material = Elastic { e: 200e3, nu: 0.3, alpha: 0., aniso: None };
    let mut mesh = Mesh::new(Physics::Solid);
    mesh.nodes = c.points.clone();
    mesh.add_block(ElementKind::Tet4, c.cells.iter().flatten().copied().collect(), material, "t").unwrap();
    let q = mesh.quality();
    let below = q.elements.iter().filter(|e| e.scaled_jacobian < 0.1).count();
    (q.min_scaled_jacobian, below, c.to_solver_mesh(material).map(|_| ()).map_err(|e| e.to_string()))
}

#[test]
fn sliver_refinement_removes_most_cells_under_the_solver_gate_and_terminates() {
    // A square ring plate: the plain recovery leaves tetrahedra of four boundary vertices, flat to rounding, under the solver
    // gate's scaled Jacobian (79 of 435 here, one of them inverted for the gate). The sliver stage removes all but a few
    // (they sit at the floor of the input resolution), keeps the volume, and ends (a livelock between a refused segment
    // split and the sub-facet that asked for it once made it run for ever).
    let (v, t) = plate(4., Some(2.), 1.);
    let plain = Options { max_steiner: 20_000, ..Options::default() };
    let refined = Options { min_scaled_jacobian: Some(0.2), reconnect: Some(Default::default()), smooth_rounds: 5, ..plain };
    let a = conform(&v, &t, plain, |_| true).unwrap();
    let b = conform(&v, &t, refined, |_| true).unwrap();
    let (_, below_plain, _) = solver_gate(&a);
    let (_, below_refined, _) = solver_gate(&b);
    assert!(below_plain >= 40, "{below_plain}");
    assert!(below_refined * 8 <= below_plain, "{below_refined} of {} cells vs {below_plain}", b.cells.len());
    assert!(b.cells.len() < 3 * a.cells.len(), "{} vs {}", b.cells.len(), a.cells.len());
    let exact = 4. * 4. * 1. - 2. * 2. * 1.;
    assert!((volume_of(&b.points, &b.cells) - exact).abs() < 1e-9);
    // a box already passes and is not touched
    let (v, t) = plate(4., None, 1.);
    let c = conform(&v, &t, refined, |_| true).unwrap();
    assert_eq!(c.cells.len(), 17);
    assert!(solver_gate(&c).2.is_ok());
}
