//! Flat-patch cleanup of a closed, outward-oriented triangulated shell.
//!
//! A sampled CAD shell often carries vertices that add nothing to the geometry: points strung along a straight edge between
//! two flat faces, and points inside a flat face. Triangles spanned by such points are needles (the NEMA17 bracket shell has
//! a 0.012 degree corner), and a needle is exactly what makes constrained Delaunay recovery diverge. This pass dissolves
//! those vertices and re-triangulates each affected flat patch with a constrained Delaunay triangulation of its boundary
//! polygon (holes bridged), without adding any point.
//!
//! * A *patch* is a set of triangles connected through flat edges. An edge is flat when the opposite vertex of the smaller
//!   triangle lies within `tolerance` of the plane of the larger one and on the far side of the edge (so folded flaps are
//!   creases). The plane of a patch is that of its largest triangle; a patch with a vertex off it is left alone.
//! * A vertex is *dissolvable* when it is interior to one patch, or when it lies on exactly two crease edges that are
//!   collinear (a point on a straight feature line). Corners and bends are never removed.
//! * Patches that lose no vertex are copied unchanged. Any patch whose boundary cannot be turned into a clean polygon
//!   (pinched loops, several outer loops, no ear) keeps its dissolvable vertices and the whole pass is repeated.
//!
//! The result is verified: every directed edge occurs once with its reverse, the signed volume and the area of every
//! changed patch are unchanged to `1e-9` relative, and no triangle is degenerate.

use crate::V3;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Absolute distance within which points count as coplanar, as a fraction of the bounding-box diagonal.
    pub relative_tolerance: f64,
    /// A vertex on a feature line is removable when it lies within this distance (fraction of the bounding-box diagonal) of
    /// the chord between its two neighbours on the line, and between them. The geometry changes by at most this much.
    pub collinear_distance: f64,
    /// Re-triangulating a patch may reduce its smallest corner angle, but never below `min(old, acceptable_angle_deg)`:
    /// angles above this are considered healthy, anything thinner must not get thinner.
    pub acceptable_angle_deg: f64,
    pub max_rounds: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            relative_tolerance: 1e-9,
            collinear_distance: 1e-7,
            acceptable_angle_deg: 10.,
            max_rounds: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rounds: usize,
    pub patches: usize,
    pub patches_retriangulated: usize,
    pub patches_kept_whole: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub facets_before: usize,
    pub facets_after: usize,
    pub min_angle_before_deg: f64,
    pub min_angle_after_deg: f64,
}

#[derive(Debug, Clone)]
pub struct Cleaned {
    pub vertices: Vec<V3>,
    pub facets: Vec<[usize; 3]>,
    pub report: Report,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// Smallest corner angle of the triangles, in degrees.
pub fn min_corner_angle_deg(vertices: &[V3], facets: &[[usize; 3]]) -> f64 {
    let mut smallest = std::f64::consts::PI;
    for f in facets {
        let [a, b, c] = f.map(|v| vertices[v]);
        for (p, q, r) in [(a, b, c), (b, c, a), (c, a, b)] {
            let (u, w) = (sub(q, p), sub(r, p));
            let d = norm(u) * norm(w);
            if d > 0. {
                smallest = smallest.min((dot(u, w) / d).clamp(-1., 1.).acos());
            } else {
                return 0.;
            }
        }
    }
    smallest.to_degrees()
}

/// The `n` triangles with the smallest corner angle: `(angle in degrees, facet, its three points)`. A diagnostic for
/// locating the needles that block constrained recovery.
pub fn worst_triangles(
    vertices: &[V3],
    facets: &[[usize; 3]],
    n: usize,
) -> Vec<(f64, [usize; 3], [V3; 3])> {
    let mut all: Vec<(f64, [usize; 3], [V3; 3])> = facets
        .iter()
        .map(|&f| {
            (
                min_corner_angle_deg(vertices, &[f]),
                f,
                f.map(|v| vertices[v]),
            )
        })
        .collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    all.truncate(n);
    all
}

fn signed_volume(vertices: &[V3], facets: &[[usize; 3]]) -> f64 {
    facets
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(|v| vertices[v]);
            dot(a, cross(b, c)) / 6.
        })
        .sum()
}

fn area(vertices: &[V3], f: [usize; 3]) -> f64 {
    let [a, b, c] = f.map(|v| vertices[v]);
    0.5 * norm(cross(sub(b, a), sub(c, a)))
}

fn check_closed(facets: &[[usize; 3]]) -> Result<(), String> {
    let mut directed = BTreeMap::<[usize; 2], usize>::new();
    for f in facets {
        if f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
            return Err("degenerate facet with a repeated vertex".into());
        }
        for k in 0..3 {
            *directed.entry([f[k], f[(k + 1) % 3]]).or_default() += 1;
        }
    }
    for (e, n) in &directed {
        if *n != 1 || directed.get(&[e[1], e[0]]) != Some(&1) {
            return Err(format!(
                "shell is not a closed oriented 2-manifold at edge {e:?}"
            ));
        }
    }
    Ok(())
}

/// Dissolve flat and collinear vertices. See the module documentation. Returns `Err` when the input is not a closed oriented
/// manifold or when the result fails its own verification (the input is then untouched by definition: nothing is returned).
pub fn dissolve_flat(
    vertices: &[V3],
    facets: &[[usize; 3]],
    options: Options,
) -> Result<Cleaned, String> {
    check_closed(facets)?;
    let before_angle = min_corner_angle_deg(vertices, facets);
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    for round in 1..=options.max_rounds {
        match attempt(vertices, facets, options, &keep) {
            Ok((v, f, mut report)) => {
                report.rounds = round;
                report.min_angle_before_deg = before_angle;
                return Ok(Cleaned {
                    vertices: v,
                    facets: f,
                    report,
                });
            }
            Err(Failure::Patch(vs)) => {
                let before = keep.len();
                keep.extend(vs);
                if keep.len() == before {
                    return Err("a patch failed again without new vertices to keep".into());
                }
            }
            Err(Failure::Fatal(e)) => return Err(e),
        }
    }
    Err("flat-patch cleanup did not stabilise".into())
}

enum Failure {
    /// Vertices of patches that could not be re-triangulated; keep them and retry.
    Patch(Vec<usize>),
    Fatal(String),
}

type Edge = [usize; 2];
/// A successful round: the new vertices, the new facets and the (partly filled) report.
type Attempt = (Vec<V3>, Vec<[usize; 3]>, Report);

fn attempt(
    vertices: &[V3],
    facets: &[[usize; 3]],
    options: Options,
    keep: &BTreeSet<usize>,
) -> Result<Attempt, Failure> {
    let fatal = |s: &str| Failure::Fatal(s.to_string());
    // Bounding-box diagonal and absolute tolerance.
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for v in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let diag = norm(sub(hi, lo));
    if !(diag.is_finite() && diag > 0.) {
        return Err(fatal("degenerate bounding box"));
    }
    let tol = options.relative_tolerance * diag;
    let collinear_tol = options.collinear_distance * diag;
    let n_tri = facets.len();
    // Edge -> owning triangle (directed).
    let mut owner = BTreeMap::<Edge, usize>::new();
    for (t, f) in facets.iter().enumerate() {
        for k in 0..3 {
            owner.insert([f[k], f[(k + 1) % 3]], t);
        }
    }
    let area_vec: Vec<V3> = facets
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(|v| vertices[v]);
            cross(sub(b, a), sub(c, a))
        })
        .collect();
    let tri_area: Vec<f64> = area_vec.iter().map(|n| 0.5 * norm(*n)).collect();
    // Union-find over flat edges.
    let mut parent: Vec<usize> = (0..n_tri).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let flat_edge = |t1: usize, t2: usize, a: usize, b: usize| -> bool {
        let (big, small) = if tri_area[t1] >= tri_area[t2] {
            (t1, t2)
        } else {
            (t2, t1)
        };
        if tri_area[big] <= tol * tol {
            return false;
        }
        let nb = area_vec[big];
        let nlen = norm(nb);
        let n = [nb[0] / nlen, nb[1] / nlen, nb[2] / nlen];
        let opp = |t: usize| {
            *facets[t]
                .iter()
                .find(|&&v| v != a && v != b)
                .expect("a triangle has a third vertex")
        };
        let (ob, os) = (vertices[opp(big)], vertices[opp(small)]);
        let (pa, pb) = (vertices[a], vertices[b]);
        if dot(n, sub(os, pa)).abs() > tol {
            return false;
        }
        // Opposite sides of the shared edge within the plane.
        let e = sub(pb, pa);
        let side = |p: V3| dot(cross(e, sub(p, pa)), n);
        side(ob) * side(os) < 0.
    };
    for (t, f) in facets.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            let Some(&u) = owner.get(&[b, a]) else {
                return Err(fatal("open edge"));
            };
            if u > t && flat_edge(t, u, a, b) {
                let (ra, rb) = (find(&mut parent, t), find(&mut parent, u));
                parent[ra] = rb;
            }
        }
    }
    let patch_of: Vec<usize> = (0..n_tri).map(|t| find(&mut parent, t)).collect();
    let mut patches = BTreeMap::<usize, Vec<usize>>::new();
    for (t, p) in patch_of.iter().enumerate() {
        patches.entry(*p).or_default().push(t);
    }
    // Reference plane per patch (largest triangle) and a flatness verification of all its vertices.
    let mut plane = BTreeMap::<usize, (V3, V3)>::new(); // patch -> (unit normal, origin)
    let mut flat_patch = BTreeSet::<usize>::new();
    for (p, ts) in &patches {
        let big = *ts
            .iter()
            .max_by(|a, b| tri_area[**a].total_cmp(&tri_area[**b]))
            .expect("non-empty patch");
        if tri_area[big] <= tol * tol {
            continue;
        }
        let nb = area_vec[big];
        let nlen = norm(nb);
        let n = [nb[0] / nlen, nb[1] / nlen, nb[2] / nlen];
        let origin = vertices[facets[big][0]];
        let ok = ts.iter().all(|&t| {
            facets[t]
                .iter()
                .all(|&v| dot(n, sub(vertices[v], origin)).abs() <= 4. * tol)
        });
        if ok {
            plane.insert(*p, (n, origin));
            flat_patch.insert(*p);
        }
    }
    // Crease edges around each vertex.
    let mut incident = BTreeMap::<usize, Vec<usize>>::new();
    for (t, f) in facets.iter().enumerate() {
        for &v in f {
            incident.entry(v).or_default().push(t);
        }
    }
    let is_crease = |a: usize, b: usize| -> bool {
        let (Some(&t1), Some(&t2)) = (owner.get(&[a, b]), owner.get(&[b, a])) else {
            return true;
        };
        let (p1, p2) = (patch_of[t1], patch_of[t2]);
        !(p1 == p2 && flat_patch.contains(&p1))
    };
    let mut removable = BTreeSet::<usize>::new();
    for (&v, ts) in &incident {
        if keep.contains(&v) {
            continue;
        }
        // Every incident triangle must belong to a flat patch.
        if !ts.iter().all(|&t| flat_patch.contains(&patch_of[t])) {
            continue;
        }
        let mut neighbours = BTreeSet::new();
        for &t in ts {
            for &w in &facets[t] {
                if w != v {
                    neighbours.insert(w);
                }
            }
        }
        let creases: Vec<usize> = neighbours
            .iter()
            .copied()
            .filter(|&w| is_crease(v, w))
            .collect();
        let ok = match creases.len() {
            0 => true,
            2 => {
                let (u, w) = (
                    sub(vertices[creases[0]], vertices[v]),
                    sub(vertices[creases[1]], vertices[v]),
                );
                let chord = sub(vertices[creases[1]], vertices[creases[0]]);
                let lc = norm(chord);
                // Distance of v from the chord, and v strictly between the two neighbours.
                lc > 0.
                    && dot(u, w) < 0.
                    && dot(u, chord) < 0.
                    && dot(w, chord) > 0.
                    && norm(cross(u, chord)) / lc <= collinear_tol
            }
            _ => false,
        };
        if ok {
            removable.insert(v);
        }
    }
    // Which patches lose a vertex?
    let mut touched = BTreeSet::<usize>::new();
    for &v in &removable {
        for &t in &incident[&v] {
            touched.insert(patch_of[t]);
        }
    }
    let mut new_facets: Vec<[usize; 3]> = Vec::new();
    let mut failed: Vec<usize> = Vec::new();
    let mut retriangulated = 0;
    for (p, ts) in &patches {
        if !touched.contains(p) {
            new_facets.extend(ts.iter().map(|&t| facets[t]));
            continue;
        }
        let (n, origin) = plane[p];
        match retriangulate(vertices, facets, ts, &patch_of, *p, &removable, n, origin) {
            Some(tris) => {
                let before: f64 = ts.iter().map(|&t| tri_area[t]).sum();
                let after: f64 = tris.iter().map(|&f| area(vertices, f)).sum();
                let patch_edge: BTreeSet<(usize, usize)> = ts
                    .iter()
                    .flat_map(|&t| {
                        let f = facets[t];
                        (0..3).map(move |k| (f[k], f[(k + 1) % 3]))
                    })
                    .collect();
                let perimeter: f64 = ts
                    .iter()
                    .flat_map(|&t| {
                        let f = facets[t];
                        (0..3).map(move |k| (f[k], f[(k + 1) % 3]))
                    })
                    .filter(|(a, b)| !patch_edge.contains(&(*b, *a)))
                    .map(|(a, b)| norm(sub(vertices[a], vertices[b])))
                    .sum();
                if (before - after).abs()
                    > 1e-9 * before.max(f64::MIN_POSITIVE) + 4. * collinear_tol * perimeter
                    || worsens_min_angle(vertices, facets, ts, &tris, options.acceptable_angle_deg)
                {
                    failed.extend(removed_in(facets, ts, &removable));
                } else {
                    retriangulated += 1;
                    new_facets.extend(tris);
                }
            }
            None => failed.extend(removed_in(facets, ts, &removable)),
        }
    }
    if !failed.is_empty() {
        return Err(Failure::Patch(failed));
    }
    // Compact.
    let mut used = BTreeSet::new();
    for f in &new_facets {
        used.extend(f.iter().copied());
    }
    let mut map = BTreeMap::new();
    let mut out_v = Vec::new();
    for (i, v) in vertices.iter().enumerate() {
        if used.contains(&i) {
            map.insert(i, out_v.len());
            out_v.push(*v);
        }
    }
    let out_f: Vec<[usize; 3]> = new_facets.iter().map(|f| f.map(|v| map[&v])).collect();
    // Verification of the whole shell.
    check_closed(&out_f).map_err(|e| Failure::Fatal(format!("result invalid: {e}")))?;
    let (v0, v1) = (
        signed_volume(vertices, facets),
        signed_volume(&out_v, &out_f),
    );
    let surface: f64 = facets.iter().map(|&f| area(vertices, f)).sum();
    if (v0 - v1).abs() > 1e-9 * v0.abs().max(diag.powi(3) * 1e-12) + 4. * collinear_tol * surface {
        return Err(Failure::Fatal(format!("volume changed: {v0} -> {v1}")));
    }
    if out_f.iter().any(|&f| area(&out_v, f) <= 0.) {
        return Err(Failure::Fatal("result has a zero-area facet".into()));
    }
    let report = Report {
        rounds: 0,
        patches: patches.len(),
        patches_retriangulated: retriangulated,
        patches_kept_whole: patches.len() - touched.len(),
        vertices_before: vertices.len(),
        vertices_after: out_v.len(),
        facets_before: facets.len(),
        facets_after: out_f.len(),
        min_angle_before_deg: 0.,
        min_angle_after_deg: min_corner_angle_deg(&out_v, &out_f),
    };
    Ok((out_v, out_f, report))
}

/// Re-triangulating must never create a thinner triangle than the patch had: dropping the points between a needle's ends
/// stretches it.
fn worsens_min_angle(
    vertices: &[V3],
    facets: &[[usize; 3]],
    old: &[usize],
    new: &[[usize; 3]],
    acceptable_deg: f64,
) -> bool {
    let old_tris: Vec<[usize; 3]> = old.iter().map(|&t| facets[t]).collect();
    let floor = min_corner_angle_deg(vertices, &old_tris).min(acceptable_deg);
    min_corner_angle_deg(vertices, new) < floor * (1. - 1e-9)
}

fn removed_in(facets: &[[usize; 3]], ts: &[usize], removable: &BTreeSet<usize>) -> Vec<usize> {
    ts.iter()
        .flat_map(|&t| facets[t])
        .filter(|v| removable.contains(v))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn retriangulate(
    vertices: &[V3],
    facets: &[[usize; 3]],
    ts: &[usize],
    patch_of: &[usize],
    patch: usize,
    removable: &BTreeSet<usize>,
    n: V3,
    origin: V3,
) -> Option<Vec<[usize; 3]>> {
    // Directed boundary edges of the patch.
    let in_patch: BTreeSet<usize> = ts.iter().copied().collect();
    let mut owner = BTreeMap::<Edge, usize>::new();
    for &t in ts {
        let f = facets[t];
        for k in 0..3 {
            owner.insert([f[k], f[(k + 1) % 3]], t);
        }
    }
    let mut next = BTreeMap::<usize, usize>::new();
    for (&[a, b], _) in owner.iter() {
        if !owner.contains_key(&[b, a]) && next.insert(a, b).is_some() {
            return None; // pinched boundary
        }
    }
    let _ = (&in_patch, patch_of, patch);
    // Loops.
    let mut seen = BTreeSet::new();
    let mut loops: Vec<Vec<usize>> = Vec::new();
    for &start in next.keys() {
        if seen.contains(&start) {
            continue;
        }
        let mut ring = vec![start];
        seen.insert(start);
        let mut cur = *next.get(&start)?;
        while cur != start {
            if !seen.insert(cur) {
                return None;
            }
            ring.push(cur);
            cur = *next.get(&cur)?;
        }
        loops.push(ring);
    }
    // Planar coordinates.
    let u = {
        let a = if n[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        };
        let c = cross(n, a);
        let l = norm(c);
        [c[0] / l, c[1] / l, c[2] / l]
    };
    let w = cross(n, u);
    let mut pts2 = BTreeMap::<usize, [f64; 2]>::new();
    for &t in ts {
        for &v in &facets[t] {
            let d = sub(vertices[v], origin);
            pts2.insert(v, [dot(d, u), dot(d, w)]);
        }
    }
    let mut rings: Vec<Vec<usize>> = Vec::new();
    for l in loops {
        let kept: Vec<usize> = l.into_iter().filter(|v| !removable.contains(v)).collect();
        if kept.len() < 3 {
            return None;
        }
        rings.push(kept);
    }
    let signed = |r: &[usize]| -> f64 {
        0.5 * (0..r.len())
            .map(|i| {
                let (p, q) = (pts2[&r[i]], pts2[&r[(i + 1) % r.len()]]);
                p[0] * q[1] - q[0] * p[1]
            })
            .sum::<f64>()
    };
    let areas: Vec<f64> = rings.iter().map(|r| signed(r)).collect();
    let outers: Vec<usize> = (0..rings.len()).filter(|&i| areas[i] > 0.).collect();
    if outers.len() != 1 || areas.contains(&0.) {
        return None;
    }
    let outer = rings[outers[0]].clone();
    let mut holes: Vec<Vec<usize>> = (0..rings.len())
        .filter(|&i| areas[i] < 0.)
        .map(|i| rings[i].clone())
        .collect();
    let constrained: BTreeSet<[usize; 2]> = std::iter::once(&outer)
        .chain(holes.iter())
        .flat_map(|r| {
            (0..r.len()).map(move |i| {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                [a.min(b), a.max(b)]
            })
        })
        .collect();
    // Bridge holes into the outer ring.
    holes.sort_by(|a, b| {
        let mx = |h: &Vec<usize>| {
            h.iter()
                .map(|v| pts2[v][0])
                .fold(f64::NEG_INFINITY, f64::max)
        };
        mx(b).total_cmp(&mx(a))
    });
    let mut ring = outer;
    let all_edges = |ring: &[usize], holes: &[Vec<usize>]| -> Vec<[usize; 2]> {
        let mut e = Vec::new();
        for r in std::iter::once(ring).chain(holes.iter().map(|h| h.as_slice())) {
            for i in 0..r.len() {
                e.push([r[i], r[(i + 1) % r.len()]]);
            }
        }
        e
    };
    while !holes.is_empty() {
        let hole = holes.remove(0);
        // Hole vertex with the largest x.
        let hi = (0..hole.len())
            .max_by(|&a, &b| pts2[&hole[a]][0].total_cmp(&pts2[&hole[b]][0]))
            .expect("hole has vertices");
        let hv = hole[hi];
        let mut cand: Vec<usize> = (0..ring.len()).collect();
        let d2 = |i: usize| {
            let (p, q) = (pts2[&ring[i]], pts2[&hv]);
            (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
        };
        cand.sort_by(|&a, &b| d2(a).total_cmp(&d2(b)));
        let edges = all_edges(&ring, &holes)
            .into_iter()
            .chain((0..hole.len()).map(|i| [hole[i], hole[(i + 1) % hole.len()]]))
            .collect::<Vec<_>>();
        let mut chosen = None;
        for i in cand {
            let ov = ring[i];
            if ov == hv {
                continue;
            }
            let (p, q) = (pts2[&ov], pts2[&hv]);
            let blocked = edges.iter().any(|e| {
                if e.contains(&ov) || e.contains(&hv) {
                    return false;
                }
                segments_cross(p, q, pts2[&e[0]], pts2[&e[1]])
            });
            if blocked {
                continue;
            }
            let mid = [(p[0] + q[0]) / 2., (p[1] + q[1]) / 2.];
            if point_in_ring(mid, &ring, &pts2)
                && !holes.iter().any(|h| point_in_ring(mid, h, &pts2))
                && !point_in_ring(mid, &hole, &pts2)
            {
                chosen = Some(i);
                break;
            }
        }
        let i = chosen?;
        let mut merged = ring[..=i].to_vec();
        for k in 0..=hole.len() {
            merged.push(hole[(hi + k) % hole.len()]);
        }
        merged.push(ring[i]);
        merged.extend_from_slice(&ring[i + 1..]);
        ring = merged;
    }
    let tris = ear_clip(&ring, &pts2)?;
    let tris = lawson(tris, &constrained, &pts2);
    Some(tris)
}

fn orient2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn segments_cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> bool {
    let (d1, d2) = (orient2(p, q, r), orient2(p, q, s));
    let (d3, d4) = (orient2(r, s, p), orient2(r, s, q));
    ((d1 > 0. && d2 < 0.) || (d1 < 0. && d2 > 0.)) && ((d3 > 0. && d4 < 0.) || (d3 < 0. && d4 > 0.))
}

fn point_in_ring(p: [f64; 2], ring: &[usize], pts: &BTreeMap<usize, [f64; 2]>) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let (a, b) = (pts[&ring[i]], pts[&ring[(i + 1) % ring.len()]]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

fn min_angle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let ang = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        let (u, v) = ([q[0] - p[0], q[1] - p[1]], [r[0] - p[0], r[1] - p[1]]);
        let d = (u[0].hypot(u[1])) * (v[0].hypot(v[1]));
        if d == 0. {
            0.
        } else {
            ((u[0] * v[0] + u[1] * v[1]) / d).clamp(-1., 1.).acos()
        }
    };
    ang(a, b, c).min(ang(b, c, a)).min(ang(c, a, b))
}

/// Ear clipping, choosing the ear of best minimum angle at every step. `ring` is counter-clockwise and may repeat vertex
/// ids (hole bridges).
fn ear_clip(ring: &[usize], pts: &BTreeMap<usize, [f64; 2]>) -> Option<Vec<[usize; 3]>> {
    let mut poly: Vec<usize> = ring.to_vec();
    let mut out = Vec::new();
    while poly.len() > 3 {
        let n = poly.len();
        let mut best: Option<(f64, usize)> = None;
        for i in 0..n {
            let (a, b, c) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            if a == b || b == c || a == c {
                continue;
            }
            let (pa, pb, pc) = (pts[&a], pts[&b], pts[&c]);
            if orient2(pa, pb, pc) <= 0. {
                continue;
            }
            let inside = poly.iter().any(|&v| {
                if v == a || v == b || v == c {
                    return false;
                }
                let p = pts[&v];
                if p == pa || p == pb || p == pc {
                    return false;
                }
                orient2(pa, pb, p) >= 0. && orient2(pb, pc, p) >= 0. && orient2(pc, pa, p) >= 0.
            });
            if inside {
                continue;
            }
            let q = min_angle(pa, pb, pc);
            if best.is_none_or(|(bq, _)| q > bq) {
                best = Some((q, i));
            }
        }
        let (_, i) = best?;
        out.push([poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]]);
        poly.remove(i);
    }
    if poly.len() == 3 && orient2(pts[&poly[0]], pts[&poly[1]], pts[&poly[2]]) > 0. {
        out.push([poly[0], poly[1], poly[2]]);
    } else if poly.len() == 3 {
        return None;
    }
    Some(out)
}

/// Lawson flips towards the constrained Delaunay triangulation (edges in `constrained` never flip).
fn lawson(
    mut tris: Vec<[usize; 3]>,
    constrained: &BTreeSet<[usize; 2]>,
    pts: &BTreeMap<usize, [f64; 2]>,
) -> Vec<[usize; 3]> {
    let incircle = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| -> f64 {
        let (ax, ay) = (a[0] - d[0], a[1] - d[1]);
        let (bx, by) = (b[0] - d[0], b[1] - d[1]);
        let (cx, cy) = (c[0] - d[0], c[1] - d[1]);
        (ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay)
            + (cx * cx + cy * cy) * (ax * by - bx * ay)
    };
    for _ in 0..(tris.len() * tris.len() + 16) {
        let mut edge_tri = BTreeMap::<[usize; 2], Vec<(usize, usize)>>::new();
        for (t, f) in tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (f[k], f[(k + 1) % 3]);
                edge_tri
                    .entry([a.min(b), a.max(b)])
                    .or_default()
                    .push((t, k));
            }
        }
        let mut flipped = false;
        for (e, owners) in &edge_tri {
            if owners.len() != 2 || constrained.contains(e) {
                continue;
            }
            let ((t1, k1), (t2, k2)) = (owners[0], owners[1]);
            let (f1, f2) = (tris[t1], tris[t2]);
            let (a, b) = (f1[k1], f1[(k1 + 1) % 3]);
            let c = f1[(k1 + 2) % 3];
            let d = f2[(k2 + 2) % 3];
            if f2[k2] != b || f2[(k2 + 1) % 3] != a {
                continue;
            }
            let (pa, pb, pc, pd) = (pts[&a], pts[&b], pts[&c], pts[&d]);
            // f1 = (a, b, c) counter-clockwise; d must lie strictly inside its circumcircle.
            let scale = {
                let m = [pa, pb, pc, pd]
                    .iter()
                    .flat_map(|p| p.iter().map(|x| x.abs()))
                    .fold(0., f64::max);
                m * m * m * m
            };
            if incircle(pa, pb, pc, pd) <= 1e-12 * scale {
                continue;
            }
            // The quad a, d, b, c must be strictly convex: both new triangles positive.
            if orient2(pc, pd, pa) <= 0. || orient2(pd, pc, pb) <= 0. {
                continue;
            }
            tris[t1] = [c, a, d];
            tris[t2] = [d, b, c];
            flipped = true;
            break;
        }
        if !flipped {
            break;
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outward-oriented box `[0,nx] x [0,ny] x [0,nz]` whose faces are unit-cell grids (every grid point is a vertex).
    fn grid_box(nx: usize, ny: usize, nz: usize) -> (Vec<V3>, Vec<[usize; 3]>) {
        let mut verts = Vec::new();
        let mut index = BTreeMap::<[i64; 3], usize>::new();
        let mut id = |p: [i64; 3], verts: &mut Vec<V3>| {
            *index.entry(p).or_insert_with(|| {
                verts.push([p[0] as f64, p[1] as f64, p[2] as f64]);
                verts.len() - 1
            })
        };
        let mut facets = Vec::new();
        // For each face: fixed axis, side, and the two free axes in an order that makes the normal point outward.
        let dims = [nx as i64, ny as i64, nz as i64];
        for axis in 0..3 {
            for side in [0i64, 1] {
                let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
                for i in 0..dims[a1] {
                    for j in 0..dims[a2] {
                        let mut mk = |di: i64, dj: i64, verts: &mut Vec<V3>| {
                            let mut p = [0i64; 3];
                            p[axis] = side * dims[axis];
                            p[a1] = i + di;
                            p[a2] = j + dj;
                            id(p, verts)
                        };
                        let (p00, p10, p11, p01) = (
                            mk(0, 0, &mut verts),
                            mk(1, 0, &mut verts),
                            mk(1, 1, &mut verts),
                            mk(0, 1, &mut verts),
                        );
                        // (a1, a2, axis) is a right-handed cyclic order, so (p00, p10, p11) points along +axis.
                        let quad = if side == 1 {
                            [[p00, p10, p11], [p00, p11, p01]]
                        } else {
                            [[p00, p11, p10], [p00, p01, p11]]
                        };
                        facets.extend(quad);
                    }
                }
            }
        }
        (verts, facets)
    }

    #[test]
    fn a_gridded_box_collapses_to_twelve_triangles() {
        let (v, f) = grid_box(4, 3, 2);
        assert!(signed_volume(&v, &f) > 0.);
        assert!((signed_volume(&v, &f) - 24.).abs() < 1e-12);
        let c = dissolve_flat(&v, &f, Options::default()).unwrap();
        assert_eq!(
            (c.vertices.len(), c.facets.len()),
            (8, 12),
            "{:?}",
            c.report
        );
        assert!((signed_volume(&c.vertices, &c.facets) - 24.).abs() < 1e-12);
        let a: f64 = c.facets.iter().map(|&t| area(&c.vertices, t)).sum();
        assert!((a - 2. * (12. + 8. + 6.)).abs() < 1e-12);
        // Best possible for the 4 x 2 faces: atan(2/4) = 26.57 degrees.
        assert!(c.report.min_angle_after_deg > 26.5, "{:?}", c.report);
        assert!(c.report.vertices_before > 30);
    }

    #[test]
    fn needles_from_collinear_points_are_removed_and_the_angle_recovers() {
        // Slide a point along a straight edge to 1e-4 from its end: every triangle at it becomes a needle.
        let (mut v, f) = grid_box(4, 4, 1);
        for p in v.iter_mut() {
            if p[2] == 0. && p[1] == 0. && p[0] == 1. {
                p[0] = 1e-4;
            }
        }
        assert!(min_corner_angle_deg(&v, &f) < 0.1);
        let c = dissolve_flat(&v, &f, Options::default()).unwrap();
        assert_eq!((c.vertices.len(), c.facets.len()), (8, 12));
        // Best possible for the 4 x 1 walls: atan(1/4) = 14.04 degrees.
        assert!(c.report.min_angle_after_deg > 14.0, "{:?}", c.report);
        assert!((signed_volume(&c.vertices, &c.facets) - 16.).abs() < 1e-9);
    }

    /// Square annulus prism: outer 4 x 4, hole [1,3]^2, height 1. Faces are unit grids with the hole cells removed.
    fn annulus_prism() -> (Vec<V3>, Vec<[usize; 3]>) {
        let mut verts = Vec::new();
        let mut index = BTreeMap::<[i64; 3], usize>::new();
        let mut id = |p: [i64; 3], verts: &mut Vec<V3>| {
            *index.entry(p).or_insert_with(|| {
                verts.push([p[0] as f64, p[1] as f64, p[2] as f64]);
                verts.len() - 1
            })
        };
        let mut facets = Vec::new();
        // top (z = 1, normal +z) and bottom (z = 0, normal -z)
        for z in [0i64, 1] {
            for i in 0..4i64 {
                for j in 0..4i64 {
                    if (1..3).contains(&i) && (1..3).contains(&j) {
                        continue;
                    }
                    let mut q = |di, dj, verts: &mut Vec<V3>| id([i + di, j + dj, z], verts);
                    let (a, b, c, d) = (
                        q(0, 0, &mut verts),
                        q(1, 0, &mut verts),
                        q(1, 1, &mut verts),
                        q(0, 1, &mut verts),
                    );
                    if z == 1 {
                        facets.extend([[a, b, c], [a, c, d]]);
                    } else {
                        facets.extend([[a, c, b], [a, d, c]]);
                    }
                }
            }
        }
        // Walls: outer ring outward, inner ring facing into the hole (outward from the solid).
        let mut wall = |p0: [i64; 2],
                        dir: [i64; 2],
                        len: i64,
                        outward: bool,
                        verts: &mut Vec<V3>,
                        facets: &mut Vec<[usize; 3]>| {
            for s in 0..len {
                let (x0, y0) = (p0[0] + dir[0] * s, p0[1] + dir[1] * s);
                let (x1, y1) = (x0 + dir[0], y0 + dir[1]);
                let (a, b) = (id([x0, y0, 0], verts), id([x1, y1, 0], verts));
                let (c, d) = (id([x1, y1, 1], verts), id([x0, y0, 1], verts));
                if outward {
                    facets.extend([[a, b, c], [a, c, d]]);
                } else {
                    facets.extend([[a, c, b], [a, d, c]]);
                }
            }
        };
        // Outer boundary counter-clockwise seen from +z: outward normal is to the right of travel.
        wall([0, 0], [1, 0], 4, true, &mut verts, &mut facets);
        wall([4, 0], [0, 1], 4, true, &mut verts, &mut facets);
        wall([4, 4], [-1, 0], 4, true, &mut verts, &mut facets);
        wall([0, 4], [0, -1], 4, true, &mut verts, &mut facets);
        // Hole boundary clockwise seen from +z, so the solid is again on the left of travel.
        wall([1, 1], [0, 1], 2, true, &mut verts, &mut facets);
        wall([1, 3], [1, 0], 2, true, &mut verts, &mut facets);
        wall([3, 3], [0, -1], 2, true, &mut verts, &mut facets);
        wall([3, 1], [-1, 0], 2, true, &mut verts, &mut facets);
        (verts, facets)
    }

    #[test]
    fn a_ring_with_a_hole_is_re_triangulated_with_the_hole_intact() {
        let (v, f) = annulus_prism();
        check_closed(&f).unwrap();
        assert!(
            (signed_volume(&v, &f) - 12.).abs() < 1e-12,
            "{}",
            signed_volume(&v, &f)
        );
        let c = dissolve_flat(&v, &f, Options::default()).unwrap();
        assert_eq!(
            (c.vertices.len(), c.facets.len()),
            (16, 32),
            "{:?}",
            c.report
        );
        assert!((signed_volume(&c.vertices, &c.facets) - 12.).abs() < 1e-12);
        let a: f64 = c.facets.iter().map(|&t| area(&c.vertices, t)).sum();
        assert!((a - 48.).abs() < 1e-12, "area {a}");
        // The 4 x 1 outer walls bound it: atan(1/4) = 14.04 degrees.
        assert!(c.report.min_angle_after_deg >= 14.0, "{:?}", c.report);
        // The hole survives: no vertex inside the hole and the hole corners remain.
        for corner in [[1., 1.], [1., 3.], [3., 1.], [3., 3.]] {
            assert!(c
                .vertices
                .iter()
                .any(|p| p[0] == corner[0] && p[1] == corner[1]));
        }
    }

    #[test]
    fn cleanup_never_makes_the_worst_triangle_worse() {
        // A vertex pulled 1e-3 off its edge line inside the top plane cannot be dissolved; the points around it can.
        // Whatever the pass decides, the shell's thinnest corner must not get thinner.
        let (mut v, f) = grid_box(6, 3, 1);
        for p in v.iter_mut() {
            if p[2] == 1. && p[1] == 0. && p[0] == 5. {
                p[1] = 1e-3;
            }
        }
        let before = min_corner_angle_deg(&v, &f);
        let c = dissolve_flat(&v, &f, Options::default()).unwrap();
        assert!(
            c.report.min_angle_after_deg >= before * (1. - 1e-9),
            "{before} -> {:?}",
            c.report
        );
        assert!((signed_volume(&c.vertices, &c.facets) - signed_volume(&v, &f)).abs() < 1e-6);
    }

    #[test]
    fn a_shell_without_flat_redundancy_is_unchanged() {
        let (v, f) = grid_box(1, 1, 1);
        let c = dissolve_flat(&v, &f, Options::default()).unwrap();
        assert_eq!((c.vertices.len(), c.facets.len()), (8, 12));
        assert_eq!(c.report.patches_retriangulated, 0);
        let mut a = c.facets.clone();
        let mut b = f.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn an_open_shell_is_refused() {
        let (v, mut f) = grid_box(1, 1, 1);
        f.pop();
        assert!(dissolve_flat(&v, &f, Options::default()).is_err());
    }
}
