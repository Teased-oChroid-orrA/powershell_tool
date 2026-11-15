//! Quality triangulation of a [`Region`]: incremental Delaunay insertion with exact predicates,
//! boundary subsegments conformed by splitting encroached segments at their curve midpoints, exterior
//! and hole triangles removed by crossing parity, then Ruppert refinement (circumcentre insertion for
//! triangles with a large radius-edge ratio or larger than the size function asks for).
//!
//! Boundary vertices lie exactly on the geometry (arc midpoints are evaluated on the circle), and
//! every boundary subsegment remembers its curve and parameter interval so that quadratic elements
//! can place their midside nodes on the true boundary (`mesh2d.rs`).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::geometry::{Curve, Region};
use robust::{incircle, orient2d, Coord};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
struct Tri {
    v: [u32; 3],
    /// `n[i]` is the neighbour across the edge opposite `v[i]`.
    n: [u32; 3],
    alive: bool,
}

#[derive(Debug, Clone, Copy)]
struct Sub {
    a: u32,
    b: u32,
    seg: u32,
    /// Curve parameters at `a` and `b`.
    t0: f64,
    t1: f64,
    alive: bool,
}

/// Meshing controls.
#[derive(Debug, Clone, Copy)]
pub struct MeshOptions {
    /// Largest accepted circumradius / shortest-edge ratio (1.2 guarantees ~25 degree angles away
    /// from small input angles; `1/(2 sin(min angle))`).
    pub max_ratio: f64,
    /// A triangle is too big when its circumradius exceeds `size_factor * h(centroid)`; an
    /// equilateral triangle of side `h` has circumradius `0.577 h`.
    pub size_factor: f64,
    /// Arcs are split until each piece sweeps at most this angle (radians).
    pub max_arc_angle: f64,
    /// No new vertex is created closer than this to the nearest edge of a triangle being refined
    /// (guards against infinite refinement at tiny input angles). `0` = off.
    pub min_edge: f64,
    pub max_vertices: usize,
}

impl Default for MeshOptions {
    fn default() -> Self {
        Self { max_ratio: 1.2, size_factor: 0.66, max_arc_angle: 0.35, min_edge: 0.0, max_vertices: 2_000_000 }
    }
}

/// A boundary edge of the output mesh, oriented so the domain is on its left.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryEdge {
    pub a: usize,
    pub b: usize,
    /// Flat segment index: outer loop segments first, then each hole's.
    pub seg: usize,
    /// Curve parameters at `a` and `b`.
    pub t_a: f64,
    pub t_b: f64,
}

#[derive(Debug, Clone)]
pub struct TriMesh {
    pub pts: Vec<[f64; 2]>,
    /// Counter-clockwise vertex triples.
    pub tris: Vec<[usize; 3]>,
    pub boundary: Vec<BoundaryEdge>,
    /// Curve and name of every flat segment index.
    pub segments: Vec<(Curve, String)>,
}

struct Builder<'a> {
    pts: Vec<[f64; 2]>,
    tris: Vec<Tri>,
    free: Vec<u32>,
    /// A live triangle incident to each vertex.
    vt: Vec<u32>,
    subs: Vec<Sub>,
    sub_of: HashMap<(u32, u32), u32>,
    segs: Vec<(Curve, String)>,
    size: &'a dyn Fn([f64; 2]) -> f64,
    opt: MeshOptions,
    last: u32,
    rng: u64,
    /// Triangles created or changed since the last `drain_touched`.
    touched: Vec<u32>,
    /// Changed triangles awaiting re-scoring by the refinement loop.
    dirty: Vec<u32>,
}

fn key(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    orient2d(Coord { x: a[0], y: a[1] }, Coord { x: b[0], y: b[1] }, Coord { x: c[0], y: c[1] })
}

fn in_circle(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    incircle(Coord { x: a[0], y: a[1] }, Coord { x: b[0], y: b[1] }, Coord { x: c[0], y: c[1] }, Coord { x: d[0], y: d[1] })
}

enum Loc {
    Inside(u32),
    OnEdge(u32, usize),
    OnVertex(u32),
    /// The walk left the triangulation through edge `i` of triangle `t`.
    Outside(u32, usize),
}

#[derive(PartialEq)]
struct Bad(f64, u32);
impl Eq for Bad {}
impl PartialOrd for Bad {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Bad {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then(self.1.cmp(&o.1))
    }
}

impl<'a> Builder<'a> {
    fn p(&self, v: u32) -> [f64; 2] {
        self.pts[v as usize]
    }

    fn tri(&self, t: u32) -> &Tri {
        &self.tris[t as usize]
    }

    fn new_tri(&mut self, v: [u32; 3], n: [u32; 3]) -> u32 {
        let tri = Tri { v, n, alive: true };
        if let Some(id) = self.free.pop() {
            self.tris[id as usize] = tri;
            id
        } else {
            self.tris.push(tri);
            (self.tris.len() - 1) as u32
        }
    }

    fn set_tri(&mut self, t: u32, v: [u32; 3], n: [u32; 3]) {
        self.tris[t as usize] = Tri { v, n, alive: true };
        for &x in &v {
            self.vt[x as usize] = t;
        }
        self.touched.push(t);
    }

    fn index_of(&self, t: u32, v: u32) -> usize {
        self.tri(t).v.iter().position(|&x| x == v).expect("vertex in triangle")
    }

    fn replace_neighbor(&mut self, t: u32, old: u32, new: u32) {
        if t == NONE {
            return;
        }
        let tr = &mut self.tris[t as usize];
        for k in 0..3 {
            if tr.n[k] == old {
                tr.n[k] = new;
                return;
            }
        }
        unreachable!("neighbour link {old} not found in triangle {t}");
    }

    fn is_sub(&self, a: u32, b: u32) -> bool {
        self.sub_of.contains_key(&key(a, b))
    }

    fn next_rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    // ------------------------------------------------------------------ location

    fn locate(&mut self, p: [f64; 2], start: u32) -> Loc {
        let mut cur = if self.tri(start).alive { start } else { self.last };
        for _ in 0..(self.tris.len() * 2 + 16) {
            let tr = *self.tri(cur);
            let first = (self.next_rand() % 3) as usize;
            let mut zeros = [false; 3];
            let mut moved = false;
            for k in 0..3 {
                let i = (first + k) % 3;
                let o = orient(self.p(tr.v[(i + 1) % 3]), self.p(tr.v[(i + 2) % 3]), p);
                if o < 0.0 {
                    if tr.n[i] == NONE {
                        return Loc::Outside(cur, i);
                    }
                    cur = tr.n[i];
                    moved = true;
                    break;
                }
                zeros[i] = o == 0.0;
            }
            if moved {
                continue;
            }
            self.last = cur;
            return match zeros.iter().filter(|z| **z).count() {
                0 => Loc::Inside(cur),
                1 => Loc::OnEdge(cur, zeros.iter().position(|z| *z).unwrap()),
                _ => {
                    // On two edges: the vertex they share (the index whose own edge is not zero).
                    let k = zeros.iter().position(|z| !*z).unwrap();
                    Loc::OnVertex(tr.v[k])
                }
            };
        }
        // Degenerate walk (cycling): fall back to a scan.
        for t in 0..self.tris.len() as u32 {
            let tr = self.tri(t);
            if tr.alive && (0..3).all(|i| orient(self.p(tr.v[(i + 1) % 3]), self.p(tr.v[(i + 2) % 3]), p) >= 0.0) {
                self.last = t;
                return Loc::Inside(t);
            }
        }
        Loc::Outside(self.last, 0)
    }

    /// Live triangles around vertex `a` (both directions, so boundary vertices work).
    fn fan(&self, a: u32) -> Vec<u32> {
        let start = self.vt[a as usize];
        let mut out = vec![start];
        for dir in [2usize, 1] {
            let mut t = start;
            loop {
                let k = self.index_of(t, a);
                let nb = self.tri(t).n[(k + dir) % 3];
                if nb == NONE {
                    break;
                }
                if nb == start {
                    return out; // a full circle around an interior vertex
                }
                out.push(nb);
                t = nb;
            }
        }
        out
    }

    /// `(triangle, index of the vertex opposite)` of a triangle that has the edge `a-b`.
    fn find_edge(&self, a: u32, b: u32) -> Option<(u32, usize)> {
        for t in self.fan(a) {
            let tr = self.tri(t);
            for i in 0..3 {
                let (x, y) = (tr.v[(i + 1) % 3], tr.v[(i + 2) % 3]);
                if (x == a && y == b) || (x == b && y == a) {
                    return Some((t, i));
                }
            }
        }
        None
    }

    // ------------------------------------------------------------------ insertion

    fn add_vertex(&mut self, p: [f64; 2]) -> u32 {
        self.pts.push(p);
        self.vt.push(NONE);
        (self.pts.len() - 1) as u32
    }

    /// Insert a point; returns its vertex (the existing one for a duplicate).
    fn insert(&mut self, p: [f64; 2], hint: u32) -> Result<u32, String> {
        match self.locate(p, hint) {
            Loc::Inside(t) => {
                let v = self.add_vertex(p);
                self.split_tri(t, v);
                Ok(v)
            }
            Loc::OnEdge(t, i) => {
                let v = self.add_vertex(p);
                self.split_edge(t, i, v);
                Ok(v)
            }
            Loc::OnVertex(v) => Ok(v),
            Loc::Outside(..) => Err("point outside the triangulation".into()),
        }
    }

    fn split_tri(&mut self, t: u32, p: u32) {
        let tr = *self.tri(t);
        let (a, b, c) = (tr.v[0], tr.v[1], tr.v[2]);
        let (n0, n1, n2) = (tr.n[0], tr.n[1], tr.n[2]);
        let tb = self.new_tri([b, c, p], [NONE; 3]);
        let tc = self.new_tri([c, a, p], [NONE; 3]);
        self.set_tri(t, [a, b, p], [tb, tc, n2]);
        self.set_tri(tb, [b, c, p], [tc, t, n0]);
        self.set_tri(tc, [c, a, p], [t, tb, n1]);
        self.replace_neighbor(n0, t, tb);
        self.replace_neighbor(n1, t, tc);
        self.legalize(vec![(t, 2), (tb, 2), (tc, 2)]);
    }

    /// Split the edge opposite `v[i]` of `t` at the new vertex `m`.
    fn split_edge(&mut self, t: u32, i: usize, m: u32) {
        let tr = *self.tri(t);
        let (c, a, b) = (tr.v[i], tr.v[(i + 1) % 3], tr.v[(i + 2) % 3]);
        let (ta, tb) = (tr.n[(i + 1) % 3], tr.n[(i + 2) % 3]);
        let u = tr.n[i];
        let t2 = self.new_tri([c, m, b], [NONE; 3]);
        let mut stack = Vec::with_capacity(4);
        if u == NONE {
            self.set_tri(t, [c, a, m], [NONE, t2, tb]);
            self.set_tri(t2, [c, m, b], [NONE, ta, t]);
            self.replace_neighbor(ta, t, t2);
            stack.extend([(t, 2), (t2, 1)]);
        } else {
            let ur = *self.tri(u);
            let j = ur.n.iter().position(|&x| x == t).expect("neighbour link");
            let d = ur.v[j];
            // u = (d, b, a): neighbours opposite b (edge a-d) and opposite a (edge d-b).
            let (ub, ua) = (ur.n[(j + 1) % 3], ur.n[(j + 2) % 3]);
            let u2 = self.new_tri([d, m, a], [NONE; 3]);
            self.set_tri(t, [c, a, m], [u2, t2, tb]);
            self.set_tri(t2, [c, m, b], [u, ta, t]);
            self.set_tri(u, [d, b, m], [t2, u2, ua]);
            self.set_tri(u2, [d, m, a], [t, ub, u]);
            self.replace_neighbor(ta, t, t2);
            self.replace_neighbor(ub, u, u2);
            stack.extend([(t, 2), (t2, 1), (u, 2), (u2, 1)]);
        }
        self.legalize(stack);
    }

    /// Lawson flips from the edges opposite a newly inserted vertex (`(triangle, index of that vertex)`).
    fn legalize(&mut self, mut stack: Vec<(u32, usize)>) {
        while let Some((t, i)) = stack.pop() {
            if !self.tri(t).alive {
                continue;
            }
            let tr = *self.tri(t);
            let u = tr.n[i];
            if u == NONE {
                continue;
            }
            let (p, a, b) = (tr.v[i], tr.v[(i + 1) % 3], tr.v[(i + 2) % 3]);
            if self.is_sub(a, b) {
                continue;
            }
            let ur = *self.tri(u);
            let j = ur.n.iter().position(|&x| x == t).expect("neighbour link");
            let q = ur.v[j];
            if in_circle(self.p(p), self.p(a), self.p(b), self.p(q)) <= 0.0 {
                continue;
            }
            // Flip a-b to p-q.
            let (x1, x2) = (tr.n[(i + 1) % 3], tr.n[(i + 2) % 3]);
            let (y1, y2) = (ur.n[(j + 1) % 3], ur.n[(j + 2) % 3]);
            self.set_tri(t, [p, a, q], [y1, u, x2]);
            self.set_tri(u, [p, q, b], [y2, x1, t]);
            self.replace_neighbor(y1, u, t);
            self.replace_neighbor(x1, t, u);
            stack.push((t, 0));
            stack.push((u, 0));
        }
    }

    // ------------------------------------------------------------------ subsegments

    fn add_sub(&mut self, a: u32, b: u32, seg: u32, t0: f64, t1: f64) -> u32 {
        let id = self.subs.len() as u32;
        self.subs.push(Sub { a, b, seg, t0, t1, alive: true });
        self.sub_of.insert(key(a, b), id);
        id
    }

    /// Whether subsegment `s` must be split: missing from the triangulation, encroached by the apex
    /// of an adjacent triangle, or sweeping too wide an arc.
    fn sub_needs_split(&self, s: u32) -> bool {
        let sb = self.subs[s as usize];
        if !sb.alive {
            return false;
        }
        let curve = &self.segs[sb.seg as usize].0;
        if curve.sweep() * (sb.t1 - sb.t0).abs() > self.opt.max_arc_angle {
            return true;
        }
        let Some((t, i)) = self.find_edge(sb.a, sb.b) else { return true };
        let (pa, pb) = (self.p(sb.a), self.p(sb.b));
        let tr = self.tri(t);
        let mut apexes = vec![tr.v[i]];
        if tr.n[i] != NONE {
            let ur = self.tri(tr.n[i]);
            let j = ur.n.iter().position(|&x| x == t).unwrap();
            apexes.push(ur.v[j]);
        }
        apexes.into_iter().any(|c| {
            let pc = self.p(c);
            (pa[0] - pc[0]) * (pb[0] - pc[0]) + (pa[1] - pc[1]) * (pb[1] - pc[1]) < 0.0
        })
    }

    /// Split subsegment `s` at the midpoint of its curve; returns the two new subsegments.
    fn split_sub(&mut self, s: u32) -> Result<(u32, u32), String> {
        let sb = self.subs[s as usize];
        let tm = 0.5 * (sb.t0 + sb.t1);
        let curve = self.segs[sb.seg as usize].0;
        let (pa, pb) = (self.p(sb.a), self.p(sb.b));
        // A straight piece splits at the midpoint of the actual chord (keeps the vertex on the edge
        // to rounding); an arc piece at the curve point.
        let m = if matches!(curve, Curve::Line { .. }) { [0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1])] } else { curve.point(tm) };
        self.subs[s as usize].alive = false;
        self.sub_of.remove(&key(sb.a, sb.b));
        let mv = match self.find_edge(sb.a, sb.b) {
            Some((t, i)) => {
                let v = self.add_vertex(m);
                self.split_edge(t, i, v);
                v
            }
            None => {
                let hint = self.last;
                self.insert(m, hint)?
            }
        };
        let s1 = self.add_sub(sb.a, mv, sb.seg, sb.t0, tm);
        let s2 = self.add_sub(mv, sb.b, sb.seg, tm, sb.t1);
        Ok((s1, s2))
    }

    /// Move the changed triangles to `dirty` and queue the subsegments on their edges (their
    /// adjacent apexes changed, so they may have become encroached).
    fn drain_touched(&mut self, queue: &mut Vec<u32>) {
        for t in std::mem::take(&mut self.touched) {
            let tr = *self.tri(t);
            if !tr.alive {
                continue;
            }
            self.dirty.push(t);
            for i in 0..3 {
                if let Some(&s) = self.sub_of.get(&key(tr.v[(i + 1) % 3], tr.v[(i + 2) % 3])) {
                    queue.push(s);
                }
            }
        }
    }

    /// Split every subsegment in `queue` (and those the splits endanger) until none needs it.
    fn conform(&mut self, queue: &mut Vec<u32>) -> Result<(), String> {
        self.drain_touched(queue);
        let mut splits = 0usize;
        while let Some(s) = queue.pop() {
            if self.pts.len() > self.opt.max_vertices {
                return Err(format!("mesh exceeds {} vertices (size function too small?)", self.opt.max_vertices));
            }
            if !self.sub_needs_split(s) {
                continue;
            }
            splits += 1;
            if splits > 4 * self.opt.max_vertices {
                return Err("boundary conformity did not converge".into());
            }
            let (s1, s2) = self.split_sub(s)?;
            queue.push(s1);
            queue.push(s2);
            self.drain_touched(queue);
        }
        Ok(())
    }

    // ------------------------------------------------------------------ exterior removal

    fn remove_exterior(&mut self) {
        // Depth by crossing subsegments from a triangle that has a super vertex; odd depth = domain.
        let start = self.fan(0)[0];
        let mut depth = vec![-1i32; self.tris.len()];
        let mut queue = std::collections::VecDeque::new();
        depth[start as usize] = 0;
        queue.push_back(start);
        while let Some(t) = queue.pop_front() {
            let tr = *self.tri(t);
            for i in 0..3 {
                let u = tr.n[i];
                if u == NONE || depth[u as usize] >= 0 {
                    continue;
                }
                let crosses = self.is_sub(tr.v[(i + 1) % 3], tr.v[(i + 2) % 3]);
                depth[u as usize] = depth[t as usize] + i32::from(crosses);
                queue.push_back(u);
            }
        }
        for t in 0..self.tris.len() {
            if self.tris[t].alive && depth[t] % 2 == 0 {
                self.tris[t].alive = false;
                self.free.push(t as u32);
            }
        }
        // Cut the links into removed triangles and re-anchor the vertex map.
        for t in 0..self.tris.len() {
            if !self.tris[t].alive {
                continue;
            }
            for k in 0..3 {
                let u = self.tris[t].n[k];
                if u != NONE && !self.tris[u as usize].alive {
                    self.tris[t].n[k] = NONE;
                }
            }
            for &v in &self.tris[t].v.clone() {
                self.vt[v as usize] = t as u32;
            }
        }
        self.last = (0..self.tris.len() as u32).find(|&t| self.tris[t as usize].alive).unwrap_or(0);
    }

    // ------------------------------------------------------------------ refinement

    fn circumcentre(&self, t: u32) -> ([f64; 2], f64) {
        let tr = self.tri(t);
        let (a, b, c) = (self.p(tr.v[0]), self.p(tr.v[1]), self.p(tr.v[2]));
        let (bx, by, cx, cy) = (b[0] - a[0], b[1] - a[1], c[0] - a[0], c[1] - a[1]);
        let d = 2.0 * (bx * cy - by * cx);
        let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
        let (ux, uy) = ((cy * b2 - by * c2) / d, (bx * c2 - cx * b2) / d);
        ([a[0] + ux, a[1] + uy], ux.hypot(uy))
    }

    /// Badness score (> 1 means refine): radius-edge ratio against `max_ratio`, circumradius
    /// against the size function.
    fn badness(&self, t: u32) -> f64 {
        let tr = self.tri(t);
        let (a, b, c) = (self.p(tr.v[0]), self.p(tr.v[1]), self.p(tr.v[2]));
        let (_, r) = self.circumcentre(t);
        let e = [(b[0] - c[0]).hypot(b[1] - c[1]), (c[0] - a[0]).hypot(c[1] - a[1]), (a[0] - b[0]).hypot(a[1] - b[1])];
        let emin = e.iter().cloned().fold(f64::INFINITY, f64::min);
        if !r.is_finite() || emin <= 0.0 {
            return 0.0;
        }
        if emin < self.opt.min_edge {
            return 0.0;
        }
        let cen = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
        let h = (self.size)(cen);
        (r / emin / self.opt.max_ratio).max(r / (self.opt.size_factor * h))
    }

    fn refine(&mut self) -> Result<(), String> {
        let mut heap: BinaryHeap<Bad> = BinaryHeap::new();
        for t in 0..self.tris.len() as u32 {
            if self.tri(t).alive {
                let sc = self.badness(t);
                if sc > 1.0 {
                    heap.push(Bad(sc, t));
                }
            }
        }
        let mut segq: Vec<u32> = Vec::new();
        while let Some(Bad(_, t)) = heap.pop() {
            if self.pts.len() > self.opt.max_vertices {
                return Err(format!("mesh exceeds {} vertices (size function too small?)", self.opt.max_vertices));
            }
            if !self.tri(t).alive || self.badness(t) <= 1.0 {
                continue;
            }
            let (cc, _) = self.circumcentre(t);
            // Where does the circumcentre fall, and which subsegments would it encroach? A walk that
            // leaves the domain exits through a boundary subsegment, which is then the one to split.
            let (encroached, inside) = match self.locate(cc, t) {
                Loc::Outside(ot, oi) => {
                    let tr = self.tri(ot);
                    let s = self.sub_of.get(&key(tr.v[(oi + 1) % 3], tr.v[(oi + 2) % 3])).copied();
                    (s.into_iter().collect::<Vec<u32>>(), false)
                }
                Loc::OnVertex(_) => continue,
                Loc::Inside(tt) | Loc::OnEdge(tt, _) => (self.encroached_by(cc, tt), true),
            };
            if !encroached.is_empty() {
                for s in encroached {
                    if self.subs[s as usize].alive {
                        let (s1, s2) = self.split_sub(s)?;
                        segq.push(s1);
                        segq.push(s2);
                    }
                }
                self.conform(&mut segq)?;
                heap.push(Bad(f64::MAX, t)); // try again with the finer boundary
            } else if inside {
                self.insert(cc, t)?;
                self.conform(&mut segq)?;
            } else {
                continue;
            }
            for d in std::mem::take(&mut self.dirty) {
                if self.tri(d).alive {
                    let sc = self.badness(d);
                    if sc > 1.0 {
                        heap.push(Bad(sc, d));
                    }
                }
            }
        }
        Ok(())
    }

    /// Subsegments on the edges of the Delaunay cavity of `p` (or adjacent to it) whose diametral
    /// circle contains `p`.
    fn encroached_by(&self, p: [f64; 2], start: u32) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![start];
        seen.insert(start);
        let mut out = Vec::new();
        while let Some(t) = stack.pop() {
            let tr = *self.tri(t);
            for i in 0..3 {
                let (x, y) = (tr.v[(i + 1) % 3], tr.v[(i + 2) % 3]);
                if let Some(&s) = self.sub_of.get(&key(x, y)) {
                    let (a, b) = (self.p(x), self.p(y));
                    if (a[0] - p[0]) * (b[0] - p[0]) + (a[1] - p[1]) * (b[1] - p[1]) < 0.0 && !out.contains(&s) {
                        out.push(s);
                    }
                    continue;
                }
                let u = tr.n[i];
                if u != NONE && !seen.contains(&u) {
                    let ur = self.tri(u);
                    if in_circle(self.p(ur.v[0]), self.p(ur.v[1]), self.p(ur.v[2]), p) > 0.0 {
                        seen.insert(u);
                        stack.push(u);
                    }
                }
            }
        }
        out
    }
}

/// Triangulate `region` with element sizes `size(x)` (target edge length at `x`).
pub fn triangulate(region: &Region, size: &dyn Fn([f64; 2]) -> f64, opt: MeshOptions) -> Result<TriMesh, String> {
    let (lo, hi) = region.bounds();
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    if extent.is_nan() || extent <= 0.0 {
        return Err("empty region".into());
    }
    let (cx, cy) = (0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]));
    let big = 1.0e3 * extent;
    let mut b = Builder {
        pts: Vec::new(),
        tris: Vec::new(),
        free: Vec::new(),
        vt: Vec::new(),
        subs: Vec::new(),
        sub_of: HashMap::new(),
        segs: Vec::new(),
        size,
        opt,
        last: 0,
        rng: 0x9E37_79B9_7F4A_7C15,
        touched: Vec::new(),
        dirty: Vec::new(),
    };
    // Super triangle (vertices 0, 1, 2).
    let s0 = b.add_vertex([cx - 2.0 * big, cy - big]);
    let s1 = b.add_vertex([cx + 2.0 * big, cy - big]);
    let s2 = b.add_vertex([cx, cy + 2.0 * big]);
    let t0 = b.new_tri([s0, s1, s2], [NONE; 3]);
    b.set_tri(t0, [s0, s1, s2], [NONE; 3]);
    b.touched.clear();
    // Boundary vertices and subsegments.
    let mut loop_starts: Vec<(usize, usize)> = Vec::new(); // (first flat segment, count)
    for l in region.loops() {
        loop_starts.push((b.segs.len(), l.segments.len()));
        for s in &l.segments {
            b.segs.push((s.curve, s.name.clone()));
        }
    }
    for &(first, count) in &loop_starts {
        // One vertex per segment start (shared with the previous segment's end).
        let mut starts: Vec<u32> = Vec::with_capacity(count);
        for k in 0..count {
            let p = b.segs[first + k].0.start();
            let hint = b.last;
            starts.push(b.insert(p, hint)?);
        }
        for k in 0..count {
            let seg = first + k;
            let curve = b.segs[seg].0;
            let (va, vb) = (starts[k], starts[(k + 1) % count]);
            let mid = curve.point(0.5);
            let n = ((curve.length() / (*size)(mid).max(1e-300)).ceil() as usize).max((curve.sweep() / opt.max_arc_angle).ceil() as usize).max(1);
            let mut prev = va;
            for j in 1..=n {
                let t = j as f64 / n as f64;
                let cur = if j == n {
                    vb
                } else {
                    let hint = b.last;
                    b.insert(curve.point(t), hint)?
                };
                b.add_sub(prev, cur, seg as u32, (j - 1) as f64 / n as f64, t);
                prev = cur;
            }
        }
    }
    b.touched.clear();
    // Conform: split subsegments until all are edges of the triangulation and none is encroached.
    let mut queue: Vec<u32> = (0..b.subs.len() as u32).collect();
    b.conform(&mut queue)?;
    b.remove_exterior();
    b.touched.clear();
    b.dirty.clear();
    b.refine()?;
    b.finish(region)
}

impl Builder<'_> {
    fn finish(&self, region: &Region) -> Result<TriMesh, String> {
        let _ = region;
        let mut new_id = vec![usize::MAX; self.pts.len()];
        let mut pts = Vec::new();
        let mut tris = Vec::new();
        for tr in self.tris.iter().filter(|t| t.alive) {
            let mut v = [0usize; 3];
            for k in 0..3 {
                let o = tr.v[k] as usize;
                if o < 3 {
                    return Err("internal error: a triangle still uses the super triangle".into());
                }
                if new_id[o] == usize::MAX {
                    new_id[o] = pts.len();
                    pts.push(self.pts[o]);
                }
                v[k] = new_id[o];
            }
            tris.push(v);
        }
        // Boundary edges, oriented with the domain on the left (as they appear in their triangle).
        let mut boundary = Vec::new();
        for tr in self.tris.iter().filter(|t| t.alive) {
            for i in 0..3 {
                if tr.n[i] != NONE {
                    continue;
                }
                let (x, y) = (tr.v[(i + 1) % 3], tr.v[(i + 2) % 3]);
                let sid = *self.sub_of.get(&key(x, y)).ok_or("internal error: a hull edge is not a boundary subsegment")?;
                let s = self.subs[sid as usize];
                let (t_a, t_b) = if s.a == x { (s.t0, s.t1) } else { (s.t1, s.t0) };
                boundary.push(BoundaryEdge { a: new_id[x as usize], b: new_id[y as usize], seg: s.seg as usize, t_a, t_b });
            }
        }
        Ok(TriMesh { pts, tris, boundary, segments: self.segs.clone() })
    }
}

impl TriMesh {
    /// Structural check used by the tests: counter-clockwise triangles, symmetric adjacency, local
    /// Delaunay across every non-boundary edge. Returns the worst (largest) incircle violation
    /// relative to the squared edge length.
    pub fn validate(&self) -> Result<f64, String> {
        let mut edge_tri: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
        for (t, tr) in self.tris.iter().enumerate() {
            if orient(self.pts[tr[0]], self.pts[tr[1]], self.pts[tr[2]]) <= 0.0 {
                return Err(format!("triangle {t} is not counter-clockwise"));
            }
            for i in 0..3 {
                let (a, b) = (tr[(i + 1) % 3], tr[(i + 2) % 3]);
                edge_tri.entry((a.min(b), a.max(b))).or_default().push((t, i));
            }
        }
        let mut worst = 0.0f64;
        for ((a, b), v) in &edge_tri {
            match v.len() {
                1 => {
                    if !self.boundary.iter().any(|e| (e.a.min(e.b), e.a.max(e.b)) == (*a, *b)) {
                        return Err(format!("edge {a}-{b} has one triangle but is not a boundary edge"));
                    }
                }
                2 => {
                    let (t1, i1) = v[0];
                    let t2 = v[1].0;
                    let apex = self.tris[t2].iter().copied().find(|x| x != a && x != b).unwrap();
                    let tr = self.tris[t1];
                    let _ = i1;
                    let inc = in_circle(self.pts[tr[0]], self.pts[tr[1]], self.pts[tr[2]], self.pts[apex]);
                    let len2 = (self.pts[*a][0] - self.pts[*b][0]).powi(2) + (self.pts[*a][1] - self.pts[*b][1]).powi(2);
                    worst = worst.max(inc / (len2 * len2));
                }
                n => return Err(format!("edge {a}-{b} has {n} triangles")),
            }
        }
        Ok(worst)
    }

    /// `(smallest angle in degrees, largest radius-edge ratio, longest edge, shortest edge)`.
    pub fn quality(&self) -> (f64, f64, f64, f64) {
        let (mut amin, mut rmax, mut lmax, mut lmin) = (180.0f64, 0.0f64, 0.0f64, f64::INFINITY);
        for tr in &self.tris {
            let p = [self.pts[tr[0]], self.pts[tr[1]], self.pts[tr[2]]];
            let e: [f64; 3] = std::array::from_fn(|i| {
                let (a, b) = (p[(i + 1) % 3], p[(i + 2) % 3]);
                (a[0] - b[0]).hypot(a[1] - b[1])
            });
            for i in 0..3 {
                let cosv = (e[(i + 1) % 3].powi(2) + e[(i + 2) % 3].powi(2) - e[i].powi(2)) / (2.0 * e[(i + 1) % 3] * e[(i + 2) % 3]);
                amin = amin.min(cosv.clamp(-1.0, 1.0).acos().to_degrees());
            }
            let area = 0.5 * orient(p[0], p[1], p[2]);
            let r = e[0] * e[1] * e[2] / (4.0 * area);
            let emin = e.iter().cloned().fold(f64::INFINITY, f64::min);
            rmax = rmax.max(r / emin);
            lmax = lmax.max(e.iter().cloned().fold(0.0, f64::max));
            lmin = lmin.min(emin);
        }
        (amin, rmax, lmax, lmin)
    }

    /// Total triangle area.
    pub fn area(&self) -> f64 {
        self.tris.iter().map(|t| 0.5 * orient(self.pts[t[0]], self.pts[t[1]], self.pts[t[2]])).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Loop, Region};
    use crate::mesh::Elastic;
    use std::f64::consts::PI;

    fn steel() -> Elastic {
        Elastic::new(30e6, 0.3)
    }

    /// Shoelace area of the boundary polyline (what the straight-sided triangles must cover).
    fn boundary_area(m: &TriMesh) -> f64 {
        m.boundary.iter().map(|e| 0.5 * (m.pts[e.a][0] * m.pts[e.b][1] - m.pts[e.b][0] * m.pts[e.a][1])).sum()
    }

    fn check(m: &TriMesh, label: &str) {
        let worst = m.validate().unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(worst < 1e-9, "{label}: local Delaunay violated by {worst:e}");
        let (a, b) = (m.area(), boundary_area(m));
        assert!((a - b).abs() < 1e-10 * a.abs().max(1.0), "{label}: triangle area {a} vs boundary polygon area {b}");
    }

    #[test]
    fn rectangle_with_uniform_size_is_valid_and_well_shaped() {
        let region = Region::new(Loop::rectangle(0.0, 0.0, 2.0, 1.0).unwrap(), vec![], steel()).unwrap();
        let m = triangulate(&region, &|_| 0.2, MeshOptions::default()).unwrap();
        check(&m, "rectangle");
        assert!((m.area() - 2.0).abs() < 1e-12, "area {}", m.area());
        let (amin, rmax, lmax, lmin) = m.quality();
        eprintln!("rectangle: {} triangles, min angle {amin:.1}, max radius-edge {rmax:.3}, edges {lmin:.3}..{lmax:.3}", m.tris.len());
        assert!(amin > 24.0 && rmax < 1.21, "angles {amin} ratio {rmax}");
        assert!(lmax < 0.2 * 1.5 && lmin > 0.2 * 0.4, "edge range {lmin}..{lmax}");
        // The boundary edges chain the full perimeter, each with the domain on its left: the centroid
        // of the adjacent triangle is on the left of the directed edge.
        let perimeter: f64 = m.boundary.iter().map(|e| (m.pts[e.a][0] - m.pts[e.b][0]).hypot(m.pts[e.a][1] - m.pts[e.b][1])).sum();
        assert!((perimeter - 6.0).abs() < 1e-12, "perimeter {perimeter}");
        for e in &m.boundary {
            let tr = m.tris.iter().find(|t| t.contains(&e.a) && t.contains(&e.b)).unwrap();
            let cen = [(m.pts[tr[0]][0] + m.pts[tr[1]][0] + m.pts[tr[2]][0]) / 3.0, (m.pts[tr[0]][1] + m.pts[tr[1]][1] + m.pts[tr[2]][1]) / 3.0];
            assert!(orient(m.pts[e.a], m.pts[e.b], cen) > 0.0, "boundary edge {}-{} has the domain on its right", e.a, e.b);
        }
    }

    #[test]
    fn hole_boundary_lies_exactly_on_the_circle_and_the_mesh_is_graded() {
        let c = [1.0, 0.5];
        let region = Region::new(Loop::rectangle(0.0, 0.0, 4.0, 2.0).unwrap(), vec![Loop::circle(c, 0.4, "bore").unwrap()], steel()).unwrap();
        // Fine at the hole, coarse far away.
        let size = move |x: [f64; 2]| (0.04 + 0.35 * ((x[0] - c[0]).hypot(x[1] - c[1]) - 0.4).max(0.0)).min(0.6);
        let m = triangulate(&region, &size, MeshOptions::default()).unwrap();
        check(&m, "plate with hole");
        let (amin, rmax, ..) = m.quality();
        eprintln!("plate with hole: {} triangles, min angle {amin:.1}, max radius-edge {rmax:.3}", m.tris.len());
        assert!(amin > 24.0, "min angle {amin}");
        for e in m.boundary.iter().filter(|e| m.segments[e.seg].1 == "bore") {
            for v in [e.a, e.b] {
                let r = (m.pts[v][0] - c[0]).hypot(m.pts[v][1] - c[1]);
                assert!((r - 0.4).abs() < 1e-13, "bore vertex off the circle: {r}");
            }
            // Each bore edge sweeps a small arc.
            assert!((e.t_b - e.t_a).abs() * PI * 0.4 < 0.4 * 0.35 + 1e-9);
        }
        // Grading: the mean edge length near the hole is well below the mean far from it.
        let (mut near, mut far) = ((0.0, 0), (0.0, 0));
        for t in &m.tris {
            let cen = [(m.pts[t[0]][0] + m.pts[t[1]][0] + m.pts[t[2]][0]) / 3.0, (m.pts[t[0]][1] + m.pts[t[1]][1] + m.pts[t[2]][1]) / 3.0];
            let e = ((m.pts[t[0]][0] - m.pts[t[1]][0]).hypot(m.pts[t[0]][1] - m.pts[t[1]][1])).min((m.pts[t[1]][0] - m.pts[t[2]][0]).hypot(m.pts[t[1]][1] - m.pts[t[2]][1]));
            let d = (cen[0] - c[0]).hypot(cen[1] - c[1]);
            if d < 0.6 {
                near.0 += e;
                near.1 += 1;
            } else if d > 2.5 {
                far.0 += e;
                far.1 += 1;
            }
        }
        let (n, f) = (near.0 / near.1 as f64, far.0 / far.1 as f64);
        eprintln!("mean short edge: near the hole {n:.3}, far {f:.3}");
        assert!(f > 3.0 * n, "grading ratio {}", f / n);
    }

    #[test]
    fn concave_l_shape_and_a_thin_slot() {
        let l = Loop::polygon(&[[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]], "l").unwrap();
        let m = triangulate(&Region::new(l, vec![], steel()).unwrap(), &|_| 0.3, MeshOptions::default()).unwrap();
        check(&m, "L shape");
        assert!((m.area() - 5.0).abs() < 1e-12);
        assert!(m.quality().0 > 24.0);
        // A long thin rectangle much narrower than the requested size.
        let slot = Loop::rectangle(0.0, 0.0, 5.0, 0.05).unwrap();
        let m = triangulate(&Region::new(slot, vec![], steel()).unwrap(), &|_| 0.5, MeshOptions::default()).unwrap();
        check(&m, "slot");
        assert!((m.area() - 0.25).abs() < 1e-12);
    }

    /// Smallest interior angle (degrees) of a polygon given counter-clockwise.
    fn min_corner_angle(p: &[[f64; 2]]) -> f64 {
        let n = p.len();
        (0..n)
            .map(|i| {
                let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
                let (u, v) = ([a[0] - b[0], a[1] - b[1]], [c[0] - b[0], c[1] - b[1]]);
                (u[0] * v[0] + u[1] * v[1]).atan2(u[0] * v[1] - u[1] * v[0]).abs().to_degrees()
            })
            .fold(180.0, f64::min)
    }

    #[test]
    fn random_star_shaped_polygons_always_give_valid_meshes() {
        let mut seed = 12345u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut worst_angle_margin = f64::INFINITY;
        for case in 0..150 {
            let n = 5 + (rnd() * 12.0) as usize;
            let pts: Vec<[f64; 2]> = (0..n)
                .map(|k| {
                    let th = 2.0 * PI * (k as f64 + 0.3 * rnd()) / n as f64;
                    let r = 0.6 + 0.9 * rnd();
                    [r * th.cos(), r * th.sin()]
                })
                .collect();
            let corner = min_corner_angle(&pts);
            let region = Region::new(Loop::polygon(&pts, "p").unwrap(), vec![], steel()).unwrap();
            let h = 0.12 + 0.2 * rnd();
            let m = triangulate(&region, &|_| h, MeshOptions { min_edge: 1e-4, ..MeshOptions::default() }).unwrap_or_else(|e| panic!("case {case}: {e}"));
            check(&m, &format!("star case {case}"));
            assert!((m.area() - region.area()).abs() < 1e-10, "case {case}: area");
            // Away from small input angles the quality bound holds; a corner of angle a limits the mesh to ~a.
            let want = 24.0f64.min(0.9 * corner);
            worst_angle_margin = worst_angle_margin.min(m.quality().0 - want);
            assert!(m.quality().0 > want - 1e-9, "case {case}: min angle {} (corner {corner:.1})", m.quality().0);
        }
        eprintln!("150 random polygons meshed; worst angle margin over the guaranteed bound {worst_angle_margin:.2} deg");
    }
}
