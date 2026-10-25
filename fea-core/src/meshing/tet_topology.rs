//! Exact-sign 3D predicates and checked tetrahedral connectivity for future cavity operations.
//! This is not a PLC mesher: geometric intersections and boundary recovery need separate checks.
use super::MeshError;
use robust::{insphere, orient3d, Coord3D};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
pub type Point = [f64; 3];

fn coord(p: Point) -> Coord3D<f64> {
    Coord3D {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}
fn checked(points: &[Point]) -> Result<(), MeshError> {
    // Conservative arithmetic envelope for degree-five adaptive expansions. Do not
    // rescale/round the inputs or replace an exact zero with an arbitrary epsilon.
    if points
        .iter()
        .flatten()
        .any(|x| !x.is_finite() || (*x != 0. && !(1e-30..=1e30).contains(&x.abs())))
    {
        return Err(MeshError::Unsupported(
            "3D predicate coordinates must be zero or have magnitude in [1e-30, 1e30]".into(),
        ));
    }
    Ok(())
}
/// Sign of det(b-a,c-a,d-a), with exact zero for coplanarity.
pub fn orientation(a: Point, b: Point, c: Point, d: Point) -> Result<Ordering, MeshError> {
    checked(&[a, b, c, d])?;
    (-orient3d(coord(a), coord(b), coord(c), coord(d)))
        .partial_cmp(&0.)
        .ok_or_else(|| MeshError::Failed("non-finite orientation predicate".into()))
}
/// Exact sign of the planar orientation of `a, b, c` (Greater = counter-clockwise).
pub fn orientation_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Ordering {
    let p = |q: [f64; 2]| robust::Coord { x: q[0], y: q[1] };
    robust::orient2d(p(a), p(b), p(c))
        .partial_cmp(&0.)
        .unwrap_or(Ordering::Equal)
}
/// Exact circle test for a counter-clockwise triangle `a, b, c`: Greater when `d`
/// lies strictly inside its circumcircle, Equal on it.
pub fn in_circle_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Ordering {
    let p = |q: [f64; 2]| robust::Coord { x: q[0], y: q[1] };
    robust::incircle(p(a), p(b), p(c), p(d))
        .partial_cmp(&0.)
        .unwrap_or(Ordering::Equal)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SphereSide {
    Inside,
    On,
    Outside,
}
/// Orientation-independent circumsphere classification; a flat defining tetrahedron is refused.
pub fn sphere_side(t: [Point; 4], p: Point) -> Result<SphereSide, MeshError> {
    checked(&[t[0], t[1], t[2], t[3], p])?;
    let o = orientation(t[0], t[1], t[2], t[3])?;
    if o == Ordering::Equal {
        return Err(MeshError::Failed(
            "a flat tetrahedron has no circumsphere".into(),
        ));
    }
    let s = insphere(coord(t[0]), coord(t[1]), coord(t[2]), coord(t[3]), coord(p));
    if !s.is_finite() {
        return Err(MeshError::Failed("non-finite sphere predicate".into()));
    }
    Ok(if s == 0. {
        SphereSide::On
    } else if (s > 0.) == (o == Ordering::Less) {
        SphereSide::Inside
    } else {
        SphereSide::Outside
    })
}
fn faces(t: [usize; 4]) -> [[usize; 3]; 4] {
    let [a, b, c, d] = t;
    [[b, c, d], [a, d, c], [a, b, d], [a, c, b]]
}
fn key(mut f: [usize; 3]) -> [usize; 3] {
    f.sort_unstable();
    f
}
fn parity(f: [usize; 3]) -> bool {
    ((f[0] > f[1]) as u8 + (f[0] > f[2]) as u8 + (f[1] > f[2]) as u8).is_multiple_of(2)
}

/// Validated local topology. Coordinates/connectivity are immutable outside checked transactions.
/// Disjoint tetrahedron intersections and hanging nodes are not certified by this local structure.
#[derive(Debug, Clone, PartialEq)]
pub struct TetTopology {
    points: Vec<Point>,
    cells: Vec<[usize; 4]>,
    neighbors: Vec<[Option<usize>; 4]>,
    boundary: Vec<[usize; 3]>,
    limit: usize,
}
impl TetTopology {
    pub fn new(
        points: Vec<Point>,
        mut cells: Vec<[usize; 4]>,
        limit: usize,
    ) -> Result<Self, MeshError> {
        if cells.len() > limit {
            return Err(MeshError::LimitExceeded(
                "tetrahedron budget exceeded".into(),
            ));
        }
        checked(&points)?;
        let mut vertices = BTreeSet::new();
        for p in &points {
            let k = p.map(|x| if x == 0. { 0 } else { x.to_bits() });
            if !vertices.insert(k) {
                return Err(MeshError::NonConforming(
                    "duplicate point coordinates".into(),
                ));
            }
        }
        let mut unique = BTreeSet::new();
        let mut neighbors = vec![[None; 4]; cells.len()];
        let mut owners = BTreeMap::<[usize; 3], Vec<(usize, usize, [usize; 3])>>::new();
        for (i, t) in cells.iter_mut().enumerate() {
            if t.iter().any(|&v| v >= points.len()) {
                return Err(MeshError::NonConforming(
                    "tetrahedron node index out of range".into(),
                ));
            }
            let mut k = *t;
            k.sort_unstable();
            if k.windows(2).any(|w| w[0] == w[1]) || !unique.insert(k) {
                return Err(MeshError::NonConforming(
                    "repeated node or duplicate tetrahedron".into(),
                ));
            }
            match orientation(points[t[0]], points[t[1]], points[t[2]], points[t[3]])? {
                Ordering::Equal => {
                    return Err(MeshError::InvalidElements {
                        count: 1,
                        first: (0, i),
                    })
                }
                Ordering::Less => t.swap(0, 1),
                Ordering::Greater => {}
            }
            for (local, f) in faces(*t).into_iter().enumerate() {
                owners.entry(key(f)).or_default().push((i, local, f));
            }
        }
        let mut boundary = Vec::new();
        for entries in owners.values() {
            match entries.as_slice() {
                [(_, _, f)] => boundary.push(*f),
                [(a, fa, x), (b, fb, y)] if parity(*x) != parity(*y) => {
                    neighbors[*a][*fa] = Some(*b);
                    neighbors[*b][*fb] = Some(*a);
                }
                _ => {
                    return Err(MeshError::NonConforming(
                        "non-manifold or equally oriented shared tetrahedron face".into(),
                    ))
                }
            }
        }
        Ok(Self {
            points,
            cells,
            neighbors,
            boundary,
            limit,
        })
    }
    /// Convert the research topology to a Tet4 solver mesh only after the existing
    /// quality and conformity gates pass. Boundary tags here are generic, not CAD IDs.
    pub fn to_mesh(&self, material: crate::Elastic) -> Result<crate::Mesh, MeshError> {
        let used: BTreeSet<_> = self.cells.iter().flatten().copied().collect();
        if self.cells.is_empty() || used.len() != self.points.len() {
            return Err(MeshError::NonConforming(
                "solver mesh has empty cells or unused points".into(),
            ));
        }
        let mut mesh = crate::Mesh::new(crate::Physics::Solid);
        mesh.nodes = self.points.clone();
        mesh.add_block(
            crate::ElementKind::Tet4,
            self.cells.iter().flatten().copied().collect(),
            material,
            "reference tetrahedra",
        )
        .map_err(MeshError::Failed)?;
        mesh.surfaces.insert(
            "boundary".into(),
            self.boundary.iter().map(|f| f.to_vec()).collect(),
        );
        mesh.quality()
            .require(&super::quality::QualityThresholds::default())?;
        let adjacency = super::adjacency::Adjacency::new(&mesh);
        let conformity = super::adjacency::check_conformity(&mesh, &adjacency);
        conformity.require_conforming()?;
        Ok(mesh)
    }
    pub fn points(&self) -> &[Point] {
        &self.points
    }
    pub fn cells(&self) -> &[[usize; 4]] {
        &self.cells
    }
    pub fn neighbors(&self) -> &[[Option<usize>; 4]] {
        &self.neighbors
    }
    pub fn boundary(&self) -> &[[usize; 3]] {
        &self.boundary
    }
    /// Replace an interior cavity atomically, preserving its external oriented boundary.
    /// Full rebuilding favors verification over performance in this foundation.
    pub fn replace_cells(
        &mut self,
        remove: &[usize],
        insert: &[[usize; 4]],
    ) -> Result<(), MeshError> {
        let removed: BTreeSet<_> = remove.iter().copied().collect();
        if removed.len() != remove.len() || removed.iter().any(|&i| i >= self.cells.len()) {
            return Err(MeshError::NonConforming(
                "invalid cavity cell selection".into(),
            ));
        }
        let count = self.cells.len() - removed.len();
        if insert.len() > self.limit.saturating_sub(count) {
            return Err(MeshError::LimitExceeded(
                "cavity replacement exceeds tetrahedron budget".into(),
            ));
        }
        let mut cells: Vec<_> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(i, _)| !removed.contains(i))
            .map(|(_, t)| *t)
            .collect();
        cells.extend_from_slice(insert);
        let candidate = Self::new(self.points.clone(), cells, self.limit)?;
        let signature = |s: &Self| -> BTreeMap<_, _> {
            s.boundary.iter().map(|&f| (key(f), parity(f))).collect()
        };
        if signature(self) != signature(&candidate) {
            return Err(MeshError::NonConforming(
                "cavity replacement changes the external boundary".into(),
            ));
        }
        *self = candidate;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn points() -> Vec<Point> {
        vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [0.2, 0.2, 0.2],
            [0., 0., -1.],
        ]
    }
    #[test]
    fn predicates_resolve_degenerate_near_degenerate_and_reversed_cases() {
        let p = points();
        assert_eq!(
            orientation(p[0], p[1], p[2], [0., 0., 1e-20]).unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            orientation(p[0], p[1], p[2], [0.2, 0.2, 0.]).unwrap(),
            Ordering::Equal
        );
        for t in [[p[0], p[1], p[2], p[3]], [p[1], p[0], p[2], p[3]]] {
            assert_eq!(sphere_side(t, p[4]).unwrap(), SphereSide::Inside);
            assert_eq!(sphere_side(t, [2., 2., 2.]).unwrap(), SphereSide::Outside);
            assert_eq!(sphere_side(t, p[1]).unwrap(), SphereSide::On);
        }
        assert!(orientation(p[0], p[1], p[2], [f64::INFINITY, 0., 0.]).is_err());
        assert!(orientation(p[0], p[1], p[2], [1e100, 0., 1.]).is_err());
    }
    #[test]
    fn reciprocal_neighbors_and_atomic_cavity_refinement() {
        let mut t = TetTopology::new(points(), vec![[0, 1, 2, 3], [0, 2, 1, 5]], 10).unwrap();
        assert_eq!(t.boundary().len(), 6);
        assert_eq!(
            t.neighbors()[0].iter().filter(|n| **n == Some(1)).count(),
            1
        );
        let old = t.clone();
        assert!(t.replace_cells(&[0], &[[0, 1, 2, 4]]).is_err());
        assert_eq!(t, old);
        t.replace_cells(
            &[0],
            &[[4, 1, 2, 3], [0, 4, 2, 3], [0, 1, 4, 3], [0, 1, 2, 4]],
        )
        .unwrap();
        assert_eq!(t.cells().len(), 5);
        assert_eq!(t.boundary().len(), 6);
        let old = t.clone();
        assert!(t.replace_cells(&[1, 1], &[]).is_err());
        assert_eq!(t, old);
    }
    #[test]
    fn checked_two_to_three_flip_can_clear_quality_gate_without_moving_boundary() {
        let p = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0],
            [0.5, 0.8660254, 0.0],
            [0.4676201654449199, 0.3117365514855523, 0.41338745434207647],
            [0.44135538017430254, 0.3773988350832113, -0.040774254313441186],
        ];
        let mut top = TetTopology::new(p.clone(), vec![[0, 1, 2, 3], [0, 2, 1, 4]], 10).unwrap();
        let signature = |faces: &[[usize; 3]]| -> BTreeMap<_, _> {
            faces.iter().map(|&face| (key(face), parity(face))).collect()
        };
        let before = signature(top.boundary());
        let q = |cells: &[[usize; 4]]| cells.iter().map(|&c| super::super::boundary_repair::scaled_jacobian(&p, c)).fold(f64::INFINITY, f64::min);
        assert!(q(top.cells()) < super::super::quality::QualityThresholds::default().min_scaled_jacobian);
        top.replace_cells(&[0, 1], &[[3, 0, 4, 1], [3, 4, 1, 2], [3, 2, 4, 0]]).unwrap();
        assert_eq!(signature(top.boundary()), before);
        assert!(q(top.cells()) > 0.3);
    }
    #[test]
    fn rejects_bad_indices_duplicates_flat_and_same_side_shared_faces() {
        for cells in [
            vec![[0, 1, 2, 99]],
            vec![[0, 1, 2, 2]],
            vec![[0, 1, 2, 3], [3, 2, 1, 0]],
            vec![[0, 1, 2, 3], [0, 1, 2, 4]],
        ] {
            assert!(TetTopology::new(points(), cells, 10).is_err());
        }
        assert!(TetTopology::new(points(), vec![[0, 1, 2, 3]], 0).is_err());
        let mut p = points();
        p.push(p[0]);
        assert!(TetTopology::new(p, vec![], 10).is_err());
    }
}
