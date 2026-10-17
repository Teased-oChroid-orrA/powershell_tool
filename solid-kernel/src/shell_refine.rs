//! Steiner refinement of a closed, outward-oriented triangulated shell that never changes its shape.
//!
//! Needle triangles that survive cleanup, weld and defeaturing are chords across coarsely sampled arcs on flat faces: the
//! polygon turns by under a degree at the arc's vertices, and no triangulation of the face with only those vertices avoids a
//! thin triangle. Extra points on the surface do. This pass adds them:
//!
//! 1. every triangle whose smallest corner is below `target_angle_deg` bisects its longest edge at the midpoint (the edge's
//!    other triangle splits with it, so the shell stays conforming and closed); one split per neighbourhood per round;
//! 2. edges between *coplanar* triangles (the opposite vertex of one lies within `coplanar_tolerance` of the plane of the
//!    other) are flipped when they violate the Delaunay condition and the quad is strictly convex.
//!
//! Midpoints lie on existing straight edges and flips stay inside one plane, so the surface, the enclosed volume and the area
//! are unchanged (up to rounding; both are re-checked). Dihedral angles are untouched, so the shell's smallest *input* angle
//! can improve only up to the sharpest feature angle the part really has; the report gives both.
//!
//! Refinement stops at the target, at the budget of new vertices, or when a round makes no progress (reported as not reached).

use crate::plc_conform::min_shell_angle_deg;
use crate::shell_patch::{
    area, check_closed, cross, dot, min_corner_angle_deg, norm, signed_volume, sub,
};
use crate::V3;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Triangles with a corner below this are refined.
    pub target_angle_deg: f64,
    /// New vertices allowed, as a multiple of the input vertex count (at least 16).
    pub max_new_vertex_factor: f64,
    /// Distance, as a fraction of the bounding-box diagonal, within which a fourth vertex counts as in the plane.
    pub coplanar_tolerance: f64,
    pub max_rounds: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            target_angle_deg: 20.,
            max_new_vertex_factor: 4.,
            coplanar_tolerance: 1e-9,
            max_rounds: 200,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rounds: usize,
    pub vertices_added: usize,
    pub flips: usize,
    pub reached_target: bool,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub facets_before: usize,
    pub facets_after: usize,
    pub min_corner_angle_before_deg: f64,
    pub min_corner_angle_after_deg: f64,
    /// Smallest input angle (corner or dihedral) as the registry gate measures it.
    pub min_shell_angle_before_deg: f64,
    pub min_shell_angle_after_deg: f64,
}

#[derive(Debug, Clone)]
pub struct Refined {
    pub vertices: Vec<V3>,
    pub facets: Vec<[usize; 3]>,
    pub report: Report,
}

fn mid(a: V3, b: V3) -> V3 {
    [
        0.5 * (a[0] + b[0]),
        0.5 * (a[1] + b[1]),
        0.5 * (a[2] + b[2]),
    ]
}

fn tri_angle(vertices: &[V3], f: [usize; 3]) -> f64 {
    min_corner_angle_deg(vertices, &[f])
}

/// Flip edges between coplanar triangles that violate the Delaunay condition (a work queue: a flip re-examines only the four
/// edges around it). Returns the number of flips.
fn flip_coplanar(vertices: &[V3], tris: &mut [[usize; 3]], tol: f64) -> usize {
    use std::collections::{HashMap, VecDeque};
    let mut at: HashMap<[usize; 2], (usize, usize)> = HashMap::with_capacity(tris.len() * 3);
    for (t, f) in tris.iter().enumerate() {
        for k in 0..3 {
            at.insert([f[k], f[(k + 1) % 3]], (t, k));
        }
    }
    let mut queue: VecDeque<[usize; 2]> = at.keys().filter(|e| e[0] < e[1]).copied().collect();
    let mut queued: std::collections::HashSet<[usize; 2]> = queue.iter().copied().collect();
    let mut total = 0;
    let limit = 64 * tris.len() + 64;
    let mut steps = 0;
    while let Some([a, b]) = queue.pop_front() {
        queued.remove(&[a, b]);
        steps += 1;
        if steps > limit {
            break;
        }
        let (Some(&(t, k)), Some(&(u, l))) = (at.get(&[a, b]), at.get(&[b, a])) else {
            continue;
        };
        let (f, g) = (tris[t], tris[u]);
        if f[k] != a || f[(k + 1) % 3] != b || g[l] != b || g[(l + 1) % 3] != a {
            continue;
        }
        let (c, d) = (f[(k + 2) % 3], g[(l + 2) % 3]);
        let (pa, pb, pc, pd) = (vertices[a], vertices[b], vertices[c], vertices[d]);
        // Coplanar: d within `tol` of the plane of (a, b, c).
        let n = cross(sub(pb, pa), sub(pc, pa));
        let nl = norm(n);
        if nl <= 0. || (dot(n, sub(pd, pa)) / nl).abs() > tol {
            continue;
        }
        // Delaunay: the angles at c and d (opposite the shared edge) sum to more than pi.
        let angle_at = |p: V3, q: V3, r: V3| {
            let (u, w) = (sub(q, p), sub(r, p));
            (dot(u, w) / (norm(u) * norm(w))).clamp(-1., 1.).acos()
        };
        if angle_at(pc, pa, pb) + angle_at(pd, pb, pa) <= std::f64::consts::PI * (1. + 1e-12) {
            continue;
        }
        // The new diagonal c-d must lie inside the quad a, d, b, c: both new triangles keep the orientation of the old ones.
        let (n1, n2) = (
            cross(sub(pa, pc), sub(pd, pc)),
            cross(sub(pd, pc), sub(pb, pc)),
        );
        if dot(n1, n) <= 0. || dot(n2, n) <= 0. {
            continue;
        }
        if at.contains_key(&[c, d]) || at.contains_key(&[d, c]) {
            continue;
        }
        for e in [[a, b], [b, c], [c, a], [b, a], [a, d], [d, b]] {
            at.remove(&e);
        }
        tris[t] = [c, a, d];
        tris[u] = [c, d, b];
        for (tt, f) in [(t, tris[t]), (u, tris[u])] {
            for k in 0..3 {
                at.insert([f[k], f[(k + 1) % 3]], (tt, k));
            }
        }
        total += 1;
        for e in [[a, d], [d, b], [b, c], [c, a]] {
            let key = [e[0].min(e[1]), e[0].max(e[1])];
            if queued.insert(key) {
                queue.push_back(key);
            }
        }
    }
    total
}

/// Refine a shell until every triangle's smallest corner reaches the target, within the budget. See the module documentation.
pub fn refine_surface(
    vertices: &[V3],
    facets: &[[usize; 3]],
    options: Options,
) -> Result<Refined, String> {
    check_closed(facets)?;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for v in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let diag = norm(sub(hi, lo));
    if !(diag.is_finite() && diag > 0.) {
        return Err("degenerate bounding box".into());
    }
    let tol = options.coplanar_tolerance * diag;
    let budget = ((options.max_new_vertex_factor * vertices.len() as f64) as usize).max(16);
    let (volume0, area0): (f64, f64) = (
        signed_volume(vertices, facets),
        facets.iter().map(|&f| area(vertices, f)).sum(),
    );
    let mut v: Vec<V3> = vertices.to_vec();
    let mut tris: Vec<[usize; 3]> = facets.to_vec();
    let mut report = Report {
        rounds: 0,
        vertices_added: 0,
        flips: 0,
        reached_target: false,
        vertices_before: vertices.len(),
        vertices_after: vertices.len(),
        facets_before: facets.len(),
        facets_after: facets.len(),
        min_corner_angle_before_deg: min_corner_angle_deg(vertices, facets),
        min_corner_angle_after_deg: 0.,
        min_shell_angle_before_deg: min_shell_angle_deg(vertices, facets),
        min_shell_angle_after_deg: 0.,
    };
    for _ in 0..options.max_rounds {
        let mut bad: Vec<(f64, usize)> = tris
            .iter()
            .enumerate()
            .map(|(t, &f)| (tri_angle(&v, f), t))
            .filter(|(a, _)| *a < options.target_angle_deg)
            .collect();
        if bad.is_empty() {
            report.reached_target = true;
            break;
        }
        bad.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut owner: BTreeMap<[usize; 2], usize> = BTreeMap::new();
        for (t, f) in tris.iter().enumerate() {
            for k in 0..3 {
                owner.insert([f[k], f[(k + 1) % 3]], t);
            }
        }
        let mut used = vec![false; tris.len()];
        let mut next: Vec<[usize; 3]> = Vec::with_capacity(tris.len() + 2 * bad.len());
        let mut replaced = vec![false; tris.len()];
        let mut added = 0;
        let longest = |v: &[V3], f: [usize; 3]| -> usize {
            (0..3)
                .max_by(|&i, &j| {
                    let (li, lj) = (
                        norm(sub(v[f[i]], v[f[(i + 1) % 3]])),
                        norm(sub(v[f[j]], v[f[(j + 1) % 3]])),
                    );
                    li.total_cmp(&lj)
                        .then(f[j].min(f[(j + 1) % 3]).cmp(&f[i].min(f[(i + 1) % 3])))
                })
                .expect("three edges")
        };
        for (_, start) in bad {
            if used[start] || report.vertices_added + added >= budget {
                continue;
            }
            // Rivara's terminal edge: follow the longest edges across neighbours until both sides agree, so each split halves
            // an edge that is the longest of every triangle it touches and the grading stays bounded.
            let (mut t, mut guard) = (start, 0);
            let (a, b, c, u) = loop {
                let f = tris[t];
                let k = longest(&v, f);
                let (a, b, c) = (f[k], f[(k + 1) % 3], f[(k + 2) % 3]);
                let Some(&u) = owner.get(&[b, a]) else {
                    break (a, b, c, usize::MAX);
                };
                let g = tris[u];
                let ku = longest(&v, g);
                let terminal = g[ku] == b && g[(ku + 1) % 3] == a;
                guard += 1;
                if terminal || guard > 64 {
                    break (a, b, c, u);
                }
                t = u;
            };
            if u == usize::MAX || used[t] || used[u] {
                continue;
            }
            let d = *tris[u]
                .iter()
                .find(|&&w| w != a && w != b)
                .expect("a third vertex");
            let m = v.len();
            used[t] = true;
            used[u] = true;
            replaced[t] = true;
            replaced[u] = true;
            next.push([a, m, c]);
            next.push([m, b, c]);
            next.push([b, m, d]);
            next.push([m, a, d]);
            v.push(mid(v[a], v[b]));
            added += 1;
        }
        if added == 0 {
            break;
        }
        // `v` grew while indices were handed out as `v.len() + added`: those were computed before each push, so they match.
        let mut rebuilt: Vec<[usize; 3]> = tris
            .iter()
            .zip(&replaced)
            .filter(|(_, r)| !**r)
            .map(|(f, _)| *f)
            .collect();
        rebuilt.extend(next);
        tris = rebuilt;
        report.vertices_added += added;
        report.flips += flip_coplanar(&v, &mut tris, tol);
        report.rounds += 1;
    }
    check_closed(&tris).map_err(|e| format!("refinement produced an invalid shell: {e}"))?;
    let (volume1, area1): (f64, f64) = (
        signed_volume(&v, &tris),
        tris.iter().map(|&f| area(&v, f)).sum(),
    );
    if (volume1 - volume0).abs() > 1e-9 * volume0.abs() + 1e-12 * diag.powi(3)
        || (area1 - area0).abs() > 1e-9 * area0
    {
        return Err(format!(
            "refinement changed the shape: volume {volume0} -> {volume1}, area {area0} -> {area1}"
        ));
    }
    report.vertices_after = v.len();
    report.facets_after = tris.len();
    report.min_corner_angle_after_deg = min_corner_angle_deg(&v, &tris);
    report.min_shell_angle_after_deg = min_shell_angle_deg(&v, &tris);
    if !report.reached_target {
        report.reached_target = report.min_corner_angle_after_deg >= options.target_angle_deg;
    }
    Ok(Refined {
        vertices: v,
        facets: tris,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_patch::tests::grid_box;

    /// A prism of height `h` over the polygon `(0,0) (s/2,-bulge) (s,0) (s,s) (0,s)`, caps fanned from the first vertex: the
    /// first triangle `(0,0) (s/2,-bulge) (s,0)` is a needle of height `bulge` over a base `s` lying inside the face.
    fn arc_chord_box(s: f64, bulge: f64, h: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
        let poly = [[0., 0.], [s / 2., -bulge], [s, 0.], [s, s], [0., s]];
        let n = poly.len();
        let mut v: Vec<V3> = poly.iter().map(|p| [p[0], p[1], 0.]).collect();
        v.extend(poly.iter().map(|p| [p[0], p[1], h]));
        let mut f = Vec::new();
        for k in 1..n - 1 {
            f.push([0, k + 1, k]); // bottom, outward normal -z
            f.push([n, n + k, n + k + 1]); // top, outward normal +z
        }
        for k in 0..n {
            let (a, b) = (k, (k + 1) % n);
            f.push([a, b, n + b]);
            f.push([a, n + b, n + a]);
        }
        (v, f)
    }

    #[test]
    fn needles_across_a_bent_edge_are_refined_without_changing_the_shape() {
        let (v, f) = arc_chord_box(10., 0.05, 10.);
        check_closed(&f).unwrap();
        let before = min_corner_angle_deg(&v, &f);
        assert!(before < 2., "{before}");
        let r = refine_surface(
            &v,
            &f,
            Options {
                max_new_vertex_factor: 300.,
                ..Options::default()
            },
        )
        .unwrap();
        eprintln!("{:?}", r.report);
        assert!(r.report.reached_target, "{:?}", r.report);
        assert!(r.report.min_corner_angle_after_deg >= 20., "{:?}", r.report);
        assert!(r.report.vertices_added > 0 && r.report.facets_after > r.report.facets_before);
        assert!((signed_volume(&r.vertices, &r.facets) - signed_volume(&v, &f)).abs() < 1e-9);
        let area_of = |v: &[V3], f: &[[usize; 3]]| f.iter().map(|&t| area(v, t)).sum::<f64>();
        assert!((area_of(&r.vertices, &r.facets) - area_of(&v, &f)).abs() < 1e-9);
        check_closed(&r.facets).unwrap();
        // The original vertices are untouched.
        assert_eq!(&r.vertices[..v.len()], &v[..]);
    }

    #[test]
    fn healthy_shells_open_shells_and_tiny_budgets_are_handled() {
        let (v, f) = grid_box(2, 2, 2);
        let r = refine_surface(&v, &f, Options::default()).unwrap();
        assert_eq!((r.report.vertices_added, r.report.rounds), (0, 0));
        assert_eq!(r.facets, f);
        let (v, mut f) = grid_box(1, 1, 1);
        f.pop();
        assert!(refine_surface(&v, &f, Options::default()).is_err());
        // With no rounds allowed nothing changes and the report says the target was not reached; it never claims success.
        let (v, f) = arc_chord_box(10., 0.01, 10.);
        let r = refine_surface(
            &v,
            &f,
            Options {
                max_rounds: 0,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(
            !r.report.reached_target && r.report.vertices_added == 0 && r.facets == f,
            "{:?}",
            r.report
        );
        // A budget too small for the work is reported the same way and leaves a valid shell.
        let r = refine_surface(
            &v,
            &f,
            Options {
                max_new_vertex_factor: 0.0,
                target_angle_deg: 89.,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(
            !r.report.reached_target && r.report.vertices_added <= 16,
            "{:?}",
            r.report
        );
        check_closed(&r.facets).unwrap();
    }

    #[test]
    fn refinement_is_deterministic_and_never_lowers_the_shell_angle() {
        let (v, f) = arc_chord_box(12., 0.1, 12.);
        let (a, b) = (
            refine_surface(&v, &f, Options::default()).unwrap(),
            refine_surface(&v, &f, Options::default()).unwrap(),
        );
        assert_eq!(a.facets, b.facets);
        assert_eq!(a.vertices, b.vertices);
        assert!(a.report.min_shell_angle_after_deg >= a.report.min_shell_angle_before_deg - 1e-12);
    }
}
