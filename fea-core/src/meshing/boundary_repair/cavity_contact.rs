//! Exact common-simplex contact for nondegenerate, unoriented tetrahedra.
//!
//! Every vertex of their closed intersection is a contained original vertex or
//! a transverse edge/face contact. A coplanar edge crossing another edge also
//! crosses one of that edge's incident face planes. Both checks use shared IDs.
//! Within a simplex, an edge meets a face of that simplex only in their common
//! vertices' hull, so each edge/face test can use its own shared IDs.

use crate::meshing::{
    tet_topology::orientation,
    MeshError,
};
use std::cmp::Ordering::{self, Equal, Greater, Less};

const FACES: [[usize; 3]; 4] = [[1, 2, 3], [0, 3, 2], [0, 1, 3], [0, 2, 1]];
const EDGES: [[usize; 2]; 6] = [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]];

/// Whether the closed intersection equals the hull of the shared vertex IDs.
/// Invalid indices, repeated IDs, flat cells and unsupported coordinates error.
/// Cell ordering is immaterial; rejecting duplicate cells belongs to the caller.
pub(super) fn compatible(points: &[[f64; 3]], a: [usize; 4], b: [usize; 4]) -> Result<bool, MeshError> {
    let mut signs = [Equal; 2];
    for (i, t) in [a, b].into_iter().enumerate() {
        if t.iter().any(|&v| v >= points.len()) {
            return Err(MeshError::NonConforming(
                "tetrahedron node index out of range".into(),
            ));
        }
        if (0..4).any(|j| t[j + 1..].contains(&t[j])) {
            return Err(MeshError::NonConforming(
                "repeated tetrahedron node".into(),
            ));
        }
        let [p, q, r, s] = t.map(|v| points[v]);
        signs[i] = orientation(p, q, r, s)?;
        if signs[i] == Equal {
            return Err(MeshError::InvalidElements {
                count: 1,
                first: (0, i),
            });
        }
    }

    for (source, target, sign) in [(a, b, signs[1]), (b, a, signs[0])] {
        for v in source {
            if !target.contains(&v) && in_tet(points, v, target, sign)? {
                return Ok(false);
            }
        }
        for edge in EDGES.map(|e| e.map(|i| source[i])) {
            for face in FACES.map(|f| f.map(|i| target[i])) {
                if !edge_face_compatible(points, edge, face)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn in_tet(
    points: &[[f64; 3]],
    v: usize,
    t: [usize; 4],
    sign: Ordering,
) -> Result<bool, MeshError> {
    for i in 0..4 {
        let mut replaced = t;
        replaced[i] = v;
        let [a, b, c, d] = replaced.map(|j| points[j]);
        let side = orientation(a, b, c, d)?;
        if side != Equal && side != sign {
            return Ok(false);
        }
    }
    Ok(true)
}

fn edge_face_compatible(
    points: &[[f64; 3]],
    edge: [usize; 2],
    face: [usize; 3],
) -> Result<bool, MeshError> {
    let [p, q] = edge.map(|v| points[v]);
    let [a, b, c] = face.map(|v| points[v]);
    let sp = orientation(a, b, c, p)?;
    let sq = orientation(a, b, c, q)?;
    if sp == sq {
        return Ok(true);
    }
    // A noncoplanar segment meets this plane at most once.
    if (sp == Equal && face.contains(&edge[0])) || (sq == Equal && face.contains(&edge[1])) {
        return Ok(true);
    }
    let sides = [
        orientation(p, q, a, b)?,
        orientation(p, q, b, c)?,
        orientation(p, q, c, a)?,
    ];
    Ok(!same_closed_half_plane(sides))
}

fn same_closed_half_plane(sides: [Ordering; 3]) -> bool {
    sides.iter().all(|&s| s != Less) || sides.iter().all(|&s| s != Greater)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [usize; 4] = [0, 1, 2, 3];
    const BASE: [[f64; 3]; 4] = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [0., 0., 2.]];

    fn with_points(extra: &[[f64; 3]]) -> Vec<[f64; 3]> {
        BASE.into_iter().chain(extra.iter().copied()).collect()
    }

    fn check(points: &[[f64; 3]], a: [usize; 4], b: [usize; 4], expected: bool) {
        assert_eq!(compatible(points, a, b).unwrap(), expected, "{a:?}, {b:?}");
        assert_eq!(compatible(points, b, a).unwrap(), expected, "{b:?}, {a:?}");
    }

    fn permutations(t: [usize; 4]) -> Vec<[usize; 4]> {
        let mut result = Vec::new();
        for &a in &t {
            for &b in &t {
                for &c in &t {
                    for &d in &t {
                        if a != b && a != c && a != d && b != c && b != d && c != d {
                            result.push([a, b, c, d]);
                        }
                    }
                }
            }
        }
        result
    }

    #[test]
    fn accepts_disjoint_tets_even_with_overlapping_coordinate_bounds() {
        let p = with_points(&[[2., 2., 2.], [0.5, 2., 2.], [2., 0.5, 2.], [2., 2., 0.5]]);
        check(&p, A, [4, 5, 6, 7], true);
    }

    #[test]
    fn accepts_exact_face_edge_and_vertex_contacts() {
        let p = with_points(&[[0., 0., -2.], [0., -2., 0.], [-2., 0., 0.]]);
        for b in [[0, 1, 2, 4], [0, 1, 4, 5], [0, 4, 5, 6]] {
            check(&p, A, b, true);
        }
        check(&p, A, A, true);
    }

    #[test]
    fn rejects_strict_containment_in_either_direction() {
        let p = with_points(&[
            [0.25, 0.25, 0.25],
            [0.5, 0.25, 0.25],
            [0.25, 0.5, 0.25],
            [0.25, 0.25, 0.5],
        ]);
        check(&p, A, [4, 5, 6, 7], false);
    }

    #[test]
    fn rejects_nonshared_vertices_on_faces_and_edges() {
        let face = with_points(&[
            [0.5, 0.5, 0.],
            [0.5, 0.5, -1.],
            [1., 0.5, -1.],
            [0.5, 1., -1.],
        ]);
        check(&face, A, [4, 5, 6, 7], false);
        let edge = with_points(&[[1., 0., 0.], [1., -1., 0.], [1., 0., -1.], [2., -1., -1.]]);
        check(&edge, A, [4, 5, 6, 7], false);
    }

    #[test]
    fn rejects_duplicate_coordinates_with_different_ids_including_signed_zero() {
        let p = with_points(&[[-0., 0., -0.], [-2., 0., 0.], [0., -2., 0.], [0., 0., -2.]]);
        check(&p, A, [4, 5, 6, 7], false);
        let p = with_points(&BASE);
        check(&p, A, [4, 5, 6, 7], false);
    }

    #[test]
    fn rejects_overlap_beyond_a_shared_face_edge_or_vertex() {
        let face = with_points(&[[2., 2., 2.]]);
        check(&face, A, [0, 1, 2, 4], false);
        let edge = with_points(&[[0., 2., 2.], [0., -2., 1.]]);
        check(&edge, A, [0, 1, 4, 5], false);
        let vertex = with_points(&[[2., 2., -1.], [2., -1., 2.], [-1., 2., 2.]]);
        check(&vertex, A, [0, 4, 5, 6], false);
    }

    #[test]
    fn rejects_edge_face_crossings_with_no_contained_vertices() {
        let p = [
            [1., 1., 1.],
            [1., -1., -1.],
            [-1., 1., -1.],
            [-1., -1., 1.],
            [-0.75, -0.875, -1.],
            [-0.75, 1.125, 1.],
            [1.25, -0.875, 1.],
            [1.25, 1.125, -1.],
        ];
        for (a, b) in [(A, [4, 5, 6, 7]), ([4, 5, 6, 7], A)] {
            let [w, x, y, z] = b.map(|i| p[i]);
            let sign = orientation(w, x, y, z).unwrap();
            assert!(a.into_iter().all(|v| !in_tet(&p, v, b, sign).unwrap()));
        }
        check(&p, A, [4, 5, 6, 7], false);
    }

    #[test]
    fn rejects_coplanar_partial_face_overlap_with_no_contained_vertices() {
        let p = [
            [-2., -1., 0.],
            [2., -1., 0.],
            [0., 2., 0.],
            [0., 0., 2.],
            [-2., 1., 0.],
            [2., 1., 0.],
            [0., -2., 0.],
            [0., 0., -2.],
        ];
        for axes in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
            let p = p.map(|v| axes.map(|i| v[i]));
            check(&p, A, [4, 5, 6, 7], false);
        }
    }

    #[test]
    fn rejects_nonincident_edges_that_touch_at_one_point() {
        let p = with_points(&[[1., -1., 1.], [1., 1., -1.], [0., -1., -1.], [2., -1., -1.]]);
        for a in permutations(A) {
            for b in permutations([4, 5, 6, 7]) {
                check(&p, a, b, false);
            }
        }
    }

    #[test]
    fn contact_is_invariant_under_all_vertex_permutations() {
        let p = with_points(&[
            [0., 0., -2.],
            [0., -2., 0.],
            [-2., 0., 0.],
            [2., 2., 2.],
            [0., 2., 2.],
            [0., -2., 1.],
            [2., 2., -1.],
            [2., -1., 2.],
            [-1., 2., 2.],
        ]);
        for (b, expected) in [
            ([0, 1, 2, 4], true),
            ([0, 1, 4, 5], true),
            ([0, 4, 5, 6], true),
            ([0, 1, 2, 7], false),
            ([0, 1, 8, 9], false),
            ([0, 10, 11, 12], false),
        ] {
            for a in permutations(A) {
                for b in permutations(b) {
                    check(&p, a, b, expected);
                }
            }
        }
    }

    #[test]
    fn contact_is_invariant_under_exact_translations_and_shears() {
        let p = with_points(&[[0., 0., -2.], [0., -2., 0.], [-2., 0., 0.], [2., 2., 2.]]);
        for offset in [[0., 0., 0.], [8., -4., 16.], [-0.5, 0.25, -0.125]] {
            for axes in [[0, 1, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
                let p: Vec<_> = p
                    .iter()
                    .map(|v| {
                        let q = [v[0] + v[1], v[1] + v[2], v[2]];
                        std::array::from_fn(|i| q[axes[i]] + offset[i])
                    })
                    .collect();
                for (b, expected) in [
                    ([0, 1, 2, 4], true),
                    ([0, 1, 4, 5], true),
                    ([0, 4, 5, 6], true),
                    ([0, 1, 2, 7], false),
                ] {
                    check(&p, A, b, expected);
                }
            }
        }
    }

    #[test]
    fn distinguishes_nearly_coplanar_contacts_without_a_tolerance() {
        let h = 2_f64.powi(-90);
        let p = [
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [0., 0., h],
            [2., 2., -h],
            [2., 2., h],
        ];
        check(&p, A, [0, 1, 2, 4], true);
        check(&p, A, [0, 1, 2, 5], false);

        let h = 2_f64.powi(-40);
        let p = [
            [0., 0., 0.],
            [1., 0., 1.],
            [0., 1., 1.],
            [1., 1., 2. + h],
            [1., 1., 2. - h],
            [0., 0., h],
        ];
        for offset in [[0., 0., 0.], [8., -4., 16.]] {
            let p = p.map(|v| std::array::from_fn(|i| v[i] + offset[i]));
            for a in permutations(A) {
                check(&p, a, [0, 1, 2, 4], true);
                check(&p, a, [0, 1, 2, 5], false);
            }
        }
    }

    #[test]
    fn refuses_invalid_cells_and_predicate_ranges_before_contact_checks() {
        for b in [[0, 1, 2, 99], [0, 1, 2, 2]] {
            assert!(matches!(compatible(&BASE, A, b), Err(MeshError::NonConforming(_))));
        }
        let p = with_points(&[[0.5, 0.5, 0.]]);
        assert!(matches!(compatible(&p, A, [0, 1, 2, 4]), Err(MeshError::InvalidElements { .. })));

        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e31, -1e31, 2_f64.powi(-100)] {
            let p = with_points(&[[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [0., 0., value]]);
            for (a, b) in [(A, [4, 5, 6, 7]), ([4, 5, 6, 7], A)] {
                assert!(matches!(compatible(&p, a, b), Err(MeshError::Unsupported(_))), "{value:?}");
            }
        }
    }
}
