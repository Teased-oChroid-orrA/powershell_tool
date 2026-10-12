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
    /// Optional: for a point, the faces it is close to with the signed distance to each (positive outside the solid)
    /// and the face's outward unit normal there. With it, a vertex near two or three faces is snapped onto their edge or
    /// corner instead of rounding the feature off, and boundary triangles are assigned to the face all their vertices
    /// lie on. A B-rep kernel answers it from the distance to each face.
    pub face_dists: Option<&'a dyn Fn([f64; 3]) -> Vec<FaceHit>>,
}

/// One face near a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceHit {
    pub face: usize,
    /// Signed distance to the face (positive outside the solid).
    pub dist: f64,
    /// Outward unit normal of the face at the nearest point.
    pub normal: [f64; 3],
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
}
impl Default for TetOptions {
    fn default() -> Self {
        Self { base: 1.0, boundary_size: 0.25, max_tets: 2_000_000, quadratic: false, snap: true }
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

/// Mesh `s` into tetrahedra with material `mat`.
pub fn mesh_solid(s: &Solid, mat: Elastic, opts: TetOptions) -> Result<(Mesh, MeshReport), String> {
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
    let kept: Vec<[usize; 4]> = (0..r.tets.len()).filter(|&t| r.alive[t] && (s.sdf)(r.centroid(t)) < 0.0).map(|t| r.tets[t]).collect();
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

    // Boundary faces: faces of exactly one tetrahedron.
    let faces_of = |v: [usize; 4]| -> [([usize; 3], usize); 4] { [([v[1], v[2], v[3]], v[0]), ([v[0], v[3], v[2]], v[1]), ([v[0], v[1], v[3]], v[2]), ([v[0], v[2], v[1]], v[3])] };
    let mut count: HashMap<[usize; 3], (usize, [usize; 3], usize)> = HashMap::new();
    for v in &tet {
        for (f, opposite) in faces_of(*v) {
            let mut k = f;
            k.sort_unstable();
            let e = count.entry(k).or_insert((0, f, opposite));
            e.0 += 1;
        }
    }
    let mut boundary: Vec<([usize; 3], usize)> = count.values().filter(|c| c.0 == 1).map(|c| (c.1, c.2)).collect();
    boundary.sort();
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
        let local: Vec<f64> = (0..p.len()).map(|v| neighbours[v].iter().map(|&u| norm(sub(p[u], p[v]))).fold(0.0, f64::max).min(opts.boundary_size)).collect();
        let mut order: Vec<usize> = (0..p.len()).filter(|&v| on_boundary[v]).collect();
        order.sort_by(|&a, &b| (s.sdf)(p[a]).abs().total_cmp(&(s.sdf)(p[b]).abs()));
        for _pass in 0..SNAP_PASSES {
            let mut moved_any = false;
            for &v in &order {
                let (reach, eps) = (1.1 * local[v], local[v] * 1e-3);
                if (s.sdf)(p[v]).abs() > reach {
                    continue;
                }
                let target = match s.face_dists {
                    Some(fd) => snap_to_features(fd, p[v], 0.5 * local[v], reach).unwrap_or_else(|| project(s.sdf, p[v], eps)),
                    None => project(s.sdf, p[v], eps),
                };
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
        // What ended on the surface and what did not.
        for &v in &order {
            if (s.sdf)(p[v]).abs() <= 0.02 * local[v] {
                snapped += 1;
            } else {
                refused += 1;
            }
        }
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
    Ok((mesh, report))
}

fn faces_of_named(boundary: &[([usize; 3], usize)], p: &[V3], s: &Solid, mids: Option<&HashMap<(usize, usize), usize>>, size: f64) -> Vec<(String, Vec<Vec<usize>>)> {
    let mut by_face: std::collections::BTreeMap<usize, Vec<Vec<usize>>> = std::collections::BTreeMap::new();
    for &(f, opposite) in boundary {
        let [a, b, c] = f;
        // Outward: the normal must point away from the tetrahedron's fourth vertex.
        let n = cross(sub(p[b], p[a]), sub(p[c], p[a]));
        let (b, c) = if dot(n, sub(p[opposite], p[a])) > 0.0 { (c, b) } else { (b, c) };
        let centre = [(p[a][0] + p[b][0] + p[c][0]) / 3.0, (p[a][1] + p[b][1] + p[c][1]) / 3.0, (p[a][2] + p[b][2] + p[c][2]) / 3.0];
        let mut nodes = vec![a, b, c];
        if let Some(m) = mids {
            nodes.extend([m[&key(a, b)], m[&key(b, c)], m[&key(c, a)]]);
        }
        by_face.entry(face_of_triangle(s, [p[a], p[b], p[c]], centre, size)).or_default().push(nodes);
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
    true
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

/// Snap `x` onto the face, edge or corner it is close to: faces within `edge_reach` all count (their intersection is
/// the target), otherwise the nearest face within `reach`. `None` when no face is that close.
fn snap_to_features(fd: &dyn Fn(V3) -> Vec<FaceHit>, x: V3, edge_reach: f64, reach: f64) -> Option<V3> {
    let hits = fd(x);
    let mut active: Vec<usize> = hits.iter().filter(|h| h.dist.abs() < edge_reach).map(|h| h.face).collect();
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

/// The model face a boundary triangle belongs to: the one every vertex lies on (when the faces can be asked), else the
/// face of its centroid.
fn face_of_triangle(s: &Solid, v: [V3; 3], centre: V3, size: f64) -> usize {
    if let Some(fd) = s.face_dists {
        let tol = 0.1 * size;
        let per: Vec<Vec<FaceHit>> = v.iter().map(|x| fd(*x)).collect();
        let mut best: Option<(f64, usize)> = None;
        for h in &per[0] {
            let others: Vec<f64> = per[1..].iter().filter_map(|l| l.iter().find(|o| o.face == h.face).map(|o| o.dist.abs())).collect();
            if others.len() == 2 && h.dist.abs() < tol && others.iter().all(|d| *d < tol) {
                let cost = h.dist.abs() + others.iter().sum::<f64>();
                if best.is_none_or(|b| cost < b.0) {
                    best = Some((cost, h.face));
                }
            }
        }
        if let Some((_, f)) = best {
            return f;
        }
    }
    (s.face_of)(centre)
}
