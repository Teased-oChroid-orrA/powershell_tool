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
        ..Options::default()
    };
    assert_eq!(
        conform(&v, &t, small, |_| true).unwrap_err().kind,
        ErrorKind::Limit
    );
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
    assert_eq!(
        conform(&v, &t, options, |_| true).unwrap_err().kind,
        ErrorKind::Limit
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
