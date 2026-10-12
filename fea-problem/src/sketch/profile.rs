//! Closed profile of a sketch: the loops its lines / arcs / circles form, as an outer boundary plus holes,
//! in the `Shape` vocabulary the mesher already understands. Loop order and start are deterministic (lowest
//! entity id first) so the names `edgeN` / `holeN` that supports and loads refer to survive a rebuild.
use super::*;
use crate::problem::{Seg, Shape};

/// Segments per full turn when an arc becomes polygon edges.
const ARC_SEGMENTS: f64 = 48.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub outer: Shape,
    pub holes: Vec<Shape>,
}

struct Loop {
    key: Id,
    shape: Shape,
    pts: Vec<[f64; 2]>,
    /// Circles are tested by centre and radius, polygons by vertices.
    circle: Option<([f64; 2], f64)>,
    /// Entities in walk order with the number of polygon edges each contributes (a circle: 0).
    members: Vec<(Id, usize)>,
}

/// Solver noise (1e-16) must not leak into names-and-values the user sees: snap to 1e-10.
fn snap(v: f64) -> f64 {
    (v * 1e10).round() / 1e10
}
fn snap2(p: [f64; 2]) -> [f64; 2] {
    [snap(p[0]), snap(p[1])]
}

fn area(p: &[[f64; 2]]) -> f64 {
    0.5 * (0..p.len()).map(|i| cross(p[i], p[(i + 1) % p.len()])).sum::<f64>()
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn inside(poly: &[[f64; 2]], q: [f64; 2]) -> bool {
    let mut c = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

fn seg_dist(a: [f64; 2], b: [f64; 2], q: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 { (((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (q[0] - a[0] - t * d[0]).hypot(q[1] - a[1] - t * d[1])
}

fn poly_dist(poly: &[[f64; 2]], q: [f64; 2]) -> f64 {
    (0..poly.len()).map(|i| seg_dist(poly[i], poly[(i + 1) % poly.len()], q)).fold(f64::INFINITY, f64::min)
}

impl Loop {
    /// Is this loop entirely inside `outer`?
    fn within(&self, outer: &Loop) -> bool {
        match self.circle {
            Some((c, r)) => inside(&outer.pts, c) && poly_dist(&outer.pts, c) >= r - 1e-9,
            None => self.pts.iter().all(|&p| inside(&outer.pts, p) || poly_dist(&outer.pts, p) < 1e-9),
        }
    }
}

impl Sketch {
    /// Mesh surface name of every profile edge, as `(entity id, name)`: what a support or load must name to act on
    /// that sketch entity once the part is built (`bottom/right/top/left` of a rectangle outline, `edgeN` of a
    /// polygon outline, `outer` of a circle, `holeN` of the N-th hole). An arc contributes several `edgeN`.
    pub fn surface_names(&self) -> Result<Vec<(Id, String)>, String> {
        let mut loops = self.loops()?;
        loops.sort_by_key(|l| l.key);
        let outer_i = (0..loops.len()).max_by(|&i, &j| area(&loops[i].pts).abs().total_cmp(&area(&loops[j].pts).abs())).ok_or("the sketch has no closed profile")?;
        let outer = loops.remove(outer_i);
        let mut out = Vec::new();
        match &outer.shape {
            Shape::Rect { x0, y0, x1, y1 } => {
                for (id, _) in &outer.members {
                    let Some((a, b)) = self.line_pts(*id) else { continue };
                    let eps = 1e-6 * (x1 - x0).max(y1 - y0);
                    let name = if (a[1] - b[1]).abs() < eps { if (a[1] - y0).abs() < eps { "bottom" } else { "top" } } else if (a[0] - x0).abs() < eps { "left" } else { "right" };
                    out.push((*id, name.to_string()));
                }
            }
            Shape::Polygon { .. } | Shape::Outline { .. } => {
                let mut k = 1;
                for (id, n) in &outer.members {
                    for _ in 0..*n {
                        out.push((*id, format!("edge{k}")));
                        k += 1;
                    }
                }
            }
            _ => {
                for (id, _) in &outer.members {
                    out.push((*id, "outer".to_string()));
                }
            }
        }
        for (h, hole) in loops.iter().enumerate() {
            for (id, _) in &hole.members {
                out.push((*id, format!("hole{}", h + 1)));
            }
        }
        Ok(out)
    }

    /// Loops the sketch forms. Errors name what is wrong (open profile, branch, crossing, nesting).
    pub fn profile(&self) -> Result<Profile, String> {
        let mut loops = self.loops()?;
        if loops.is_empty() {
            return Err("the sketch has no closed profile".into());
        }
        loops.sort_by_key(|l| l.key);
        let outer_i = (0..loops.len()).max_by(|&i, &j| area(&loops[i].pts).abs().total_cmp(&area(&loops[j].pts).abs())).expect("non-empty");
        let outer = loops.remove(outer_i);
        for h in &loops {
            if !h.within(&outer) {
                return Err("the sketch has separate regions; a profile is one outline with holes inside it".into());
            }
        }
        for (i, h) in loops.iter().enumerate() {
            if loops.iter().enumerate().any(|(j, o)| j != i && h.within(o)) {
                return Err("a profile inside a hole (an island) is not supported".into());
            }
        }
        Ok(Profile { outer: outer.shape, holes: loops.into_iter().map(|l| l.shape).collect() })
    }

    /// Union-find class of each point id, merging `Coincident` constraints and points at the same place.
    fn point_classes(&self) -> Vec<(Id, usize)> {
        let pts: Vec<(Id, [f64; 2])> = self.entities.iter().filter_map(|e| if let Entity::Point { x, y } = e.v { Some((e.id, [x, y])) } else { None }).collect();
        let mut parent: Vec<usize> = (0..pts.len()).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while p[r] != r {
                r = p[r];
            }
            p[i] = r;
            r
        }
        let idx = |id: Id| pts.iter().position(|p| p.0 == id);
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (ra, rb) = (find(parent, a), find(parent, b));
            parent[ra.max(rb)] = ra.min(rb);
        };
        for c in &self.constraints {
            if let Constraint::Coincident { a, b } = c.v {
                if let (Some(i), Some(j)) = (idx(a), idx(b)) {
                    union(&mut parent, i, j);
                }
            }
        }
        for i in 0..pts.len() {
            for j in 0..i {
                if (pts[i].1[0] - pts[j].1[0]).hypot(pts[i].1[1] - pts[j].1[1]) < 1e-7 {
                    union(&mut parent, i, j);
                }
            }
        }
        (0..pts.len()).map(|i| (pts[i].0, find(&mut parent, i))).collect()
    }

    fn loops(&self) -> Result<Vec<Loop>, String> {
        let class = self.point_classes();
        let cls = |id: Id| class.iter().find(|c| c.0 == id).map(|c| c.1).expect("point");
        let mut out = Vec::new();
        // Edges: (entity id, class of start, class of end).
        let mut edges: Vec<(Id, usize, usize)> = Vec::new();
        for e in self.entities.iter().filter(|e| !self.is_construction(e.id)) {
            match e.v {
                Entity::Circle { c, r } => {
                    let c = snap2(self.point(c).ok_or("circle without centre")?);
                    let r = snap(r);
                    if !(r.is_finite() && r > 1e-9) {
                        return Err("a circle has no radius".into());
                    }
                    let pts = (0..ARC_SEGMENTS as usize).map(|i| {
                        let t = std::f64::consts::TAU * i as f64 / ARC_SEGMENTS;
                        [c[0] + r * t.cos(), c[1] + r * t.sin()]
                    });
                    out.push(Loop { key: e.id, shape: Shape::Circle { cx: c[0], cy: c[1], r }, pts: pts.collect(), circle: Some((c, r)), members: vec![(e.id, 0)] });
                }
                Entity::Line { a, b } => edges.push((e.id, cls(a), cls(b))),
                Entity::Arc { s, e: en, .. } => edges.push((e.id, cls(s), cls(en))),
                Entity::Point { .. } => {}
            }
        }
        let mut used = vec![false; edges.len()];
        for start in 0..edges.len() {
            if used[start] {
                continue;
            }
            // Walk: (edge index, forward?) until back at the first class.
            let (first, mut at) = (edges[start].1, edges[start].2);
            let mut chain = vec![(start, true)];
            used[start] = true;
            while at != first {
                let nexts: Vec<(usize, bool)> = (0..edges.len()).filter(|&k| !used[k]).filter_map(|k| if edges[k].1 == at { Some((k, true)) } else if edges[k].2 == at { Some((k, false)) } else { None }).collect();
                match nexts.as_slice() {
                    [] => return Err(format!("open profile: entity {} does not close", edges[start].0)),
                    [(k, f)] => {
                        used[*k] = true;
                        at = if *f { edges[*k].2 } else { edges[*k].1 };
                        chain.push((*k, *f));
                    }
                    _ => return Err("a sketch point joins more than two edges; a profile has no branches".into()),
                }
            }
            // A point joined by a third unused or used edge elsewhere is a branch too.
            let key = chain.iter().map(|c| edges[c.0].0).min().expect("non-empty");
            let loop_ = self.chain_loop(&edges, &chain, key)?;
            out.push(loop_);
        }
        // Branch check: every class of an edge end must carry exactly two edge ends.
        let mut deg: Vec<(usize, usize)> = Vec::new();
        for &(_, a, b) in &edges {
            for c in [a, b] {
                match deg.iter_mut().find(|d| d.0 == c) {
                    Some(d) => d.1 += 1,
                    None => deg.push((c, 1)),
                }
            }
        }
        if deg.iter().any(|d| d.1 > 2) {
            return Err("a sketch point joins more than two edges; a profile has no branches".into());
        }
        Ok(out)
    }

    fn chain_loop(&self, edges: &[(Id, usize, usize)], chain: &[(usize, bool)], key: Id) -> Result<Loop, String> {
        // Rotate so the walk starts at the edge with the lowest id, in the direction it is stored.
        let k0 = chain.iter().position(|c| edges[c.0].0 == key).expect("key in chain");
        let mut segs: Vec<Seg> = Vec::new();
        let mut members: Vec<(Id, usize)> = Vec::new();
        for n in 0..chain.len() {
            let (ei, fwd) = chain[(k0 + n) % chain.len()];
            let id = edges[ei].0;
            match self.entity(id) {
                Some(Entity::Line { a, b }) => {
                    let (p, q) = (snap2(self.point(*a).ok_or("line end")?), snap2(self.point(*b).ok_or("line end")?));
                    segs.push(if fwd { Seg::Line { a: p, b: q } } else { Seg::Line { a: q, b: p } });
                    members.push((id, 1));
                }
                Some(Entity::Arc { c, s, e }) => {
                    let (c, s, e) = (snap2(self.point(*c).ok_or("arc centre")?), snap2(self.point(*s).ok_or("arc start")?), snap2(self.point(*e).ok_or("arc end")?));
                    let r = (s[0] - c[0]).hypot(s[1] - c[1]);
                    let (a0, mut a1) = ((s[1] - c[1]).atan2(s[0] - c[0]), (e[1] - c[1]).atan2(e[0] - c[0]));
                    while a1 <= a0 {
                        a1 += std::f64::consts::TAU;
                    }
                    // Stored counter-clockwise from `s` to `e`; walked backwards it runs clockwise from `e`'s end.
                    segs.push(if fwd { Seg::Arc { c, r, a0, a1 } } else { Seg::Arc { c, r, a0: a1, a1: a0 } });
                    members.push((id, 1));
                }
                _ => return Err("a profile edge is not a line or arc".into()),
            }
        }
        let has_arc = segs.iter().any(|sg| matches!(sg, Seg::Arc { .. }));
        // The polygon the edges approximate: for orientation, nesting and the area check.
        let pts: Vec<[f64; 2]> = segs.iter().flat_map(|sg| sg.sample(ARC_SEGMENTS)).map(snap2).collect();
        if pts.len() < 3 || area(&pts).abs() < 1e-12 {
            return Err("a profile loop has no area".into());
        }
        // Orientation is kept as drawn: `edgeN` is the N-th edge walked from the lowest-id edge, and the mesher
        // orients loops itself.
        let eps = 1e-9;
        let shape = if has_arc {
            Shape::Outline { segs }
        } else if pts.len() == 4 && (0..4).all(|i| (pts[i][0] - pts[(i + 1) % 4][0]).abs() < eps || (pts[i][1] - pts[(i + 1) % 4][1]).abs() < eps) {
            let (xs, ys) = (pts.iter().map(|p| p[0]), pts.iter().map(|p| p[1]));
            Shape::Rect { x0: xs.clone().fold(f64::INFINITY, f64::min), y0: ys.clone().fold(f64::INFINITY, f64::min), x1: xs.fold(f64::NEG_INFINITY, f64::max), y1: ys.fold(f64::NEG_INFINITY, f64::max) }
        } else {
            Shape::Polygon { pts: pts.clone() }
        };
        Ok(Loop { key, shape, pts, circle: None, members })
    }
}
