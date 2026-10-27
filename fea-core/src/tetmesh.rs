//! Tetrahedral meshes of a solid given only as a **signed distance** (negative inside) and a **size field**: the volume
//! mesher for solids that come from a B-rep kernel (which triangulates surfaces but does not mesh volumes) and the 3D
//! size-field remesher that adaptive refinement needs.
//!
//! The method is deliberately simple and robust rather than clever:
//!
//! 1. A Kuhn (six tetrahedra per cube) lattice covers the bounding box. It is conforming and every tetrahedron shares the
//!    same diagonal, which is what makes the next step terminate with bounded quality loss.
//! 2. **Longest-edge bisection with Rivara propagation** refines it until each tetrahedron is no larger than the size field
//!    at its centroid, and no larger than `boundary_size` where it touches or is near the boundary. A bisected edge is split
//!    in every tetrahedron around it (after first splitting those for which it is not the longest edge), so the mesh stays
//!    conforming. The size field is any function of position: an error estimate turned into a size map refines the mesh
//!    where the solution needs it.
//! 3. Tetrahedra whose centroid is inside are kept. Vertices of the resulting boundary faces are **snapped** onto the surface
//!    along the distance gradient, a step being refused when it would invert or flatten a neighbouring tetrahedron.
//! 4. Boundary faces are named by `face_of`, a function from a point on the surface to the index of the model face it
//!    belongs to (a B-rep face id, or the primitive that defines it), so loads and supports bind to faces by name (`faceN`)
//!    exactly like sketch edges. Faces are oriented outward, ready for `Loads::faces`.
//!
//! `Tet10` adds midside nodes, those on boundary edges moved onto the surface so a curved wall is represented by curved
//! elements (each move is kept only while every tetrahedron around the edge stays a proper element).
//!
//! Limits, stated so they are not rediscovered: thin features smaller than `boundary_size` are not resolved (a wall thinner
//! than two elements gets one), a closed sharp feature is rounded by the snapping within one element, and the quality of the
//! boundary layer is only as good as the snapping allows (`MeshReport::worst_quality`).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::mesh::{Elastic, Mesh, Physics};
use std::collections::HashMap;

/// A solid and how finely to mesh it.
pub struct Solid<'a> {
    /// Corners of a box that contains the solid.
    pub lo: [f64; 3],
    pub hi: [f64; 3],
    /// Signed distance (negative inside). It only has to be a good distance near the surface.
    pub sdf: &'a dyn Fn([f64; 3]) -> f64,
    /// Target edge length at a point (the size field).
    pub size: &'a dyn Fn([f64; 3]) -> f64,
    /// Index of the model face a point on (or next to) the surface belongs to.
    pub face_of: &'a dyn Fn([f64; 3]) -> usize,
    /// Optional: for a point, nearby faces with signed distances and projection directions to their trimmed patches.
    /// At a point on a patch the direction must be its outward unit normal. With it, a vertex near two or three faces is snapped onto their edge or
    /// corner instead of rounding the feature off, and boundary triangles are assigned to the face all their vertices
    /// lie on. A B-rep kernel answers it from the distance to each face.
    pub face_dists: Option<&'a dyn Fn([f64; 3]) -> Vec<FaceHit>>,
}

/// One face near a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceHit {
    pub face: usize,
    /// Distance to the trimmed face, signed by its supporting surface's outward side.
    pub dist: f64,
    /// Unit projection direction: `x - dist * normal` reaches the nearest trimmed face point.
    /// On the face, this is its outward unit normal.
    pub normal: [f64; 3],
}

impl FaceHit {
    pub(crate) fn surface_normal(&self, x: V3, fd: &dyn Fn(V3) -> Vec<FaceHit>, tol: f64) -> Option<V3> {
        if !self.dist.is_finite() || self.normal.iter().any(|v| !v.is_finite()) { return None; }
        let foot = std::array::from_fn(|k| x[k] - self.dist * self.normal[k]);
        if foot.iter().any(|v| !v.is_finite()) { return None; }
        let on_face = fd(foot).into_iter().find(|f| f.face == self.face)?;
        if !(on_face.dist.abs() <= tol)
            || on_face.normal.iter().any(|v| !v.is_finite())
            || (norm(on_face.normal) - 1.0).abs() > 1e-6
        { return None; }
        Some(on_face.normal)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TetOptions {
    /// Edge length of the starting lattice (the coarsest element the size field may ask for).
    pub base: f64,
    /// Largest edge of a tetrahedron that touches or lies near the boundary.
    pub boundary_size: f64,
    /// Refuse to build more tetrahedra than this.
    pub max_tets: usize,
    pub quadratic: bool,
    pub snap: bool,
    /// After the snap, repair the boundary layer so every element passes the scaled-Jacobian gate
    /// (`meshing::boundary_repair`). Needs `snap`.
    pub repair: bool,
}
impl Default for TetOptions {
    fn default() -> Self {
        Self { base: 1.0, boundary_size: 0.25, max_tets: 2_000_000, quadratic: false, snap: true, repair: false }
    }
}

/// What the mesher did, for the panel and the tests.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshReport {
    pub tets: usize,
    pub nodes: usize,
    /// Enclosed volume of the mesh.
    pub volume: f64,
    /// Smallest `6 sqrt(2) V / l_rms^3` over the tetrahedra (1 is a regular tetrahedron, 0 a flat one).
    pub worst_quality: f64,
    /// Boundary vertices moved onto the surface / vertices whose move was refused.
    pub snapped: usize,
    pub refused: usize,
}

/// A snap or a smoothing step may not leave a tetrahedron below this quality (unless it already was).
const MIN_QUALITY: f64 = 0.05;
/// ... and not below this fraction of what it was.
const MAX_QUALITY_LOSS: f64 = 0.3;
/// Snap-and-smooth rounds.
const SNAP_PASSES: usize = 6;
/// A feature-snap target further than this fraction of the vertex's local size from the surface is rejected (ill-conditioned
/// plane intersection at a tangent edge); the plain projection is used instead.
const OFF_SURFACE: f64 = 0.05;
/// A boundary vertex counts as lying on a model face when within this fraction of the boundary size of it.
const FACE_TOL: f64 = 0.01;
/// Reconnect-and-snap-again rounds after the first snap.
const RECONNECT_ROUNDS: usize = 4;

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn mid(a: V3, b: V3) -> V3 {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0]
}
/// Signed volume of the tetrahedron (positive when `d` is on the side `(b - a) x (c - a)` points to).
fn volume(p: &[V3], t: [usize; 4]) -> f64 {
    dot(cross(sub(p[t[1]], p[t[0]]), sub(p[t[2]], p[t[0]])), sub(p[t[3]], p[t[0]])) / 6.0
}
/// Remove the smaller sectors where the kept cut is pinched: the tetrahedra around a vertex or an edge must form one
/// face-connected fan, otherwise the boundary is not a manifold there (two sectors touching only along that edge or at that
/// point), and no later snap or repair can make it one. Each fan keeps its largest-volume sector (ties: lowest cell index);
/// removal can expose a new pinch, so it repeats until none is left. Returns the number of tetrahedra removed.
fn drop_pinched_sectors(p: &[V3], cells: &mut Vec<[usize; 4]>) -> usize {
    use std::collections::{BTreeMap, BTreeSet};
    let mut removed = 0;
    loop {
        let mut vt = vec![Vec::new(); p.len()];
        for (i, t) in cells.iter().enumerate() {
            for &v in t {
                vt[v].push(i);
            }
        }
        let mut doomed = BTreeSet::new();
        let mut keys: Vec<Vec<usize>> = (0..p.len()).filter(|&v| !vt[v].is_empty()).map(|v| vec![v]).collect();
        let mut edges = BTreeSet::new();
        for t in cells.iter() {
            for i in 0..4 {
                for j in i + 1..4 {
                    edges.insert((t[i].min(t[j]), t[i].max(t[j])));
                }
            }
        }
        keys.extend(edges.into_iter().map(|(a, b)| vec![a, b]));
        for key in keys {
            let fan: Vec<usize> = vt[key[0]].iter().copied().filter(|&i| !doomed.contains(&i) && key.iter().all(|v| cells[i].contains(v))).collect();
            if fan.len() < 2 {
                continue;
            }
            // two cells of the fan are neighbours when they share a face holding `key`, i.e. `3 - key.len()` more vertices
            let need = 3 - key.len();
            let mut by_rest = BTreeMap::<Vec<usize>, Vec<usize>>::new();
            for &i in &fan {
                let rest: Vec<usize> = cells[i].iter().copied().filter(|v| !key.contains(v)).collect();
                for mask in 0u32..1 << rest.len() {
                    if mask.count_ones() as usize == need {
                        let mut f: Vec<usize> = (0..rest.len()).filter(|k| mask & (1 << k) != 0).map(|k| rest[k]).collect();
                        f.sort_unstable();
                        by_rest.entry(f).or_default().push(i);
                    }
                }
            }
            let mut comp = BTreeMap::<usize, usize>::new();
            let mut sectors: Vec<Vec<usize>> = Vec::new();
            for &start in &fan {
                if comp.contains_key(&start) {
                    continue;
                }
                let id = sectors.len();
                let mut sector = Vec::new();
                let mut stack = vec![start];
                comp.insert(start, id);
                while let Some(i) = stack.pop() {
                    sector.push(i);
                    for group in by_rest.values().filter(|g| g.contains(&i)) {
                        for &j in group {
                            if let std::collections::btree_map::Entry::Vacant(e) = comp.entry(j) {
                                e.insert(id);
                                stack.push(j);
                            }
                        }
                    }
                }
                sectors.push(sector);
            }
            if sectors.len() < 2 {
                continue;
            }
            let weight = |s: &Vec<usize>| s.iter().map(|&i| volume(p, cells[i]).abs()).sum::<f64>();
            let keep = (0..sectors.len())
                .max_by(|&a, &b| weight(&sectors[a]).total_cmp(&weight(&sectors[b])).then_with(|| sectors[b].iter().min().cmp(&sectors[a].iter().min())))
                .unwrap();
            for (k, s) in sectors.iter().enumerate() {
                if k != keep {
                    doomed.extend(s.iter().copied());
                }
            }
        }
        if doomed.is_empty() {
            return removed;
        }
        removed += doomed.len();
        let mut i = 0;
        cells.retain(|_| {
            let keep = !doomed.contains(&i);
            i += 1;
            keep
        });
    }
}
/// `6 sqrt(2) V / l_rms^3`: 1 for a regular tetrahedron, 0 for a flat one, negative when inverted.
fn quality(p: &[V3], t: [usize; 4]) -> f64 {
    let mut sum = 0.0;
    for i in 0..4 {
        for j in i + 1..4 {
            let l = norm(sub(p[t[i]], p[t[j]]));
            sum += l * l;
        }
    }
    let lrms = (sum / 6.0).sqrt();
    if lrms <= 0.0 {
        return -1.0;
    }
    6.0 * 2f64.sqrt() * volume(p, t) / lrms.powi(3)
}
fn key(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// The lattice and its refinement.
struct Refiner<'a> {
    p: Vec<V3>,
    tets: Vec<[usize; 4]>,
    alive: Vec<bool>,
    /// Tetrahedra around each edge.
    around: HashMap<(usize, usize), Vec<usize>>,
    mids: HashMap<(usize, usize), usize>,
    s: &'a Solid<'a>,
    boundary_size: f64,
    max_tets: usize,
    live: usize,
}

impl<'a> Refiner<'a> {
    fn len(&self, e: (usize, usize)) -> f64 {
        norm(sub(self.p[e.0], self.p[e.1]))
    }
    /// A strict total order on edges: length, then vertex ids, the same for every tetrahedron that contains the edge.
    fn longer(&self, a: (usize, usize), b: (usize, usize)) -> bool {
        self.len(a).total_cmp(&self.len(b)).then(a.cmp(&b)).is_gt()
    }
    fn longest(&self, t: usize) -> (usize, usize) {
        let v = self.tets[t];
        let mut best = key(v[0], v[1]);
        for i in 0..4 {
            for j in i + 1..4 {
                let e = key(v[i], v[j]);
                if self.longer(e, best) {
                    best = e;
                }
            }
        }
        best
    }
    fn centroid(&self, t: usize) -> V3 {
        let v = self.tets[t];
        let mut c = [0.0; 3];
        for &i in &v {
            for k in 0..3 {
                c[k] += self.p[i][k] / 4.0;
            }
        }
        c
    }
    fn add_tet(&mut self, v: [usize; 4]) -> usize {
        let id = self.tets.len();
        self.tets.push(v);
        self.alive.push(true);
        self.live += 1;
        for i in 0..4 {
            for j in i + 1..4 {
                self.around.entry(key(v[i], v[j])).or_default().push(id);
            }
        }
        id
    }
    fn kill(&mut self, t: usize) {
        self.alive[t] = false;
        self.live -= 1;
        let v = self.tets[t];
        for i in 0..4 {
            for j in i + 1..4 {
                if let Some(list) = self.around.get_mut(&key(v[i], v[j])) {
                    list.retain(|&u| u != t);
                }
            }
        }
    }
    /// Does tetrahedron `t` still have to be split to meet the size field?
    fn needs(&self, t: usize) -> bool {
        let l = self.len(self.longest(t));
        let c = self.centroid(t);
        let mut target = (self.s.size)(c);
        let v = self.tets[t];
        let signs: Vec<bool> = v.iter().map(|&i| (self.s.sdf)(self.p[i]) < 0.0).collect();
        let near = (self.s.sdf)(c).abs() < l;
        if near || signs.iter().any(|&b| b) != signs.iter().all(|&b| b) {
            target = target.min(self.boundary_size);
        }
        l > target
    }
    /// Split edge `e` in every tetrahedron around it.
    fn bisect(&mut self, e: (usize, usize)) {
        let m = *self.mids.entry(e).or_insert_with(|| {
            self.p.push(mid(self.p[e.0], self.p[e.1]));
            self.p.len() - 1
        });
        let list = self.around.get(&e).cloned().unwrap_or_default();
        for t in list {
            let v = self.tets[t];
            self.kill(t);
            let mut a = v;
            let mut b = v;
            for k in 0..4 {
                if v[k] == e.0 {
                    a[k] = m;
                }
                if v[k] == e.1 {
                    b[k] = m;
                }
            }
            self.add_tet(a);
            self.add_tet(b);
        }
        self.mids.remove(&e);
    }
    /// Refine `t` at its longest edge, first refining any tetrahedron around that edge for which it is not the longest
    /// (Rivara propagation), so the mesh stays conforming. An explicit stack: the chain can be long.
    fn refine(&mut self, t: usize) -> Result<(), String> {
        let mut stack = vec![t];
        while let Some(&top) = stack.last() {
            if !self.alive[top] {
                stack.pop();
                continue;
            }
            if self.live > self.max_tets {
                return Err(format!("more than {} tetrahedra: the size field is too fine (raise the sizes or the limit)", self.max_tets));
            }
            let e = self.longest(top);
            let blocker = self.around.get(&e).and_then(|l| l.iter().copied().find(|&u| self.longest(u) != e));
            match blocker {
                Some(u) => stack.push(u),
                None => {
                    self.bisect(e);
                    stack.pop();
                }
            }
        }
        Ok(())
    }
}

/// Kuhn lattice over the box: `cells` cubes per axis, six tetrahedra each.
fn lattice(lo: V3, hi: V3, base: f64) -> Result<(Vec<V3>, Vec<[usize; 4]>), String> {
    let n: [usize; 3] = std::array::from_fn(|k| (((hi[k] - lo[k]) / base).ceil() as usize).max(1));
    if n[0].saturating_mul(n[1]).saturating_mul(n[2]) > 2_000_000 {
        return Err("the starting lattice is too fine: raise the base size".into());
    }
    let h: V3 = std::array::from_fn(|k| (hi[k] - lo[k]) / n[k] as f64);
    let idx = |i: usize, j: usize, k: usize| i + (n[0] + 1) * (j + (n[1] + 1) * k);
    let mut p = Vec::with_capacity((n[0] + 1) * (n[1] + 1) * (n[2] + 1));
    for k in 0..=n[2] {
        for j in 0..=n[1] {
            for i in 0..=n[0] {
                p.push([lo[0] + h[0] * i as f64, lo[1] + h[1] * j as f64, lo[2] + h[2] * k as f64]);
            }
        }
    }
    const PERMS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let mut tets = Vec::new();
    for k in 0..n[2] {
        for j in 0..n[1] {
            for i in 0..n[0] {
                for perm in PERMS {
                    let mut b = [0usize; 3];
                    let mut v = [idx(i, j, k); 4];
                    for (s, axis) in perm.iter().enumerate() {
                        b[*axis] = 1;
                        v[s + 1] = idx(i + b[0], j + b[1], k + b[2]);
                    }
                    tets.push(v);
                }
            }
        }
    }
    Ok((p, tets))
}

fn gradient(sdf: &dyn Fn(V3) -> f64, x: V3, eps: f64) -> V3 {
    let mut g = [0.0; 3];
    for k in 0..3 {
        let (mut a, mut b) = (x, x);
        a[k] -= eps;
        b[k] += eps;
        g[k] = (sdf(b) - sdf(a)) / (2.0 * eps);
    }
    g
}

/// Move `x` onto the zero level set of `sdf` along its gradient (a few Newton steps).
fn project(sdf: &dyn Fn(V3) -> f64, mut x: V3, eps: f64) -> V3 {
    for _ in 0..4 {
        let d = sdf(x);
        let g = gradient(sdf, x, eps);
        let g2 = dot(g, g);
        if g2 < 1e-24 || d.abs() < 1e-10 * eps.max(1e-300) {
            break;
        }
        for k in 0..3 {
            x[k] -= d * g[k] / g2;
        }
    }
    x
}

/// Flips and interior smoothing of the tetrahedra (surface faces and boundary vertices never change), re-oriented positively.
/// `None` when the mesh is not a valid topology for the exact machinery; the caller then keeps what it has.
fn reconnect(p: &[V3], tet: &[[usize; 4]]) -> Option<(Vec<[usize; 4]>, Vec<V3>)> {
    use crate::meshing::{
        tet_flip::FlipOptions,
        tet_smooth::{optimize, SmoothOptions},
        tet_topology::TetTopology,
    };
    let top = TetTopology::new(p.to_vec(), tet.to_vec(), tet.len() * 4 + 16).ok()?;
    let (out, _) = optimize(&top, &std::collections::BTreeSet::new(), FlipOptions { max_passes: 3, ..FlipOptions::default() }, &SmoothOptions { max_passes: 2, ..SmoothOptions::default() }, 1).ok()?;
    let points = out.points().to_vec();
    let cells = out
        .cells()
        .iter()
        .map(|&c| {
            let mut v = c;
            if volume(&points, v) < 0.0 {
                v.swap(1, 2);
            }
            v
        })
        .collect();
    Some((cells, points))
}

/// Mesh `s` into tetrahedra with material `mat`.
pub fn mesh_solid(s: &Solid, mat: Elastic, opts: TetOptions) -> Result<(Mesh, MeshReport), String> {
    mesh_solid_reported(s, mat, opts).map(|(m, r, _)| (m, r))
}

/// [`mesh_solid`] that also returns what the boundary-layer repair did (`None` when it did not run).
pub fn mesh_solid_reported(s: &Solid, mat: Elastic, opts: TetOptions) -> Result<(Mesh, MeshReport, Option<crate::meshing::boundary_repair::RepairReport>), String> {
    let finite_box = (0..3).all(|k| s.hi[k].is_finite() && s.lo[k].is_finite() && s.hi[k] > s.lo[k]);
    if !(opts.base.is_finite() && opts.base > 0.0 && opts.boundary_size.is_finite() && opts.boundary_size > 0.0 && finite_box) {
        return Err("the mesher needs a positive base size, a positive boundary size and a box with volume".into());
    }
    let (p, tets) = lattice(s.lo, s.hi, opts.base)?;
    let mut r = Refiner { p, tets: Vec::new(), alive: Vec::new(), around: HashMap::new(), mids: HashMap::new(), s, boundary_size: opts.boundary_size, max_tets: opts.max_tets, live: 0 };
    for t in tets {
        r.add_tet(t);
    }
    // Refine until nothing asks for more. A split creates new tetrahedra, which are looked at in turn.
    let mut queue: Vec<usize> = (0..r.tets.len()).collect();
    while let Some(t) = queue.pop() {
        if !r.alive[t] || !r.needs(t) {
            continue;
        }
        let before = r.tets.len();
        r.refine(t)?;
        queue.extend(before..r.tets.len());
        if r.alive[t] {
            queue.push(t);
        }
    }

    // Keep what is inside.
    let mut kept: Vec<[usize; 4]> = (0..r.tets.len()).filter(|&t| r.alive[t] && (s.sdf)(r.centroid(t)) < 0.0).map(|t| r.tets[t]).collect();
    drop_pinched_sectors(&r.p, &mut kept);
    if kept.is_empty() {
        return Err("no tetrahedron lies inside the solid: the size is too coarse for it or the box misses it".into());
    }
    // Compact the vertices and orient every tetrahedron positively.
    let mut renum = vec![usize::MAX; r.p.len()];
    let mut p: Vec<V3> = Vec::new();
    let mut tet: Vec<[usize; 4]> = Vec::with_capacity(kept.len());
    for t in &kept {
        let mut v = [0usize; 4];
        for (k, &old) in t.iter().enumerate() {
            if renum[old] == usize::MAX {
                renum[old] = p.len();
                p.push(r.p[old]);
            }
            v[k] = renum[old];
        }
        if volume(&p, v) < 0.0 {
            v.swap(1, 2);
        }
        tet.push(v);
    }

    // Boundary faces: faces of exactly one tetrahedron, each with the fourth vertex of that tetrahedron (the interior side).
    let boundary_of = |tet: &[[usize; 4]]| -> Vec<([usize; 3], usize)> {
        let faces_of = |v: [usize; 4]| -> [([usize; 3], usize); 4] { [([v[1], v[2], v[3]], v[0]), ([v[0], v[3], v[2]], v[1]), ([v[0], v[1], v[3]], v[2]), ([v[0], v[2], v[1]], v[3])] };
        let mut count: HashMap<[usize; 3], (usize, [usize; 3], usize)> = HashMap::new();
        for v in tet {
            for (f, opposite) in faces_of(*v) {
                let mut k = f;
                k.sort_unstable();
                let e = count.entry(k).or_insert((0, f, opposite));
                e.0 += 1;
            }
        }
        let mut boundary: Vec<([usize; 3], usize)> = count.values().filter(|c| c.0 == 1).map(|c| (c.1, c.2)).collect();
        boundary.sort();
        boundary
    };
    let mut boundary = boundary_of(&tet);
    let mut on_boundary = vec![false; p.len()];
    for (f, _) in &boundary {
        for &v in f {
            on_boundary[v] = true;
        }
    }

    // Snap boundary vertices onto the surface, refusing a step that flattens or badly degrades a neighbour, and smooth the
    // interior between passes so the layer next to the surface has room. Nearest-to-the-surface first: a vertex that is
    // almost there moves with little disturbance and gives its neighbours room; the far ones follow in later passes.
    let (mut snapped, mut refused) = (0, 0);
    if opts.snap {
        // Round 0 snaps and smooths; the boundary layer is then reconnected (interior flips and smoothing only, so the surface
        // faces are untouched) and round 1 snaps again: boundary-layer slivers were what kept vertices from the surface.
        let mut order: Vec<usize> = Vec::new();
        let mut local: Vec<f64> = Vec::new();
        for round in 0..RECONNECT_ROUNDS + 1 {
            let mut incident: Vec<Vec<usize>> = vec![Vec::new(); p.len()];
            let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); p.len()];
            for (t, v) in tet.iter().enumerate() {
                for i in 0..4 {
                    incident[v[i]].push(t);
                    for j in 0..4 {
                        if i != j && !neighbours[v[i]].contains(&v[j]) {
                            neighbours[v[i]].push(v[j]);
                        }
                    }
                }
            }
            // Reach is local: the longest edge at the vertex, not the global boundary size. Where the size field asks for
            // elements far smaller than `boundary_size`, a global reach would pull several vertices onto one feature point.
            local = (0..p.len()).map(|v| neighbours[v].iter().map(|&u| norm(sub(p[u], p[v]))).fold(0.0, f64::max).min(opts.boundary_size)).collect();
            order = (0..p.len()).filter(|&v| on_boundary[v]).collect();
            order.sort_by(|&a, &b| (s.sdf)(p[a]).abs().total_cmp(&(s.sdf)(p[b]).abs()));
            for _pass in 0..SNAP_PASSES {
                let mut moved_any = false;
                for &v in &order {
                    let (reach, eps) = (1.1 * local[v], local[v] * 1e-3);
                    if (s.sdf)(p[v]).abs() > reach {
                        continue;
                    }
                    let target = match s.face_dists {
                        Some(fd) => snap_to_features(fd, s.sdf, p[v], 0.5 * local[v], reach, eps).unwrap_or_else(|| project(s.sdf, p[v], eps)),
                        None => project(s.sdf, p[v], eps),
                    };
                    // The feature snap intersects the planes of the nearest triangle of each face; where those planes are nearly
                    // parallel (a fillet meeting a flat face tangentially) the intersection is ill-conditioned and the alternating
                    // projection can end more than an element away (a vertex 1.4 mm above a 16 mm face at a 1 mm size). A target
                    // that is not on the surface is no feature point: take the plain projection instead.
                    let target = if s.face_dists.is_some() && (s.sdf)(target).abs() > OFF_SURFACE * local[v] { project(s.sdf, p[v], eps) } else { target };
                    if norm(sub(target, p[v])) < 1e-9 * local[v] {
                        continue;
                    }
                    let old = p[v];
                    let before = incident[v].iter().map(|&t| quality(&p, tet[t])).fold(f64::INFINITY, f64::min);
                    for fraction in [1.0, 0.5, 0.25] {
                        p[v] = [old[0] + fraction * (target[0] - old[0]), old[1] + fraction * (target[1] - old[1]), old[2] + fraction * (target[2] - old[2])];
                        let after = incident[v].iter().map(|&t| quality(&p, tet[t])).fold(f64::INFINITY, f64::min);
                        if after >= MIN_QUALITY.min(before) && after >= MAX_QUALITY_LOSS * before {
                            moved_any = true;
                            break;
                        }
                        p[v] = old;
                    }
                }
                // Laplacian smoothing of the interior, kept only when the worst neighbouring tetrahedron improves.
                for _ in 0..2 {
                    for v in 0..p.len() {
                        if on_boundary[v] || neighbours[v].is_empty() {
                            continue;
                        }
                        let old = p[v];
                        let worst = |p: &[V3]| incident[v].iter().map(|&t| quality(p, tet[t])).fold(f64::INFINITY, f64::min);
                        let before = worst(&p);
                        let n = neighbours[v].len() as f64;
                        let mut c = [0.0; 3];
                        for &u in &neighbours[v] {
                            for k in 0..3 {
                                c[k] += p[u][k] / n;
                            }
                        }
                        p[v] = c;
                        if worst(&p) <= before {
                            p[v] = old;
                        }
                    }
                }
                if !moved_any {
                    break;
                }
            }
            // Nothing left to gain once every boundary vertex that could be snapped sits on the surface.
            if order.iter().all(|&v| (s.sdf)(p[v]).abs() <= 0.02 * local[v]) {
                break;
            }
            if round < RECONNECT_ROUNDS {
                // Accept the reconnection only if the mesher's own worst quality does not fall below the floor it keeps.
                let worst = |p: &[V3], tet: &[[usize; 4]]| tet.iter().map(|&t| quality(p, t)).fold(f64::INFINITY, f64::min);
                let before = worst(&p, &tet);
                if let Some((cells, points)) = reconnect(&p, &tet) {
                    if worst(&points, &cells) >= before.min(MIN_QUALITY) {
                        tet = cells;
                        p = points;
                    }
                }
            }
        }
        // Reconnection may have replaced the tetrahedron behind a boundary face: refresh each face's interior-side vertex (the
        // faces themselves are unchanged), which the outward orientation of the named faces is taken from.
        boundary = boundary_of(&tet);
        // What ended on the surface and what did not.
        for &v in &order {
            if (s.sdf)(p[v]).abs() <= 0.02 * local[v] {
                snapped += 1;
            } else {
                refused += 1;
            }
        }
    }

    // Boundary-layer repair: the snap leaves a few percent of the elements next to a curved surface flat.
    let mut repair_report = None;
    if opts.snap && opts.repair {
        let classify = |v: [V3; 3]| face_of_triangle(s, v, opts.boundary_size);
        let ctx = crate::meshing::boundary_repair::RepairContext {
            sdf: s.sdf,
            face_dists: s.face_dists,
            triangle_face: s.face_dists.map(|_| &classify as &dyn Fn([V3; 3]) -> usize),
        };
        let (remap, rep) = crate::meshing::boundary_repair::repair(&mut p, &mut tet, &ctx, crate::meshing::boundary_repair::RepairOptions::default());
        {
            boundary = boundary_of(&tet);
            on_boundary = vec![false; p.len()];
            for (f, _) in &boundary {
                for &v in f {
                    on_boundary[v] = true;
                }
            }
            let _ = remap;
            // what ended on the surface, recounted (the repair slides and removes boundary vertices)
            let mut longest = vec![0.0f64; p.len()];
            for v in &tet {
                for i in 0..4 {
                    for j in i + 1..4 {
                        let l = norm(sub(p[v[i]], p[v[j]]));
                        longest[v[i]] = longest[v[i]].max(l);
                        longest[v[j]] = longest[v[j]].max(l);
                    }
                }
            }
            snapped = 0;
            refused = 0;
            for v in 0..p.len() {
                if on_boundary[v] {
                    if (s.sdf)(p[v]).abs() <= 0.02 * longest[v].min(opts.boundary_size) {
                        snapped += 1;
                    } else {
                        refused += 1;
                    }
                }
            }
        }
        repair_report = Some(rep);
    }

    // Report.
    let mut total = 0.0;
    let mut worst = f64::INFINITY;
    for v in &tet {
        let vol = volume(&p, *v);
        total += vol;
        let mut sum = 0.0;
        for i in 0..4 {
            for j in i + 1..4 {
                let l = norm(sub(p[v[i]], p[v[j]]));
                sum += l * l;
            }
        }
        let lrms = (sum / 6.0).sqrt();
        worst = worst.min(6.0 * 2f64.sqrt() * vol / lrms.powi(3));
    }
    if worst <= 0.0 {
        return Err("the mesh has an inverted or flat tetrahedron: the size is too coarse for the boundary".into());
    }

    // Build the mesh.
    let mut mesh = Mesh::new(Physics::Solid);
    for x in &p {
        mesh.add_node(*x);
    }
    let kind = if opts.quadratic { ElementKind::Tet10 } else { ElementKind::Tet4 };
    const EDGES: [[usize; 2]; 6] = [[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]];
    // Midside nodes (Tet10) start on the straight edges.
    let mut mids: HashMap<(usize, usize), usize> = HashMap::new();
    let mut conn: Vec<usize> = Vec::with_capacity(tet.len() * kind.n_nodes());
    for v in &tet {
        conn.extend_from_slice(v);
        if opts.quadratic {
            for [a, b] in EDGES {
                let m = *mids.entry(key(v[a], v[b])).or_insert_with(|| mesh.add_node(mid(mesh.nodes[v[a]], mesh.nodes[v[b]])));
                conn.push(m);
            }
        }
    }
    mesh.add_block(kind, conn, mat, "solid")?;
    if opts.quadratic && opts.snap {
        curve_boundary_edges(&mut mesh, s, &boundary, &mids, &tet, opts.boundary_size);
    }

    // Named, outward-oriented boundary faces.
    let faces = faces_of_named(&boundary, &mesh.nodes, s, if opts.quadratic { Some(&mids) } else { None }, opts.boundary_size);
    for (name, list) in faces {
        mesh.surfaces.insert(name, list);
    }
    let report = MeshReport { tets: tet.len(), nodes: mesh.nodes.len(), volume: total, worst_quality: worst, snapped, refused };
    Ok((mesh, report, repair_report))
}

fn faces_of_named(boundary: &[([usize; 3], usize)], p: &[V3], s: &Solid, mids: Option<&HashMap<(usize, usize), usize>>, size: f64) -> Vec<(String, Vec<Vec<usize>>)> {
    let mut by_face: std::collections::BTreeMap<usize, Vec<Vec<usize>>> = std::collections::BTreeMap::new();
    for &(f, _) in boundary {
        // Outward by construction: the face is taken from a positively oriented tetrahedron, whose face opposite vertex k
        // is ordered so that its normal points away from that vertex. (Deciding it by a dot product with the fourth
        // vertex is ill-conditioned for the sliver triangles that lie on the surface itself.)
        let [a, b, c] = f;
        let mut nodes = vec![a, b, c];
        if let Some(m) = mids {
            nodes.extend([m[&key(a, b)], m[&key(b, c)], m[&key(c, a)]]);
        }
        by_face.entry(face_of_triangle(s, [p[a], p[b], p[c]], size)).or_default().push(nodes);
    }
    by_face.into_iter().map(|(i, list)| (format!("face{}", i + 1), list)).collect()
}

/// Reference points at which a quadratic tetrahedron's mapping must stay orientation preserving.
const SAMPLES: [[f64; 3]; 11] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.5, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.5], [0.5, 0.0, 0.5], [0.0, 0.5, 0.5], [0.25, 0.25, 0.25]];

/// Is the quadratic tetrahedron `nodes` (10 node ids) a proper, not badly distorted, element?
fn tet10_ok(mesh: &Mesh, nodes: &[usize]) -> bool {
    let corner = |i: usize| mesh.nodes[nodes[i]];
    let linear = 6.0 * volume(&[corner(0), corner(1), corner(2), corner(3)], [0, 1, 2, 3]);
    for xi in SAMPLES {
        let (_, dn) = ElementKind::Tet10.shape(xi);
        let mut j = [[0.0; 3]; 3];
        for (i, &n) in nodes.iter().enumerate() {
            for a in 0..3 {
                for b in 0..3 {
                    j[a][b] += mesh.nodes[n][a] * dn[i][b];
                }
            }
        }
        let det = j[0][0] * (j[1][1] * j[2][2] - j[1][2] * j[2][1]) - j[0][1] * (j[1][0] * j[2][2] - j[1][2] * j[2][0]) + j[0][2] * (j[1][0] * j[2][1] - j[1][1] * j[2][0]);
        if det < 0.1 * linear {
            return false;
        }
    }
    // The samples alone miss folds between them (about 0.5 % of randomly bent elements pass them yet invert somewhere):
    // the Bernstein bound proves the Jacobian positive over the whole element.
    let xyz: Vec<V3> = nodes.iter().map(|&n| mesh.nodes[n]).collect();
    matches!(crate::meshing::curved::simplex_validity(ElementKind::Tet10, &xyz, 5), Ok(v) if v.verdict == crate::meshing::curved::Verdict::Valid)
}

/// Move the midside node of every boundary edge onto the surface (curved elements), as far as keeps every tetrahedron
/// around it a proper element.
fn curve_boundary_edges(mesh: &mut Mesh, s: &Solid, boundary: &[([usize; 3], usize)], mids: &HashMap<(usize, usize), usize>, tet: &[[usize; 4]], size: f64) {
    let mut around: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (t, v) in tet.iter().enumerate() {
        for i in 0..4 {
            for j in i + 1..4 {
                around.entry(key(v[i], v[j])).or_default().push(t);
            }
        }
    }
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for (f, _) in boundary {
        for k in 0..3 {
            edges.push(key(f[k], f[(k + 1) % 3]));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    for e in edges {
        let m = mids[&e];
        let straight = mesh.nodes[m];
        let target = project(s.sdf, straight, size * 1e-3);
        let travel = sub(target, straight);
        for fraction in [1.0, 0.5, 0.25] {
            mesh.nodes[m] = [straight[0] + fraction * travel[0], straight[1] + fraction * travel[1], straight[2] + fraction * travel[2]];
            let ok = around[&e].iter().all(|&t| tet10_ok(mesh, &mesh.blocks[0].conn[t * 10..t * 10 + 10]));
            if ok {
                break;
            }
            mesh.nodes[m] = straight;
        }
    }
}

fn reentrant_edge(fd: &dyn Fn(V3) -> Vec<FaceHit>, sdf: &dyn Fn(V3) -> f64, y: V3, faces: [usize; 2], eps: f64) -> bool {
    if !eps.is_finite() || eps <= 0.0 { return false; }
    let hits = fd(y);
    let [Some(a), Some(b)] = faces.map(|f| hits.iter().find(|h| h.face == f)) else { return false };
    // `eps` is the surface accuracy (a thousandth of the size), not round-off: the two faces of a tessellated model need not
    // meet exactly at their seam, and the alternating projection stops up to about `eps` off one of them (1e-6 to 1.2e-3 on
    // the t-junction at sizes near 1). A round-off tolerance never proved such a seam, and the snap pulled the wall vertices
    // beside it onto the edge. The sectors are sampled well beyond that gap.
    let tol = eps;
    if !(a.dist.abs() <= tol && b.dist.abs() <= tol) { return false; }
    let (Some(n0), Some(n1)) = (a.surface_normal(y, fd, tol), b.surface_normal(y, fd, tol)) else { return false };
    let c = dot(n0, n1);
    if c.abs() >= 0.9 { return false; }
    let dual0: V3 = std::array::from_fn(|k| (n0[k] - c * n1[k]) / (1.0 - c * c));
    let dual1: V3 = std::array::from_fn(|k| (n1[k] - c * n0[k]) / (1.0 - c * c));
    for (s0, s1) in [(1.0, 1.0), (-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0)] {
        let step = 8.0 * eps;
        let sample = std::array::from_fn(|k| y[k] + step * (s0 * dual0[k] + s1 * dual1[k]));
        let d = sdf(sample);
        if !d.is_finite() || if s0 > 0.0 && s1 > 0.0 { d <= step / 2.0 } else { d >= -step / 2.0 } { return false; }
    }
    true
}

/// Project onto nearby faces, retaining a single patch at a proven reentrant edge.
fn snap_to_features(fd: &dyn Fn(V3) -> Vec<FaceHit>, sdf: &dyn Fn(V3) -> f64, x: V3, edge_reach: f64, reach: f64, eps: f64) -> Option<V3> {
    let hits = fd(x);
    let mut active: Vec<usize> = hits.iter().filter(|h| h.dist.abs() < edge_reach || (h.dist > 0.0 && h.dist < reach)).map(|h| h.face).collect();
    if active.is_empty() {
        let nearest = hits.iter().filter(|h| h.dist.abs() < reach).min_by(|a, b| a.dist.abs().total_cmp(&b.dist.abs()))?;
        active.push(nearest.face);
    }
    let mut y = x;
    for _ in 0..12 {
        let mut moved = 0.0f64;
        for h in fd(y).iter().filter(|h| active.contains(&h.face)) {
            for k in 0..3 {
                y[k] -= h.dist * h.normal[k];
            }
            moved = moved.max(h.dist.abs());
        }
        if moved < 1e-12 {
            break;
        }
    }
    if let [a, b] = active.as_slice() {
        if reentrant_edge(fd, sdf, y, [*a, *b], eps) {
            let nearest = hits.iter().filter(|h| h.face == *a || h.face == *b)
                .min_by(|a, b| a.dist.abs().total_cmp(&b.dist.abs()).then(a.face.cmp(&b.face)))?;
            let foot: V3 = std::array::from_fn(|k| x[k] - nearest.dist * nearest.normal[k]);
            if foot.iter().all(|v| v.is_finite()) && sdf(foot).abs() <= 1e-3 * eps { return Some(foot); }
        }
    }
    Some(y)
}

/// The model face a boundary triangle belongs to: the one every vertex lies on (when the faces can be asked), else the
/// face of its centroid.
fn face_of_triangle(s: &Solid, v: [V3; 3], size: f64) -> usize {
    if let Some(fd) = s.face_dists {
        // Boundary vertices are snapped onto the surface (to within 0.02 of their local size at worst), so "on a face" means
        // a hundredth of the size, not a tenth: a tolerance of 0.1 h gave a tangent fillet's rim strip, sqrt(2 r 0.1 h) wide,
        // to the flat face next to it.
        let tol = FACE_TOL * size;
        let per: Vec<Vec<FaceHit>> = v.iter().map(|x| fd(*x)).collect();
        let mut best: Option<(f64, usize)> = None;
        for h in &per[0] {
            let others: Vec<f64> = per[1..].iter().filter_map(|l| l.iter().find(|o| o.face == h.face).map(|o| o.dist.abs())).collect();
            if others.len() == 2 && h.dist.abs() < tol && others.iter().all(|d| *d < tol) {
                let mut distances = [h.dist.abs(), others[0], others[1]];
                distances.sort_by(f64::total_cmp);
                let cost = distances.into_iter().sum::<f64>();
                if best.is_none_or(|b| cost < b.0 || (cost == b.0 && h.face < b.1)) {
                    best = Some((cost, h.face));
                }
            }
        }
        if let Some((_, f)) = best {
            return f;
        }
    }
    let centre = std::array::from_fn(|k| {
        let mut values = v.map(|p| p[k]);
        values.sort_by(f64::total_cmp);
        let (mut sum, mut correction) = (0.0_f64, 0.0_f64);
        for x in values {
            let next = sum + x;
            correction += if sum.abs() >= x.abs() { (sum - next) + x } else { (x - next) + sum };
            sum = next;
        }
        (sum + correction) / 3.0
    });
    (s.face_of)(centre)
}

#[cfg(test)]
mod tests {
    fn concave_faces(x: V3) -> Vec<FaceHit> {
        [([x[0].max(0.0), x[1], 0.0], [0.0, 0.0, 1.0]),
         ([0.0, x[1], x[2].max(0.0)], [1.0, 0.0, 0.0])]
            .into_iter().enumerate().map(|(face, (q, n))| {
                let offset = sub(x, q);
                let distance = norm(offset);
                let signed = if dot(offset, n) < 0.0 { -distance } else { distance };
                FaceHit { face, dist: signed, normal: if distance == 0.0 { n } else { offset.map(|v| v / signed) } }
            }).collect()
    }

    fn concave_sdf(x: V3) -> f64 {
        let distance = concave_faces(x).iter().map(|h| h.dist.abs()).fold(f64::INFINITY, f64::min);
        if x[0] > 0.0 && x[2] > 0.0 { distance } else { -distance }
    }

    fn legacy_snap(fd: &dyn Fn(V3) -> Vec<FaceHit>, x: V3, edge_reach: f64, reach: f64) -> Option<V3> {
        let hits = fd(x);
        let mut active: Vec<usize> = hits.iter().filter(|h| h.dist.abs() < edge_reach || (h.dist > 0.0 && h.dist < reach)).map(|h| h.face).collect();
        if active.is_empty() {
            let nearest = hits.iter().filter(|h| h.dist.abs() < reach).min_by(|a, b| a.dist.abs().total_cmp(&b.dist.abs()))?;
            active.push(nearest.face);
        }
        let mut y = x;
        for _ in 0..12 {
            let mut moved = 0.0f64;
            for h in fd(y).iter().filter(|h| active.contains(&h.face)) {
                for k in 0..3 {
                    y[k] -= h.dist * h.normal[k];
                }
                moved = moved.max(h.dist.abs());
            }
            if moved < 1e-12 {
                break;
            }
        }
        Some(y)
    }

    #[test]
    fn convex_bore_tangent_single_face_and_corner_snaps_stay_bit_identical() {
        let check = |fd: &dyn Fn(V3) -> Vec<FaceHit>, sdf: &dyn Fn(V3) -> f64, x| {
            let old = legacy_snap(fd, x, 0.125, 0.275);
            let new = snap_to_features(fd, sdf, x, 0.125, 0.275, 0.00025);
            assert_eq!(new.map(|p| p.map(f64::to_bits)), old.map(|p| p.map(f64::to_bits)), "{x:?}");
        };
        let hit = |face, x: V3, q: V3, n: V3| {
            let offset = sub(x, q);
            let distance = norm(offset);
            let signed = if dot(offset, n) < 0.0 { -distance } else { distance };
            FaceHit { face, dist: signed, normal: if distance == 0.0 { n } else { offset.map(|v| v / signed) } }
        };
        let convex = |x: V3| vec![
            hit(0, x, [x[0].min(0.0), x[1], 0.0], [0.0,0.0,1.0]),
            hit(1, x, [0.0,x[1],x[2].min(0.0)], [1.0,0.0,0.0]),
        ];
        let convex_sdf = |x: V3| norm([x[0].max(0.0),0.0,x[2].max(0.0)]) + x[0].max(x[2]).min(0.0);
        for x in [[-0.02,0.0,0.03],[0.03,0.0,0.02],[-0.02,0.0,-0.03]] { check(&convex, &convex_sdf, x); }
        let bore = |x: V3| {
            let r = x[0].hypot(x[1]);
            let n = [x[0]/r,x[1]/r,0.0];
            let cap = if r < 1.0 { [n[0],n[1],1.0] } else { [x[0],x[1],1.0] };
            vec![hit(0,x,[n[0],n[1],x[2].min(1.0)],[-n[0],-n[1],0.0]), hit(1,x,cap,[0.0,0.0,1.0])]
        };
        let bore_sdf = |x: V3| {
            let (a,b) = (1.0-x[0].hypot(x[1]),x[2]-1.0);
            a.max(0.0).hypot(b.max(0.0)) + a.max(b).min(0.0)
        };
        for x in [[0.98,0.0,0.98],[1.02,0.0,0.98],[1.02,0.0,1.02]] { check(&bore, &bore_sdf, x); }
        let normals = [[1.0,0.0,0.0], [0.1_f64.cos(),0.1_f64.sin(),0.0]];
        let tangent = |x: V3| normals.iter().enumerate().map(|(face,&normal)| FaceHit { face, dist: dot(x,normal), normal }).collect();
        let tangent_sdf = |x: V3| normals.iter().map(|&n|dot(x,n)).fold(f64::NEG_INFINITY,f64::max);
        check(&tangent, &tangent_sdf, [0.02,0.03,0.0]);
        let sphere = |x: V3| { let r=norm(x); vec![FaceHit { face:0, dist:r-1.0, normal:x.map(|v|v/r) }] };
        check(&sphere, &|x| norm(x)-1.0, [1.02,0.03,0.04]);
        let corner = |x: V3| (0..3).map(|face| FaceHit { face, dist:x[face], normal:std::array::from_fn(|k| if k==face {1.0} else {0.0}) }).collect();
        check(&corner, &|x|x.into_iter().fold(f64::NEG_INFINITY,f64::max), [0.02,0.03,0.04]);
        check(&concave_faces, &|_|0.0, [0.02,0.0,0.03]);
    }

    #[test]
    fn reentrant_snap_selects_the_nearest_trimmed_patch_deterministically() {
        let reversed = |x| { let mut h = concave_faces(x); h.reverse(); h };
        for fd in [&concave_faces as &dyn Fn(V3) -> Vec<FaceHit>, &reversed] {
            for (x, expected) in [
                ([0.0, 0.0, 0.05], [0.0, 0.0, 0.05]),
                ([0.0, 0.0, 0.2], [0.0, 0.0, 0.2]),
                ([0.02, 0.0, 0.03], [0.0, 0.0, 0.03]),
                ([0.03, 0.0, 0.03], [0.03, 0.0, 0.0]),
                ([-0.03, 0.0, 0.02], [0.0, 0.0, 0.02]),
                ([-0.02, 0.0, -0.03], [0.0, 0.0, 0.0]),
            ] {
                let y = snap_to_features(fd, &concave_sdf, x, 0.125, 0.275, 0.00025).unwrap();
                assert!(norm(sub(y, expected)) < 1e-12, "{x:?} -> {y:?}, expected {expected:?}");
                assert!(concave_sdf(y).abs() < 1e-12);
                let again = snap_to_features(fd, &concave_sdf, y, 0.125, 0.275, 0.00025).unwrap();
                assert!(norm(sub(again, y)) < 1e-12);
            }
        }
    }

    #[test]
    fn reentrant_sector_proof_uses_oblique_dual_directions_and_refuses_ambiguity() {
        let normals = [[1.0, 0.0, 0.0], [0.5, 0.0, 0.75_f64.sqrt()]];
        let fd = |x: V3| normals.iter().enumerate().map(|(face, &n)| {
            let signed_plane = dot(x, n);
            let mut q: V3 = std::array::from_fn(|k| x[k] - signed_plane * n[k]);
            if dot(q, normals[1 - face]) < 0.0 { q = [0.0, x[1], 0.0]; }
            let offset = sub(x, q);
            let distance = norm(offset);
            let signed = if signed_plane < 0.0 { -distance } else { distance };
            FaceHit { face, dist: signed, normal: if distance == 0.0 { n } else { offset.map(|v| v / signed) } }
        }).collect::<Vec<_>>();
        let sdf = |x: V3| {
            let d = fd(x).iter().map(|h| h.dist.abs()).fold(f64::INFINITY, f64::min);
            if normals.iter().all(|&n| dot(x, n) > 0.0) { d } else { -d }
        };
        let x = [0.0, 0.0, 0.05];
        assert!(reentrant_edge(&fd, &sdf, [0.0; 3], [0, 1], 0.00025));
        let y = snap_to_features(&fd, &sdf, x, 0.125, 0.275, 0.00025).unwrap();
        assert!(norm(sub(x, y)) < 1e-12);
        let reversed = |x| { let mut hits = fd(x); hits.reverse(); hits };
        let y = snap_to_features(&reversed, &sdf, x, 0.125, 0.275, 0.00025).unwrap();
        assert!(norm(sub(x, y)) < 1e-12);
        assert!(!reentrant_edge(&fd, &|_| 0.0, [0.0; 3], [0, 1], 0.00025));
        assert!(!reentrant_edge(&fd, &|_| f64::NAN, [0.0; 3], [0, 1], 0.00025));
        assert!(!reentrant_edge(&fd, &sdf, [0.01; 3], [0, 1], 0.00025));
    }

    /// A tessellated model's two faces need not meet exactly at their seam: the alternating projection onto both stops a
    /// fraction of the tessellation tolerance off one of them. The reentrant proof must still hold there, or a wall vertex next
    /// to the edge is pulled onto it (on the t-junction this left 5 / 7 elements under the gate).
    #[test]
    fn reentrant_snap_holds_where_the_two_faces_miss_each_other_by_a_tessellation_gap() {
        let gap = 1e-4;
        let faces = |x: V3, gap: f64| -> Vec<FaceHit> {
            [([x[0].max(0.0), x[1], 0.0], [0.0, 0.0, 1.0]), ([0.0, x[1], x[2].max(gap)], [1.0, 0.0, 0.0])]
                .into_iter().enumerate().map(|(face, (q, n))| {
                    let offset = sub(x, q);
                    let distance = norm(offset);
                    let signed = if dot(offset, n) < 0.0 { -distance } else { distance };
                    FaceHit { face, dist: signed, normal: if distance == 0.0 { n } else { offset.map(|v| v / signed) } }
                }).collect()
        };
        let fd = |x: V3| faces(x, gap);
        let sdf = |x: V3| {
            let distance = fd(x).iter().map(|h| h.dist.abs()).fold(f64::INFINITY, f64::min);
            if x[0] > 0.0 && x[2] > 0.0 { distance } else { -distance }
        };
        for (x, expected) in [([0.0, 0.0, 0.05], [0.0, 0.0, 0.05]), ([0.02, 0.0, 0.03], [0.0, 0.0, 0.03]), ([0.03, 0.0, 0.02], [0.03, 0.0, 0.0])] {
            let y = snap_to_features(&fd, &sdf, x, 0.125, 0.275, 0.00025).unwrap();
            assert!(norm(sub(y, expected)) < 1e-12, "{x:?} -> {y:?}, expected {expected:?}");
        }
        // a gap wider than the tolerance is no seam: the edge is not proven and the wall point is still projected towards it
        let wide = |x: V3| faces(x, 0.01);
        let y = snap_to_features(&wide, &sdf, [0.0, 0.0, 0.05], 0.125, 0.275, 0.00025).unwrap();
        assert!(norm(y) < 0.011, "{y:?}");
    }

    #[test]
    fn reentrant_snap_preserves_a_point_already_on_the_wall() {
        let x = [0.0, 0.0, 0.05];
        assert_eq!(snap_to_features(&concave_faces, &concave_sdf, x, 0.125, 0.275, 0.00025), Some(x));
    }

    use super::*;
    use crate::meshing::curved::{simplex_validity, Verdict};

    #[test]
    fn pinched_sectors_keep_the_largest_fan() {
        let p = [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.5], [1.0, 1.0, 0.5], [-1.0, 0.0, 0.5], [-2.0, -2.0, 0.5], [0.0, 3.0, 3.0], [1.0, 3.0, 3.0], [0.0, 4.0, 3.0]];
        // two sectors meeting only along the edge 0-1: the larger (cell 1) survives
        let mut cells = vec![[0, 1, 2, 3], [0, 1, 4, 5]];
        assert_eq!(drop_pinched_sectors(&p, &mut cells), 1);
        assert_eq!(cells, vec![[0, 1, 4, 5]]);
        // two sectors meeting only at vertex 1
        let mut cells = vec![[0, 1, 2, 3], [1, 6, 7, 8]];
        assert_eq!(drop_pinched_sectors(&p, &mut cells), 1);
        assert_eq!(cells.len(), 1);
        // a face-connected pair is a manifold fan and stays
        let mut cells = vec![[0, 1, 2, 3], [0, 1, 3, 4]];
        assert_eq!(drop_pinched_sectors(&p, &mut cells), 0);
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn face_tagging_is_invariant_under_vertex_and_hit_order_at_near_ties() {
        let vertices = [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 0.0, 0.0]];
        for tie in [false, true] {
            let fd = |p: V3| {
                let scale = 2_f64.powi(-10);
                let small = 2_f64.powi(-63);
                let values = [[scale, small, small], [scale, 0.0, small]];
                let mut hits: Vec<_> = (0..2).map(|face| FaceHit {
                    face, dist: if tie { 0.0 } else { values[face][p[0] as usize] }, normal: [0.0, 0.0, 1.0],
                }).collect();
                if p[0] == 0.0 { hits.reverse(); }
                hits
            };
            let sdf = |p: V3| p[2];
            let size = |_: V3| 1.0;
            let face = |_: V3| 0;
            let solid = Solid { lo: [-2.0; 3], hi: [2.0; 3], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&fd) };
            for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
                assert_eq!(face_of_triangle(&solid, order.map(|i| vertices[i]), 0.25), usize::from(!tie));
            }
        }
    }

    #[test]
    fn common_vertex_face_tags_do_not_follow_a_nearer_centroid_patch() {
        let fd = |p: V3| {
            let r = p[0].hypot(p[1]);
            let direction = if r > 0.0 { [p[0] / r, p[1] / r] } else { [1.0, 0.0] };
            [(0, r.max(0.5)), (1, r.min(0.5))].map(|(face, radius)| {
                let off = [p[0] - radius * direction[0], p[1] - radius * direction[1], p[2]];
                let d = (off[0].powi(2) + off[1].powi(2) + off[2].powi(2)).sqrt();
                let side = if p[2] < 0.0 { -1.0 } else { 1.0 };
                FaceHit { face, dist: side * d, normal: if d > 1e-12 { off.map(|x| side * x / d) } else { [0.0, 0.0, 1.0] } }
            }).to_vec()
        };
        let sdf = |p: V3| p[2];
        let size = |_: V3| 1.0;
        let face = |p: V3| usize::from(p[0].hypot(p[1]) < 0.5);
        let solid = Solid { lo: [-2.0; 3], hi: [2.0; 3], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&fd) };
        let vertices = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
        let centre = [0.0, -1.0 / 3.0, 0.0];
        assert_eq!(face(centre), 1);
        for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            assert_eq!(face_of_triangle(&solid, order.map(|i| vertices[i]), 0.25), 0);
        }
        let fallback = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        assert_eq!(face_of_triangle(&solid, fallback, 0.25), 1);
    }

    #[test]
    fn fallback_face_tags_use_one_accurate_centroid_for_every_corner_order() {
        let vertices = [[1e16, 0.0, 0.0], [1.0, 1.0, 0.0], [-1e16, 0.0, 0.0]];
        let fd = |_: V3| Vec::new();
        let sdf = |p: V3| p[2];
        let size = |_: V3| 1.0;
        let face = |p: V3| {
            assert_eq!(p[0], 1.0 / 3.0);
            assert_eq!(p[1], 1.0 / 3.0);
            1
        };
        let solid = Solid { lo: [-1e16; 3], hi: [1e16; 3], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&fd) };
        for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            assert_eq!(face_of_triangle(&solid, order.map(|i| vertices[i]), 0.25), 1);
        }
    }

    /// A bent Tet10 that the eleven-point Jacobian test accepts although its mapping folds between the samples is
    /// refused by `tet10_ok` (found by random search; about one in two hundred heavily bent elements is such a case).
    #[test]
    fn a_fold_between_the_jacobian_samples_is_still_refused() {
        let corners: [V3; 4] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let mut s = 99u64;
        let mut next = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / (1u64 << 30) as f64 - 1.
        };
        let mut checked = 0;
        for _ in 0..5000 {
            let mut nodes: Vec<V3> = corners.to_vec();
            for (a, b) in [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)] {
                nodes.push(mid(corners[a], corners[b]));
            }
            for node in nodes.iter_mut().skip(4) {
                for x in node.iter_mut() {
                    *x += 0.7 * next();
                }
            }
            let mut mesh = Mesh::new(Physics::Solid);
            mesh.nodes = nodes.clone();
            let ids: Vec<usize> = (0..10).collect();
            // Replay the sampled part of `tet10_ok` on its own.
            let sampled = SAMPLES.iter().all(|xi| {
                let (_, dn) = ElementKind::Tet10.shape(*xi);
                let mut j = [[0.0; 3]; 3];
                for (i, &n) in ids.iter().enumerate() {
                    for a in 0..3 {
                        for b in 0..3 {
                            j[a][b] += mesh.nodes[n][a] * dn[i][b];
                        }
                    }
                }
                let det = j[0][0] * (j[1][1] * j[2][2] - j[1][2] * j[2][1]) - j[0][1] * (j[1][0] * j[2][2] - j[1][2] * j[2][0]) + j[0][2] * (j[1][0] * j[2][1] - j[1][1] * j[2][0]);
                det >= 0.1
            });
            let proved = simplex_validity(ElementKind::Tet10, &nodes, 6).unwrap().verdict;
            if sampled && proved == Verdict::Invalid {
                assert!(!tet10_ok(&mesh, &ids), "the rigorous check must refuse a folded element");
                checked += 1;
            }
            if proved == Verdict::Valid && sampled {
                assert!(tet10_ok(&mesh, &ids) || simplex_validity(ElementKind::Tet10, &nodes, 5).unwrap().verdict != Verdict::Valid);
            }
        }
        assert!(checked >= 3, "the search must find sample-blind elements: {checked}");
    }
}
