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
    let topology = TetTopology::new(input.to_vec(), vec![seed], limits.max_cells)?;
    let mut bulk = Bulk::from_topology(input, &topology, limits.max_cells);
    for index in 0..input.len() {
        observe(index, input.len())?;
        if seed.contains(&index) {
            continue;
        }
        if !bulk.insert(index)? {
            let next = insert_index(input, &bulk.to_topology()?, index, limits.max_cells)?;
            bulk = Bulk::from_topology(input, &next, limits.max_cells);
        }
    }
    let topology = bulk.to_topology()?;
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
const NONE: usize = usize::MAX;
fn sorted(mut f: [usize; 3]) -> [usize; 3] {
    f.sort_unstable();
    f
}
fn parity(f: [usize; 3]) -> bool {
    ((f[0] > f[1]) as u8 + (f[0] > f[2]) as u8 + (f[1] > f[2]) as u8).is_multiple_of(2)
}
#[derive(Clone, Copy)]
struct Slot {
    v: [usize; 4],
    /// `n[k]` is the slot across the face opposite `v[k]` (`faces` order), `NONE` on the hull.
    n: [usize; 4],
    alive: bool,
}
/// Adjacency-linked working copy of a [`TetTopology`] for the bulk insertion loop: a located cavity is replaced locally
/// instead of re-deriving the whole topology per point. Slots are append-only (a removed cell is only marked dead), so
/// the alive slots in slot order are exactly the cell list the full rebuild of [`insert_index`] produces (retained cells
/// in order, then the cone cells in sorted-face order), with the same vertex order per cell.
struct Bulk<'a> {
    points: &'a [Point],
    slots: Vec<Slot>,
    alive: usize,
    /// Hull faces as (slot, local face): every `n[k] == NONE` of a live slot.
    hull: Vec<(usize, usize)>,
    last: usize,
    in_cavity: Vec<u32>,
    tested: Vec<u32>,
    epoch: u32,
    max_cells: usize,
}
struct Cone {
    face: [usize; 3],
    outside: Option<(usize, usize)>,
}
impl<'a> Bulk<'a> {
    fn from_topology(points: &'a [Point], t: &TetTopology, max_cells: usize) -> Self {
        let slots: Vec<Slot> = t
            .cells()
            .iter()
            .zip(t.neighbors())
            .map(|(v, n)| Slot {
                v: *v,
                n: n.map(|x| x.unwrap_or(NONE)),
                alive: true,
            })
            .collect();
        let hull = slots
            .iter()
            .enumerate()
            .flat_map(|(i, s)| (0..4).filter(move |&k| s.n[k] == NONE).map(move |k| (i, k)))
            .collect();
        Self {
            points,
            alive: slots.len(),
            last: slots.len().saturating_sub(1),
            in_cavity: vec![0; slots.len()],
            tested: vec![0; slots.len()],
            slots,
            hull,
            epoch: 0,
            max_cells,
        }
    }
    fn to_topology(&self) -> Result<TetTopology, MeshError> {
        let cells = self.slots.iter().filter(|s| s.alive).map(|s| s.v).collect();
        TetTopology::new(self.points.to_vec(), cells, self.max_cells)
    }
    fn face_points(&self, slot: usize, k: usize) -> [Point; 3] {
        faces(self.slots[slot].v)[k].map(|v| self.points[v])
    }
    /// Walk to the tetrahedron containing `p`; the flag is true when the walk left the hull. `None`: no verdict.
    fn locate(&self, p: Point) -> Result<Option<(usize, bool)>, MeshError> {
        let mut c = self.last;
        if !self.slots[c].alive {
            let Some(first) = self.slots.iter().position(|s| s.alive) else {
                return Ok(None);
            };
            c = first;
        }
        for step in 0..self.slots.len() * 4 + 64 {
            let t = &self.slots[c];
            let mut next = None;
            for j in 0..4 {
                // Rotate the starting face so a degenerate walk cannot loop forever.
                let k = (j + step) % 4;
                let f = self.face_points(c, k);
                if orientation(f[0], f[1], f[2], p)? == Ordering::Greater {
                    next = Some(t.n[k]);
                    break;
                }
            }
            match next {
                None => return Ok(Some((c, false))),
                Some(NONE) => return Ok(Some((c, true))),
                Some(n) => c = n,
            }
        }
        Ok(None)
    }
    fn conflicts(&self, slot: usize, p: Point) -> Result<bool, MeshError> {
        Ok(sphere_side(self.slots[slot].v.map(|v| self.points[v]), p)? != SphereSide::Outside)
    }
    /// Insert `points[index]` as [`insert_index`] would. `Ok(false)`: the local path cannot certify an identical result
    /// (degenerate coplanar hull case, budget, unmatched cavity surface): nothing was changed, use the full rebuild.
    fn insert(&mut self, index: usize) -> Result<bool, MeshError> {
        let p = self.points[index];
        let Some((start, outside)) = self.locate(p)? else {
            return Ok(false);
        };
        self.epoch += 1;
        let epoch = self.epoch;
        let mut cavity: Vec<usize> = Vec::new();
        let mut seeds: Vec<usize> = Vec::new();
        if outside {
            for &(c, k) in &self.hull {
                if self.tested[c] == epoch || self.in_cavity[c] == epoch {
                    continue;
                }
                let f = self.face_points(c, k);
                if orientation(f[0], f[1], f[2], p)? == Ordering::Less {
                    continue;
                }
                self.tested[c] = epoch;
                if self.conflicts(c, p)? {
                    seeds.push(c);
                }
            }
        } else if self.conflicts(start, p)? {
            seeds.push(start);
        } else {
            return Ok(false);
        }
        for &c in &seeds {
            self.in_cavity[c] = epoch;
        }
        cavity.extend(&seeds);
        let mut i = 0;
        while i < cavity.len() {
            let c = cavity[i];
            i += 1;
            for k in 0..4 {
                let m = self.slots[c].n[k];
                if m == NONE || self.in_cavity[m] == epoch || self.tested[m] == epoch {
                    continue;
                }
                if self.conflicts(m, p)? {
                    self.in_cavity[m] = epoch;
                    cavity.push(m);
                } else {
                    self.tested[m] = epoch;
                }
            }
        }
        let mut cones: Vec<Cone> = Vec::new();
        let mut detached: Vec<(usize, usize)> = Vec::new();
        let mut degenerate = false;
        for &c in &cavity {
            for k in 0..4 {
                let m = self.slots[c].n[k];
                let f = faces(self.slots[c].v)[k];
                if m != NONE && self.in_cavity[m] == epoch {
                    continue;
                }
                let o = orientation(self.points[f[0]], self.points[f[1]], self.points[f[2]], p)?;
                if o == Ordering::Equal {
                    // coplanar with the face: no cone (the full rebuild skips the flat tetrahedron), so the face stays open
                    // and the cell across it (if any) now has a hull face there
                    degenerate = true;
                    if m != NONE {
                        let lk = self.slots[m].n.iter().position(|&x| x == c).ok_or_else(|| {
                            MeshError::NonConforming("asymmetric tetrahedron adjacency".into())
                        })?;
                        detached.push((m, lk));
                    }
                    continue;
                }
                if m == NONE {
                    // a hull face of a removed cell: the new hull keeps it unless the point sees it
                    if o == Ordering::Less {
                        cones.push(Cone { face: f, outside: None });
                    }
                    continue;
                }
                // the face as listed by its lower-numbered owner, as the full rebuild does
                let lk = self.slots[m].n.iter().position(|&x| x == c).ok_or_else(|| {
                    MeshError::NonConforming("asymmetric tetrahedron adjacency".into())
                })?;
                let face = if c < m { f } else { faces(self.slots[m].v)[lk] };
                cones.push(Cone { face, outside: Some((m, lk)) });
            }
        }
        if outside {
            for &(c, k) in &self.hull {
                if self.in_cavity[c] == epoch {
                    continue;
                }
                let f = faces(self.slots[c].v)[k];
                if orientation(self.points[f[0]], self.points[f[1]], self.points[f[2]], p)?
                    == Ordering::Greater
                {
                    cones.push(Cone { face: f, outside: Some((c, k)) });
                }
            }
        }
        if cones.is_empty() || self.alive - cavity.len() + cones.len() > self.max_cells {
            return Ok(false);
        }
        cones.sort_unstable_by_key(|c| sorted(c.face));
        let base = self.slots.len();
        let mut fresh: Vec<Slot> = Vec::with_capacity(cones.len());
        let mut shared = BTreeMap::<[usize; 3], (usize, usize)>::new();
        let mut links: Vec<(usize, usize, usize, usize)> = Vec::new();
        for (i, cone) in cones.iter().enumerate() {
            let [a, b, c] = cone.face;
            let mut v = [a, b, c, index];
            if orientation(self.points[a], self.points[b], self.points[c], p)? == Ordering::Less {
                v.swap(0, 1);
            }
            let mut n = [NONE; 4];
            if let Some((m, lk)) = cone.outside {
                n[3] = m;
                if parity(faces(v)[3]) == parity(faces(self.slots[m].v)[lk]) {
                    return Ok(false);
                }
                links.push((m, lk, base + i, 3));
            }
            for (k, f) in faces(v).into_iter().enumerate().take(3) {
                match shared.remove(&sorted(f)) {
                    Some((j, kj)) => {
                        if parity(faces(fresh[j].v)[kj]) == parity(f) {
                            return Ok(false);
                        }
                        n[k] = base + j;
                        fresh[j].n[kj] = base + i;
                    }
                    None => {
                        shared.insert(sorted(f), (i, k));
                    }
                }
            }
            fresh.push(Slot { v, n, alive: true });
        }
        // Faces through the new point left unmatched are the new hull faces of a point outside the hull (or beside a
        // coplanar one); on a closed cavity surface they cannot occur.
        if !outside && !degenerate && !shared.is_empty() {
            return Ok(false);
        }
        for &c in &cavity {
            self.slots[c].alive = false;
        }
        for (m, lk, slot, _) in links {
            self.slots[m].n[lk] = slot;
        }
        for &(m, lk) in &detached {
            self.slots[m].n[lk] = NONE;
        }
        self.alive = self.alive - cavity.len() + fresh.len();
        let slots = &self.slots;
        self.hull.retain(|&(c, k)| slots[c].alive && slots[c].n[k] == NONE);
        for (i, s) in fresh.iter().enumerate() {
            for k in 0..4 {
                if s.n[k] == NONE {
                    self.hull.push((base + i, k));
                }
            }
        }
        self.hull.extend(detached);
        self.slots.extend(fresh);
        self.in_cavity.resize(self.slots.len(), 0);
        self.tested.resize(self.slots.len(), 0);
        self.last = base;
        Ok(true)
    }
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

    /// The local-cavity bulk loop against the full-rebuild reference: same cells in the same order with the same vertex
    /// order, on clouds that grow the hull, on cospherical lattices (every tie rule), and on coplanar-heavy sets.
    fn seed_of(input: &[Point]) -> [usize; 4] {
        let c = (2..input.len())
            .find(|&i| {
                [(0, 1), (0, 2), (1, 2)].iter().any(|&(x, y)| {
                    let q = |p: Point| robust::Coord { x: p[x], y: p[y] };
                    robust::orient2d(q(input[0]), q(input[1]), q(input[i])) != 0.
                })
            })
            .unwrap();
        let d = (2..input.len())
            .find(|&i| orientation(input[0], input[1], input[c], input[i]).unwrap() != Ordering::Equal)
            .unwrap();
        [0, 1, c, d]
    }
    fn both(input: &[Point]) -> (TetTopology, TetTopology, usize) {
        let limits = Limits { max_points: 100_000, max_cells: 1_000_000 };
        let seed = seed_of(input);
        let start = TetTopology::new(input.to_vec(), vec![seed], limits.max_cells).unwrap();
        let mut reference = start.clone();
        let mut bulk = Bulk::from_topology(input, &start, limits.max_cells);
        let mut fallbacks = 0;
        for index in 0..input.len() {
            if seed.contains(&index) {
                continue;
            }
            reference = insert_index(input, &reference, index, limits.max_cells).unwrap();
            if !bulk.insert(index).unwrap() {
                fallbacks += 1;
                let next = insert_index(input, &bulk.to_topology().unwrap(), index, limits.max_cells).unwrap();
                bulk = Bulk::from_topology(input, &next, limits.max_cells);
            }
            assert_eq!(bulk.to_topology().unwrap(), reference, "diverged after point {index}");
        }
        (bulk.to_topology().unwrap(), reference, fallbacks)
    }
    #[test]
    fn bulk_insertion_equals_the_full_rebuild() {
        let mut state = 99u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        // random cloud in arbitrary order: the hull grows on most insertions
        let cloud: Vec<Point> = (0..120).map(|_| std::array::from_fn(|_| next() * 2. - 1.)).collect();
        let (a, b, fallbacks) = both(&cloud);
        assert_eq!(a, b);
        assert!(fallbacks < 10, "the local path should handle generic clouds, fell back {fallbacks} times");
        // cospherical lattice, scrambled: ties everywhere, coplanar hull faces
        let mut lattice: Vec<Point> = Vec::new();
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    lattice.push([x as f64, y as f64, z as f64]);
                }
            }
        }
        for i in (1..lattice.len()).rev() {
            lattice.swap(i, (next() * (i + 1) as f64) as usize);
        }
        let (a, b, _) = both(&lattice);
        assert_eq!(a, b);
        // box corners last, as `plc_conform` appends its enclosing points
        let mut boxed: Vec<Point> = (0..80).map(|_| std::array::from_fn(|_| next())).collect();
        for x in [-9., 10.] {
            for y in [-9., 10.] {
                for z in [-9., 10.] {
                    boxed.push([x, y, z]);
                }
            }
        }
        let (a, b, _) = both(&boxed);
        assert_eq!(a, b);
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
