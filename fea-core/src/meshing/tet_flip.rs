//! Sliver elimination by local reconnection of a validated tetrahedral topology.
//!
//! Two exact-predicate operations: the 2-3 face flip and edge removal (the general
//! n-to-2(n-2) reconnection; n = 3 is the 3-2 flip, n = 4 the 4-4 flip), the latter
//! choosing the best triangulation of the edge's link polygon by dynamic
//! programming. An operation is accepted only when every new tetrahedron is
//! positively oriented, the replaced cavity is re-filled exactly (the new tetrahedra
//! are the only possible triangulation of the same polyhedron) and the worst
//! radius ratio among the affected cells improves by `min_gain`, so the pass
//! terminates. Boundary faces, caller-protected faces (for example recovered PLC
//! sub-facets) and every edge lying on one of them are never touched, so the
//! boundary and its conformity are preserved. Vertices never move.
use super::{
    quality::element_quality,
    tet_topology::{orientation, Point, TetTopology},
    MeshError,
};
use crate::element::ElementKind;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct FlipOptions {
    pub max_passes: usize,
    /// Only cells with a radius ratio below this are reconnected.
    pub target: f64,
    /// Smallest worst-quality gain an operation must achieve.
    pub min_gain: f64,
    /// Longest edge link (number of tetrahedra around an edge) that is removed.
    pub max_link: usize,
    pub max_cells: usize,
}
impl Default for FlipOptions {
    fn default() -> Self {
        Self {
            max_passes: 50,
            target: 0.4,
            min_gain: 1e-6,
            max_link: 8,
            max_cells: 5_000_000,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct FlipReport {
    pub passes: usize,
    pub face_flips: usize,
    /// Edge removals by link size (3 = the 3-2 flip).
    pub edge_removals: BTreeMap<usize, usize>,
    pub min_radius_ratio_before: f64,
    pub min_radius_ratio_after: f64,
    pub cells_below_target_before: usize,
    pub cells_below_target_after: usize,
}
fn quality(points: &[Point], t: [usize; 4]) -> f64 {
    let q = element_quality(ElementKind::Tet4, &t.map(|v| points[v])).radius_ratio;
    if q.is_finite() {
        q.max(0.)
    } else {
        0.
    }
}
/// The ordering of `t` with positive orientation, if it is not flat.
fn positive(points: &[Point], t: [usize; 4]) -> Result<Option<[usize; 4]>, MeshError> {
    Ok(
        match orientation(points[t[0]], points[t[1]], points[t[2]], points[t[3]])? {
            Ordering::Greater => Some(t),
            Ordering::Less => Some([t[1], t[0], t[2], t[3]]),
            Ordering::Equal => None,
        },
    )
}
fn key3(mut f: [usize; 3]) -> [usize; 3] {
    f.sort_unstable();
    f
}
fn key2(a: usize, b: usize) -> [usize; 2] {
    [a.min(b), a.max(b)]
}
fn faces_of(t: [usize; 4]) -> [[usize; 3]; 4] {
    let [a, b, c, d] = t;
    [[b, c, d], [a, d, c], [a, b, d], [a, c, b]]
}
/// Quality of a link triangle's tetrahedron pair, with the pair itself.
type LinkTriangle = Option<(f64, [[usize; 4]; 2])>;
struct Candidate {
    gain: f64,
    old: Vec<usize>,
    new: Vec<[usize; 4]>,
    link: Option<usize>,
}
/// Cell quality in `[0, 1]`-like units, larger is better.
pub type QualityFn<'a> = &'a dyn Fn(&[Point], [usize; 4]) -> f64;
struct Pass<'a> {
    points: &'a [Point],
    quality: QualityFn<'a>,
    cells: Vec<[usize; 4]>,
    alive: Vec<bool>,
    touched: Vec<bool>,
    face_cells: BTreeMap<[usize; 3], Vec<usize>>,
    edge_cells: BTreeMap<[usize; 2], Vec<usize>>,
    locked_faces: BTreeSet<[usize; 3]>,
    locked_edges: BTreeSet<[usize; 2]>,
    options: FlipOptions,
}
impl Pass<'_> {
    fn q(&self, t: [usize; 4]) -> f64 {
        (self.quality)(self.points, t)
    }
    fn face_flip(&self, t: usize, k: usize) -> Result<Option<Candidate>, MeshError> {
        let face = faces_of(self.cells[t])[k];
        let key = key3(face);
        if self.locked_faces.contains(&key) {
            return Ok(None);
        }
        let d = self.cells[t][k];
        let Some(&u) = self.face_cells[&key].iter().find(|&&c| c != t) else {
            return Ok(None);
        };
        if self.touched[u] {
            return Ok(None);
        }
        let Some(&e) = self.cells[u].iter().find(|v| !face.contains(v)) else {
            return Ok(None);
        };
        let mut signs = Vec::with_capacity(3);
        let mut new = Vec::with_capacity(3);
        for (x, y) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
            let raw = [x, y, d, e];
            let s = orientation(
                self.points[x],
                self.points[y],
                self.points[d],
                self.points[e],
            )?;
            if s == Ordering::Equal {
                return Ok(None);
            }
            signs.push(s);
            new.push(positive(self.points, raw)?.unwrap());
        }
        // d-e crosses the face interior exactly when the three cones agree.
        if signs.iter().any(|s| *s != signs[0]) {
            return Ok(None);
        }
        let old_min = self.q(self.cells[t]).min(self.q(self.cells[u]));
        let new_min = new.iter().map(|c| self.q(*c)).fold(f64::INFINITY, f64::min);
        Ok(Some(Candidate {
            gain: new_min - old_min,
            old: vec![t, u],
            new,
            link: None,
        }))
    }
    fn edge_removal(&self, edge: [usize; 2]) -> Result<Option<Candidate>, MeshError> {
        if self.locked_edges.contains(&edge) {
            return Ok(None);
        }
        let ring_cells = &self.edge_cells[&edge];
        let n = ring_cells.len();
        if !(3..=self.options.max_link).contains(&n) || ring_cells.iter().any(|&c| self.touched[c])
        {
            return Ok(None);
        }
        let [p, q] = edge;
        // Walk the ring of tetrahedra around the edge to order its link vertices.
        let others = |c: usize| -> [usize; 2] {
            let v: Vec<usize> = self.cells[c]
                .iter()
                .copied()
                .filter(|x| *x != p && *x != q)
                .collect();
            [v[0], v[1]]
        };
        let mut link = Vec::with_capacity(n);
        let first = others(ring_cells[0]);
        link.push(first[0]);
        let (mut at, mut prev) = (first[1], ring_cells[0]);
        while at != first[0] {
            link.push(at);
            let Some(&next) = ring_cells
                .iter()
                .find(|&&c| c != prev && others(c).contains(&at))
            else {
                return Ok(None); // open ring: the edge touches the boundary
            };
            let [x, y] = others(next);
            at = if x == at { y } else { x };
            prev = next;
            if link.len() > n {
                return Ok(None);
            }
        }
        if link.len() != n {
            return Ok(None);
        }
        // Every ring tetrahedron (p, q, link[i], link[i + 1]) must turn the same way.
        let turn = |i: usize| {
            orientation(
                self.points[p],
                self.points[q],
                self.points[link[i]],
                self.points[link[(i + 1) % n]],
            )
        };
        let sigma = turn(0)?;
        if sigma == Ordering::Equal {
            return Ok(None);
        }
        for i in 1..n {
            if turn(i)? != sigma {
                return Ok(None);
            }
        }
        // Best triangulation of the link polygon (maximise the worst quality). For
        // the polygon winding the old ring fixes, a link triangle (i < k < j) must
        // see p on the side opposite to sigma and q on sigma's side; this also
        // guarantees the new tetrahedra tile the old star without overlap.
        let (side_p, side_q) = if sigma == Ordering::Greater {
            (Ordering::Less, Ordering::Greater)
        } else {
            (Ordering::Greater, Ordering::Less)
        };
        let tri = |i: usize, k: usize, j: usize| -> Result<LinkTriangle, MeshError> {
            let (a, b, c) = (link[i], link[k], link[j]);
            let sp = orientation(
                self.points[a],
                self.points[b],
                self.points[c],
                self.points[p],
            )?;
            let sq = orientation(
                self.points[a],
                self.points[b],
                self.points[c],
                self.points[q],
            )?;
            if sp != side_p || sq != side_q {
                return Ok(None);
            }
            let tp = positive(self.points, [a, b, c, p])?.unwrap();
            let tq = positive(self.points, [a, b, c, q])?.unwrap();
            Ok(Some((self.q(tp).min(self.q(tq)), [tp, tq])))
        };
        let mut best = vec![vec![f64::NEG_INFINITY; n]; n];
        let mut pick = vec![vec![0usize; n]; n];
        for i in 0..n - 1 {
            best[i][i + 1] = f64::INFINITY;
        }
        for span in 2..n {
            for i in 0..n - span {
                let j = i + span;
                for k in i + 1..j {
                    let (l, r) = (best[i][k], best[k][j]);
                    if l == f64::NEG_INFINITY || r == f64::NEG_INFINITY {
                        continue;
                    }
                    let Some((t, _)) = tri(i, k, j)? else {
                        continue;
                    };
                    let v = l.min(r).min(t);
                    if v > best[i][j] {
                        best[i][j] = v;
                        pick[i][j] = k;
                    }
                }
            }
        }
        let new_min = best[0][n - 1];
        if !(new_min.is_finite() && new_min > 0.) {
            return Ok(None);
        }
        let mut new = Vec::with_capacity(2 * (n - 2));
        let mut stack = vec![(0, n - 1)];
        while let Some((i, j)) = stack.pop() {
            if j - i < 2 {
                continue;
            }
            let k = pick[i][j];
            let (_, pair) = tri(i, k, j)?.unwrap();
            new.extend(pair);
            stack.push((i, k));
            stack.push((k, j));
        }
        let old_min = ring_cells
            .iter()
            .map(|&c| self.q(self.cells[c]))
            .fold(f64::INFINITY, f64::min);
        Ok(Some(Candidate {
            gain: new_min - old_min,
            old: ring_cells.clone(),
            new,
            link: Some(n),
        }))
    }
}
/// Reconnect low-quality tetrahedra of `top` in place of nothing else: vertices,
/// boundary faces and `protected` faces are preserved exactly.
pub fn improve(
    top: &TetTopology,
    protected: &BTreeSet<[usize; 3]>,
    options: FlipOptions,
) -> Result<(TetTopology, FlipReport), MeshError> {
    improve_by(top, protected, options, &quality)
}
/// [`improve`] with a caller-supplied cell quality (for example the radius ratio in
/// a metric). The quality must be positive for valid cells and is only compared,
/// so any monotone scale works; `options.target` is in the same units.
pub fn improve_by(
    top: &TetTopology,
    protected: &BTreeSet<[usize; 3]>,
    options: FlipOptions,
    quality: QualityFn<'_>,
) -> Result<(TetTopology, FlipReport), MeshError> {
    let points = top.points();
    let mut cells: Vec<[usize; 4]> = top.cells().to_vec();
    let measure = |cells: &[[usize; 4]]| {
        let q: Vec<f64> = cells.iter().map(|c| quality(points, *c)).collect();
        (
            q.iter().copied().fold(f64::INFINITY, f64::min),
            q.iter().filter(|x| **x < options.target).count(),
        )
    };
    let (min_before, below_before) = measure(&cells);
    let volume_before = volume(points, &cells);
    let boundary_before: BTreeSet<[usize; 3]> = top.boundary().iter().map(|f| key3(*f)).collect();
    let mut report = FlipReport {
        passes: 0,
        face_flips: 0,
        edge_removals: BTreeMap::new(),
        min_radius_ratio_before: min_before,
        min_radius_ratio_after: min_before,
        cells_below_target_before: below_before,
        cells_below_target_after: below_before,
    };
    for _ in 0..options.max_passes {
        let mut face_cells = BTreeMap::<[usize; 3], Vec<usize>>::new();
        let mut edge_cells = BTreeMap::<[usize; 2], Vec<usize>>::new();
        for (i, t) in cells.iter().enumerate() {
            for f in faces_of(*t) {
                face_cells.entry(key3(f)).or_default().push(i);
            }
            for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
                edge_cells.entry(key2(t[a], t[b])).or_default().push(i);
            }
        }
        let mut locked_faces: BTreeSet<[usize; 3]> = protected.clone();
        locked_faces.extend(
            face_cells
                .iter()
                .filter(|(_, c)| c.len() == 1)
                .map(|(f, _)| *f),
        );
        let mut locked_edges = BTreeSet::new();
        for f in &locked_faces {
            locked_edges.insert(key2(f[0], f[1]));
            locked_edges.insert(key2(f[0], f[2]));
            locked_edges.insert(key2(f[1], f[2]));
        }
        let n = cells.len();
        let mut pass = Pass {
            points,
            quality,
            cells: cells.clone(),
            alive: vec![true; n],
            touched: vec![false; n],
            face_cells,
            edge_cells,
            locked_faces,
            locked_edges,
            options,
        };
        let mut order: Vec<(f64, usize)> = (0..n)
            .map(|i| (quality(points, cells[i]), i))
            .filter(|(q, _)| *q < options.target)
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut changed = 0;
        for (_, t) in order {
            if !pass.alive[t] || pass.touched[t] {
                continue;
            }
            let mut best: Option<Candidate> = None;
            let mut consider = |c: Option<Candidate>| {
                if let Some(c) = c {
                    if c.gain > options.min_gain && best.as_ref().is_none_or(|b| c.gain > b.gain) {
                        best = Some(c);
                    }
                }
            };
            for k in 0..4 {
                consider(pass.face_flip(t, k)?);
            }
            let ce = pass.cells[t];
            for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
                consider(pass.edge_removal(key2(ce[a], ce[b]))?);
            }
            let Some(chosen) = best else { continue };
            if pass.cells.len() + chosen.new.len() > options.max_cells {
                return Err(MeshError::LimitExceeded("flip cell budget exceeded".into()));
            }
            for &c in &chosen.old {
                pass.alive[c] = false;
                pass.touched[c] = true;
            }
            for c in chosen.new {
                pass.cells.push(c);
                pass.alive.push(true);
                pass.touched.push(true);
            }
            match chosen.link {
                None => report.face_flips += 1,
                Some(n) => *report.edge_removals.entry(n).or_default() += 1,
            }
            changed += 1;
        }
        report.passes += 1;
        cells = pass
            .cells
            .iter()
            .zip(&pass.alive)
            .filter(|(_, a)| **a)
            .map(|(c, _)| *c)
            .collect();
        if changed == 0 {
            break;
        }
    }
    let result = TetTopology::new(points.to_vec(), cells.clone(), options.max_cells)?;
    let boundary_after: BTreeSet<[usize; 3]> = result.boundary().iter().map(|f| key3(*f)).collect();
    if boundary_after != boundary_before {
        return Err(MeshError::NonConforming(
            "reconnection changed the boundary".into(),
        ));
    }
    let volume_after = volume(points, &cells);
    if (volume_after - volume_before).abs() > 1e-9 * volume_before.abs().max(f64::MIN_POSITIVE) {
        return Err(MeshError::NonConforming(
            "reconnection changed the total volume".into(),
        ));
    }
    let (min_after, below_after) = measure(&cells);
    report.min_radius_ratio_after = min_after;
    report.cells_below_target_after = below_after;
    Ok((result, report))
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshing::tet_delaunay::{tetrahedralize, Limits};
    /// Reproducible scatter of points in the unit cube plus its corners.
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
    #[test]
    fn reconnection_removes_slivers_without_touching_the_boundary_or_volume() {
        let mut total_ops = 0;
        let mut removed = 0;
        for seed in 1..=6 {
            let points = cloud(60, seed);
            let top = tetrahedralize(&points, Limits::default()).unwrap();
            let (out, report) = improve(&top, &BTreeSet::new(), FlipOptions::default()).unwrap();
            assert!(report.min_radius_ratio_after >= report.min_radius_ratio_before);
            assert!(report.cells_below_target_after <= report.cells_below_target_before);
            assert_eq!(out.points(), top.points());
            let mut b0: Vec<_> = top.boundary().iter().map(|f| key3(*f)).collect();
            let mut b1: Vec<_> = out.boundary().iter().map(|f| key3(*f)).collect();
            b0.sort();
            b1.sort();
            assert_eq!(b0, b1);
            assert!((volume(out.points(), out.cells()) - 1.).abs() < 1e-12);
            total_ops += report.face_flips + report.edge_removals.values().sum::<usize>();
            removed += report.cells_below_target_before - report.cells_below_target_after;
        }
        assert!(total_ops > 0, "the random clouds contain slivers to remove");
        // Boundary-adjacent slivers are locked, but interior ones must go.
        assert!(removed > 0);
    }
    #[test]
    fn improvement_is_deterministic_and_idempotent() {
        let points = cloud(60, 3);
        let top = tetrahedralize(&points, Limits::default()).unwrap();
        let (a, ra) = improve(&top, &BTreeSet::new(), FlipOptions::default()).unwrap();
        let (b, _) = improve(&top, &BTreeSet::new(), FlipOptions::default()).unwrap();
        assert_eq!(a, b);
        let (c, rc) = improve(&a, &BTreeSet::new(), FlipOptions::default()).unwrap();
        // A second run never makes the worst cell worse.
        assert!(rc.min_radius_ratio_after >= ra.min_radius_ratio_after);
        assert!(c.cells().len() > 0);
    }
    #[test]
    fn protected_faces_are_never_flipped_away() {
        let points = cloud(60, 2);
        let top = tetrahedralize(&points, Limits::default()).unwrap();
        // Protect every interior face of the first twenty cells.
        let mut protected = BTreeSet::new();
        for c in top.cells().iter().take(20) {
            for f in faces_of(*c) {
                protected.insert(key3(f));
            }
        }
        let (out, _) = improve(&top, &protected, FlipOptions::default()).unwrap();
        let faces: BTreeSet<[usize; 3]> = out
            .cells()
            .iter()
            .flat_map(|c| faces_of(*c))
            .map(key3)
            .collect();
        assert!(protected.iter().all(|f| faces.contains(f)));
    }
    #[test]
    fn a_zero_gain_requirement_target_leaves_good_meshes_alone() {
        let points = cloud(30, 5);
        let top = tetrahedralize(&points, Limits::default()).unwrap();
        let options = FlipOptions {
            target: 0.0,
            ..FlipOptions::default()
        };
        let (out, report) = improve(&top, &BTreeSet::new(), options).unwrap();
        assert_eq!(out.cells(), top.cells());
        assert_eq!(report.passes, 1);
    }
}
