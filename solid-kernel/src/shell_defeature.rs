//! Defeaturing of sliver faces on a closed, outward-oriented triangulated shell by cluster welding.
//!
//! A sliver face is a strip a hair wide (a chamfer or fillet of a hundredth of a millimetre along a fifty millimetre edge): the
//! triangles spanning it are needles whatever the triangulation, and its two rails are joined by short rungs. The edge
//! collapse of `shell_weld` cannot remove it, because a rung's endpoints are also joined by the long rails, which fails the
//! link condition. Welding the whole cluster of vertices at once does:
//!
//! 1. A *cluster* is a connected set of vertices joined by edges of a needle triangle (smallest corner below `bad_angle_deg`)
//!    no longer than `max_edge_relative` x the bounding-box diagonal, whose pairwise distance never exceeds that length.
//! 2. The cluster collapses onto its medoid (the member whose farthest other member is nearest), so geometry moves by at most
//!    that length. Triangles with two equal vertices disappear; two triangles on the same vertices with opposite orientation
//!    (the two faces of the former strip, now coincident) cancel.
//! 3. A cluster is kept only if the shell is still a closed oriented 2-manifold with no zero-area facet, the signed volume has
//!    moved by no more than four times the weld length times the surface area, and the smallest corner angle of the triangles
//!    that were touched is not worse than `min(before, acceptable_angle_deg)`. Otherwise it is rejected and counted.
//!
//! Self-intersections are not tested here: the bounded displacement makes them unlikely, and the checked-PLC admission that
//! precedes constrained recovery detects them.

use crate::shell_patch::{area, check_closed, min_corner_angle_deg, norm, signed_volume, sub};
use crate::V3;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Longest cluster extent and edge considered, as a fraction of the bounding-box diagonal.
    pub max_edge_relative: f64,
    pub bad_angle_deg: f64,
    pub acceptable_angle_deg: f64,
    pub max_rounds: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_edge_relative: 1e-3,
            bad_angle_deg: 5.,
            acceptable_angle_deg: 10.,
            max_rounds: 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rounds: usize,
    pub clusters_found: usize,
    pub clusters_applied: usize,
    pub clusters_rejected: usize,
    /// Rejections by reason: multiplicity/orientation of coincident sheets, shell no longer closed, zero-area facet, volume
    /// moved too far, or the touched triangles' smallest angle made worse.
    pub rejected_sheets: usize,
    pub rejected_not_closed: usize,
    pub rejected_zero_area: usize,
    pub rejected_volume: usize,
    pub rejected_angle: usize,
    pub degenerate_removed: usize,
    pub sheets_cancelled: usize,
    /// Zero-area caps left by welds and removed by edge flips.
    pub caps_flipped: usize,
    pub longest_weld: f64,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub facets_before: usize,
    pub facets_after: usize,
    pub min_angle_before_deg: f64,
    pub min_angle_after_deg: f64,
}

#[derive(Debug, Clone)]
pub struct Defeatured {
    pub vertices: Vec<V3>,
    pub facets: Vec<[usize; 3]>,
    pub report: Report,
}

fn dist(a: V3, b: V3) -> f64 {
    norm(sub(a, b))
}

/// Parity of the permutation taking the sorted triple to `f` (`true` = odd).
fn parity(f: [usize; 3]) -> bool {
    let mut inversions = 0;
    for i in 0..3 {
        for j in i + 1..3 {
            if f[i] > f[j] {
                inversions += 1;
            }
        }
    }
    inversions % 2 == 1
}

/// Move every vertex `v` with `onto[v]` to `onto[v]` in `tris`, drop degenerate triangles and cancel opposite coincident pairs. `None` when a
/// triple occurs with a multiplicity or orientation that would make the shell non-manifold.
fn weld(
    tris: &[[usize; 3]],
    onto: &BTreeMap<usize, usize>,
) -> Option<(Vec<[usize; 3]>, usize, usize)> {
    let mut degenerate = 0;
    let mut by_key: BTreeMap<[usize; 3], Vec<[usize; 3]>> = BTreeMap::new();
    for f in tris {
        let g = f.map(|v| onto.get(&v).copied().unwrap_or(v));
        if g[0] == g[1] || g[1] == g[2] || g[0] == g[2] {
            degenerate += 1;
            continue;
        }
        let mut k = g;
        k.sort_unstable();
        by_key.entry(k).or_default().push(g);
    }
    let mut out = Vec::with_capacity(tris.len());
    let mut cancelled = 0;
    for (_, list) in by_key {
        match list.len() {
            1 => out.push(list[0]),
            2 if parity(list[0]) != parity(list[1]) => cancelled += 1,
            _ => return None,
        }
    }
    Some((out, degenerate, cancelled))
}

/// Remove the zero-area triangles a weld can leave (three collinear points: a *cap* whose middle vertex lies on the long edge)
/// by flipping the long edge with the triangle across it. Returns the repaired triangles and the number of flips, or `None`
/// when a cap has no neighbour to flip with or the flip would duplicate an edge.
fn repair_caps(
    vertices: &[V3],
    mut tris: Vec<[usize; 3]>,
    threshold: f64,
) -> Option<(Vec<[usize; 3]>, usize)> {
    // Each pass flips exactly one cap, so the pass index is the number of flips made when the last cap is gone.
    for flips in 0..(2 * tris.len() + 8) {
        let Some(t) = tris.iter().position(|&f| area(vertices, f) <= threshold) else {
            return Some((tris, flips));
        };
        let f = tris[t];
        // The longest edge a -> c with the apex b (the point between them).
        let k = (0..3).max_by(|&i, &j| {
            dist(vertices[f[i]], vertices[f[(i + 1) % 3]])
                .total_cmp(&dist(vertices[f[j]], vertices[f[(j + 1) % 3]]))
        })?;
        let (a, c, b) = (f[k], f[(k + 1) % 3], f[(k + 2) % 3]);
        let n = tris
            .iter()
            .position(|g| (0..3).any(|i| g[i] == c && g[(i + 1) % 3] == a))?;
        let g = tris[n];
        let x = *g.iter().find(|&&v| v != a && v != c)?;
        // The quad b, a, x, c (counter-clockwise) swaps its diagonal a-c for b-x.
        if tris.iter().any(|h| h.contains(&b) && h.contains(&x)) {
            return None;
        }
        tris[t] = [b, a, x];
        tris[n] = [b, x, c];
    }
    None
}

/// Weld clusters of needle-edge vertices. See the module documentation.
pub fn defeature_slivers(
    vertices: &[V3],
    facets: &[[usize; 3]],
    options: Options,
) -> Result<Defeatured, String> {
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
    let max_len = options.max_edge_relative * diag;
    let volume_before = signed_volume(vertices, facets);
    let surface: f64 = facets.iter().map(|&f| area(vertices, f)).sum();
    let mut tris = facets.to_vec();
    let before_angle = min_corner_angle_deg(vertices, facets);
    let mut report = Report {
        rounds: 0,
        clusters_found: 0,
        clusters_applied: 0,
        clusters_rejected: 0,
        rejected_sheets: 0,
        rejected_not_closed: 0,
        rejected_zero_area: 0,
        rejected_volume: 0,
        rejected_angle: 0,
        degenerate_removed: 0,
        sheets_cancelled: 0,
        caps_flipped: 0,
        longest_weld: 0.,
        vertices_before: vertices.len(),
        vertices_after: vertices.len(),
        facets_before: facets.len(),
        facets_after: facets.len(),
        min_angle_before_deg: before_angle,
        min_angle_after_deg: before_angle,
    };
    let mut rejected_sets: BTreeSet<Vec<usize>> = BTreeSet::new();
    for _ in 0..options.max_rounds {
        // Union-find over the needle edges, limited by the cluster extent.
        let mut members: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let mut root: BTreeMap<usize, usize> = BTreeMap::new();
        let mut edges: Vec<(f64, [usize; 2])> = Vec::new();
        for f in &tris {
            if min_corner_angle_deg(vertices, &[*f]) >= options.bad_angle_deg {
                continue;
            }
            for k in 0..3 {
                let (a, b) = (f[k], f[(k + 1) % 3]);
                let l = dist(vertices[a], vertices[b]);
                if l <= max_len {
                    edges.push((l, [a.min(b), a.max(b)]));
                }
            }
        }
        edges.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        edges.dedup_by(|x, y| x.1 == y.1);
        for (_, [a, b]) in edges {
            let ra = *root.entry(a).or_insert(a);
            let rb = *root.entry(b).or_insert(b);
            members.entry(ra).or_insert_with(|| BTreeSet::from([ra]));
            members.entry(rb).or_insert_with(|| BTreeSet::from([rb]));
            if ra == rb {
                continue;
            }
            let (ma, mb) = (members[&ra].clone(), members[&rb].clone());
            let fits = ma.iter().all(|&x| {
                mb.iter()
                    .all(|&y| dist(vertices[x], vertices[y]) <= max_len)
            });
            if fits {
                let merged: BTreeSet<usize> = ma.union(&mb).copied().collect();
                for &m in &merged {
                    root.insert(m, ra);
                }
                members.remove(&rb);
                members.insert(ra, merged);
            }
        }
        let mut clusters: Vec<BTreeSet<usize>> =
            members.into_values().filter(|m| m.len() >= 2).collect();
        clusters.sort_by_key(|m| *m.iter().next().expect("non-empty"));
        report.clusters_found = report.clusters_found.max(clusters.len());
        // Clusters that share a triangle belong to one sliver region (the stations of a strip): they are welded together or not
        // at all, because welding a single station leaves needles to its neighbours.
        let mut group: Vec<usize> = (0..clusters.len()).collect();
        fn find(g: &mut [usize], mut x: usize) -> usize {
            while g[x] != x {
                g[x] = g[g[x]];
                x = g[x];
            }
            x
        }
        let owner: BTreeMap<usize, usize> = clusters
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.iter().map(move |&v| (v, i)))
            .collect();
        for f in &tris {
            let hit: Vec<usize> = f.iter().filter_map(|v| owner.get(v).copied()).collect();
            for w in hit.windows(2) {
                let (ra, rb) = (find(&mut group, w[0]), find(&mut group, w[1]));
                group[ra] = rb;
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..clusters.len() {
            let r = find(&mut group, i);
            groups.entry(r).or_default().push(i);
        }
        let mut applied_this_round = 0;
        for (_, ids) in groups {
            let members: BTreeSet<usize> = ids
                .iter()
                .flat_map(|&i| clusters[i].iter().copied())
                .collect();
            let key: Vec<usize> = members.iter().copied().collect();
            if rejected_sets.contains(&key)
                || members.iter().any(|v| !tris.iter().any(|f| f.contains(v)))
            {
                continue;
            }
            // Each cluster collapses onto its medoid (the member whose farthest other member is nearest).
            let mut onto: BTreeMap<usize, usize> = BTreeMap::new();
            let mut reach = 0f64;
            for &i in &ids {
                let c = &clusters[i];
                let rep = *c
                    .iter()
                    .min_by(|&&a, &&b| {
                        let far = |x: usize| {
                            c.iter()
                                .map(|&y| dist(vertices[x], vertices[y]))
                                .fold(0., f64::max)
                        };
                        far(a).total_cmp(&far(b)).then(a.cmp(&b))
                    })
                    .expect("non-empty");
                for &m in c {
                    onto.insert(m, rep);
                    reach = reach.max(dist(vertices[rep], vertices[m]));
                }
            }
            let reps: BTreeSet<usize> = onto.values().copied().collect();
            let mut caps = 0usize;
            let attempt = match weld(&tris, &onto) {
                None => {
                    report.rejected_sheets += 1;
                    None
                }
                Some((new, degenerate, cancelled)) => {
                    let repaired = repair_caps(vertices, new, 1e-14 * diag * diag);
                    let Some((new, flips)) = repaired else {
                        report.rejected_zero_area += 1;
                        rejected_sets.insert(key);
                        report.clusters_rejected += ids.len();
                        continue;
                    };
                    caps += flips;
                    if check_closed(&new).is_err() {
                        report.rejected_not_closed += 1;
                        None
                    } else if new.iter().any(|&f| area(vertices, f) <= 0.) {
                        report.rejected_zero_area += 1;
                        None
                    } else if (signed_volume(vertices, &new) - volume_before).abs()
                        > 1e-9 * volume_before.abs() + 4. * report.longest_weld.max(reach) * surface
                    {
                        report.rejected_volume += 1;
                        None
                    } else {
                        let touched: Vec<[usize; 3]> = tris
                            .iter()
                            .copied()
                            .filter(|f| f.iter().any(|v| members.contains(v)))
                            .collect();
                        let newly: Vec<[usize; 3]> = new
                            .iter()
                            .copied()
                            .filter(|f| f.iter().any(|v| reps.contains(v)))
                            .collect();
                        let floor = min_corner_angle_deg(vertices, &touched)
                            .min(options.acceptable_angle_deg);
                        if !newly.is_empty()
                            && min_corner_angle_deg(vertices, &newly) < floor * (1. - 1e-9)
                        {
                            report.rejected_angle += 1;
                            None
                        } else {
                            Some((new, degenerate, cancelled))
                        }
                    }
                }
            };
            match attempt {
                Some((new, degenerate, cancelled)) => {
                    tris = new;
                    report.clusters_applied += ids.len();
                    report.caps_flipped += caps;
                    report.degenerate_removed += degenerate;
                    report.sheets_cancelled += cancelled;
                    report.longest_weld = report.longest_weld.max(reach);
                    applied_this_round += 1;
                }
                None => {
                    report.clusters_rejected += ids.len();
                    rejected_sets.insert(key);
                }
            }
        }
        report.rounds += 1;
        if applied_this_round == 0 {
            break;
        }
    }
    // Welded vertices are mapped onto their medoid in `tris`; compact what is still referenced.
    let used: BTreeSet<usize> = tris.iter().flatten().copied().collect();
    let mut map = BTreeMap::new();
    let mut out_v = Vec::new();
    for (i, v) in vertices.iter().enumerate() {
        if used.contains(&i) {
            map.insert(i, out_v.len());
            out_v.push(*v);
        }
    }
    let out_f: Vec<[usize; 3]> = tris.iter().map(|f| f.map(|v| map[&v])).collect();
    check_closed(&out_f).map_err(|e| format!("defeaturing produced an invalid shell: {e}"))?;
    report.vertices_after = out_v.len();
    report.facets_after = out_f.len();
    report.min_angle_after_deg = min_corner_angle_deg(&out_v, &out_f);
    Ok(Defeatured {
        vertices: out_v,
        facets: out_f,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_patch::tests::grid_box;

    /// A square prism (side 1, height `height`) with one vertical edge replaced by a chamfer face `width` wide: the polygon
    /// `(0,0) (1,0) (1,1-w) (1-w,1) (0,1)` extruded along z. Closed by construction; the chamfer wall is a sliver face and the
    /// cap triangles that reach it are needles. The caps are fans from the first vertex.
    fn chamfered_prism(width: f64, height: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
        let poly = [
            [0., 0.],
            [1., 0.],
            [1., 1. - width],
            [1. - width, 1.],
            [0., 1.],
        ];
        let n = poly.len();
        let mut v: Vec<V3> = poly.iter().map(|p| [p[0], p[1], 0.]).collect();
        v.extend(poly.iter().map(|p| [p[0], p[1], height]));
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

    /// [`chamfered_prism`] with a mid-width vertex on the chamfer (slightly bulged) and `levels` stations along the height, the
    /// way a sampled sliver face looks: needles along its whole length, joined by rungs at every station.
    fn stationed_chamfer(width: f64, height: f64, levels: usize) -> (Vec<V3>, Vec<[usize; 3]>) {
        let w = width;
        let poly = [
            [0., 0.],
            [1., 0.],
            [1., 1. - w],
            [1. - 0.5 * w + 0.01 * w, 1. - 0.5 * w + 0.01 * w],
            [1. - w, 1.],
            [0., 1.],
        ];
        let n = poly.len();
        let mut v: Vec<V3> = Vec::new();
        for k in 0..levels {
            let z = height * k as f64 / (levels - 1) as f64;
            v.extend(poly.iter().map(|p| [p[0], p[1], z]));
        }
        let mut f = Vec::new();
        let top = (levels - 1) * n;
        for k in 1..n - 1 {
            f.push([0, k + 1, k]);
            f.push([top, top + k, top + k + 1]);
        }
        for l in 0..levels - 1 {
            for k in 0..n {
                let (a, b) = (l * n + k, l * n + (k + 1) % n);
                f.push([a, b, b + n]);
                f.push([a, b + n, a + n]);
            }
        }
        (v, f)
    }

    #[test]
    fn a_sliver_strip_with_stations_along_its_length_is_welded_as_one_group() {
        let (v, f) = stationed_chamfer(1e-3, 3.0, 4);
        check_closed(&f).unwrap();
        let before = min_corner_angle_deg(&v, &f);
        assert!(before < 0.5, "{before}");
        let d = defeature_slivers(&v, &f, Options::default()).unwrap();
        eprintln!("{:?}", d.report);
        assert!(
            d.report.clusters_applied >= 2 && d.report.clusters_rejected == 0,
            "{:?}",
            d.report
        );
        assert!(
            d.report.min_angle_after_deg > 10. * before,
            "{:?}",
            d.report
        );
        check_closed(&d.facets).unwrap();
        assert!(
            (signed_volume(&d.vertices, &d.facets) - signed_volume(&v, &f)).abs() < 4. * 1e-3 * 14.
        );
    }

    #[test]
    fn a_chamfer_sliver_face_is_welded_away_and_the_shell_stays_valid() {
        let (v, f) = chamfered_prism(1e-3, 1.0);
        assert!(check_closed(&f).is_ok(), "the test shell is closed");
        let before = min_corner_angle_deg(&v, &f);
        assert!(before < 1.0, "{before}");
        let d = defeature_slivers(&v, &f, Options::default()).unwrap();
        assert!(d.report.clusters_applied >= 1, "{:?}", d.report);
        assert!(
            d.report.min_angle_after_deg > before * 10.,
            "{:?}",
            d.report
        );
        assert!(d.report.facets_after < d.report.facets_before);
        assert!(
            (signed_volume(&d.vertices, &d.facets) - signed_volume(&v, &f)).abs()
                < 4. * 1e-3 * 2.0 * 6. * 4.
        );
        check_closed(&d.facets).unwrap();
    }

    #[test]
    fn healthy_shells_are_untouched_and_open_shells_refused() {
        let (v, f) = grid_box(2, 2, 2);
        let d = defeature_slivers(&v, &f, Options::default()).unwrap();
        assert_eq!(d.report.clusters_applied, 0);
        assert_eq!(d.facets, f);
        let (v, mut f) = grid_box(1, 1, 1);
        f.pop();
        assert!(defeature_slivers(&v, &f, Options::default()).is_err());
    }

    #[test]
    fn the_result_is_deterministic_and_never_has_a_worse_minimum_angle() {
        let (v, f) = chamfered_prism(2e-3, 2.0);
        let (a, b) = (
            defeature_slivers(&v, &f, Options::default()).unwrap(),
            defeature_slivers(&v, &f, Options::default()).unwrap(),
        );
        assert_eq!(a.facets, b.facets);
        assert_eq!(a.vertices, b.vertices);
        assert!(a.report.min_angle_after_deg >= a.report.min_angle_before_deg);
    }
}
