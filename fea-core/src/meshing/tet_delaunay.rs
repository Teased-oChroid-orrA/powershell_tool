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
    // Validates coordinate envelope and exact duplicates before arithmetic on bounds.
    TetTopology::new(input.to_vec(), vec![], limits.max_cells)?;
    let mut lo = input[0];
    let mut hi = lo;
    for p in input {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let centre = std::array::from_fn::<_, 3, _>(|k| lo[k] + (hi[k] - lo[k]) * 0.5);
    let scale = (0..3).map(|k| hi[k] - lo[k]).fold(0f64, f64::max) * 8.;
    if !scale.is_finite() || scale == 0. {
        return Err(MeshError::Failed(
            "point set has zero or unsupported extent".into(),
        ));
    }
    let mut points = input.to_vec();
    let n = points.len();
    for d in [[-1., -1., -1.], [1., -1., 1.], [-1., 1., 1.], [1., 1., -1.]] {
        points.push(std::array::from_fn(|k| centre[k] + scale * d[k]));
    }
    let mut topology = TetTopology::new(
        points.clone(),
        vec![[n, n + 1, n + 2, n + 3]],
        limits.max_cells,
    )?;
    for (index, &point) in input.iter().enumerate() {
        observe(index, input.len())?;
        let mut cavity = BTreeSet::new();
        for (i, t) in topology.cells().iter().enumerate() {
            if sphere_side(t.map(|v| points[v]), point)? != SphereSide::Outside {
                cavity.insert(i);
            }
        }
        if cavity.is_empty() {
            return Err(MeshError::Failed(
                "no insertion cavity; numerical range or enclosure failure".into(),
            ));
        }
        let mut boundary = BTreeMap::<[usize; 3], ([usize; 3], usize)>::new();
        for &i in &cavity {
            for f in faces(topology.cells()[i]) {
                let mut k = f;
                k.sort_unstable();
                let e = boundary.entry(k).or_insert((f, 0));
                e.1 += 1;
            }
        }
        let mut cells: Vec<_> = topology
            .cells()
            .iter()
            .enumerate()
            .filter(|(i, _)| !cavity.contains(i))
            .map(|(_, t)| *t)
            .collect();
        for (face, count) in boundary.values() {
            if *count == 1 {
                let [a, b, c] = *face;
                if orientation(points[a], points[b], points[c], point)? == Ordering::Equal {
                    continue;
                }
                if cells.len() >= limits.max_cells {
                    return Err(MeshError::LimitExceeded(
                        "insertion cell budget exceeded".into(),
                    ));
                }
                cells.push([a, b, c, index]);
            }
        }
        let next = TetTopology::new(points.clone(), cells, limits.max_cells)?;
        // Insertion inside the superstructure cannot change its outer boundary.
        let signature = |t: &TetTopology| -> BTreeSet<_> {
            t.boundary()
                .iter()
                .map(|f| {
                    let mut f = *f;
                    f.sort_unstable();
                    f
                })
                .collect()
        };
        if signature(&topology) != signature(&next) {
            return Err(MeshError::NonConforming(
                "insertion changes the superstructure boundary".into(),
            ));
        }
        topology = next;
    }
    let cells: Vec<_> = topology
        .cells()
        .iter()
        .filter(|t| t.iter().all(|&v| v < n))
        .copied()
        .collect();
    if cells.is_empty() {
        return Err(MeshError::Unsupported(
            "point set is not a resolved three-dimensional volume".into(),
        ));
    }
    let used: BTreeSet<_> = cells.iter().flatten().copied().collect();
    if used.len() != n {
        return Err(MeshError::Failed(
            "not every input point survived superstructure removal".into(),
        ));
    }
    observe(input.len(), input.len())?;
    TetTopology::new(input.to_vec(), cells, limits.max_cells)
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
        assert!(tetrahedralize(
            &p,
            Limits {
                max_points: 4,
                max_cells: 1
            }
        )
        .is_err());
        assert!((volume(&tetrahedralize(&p, Limits::default()).unwrap()) - 1. / 6.).abs() < 1e-15);
    }
}
