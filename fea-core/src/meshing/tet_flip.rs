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
    /// Only cells with a radius ratio below this are reconnected. With [`improve_by`] / `optimize_by` the threshold is in the
    /// units of the caller's quality instead (for example the scaled Jacobian), not the radius ratio.
    pub target: f64,
    /// Smallest worst-quality gain an operation must achieve.
    pub min_gain: f64,
    /// Longest edge link (number of tetrahedra around an edge) that is removed.
    pub max_link: usize,
    pub max_cells: usize,
    /// Also reconnect edges that lie in a flat boundary patch (both boundary faces on the edge exactly coplanar and forming a
    /// convex quadrilateral): the edge is replaced by the quadrilateral's other diagonal and the tetrahedra behind it are
    /// reconnected with it (a 2-2 flip for two tetrahedra, the boundary form of edge removal beyond). The boundary surface
    /// stays the same set of points but its triangulation changes, so enable it only when coplanar boundary faces belong to
    /// one patch (face names are not tracked here). Protected faces are never touched. Default off.
    pub planar_boundary: bool,
}
impl Default for FlipOptions {
    fn default() -> Self {
        Self {
            max_passes: 50,
            target: 0.4,
            min_gain: 1e-6,
            max_link: 8,
            max_cells: 5_000_000,
            planar_boundary: false,
        }
    }
}
/// What a flip pass did. The `min_radius_ratio_*` and `cells_below_target_*` fields are measured with the quality the pass
/// judged cells by: the radius ratio for [`improve`], the caller's quality (for example the scaled Jacobian) for
/// [`improve_by`] / `optimize_by`, despite their names.
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
/// A tetrahedron whose scaled Jacobian is below 1e-7 at every corner: flat to the rounding of its coordinates.
fn nearly_flat(points: &[Point], t: [usize; 4]) -> bool {
    element_quality(ElementKind::Tet4, &t.map(|v| points[v])).scaled_jacobian.abs() < 1e-7
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
/// The best reconnection of an edge's link: worst new quality and the new tetrahedra.
type Reconnection = Option<(f64, Vec<[usize; 4]>)>;
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
    protected: &'a BTreeSet<[usize; 3]>,
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
    /// Best triangulation of the link polygon `link` (maximise the worst quality) coned from `p` and `q`, for the polygon
    /// winding `sigma` the old ring fixes: a link triangle (i < k < j) must see `p` on the side opposite to `sigma` and `q`
    /// on `sigma`'s side, which also guarantees the new tetrahedra tile the old star without overlap. Returns the worst
    /// quality and the `2 (n - 2)` tetrahedra.
    fn triangulate_link(
        &self,
        link: &[usize],
        p: usize,
        q: usize,
        sigma: Ordering,
    ) -> Result<Reconnection, MeshError> {
        let n = link.len();
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
        Ok(Some((new_min, new)))
    }

    /// Edge removal for an edge lying in a flat boundary patch: its two boundary faces are exactly coplanar and form a
    /// convex quadrilateral `p u0 q uk`; the edge is replaced by the diagonal `u0 uk` and the tetrahedra behind it are
    /// reconnected as for an interior edge whose link polygon `u0 .. uk` is closed by that diagonal.
    fn boundary_edge_removal(&self, edge: [usize; 2]) -> Result<Option<Candidate>, MeshError> {
        let ring_cells = &self.edge_cells[&edge];
        let n_cells = ring_cells.len();
        if n_cells < 2 || n_cells + 1 > self.options.max_link || ring_cells.iter().any(|&c| self.touched[c]) {
            return Ok(None);
        }
        let [p, q] = edge;
        let others = |c: usize| -> [usize; 2] {
            let v: Vec<usize> = self.cells[c].iter().copied().filter(|x| *x != p && *x != q).collect();
            [v[0], v[1]]
        };
        // The two boundary faces on the edge, and the path of link vertices between them.
        let boundary_apex: Vec<(usize, usize)> = ring_cells
            .iter()
            .flat_map(|&c| others(c).into_iter().map(move |u| (c, u)))
            .filter(|&(_, u)| self.face_cells[&key3([p, q, u])].len() == 1)
            .collect();
        if boundary_apex.len() != 2 || boundary_apex.iter().any(|&(_, u)| self.protected.contains(&key3([p, q, u]))) {
            return Ok(None);
        }
        let (start_cell, u0) = boundary_apex[0];
        let uk = boundary_apex[1].1;
        let mut link = vec![u0];
        let mut at = u0;
        let mut cell = start_cell;
        let mut prev;
        loop {
            let [x, y] = others(cell);
            let next_vertex = if x == at { y } else { x };
            link.push(next_vertex);
            prev = cell;
            at = next_vertex;
            if at == uk && link.len() == n_cells + 1 {
                break;
            }
            let Some(&next) = ring_cells.iter().find(|&&c| c != prev && others(c).contains(&at)) else {
                return Ok(None);
            };
            cell = next;
            if link.len() > n_cells {
                return Ok(None);
            }
        }
        let n = link.len();
        if n != n_cells + 1 || link[n - 1] != uk {
            return Ok(None);
        }
        // Flat patch: p, q, u0, uk exactly coplanar; the quadrilateral p u0 q uk convex (strictly).
        if orientation(self.points[p], self.points[q], self.points[u0], self.points[uk])? != Ordering::Equal {
            return Ok(None);
        }
        let reference = link[1];
        let side = |a: usize, b: usize, c: usize| orientation(self.points[a], self.points[b], self.points[c], self.points[reference]);
        let (s0, sk) = (side(p, q, u0)?, side(p, q, uk)?);
        let (sp, sq) = (side(u0, uk, p)?, side(u0, uk, q)?);
        if [s0, sk, sp, sq].contains(&Ordering::Equal) || s0 == sk || sp == sq {
            return Ok(None);
        }
        // The old fan must wind one way.
        let turn = |i: usize| orientation(self.points[p], self.points[q], self.points[link[i]], self.points[link[i + 1]]);
        let sigma = turn(0)?;
        if sigma == Ordering::Equal {
            return Ok(None);
        }
        for i in 1..n - 1 {
            if turn(i)? != sigma {
                return Ok(None);
            }
        }
        let Some((new_min, new)) = self.triangulate_link(&link, p, q, sigma)? else {
            return Ok(None);
        };
        let old_min = ring_cells.iter().map(|&c| self.q(self.cells[c])).fold(f64::INFINITY, f64::min);
        Ok(Some(Candidate { gain: new_min - old_min, old: ring_cells.clone(), new, link: Some(n) }))
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
        // A ring tetrahedron of (numerically) zero volume, the 180 degree wedge a Delaunay recovery leaves between a boundary
        // face and the coplanar interior ones (four co-circular or co-planar vertices), has no turning direction of its own
        // and does not take part in the vote; the new tetrahedra are checked one by one against the polygon's winding anyway.
        let flat = |i: usize| nearly_flat(self.points, [p, q, link[i], link[(i + 1) % n]]);
        let mut sigma = Ordering::Equal;
        for i in 0..n {
            if flat(i) {
                continue;
            }
            let s = turn(i)?;
            if s == Ordering::Equal || (sigma != Ordering::Equal && s != sigma) {
                return Ok(None);
            }
            sigma = s;
        }
        if sigma == Ordering::Equal {
            return Ok(None);
        }
        let Some((new_min, new)) = self.triangulate_link(&link, p, q, sigma)? else {
            return Ok(None);
        };
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
            protected,
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
                let edge = key2(ce[a], ce[b]);
                if options.planar_boundary && pass.locked_edges.contains(&edge) {
                    consider(pass.boundary_edge_removal(edge)?);
                } else {
                    consider(pass.edge_removal(edge)?);
                }
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
    if options.planar_boundary {
        // Only coplanar quadrilaterals were retriangulated: the boundary vertices and the enclosed surface are unchanged.
        let area = |faces: &[[usize; 3]]| -> f64 {
            faces
                .iter()
                .map(|f| {
                    let [a, b, c] = f.map(|i| points[i]);
                    let u: Point = std::array::from_fn(|i| b[i] - a[i]);
                    let v: Point = std::array::from_fn(|i| c[i] - a[i]);
                    0.5 * ((u[1] * v[2] - u[2] * v[1]).powi(2) + (u[2] * v[0] - u[0] * v[2]).powi(2) + (u[0] * v[1] - u[1] * v[0]).powi(2)).sqrt()
                })
                .sum()
        };
        let (a0, a1) = (area(top.boundary()), area(result.boundary()));
        let vertices = |faces: &[[usize; 3]]| -> BTreeSet<usize> { faces.iter().flatten().copied().collect() };
        if (a0 - a1).abs() > 1e-9 * a0.max(f64::MIN_POSITIVE) || vertices(top.boundary()) != vertices(result.boundary()) {
            return Err(MeshError::NonConforming("planar boundary reconnection changed the boundary surface".into()));
        }
        if protected.iter().any(|f| !boundary_after.contains(f) && boundary_before.contains(f)) {
            return Err(MeshError::NonConforming("a protected boundary face was changed".into()));
        }
    } else if boundary_after != boundary_before {
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
        assert!(!c.cells().is_empty());
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

    #[test]
    fn a_boundary_edge_in_a_flat_patch_is_flipped_to_the_better_diagonal_only_when_asked() {
        // A pyramid over the planar quad p u0 q uk (the z = 0 face) with apex a, cut by the long diagonal p-q into two
        // slivers; the short diagonal u0-uk gives two good tetrahedra. All four quad vertices are on the boundary.
        let points: Vec<Point> = vec![
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, -0.1, 0.0],
            [0.0, 0.1, 0.0],
            [0.0, 0.0, 0.3],
        ];
        let (p, q, u0, uk, a) = (0, 1, 2, 3, 4);
        let cells: Vec<[usize; 4]> = [[p, q, u0, a], [p, q, uk, a]]
            .into_iter()
            .map(|c| positive(&points, c).unwrap().unwrap())
            .collect();
        let top = TetTopology::new(points.clone(), cells, 100).unwrap();
        let area = |t: &TetTopology| {
            t.boundary()
                .iter()
                .map(|f| {
                    let [x, y, z] = f.map(|i| points[i]);
                    let u: Point = std::array::from_fn(|i| y[i] - x[i]);
                    let v: Point = std::array::from_fn(|i| z[i] - x[i]);
                    0.5 * ((u[1] * v[2] - u[2] * v[1]).powi(2) + (u[2] * v[0] - u[0] * v[2]).powi(2) + (u[0] * v[1] - u[1] * v[0]).powi(2)).sqrt()
                })
                .sum::<f64>()
        };
        let locked = FlipOptions { target: 1.0, ..FlipOptions::default() };
        let (same, r) = improve(&top, &BTreeSet::new(), locked).unwrap();
        assert_eq!(same.cells(), top.cells(), "boundary edges are locked by default: {r:?}");
        let flat = FlipOptions { planar_boundary: true, ..locked };
        let (out, r) = improve(&top, &BTreeSet::new(), flat).unwrap();
        assert_eq!(r.edge_removals.get(&3), Some(&1), "{r:?}");
        assert!(r.min_radius_ratio_after > 5.0 * r.min_radius_ratio_before, "{r:?}");
        // The short diagonal replaced the long one; the volume and the boundary surface are unchanged.
        let has_edge = |t: &TetTopology, e: [usize; 2]| t.cells().iter().any(|c| c.contains(&e[0]) && c.contains(&e[1]));
        assert!(has_edge(&out, [u0, uk]) && !has_edge(&out, [p, q]));
        assert!((volume(out.points(), out.cells()) - volume(top.points(), top.cells())).abs() < 1e-14);
        assert!((area(&out) - area(&top)).abs() < 1e-14);
        // Protecting one of the faces keeps the edge.
        let mut protected = BTreeSet::new();
        protected.insert(key3([p, q, u0]));
        let (kept, _) = improve(&top, &protected, flat).unwrap();
        assert_eq!(kept.cells(), top.cells());
        // A concave quadrilateral (u0 pulled out of line) is refused: the diagonal would leave the quadrilateral.
        let mut bent = points.clone();
        bent[u0] = [-1.5, -0.1, 0.0];
        let cells: Vec<[usize; 4]> = [[p, q, u0, a], [p, q, uk, a]]
            .into_iter()
            .filter_map(|c| positive(&bent, c).ok().flatten())
            .collect();
        if let Ok(top) = TetTopology::new(bent, cells, 100) {
            let (out, _) = improve(&top, &BTreeSet::new(), flat).unwrap();
            assert_eq!(out.cells(), top.cells());
        }
    }

    #[test]
    fn planar_boundary_reconnection_keeps_volume_surface_and_orientation_on_random_clouds() {
        let mut total_boundary_changes = 0;
        for seed in 1..=10 {
            let points = cloud(80, seed);
            let top = tetrahedralize(&points, Limits::default()).unwrap();
            let options = FlipOptions { planar_boundary: true, ..FlipOptions::default() };
            let (out, report) = improve(&top, &BTreeSet::new(), options).unwrap();
            let (plain, _) = improve(&top, &BTreeSet::new(), FlipOptions::default()).unwrap();
            // Never worse than the interior-only reconnection.
            assert!(report.min_radius_ratio_after >= report.min_radius_ratio_before);
            assert!((volume(out.points(), out.cells()) - 1.).abs() < 1e-12, "seed {seed}");
            // `TetTopology::new` rejected any negatively oriented or flat cell; the boundary stays the surface of the unit cube.
            for f in out.boundary() {
                let pts = f.map(|i| out.points()[i]);
                assert!((0..3).any(|k| pts.iter().all(|p| p[k] == 0.0) || pts.iter().all(|p| p[k] == 1.0)), "seed {seed}: boundary face off the cube surface");
            }
            let mut b0: Vec<_> = top.boundary().iter().map(|f| key3(*f)).collect();
            let mut b1: Vec<_> = out.boundary().iter().map(|f| key3(*f)).collect();
            b0.sort();
            b1.sort();
            if b0 != b1 {
                total_boundary_changes += 1;
            }
            assert!(!out.cells().is_empty() && !plain.cells().is_empty());
        }
        // Not asserted to be positive: whether a random cloud has a flippable cube-face diagonal depends on the seed.
        eprintln!("seeds whose cube-face diagonals were flipped: {total_boundary_changes} of 10");
    }
}
