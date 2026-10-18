//! Incremental 3D Delaunay point insertion on an adjacency-linked tetrahedral
//! mesh: visibility-walk point location, connected conflict-cavity search and
//! cone re-triangulation, with the exact predicates of `fea-core`'s reference
//! mesher. Points must lie inside or on the convex hull of the seed mesh, or just
//! outside it (rounding of a Steiner point on a hull plane): such a point replaces
//! the hull faces of its conflict cavity that it sees, which can leave the hull
//! slightly non-convex. A point whose walk exits the hull outside every
//! conflicting tetrahedron is refused. `plc_conform` therefore surrounds its input with far points so that no insertion
//! is ever outside the hull.
use fea_core::meshing::{
    tet_topology::{orientation, sphere_side, Point, SphereSide, TetTopology},
    MeshError,
};
use std::cmp::Ordering;
use std::collections::BTreeMap;
const NONE: usize = usize::MAX;
#[derive(Debug, Clone)]
struct Tet {
    /// Positively oriented (`orientation(v0, v1, v2, v3) == Greater`).
    v: [usize; 4],
    /// `n[k]` is the tetrahedron across the face opposite `v[k]`.
    n: [usize; 4],
    alive: bool,
}
fn face(v: [usize; 4], k: usize) -> [usize; 3] {
    let [a, b, c, d] = v;
    [[b, c, d], [a, d, c], [a, b, d], [a, c, b]][k]
}
pub(crate) struct Delaunay3 {
    points: Vec<Point>,
    tets: Vec<Tet>,
    free: Vec<usize>,
    live: usize,
    last: usize,
    max_cells: usize,
    marks: Vec<u32>,
    epoch: u32,
    removed: Vec<[usize; 4]>,
}
impl Delaunay3 {
    pub fn new(seed: &TetTopology, max_cells: usize) -> Self {
        let mut tets: Vec<Tet> = seed
            .cells()
            .iter()
            .zip(seed.neighbors())
            .map(|(v, n)| Tet {
                v: *v,
                n: n.map(|x| x.unwrap_or(NONE)),
                alive: true,
            })
            .collect();
        tets.shrink_to_fit();
        Self {
            points: seed.points().to_vec(),
            live: tets.len(),
            marks: vec![0; tets.len()],
            tets,
            free: Vec::new(),
            last: 0,
            max_cells,
            epoch: 0,
            removed: Vec::new(),
        }
    }
    /// Vertex sets of the tetrahedra destroyed by the most recent insertion.
    pub fn last_removed(&self) -> &[[usize; 4]] {
        &self.removed
    }
    /// Dense copy of the live tetrahedra with their face neighbours (same face
    /// order as `TetTopology::neighbors`).
    pub fn snapshot(&self) -> (Vec<[usize; 4]>, Vec<[Option<usize>; 4]>) {
        let mut dense = vec![NONE; self.tets.len()];
        let mut cells = Vec::with_capacity(self.live);
        for (i, t) in self.tets.iter().enumerate() {
            if t.alive {
                dense[i] = cells.len();
                cells.push(t.v);
            }
        }
        let neighbors = self
            .tets
            .iter()
            .filter(|t| t.alive)
            .map(|t| t.n.map(|n| (n != NONE).then(|| dense[n])))
            .collect();
        (cells, neighbors)
    }
    pub fn points(&self) -> &[Point] {
        &self.points
    }
    pub fn cells(&self) -> impl Iterator<Item = [usize; 4]> + '_ {
        self.tets.iter().filter(|t| t.alive).map(|t| t.v)
    }
    pub fn into_topology(self) -> Result<TetTopology, MeshError> {
        let cells: Vec<_> = self.cells().collect();
        TetTopology::new(self.points, cells, self.max_cells)
    }
    /// Returns the located tetrahedron and whether `p` is outside the hull (then
    /// the tetrahedron is the one whose hull face the walk left through).
    fn locate(&self, p: Point) -> Result<(usize, bool), MeshError> {
        let mut c = if self.tets[self.last].alive {
            self.last
        } else {
            self.tets
                .iter()
                .position(|t| t.alive)
                .ok_or_else(|| MeshError::Failed("empty tetrahedral mesh".into()))?
        };
        let budget = self.tets.len() * 4 + 64;
        for step in 0..budget {
            let t = &self.tets[c];
            let mut next = None;
            for j in 0..4 {
                // Rotate the starting face so a degenerate walk cannot loop forever.
                let k = (j + step) % 4;
                let f = face(t.v, k).map(|v| self.points[v]);
                if orientation(f[0], f[1], f[2], p)? == Ordering::Greater {
                    next = Some(t.n[k]);
                    break;
                }
            }
            match next {
                None => return Ok((c, false)),
                Some(NONE) => return Ok((c, true)),
                Some(n) => c = n,
            }
        }
        Err(MeshError::Failed("point location did not terminate".into()))
    }
    /// Insert `p`, returning its index. A failed insertion leaves the mesh unchanged.
    pub fn insert(&mut self, p: Point) -> Result<usize, MeshError> {
        match self.try_insert(p, false) {
            Err(MeshError::NonConforming(_)) => self.try_insert(p, true),
            other => other,
        }
    }
    fn try_insert(&mut self, p: Point, strict: bool) -> Result<usize, MeshError> {
        let (start, outside) = self.locate(p)?;
        if outside
            && sphere_side(self.tets[start].v.map(|v| self.points[v]), p)? == SphereSide::Outside
        {
            return Err(MeshError::Unsupported(
                "point lies outside the hull and outside every conflicting tetrahedron".into(),
            ));
        }
        self.epoch += 1;
        if self.marks.len() < self.tets.len() {
            self.marks.resize(self.tets.len(), 0);
        }
        let epoch = self.epoch;
        let mut cavity = vec![start];
        self.marks[start] = epoch;
        let mut i = 0;
        while i < cavity.len() {
            let c = cavity[i];
            i += 1;
            for k in 0..4 {
                let nb = self.tets[c].n[k];
                if nb == NONE || self.marks[nb] == epoch {
                    continue;
                }
                let side = sphere_side(self.tets[nb].v.map(|v| self.points[v]), p)?;
                let joins = if strict {
                    side == SphereSide::Inside
                } else {
                    side != SphereSide::Outside
                };
                if joins {
                    self.marks[nb] = epoch;
                    cavity.push(nb);
                }
            }
        }
        // Boundary faces of the cavity, each coned to `p`.
        let index = self.points.len();
        let mut cones: Vec<([usize; 4], usize)> = Vec::new();
        for &c in &cavity {
            for k in 0..4 {
                let nb = self.tets[c].n[k];
                if nb != NONE && self.marks[nb] == epoch {
                    continue;
                }
                let f = face(self.tets[c].v, k);
                match orientation(self.points[f[0]], self.points[f[1]], self.points[f[2]], p)? {
                    // The interior side of an outward face is the Less side.
                    Ordering::Less => cones.push(([f[0], f[2], f[1], index], nb)),
                    // A hull face the point lies on or sees is dropped: the hull moves.
                    Ordering::Equal | Ordering::Greater if nb == NONE => {}
                    _ => {
                        return Err(MeshError::NonConforming(
                            "insertion cavity is not star-shaped".into(),
                        ))
                    }
                }
            }
        }
        if self.live - cavity.len() + cones.len() > self.max_cells {
            return Err(MeshError::LimitExceeded(
                "insertion cell budget exceeded".into(),
            ));
        }
        // Commit: allocate the cone tetrahedra, link them, retire the cavity.
        let removed_vertices: BTreeMap<usize, [usize; 4]> =
            cavity.iter().map(|&c| (c, self.tets[c].v)).collect();
        let mut created = Vec::with_capacity(cones.len());
        let mut edges: BTreeMap<[usize; 2], (usize, usize)> = BTreeMap::new();
        let mut dead = cavity.clone();
        for (v, outer) in cones {
            let id = match self.free.pop().or_else(|| dead.pop()) {
                Some(id) => id,
                None => {
                    self.tets.push(Tet {
                        v,
                        n: [NONE; 4],
                        alive: false,
                    });
                    self.marks.push(0);
                    self.tets.len() - 1
                }
            };
            self.tets[id] = Tet {
                v,
                n: [NONE, NONE, NONE, outer],
                alive: true,
            };
            created.push(id);
            if outer != NONE {
                let back = face(v, 3);
                let j = (0..4)
                    .find(|&j| {
                        let mut a = face(self.tets[outer].v, j);
                        let mut b = back;
                        a.sort_unstable();
                        b.sort_unstable();
                        a == b
                    })
                    .ok_or_else(|| MeshError::Failed("lost cavity boundary face".into()))?;
                self.tets[outer].n[j] = id;
            }
            for k in 0..3 {
                let mut e: Vec<usize> = (0..3).filter(|&m| m != k).map(|m| v[m]).collect();
                e.sort_unstable();
                let key = [e[0], e[1]];
                match edges.remove(&key) {
                    Some((other, kk)) => {
                        self.tets[id].n[k] = other;
                        self.tets[other].n[kk] = id;
                    }
                    None => {
                        edges.insert(key, (id, k));
                    }
                }
            }
        }
        self.removed = cavity.iter().map(|&c| removed_vertices[&c]).collect();
        for c in dead {
            self.tets[c].alive = false;
            self.free.push(c);
        }
        self.live = self.live - cavity.len() + created.len();
        self.points.push(p);
        self.last = created.first().copied().unwrap_or(self.last);
        Ok(index)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use fea_core::meshing::tet_delaunay::{tetrahedralize, Limits};
    fn cube() -> Vec<Point> {
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        p
    }
    fn volume(points: &[Point], cells: impl Iterator<Item = [usize; 4]>) -> f64 {
        cells
            .map(|c| {
                let [a, b, cc, d] = c.map(|i| points[i]);
                let u: Point = std::array::from_fn(|i| b[i] - a[i]);
                let v: Point = std::array::from_fn(|i| cc[i] - a[i]);
                let w: Point = std::array::from_fn(|i| d[i] - a[i]);
                (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
                    + u[2] * (v[0] * w[1] - v[1] * w[0]))
                    / 6.
            })
            .sum()
    }
    #[test]
    fn incremental_points_match_the_reference_mesher_and_stay_delaunay() {
        let mut points = cube();
        let seed = tetrahedralize(&points, Limits::default()).unwrap();
        let mut d = Delaunay3::new(&seed, 10_000);
        // interior, on a face, on an edge, and generic points
        let extra = [
            [0.5, 0.5, 0.5],
            [0.31, 0.27, 0.64],
            [0.5, 0.5, 0.],
            [0.5, 0., 0.],
            [0.12, 0.83, 0.41],
            [0.77, 0.19, 0.93],
        ];
        for q in extra {
            let i = d.insert(q).unwrap();
            assert_eq!(i, points.len());
            points.push(q);
            let v = volume(d.points(), d.cells());
            assert!((v - 1.).abs() < 1e-13, "volume {v} after inserting {q:?}");
        }
        for t in d.cells() {
            for (i, &q) in points.iter().enumerate() {
                if !t.contains(&i) {
                    assert_ne!(
                        sphere_side(t.map(|j| points[j]), q).unwrap(),
                        SphereSide::Inside
                    );
                }
            }
        }
        let topology = d.into_topology().unwrap();
        let reference = tetrahedralize(&points, Limits::default()).unwrap();
        assert!((volume(topology.points(), topology.cells().iter().copied()) - 1.).abs() < 1e-13);
        assert_eq!(topology.boundary().len(), reference.boundary().len());
    }
    #[test]
    fn a_point_just_outside_a_hull_plane_is_absorbed_and_a_far_one_refused() {
        let points = cube();
        let seed = tetrahedralize(&points, Limits::default()).unwrap();
        let mut d = Delaunay3::new(&seed, 10_000);
        // One ulp above the z = 1 face, over the face interior.
        let above = [0.3, 0.4, f64::from_bits(1.0f64.to_bits() + 1)];
        d.insert(above).unwrap();
        assert!((volume(d.points(), d.cells()) - 1.).abs() < 1e-12);
        assert!(d.insert([0.5, 0.5, 40.]).is_err());
        // The failed insertion left the mesh usable.
        d.insert([0.6, 0.6, 0.6]).unwrap();
    }
    #[test]
    fn the_cell_budget_is_a_hard_limit_and_a_failed_insertion_changes_nothing() {
        let seed = tetrahedralize(&cube(), Limits::default()).unwrap();
        let mut d = Delaunay3::new(&seed, seed.cells().len() + 1);
        let before: Vec<_> = d.cells().collect();
        assert!(matches!(
            d.insert([0.5, 0.5, 0.5]),
            Err(MeshError::LimitExceeded(_))
        ));
        assert_eq!(before, d.cells().collect::<Vec<_>>());
        assert_eq!(d.points().len(), 8);
    }
}
