//! Edge collapse (coarsening) of a validated tetrahedral topology.
//!
//! Collapsing the edge `u-v` removes the vertex `u`: every tetrahedron holding both vertices is deleted and `u` is replaced by
//! `v` in the others. Only interior, unprotected vertices are removed (so the boundary surface and every protected vertex
//! survive exactly), but they may collapse onto any vertex, boundary or not. A collapse is accepted only when
//! - every changed tetrahedron keeps the exact orientation it had (so the star of `u` is re-filled by a valid star of `v`, no
//!   cell is inverted and the volume is unchanged),
//! - no changed tetrahedron coincides with an existing one,
//! - every new edge at `v` is no longer than `max_length` and every new cell has quality at least `min_quality`
//!   (caller-supplied, only compared).
//!
//! Collapses of one pass touch disjoint neighbourhoods, so they are independent; passes repeat until nothing changes.
//! The result is compacted: the returned map gives, for every old vertex, its new index (`None` = removed).

use super::{
    tet_topology::{orientation, Point, TetTopology},
    MeshError,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct CollapseOptions {
    /// Edges shorter than this are candidates.
    pub min_length: f64,
    /// A collapse may not create an edge longer than this.
    pub max_length: f64,
    /// Smallest quality a new tetrahedron may have.
    pub min_quality: f64,
    pub max_passes: usize,
    /// Vertices that must not be removed (indices of the input topology).
    pub protect: BTreeSet<usize>,
}

impl Default for CollapseOptions {
    fn default() -> Self {
        Self {
            min_length: std::f64::consts::FRAC_1_SQRT_2,
            max_length: std::f64::consts::SQRT_2,
            min_quality: 0.05,
            max_passes: 20,
            protect: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollapseReport {
    pub passes: usize,
    pub collapses: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub cells_before: usize,
    pub cells_after: usize,
}

/// Cell quality (larger is better) and edge length, both functions of the current points.
pub type QualityFn<'a> = &'a dyn Fn(&[Point], [usize; 4]) -> f64;
pub type LengthFn<'a> = &'a dyn Fn(&[Point], usize, usize) -> f64;

/// One feasible collapse of `u` onto `v`: the cells it deletes, the cells that replace the rest of `u`'s star, and the worst
/// quality among those.
struct Candidate {
    worst: f64,
    u: usize,
    v: usize,
    gone: Vec<usize>,
    new_cells: Vec<[usize; 4]>,
}

fn key4(mut c: [usize; 4]) -> [usize; 4] {
    c.sort_unstable();
    c
}

fn sign(points: &[Point], c: [usize; 4]) -> Result<Ordering, MeshError> {
    let [a, b, cc, d] = c.map(|v| points[v]);
    orientation(a, b, cc, d)
}

fn volume(points: &[Point], cells: &[[usize; 4]]) -> f64 {
    cells
        .iter()
        .map(|c| {
            let [a, b, cc, d] = c.map(|i| points[i]);
            let u: Point = std::array::from_fn(|i| b[i] - a[i]);
            let v: Point = std::array::from_fn(|i| cc[i] - a[i]);
            let w: Point = std::array::from_fn(|i| d[i] - a[i]);
            (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
                + u[2] * (v[0] * w[1] - v[1] * w[0]))
                .abs()
                / 6.
        })
        .sum()
}

/// Collapse short edges of `top`. Returns the coarser topology, a report and the old-to-new vertex map.
pub fn coarsen(
    top: &TetTopology,
    options: &CollapseOptions,
    quality: QualityFn<'_>,
    length: LengthFn<'_>,
) -> Result<(TetTopology, CollapseReport, Vec<Option<usize>>), MeshError> {
    let points: Vec<Point> = top.points().to_vec();
    let mut cells: Vec<[usize; 4]> = top.cells().to_vec();
    let mut fixed: BTreeSet<usize> = options.protect.clone();
    fixed.extend(top.boundary().iter().flatten().copied());
    let volume_before = volume(&points, &cells);
    let boundary_before: BTreeSet<[usize; 3]> = top
        .boundary()
        .iter()
        .map(|f| {
            let mut f = *f;
            f.sort_unstable();
            f
        })
        .collect();
    let mut removed = vec![false; points.len()];
    let mut report = CollapseReport {
        passes: 0,
        collapses: 0,
        vertices_before: points.len(),
        vertices_after: points.len(),
        cells_before: cells.len(),
        cells_after: cells.len(),
    };
    for _ in 0..options.max_passes {
        let mut incident: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut edges: BTreeSet<[usize; 2]> = BTreeSet::new();
        for (i, c) in cells.iter().enumerate() {
            for &v in c {
                incident.entry(v).or_default().push(i);
            }
            for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
                edges.insert([c[a].min(c[b]), c[a].max(c[b])]);
            }
        }
        let mut short: Vec<(f64, [usize; 2])> = edges
            .into_iter()
            .map(|e| (length(&points, e[0], e[1]), e))
            .filter(|(l, _)| *l < options.min_length)
            .collect();
        short.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut dead = vec![false; cells.len()];
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        let mut additions: Vec<[usize; 4]> = Vec::new();
        let mut changed = 0;
        for (_, [a, b]) in short {
            let mut best: Option<Candidate> = None;
            for (u, v) in [(a, b), (b, a)] {
                if fixed.contains(&u) || removed[u] || touched.contains(&u) || touched.contains(&v)
                {
                    continue;
                }
                let star = &incident[&u];
                if star.iter().any(|&c| dead[c]) {
                    continue;
                }
                let ring: BTreeSet<usize> = star.iter().flat_map(|&c| cells[c]).collect();
                if ring.iter().any(|w| touched.contains(w)) {
                    continue;
                }
                let mut gone = Vec::new();
                let mut new_cells = Vec::new();
                let mut ok = true;
                let mut worst = f64::INFINITY;
                for &c in star {
                    let old = cells[c];
                    if old.contains(&v) {
                        gone.push(c);
                        continue;
                    }
                    let new = old.map(|x| if x == u { v } else { x });
                    match (sign(&points, old), sign(&points, new)) {
                        (Ok(s0), Ok(s1)) if s0 == s1 && s1 != Ordering::Equal => {}
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                    let q = quality(&points, new);
                    if q < options.min_quality {
                        ok = false;
                        break;
                    }
                    worst = worst.min(q);
                    new_cells.push(new);
                }
                if !ok || gone.is_empty() {
                    continue;
                }
                // New edges at v.
                let neighbours: BTreeSet<usize> = new_cells
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|&w| w != v)
                    .collect();
                if neighbours
                    .iter()
                    .any(|&w| length(&points, v, w) > options.max_length)
                {
                    continue;
                }
                // No coincident cells: among the new ones and against the cells already around v.
                let mut seen: BTreeSet<[usize; 4]> = new_cells.iter().map(|c| key4(*c)).collect();
                if seen.len() != new_cells.len() {
                    continue;
                }
                let around_v = incident.get(&v).map(Vec::as_slice).unwrap_or(&[]);
                if around_v
                    .iter()
                    .any(|&c| !dead[c] && !star.contains(&c) && !seen.insert(key4(cells[c])))
                {
                    continue;
                }
                if best.as_ref().is_none_or(|b| worst > b.worst) {
                    best = Some(Candidate {
                        worst,
                        u,
                        v,
                        gone,
                        new_cells,
                    });
                }
            }
            let Some(Candidate {
                u,
                v,
                gone,
                new_cells,
                ..
            }) = best
            else {
                continue;
            };
            for &c in &incident[&u] {
                dead[c] = true;
            }
            for &c in &gone {
                dead[c] = true;
            }
            touched.extend(incident[&u].iter().flat_map(|&c| cells[c]));
            touched.insert(v);
            additions.extend(new_cells);
            removed[u] = true;
            changed += 1;
        }
        report.passes += 1;
        if changed == 0 {
            break;
        }
        report.collapses += changed;
        cells = cells
            .iter()
            .zip(&dead)
            .filter(|(_, d)| !**d)
            .map(|(c, _)| *c)
            .chain(additions)
            .collect();
    }
    // Compact.
    let mut map: Vec<Option<usize>> = vec![None; points.len()];
    let mut kept: Vec<Point> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        if !removed[i] {
            map[i] = Some(kept.len());
            kept.push(*p);
        }
    }
    let remapped: Vec<[usize; 4]> = cells
        .iter()
        .map(|c| c.map(|v| map[v].expect("cells reference only kept vertices")))
        .collect();
    let result = TetTopology::new(kept, remapped, top.cells().len().max(1))?;
    let boundary_after: BTreeSet<[usize; 3]> = result
        .boundary()
        .iter()
        .map(|f| {
            let mut f = [f[0], f[1], f[2]];
            f.sort_unstable();
            f
        })
        .collect();
    let expected: BTreeSet<[usize; 3]> = boundary_before
        .iter()
        .map(|f| {
            let mut g = f.map(|v| map[v].expect("boundary vertices are never removed"));
            g.sort_unstable();
            g
        })
        .collect();
    if boundary_after != expected {
        return Err(MeshError::NonConforming(
            "collapse changed the boundary".into(),
        ));
    }
    let volume_after = volume(result.points(), result.cells());
    if (volume_after - volume_before).abs() > 1e-9 * volume_before.abs().max(f64::MIN_POSITIVE) {
        return Err(MeshError::NonConforming(
            "collapse changed the total volume".into(),
        ));
    }
    report.vertices_after = result.points().len();
    report.cells_after = result.cells().len();
    Ok((result, report, map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::meshing::{
        quality::element_quality,
        tet_delaunay::{tetrahedralize, Limits},
    };

    fn cloud(n: usize, seed: u64) -> Vec<Point> {
        let mut s = seed;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / (1u64 << 31) as f64
        };
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        for _ in 0..n {
            p.push([
                0.02 + 0.96 * next(),
                0.02 + 0.96 * next(),
                0.02 + 0.96 * next(),
            ]);
        }
        p
    }

    fn q(p: &[Point], t: [usize; 4]) -> f64 {
        let r = element_quality(ElementKind::Tet4, &t.map(|v| p[v])).radius_ratio;
        if r.is_finite() {
            r.max(0.)
        } else {
            0.
        }
    }

    fn len(p: &[Point], a: usize, b: usize) -> f64 {
        (0..3)
            .map(|k| (p[a][k] - p[b][k]).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    #[test]
    fn collapsing_short_edges_removes_interior_vertices_and_keeps_boundary_and_volume() {
        let mut total = 0;
        for seed in 1..=6 {
            let top = tetrahedralize(&cloud(150, seed), Limits::default()).unwrap();
            let opt = CollapseOptions {
                min_length: 0.25,
                max_length: 0.6,
                min_quality: 0.05,
                ..CollapseOptions::default()
            };
            let (out, r, map) = coarsen(&top, &opt, &q, &len).unwrap();
            assert_eq!(
                r.vertices_before - r.vertices_after,
                r.collapses,
                "seed {seed}"
            );
            assert!(r.cells_after <= r.cells_before);
            assert_eq!(out.points().len(), r.vertices_after);
            for v in top.boundary().iter().flatten() {
                assert_eq!(
                    out.points()[map[*v].unwrap()],
                    top.points()[*v],
                    "boundary vertex moved"
                );
            }
            assert!((volume(out.points(), out.cells()) - 1.).abs() < 1e-12);
            let worst = out
                .cells()
                .iter()
                .map(|c| q(out.points(), *c))
                .fold(f64::INFINITY, f64::min);
            let before = top
                .cells()
                .iter()
                .map(|c| q(top.points(), *c))
                .fold(f64::INFINITY, f64::min);
            assert!(
                worst >= before.min(opt.min_quality) - 1e-12,
                "seed {seed}: {worst} vs {before}"
            );
            total += r.collapses;
            eprintln!(
                "seed {seed}: {} -> {} vertices, {} -> {} cells",
                r.vertices_before, r.vertices_after, r.cells_before, r.cells_after
            );
        }
        assert!(total > 0, "the clouds contain short interior edges");
    }

    #[test]
    fn protected_vertices_survive_and_the_result_is_deterministic() {
        let top = tetrahedralize(&cloud(150, 2), Limits::default()).unwrap();
        let protect: BTreeSet<usize> = (8..top.points().len()).collect();
        let opt = CollapseOptions {
            min_length: 0.3,
            max_length: 0.7,
            protect,
            ..CollapseOptions::default()
        };
        let (out, r, _) = coarsen(&top, &opt, &q, &len).unwrap();
        assert_eq!(r.collapses, 0);
        assert_eq!(out.points(), top.points());
        let opt = CollapseOptions {
            min_length: 0.25,
            max_length: 0.6,
            ..CollapseOptions::default()
        };
        let a = coarsen(&top, &opt, &q, &len).unwrap();
        let b = coarsen(&top, &opt, &q, &len).unwrap();
        assert_eq!(a.0, b.0);
        assert_eq!(a.2, b.2);
    }

    #[test]
    fn no_candidate_edges_change_nothing() {
        let top = tetrahedralize(&cloud(60, 3), Limits::default()).unwrap();
        let opt = CollapseOptions {
            min_length: 1e-9,
            ..CollapseOptions::default()
        };
        let (out, r, map) = coarsen(&top, &opt, &q, &len).unwrap();
        assert_eq!(r.collapses, 0);
        assert_eq!(out.cells(), top.cells());
        assert!(map.iter().enumerate().all(|(i, m)| *m == Some(i)));
    }
}
