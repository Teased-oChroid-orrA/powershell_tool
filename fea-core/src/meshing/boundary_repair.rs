//! Boundary-layer repair of a snapped tetrahedral mesh (`tetmesh::mesh_solid`), run after the snap so that every element of the
//! solver mesh passes the scaled-Jacobian gate.
//!
//! The lattice mesher cuts a regular grid with a curved surface and pulls the cut vertices onto it; a few percent of the
//! elements next to a curved boundary come out flat (scaled Jacobian 0.02 to 0.05): caps (one surface triangle, the apex nearly in
//! its plane), wedges between two surface triangles of one ridge, and tetrahedra whose four corners all lie on the surface.
//! All of them touch the boundary. This module removes them with local operations, each applied on a journal and kept only
//! when the worst scaled Jacobian of the cavity it changed improves (otherwise rolled back), so the global minimum never
//! decreases and the run terminates:
//!
//! 1. interior vertex optimisation (pattern search on the worst incident element),
//! 2. face flips (2-3) and edge removal (n-to-2(n-2), best polygon triangulation by dynamic programming) of interior edges,
//! 3. sliding of a non-feature boundary vertex along the surface (re-projected on the signed distance, or on its own model
//!    face when faces can be asked, with a bound on how close it may come to a second face),
//! 4. surface peeling of a flat wedge (the quadrilateral's other diagonal) and promotion of a flat cap's apex to the surface,
//! 5. edge collapse (interior vertex onto any neighbour, boundary vertex onto a neighbour along a surface edge; link
//!    condition with a ghost vertex, enclosed-volume and surface-normal guards),
//! 6. as a last resort the longest edge of the element is split and the new vertex optimised.
//!
//! Feature vertices (on an edge or corner of the model, or where the incident surface normals differ by more than 60 degrees)
//! never move and are never removed, so sharp features and face names are untouched. Element count may grow by at most
//! [`RepairOptions::max_growth`].

use super::quality::QualityThresholds;
use crate::tetmesh::FaceHit;
use std::collections::{BTreeMap, BTreeSet};

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> V3 {
    let n = norm(a);
    if n > 0.0 {
        scale(a, 1.0 / n)
    } else {
        a
    }
}

/// Scaled Jacobian of a tetrahedron (1 regular, 0 flat, negative inverted): `6 V / max_corner(product of its three edges)`
/// divided by the regular tetrahedron's value, which is what `quality::element_quality` reports for `Tet4`.
pub fn scaled_jacobian(p: &[V3], t: [usize; 4]) -> f64 {
    let [a, b, c, d] = t.map(|i| p[i]);
    let six_v = dot(cross(sub(b, a), sub(c, a)), sub(d, a));
    let l = |x: V3, y: V3| norm(sub(x, y));
    let (l01, l02, l03, l12, l13, l23) = (l(a, b), l(a, c), l(a, d), l(b, c), l(b, d), l(c, d));
    let m = (l01 * l02 * l03).max(l01 * l12 * l13).max(l02 * l12 * l23).max(l03 * l13 * l23);
    if m <= 0.0 {
        return -1.0;
    }
    six_v / m / std::f64::consts::FRAC_1_SQRT_2
}

fn vol6(p: &[V3], t: [usize; 4]) -> f64 {
    let [a, b, c, d] = t.map(|i| p[i]);
    dot(cross(sub(b, a), sub(c, a)), sub(d, a))
}

/// Distance from `x` to the triangle `abc`.
fn point_triangle_distance(x: V3, a: V3, b: V3, c: V3) -> f64 {
    // closest point by barycentric regions (Ericson, Real-Time Collision Detection)
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(x, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return norm(ap);
    }
    let bp = sub(x, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return norm(bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return norm(sub(ap, scale(ab, d1 / (d1 - d3))));
    }
    let cp = sub(x, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return norm(cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return norm(sub(ap, scale(ac, d2 / (d2 - d6))));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return norm(sub(bp, scale(sub(c, b), w)));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    norm(sub(ap, add(scale(ab, v), scale(ac, w))))
}

/// What the repair may do.
#[derive(Clone, Copy, Debug)]
pub struct RepairOptions {
    /// Elements below this scaled Jacobian are worked on (the gate is 0.1; a margin keeps the result off the edge).
    pub target: f64,
    /// Rounds over the remaining bad elements.
    pub max_rounds: usize,
    /// The element count may grow by at most this fraction.
    pub max_growth: f64,
    /// Allow the last resort (edge splits).
    pub allow_split: bool,
}
impl Default for RepairOptions {
    fn default() -> Self {
        Self { target: 0.12, max_rounds: 100, max_growth: 0.10, allow_split: true }
    }
}

/// What the repair did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepairReport {
    pub tets_before: usize,
    pub tets_after: usize,
    /// Elements under the gate (scaled Jacobian 0.1) before and after.
    pub bad_before: usize,
    pub bad_after: usize,
    pub min_sj_before: f64,
    pub min_sj_after: f64,
    pub volume_before: f64,
    pub volume_after: f64,
    pub rounds: usize,
    pub smooths: usize,
    pub flips: usize,
    pub removals: usize,
    pub slides: usize,
    pub peels: usize,
    pub promotions: usize,
    pub collapses: usize,
    pub splits: usize,
    /// Boundary vertices sunk a little below the surface (within `RELAX_DEPTH` of the size) to lift a flat element.
    pub relaxed: usize,
}

/// The geometry the repair may ask about.
pub struct RepairContext<'a> {
    pub sdf: &'a dyn Fn(V3) -> f64,
    pub face_dists: Option<&'a dyn Fn(V3) -> Vec<FaceHit>>,
}

enum Ch {
    Kill(usize),
    Add(usize),
    Move(usize, V3),
    Bnd(usize, bool),
    Vertex,
}

struct Work<'a> {
    p: Vec<V3>,
    tets: Vec<[usize; 4]>,
    alive: Vec<bool>,
    vt: Vec<Vec<usize>>,
    bnd: Vec<bool>,
    /// 0 unknown, 1 free, 2 fixed (feature).
    state: Vec<u8>,
    /// Reference size (mean incident edge) of each vertex, fixed at the start.
    hv: Vec<f64>,
    live: usize,
    max_live: usize,
    log: Vec<Ch>,
    /// Clock value of the last committed change around each vertex, and the clock.
    ver: Vec<u64>,
    clock: u64,
    /// Bad elements that every operation failed on: element -> (clock, split tried). Retried only once something around
    /// them changed.
    failed: std::collections::HashMap<usize, (u64, bool)>,
    /// Open transactions (a split journals the clean-up it triggers; only the outermost `commit` clears the log).
    depth: usize,
    trace: bool,
    verbose: bool,
    relax_enabled: bool,
    reasons: std::collections::BTreeMap<&'static str, usize>,
    ctx: &'a RepairContext<'a>,
    rep: RepairReport,
    target: f64,
}

const EPS: f64 = 1e-9;
/// Smallest improvement of a cavity's worst element that justifies an operation (unless it lifts it over the target).
const MIN_GAIN: f64 = 0.01;
/// Depth below the surface, as a fraction of the local size, a boundary vertex may be relaxed to as a last resort.
const RELAX_DEPTH: f64 = 0.08;
/// The surface may move by at most this fraction of the local size when a repair changes it (an edge collapse or a peel).
fn max_dev() -> f64 {
    std::env::var("REPAIR_DEV").ok().and_then(|s| s.parse().ok()).unwrap_or(0.1)
}
const GHOST: usize = usize::MAX;

impl<'a> Work<'a> {
    /// Is `new` a worthwhile improvement of `old` (a real step, or over the target)?
    fn gains(&self, old: f64, new: f64) -> bool {
        let gate = QualityThresholds::default().min_scaled_jacobian;
        new > old + MIN_GAIN
            || (old < gate && new >= gate && new > old + EPS)
            || (old < self.target && new >= self.target && new > old + EPS)
    }
    fn sj(&self, t: [usize; 4]) -> f64 {
        scaled_jacobian(&self.p, t)
    }
    fn min_sj(&self, ts: &[usize]) -> f64 {
        ts.iter().map(|&t| self.sj(self.tets[t])).fold(f64::INFINITY, f64::min)
    }
    /// Sum over elements of the squared shortfall below twice the target: smooth, so a move that helps the second-worst element
    /// of a cavity counts even when the worst one is held by something else.
    fn penalty_cells(&self, cs: &[[usize; 4]]) -> f64 {
        let cap = 2.0 * self.target;
        cs.iter().map(|&t| (cap - self.sj(t)).max(0.0).powi(2)).sum()
    }
    fn penalty_ids(&self, ts: &[usize]) -> f64 {
        let cap = 2.0 * self.target;
        ts.iter().map(|&t| (cap - self.sj(self.tets[t])).max(0.0).powi(2)).sum()
    }
    /// Is replacing the cavity `old` (ids) by `new` (cells) an improvement: the worst element better by a margin or over the
    /// target, or the worst no worse and the penalty clearly lower.
    fn better(&self, old: &[usize], new: &[[usize; 4]]) -> bool {
        self.better_than(self.stats(old), new)
    }
    /// (worst element, penalty) of the given elements.
    fn stats(&self, ids: &[usize]) -> (f64, f64) {
        (self.min_sj(ids), self.penalty_ids(ids))
    }
    fn better_than(&self, old: (f64, f64), new: &[[usize; 4]]) -> bool {
        if new.is_empty() {
            return true;
        }
        let new_min = self.min_sj_cells(new);
        if new_min <= 1e-6 {
            return false;
        }
        self.gains(old.0, new_min) || (new_min >= old.0 - 1e-12 && self.penalty_cells(new) < 0.98 * old.1 - 1e-9)
    }
    fn min_sj_cells(&self, cs: &[[usize; 4]]) -> f64 {
        cs.iter().map(|&t| self.sj(t)).fold(f64::INFINITY, f64::min)
    }
    fn why(&mut self, k: &'static str) -> bool {
        if self.verbose {
            eprintln!("      fail {k}");
        }
        if self.trace {
            *self.reasons.entry(k).or_default() += 1;
        }
        false
    }
    fn mark(&self) -> usize {
        self.log.len()
    }
    fn commit(&mut self) {
        if self.depth == 0 {
            self.clock += 1;
            let clock = self.clock;
            for ch in &self.log {
                match ch {
                    Ch::Kill(t) | Ch::Add(t) => {
                        for v in self.tets[*t] {
                            self.ver[v] = clock;
                        }
                    }
                    Ch::Move(v, _) => self.ver[*v] = clock,
                    _ => {}
                }
            }
            self.log.clear();
        }
    }
    /// Latest change at the vertices of `t` or their neighbours.
    fn changed_around(&self, t: usize) -> u64 {
        let mut latest = 0;
        for &v in &self.tets[t] {
            latest = latest.max(self.ver[v]);
            for &u in &self.vt[v] {
                for &w in &self.tets[u] {
                    latest = latest.max(self.ver[w]);
                }
            }
        }
        latest
    }
    fn rollback(&mut self, m: usize) {
        while self.log.len() > m {
            match self.log.pop().unwrap() {
                Ch::Kill(t) => {
                    self.alive[t] = true;
                    self.live += 1;
                    for v in self.tets[t] {
                        self.vt[v].push(t);
                    }
                }
                Ch::Add(t) => {
                    for v in self.tets[t] {
                        self.vt[v].retain(|&u| u != t);
                    }
                    self.tets.pop();
                    self.alive.pop();
                    self.live -= 1;
                }
                Ch::Move(v, old) => self.p[v] = old,
                Ch::Bnd(v, old) => self.bnd[v] = old,
                Ch::Vertex => {
                    self.ver.pop();
                    self.p.pop();
                    self.vt.pop();
                    self.bnd.pop();
                    self.state.pop();
                    self.hv.pop();
                }
            }
        }
    }
    fn kill(&mut self, t: usize) {
        self.alive[t] = false;
        self.live -= 1;
        for v in self.tets[t] {
            self.vt[v].retain(|&u| u != t);
        }
        self.log.push(Ch::Kill(t));
    }
    fn add(&mut self, c: [usize; 4]) -> usize {
        let id = self.tets.len();
        self.tets.push(c);
        self.alive.push(true);
        self.live += 1;
        for v in c {
            self.vt[v].push(id);
        }
        self.log.push(Ch::Add(id));
        id
    }
    fn set_pos(&mut self, v: usize, x: V3) {
        self.log.push(Ch::Move(v, self.p[v]));
        self.p[v] = x;
    }
    fn tets_with(&self, vs: &[usize]) -> Vec<usize> {
        let base = *vs.iter().min_by_key(|&&v| self.vt[v].len()).unwrap();
        self.vt[base].iter().copied().filter(|&t| vs.iter().all(|v| self.tets[t].contains(v))).collect()
    }
    fn is_bface(&self, a: usize, b: usize, c: usize) -> bool {
        self.tets_with(&[a, b, c]).len() == 1
    }
    fn edge_is_boundary(&self, a: usize, b: usize) -> bool {
        for t in self.tets_with(&[a, b]) {
            for w in self.tets[t] {
                if w != a && w != b && self.is_bface(a, b, w) {
                    return true;
                }
            }
        }
        false
    }
    fn compute_bnd(&self, v: usize) -> bool {
        for &t in &self.vt[v] {
            let c = self.tets[t];
            for x in c {
                if x == v {
                    continue;
                }
                // the face opposite x contains v
                let f: Vec<usize> = c.iter().copied().filter(|&y| y != x).collect();
                if self.is_bface(f[0], f[1], f[2]) {
                    return true;
                }
            }
        }
        false
    }
    fn refresh_bnd(&mut self, vs: &[usize]) {
        for &v in vs {
            let now = self.compute_bnd(v);
            if now != self.bnd[v] {
                self.log.push(Ch::Bnd(v, self.bnd[v]));
                self.bnd[v] = now;
            }
        }
    }
    fn neighbours(&self, v: usize) -> Vec<usize> {
        let mut out: BTreeSet<usize> = BTreeSet::new();
        for &t in &self.vt[v] {
            out.extend(self.tets[t].iter().copied().filter(|&w| w != v));
        }
        out.into_iter().collect()
    }
    /// Outward unit normal of the boundary face `f` whose tetrahedron has the fourth vertex `apex`.
    fn outward(&self, f: [usize; 3], apex: usize) -> V3 {
        let n = cross(sub(self.p[f[1]], self.p[f[0]]), sub(self.p[f[2]], self.p[f[0]]));
        let n = if dot(n, sub(self.p[apex], self.p[f[0]])) > 0.0 { scale(n, -1.0) } else { n };
        unit(n)
    }
    /// Boundary faces (sorted vertex triples, with the apex of their tetrahedron) among the given tetrahedra.
    fn boundary_faces(&self, ts: &[usize]) -> BTreeMap<[usize; 3], usize> {
        let mut out = BTreeMap::new();
        for &t in ts {
            let c = self.tets[t];
            for x in c {
                let mut f: Vec<usize> = c.iter().copied().filter(|&y| y != x).collect();
                f.sort_unstable();
                let key = [f[0], f[1], f[2]];
                if self.is_bface(key[0], key[1], key[2]) {
                    out.insert(key, x);
                }
            }
        }
        out
    }
    fn gradient_dir(&self, x: V3, eps: f64) -> V3 {
        let mut g = [0.0; 3];
        for k in 0..3 {
            let (mut a, mut b) = (x, x);
            a[k] -= eps;
            b[k] += eps;
            g[k] = ((self.ctx.sdf)(b) - (self.ctx.sdf)(a)) / (2.0 * eps);
        }
        unit(g)
    }
    /// New boundary faces (not in `before`) must follow the surface: outward normal within 60 degrees of the signed distance's.
    fn new_faces_follow_surface(&self, ts: &[usize], before: &BTreeMap<[usize; 3], usize>, h: f64) -> bool {
        for (f, apex) in self.boundary_faces(ts) {
            if before.contains_key(&f) {
                continue;
            }
            let n = self.outward(f, apex);
            let c = scale(add(add(self.p[f[0]], self.p[f[1]]), self.p[f[2]]), 1.0 / 3.0);
            let g = self.gradient_dir(c, 1e-3 * h);
            if dot(n, g) < 0.5 {
                return false;
            }
        }
        true
    }
    fn volume_of(&self, ts: &[usize]) -> f64 {
        ts.iter().map(|&t| vol6(&self.p, self.tets[t]) / 6.0).sum()
    }

    // ---- feature detection and surface projection ----

    /// Is the (boundary) vertex on an edge or corner of the model, or where the surface bends sharply?
    fn fixed(&mut self, v: usize) -> bool {
        if !self.bnd[v] {
            return false;
        }
        if self.state[v] != 0 {
            return self.state[v] == 2;
        }
        let mut fixed = false;
        // a vertex held by one to three tetrahedra is the apex of a sliver pyramid on the surface, a defect of the mesh and not
        // a model feature (a real edge or corner vertex has a fan of elements around it)
        if self.vt[v].len() <= 3 {
            self.state[v] = 1;
            return false;
        }
        if let Some(fd) = self.ctx.face_dists {
            // two or more faces within half the size, whose normals differ by more than 25 degrees (a tangent seam, where a
            // fillet meets its neighbour, is smooth: the vertex may move across it)
            let hits = fd(self.p[v]);
            let near: Vec<&FaceHit> = hits.iter().filter(|h| h.dist.abs() < 0.5 * self.hv[v]).collect();
            for i in 0..near.len() {
                for j in i + 1..near.len() {
                    if dot(near[i].normal, near[j].normal) < 0.9 {
                        fixed = true;
                    }
                }
            }
        }
        if self.ctx.face_dists.is_none() {
            // without model faces: where two neighbouring incident faces turn by more than 50 degrees (a coarse mesh of a
            // smooth surface turns by about its size over its radius per face, a ridge by far more)
            let ts = self.vt[v].clone();
            let faces: Vec<([usize; 3], usize)> = self.boundary_faces(&ts).into_iter().filter(|(f, _)| f.contains(&v)).collect();
            let ns: Vec<V3> = faces.iter().map(|&(f, a)| self.outward(f, a)).collect();
            'o: for i in 0..ns.len() {
                for j in i + 1..ns.len() {
                    let shared = faces[i].0.iter().filter(|x| faces[j].0.contains(x)).count();
                    let d = dot(ns[i], ns[j]);
                    // a turn of 75 to 150 degrees is a ridge; beyond 150 the two faces fold back on each other, which is a defect
                    // of the mesh (a sliver pyramid) that must stay repairable
                    if shared == 2 && d < 0.26 && d > -0.87 {
                        if self.trace {
                            eprintln!("  feature vertex {v}: neighbouring faces turn {:.1} deg, hv {:.3}, p {:?}", dot(ns[i], ns[j]).acos().to_degrees(), self.hv[v], self.p[v]);
                        }
                        fixed = true;
                        break 'o;
                    }
                }
            }
        }
        self.state[v] = if fixed { 2 } else { 1 };
        fixed
    }
    /// A point of the surface near `x` for the boundary vertex `v` (its own model face when faces can be asked), `None` when
    /// the point would leave the face or come too close to another one.
    fn surface_point(&self, v: Option<usize>, x: V3, h: f64, active: Option<&[usize]>) -> Option<V3> {
        match self.ctx.face_dists {
            Some(fd) => {
                let ref_pos = v.map(|v| self.p[v]).unwrap_or(x);
                let hits = fd(ref_pos);
                let faces: Vec<usize> = match active {
                    Some(a) => a.to_vec(),
                    None => vec![hits.iter().min_by(|a, b| a.dist.abs().total_cmp(&b.dist.abs()))?.face],
                };
                let others_before = hits.iter().filter(|h| !faces.contains(&h.face)).map(|h| h.dist.abs()).fold(f64::INFINITY, f64::min);
                let mut y = x;
                let mut here = fd(y);
                for _ in 0..12 {
                    let mut moved = 0.0f64;
                    for &f in &faces {
                        let hit = here.iter().find(|h| h.face == f)?;
                        for k in 0..3 {
                            y[k] -= hit.dist * hit.normal[k];
                        }
                        moved = moved.max(hit.dist.abs());
                        here = fd(y);
                    }
                    if moved < 1e-9 * h.max(1e-300) {
                        break;
                    }
                }
                for &f in &faces {
                    if here.iter().find(|h| h.face == f)?.dist.abs() > 1e-6 * h {
                        return None;
                    }
                }
                // A point can lie on its selected face while crossing a neighbouring face (for example the bore next to
                // an end plane). Keep the original outside depth as a ceiling; a quality move must not push a boundary
                // vertex farther out of the solid merely because its selected face is still satisfied.
                if (self.ctx.sdf)(y) > (self.ctx.sdf)(ref_pos).max(0.02 * h) + 1e-9 * h {
                    return None;
                }
                for hit in &here {
                    if let Some(old) = hits.iter().find(|old| old.face == hit.face) {
                        if hit.dist > old.dist.max(0.02 * h) + 1e-9 * h {
                            return None;
                        }
                    }
                }
                let others_after = here.iter().filter(|h| !faces.contains(&h.face)).map(|h| h.dist.abs()).fold(f64::INFINITY, f64::min);
                if others_after < (0.6 * h).min(others_before) {
                    return None;
                }
                Some(y)
            }
            None => {
                if active.is_some() {
                    return None;
                }
                let mut y = x;
                let eps = 1e-3 * h;
                for _ in 0..6 {
                    let d = (self.ctx.sdf)(y);
                    let g = self.gradient_dir(y, eps);
                    if d.abs() < 1e-10 * h {
                        break;
                    }
                    y = sub(y, scale(g, d));
                }
                if (self.ctx.sdf)(y).abs() > 1e-6 * h {
                    return None;
                }
                Some(y)
            }
        }
    }

    // ---- vertex optimisation ----

    /// Optimise vertex `v` (interior, or a free boundary vertex sliding on the surface): pattern search on the worst scaled
    /// Jacobian of its star. Returns whether it moved.
    fn optimise_vertex(&mut self, v: usize, use_pen: bool, relax: bool) -> bool {
        let boundary = self.bnd[v];
        // a vertex on an edge of the model (exactly two faces) may slide along it; corners and sharp bends stay
        let mut feature: Option<(Vec<usize>, V3)> = None;
        if boundary && self.fixed(v) {
            let Some(fd) = self.ctx.face_dists else { return false };
            let hits = fd(self.p[v]);
            let near: Vec<&FaceHit> = hits.iter().filter(|h| h.dist.abs() < 0.5 * self.hv[v]).collect();
            if near.len() != 2 {
                return self.why("optimise_vertex#1");
            }
            let t = cross(near[0].normal, near[1].normal);
            if norm(t) < 0.3 {
                return self.why("optimise_vertex#2");
            }
            feature = Some((vec![near[0].face, near[1].face], unit(t)));
        }
        let star = self.vt[v].clone();
        if star.is_empty() {
            return self.why("optimise_vertex#3");
        }
        let h = self.hv[v];
        let old = self.p[v];
        let old_min = self.min_sj(&star);
        let old_faces = if boundary { Some(self.boundary_faces(&star)) } else { None };
        let eval = |this: &mut Work, x: V3| -> (f64, f64) {
            let keep = this.p[v];
            this.p[v] = x;
            let m = this.min_sj(&star);
            let pen = this.penalty_ids(&star);
            this.p[v] = keep;
            (m, pen)
        };
        let old_pen = self.penalty_ids(&star);
        let mut best = old;
        let mut best_val = old_min;
        let mut best_pen = old_pen;
        // directions of the pattern search
        let mut dirs: Vec<V3> = Vec::new();
        if let Some((_, t)) = &feature {
            dirs.push(*t);
            dirs.push(scale(*t, -1.0));
        } else if boundary {
            let n = self.gradient_dir(old, 1e-3 * h);
            let a = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let u1 = unit(cross(n, a));
            let u2 = cross(n, u1);
            for k in 0..8 {
                let th = k as f64 * std::f64::consts::FRAC_PI_4;
                dirs.push(add(scale(u1, th.cos()), scale(u2, th.sin())));
            }
        } else {
            for i in -1i32..=1 {
                for j in -1i32..=1 {
                    for k in -1i32..=1 {
                        if i != 0 || j != 0 || k != 0 {
                            dirs.push(unit([i as f64, j as f64, k as f64]));
                        }
                    }
                }
            }
        }
        // "ideal apex" candidates: the point above each opposite face that would make that element regular
        let mut ideal = [0.0; 3];
        let mut worst_ideal = old;
        let mut worst_val = f64::INFINITY;
        let mut count = 0.0;
        for &t in &star {
            let c = self.tets[t];
            let f: Vec<usize> = c.iter().copied().filter(|&y| y != v).collect();
            let g = scale(add(add(self.p[f[0]], self.p[f[1]]), self.p[f[2]]), 1.0 / 3.0);
            let n = unit(cross(sub(self.p[f[1]], self.p[f[0]]), sub(self.p[f[2]], self.p[f[0]])));
            let n = if dot(n, sub(old, g)) < 0.0 { scale(n, -1.0) } else { n };
            let l = (norm(sub(self.p[f[0]], self.p[f[1]])) + norm(sub(self.p[f[1]], self.p[f[2]])) + norm(sub(self.p[f[2]], self.p[f[0]]))) / 3.0;
            let apex = add(g, scale(n, l * (2.0f64 / 3.0).sqrt()));
            ideal = add(ideal, apex);
            count += 1.0;
            let s = self.sj(c);
            if s < worst_val {
                worst_val = s;
                worst_ideal = apex;
            }
        }
        ideal = scale(ideal, 1.0 / count);
        let nb = self.neighbours(v);
        let mut centroid = [0.0; 3];
        for &w in &nb {
            centroid = add(centroid, self.p[w]);
        }
        centroid = scale(centroid, 1.0 / nb.len() as f64);
        let mut seeds = vec![centroid, ideal, worst_ideal];
        for r in [0.5, 0.25] {
            seeds.push(add(old, scale(sub(ideal, old), r)));
            seeds.push(add(old, scale(sub(worst_ideal, old), r)));
        }
        let place = |this: &Work, x: V3| -> Option<V3> {
            if boundary {
                let y = this.surface_point(Some(v), x, h, feature.as_ref().map(|f| f.0.as_slice()))?;
                if norm(sub(y, old)) > 0.7 * h {
                    return None;
                }
                Some(y)
            } else {
                Some(x)
            }
        };
        let good = 1.25 * self.target;
        // Relaxation: a free boundary vertex may sit a little inside the surface (never outside), within `RELAX_DEPTH` of the
        // local size, when that is what lifts a flat element; the three offsets are tried only in this last-resort mode.
        let offsets: &[f64] = if relax && boundary && feature.is_none() { &[0.0, 0.5 * RELAX_DEPTH, RELAX_DEPTH] } else { &[0.0] };
        let sink = |this: &Work, y: V3, off: f64| -> V3 {
            if off == 0.0 {
                y
            } else {
                sub(y, scale(this.gradient_dir(y, 1e-3 * h), off * h))
            }
        };
        for s in seeds {
            if best_val >= good {
                break;
            }
            for &off in offsets {
              let Some(y0) = place(self, s) else { break };
              let y = sink(self, y0, off);
              {
                let (val, pen) = eval(self, y);
                if val > 1e-6 && if use_pen { val >= old_min - 1e-9 && (val > best_val + 1e-4 || (val >= best_val - 1e-4 && pen < best_pen - 1e-9)) } else { val > best_val + 1e-4 } {
                    best_val = val;
                    best_pen = pen;
                    best = y;
                }
              }
            }
        }
        let mut r = 0.3 * h;
        let mut iter = 0;
        while r > 0.02 * h && iter < 12 && best_val < good {
            iter += 1;
            let mut improved = false;
            let centre = best;
            for d in &dirs {
                if best_val >= good {
                    break;
                }
                for &off in offsets {
                  let Some(y0) = place(self, add(centre, scale(*d, r))) else { break };
                  let y = sink(self, y0, off);
                  {
                    let (val, pen) = eval(self, y);
                    if val > 1e-6 && if use_pen { val >= old_min - 1e-9 && (val > best_val + 1e-4 || (val >= best_val - 1e-4 && pen < best_pen - 1e-9)) } else { val > best_val + 1e-4 } {
                        best_val = val;
                        best_pen = pen;
                        best = y;
                        improved = true;
                    }
                  }
                }
            }
            if !improved {
                r *= 0.5;
            }
        }
        if self.verbose {
            eprintln!("      vertex {v} (bnd {boundary}, relax {relax}, pen {use_pen}): star {} min {old_min:.4} -> {best_val:.4}, pen {old_pen:.5} -> {best_pen:.5}, moved {:.4}", star.len(), norm(sub(best, old)));
        }
        if !(self.gains(old_min, best_val) || (use_pen && best_pen < 0.98 * old_pen - 1e-9)) || best == old {
            return self.why("optimise_vertex#4");
        }
        let m = self.mark();
        self.set_pos(v, best);
        if let Some(before) = old_faces {
            if !self.new_faces_follow_surface(&star, &before, h) {
                self.rollback(m);
                return self.why("optimise_vertex#5");
            }
        }
        self.commit();
        if boundary && relax && (self.ctx.sdf)(best) < -1e-6 * h {
            self.rep.relaxed += 1;
        } else if boundary {
            self.rep.slides += 1;
        } else {
            self.rep.smooths += 1;
        }
        true
    }

    // ---- reconnection ----

    /// Link of the edge `a-b`: the vertices opposite the edge in the surrounding tetrahedra, in order. A closed ring for an
    /// interior edge, an open path (from the apex of one boundary face to the apex of the other) for a boundary edge. `None`
    /// when the link is neither.
    fn link_path(&self, a: usize, b: usize) -> Option<(Vec<usize>, bool)> {
        let ts = self.tets_with(&[a, b]);
        if ts.is_empty() {
            return None;
        }
        let mut adj: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &t in &ts {
            let o: Vec<usize> = self.tets[t].iter().copied().filter(|&v| v != a && v != b).collect();
            adj.entry(o[0]).or_default().push(o[1]);
            adj.entry(o[1]).or_default().push(o[0]);
        }
        let ends: Vec<usize> = adj.iter().filter(|(_, l)| l.len() == 1).map(|(&v, _)| v).collect();
        if adj.values().any(|l| l.is_empty() || l.len() > 2) {
            return None;
        }
        let closed = ends.is_empty();
        if !closed && ends.len() != 2 {
            return None;
        }
        let start = if closed { *adj.keys().next().unwrap() } else { ends[0] };
        let mut path = vec![start];
        let mut prev = usize::MAX;
        let mut cur = start;
        loop {
            let Some(next) = adj[&cur].iter().copied().find(|&w| w != prev) else { break };
            if next == start {
                break;
            }
            path.push(next);
            prev = cur;
            cur = next;
            if path.len() > ts.len() + 1 {
                return None;
            }
        }
        // a closed ring has as many vertices as tetrahedra, an open path one more
        if path.len() != ts.len() + usize::from(!closed) {
            return None;
        }
        Some((path, closed))
    }

    /// Remove the edge `a-b`: the best triangulation of its link polygon. An interior edge (a closed link) leaves the surface
    /// alone; a boundary edge (an open link) is swapped for the chord between the apexes of its two boundary faces (the surface
    /// quadrilateral's other diagonal), which moves the surface by at most [`MAX_DEVIATION`] of the local size. Accepted when
    /// the worst scaled Jacobian of the cavity improves.
    fn remove_edge(&mut self, a: usize, b: usize) -> bool {
        let Some((ring, closed)) = self.link_path(a, b) else { return false };
        let nv = ring.len();
        let ts = self.tets_with(&[a, b]);
        let n = ts.len();
        if nv < 3 && closed || nv < 2 || n > 7 {
            return self.why("remove_edge#1");
        }
        let old_vol: f64 = ts.iter().map(|&t| vol6(&self.p, self.tets[t])).sum();
        let new_count = 2 * (nv - 2);
        if new_count > n && self.live + new_count - n > self.max_live {
            return self.why("remove_edge#2");
        }
        let h = 0.5 * (self.hv[a] + self.hv[b]);
        if !closed {
            // a surface edge: not a feature, and the swap must stay within the deviation bound
            if self.fixed(a) && self.fixed(b) {
                return self.why("remove_edge#6");
            }
            let (c, d) = (ring[0], ring[nv - 1]);
            let (ab, cd) = (sub(self.p[b], self.p[a]), sub(self.p[d], self.p[c]));
            let nrm = cross(ab, cd);
            if norm(nrm) <= 0.0 || dot(nrm, sub(self.p[c], self.p[a])).abs() / norm(nrm) > max_dev() * h {
                return self.why("remove_edge#7");
            }
            // the chord must be new
            if self.tets_with(&[c, d]).iter().any(|t| !ts.contains(t)) {
                return self.why("remove_edge#8");
            }
        }
        // sign pattern of the pair of tetrahedra on a triangle of the link polygon, fixed by the fan from its first vertex
        let sign_a = (1..nv - 1).map(|i| vol6(&self.p, [ring[0], ring[i], ring[i + 1], a])).sum::<f64>().signum();
        let pair = |this: &Work, i: usize, j: usize, k: usize| -> Option<(f64, [usize; 4], [usize; 4])> {
            let ta = [ring[i], ring[j], ring[k], a];
            let tb = [ring[i], ring[k], ring[j], b];
            let (va, vb) = (vol6(&this.p, ta), vol6(&this.p, tb));
            if va.signum() != sign_a || vb.signum() != sign_a {
                return None;
            }
            let (ta, tb) = if sign_a > 0.0 { (ta, tb) } else { ([ta[1], ta[0], ta[2], ta[3]], [tb[1], tb[0], tb[2], tb[3]]) };
            let q = this.sj(ta).min(this.sj(tb));
            if q <= 1e-6 {
                return None;
            }
            Some((q, ta, tb))
        };
        // best[i][j]: best worst-quality triangulation of the polygon ring[i..=j]
        let mut best = vec![vec![f64::NEG_INFINITY; nv]; nv];
        let mut split = vec![vec![usize::MAX; nv]; nv];
        for i in 0..nv - 1 {
            best[i][i + 1] = f64::INFINITY;
        }
        for len in 2..nv {
            for i in 0..nv - len {
                let j = i + len;
                for k in i + 1..j {
                    if let Some((q, _, _)) = pair(self, i, k, j) {
                        let v = q.min(best[i][k]).min(best[k][j]);
                        if v > best[i][j] {
                            best[i][j] = v;
                            split[i][j] = k;
                        }
                    }
                }
            }
        }
        let new_min = best[0][nv - 1];
        if !(new_min > 1e-6) {
            return self.why("remove_edge#3");
        }
        let mut cells: Vec<[usize; 4]> = Vec::new();
        let mut diagonals: Vec<(usize, usize)> = Vec::new();
        let mut stack = vec![(0usize, nv - 1)];
        while let Some((i, j)) = stack.pop() {
            if j - i < 2 {
                continue;
            }
            let k = split[i][j];
            let (_, ta, tb) = pair(self, i, k, j).unwrap();
            cells.push(ta);
            cells.push(tb);
            for (x, y) in [(i, k), (k, j), (i, j)] {
                // an edge of the polygon is a link edge; the others, and the closing chord of an open link, are new
                let polygon_edge = y - x == 1 || (closed && x == 0 && y == nv - 1);
                if !polygon_edge {
                    diagonals.push((ring[x], ring[y]));
                }
            }
            stack.push((i, k));
            stack.push((k, j));
        }
        if !self.better(&ts, &cells) {
            return self.why("remove_edge#3");
        }
        diagonals.sort_unstable();
        diagonals.dedup();
        for &(x, y) in &diagonals {
            if self.tets_with(&[x, y]).iter().any(|t| !ts.contains(t)) {
                return self.why("remove_edge#5");
            }
        }
        let region = if closed { Vec::new() } else { self.region(&[a, b, ring[0], ring[nv - 1]]) };
        let before = if closed { BTreeMap::new() } else { self.boundary_faces(&region) };
        if closed {
            let new_vol: f64 = cells.iter().map(|&c| vol6(&self.p, c)).sum();
            if (new_vol - old_vol).abs() > 1e-7 * old_vol.abs() {
                return self.why("remove_edge#4");
            }
        }
        let m = self.mark();
        for &t in &ts {
            self.kill(t);
        }
        for c in &cells {
            self.add(*c);
        }
        let mut verts = ring.clone();
        verts.extend([a, b]);
        self.refresh_bnd(&verts);
        if !closed {
            let after: Vec<usize> = self.region(&[a, b, ring[0], ring[nv - 1]]);
            if !self.new_faces_follow_surface(&after, &before, h) {
                self.rollback(m);
                return self.why("remove_edge#9");
            }
        }
        self.commit();
        if !closed {
            self.rep.peels += 1;
        } else if n == 3 {
            self.rep.flips += 1;
        } else {
            self.rep.removals += 1;
        }
        true
    }

    /// 2-3 flip of the interior face `(a, b, c)` between the tetrahedra with apexes `d` and `e`.
    fn flip_face(&mut self, a: usize, b: usize, c: usize) -> bool {
        let ts = self.tets_with(&[a, b, c]);
        if ts.len() != 2 || self.live + 1 > self.max_live {
            return self.why("flip_face#1");
        }
        let apex = |this: &Work, t: usize| this.tets[t].iter().copied().find(|v| ![a, b, c].contains(v)).unwrap();
        let (d, e) = (apex(self, ts[0]), apex(self, ts[1]));
        if !self.tets_with(&[d, e]).is_empty() {
            return self.why("flip_face#2");
        }
        let old_vol: f64 = ts.iter().map(|&t| vol6(&self.p, self.tets[t])).sum();
        let mut cells = [[d, e, a, b], [d, e, b, c], [d, e, c, a]];
        let s: Vec<f64> = cells.iter().map(|&t| vol6(&self.p, t)).collect();
        if !(s.iter().all(|&x| x > 0.0) || s.iter().all(|&x| x < 0.0)) {
            return self.why("flip_face#3");
        }
        if s[0] < 0.0 {
            for c in cells.iter_mut() {
                c.swap(0, 1);
            }
        }
        let new_vol: f64 = cells.iter().map(|&c| vol6(&self.p, c)).sum();
        if (new_vol - old_vol).abs() > 1e-7 * old_vol.abs() {
            return self.why("flip_face#4");
        }
        if !self.better(&ts, &cells) {
            return self.why("flip_face#5");
        }
        for &t in &ts {
            self.kill(t);
        }
        for c in cells {
            self.add(c);
        }
        self.refresh_bnd(&[a, b, c, d, e]);
        self.commit();
        self.rep.flips += 1;
        true
    }

    // ---- boundary operations ----

    /// Remove a flat wedge: a tetrahedron with two boundary faces sharing an edge; the surface takes its other two faces.
    fn peel(&mut self, t: usize) -> bool {
        let c = self.tets[t];
        let bf: Vec<usize> = (0..4)
            .filter(|&k| {
                let f: Vec<usize> = (0..4).filter(|&j| j != k).map(|j| c[j]).collect();
                self.is_bface(f[0], f[1], f[2])
            })
            .collect();
        if bf.len() != 2 {
            return self.why("peel#1");
        }
        // c1, c2 are opposite the two boundary faces; the shared edge is the other two vertices
        let (c1, c2) = (c[bf[0]], c[bf[1]]);
        let shared: Vec<usize> = c.iter().copied().filter(|&v| v != c1 && v != c2).collect();
        if self.edge_is_boundary(c1, c2) || self.tets_with(&[c1, c2]).len() < 2 {
            return self.why("peel#2");
        }
        if self.fixed(shared[0]) && self.fixed(shared[1]) {
            return self.why("peel#3");
        }
        let h = self.hv[c1].max(self.hv[c2]);
        // the surface moves by the distance between the two diagonals of the quadrilateral (the wedge's height)
        let (dx, dc) = (sub(self.p[shared[1]], self.p[shared[0]]), sub(self.p[c2], self.p[c1]));
        let nrm = cross(dx, dc);
        if norm(nrm) <= 0.0 || dot(nrm, sub(self.p[c1], self.p[shared[0]])).abs() / norm(nrm) > max_dev() * h {
            return self.why("peel#4");
        }
        let region = self.region(&c);
        let before = self.boundary_faces(&region);
        let m = self.mark();
        self.kill(t);
        self.refresh_bnd(&c);
        let rest: Vec<usize> = region.iter().copied().filter(|&u| u != t).collect();
        // the surface must stay a manifold: every boundary edge among the wedge's vertices borders exactly two boundary faces
        let after = self.boundary_faces(&rest);
        let mut edge_count: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for f in after.keys() {
            for (i, j) in [(0, 1), (0, 2), (1, 2)] {
                *edge_count.entry((f[i], f[j])).or_default() += 1;
            }
        }
        let ok = edge_count.iter().filter(|(e, _)| c.contains(&e.0) && c.contains(&e.1)).all(|(_, &n)| n == 2);
        if !ok || !self.new_faces_follow_surface(&rest, &before, h) {
            self.rollback(m);
            return self.why("peel#5");
        }
        self.commit();
        self.rep.peels += 1;
        true
    }

    /// Live tetrahedra touching any of `vs`.
    fn region(&self, vs: &[usize]) -> Vec<usize> {
        let mut s = BTreeSet::new();
        for &v in vs {
            s.extend(self.vt[v].iter().copied());
        }
        s.into_iter().collect()
    }

    /// Promote the interior apex of a flat cap (one boundary face) to the surface: the cap is removed and its apex, moved onto
    /// the surface, becomes a boundary vertex.
    fn promote(&mut self, t: usize) -> bool {
        let c = self.tets[t];
        let bf: Vec<usize> = (0..4).filter(|&k| {
            let f: Vec<usize> = (0..4).filter(|&j| j != k).map(|j| c[j]).collect();
            self.is_bface(f[0], f[1], f[2])
        }).collect();
        if bf.len() != 1 {
            return self.why("promote#1");
        }
        let d = c[bf[0]];
        if self.bnd[d] {
            return self.why("promote#2");
        }
        let face: Vec<usize> = c.iter().copied().filter(|&v| v != d).collect();
        let h = self.hv[d];
        let vol = vol6(&self.p, c) / 6.0;
        if vol > 0.05 * h * h * h {
            return self.why("promote#3");
        }
        let star = self.vt[d].clone();
        if star.len() < 2 {
            return self.why("promote#4");
        }
        let old_vol = self.volume_of(&star);
        let target = {
            // aim at the surface point under the apex, on the cap's own face when faces can be asked
            let x = self.p[d];
            let y = match self.ctx.face_dists {
                Some(fd) => {
                    let g = scale(add(add(self.p[face[0]], self.p[face[1]]), self.p[face[2]]), 1.0 / 3.0);
                    let hits = fd(g);
                    let primary = match hits.iter().min_by(|a, b| a.dist.abs().total_cmp(&b.dist.abs())) {
                        Some(h) => h.face,
                        None => return false,
                    };
                    let mut y = x;
                    for _ in 0..8 {
                        let here = fd(y);
                        let Some(hit) = here.iter().find(|h| h.face == primary) else { return false };
                        for k in 0..3 {
                            y[k] -= hit.dist * hit.normal[k];
                        }
                    }
                    let here = fd(y);
                    let others = here.iter().filter(|h| h.face != primary).map(|h| h.dist.abs()).fold(f64::INFINITY, f64::min);
                    if others < 0.6 * h {
                        return self.why("promote#5");
                    }
                    y
                }
                None => match self.surface_point(None, x, h, None) {
                    Some(y) => y,
                    None => return false,
                },
            };
            y
        };
        if norm(sub(target, self.p[d])) > 0.5 * h {
            return self.why("promote#6");
        }
        let region = self.region(&c);
        let before = self.boundary_faces(&region);
        let old_stats = self.stats(&star);
        let rest: Vec<usize> = star.iter().copied().filter(|&u| u != t).collect();
        // first with the apex moved onto the surface, then (when that spoils its other elements) where it is
        for move_apex in [true, false] {
            let m = self.mark();
            self.kill(t);
            if move_apex {
                self.set_pos(d, target);
            }
            let cells: Vec<[usize; 4]> = rest.iter().map(|&u| self.tets[u]).collect();
            let new_vol = self.volume_of(&rest);
            if !self.better_than(old_stats, &cells) || (new_vol - old_vol).abs() > 0.05 * h * h * h {
                self.rollback(m);
                self.why("promote#7");
                continue;
            }
            let mut verts = c.to_vec();
            verts.extend(self.neighbours(d));
            self.refresh_bnd(&verts);
            let ts: Vec<usize> = region.iter().copied().filter(|&u| u != t).collect();
            if !self.bnd[d] || !self.new_faces_follow_surface(&ts, &before, h) {
                self.rollback(m);
                self.why("promote#8");
                continue;
            }
            self.commit();
            self.rep.promotions += 1;
            return true;
        }
        false
    }

    // ---- collapse ----

    /// Collapse `u` onto `v`.
    fn collapse(&mut self, u: usize, v: usize) -> bool {
        // A feature vertex may go only onto a neighbour on the same feature (same faces), under the tighter volume bound.
        let fixed_u = self.fixed(u);
        if fixed_u {
            if !self.bnd[v] {
                return self.why("collapse#1");
            }
            if let Some(fd) = self.ctx.face_dists {
                let near = |x: V3, r: f64| -> BTreeSet<usize> { fd(x).iter().filter(|h| h.dist.abs() < r).map(|h| h.face).collect() };
                let (fu, fv) = (near(self.p[u], 0.5 * self.hv[u]), near(self.p[v], 0.5 * self.hv[v]));
                if !fu.is_subset(&fv) {
                    return self.why("collapse#2");
                }
            }
        }
        let bu = self.bnd[u];
        if bu && (!self.bnd[v] || !self.edge_is_boundary(u, v)) {
            return self.why("collapse#3");
        }
        // link condition (the ghost vertex stands for the outside)
        let nbrs = |this: &Work, x: usize| -> BTreeSet<usize> {
            let mut s: BTreeSet<usize> = this.neighbours(x).into_iter().collect();
            if this.bnd[x] {
                s.insert(GHOST);
            }
            s
        };
        let (nu, nv) = (nbrs(self, u), nbrs(self, v));
        let common: BTreeSet<usize> = nu.intersection(&nv).copied().filter(|&w| w != u && w != v).collect();
        let around = self.tets_with(&[u, v]);
        let mut link: BTreeSet<usize> = BTreeSet::new();
        for &t in &around {
            link.extend(self.tets[t].iter().copied().filter(|&w| w != u && w != v));
        }
        if self.edge_is_boundary(u, v) {
            link.insert(GHOST);
        }
        if common != link {
            return self.why("collapse#4");
        }
        let star = self.vt[u].clone();
        let h = self.hv[u].max(self.hv[v]);
        let max_edge_old = star
            .iter()
            .flat_map(|&t| {
                let c = self.tets[t];
                (0..4).flat_map(move |i| (i + 1..4).map(move |j| (c[i], c[j])))
            })
            .map(|(x, y)| norm(sub(self.p[x], self.p[y])))
            .fold(0.0, f64::max);
        let mut replaced: Vec<[usize; 4]> = Vec::new();
        for &t in &star {
            if around.contains(&t) {
                continue;
            }
            let mut c = self.tets[t];
            for x in c.iter_mut() {
                if *x == u {
                    *x = v;
                }
            }
            if vol6(&self.p, c) <= 0.0 {
                return self.why("collapse#5");
            }
            for &w in &c {
                if w != v && norm(sub(self.p[w], self.p[v])) > 1.3 * max_edge_old {
                    return self.why("collapse#6");
                }
            }
            replaced.push(c);
        }
        // a collapse that only deletes elements (every tetrahedron of `u` also holds `v`) judges nothing new
        if !replaced.is_empty() {
            if !self.better(&star, &replaced) {
                return self.why("collapse#7");
            }
        }
        let before = if bu { Some(self.boundary_faces(&star)) } else { None };
        let m = self.mark();
        let nb = self.neighbours(u);
        for &t in &star {
            self.kill(t);
        }
        let mut new_ids = Vec::new();
        for c in replaced {
            new_ids.push(self.add(c));
        }
        self.refresh_bnd(&nb);
        self.refresh_bnd(&[v]);
        if bu {
            let before = before.unwrap();
            // how far the surface moved: the removed vertex's distance to the new boundary faces around the survivor
            let removed = self.p[u];
            let deviation = self
                .boundary_faces(&new_ids)
                .keys()
                .filter(|f| f.contains(&v))
                .map(|f| point_triangle_distance(removed, self.p[f[0]], self.p[f[1]], self.p[f[2]]))
                .fold(f64::INFINITY, f64::min);
            let limit = if fixed_u { 0.5 * max_dev() } else { max_dev() };
            if !(deviation <= limit * h) || !self.new_faces_follow_surface(&new_ids, &before, h) {
                self.rollback(m);
                return self.why("collapse#8");
            }
        }
        self.commit();
        self.rep.collapses += 1;
        true
    }

    // ---- split (last resort) ----

    fn split_edge(&mut self, a: usize, b: usize) -> bool {
        let around = self.tets_with(&[a, b]);
        if self.live + around.len() > self.max_live {
            return self.why("split_edge#1");
        }
        let boundary_edge = self.edge_is_boundary(a, b);
        if boundary_edge && self.fixed(a) && self.fixed(b) {
            return self.why("split_edge#2");
        }
        let old_min = self.min_sj(&around);
        let h = 0.5 * (self.hv[a] + self.hv[b]);
        let mid = scale(add(self.p[a], self.p[b]), 0.5);
        let pos = if boundary_edge {
            match self.surface_point(None, mid, h, None) {
                Some(y) if norm(sub(y, mid)) < 0.5 * h => y,
                _ => return false,
            }
        } else {
            mid
        };
        let before = if boundary_edge { Some(self.boundary_faces(&around)) } else { None };
        self.depth += 1;
        let m = self.mark();
        let nv = self.p.len();
        self.p.push(pos);
        self.vt.push(Vec::new());
        self.bnd.push(false);
        self.state.push(0);
        self.hv.push(h);
        self.ver.push(0);
        self.log.push(Ch::Vertex);
        let mut new_ids = Vec::new();
        for &t in &around {
            let c = self.tets[t];
            self.kill(t);
            let mut ca = c;
            let mut cb = c;
            for k in 0..4 {
                if c[k] == a {
                    ca[k] = nv;
                }
                if c[k] == b {
                    cb[k] = nv;
                }
            }
            new_ids.push(self.add(ca));
            new_ids.push(self.add(cb));
        }
        let mut verts: Vec<usize> = vec![a, b, nv];
        for &t in &new_ids {
            verts.extend(self.tets[t]);
        }
        verts.sort_unstable();
        verts.dedup();
        self.refresh_bnd(&verts);
        // improve around the new vertex
        for _ in 0..3 {
            let mut any = false;
            let ts: Vec<usize> = self.vt[nv].clone();
            for t in ts {
                if !self.alive[t] || self.sj(self.tets[t]) >= self.target {
                    continue;
                }
                any |= self.cleanup_tet(t, false);
            }
            if !any {
                break;
            }
        }
        // judge: everything touching the new vertex or the old cavity vertices
        let mut judged: BTreeSet<usize> = BTreeSet::new();
        for v in [a, b, nv] {
            judged.extend(self.vt[v].iter().copied());
        }
        let judged: Vec<usize> = judged.into_iter().collect();
        let new_min = self.min_sj(&judged);
        let surface_ok = match before {
            Some(ref b4) => self.new_faces_follow_surface(&judged, b4, h),
            None => true,
        };
        // A flat element needs every one of its long edges halved before its shape improves, so a split that makes nothing
        // worse is a step of progress (the element budget and the round limit bound the chain).
        if !(self.gains(old_min, new_min) || new_min >= old_min - 1e-12) || !surface_ok {
            // everything done since the mark (the split and the clean-up around it) is undone together
            self.rollback(m);
            self.depth -= 1;
            return self.why("split_edge#3");
        }
        self.depth -= 1;
        self.commit();
        self.rep.splits += 1;
        true
    }

    // ---- driver ----

    /// Try the operations on the bad element `t` in order; returns whether one was applied.
    fn cleanup_tet(&mut self, t: usize, allow_split: bool) -> bool {
        if !self.alive[t] {
            return false;
        }
        let c = self.tets[t];
        // 1. interior vertices, then free boundary vertices
        let mut vs = c.to_vec();
        vs.sort_by_key(|&v| self.bnd[v]);
        let interior: Vec<usize> = vs.iter().copied().filter(|&v| !self.bnd[v]).collect();
        for v in interior {
            if self.optimise_vertex(v, false, false) {
                return true;
            }
        }
        // 2. reconnection of interior edges and faces
        for i in 0..4 {
            for j in i + 1..4 {
                if !self.alive[t] {
                    return true;
                }
                if self.remove_edge(c[i], c[j]) {
                    return true;
                }
            }
        }
        for k in 0..4 {
            let f: Vec<usize> = (0..4).filter(|&j| j != k).map(|j| c[j]).collect();
            if self.flip_face(f[0], f[1], f[2]) {
                return true;
            }
        }
        // 3. surface moves
        let on_surface: Vec<usize> = vs.iter().copied().filter(|&v| self.bnd[v]).collect();
        for v in on_surface {
            if self.optimise_vertex(v, false, false) {
                return true;
            }
        }
        if self.peel(t) || self.promote(t) {
            return true;
        }
        // 4. collapse, shortest edge first
        let mut edges: Vec<(f64, usize, usize)> = Vec::new();
        for i in 0..4 {
            for j in i + 1..4 {
                edges.push((norm(sub(self.p[c[i]], self.p[c[j]])), c[i], c[j]));
            }
        }
        edges.sort_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, x, y) in &edges {
            if self.collapse(x, y) || self.collapse(y, x) {
                return true;
            }
        }
        // 5. smoothing against the smooth penalty (progress for the second-worst element when the worst is held elsewhere)
        for v in c {
            if self.alive[t] && self.optimise_vertex(v, true, false) {
                return true;
            }
        }
        // 6. split, longest edge first
        if allow_split {
            for &(_, x, y) in edges.iter().rev() {
                if self.split_edge(x, y) {
                    return true;
                }
            }
        }
        // 7. relaxation of boundary vertices off the surface by a bounded depth, once everything else has stalled
        if self.relax_enabled {
            let on_surface: Vec<usize> = c.iter().copied().filter(|&v| self.bnd[v]).collect();
            for v in on_surface {
                if self.alive[t] && self.optimise_vertex(v, false, true) {
                    return true;
                }
            }
        }
        false
    }

    /// Probe a failed element for trace reasons without committing the probe's mesh or counters.
    fn trace_retry(&mut self, t: usize) -> bool {
        let mark = self.mark();
        let report = self.rep.clone();
        self.depth += 1;
        let result = self.cleanup_tet(t, true);
        self.rollback(mark);
        self.depth -= 1;
        self.rep = report;
        result
    }
}

/// Repair the tetrahedra `tets` (positively oriented) over the points `p` in place; returns the compacted vertex map (old to
/// new, `usize::MAX` for a removed vertex) and a report. Vertices of the input that stay keep their relative order.
pub fn repair(p: &mut Vec<V3>, tets: &mut Vec<[usize; 4]>, ctx: &RepairContext, opts: RepairOptions) -> (Vec<usize>, RepairReport) {
    let gate = QualityThresholds::default().min_scaled_jacobian;
    let t_start = std::time::Instant::now();
    let n0 = tets.len();
    let mut vt: Vec<Vec<usize>> = vec![Vec::new(); p.len()];
    for (i, t) in tets.iter().enumerate() {
        for &v in t {
            vt[v].push(i);
        }
    }
    let mut hv = vec![0.0; p.len()];
    for v in 0..p.len() {
        let mut s = 0.0;
        let mut n = 0.0;
        for &t in &vt[v] {
            for &w in &tets[t] {
                if w != v {
                    s += norm(sub(p[w], p[v]));
                    n += 1.0;
                }
            }
        }
        hv[v] = if n > 0.0 { s / n } else { 1.0 };
    }
    let n_verts = p.len();
    let mut w = Work {
        p: std::mem::take(p),
        tets: std::mem::take(tets),
        alive: vec![true; n0],
        vt,
        bnd: vec![false; hv.len()],
        state: vec![0; hv.len()],
        hv,
        live: n0,
        max_live: ((n0 as f64) * (1.0 + opts.max_growth)).floor() as usize,
        log: Vec::new(),
        ver: vec![0; n_verts],
        clock: 0,
        failed: std::collections::HashMap::new(),
        depth: 0,
        trace: std::env::var_os("REPAIR_TRACE").is_some(),
        verbose: false,
        relax_enabled: false,
        reasons: std::collections::BTreeMap::new(),
        ctx,
        rep: RepairReport::default(),
        target: opts.target,
    };
    {
        let mut count: std::collections::HashMap<[usize; 3], u32> = std::collections::HashMap::with_capacity(n0 * 2);
        for c in &w.tets {
            for x in 0..4 {
                let mut f: Vec<usize> = (0..4).filter(|&j| j != x).map(|j| c[j]).collect();
                f.sort_unstable();
                *count.entry([f[0], f[1], f[2]]).or_default() += 1;
            }
        }
        for (f, n) in count {
            if n == 1 {
                for v in f {
                    w.bnd[v] = true;
                }
            }
        }
    }
    let t_init = std::time::Instant::now();
    let all: Vec<usize> = (0..n0).collect();
    w.rep.tets_before = n0;
    w.rep.min_sj_before = w.min_sj(&all);
    w.rep.bad_before = all.iter().filter(|&&t| w.sj(w.tets[t]) < gate).count();
    w.rep.volume_before = w.volume_of(&all);
    if w.trace {
        let mut hist = [0usize; 6];
        let mut bad_off = 0;
        for v in 0..w.p.len() {
            if w.bnd[v] {
                let r = (w.ctx.sdf)(w.p[v]).abs() / w.hv[v];
                let b = if r < 0.02 { 0 } else if r < 0.1 { 1 } else if r < 0.25 { 2 } else if r < 0.5 { 3 } else if r < 1.0 { 4 } else { 5 };
                hist[b] += 1;
                if r >= 0.1 && w.vt[v].iter().any(|&t| w.sj(w.tets[t]) < opts.target) {
                    bad_off += 1;
                }
            }
        }
        eprintln!("boundary vertices by |sdf|/h: <0.02 {} <0.1 {} <0.25 {} <0.5 {} <1 {} >=1 {}; off-surface (>=0.1h) on a bad element: {bad_off}", hist[0], hist[1], hist[2], hist[3], hist[4], hist[5]);
    }
    if !all.iter().any(|&t| w.sj(w.tets[t]) < opts.target) {
        // nothing to do: hand the mesh back untouched with the identity map
        let mut rep = w.rep.clone();
        rep.tets_after = n0;
        rep.bad_after = rep.bad_before;
        rep.min_sj_after = rep.min_sj_before;
        rep.volume_after = rep.volume_before;
        *p = std::mem::take(&mut w.p);
        *tets = std::mem::take(&mut w.tets);
        return ((0..p.len()).collect(), rep);
    }
    let mut best_key = (usize::MAX, usize::MAX);
    let mut stale = 0usize;
    for round in 0..opts.max_rounds {
        let mut bad: Vec<(f64, usize)> = (0..w.tets.len()).filter(|&t| w.alive[t]).map(|t| (w.sj(w.tets[t]), t)).filter(|&(s, _)| s < opts.target).collect();
        if bad.is_empty() {
            break;
        }
        bad.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Stagnation: operations keep succeeding by tiny steps without clearing anything. Three rounds without fewer elements
        // under the gate (or under the target) end the stage, as a stall would.
        let key = (bad.iter().filter(|b| b.0 < gate).count(), bad.len());
        if key < best_key {
            best_key = key;
            stale = 0;
        } else {
            stale += 1;
        }
        if stale >= 3 {
            if !w.relax_enabled {
                w.relax_enabled = true;
                w.failed.clear();
                stale = 0;
            } else {
                break;
            }
        }
        w.rep.rounds = round + 1;
        if w.trace {
            eprintln!("round {round}: {} elements under the target, {} under the gate, {} tets, relax {}", bad.len(), bad.iter().filter(|b| b.0 < gate).count(), w.live, w.relax_enabled);
        }
        let mut progress = false;
        // cheap and geometry-preserving operations first; the last resort only once they stop helping
        let split_now = opts.allow_split && round >= 2;
        for &(_, t) in &bad {
            if !w.alive[t] || w.sj(w.tets[t]) >= opts.target {
                continue;
            }
            if let Some(&(at, split)) = w.failed.get(&t) {
                if (split || !split_now) && w.changed_around(t) <= at {
                    continue;
                }
            }
            if w.cleanup_tet(t, split_now) {
                progress = true;
            } else {
                let at = w.clock;
                w.failed.insert(t, (at, split_now));
            }
        }
        if !progress && !split_now && opts.allow_split {
            // nothing without splitting: go straight to it
            for &(_, t) in &bad {
                if w.alive[t] && w.sj(w.tets[t]) < opts.target {
                    if w.cleanup_tet(t, true) {
                        progress = true;
                    } else {
                        let at = w.clock;
                        w.failed.insert(t, (at, true));
                    }
                }
            }
        }
        if !progress {
            if !w.relax_enabled {
                // everything exact has stalled: allow the bounded relaxation off the surface for what is left
                w.relax_enabled = true;
                w.failed.clear();
                continue;
            }
            break;
        }
    }
    if std::env::var_os("REPAIR_TRACE").is_some() {
        let mut shown = 0;
        for t in 0..w.tets.len() {
            if !w.alive[t] || w.sj(w.tets[t]) >= gate {
                continue;
            }
            let c = w.tets[t];
            let bf = (0..4).filter(|&k| { let f: Vec<usize> = (0..4).filter(|&j| j != k).map(|j| c[j]).collect(); w.is_bface(f[0], f[1], f[2]) }).count();
            let desc: Vec<String> = c.iter().map(|&v| { let fx = w.fixed(v); format!("[b{} f{} h{:.3} star{}]", w.bnd[v] as u8, fx as u8, w.hv[v], w.vt[v].len()) }).collect();
            let mut el = Vec::new();
            for i in 0..4 { for j in i + 1..4 { el.push(norm(sub(w.p[c[i]], w.p[c[j]]))); } }
            eprintln!("  unresolved sj {:.4} bfaces {} verts {} edges {:.3?}", w.sj(c), bf, desc.join(" "), el);
            if shown < 3 {
                shown += 1;
                eprintln!("    coords {:?} sdf {:?}", c.map(|v| w.p[v]), c.map(|v| (w.ctx.sdf)(w.p[v])));
                w.verbose = true;
                let r = w.trace_retry(t);
                w.verbose = false;
                eprintln!("    retry result {r}");
            }
        }
    }
    if std::env::var_os("REPAIR_TRACE").is_some() {
        eprintln!("reasons {:?}", w.reasons);
        eprintln!("repair loop {:.2}s (setup {:.2}s)", t_init.elapsed().as_secs_f64(), (t_init - t_start).as_secs_f64());
    }
    // compact
    let mut remap = vec![usize::MAX; w.p.len()];
    let mut np: Vec<V3> = Vec::new();
    let mut nt: Vec<[usize; 4]> = Vec::new();
    for (t, c) in w.tets.iter().enumerate() {
        if !w.alive[t] {
            continue;
        }
        let mut o = [0usize; 4];
        for k in 0..4 {
            if remap[c[k]] == usize::MAX {
                remap[c[k]] = np.len();
                np.push(w.p[c[k]]);
            }
            o[k] = remap[c[k]];
        }
        nt.push(o);
    }
    let all: Vec<usize> = (0..nt.len()).collect();
    let mut rep = w.rep.clone();
    rep.tets_after = nt.len();
    rep.min_sj_after = all.iter().map(|&t| scaled_jacobian(&np, nt[t])).fold(f64::INFINITY, f64::min);
    rep.bad_after = all.iter().filter(|&&t| scaled_jacobian(&np, nt[t]) < gate).count();
    rep.volume_after = nt.iter().map(|&c| vol6(&np, c) / 6.0).sum();
    *p = np;
    *tets = nt;
    (remap, rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_solver_gate_counts_as_a_gain_below_minimum_step() {
        let sdf = |_: V3| 0.0;
        let ctx = RepairContext { sdf: &sdf, face_dists: None };
        let w = Work {
            p: vec![], tets: vec![], alive: vec![], vt: vec![], bnd: vec![], state: vec![], hv: vec![],
            live: 0, max_live: 0, log: vec![], ver: vec![], clock: 0, failed: Default::default(),
            depth: 0, trace: false, verbose: false, relax_enabled: false, reasons: Default::default(),
            ctx: &ctx, rep: RepairReport::default(), target: 0.12,
        };
        let gate = QualityThresholds::default().min_scaled_jacobian;
        assert!(w.gains(gate - 0.004, gate + 0.002));
        assert!(!w.gains(gate + 0.001, gate + 0.003));
        assert!(!w.gains(gate - 0.004, gate - 0.001));
    }

    #[test]
    fn trace_retry_rolls_back_a_successful_repair() {
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.25, 0.25, 0.01]];
        let mut tets = vec![[4, 1, 2, 3], [0, 4, 2, 3], [0, 1, 4, 3], [0, 1, 2, 4]];
        for t in &mut tets {
            if vol6(&p, *t) < 0.0 { t.swap(1, 2); }
        }
        let mut vt = vec![Vec::new(); p.len()];
        for (i, t) in tets.iter().enumerate() {
            for &v in t { vt[v].push(i); }
        }
        let sdf = |_: V3| 0.0;
        let ctx = RepairContext { sdf: &sdf, face_dists: None };
        let mut w = Work {
            p, tets, alive: vec![true; 4], vt, bnd: vec![true, true, true, true, false], state: vec![0; 5],
            hv: vec![1.0; 5], live: 4, max_live: 8, log: Vec::new(), ver: vec![0; 5], clock: 0,
            failed: Default::default(), depth: 0, trace: false, verbose: false, relax_enabled: false,
            reasons: Default::default(), ctx: &ctx, rep: RepairReport::default(), target: 0.12,
        };
        let before = (w.p.clone(), w.tets.clone(), w.alive.clone(), w.vt.clone(), w.bnd.clone(), w.rep.clone());
        assert!(w.trace_retry(1), "the diagnostic must exercise a successful repair");
        assert_eq!((w.p, w.tets, w.alive, w.vt, w.bnd, w.rep), before);
        assert_eq!((w.live, w.depth, w.clock, w.log.len()), (4, 0, 0, 0));
    }
}
