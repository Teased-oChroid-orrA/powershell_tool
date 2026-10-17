//! Quality-guarded vertex smoothing of a validated tetrahedral topology, and its alternation with reconnection.
//!
//! Reconnection ([`super::tet_flip`]) cannot remove a sliver whose four vertices are nearly coplanar and whose neighbours
//! offer no better triangulation; moving a vertex can. Each interior vertex (one on no boundary face and not caller-protected)
//! is tried at the average of its edge neighbours, with step lengths 1, 1/2 and 1/4 of the way. A move is kept only when
//! every incident tetrahedron keeps the exact orientation it had and the worst radius ratio among the incident tetrahedra
//! improves by `min_gain`, so the pass terminates and no cell is ever inverted. Connectivity, boundary faces and boundary
//! vertices never change; the total volume is re-checked on exit.

use super::{
    quality::element_quality,
    tet_flip::{improve, FlipOptions, FlipReport},
    tet_topology::{orientation, Point, TetTopology},
    MeshError,
};
use crate::element::ElementKind;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct SmoothOptions {
    pub max_passes: usize,
    /// Smallest worst-quality gain a move must achieve.
    pub min_gain: f64,
    /// Vertices that must not move (for example sampled-surface or recovered-PLC vertices).
    pub protect: BTreeSet<usize>,
}

impl Default for SmoothOptions {
    fn default() -> Self {
        Self {
            max_passes: 20,
            min_gain: 1e-6,
            protect: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmoothReport {
    pub passes: usize,
    pub movable_vertices: usize,
    pub moves: usize,
    pub min_radius_ratio_before: f64,
    pub min_radius_ratio_after: f64,
}

fn quality(points: &[Point], t: [usize; 4]) -> f64 {
    let q = element_quality(ElementKind::Tet4, &t.map(|v| points[v])).radius_ratio;
    if q.is_finite() {
        q.max(0.)
    } else {
        0.
    }
}

fn signs(
    points: &[Point],
    cells: &[[usize; 4]],
    ids: &[usize],
) -> Result<Vec<std::cmp::Ordering>, MeshError> {
    ids.iter()
        .map(|&c| {
            let [a, b, cc, d] = cells[c].map(|v| points[v]);
            orientation(a, b, cc, d)
        })
        .collect()
}

fn worst(points: &[Point], cells: &[[usize; 4]], ids: &[usize]) -> f64 {
    ids.iter()
        .map(|&c| quality(points, cells[c]))
        .fold(f64::INFINITY, f64::min)
}

/// Move interior vertices of `top` to improve the worst incident radius ratio. See the module documentation.
pub fn smooth_vertices(
    top: &TetTopology,
    options: &SmoothOptions,
) -> Result<(TetTopology, SmoothReport), MeshError> {
    let cells = top.cells();
    let mut points: Vec<Point> = top.points().to_vec();
    let mut fixed: BTreeSet<usize> = options.protect.clone();
    fixed.extend(top.boundary().iter().flatten().copied());
    let mut incident: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut neighbours: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (i, c) in cells.iter().enumerate() {
        for &v in c {
            incident.entry(v).or_default().push(i);
            neighbours
                .entry(v)
                .or_default()
                .extend(c.iter().copied().filter(|&w| w != v));
        }
    }
    let movable: Vec<usize> = incident
        .keys()
        .copied()
        .filter(|v| !fixed.contains(v))
        .collect();
    let all: Vec<usize> = (0..cells.len()).collect();
    let mut report = SmoothReport {
        passes: 0,
        movable_vertices: movable.len(),
        moves: 0,
        min_radius_ratio_before: worst(&points, cells, &all),
        min_radius_ratio_after: 0.,
    };
    let volume_before = volume(&points, cells);
    for _ in 0..options.max_passes {
        let mut changed = 0;
        for &v in &movable {
            let ids = &incident[&v];
            let ring = &neighbours[&v];
            let before = worst(&points, cells, ids);
            let sign_before = signs(&points, cells, ids)?;
            let old = points[v];
            let mean: Point = std::array::from_fn(|k| {
                ring.iter().map(|&w| points[w][k]).sum::<f64>() / ring.len() as f64
            });
            let mut best: Option<(f64, Point)> = None;
            for step in [1.0, 0.5, 0.25] {
                let cand: Point = std::array::from_fn(|k| old[k] + step * (mean[k] - old[k]));
                points[v] = cand;
                let gain_ok = |q: f64, b: Option<(f64, Point)>| {
                    q > before + options.min_gain && b.is_none_or(|(bq, _)| q > bq)
                };
                let q = worst(&points, cells, ids);
                if gain_ok(q, best)
                    && signs(&points, cells, ids)
                        .map(|s| s == sign_before && !s.contains(&std::cmp::Ordering::Equal))
                        .unwrap_or(false)
                {
                    best = Some((q, cand));
                }
            }
            points[v] = best.map_or(old, |(_, p)| p);
            if best.is_some() {
                changed += 1;
            }
        }
        report.passes += 1;
        report.moves += changed;
        if changed == 0 {
            break;
        }
    }
    let result = TetTopology::new(points, cells.to_vec(), cells.len().max(1))?;
    if result
        .boundary()
        .iter()
        .map(|f| sorted3(*f))
        .collect::<BTreeSet<_>>()
        != top
            .boundary()
            .iter()
            .map(|f| sorted3(*f))
            .collect::<BTreeSet<_>>()
    {
        return Err(MeshError::NonConforming(
            "smoothing changed the boundary".into(),
        ));
    }
    let volume_after = volume(result.points(), result.cells());
    if (volume_after - volume_before).abs() > 1e-9 * volume_before.abs().max(f64::MIN_POSITIVE) {
        return Err(MeshError::NonConforming(
            "smoothing changed the total volume".into(),
        ));
    }
    report.min_radius_ratio_after = worst(result.points(), result.cells(), &all);
    Ok((result, report))
}

fn sorted3(mut f: [usize; 3]) -> [usize; 3] {
    f.sort_unstable();
    f
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

#[derive(Debug, Clone, PartialEq)]
pub struct OptimizeReport {
    pub rounds: usize,
    pub flips: FlipReport,
    pub smoothing_moves: usize,
    pub min_radius_ratio_before: f64,
    pub min_radius_ratio_after: f64,
}

/// Alternate reconnection and vertex smoothing until neither changes anything (or `max_rounds`). Each stage only accepts
/// strict improvements of the worst local radius ratio, so the global minimum never decreases.
pub fn optimize(
    top: &TetTopology,
    protected_faces: &BTreeSet<[usize; 3]>,
    flip: FlipOptions,
    smoothing: &SmoothOptions,
    max_rounds: usize,
) -> Result<(TetTopology, OptimizeReport), MeshError> {
    let mut current = top.clone();
    let mut flips: Option<FlipReport> = None;
    let mut moves = 0;
    let mut rounds = 0;
    let before = worst(
        top.points(),
        top.cells(),
        &(0..top.cells().len()).collect::<Vec<_>>(),
    );
    for _ in 0..max_rounds {
        let (a, fr) = improve(&current, protected_faces, flip)?;
        let n_flips = fr.face_flips + fr.edge_removals.values().sum::<usize>();
        flips = Some(match flips {
            None => fr,
            Some(mut acc) => {
                acc.passes += fr.passes;
                acc.face_flips += fr.face_flips;
                for (k, n) in fr.edge_removals {
                    *acc.edge_removals.entry(k).or_default() += n;
                }
                acc.min_radius_ratio_after = fr.min_radius_ratio_after;
                acc.cells_below_target_after = fr.cells_below_target_after;
                acc
            }
        });
        let (b, sr) = smooth_vertices(&a, smoothing)?;
        current = b;
        moves += sr.moves;
        rounds += 1;
        if n_flips == 0 && sr.moves == 0 {
            break;
        }
    }
    let after = worst(
        current.points(),
        current.cells(),
        &(0..current.cells().len()).collect::<Vec<_>>(),
    );
    Ok((
        current,
        OptimizeReport {
            rounds,
            flips: flips.expect("max_rounds >= 1"),
            smoothing_moves: moves,
            min_radius_ratio_before: before,
            min_radius_ratio_after: after,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshing::tet_delaunay::{tetrahedralize, Limits};

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

    fn boundary_set(t: &TetTopology) -> BTreeSet<[usize; 3]> {
        t.boundary().iter().map(|f| sorted3(*f)).collect()
    }

    #[test]
    fn smoothing_improves_the_worst_cell_and_keeps_boundary_connectivity_and_volume() {
        let mut improved = 0;
        for seed in 1..=6 {
            let top = tetrahedralize(&cloud(60, seed), Limits::default()).unwrap();
            let (out, r) = smooth_vertices(&top, &SmoothOptions::default()).unwrap();
            assert!(
                r.min_radius_ratio_after >= r.min_radius_ratio_before,
                "seed {seed}"
            );
            assert_eq!(out.cells(), top.cells());
            assert_eq!(boundary_set(&out), boundary_set(&top));
            for v in top.boundary().iter().flatten() {
                assert_eq!(out.points()[*v], top.points()[*v], "boundary vertex moved");
            }
            assert!((volume(out.points(), out.cells()) - 1.).abs() < 1e-12);
            if r.min_radius_ratio_after > r.min_radius_ratio_before {
                improved += 1;
            }
        }
        assert!(
            improved > 0,
            "random clouds have interior slivers that moving a vertex fixes"
        );
    }

    #[test]
    fn protected_vertices_do_not_move() {
        let top = tetrahedralize(&cloud(60, 2), Limits::default()).unwrap();
        let protect: BTreeSet<usize> = (8..top.points().len()).collect();
        let (out, r) = smooth_vertices(
            &top,
            &SmoothOptions {
                protect,
                ..SmoothOptions::default()
            },
        )
        .unwrap();
        assert_eq!(r.moves, 0);
        assert_eq!(out.points(), top.points());
    }

    #[test]
    fn alternating_with_reconnection_is_at_least_as_good_as_reconnection_alone_and_deterministic() {
        for seed in 1..=6 {
            let top = tetrahedralize(&cloud(60, seed), Limits::default()).unwrap();
            let none = BTreeSet::new();
            let (_, flip_only) = improve(&top, &none, FlipOptions::default()).unwrap();
            let (a, ra) = optimize(
                &top,
                &none,
                FlipOptions::default(),
                &SmoothOptions::default(),
                10,
            )
            .unwrap();
            let (b, _) = optimize(
                &top,
                &none,
                FlipOptions::default(),
                &SmoothOptions::default(),
                10,
            )
            .unwrap();
            assert_eq!(a, b, "deterministic");
            assert!(ra.min_radius_ratio_after >= ra.min_radius_ratio_before);
            assert!(
                ra.min_radius_ratio_after + 1e-12 >= flip_only.min_radius_ratio_after,
                "seed {seed}"
            );
            assert_eq!(boundary_set(&a), boundary_set(&top));
            assert!((volume(a.points(), a.cells()) - 1.).abs() < 1e-12);
            eprintln!(
                "seed {seed}: min rr {:.4} -> flips {:.4} -> flips+smooth {:.4}; moves {}",
                ra.min_radius_ratio_before,
                flip_only.min_radius_ratio_after,
                ra.min_radius_ratio_after,
                ra.smoothing_moves
            );
        }
    }
}
