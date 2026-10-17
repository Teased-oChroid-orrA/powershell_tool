//! Bounded reference Bowyer–Watson tetrahedralization of a point set's convex hull.
//! **Not** constrained CAD/PLC meshing. No application strategy advertises this core.
//! Full topology rebuilding is deliberate for this verification-first implementation.
use super::{
    tet_topology::{orientation, sphere_side, Point, SphereSide, TetTopology},
    MeshError,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_points: usize,
    pub max_cells: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_points: 4096,
            max_cells: 100_000,
        }
    }
}
fn faces([a, b, c, d]: [usize; 4]) -> [[usize; 3]; 4] {
    [[b, c, d], [a, d, c], [a, b, d], [a, c, b]]
}
/// Input order is the deterministic tie policy: cospherical cells join the cavity.
/// Duplicates, unsupported numeric ranges, insufficient dimension and exhausted
/// budgets are explicit errors, never silently omitted engineering features.
pub fn tetrahedralize(input: &[Point], limits: Limits) -> Result<TetTopology, MeshError> {
    tetrahedralize_observed(input, limits, &|_, _| Ok(()))
}
/// Observe insertion progress and cancel safely before the next topology transaction.
pub fn tetrahedralize_observed(
    input: &[Point],
    limits: Limits,
    observe: &dyn Fn(usize, usize) -> Result<(), MeshError>,
) -> Result<TetTopology, MeshError> {
    if input.len() < 4 {
        return Err(MeshError::Unsupported(
            "at least four 3D points are required".into(),
        ));
    }
    if input.len() > limits.max_points {
        return Err(MeshError::LimitExceeded("point budget exceeded".into()));
    }
    // Start from real non-coplanar points. Finite supervertices can steal hull
    // tetrahedra even when the enclosure is large; handle the infinite-cell
    // conflict cavity through visible boundary faces instead.
    TetTopology::new(input.to_vec(), vec![], limits.max_cells)?;
    let noncollinear = |p: Point| {
        [(0, 1), (0, 2), (1, 2)].iter().any(|&(x, y)| {
            let c = |q: Point| robust::Coord { x: q[x], y: q[y] };
            robust::orient2d(c(input[0]), c(input[1]), c(p)) != 0.
        })
    };
    let c = (2..input.len())
        .find(|&i| noncollinear(input[i]))
        .ok_or_else(|| MeshError::Unsupported("point set is collinear".into()))?;
    let mut fourth = None;
    for i in 2..input.len() {
        if orientation(input[0], input[1], input[c], input[i])? != Ordering::Equal {
            fourth = Some(i);
            break;
        }
    }
    let d = fourth.ok_or_else(|| MeshError::Unsupported("point set is coplanar".into()))?;
    let seed = [0, 1, c, d];
    let mut topology = TetTopology::new(input.to_vec(), vec![seed], limits.max_cells)?;
    for index in 0..input.len() {
        observe(index, input.len())?;
        if seed.contains(&index) {
            continue;
        }
        topology = insert_index(input, &topology, index, limits.max_cells)?;
    }
    let used: BTreeSet<_> = topology.cells().iter().flatten().copied().collect();
    if used.len() != input.len() {
        return Err(MeshError::Failed(
            "not every input point belongs to a tetrahedron".into(),
        ));
    }
    check_hull(&topology)?;
    observe(input.len(), input.len())?;
    Ok(topology)
}
/// A topologically closed boundary alone does not establish the convex hull.
fn check_hull(topology: &TetTopology) -> Result<(), MeshError> {
    let input = topology.points();
    for face in topology.boundary() {
        for &p in input {
            if orientation(input[face[0]], input[face[1]], input[face[2]], p)? == Ordering::Greater
            {
                return Err(MeshError::NonConforming(
                    "final boundary is not the point-set convex hull".into(),
                ));
            }
        }
    }
    Ok(())
}
/// Insert `points[index]` into `topology` (whose point list is `points`) by
/// Bowyer–Watson cavity replacement. Points outside the current hull grow it
/// through the visible boundary faces.
fn insert_index(
    points: &[Point],
    topology: &TetTopology,
    index: usize,
    max_cells: usize,
) -> Result<TetTopology, MeshError> {
    let point = points[index];
    let mut cavity = BTreeSet::new();
    for (i, t) in topology.cells().iter().enumerate() {
        if sphere_side(t.map(|v| points[v]), point)? != SphereSide::Outside {
            cavity.insert(i);
        }
    }
    let mut owners = BTreeMap::<[usize; 3], Vec<(usize, [usize; 3])>>::new();
    for (i, t) in topology.cells().iter().enumerate() {
        for f in faces(*t) {
            let mut k = f;
            k.sort_unstable();
            owners.entry(k).or_default().push((i, f));
        }
    }
    let mut cells: Vec<_> = topology
        .cells()
        .iter()
        .enumerate()
        .filter(|(i, _)| !cavity.contains(i))
        .map(|(_, t)| *t)
        .collect();
    for entries in owners.values() {
        let (owner, face) = entries[0];
        let cone = match entries.as_slice() {
            [(_, _)] => {
                let visible =
                    orientation(points[face[0]], points[face[1]], points[face[2]], point)?
                        == Ordering::Greater;
                cavity.contains(&owner) != visible
            }
            [(a, _), (b, _)] => cavity.contains(a) != cavity.contains(b),
            _ => {
                return Err(MeshError::NonConforming(
                    "non-manifold insertion cavity".into(),
                ))
            }
        };
        if cone {
            let [a, b, c] = face;
            if orientation(points[a], points[b], points[c], point)? == Ordering::Equal {
                continue;
            }
            if cells.len() >= max_cells {
                return Err(MeshError::LimitExceeded(
                    "insertion cell budget exceeded".into(),
                ));
            }
            cells.push([a, b, c, index]);
        }
    }
    let next = TetTopology::new(points.to_vec(), cells, max_cells)?;
    if !next.cells().iter().any(|t| t.contains(&index)) {
        return Err(MeshError::Failed(
            "insertion did not retain its point".into(),
        ));
    }
    Ok(next)
}
/// Incrementally maintained Delaunay tetrahedralization of a growing point set,
/// for Steiner-point recovery. Same predicates and tie policy as
/// [`tetrahedralize`]; each insertion is one checked topology transaction, so a
/// failure leaves the previous triangulation intact.
#[derive(Debug, Clone)]
pub struct Delaunay {
    topology: TetTopology,
    limits: Limits,
}
impl Delaunay {
    pub fn new(input: &[Point], limits: Limits) -> Result<Self, MeshError> {
        Ok(Self {
            topology: tetrahedralize(input, limits)?,
            limits,
        })
    }
    pub fn topology(&self) -> &TetTopology {
        &self.topology
    }
    /// Insert a point, returning its index (always the previous point count).
    pub fn insert(&mut self, point: Point) -> Result<usize, MeshError> {
        let mut points = self.topology.points().to_vec();
        if points.len() >= self.limits.max_points {
            return Err(MeshError::LimitExceeded("point budget exceeded".into()));
        }
        let index = points.len();
        points.push(point);
        let next = insert_index(&points, &self.topology, index, self.limits.max_cells)?;
        check_hull(&next)?;
        self.topology = next;
        Ok(index)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn volume(t: &TetTopology) -> f64 {
        t.cells()
            .iter()
            .map(|c| {
                let a = t.points()[c[0]];
                let b = t.points()[c[1]];
                let c1 = t.points()[c[2]];
                let d = t.points()[c[3]];
                let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
                let v = std::array::from_fn::<_, 3, _>(|i| c1[i] - a[i]);
                let w = std::array::from_fn::<_, 3, _>(|i| d[i] - a[i]);
                (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
                    + u[2] * (v[0] * w[1] - v[1] * w[0]))
                    / 6.
            })
            .sum()
    }
    #[test]
    fn cube_with_interior_point_has_analytic_volume_and_empty_circumspheres() {
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        p.push([0.37, 0.41, 0.53]);
        let a = tetrahedralize(&p, Limits::default()).unwrap();
        assert!((volume(&a) - 1.).abs() < 1e-14);
        for t in a.cells() {
            for (i, &q) in p.iter().enumerate() {
                if !t.contains(&i) {
                    assert_ne!(sphere_side(t.map(|j| p[j]), q).unwrap(), SphereSide::Inside);
                }
            }
        }
        assert_eq!(a, tetrahedralize(&p, Limits::default()).unwrap());
        p.reverse();
        assert!((volume(&tetrahedralize(&p, Limits::default()).unwrap()) - 1.).abs() < 1e-14);
    }
    #[test]
    fn reference_mesh_passes_a_closed_form_tension_patch_and_rejects_slivers() {
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        p.push([0.37, 0.41, 0.53]);
        let mesh = tetrahedralize(&p, Limits::default())
            .unwrap()
            .to_mesh(crate::Elastic::new(1000., 0.))
            .unwrap();
        let model = crate::Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for (i, p) in model.mesh.nodes.iter().enumerate() {
            if p[2] == 0. {
                bc.fix_node(i);
            }
        }
        let mut loads = crate::Loads::default();
        for face in &model.mesh.surfaces["boundary"] {
            if face.iter().all(|&i| model.mesh.nodes[i][2] == 1.) {
                loads
                    .faces
                    .push((face.clone(), crate::SurfaceLoad::Traction([0., 0., 10.])));
            }
        }
        let solved = model.solve_static(&loads, &bc).unwrap();
        for (i, p) in model.mesh.nodes.iter().enumerate() {
            assert!((solved.u[3 * i + 2] - 0.01 * p[2]).abs() < 1e-12);
        }
        let sliver = TetTopology::new(
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0.2, 0.2, 1e-12]],
            vec![[0, 1, 2, 3]],
            10,
        )
        .unwrap();
        assert!(sliver.to_mesh(crate::Elastic::new(1000., 0.3)).is_err());
    }

    #[test]
    fn dense_interior_cloud_keeps_the_complete_analytic_hull() {
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        let mut seed = 731u64;
        while p.len() < 100 {
            p.push(std::array::from_fn(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                0.01 + 0.98 * ((seed >> 32) as f64 / u32::MAX as f64)
            }));
        }
        let t = tetrahedralize(&p, Limits::default()).unwrap();
        assert!((volume(&t) - 1.).abs() < 1e-12, "volume {}", volume(&t));
        for cell in t.cells() {
            for (i, &point) in p.iter().enumerate() {
                if !cell.contains(&i) {
                    assert_ne!(
                        sphere_side(cell.map(|j| p[j]), point).unwrap(),
                        SphereSide::Inside
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "explicit reference-core scaling measurement"]
    fn reference_delaunay_scaling() {
        for count in [20, 100, 256] {
            let mut p = Vec::new();
            for x in [0., 1.] {
                for y in [0., 1.] {
                    for z in [0., 1.] {
                        p.push([x, y, z]);
                    }
                }
            }
            let mut seed = 731u64;
            while p.len() < count {
                let q = std::array::from_fn(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    0.01 + 0.98 * ((seed >> 32) as f64 / u32::MAX as f64)
                });
                p.push(q);
            }
            let start = std::time::Instant::now();
            let t = tetrahedralize(&p, Limits::default()).unwrap();
            let seconds = start.elapsed().as_secs_f64();
            assert!((volume(&t) - 1.).abs() < 1e-12);
            eprintln!(
                "reference_delaunay points={} cells={} seconds={seconds:.6}",
                p.len(),
                t.cells().len()
            );
        }
    }

    #[test]
    fn progress_can_cancel_without_publishing_a_partial_mesh() {
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let calls = std::cell::Cell::new(0);
        let result = tetrahedralize_observed(&p, Limits::default(), &|done, total| {
            assert_eq!(total, 4);
            calls.set(calls.get() + 1);
            if done == 2 {
                Err(MeshError::Failed("cancelled".into()))
            } else {
                Ok(())
            }
        });
        assert!(matches!(result,Err(MeshError::Failed(e)) if e=="cancelled"));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn degeneracy_and_resource_limits_are_reported() {
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]];
        assert!(tetrahedralize(&p, Limits::default()).is_err());
        assert!(tetrahedralize(
            &p,
            Limits {
                max_points: 3,
                max_cells: 100
            }
        )
        .is_err());
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let mut limited = p.to_vec();
        limited.push([0.1, 0.1, 0.1]);
        assert!(tetrahedralize(
            &limited,
            Limits {
                max_points: 5,
                max_cells: 1
            }
        )
        .is_err());
        assert_eq!(
            tetrahedralize(
                &p,
                Limits {
                    max_points: 4,
                    max_cells: 1
                }
            )
            .unwrap()
            .cells()
            .len(),
            1
        );
        assert!((volume(&tetrahedralize(&p, Limits::default()).unwrap()) - 1. / 6.).abs() < 1e-15);
    }

    #[test]
    fn incremental_steiner_insertion_keeps_the_delaunay_property_and_volume() {
        let mut p = Vec::new();
        for x in [0., 1.] {
            for y in [0., 1.] {
                for z in [0., 1.] {
                    p.push([x, y, z]);
                }
            }
        }
        let mut d = Delaunay::new(&p, Limits::default()).unwrap();
        for q in [[0.5, 0.5, 0.5], [0.25, 0.3, 0.7], [0.5, 0.5, 0.]] {
            let before = d.topology().clone();
            let i = d.insert(q).unwrap();
            assert_eq!(i, p.len());
            p.push(q);
            assert!((volume(d.topology()) - 1.).abs() < 1e-14);
            assert_ne!(before, *d.topology());
        }
        for t in d.topology().cells() {
            for (i, &q) in p.iter().enumerate() {
                if !t.contains(&i) {
                    assert_ne!(sphere_side(t.map(|j| p[j]), q).unwrap(), SphereSide::Inside);
                }
            }
        }
        // A duplicate or over-budget insertion leaves the triangulation untouched.
        let before = d.topology().clone();
        assert!(d.insert([0.5, 0.5, 0.5]).is_err());
        assert_eq!(before, *d.topology());
        let mut tiny = Delaunay::new(
            &p[..8],
            Limits {
                max_points: 8,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(tiny.insert([0.5, 0.5, 0.5]).is_err());
    }
}
