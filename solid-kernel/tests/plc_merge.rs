//! `Options::merge_coplanar`: the diagonals of a planar face (a CAD face sampled into triangles) are not features, so
//! they must not have to be recovered. Without merging, the long thin side faces below start a cascade of ever smaller
//! splits along their diagonals and the recovery refuses at the feature-size floor.
use solid_kernel::{
    plc_conform::{conform, Options},
    V3,
};

/// Outward prism: `profile` (counter-clockwise polygon in the y-z plane, as seen from +x) extruded along x over
/// `[0, length]`. Every side wall is a rectangle split by its diagonal; the two caps use `cap`, a counter-clockwise
/// triangulation of the profile.
fn prism(profile: &[[f64; 2]], cap: &[[usize; 3]], length: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let n = profile.len();
    let mut v: Vec<V3> = profile.iter().map(|p| [0., p[0], p[1]]).collect();
    v.extend(profile.iter().map(|p| [length, p[0], p[1]]));
    let mut t = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        // wall between profile edge i-j: outward is to the right of the counter-clockwise walk
        t.push([i, n + j, n + i]);
        t.push([i, j, n + j]);
    }
    for &[a, b, c] in cap {
        t.push([a, c, b]); // x = 0 cap faces -x
        t.push([n + a, n + b, n + c]); // x = length cap faces +x
    }
    (v, t)
}
fn area(points: &[V3], f: [usize; 3]) -> f64 {
    let [a, b, c] = f.map(|i| points[i]);
    let (u, w): (V3, V3) = (std::array::from_fn(|i| b[i] - a[i]), std::array::from_fn(|i| c[i] - a[i]));
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.
}
fn edge(a: usize, b: usize) -> [usize; 2] {
    [a.min(b), a.max(b)]
}

/// A 2667 x 762 x 101.6 bar with a ramped end. Its side walls are long thin rectangles, and the diagonal of each meets
/// the wall's edges at 1 to 8 degrees.
fn ramp() -> (Vec<V3>, Vec<[usize; 3]>, f64) {
    let profile = [[0., 0.], [2667., 0.], [2667., 101.6], [2514.6, 50.8], [0., 50.8]];
    let (v, t) = prism(&profile, &[[0, 1, 3], [0, 3, 4], [1, 2, 3]], 762.);
    let shoelace: f64 = (0..profile.len())
        .map(|i| {
            let (p, q) = (profile[i], profile[(i + 1) % profile.len()]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
        / 2.;
    (v, t, shoelace * 762.)
}

#[test]
fn a_planar_face_diagonal_no_longer_starts_a_cascade_when_coplanar_facets_merge() {
    let (v, t, volume) = ramp();
    let options = Options { max_steiner: 20_000, max_cells: 2_000_000, unprotected_fallback: false, ..Options::default() };
    let e = conform(&v, &t, options, |_| true).unwrap_err();
    assert!(e.detail.contains("feature-size floor"), "the unmerged recovery should hit the floor: {e:?}");
    let merged = Options { merge_coplanar: true, ..options };
    let c = conform(&v, &t, merged, |_| true).unwrap();
    let cells: f64 = c
        .cells
        .iter()
        .map(|q| {
            let [a, b, cc, d] = q.map(|i| c.points[i]);
            let (u, w, x): (V3, V3, V3) = (
                std::array::from_fn(|i| a[i] - d[i]),
                std::array::from_fn(|i| b[i] - d[i]),
                std::array::from_fn(|i| cc[i] - d[i]),
            );
            (u[0] * (w[1] * x[2] - w[2] * x[1]) - u[1] * (w[0] * x[2] - w[2] * x[0]) + u[2] * (w[0] * x[1] - w[1] * x[0])).abs() / 6.
        })
        .sum();
    assert!((cells - volume).abs() / volume < 1e-10, "cells {cells} analytic {volume}");
    // the sub-facets tile the shell: same total area, each assigned to an input triangle it lies in
    let input_area: f64 = t.iter().map(|&f| area(&v, f)).sum();
    let output_area: f64 = c.facets.iter().map(|(f, _)| area(&c.points, *f)).sum();
    assert!((input_area - output_area).abs() / input_area < 1e-12, "{input_area} {output_area}");
    for (f, parent) in &c.facets {
        assert!(*parent < t.len());
        let centroid: V3 = std::array::from_fn(|i| (c.points[f[0]][i] + c.points[f[1]][i] + c.points[f[2]][i]) / 3.);
        let [a, b, d] = t[*parent].map(|i| v[i]);
        let sub = |p: V3, q: V3| -> V3 { std::array::from_fn(|i| p[i] - q[i]) };
        let cross = |p: V3, q: V3| -> V3 { [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]] };
        let dot = |p: V3, q: V3| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
        let n = cross(sub(b, a), sub(d, a));
        let inside = [(a, b), (b, d), (d, a)].iter().all(|&(p, q)| dot(cross(sub(q, p), sub(centroid, p)), n) >= -1e-9 * dot(n, n));
        assert!(inside, "sub-facet centroid {centroid:?} is outside its parent {:?}", t[*parent]);
    }
    // no input diagonal is a segment of the merged recovery (only the wall and cap boundaries are)
    let diagonals: Vec<[usize; 2]> = (0..5).map(|i| edge(i, 5 + (i + 1) % 5)).collect();
    for d in diagonals {
        assert!(!c.segments.contains(&d), "diagonal {d:?} was kept as a segment");
    }
}

#[test]
fn merging_a_box_keeps_the_volume_and_drops_the_twelve_face_diagonals() {
    let (v, t) = prism(&[[0., 0.], [3., 0.], [3., 2.], [0., 2.]], &[[0, 1, 2], [0, 2, 3]], 5.);
    let plain = conform(&v, &t, Options::default(), |_| true).unwrap();
    let merged = conform(&v, &t, Options { merge_coplanar: true, ..Options::default() }, |_| true).unwrap();
    let volume = |c: &solid_kernel::plc_conform::Conforming| {
        c.cells
            .iter()
            .map(|q| {
                let [a, b, cc, d] = q.map(|i| c.points[i]);
                let (u, w, x): (V3, V3, V3) = (
                    std::array::from_fn(|i| a[i] - d[i]),
                    std::array::from_fn(|i| b[i] - d[i]),
                    std::array::from_fn(|i| cc[i] - d[i]),
                );
                (u[0] * (w[1] * x[2] - w[2] * x[1]) - u[1] * (w[0] * x[2] - w[2] * x[0]) + u[2] * (w[0] * x[1] - w[1] * x[0])).abs() / 6.
            })
            .sum::<f64>()
    };
    assert!((volume(&plain) - 30.).abs() < 1e-10 && (volume(&merged) - 30.).abs() < 1e-10);
    // a segment lies on a box edge when both ends share two coordinates at the box extremes (a face diagonal shares one)
    let extremes = [[0., 5.], [0., 3.], [0., 2.]];
    let on_edge = |c: &solid_kernel::plc_conform::Conforming, s: &[usize; 2]| {
        let (a, b) = (c.points[s[0]], c.points[s[1]]);
        (0..3).filter(|&k| a[k] == b[k] && extremes[k].contains(&a[k])).count() >= 2
    };
    assert!(plain.segments.iter().any(|s| !on_edge(&plain, s)), "the unmerged recovery keeps the face diagonals");
    assert!(merged.segments.iter().all(|s| on_edge(&merged, s)), "only box edges are segments after merging");
    assert!(merged.report.segment_steiner + merged.report.facet_steiner <= plain.report.segment_steiner + plain.report.facet_steiner);
}
