//! Metric-driven refinement of a tetrahedral mesh: split every edge longer than the
//! target in the metric, then reconnect by exact flips in the metric's own quality and
//! smooth the vertices in it (`RefineOptions::smooth_passes`).
//!
//! Splitting an edge at its midpoint replaces each tetrahedron around it by two, so
//! the mesh stays conforming and the boundary stays the same surface (its triangles
//! are subdivided). Collapse (`coarsen_passes`) removes interior vertices of edges shorter
//! than the band. Smoothing moves interior vertices and boundary vertices that lie in an
//! exactly planar patch (never edges, corners or curved boundaries), so the polyhedral
//! boundary is preserved; boundary points are not projected onto any curved CAD surface.
use super::{
    metric::{edge_length, Metric, MetricBounds},
    tet_collapse::{coarsen, CollapseOptions},
    tet_flip::{improve_by, FlipOptions, FlipReport},
    tet_smooth::{smooth_vertices_by, SmoothOptions},
    tet_topology::{Point, TetTopology},
    MeshError,
};
use crate::element::ElementKind;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct RefineOptions {
    /// Edges longer than this (in the metric) are split. The classical band is
    /// `[1/sqrt(2), sqrt(2)]`.
    pub max_length: f64,
    pub max_passes: usize,
    pub max_cells: usize,
    /// Reconnection after each splitting pass; `None` splits only.
    pub flips: Option<FlipOptions>,
    /// Passes of metric-quality vertex smoothing of interior vertices after each reconnection; `0` disables it. Boundary
    /// vertices never move, and the metric is re-evaluated at every moved vertex.
    pub smooth_passes: usize,
    /// Passes of edge collapse after the splitting loop (interior vertices only; new edges stay within `max_length`);
    /// `0` disables it. The refinement alone never coarsens.
    pub coarsen_passes: usize,
}
impl Default for RefineOptions {
    fn default() -> Self {
        Self {
            max_length: std::f64::consts::SQRT_2,
            max_passes: 40,
            max_cells: 1_000_000,
            flips: Some(FlipOptions::default()),
            smooth_passes: 8,
            coarsen_passes: 10,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct RefineReport {
    pub passes: usize,
    pub splits: usize,
    pub cells_before: usize,
    pub cells_after: usize,
    pub longest_edge_before: f64,
    pub longest_edge_after: f64,
    /// Fraction of edges whose metric length lies in `[1/sqrt(2), sqrt(2)]`.
    pub edges_in_band_after: f64,
    /// Worst metric radius ratio before and after (1 = unit regular tetrahedron).
    pub min_quality_before: f64,
    pub min_quality_after: f64,
    pub flips: Vec<FlipReport>,
    /// Whether no edge is longer than `max_length` at the end.
    pub converged: bool,
}
fn edge_key(a: usize, b: usize) -> [usize; 2] {
    [a.min(b), a.max(b)]
}
fn mesh_error(e: super::metric::MetricError) -> MeshError {
    MeshError::Failed(format!("metric field: {e}"))
}
/// Radius ratio of the element in the space where its mean metric is the identity.
fn metric_quality(points: &[Point], metrics: &[Metric], t: [usize; 4]) -> f64 {
    let mean = Metric::mean(&t.map(|v| metrics[v])).expect("four metrics");
    let p = t.map(|v| mean.transform(points[v]));
    let q = super::quality::element_quality(ElementKind::Tet4, &p).radius_ratio;
    if q.is_finite() {
        q.max(0.)
    } else {
        0.
    }
}
struct Stats {
    longest: f64,
    in_band: f64,
    min_quality: f64,
}
fn stats(points: &[Point], metrics: &[Metric], cells: &[[usize; 4]]) -> Stats {
    let mut edges = BTreeSet::new();
    for t in cells {
        for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
            edges.insert(edge_key(t[a], t[b]));
        }
    }
    let lengths: Vec<f64> = edges
        .iter()
        .map(|e| edge_length(points[e[0]], points[e[1]], &metrics[e[0]], &metrics[e[1]]))
        .collect();
    let lo = std::f64::consts::FRAC_1_SQRT_2;
    let hi = std::f64::consts::SQRT_2;
    Stats {
        longest: lengths.iter().copied().fold(0., f64::max),
        in_band: lengths.iter().filter(|l| (lo..=hi).contains(*l)).count() as f64
            / lengths.len().max(1) as f64,
        min_quality: cells
            .iter()
            .map(|t| metric_quality(points, metrics, *t))
            .fold(f64::INFINITY, f64::min),
    }
}
/// Refine `top` toward `field`. The field is evaluated at every vertex (new ones
/// included) and must stay inside `bounds`; a violation is an error naming the point.
pub fn refine(
    top: &TetTopology,
    field: &dyn Fn(Point) -> Result<Metric, super::metric::MetricError>,
    bounds: &MetricBounds,
    options: RefineOptions,
) -> Result<(TetTopology, RefineReport), MeshError> {
    let evaluate = |p: Point| -> Result<Metric, MeshError> {
        let m = field(p).map_err(mesh_error)?;
        m.validate(bounds).map_err(mesh_error)?;
        Ok(m)
    };
    let mut points: Vec<Point> = top.points().to_vec();
    let mut metrics = points
        .iter()
        .map(|p| evaluate(*p))
        .collect::<Result<Vec<_>, _>>()?;
    let mut cells: Vec<[usize; 4]> = top.cells().to_vec();
    let before = stats(&points, &metrics, &cells);
    let mut report = RefineReport {
        passes: 0,
        splits: 0,
        cells_before: cells.len(),
        cells_after: cells.len(),
        longest_edge_before: before.longest,
        longest_edge_after: before.longest,
        edges_in_band_after: before.in_band,
        min_quality_before: before.min_quality,
        min_quality_after: before.min_quality,
        flips: Vec::new(),
        converged: false,
    };
    let smooth_quality = |p: &[Point], t: [usize; 4]| -> f64 {
        let m: Result<Vec<Metric>, MeshError> = t.iter().map(|&v| evaluate(p[v])).collect();
        match m.ok().and_then(|m| Metric::mean(&m)) {
            Some(mean) => {
                let pts = t.map(|v| mean.transform(p[v]));
                let q = super::quality::element_quality(ElementKind::Tet4, &pts).radius_ratio;
                // Shape alone would let smoothing erase the grading, so the cell is also penalised for edges
                // leaving the classical band [1/sqrt(2), sqrt(2)] of its metric.
                let (mut lo, mut hi) = (f64::INFINITY, 0f64);
                for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
                    let e = std::array::from_fn(|k| p[t[b]][k] - p[t[a]][k]);
                    let l = mean.length(e);
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
                let band = (std::f64::consts::SQRT_2 / hi).min(1.)
                    * (lo * std::f64::consts::SQRT_2).min(1.);
                if q.is_finite() {
                    q.max(0.) * band
                } else {
                    0.
                }
            }
            None => 0.,
        }
    };
    let mut current = top.clone();
    for _ in 0..options.max_passes {
        // Long edges, longest first, each with the cells around it.
        let mut ring = BTreeMap::<[usize; 2], Vec<usize>>::new();
        for (i, t) in cells.iter().enumerate() {
            for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
                ring.entry(edge_key(t[a], t[b])).or_default().push(i);
            }
        }
        let mut long: Vec<(f64, [usize; 2])> = ring
            .keys()
            .map(|e| {
                (
                    edge_length(points[e[0]], points[e[1]], &metrics[e[0]], &metrics[e[1]]),
                    *e,
                )
            })
            .filter(|(l, _)| *l > options.max_length)
            .collect();
        if long.is_empty() {
            report.converged = true;
            break;
        }
        long.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut used = vec![false; cells.len()];
        let mut next: Vec<[usize; 4]> = Vec::with_capacity(cells.len() * 2);
        let mut replaced = vec![false; cells.len()];
        let mut splits = 0;
        let (mut added_points, mut added_metrics): (Vec<Point>, Vec<Metric>) =
            (Vec::new(), Vec::new());
        // Length of an edge in the end metrics, with the key as a deterministic tie-break.
        let measure = |e: [usize; 2]| {
            (
                edge_length(points[e[0]], points[e[1]], &metrics[e[0]], &metrics[e[1]]),
                e,
            )
        };
        let longest_of = |c: usize| -> (f64, [usize; 2]) {
            let t = cells[c];
            [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]
                .iter()
                .map(|&(a, b)| measure(edge_key(t[a], t[b])))
                .fold((0., [0, 0]), |best, x| {
                    if x.0 > best.0 || (x.0 == best.0 && x.1 > best.1) {
                        x
                    } else {
                        best
                    }
                })
        };
        for (_, start) in long {
            // Rivara terminal edge: follow the longest edges of the cells around the
            // candidate until it is the longest edge of every one of them, so each
            // split halves a longest edge and cell shapes stay bounded.
            let mut edge = start;
            let mut guard = 0;
            let [a, b] = loop {
                let here = measure(edge);
                let longer = ring[&edge]
                    .iter()
                    .map(|&c| longest_of(c))
                    .filter(|x| x.0 > here.0 || (x.0 == here.0 && x.1 > here.1))
                    .fold(None, |best: Option<(f64, [usize; 2])>, x| match best {
                        Some(b) if b.0 > x.0 || (b.0 == x.0 && b.1 > x.1) => Some(b),
                        _ => Some(x),
                    });
                match longer {
                    None => break edge,
                    Some(x) => edge = x.1,
                }
                guard += 1;
                if guard > cells.len() + 8 {
                    return Err(MeshError::Failed("terminal-edge walk did not end".into()));
                }
            };
            let around = &ring[&[a, b]];
            if around.iter().any(|&c| used[c]) {
                continue;
            }
            if cells.len() + next.len() + around.len() > options.max_cells {
                return Err(MeshError::LimitExceeded(
                    "refinement cell budget exceeded".into(),
                ));
            }
            let m: Point = std::array::from_fn(|i| 0.5 * (points[a][i] + points[b][i]));
            let mid = points.len() + added_points.len();
            // Midpoints of distinct edges must stay distinct points.
            if points.iter().chain(&added_points).any(|p| *p == m) {
                continue;
            }
            added_points.push(m);
            added_metrics.push(evaluate(m)?);
            for &c in around {
                used[c] = true;
                replaced[c] = true;
                let t = cells[c];
                // The two halves keep the orientation: a point on the edge replaces one end.
                next.push(t.map(|v| if v == b { mid } else { v }));
                next.push(t.map(|v| if v == a { mid } else { v }));
            }
            splits += 1;
        }
        if splits == 0 {
            break;
        }
        report.splits += splits;
        points.extend(added_points);
        metrics.extend(added_metrics);
        cells = cells
            .iter()
            .enumerate()
            .filter(|(i, _)| !replaced[*i])
            .map(|(_, t)| *t)
            .chain(next)
            .collect();
        current = TetTopology::new(points.clone(), cells.clone(), options.max_cells)?;
        if let Some(flip) = options.flips {
            let q = |p: &[Point], t: [usize; 4]| metric_quality(p, &metrics, t);
            let (better, flips) = improve_by(&current, &BTreeSet::new(), flip, &q)?;
            cells = better.cells().to_vec();
            current = better;
            report.flips.push(flips);
        }
        if options.smooth_passes > 0 {
            let opt = SmoothOptions {
                max_passes: options.smooth_passes,
                slide_planar_boundary: true,
                ..SmoothOptions::default()
            };
            let (smoothed, _) = smooth_vertices_by(&current, &opt, &smooth_quality)?;
            for (v, (old, new)) in points.iter_mut().zip(smoothed.points()).enumerate() {
                if old != new {
                    *old = *new;
                    metrics[v] = evaluate(*new)?;
                }
            }
            current = smoothed;
            cells = current.cells().to_vec();
        }
        report.passes += 1;
    }
    if options.coarsen_passes > 0 {
        let lo = std::f64::consts::FRAC_1_SQRT_2;
        let len =
            |p: &[Point], a: usize, b: usize| edge_length(p[a], p[b], &metrics[a], &metrics[b]);
        let opt = CollapseOptions {
            min_length: lo,
            max_length: options.max_length,
            max_passes: options.coarsen_passes,
            // Never make the worst cell worse than it already is.
            min_quality: stats(&points, &metrics, &cells).min_quality,
            ..CollapseOptions::default()
        };
        let (coarse, _, map) = coarsen(&current, &opt, &smooth_quality, &len)?;
        let mut new_metrics = vec![metrics[0]; coarse.points().len()];
        for (old, new) in map.iter().enumerate() {
            if let Some(n) = new {
                new_metrics[*n] = metrics[old];
            }
        }
        points = coarse.points().to_vec();
        metrics = new_metrics;
        cells = coarse.cells().to_vec();
        current = coarse;
    }
    let after = stats(&points, &metrics, &cells);
    report.cells_after = cells.len();
    report.longest_edge_after = after.longest;
    report.edges_in_band_after = after.in_band;
    report.min_quality_after = after.min_quality;
    report.converged = after.longest <= options.max_length;
    if report.passes == 0 {
        current = top.clone();
    }
    Ok((current, report))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshing::tet_delaunay::{tetrahedralize, Limits};
    fn cube() -> TetTopology {
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        tetrahedralize(&p, Limits::default()).unwrap()
    }
    fn volume(t: &TetTopology) -> f64 {
        t.cells()
            .iter()
            .map(|c| {
                let [a, b, cc, d] = c.map(|i| t.points()[i]);
                let u: Point = std::array::from_fn(|i| b[i] - a[i]);
                let v: Point = std::array::from_fn(|i| cc[i] - a[i]);
                let w: Point = std::array::from_fn(|i| d[i] - a[i]);
                (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
                    + u[2] * (v[0] * w[1] - v[1] * w[0]))
                    / 6.
            })
            .sum()
    }
    fn boundary_area(t: &TetTopology) -> f64 {
        t.boundary()
            .iter()
            .map(|f| {
                let [a, b, c] = f.map(|i| t.points()[i]);
                let u: Point = std::array::from_fn(|i| b[i] - a[i]);
                let v: Point = std::array::from_fn(|i| c[i] - a[i]);
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
            })
            .sum()
    }
    #[test]
    fn an_isotropic_field_refines_the_cube_to_the_target_band() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |_: Point| Metric::isotropic(0.25);
        let (out, r) = refine(&cube(), &field, &bounds, RefineOptions::default()).unwrap();
        assert!(r.converged, "{r:?}");
        assert!(r.longest_edge_before > 3. && r.longest_edge_after <= std::f64::consts::SQRT_2);
        assert!((volume(&out) - 1.).abs() < 1e-12);
        assert!((boundary_area(&out) - 6.).abs() < 1e-12);
        // The realised count is the same order as the closed-form prediction.
        let predicted = 1. / (0.25f64.powi(3) * crate::meshing::metric::UNIT_TET_VOLUME);
        let ratio = out.cells().len() as f64 / predicted;
        assert!((0.3..3.).contains(&ratio), "ratio {ratio}");
        assert!(r.min_quality_after > 0., "no inverted or flat cells");
    }
    #[test]
    fn an_anisotropic_layer_makes_thin_cells_across_it() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        // Size 0.1 across the plane x = 0.5 growing to 0.4 away from it; 0.4 along y and z.
        let field = |p: Point| {
            let hx = 0.1 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes(
                [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                [hx.min(0.4), 0.4, 0.4],
            )
        };
        let (out, r) = refine(&cube(), &field, &bounds, RefineOptions::default()).unwrap();
        let off = RefineOptions {
            smooth_passes: 0,
            ..RefineOptions::default()
        };
        let (_, r0) = refine(&cube(), &field, &bounds, off).unwrap();
        // Smoothing makes the mesh conform better to the metric (more edges in the band, a much better worst cell) with
        // fewer cells, because it stops the over-refinement that sliver cells provoke.
        assert!(
            r.edges_in_band_after > r0.edges_in_band_after,
            "{r:?} {r0:?}"
        );
        assert!(
            r.min_quality_after > 3. * r0.min_quality_after,
            "{r:?} {r0:?}"
        );
        assert!(r.cells_after < r0.cells_after);
        assert!(r.converged && r.splits > 0, "{r:?}");
        assert!((volume(&out) - 1.).abs() < 1e-12);
        assert!((boundary_area(&out) - 6.).abs() < 1e-12);
        // Count the vertices: many more cross the layer than in the far field.
        let near = out
            .points()
            .iter()
            .filter(|p| (p[0] - 0.5).abs() < 0.1)
            .count();
        let far = out
            .points()
            .iter()
            .filter(|p| (p[0] - 0.5).abs() > 0.4)
            .count();
        // Equal slab widths: the layer is still denser than the far field (the unsmoothed mesh reaches 2x only by over-refining).
        assert!(
            near as f64 > 1.3 * far.max(1) as f64,
            "near {near} far {far}"
        );
        // Cells hugging the layer are thinner across it (x) than along it (y).
        let (mut sx, mut sy, mut n) = (0., 0., 0);
        for t in out.cells() {
            let p = t.map(|v| out.points()[v]);
            if p.iter().all(|q| (q[0] - 0.5).abs() < 0.15) {
                let span = |k: usize| {
                    p.iter().map(|q| q[k]).fold(f64::MIN, f64::max)
                        - p.iter().map(|q| q[k]).fold(f64::MAX, f64::min)
                };
                sx += span(0);
                sy += span(1);
                n += 1;
            }
        }
        assert!(n > 20, "cells in the layer: {n}");
        assert!(
            sx < 0.8 * sy,
            "mean x span {} vs y span {}",
            sx / n as f64,
            sy / n as f64
        );
    }
    #[test]
    fn strong_anisotropy_keeps_a_valid_conforming_mesh_but_not_its_quality() {
        // Ratio 10 across the layer: the worst cell stays poor even with smoothing (see
        // the smoothing test; collapse or metric-Delaunay insertion are still needed).
        // What must always hold is validity, conformity and the geometry.
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |p: Point| {
            let hx = 0.04 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes(
                [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                [hx.min(0.4), 0.4, 0.4],
            )
        };
        let options = RefineOptions {
            max_passes: 10,
            ..RefineOptions::default()
        };
        let (out, r) = refine(&cube(), &field, &bounds, options).unwrap();
        assert!(r.splits > 0 && r.cells_after > r.cells_before);
        assert!((volume(&out) - 1.).abs() < 1e-12);
        assert!((boundary_area(&out) - 6.).abs() < 1e-12);
        assert!(r.min_quality_after > 0., "no flat cell: {r:?}");
        assert!(r.longest_edge_after < r.longest_edge_before);
    }
    fn q_unsorted(out: &TetTopology, field: &dyn Fn(Point) -> Result<Metric, super::super::metric::MetricError>) -> Vec<f64> {
        let metrics: Vec<Metric> = out.points().iter().map(|p| field(*p).unwrap()).collect();
        out.cells().iter().map(|&t| metric_quality(out.points(), &metrics, t)).collect()
    }
    #[test]
    fn flat_boundary_flips_raise_the_worst_cell_of_a_converging_four_to_one_layer() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |p: Point| {
            let hx = 0.1 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [hx.min(0.4), 0.4, 0.4])
        };
        let worst = |planar: bool| {
            let options = RefineOptions { max_passes: 40, flips: Some(FlipOptions { planar_boundary: planar, ..FlipOptions::default() }), ..RefineOptions::default() };
            let (out, r) = refine(&cube(), &field, &bounds, options).unwrap();
            assert!(r.converged, "planar {planar}: {r:?}");
            assert!((volume(&out) - 1.).abs() < 1e-12 && (boundary_area(&out) - 6.).abs() < 1e-12);
            r.min_quality_after
        };
        let (off, on) = (worst(false), worst(true));
        eprintln!("4:1 worst cell {off:.3} -> {on:.3}");
        assert!(on > 1.4 * off, "{off} -> {on}");
    }

    #[test]
    #[ignore = "release: 23 s; run with --release --ignored"]
    fn flat_boundary_flips_stop_the_slivers_that_bisection_leaves_on_the_cube_faces() {
        // 10:1 across a layer, 40 passes: bisection leaves boundary slivers whose triangulation the locked boundary cannot
        // change (worst cell 1e-15 and over a third of the cells below 0.1 without the flips).
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |p: Point| {
            let hx = 0.04 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [hx.min(0.4), 0.4, 0.4])
        };
        let run = |planar: bool| {
            let options = RefineOptions { max_passes: 40, flips: Some(FlipOptions { planar_boundary: planar, ..FlipOptions::default() }), ..RefineOptions::default() };
            let (out, r) = refine(&cube(), &field, &bounds, options).unwrap();
            let mut q = q_unsorted(&out, &field);
            q.sort_by(f64::total_cmp);
            (out, r, q)
        };
        let (_, _, off) = run(false);
        let (out, _, on) = run(true);
        let bad = |q: &[f64]| q.iter().filter(|x| **x < 0.1).count();
        eprintln!("flat boundary flips: min {:.1e} -> {:.1e}, median {:.3} -> {:.3}, below 0.1: {} of {} -> {} of {}", off[0], on[0], off[off.len() / 2], on[on.len() / 2], bad(&off), off.len(), bad(&on), on.len());
        assert!(on[0] > 1e-6 && on[0] > 1e6 * off[0].max(1e-300), "worst {:e} vs {:e}", on[0], off[0]);
        assert!(on[on.len() / 2] > 1.5 * off[off.len() / 2], "median");
        assert!(bad(&on) * 3 < bad(&off), "{} vs {}", bad(&on), bad(&off));
        // Same domain: volume, surface area and the cube surface itself.
        assert!((volume(&out) - 1.).abs() < 1e-12);
        assert!((boundary_area(&out) - 6.).abs() < 1e-12);
        for v in out.boundary().iter().flatten() {
            let p = out.points()[*v];
            assert!((0..3).any(|k| p[k] == 0. || p[k] == 1.), "boundary vertex left the cube surface");
        }
    }
    #[test]
    #[ignore = "study: which stage creates the slivers at 10:1"]
    fn which_stage_creates_the_slivers_at_strong_anisotropy() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |p: Point| {
            let hx = 0.04 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [hx.min(0.4), 0.4, 0.4])
        };
        let variants: Vec<(&str, RefineOptions)> = vec![
            ("default", RefineOptions { max_passes: 40, ..RefineOptions::default() }),
            ("no smoothing", RefineOptions { max_passes: 40, smooth_passes: 0, ..RefineOptions::default() }),
            ("no coarsening", RefineOptions { max_passes: 40, coarsen_passes: 0, ..RefineOptions::default() }),
            ("no smoothing, no coarsening", RefineOptions { max_passes: 40, smooth_passes: 0, coarsen_passes: 0, ..RefineOptions::default() }),
            ("planar boundary flips", RefineOptions { max_passes: 40, flips: Some(FlipOptions { planar_boundary: true, ..FlipOptions::default() }), ..RefineOptions::default() }),
            ("splits only", RefineOptions { max_passes: 40, smooth_passes: 0, coarsen_passes: 0, flips: None, ..RefineOptions::default() }),
        ];
        for (name, options) in variants {
            let (out, r) = refine(&cube(), &field, &bounds, options).unwrap();
            let mut q = q_unsorted(&out, &field);
            q.sort_by(f64::total_cmp);
            eprintln!("{name:<28}: {:>5} cells, passes {}, converged {}, min {:.2e}, p1 {:.3}, median {:.3}, below 0.1: {}", q.len(), r.passes, r.converged, q[0], q[q.len() / 100], q[q.len() / 2], q.iter().filter(|x| **x < 0.1).count());
        }
    }
    #[test]
    #[ignore = "study: quality distribution at strong anisotropy"]
    fn quality_distribution_at_strong_anisotropy() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        for ratio in [4.0f64, 10.0] {
            let field = |p: Point| {
                let hx = 0.4 / ratio + 0.7 * (p[0] - 0.5).abs();
                Metric::from_sizes([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [hx.min(0.4), 0.4, 0.4])
            };
            for passes in [10usize, 40] {
                let options = RefineOptions { max_passes: passes, flips: Some(FlipOptions { planar_boundary: true, ..FlipOptions::default() }), ..RefineOptions::default() };
                let (out, r) = refine(&cube(), &field, &bounds, options).unwrap();
                let metrics: Vec<Metric> = out.points().iter().map(|p| field(*p).unwrap()).collect();
                let mut q: Vec<f64> = out.cells().iter().map(|&t| metric_quality(out.points(), &metrics, t)).collect();
                q.sort_by(f64::total_cmp);
                let pct = |f: f64| q[((q.len() - 1) as f64 * f) as usize];
                let signed: Vec<f64> = out.cells().iter().map(|t| { let p = t.map(|v| out.points()[v]); let d = |a: usize| [p[a][0] - p[0][0], p[a][1] - p[0][1], p[a][2] - p[0][2]]; let (a, b, c) = (d(1), d(2), d(3)); (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0 }).collect();
                eprintln!("  min quality {:e}, min signed volume {:e}, max {:e}, volume sum {:.12}", q[0], signed.iter().cloned().fold(f64::INFINITY, f64::min), signed.iter().cloned().fold(f64::NEG_INFINITY, f64::max), signed.iter().sum::<f64>());
                if passes == 40 {
                    let on_b = |v: usize| (0..3).any(|k| out.points()[v][k] == 0.0 || out.points()[v][k] == 1.0);
                    let mut by_nb = [0usize; 5];
                    let mut interior_bad = 0;
                    let (mut xs, mut ys, mut zs) = ([0usize; 5], [0usize; 5], [0usize; 5]);
                    for (t, &qq) in out.cells().iter().zip(&q_unsorted(&out, &field)) {
                        if qq < 0.1 {
                            let nb = t.iter().filter(|&&v| on_b(v)).count();
                            by_nb[nb] += 1;
                            if nb == 0 { interior_bad += 1; }
                            let c: Vec<f64> = (0..3).map(|k| t.iter().map(|&v| out.points()[v][k]).sum::<f64>() / 4.0).collect();
                            xs[((c[0] * 5.0) as usize).min(4)] += 1; ys[((c[1] * 5.0) as usize).min(4)] += 1; zs[((c[2] * 5.0) as usize).min(4)] += 1;
                        }
                    }
                    eprintln!("  bad cells (<0.1) by number of boundary vertices {by_nb:?}, interior-only {interior_bad}; centroid histogram x {xs:?} y {ys:?} z {zs:?}");
                }
                eprintln!(
                    "ratio {ratio} passes {passes}: {} cells, converged {}, band {:.2}; quality min {:.4} p1 {:.3} p10 {:.3} median {:.3}; cells below 0.1: {}",
                    q.len(), r.converged, r.edges_in_band_after, q[0], pct(0.01), pct(0.1), pct(0.5), q.iter().filter(|x| **x < 0.1).count()
                );
            }
        }
    }
    #[test]
    fn metric_smoothing_raises_the_worst_cell_of_a_strongly_anisotropic_mesh() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |p: Point| {
            let hx = 0.04 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes(
                [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                [hx.min(0.4), 0.4, 0.4],
            )
        };
        let base = RefineOptions {
            max_passes: 10,
            smooth_passes: 0,
            ..RefineOptions::default()
        };
        let (_, off) = refine(&cube(), &field, &bounds, base).unwrap();
        let on = RefineOptions {
            smooth_passes: 8,
            ..base
        };
        let (out, r) = refine(&cube(), &field, &bounds, on).unwrap();
        eprintln!(
            "min metric quality: splits+flips {:.4}, with smoothing {:.4}; cells {} -> {}",
            off.min_quality_after, r.min_quality_after, off.cells_after, r.cells_after
        );
        assert!(r.min_quality_after > off.min_quality_after, "{off:?} {r:?}");
        assert!((volume(&out) - 1.).abs() < 1e-12);
        assert!((boundary_area(&out) - 6.).abs() < 1e-12);
        let ids: Vec<_> = out.boundary().iter().flatten().copied().collect();
        for v in ids {
            let p = out.points()[v];
            assert!(
                (0..3).any(|k| p[k] == 0. || p[k] == 1.),
                "boundary vertex left the cube surface"
            );
        }
    }
    #[test]
    fn coarsening_removes_over_refinement_without_losing_the_metric_band() {
        let bounds = MetricBounds::new(0.01, 10., 100.).unwrap();
        let field = |p: Point| {
            let hx = 0.1 + 0.7 * (p[0] - 0.5).abs();
            Metric::from_sizes(
                [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                [hx.min(0.4), 0.4, 0.4],
            )
        };
        let off = RefineOptions {
            coarsen_passes: 0,
            ..RefineOptions::default()
        };
        let (_, r0) = refine(&cube(), &field, &bounds, off).unwrap();
        let on = RefineOptions {
            coarsen_passes: 10,
            ..off
        };
        let (out, r) = refine(&cube(), &field, &bounds, on).unwrap();
        eprintln!(
            "COARSEN cells {} -> {}, band {:.3} -> {:.3}, minq {:.4} -> {:.4}, longest {:.3} -> {:.3}, converged {}",
            r0.cells_after, r.cells_after, r0.edges_in_band_after, r.edges_in_band_after,
            r0.min_quality_after, r.min_quality_after, r0.longest_edge_after, r.longest_edge_after, r.converged
        );
        assert!(r.cells_after < r0.cells_after);
        assert!(r.converged, "{r:?}");
        assert!(
            r.edges_in_band_after >= r0.edges_in_band_after - 1e-12,
            "{r:?} {r0:?}"
        );
        assert!(
            r.min_quality_after >= r0.min_quality_after - 1e-12,
            "{r:?} {r0:?}"
        );
        assert!((volume(&out) - 1.).abs() < 1e-12);
        assert!((boundary_area(&out) - 6.).abs() < 1e-12);
    }
    #[test]
    fn invalid_fields_budgets_and_a_met_target_are_handled_explicitly() {
        let bounds = MetricBounds::new(0.1, 10., 100.).unwrap();
        // A field below the size bound is refused with the metric error.
        let too_small = |_: Point| Metric::isotropic(0.01);
        assert!(refine(&cube(), &too_small, &bounds, RefineOptions::default()).is_err());
        let nan = |_: Point| Metric::new([[f64::NAN, 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
        assert!(refine(&cube(), &nan, &bounds, RefineOptions::default()).is_err());
        // The cell budget is a hard limit.
        let fine = |_: Point| Metric::isotropic(0.1);
        let tiny = RefineOptions {
            max_cells: 50,
            ..RefineOptions::default()
        };
        assert!(matches!(
            refine(&cube(), &fine, &bounds, tiny),
            Err(MeshError::LimitExceeded(_))
        ));
        // A coarse field changes nothing.
        let coarse = |_: Point| Metric::isotropic(10.);
        let (same, r) = refine(&cube(), &coarse, &bounds, RefineOptions::default()).unwrap();
        assert_eq!(same.cells(), cube().cells());
        assert!(r.converged && r.splits == 0 && r.passes == 0);
        // Splitting without reconnection is allowed and still conforming.
        let split_only = RefineOptions {
            flips: None,
            ..RefineOptions::default()
        };
        let iso = |_: Point| Metric::isotropic(0.3);
        let (out, r) = refine(&cube(), &iso, &bounds, split_only).unwrap();
        assert!(r.flips.is_empty() && (volume(&out) - 1.).abs() < 1e-12);
    }
}
