//! Native fillet of every edge of a convex polyhedron (`fillet_all` for the shapes it is exact for), no OpenCASCADE.
//!
//! For a convex solid `S` whose vertices each join three faces, rolling a ball of radius `r` along all edges gives the Minkowski sum
//! of the inward offset `S_r` (every face plane moved in by `r`) with the ball. Its boundary is: every original face plane, cut
//! down to the polygon of the tangent points `V' + r n` (V' = a vertex of `S_r`); one cylinder of radius `r` along every edge of
//! `S_r` (an arc of normals between the two faces); one sphere patch of radius `r` at every vertex of `S_r`. All curves are lines
//! and circle arcs. A solid that is not convex, has a face with a hole or a curved face, a vertex of other degree than three, or a
//! radius too big for the offset body to survive is refused with the reason, never filleted approximately.
use crate::native_brep::{Curve, Edge, Face, NativeBrep, Surface, Use};
use crate::native_brep_drill::polyline;
use crate::V3;
use std::collections::BTreeMap;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, s: f64) -> V3 {
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
    mul(a, 1.0 / norm(a))
}
fn det(a: V3, b: V3, c: V3) -> f64 {
    dot(a, cross(b, c))
}
fn key(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

struct Build {
    out: NativeBrep,
    lines: BTreeMap<(usize, usize), usize>,
    arcs: BTreeMap<(usize, usize), usize>,
}
impl Build {
    fn line(&mut self, a: usize, b: usize) -> usize {
        if let Some(e) = self.lines.get(&key(a, b)) {
            return *e;
        }
        let (p, q) = (self.out.vertices[a], self.out.vertices[b]);
        self.out.edges.push(Edge { curve: Curve::Line { origin: p, direction: unit(sub(q, p)) }, start: a, end: b });
        self.lines.insert(key(a, b), self.out.edges.len() - 1);
        self.out.edges.len() - 1
    }
    /// The arc of radius `r` about `centre` from vertex `a` (at `centre + r na`) to vertex `b` (at `centre + r nb`).
    fn arc(&mut self, a: usize, b: usize, na: V3, nb: V3, centre: V3, r: f64) -> usize {
        if let Some(e) = self.arcs.get(&key(a, b)) {
            return *e;
        }
        let curve = Curve::Circle { centre, axis: unit(cross(na, nb)), xdir: na, radius: r };
        self.out.edges.push(Edge { curve, start: a, end: b });
        self.arcs.insert(key(a, b), self.out.edges.len() - 1);
        self.out.edges.len() - 1
    }
    /// The loop through `cycle` (vertex indices), run counter-clockwise about `outward`.
    fn oriented(&self, cycle: &[usize], outward: V3) -> Result<Vec<Use>, String> {
        let mut uses = Vec::new();
        for i in 0..cycle.len() {
            let (a, b) = (cycle[i], cycle[(i + 1) % cycle.len()]);
            let e = self.lines.get(&key(a, b)).or_else(|| self.arcs.get(&key(a, b))).ok_or("internal: a missing fillet edge")?;
            uses.push(Use { edge: *e, forward: self.out.edges[*e].start == a });
        }
        let mut area = [0.0; 3];
        for u in &uses {
            let pts = polyline(&self.out, u);
            for w in pts.windows(2) {
                area = add(area, cross(w[0], w[1]));
            }
        }
        if dot(area, outward) < 0.0 {
            uses.reverse();
            for u in &mut uses {
                u.forward = !u.forward;
            }
        }
        Ok(uses)
    }
}

impl NativeBrep {
    /// Round every edge of a convex polyhedron with radius `radius` (see the module doc for what is accepted).
    pub fn fillet_all_convex(&self, radius: f64) -> Result<NativeBrep, String> {
        if !(radius.is_finite() && radius > 0.0) {
            return Err("a fillet radius must be positive".into());
        }
        // Planar faces with one straight loop.
        let mut normal = Vec::new();
        let mut offset = Vec::new();
        let mut cycles: Vec<Vec<usize>> = Vec::new();
        for (i, f) in self.faces.iter().enumerate() {
            let Surface::Plane { origin, normal: n, .. } = &f.surface else { return Err(format!("face {i} is not flat: the native fillet needs a polyhedron")) };
            if !f.holes.is_empty() {
                return Err(format!("face {i} has a hole: the native fillet needs a convex polyhedron"));
            }
            let n = if f.same_sense { unit(*n) } else { mul(unit(*n), -1.0) };
            let mut cyc = Vec::new();
            for u in &f.outer {
                let e = &self.edges[u.edge];
                if !matches!(e.curve, Curve::Line { .. }) {
                    return Err(format!("face {i} has a curved edge: the native fillet needs a polyhedron"));
                }
                cyc.push(if u.forward { e.start } else { e.end });
            }
            normal.push(n);
            offset.push(dot(n, *origin));
            cycles.push(cyc);
        }
        let diag = {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for p in &self.vertices {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            norm(sub(hi, lo))
        };
        let tol = 1e-9 * diag.max(1e-9);
        // Convex: every vertex lies on the inner side of every face plane.
        for (v, p) in self.vertices.iter().enumerate() {
            for (f, n) in normal.iter().enumerate() {
                if dot(*n, *p) > offset[f] + tol {
                    return Err(format!("the solid is not convex (vertex {v} is outside face {f}): the native fillet needs a convex polyhedron"));
                }
            }
        }
        // Faces around each vertex, and the two faces of each edge.
        let mut around: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut sides: BTreeMap<(usize, usize), Vec<(usize, usize, usize)>> = BTreeMap::new();
        for (f, cyc) in cycles.iter().enumerate() {
            for i in 0..cyc.len() {
                let (a, b) = (cyc[i], cyc[(i + 1) % cyc.len()]);
                around.entry(a).or_default().push(f);
                sides.entry(key(a, b)).or_default().push((f, a, b));
            }
        }
        if let Some((v, fs)) = around.iter().find(|(_, fs)| fs.len() != 3) {
            return Err(format!("vertex {v} joins {} faces: the native fillet needs every vertex to join exactly three", fs.len()));
        }
        for (k, s) in &sides {
            if s.len() != 2 || s[0].1 != s[1].2 || s[0].2 != s[1].1 {
                return Err(format!("edge {k:?} is not shared by two consistently oriented faces"));
            }
        }
        // The vertices of the inward offset body.
        let mut inner: BTreeMap<usize, V3> = BTreeMap::new();
        for (v, fs) in &around {
            let (a, b, c) = (normal[fs[0]], normal[fs[1]], normal[fs[2]]);
            let d = det(a, b, c);
            if d.abs() < 1e-9 {
                return Err(format!("vertex {v}: the three faces are not independent"));
            }
            let rhs = [offset[fs[0]] - radius, offset[fs[1]] - radius, offset[fs[2]] - radius];
            inner.insert(*v, solve3(a, b, c, rhs, d));
        }
        // The offset body must survive: its vertices inside all shifted planes and its edges still pointing the same way.
        for (v, p) in &inner {
            for (f, n) in normal.iter().enumerate() {
                if dot(*n, *p) > offset[f] - radius + tol {
                    return Err(format!("the radius {radius} is too big for this solid (vertex {v} leaves the offset body)"));
                }
            }
        }
        for (k, _) in &sides {
            let (u, v) = *k;
            if dot(sub(inner[&v], inner[&u]), sub(self.vertices[v], self.vertices[u])) <= tol {
                return Err(format!("the radius {radius} is too big for this solid (edge {k:?} vanishes)"));
            }
        }
        // Build the filleted shell.
        let mut b = Build { out: NativeBrep::default(), lines: BTreeMap::new(), arcs: BTreeMap::new() };
        let mut tangent: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for (f, cyc) in cycles.iter().enumerate() {
            for &v in cyc {
                tangent.insert((f, v), b.out.vertices.len());
                b.out.vertices.push(add(inner[&v], mul(normal[f], radius)));
            }
        }
        // Flat faces (their straight edges are shared with the edge cylinders).
        let mut flat_cycles = Vec::new();
        for (f, cyc) in cycles.iter().enumerate() {
            let pc: Vec<usize> = cyc.iter().map(|v| tangent[&(f, *v)]).collect();
            for i in 0..pc.len() {
                b.line(pc[i], pc[(i + 1) % pc.len()]);
            }
            flat_cycles.push(pc);
        }
        for (f, pc) in flat_cycles.iter().enumerate() {
            let outer = b.oriented(pc, normal[f])?;
            let (p0, p1) = (b.out.vertices[pc[0]], b.out.vertices[pc[1]]);
            b.out.faces.push(Face { surface: Surface::Plane { origin: p0, normal: normal[f], xdir: unit(sub(p1, p0)) }, same_sense: true, outer, holes: vec![] });
        }
        // Edge cylinders.
        for (_, s) in &sides {
            let (f, u, v) = s[0];
            let g = s[1].0;
            let (pfu, pfv, pgv, pgu) = (tangent[&(f, u)], tangent[&(f, v)], tangent[&(g, v)], tangent[&(g, u)]);
            b.arc(pfv, pgv, normal[f], normal[g], inner[&v], radius);
            b.arc(pfu, pgu, normal[f], normal[g], inner[&u], radius);
            b.line(pfu, pfv);
            b.line(pgu, pgv);
            let outward = unit(add(normal[f], normal[g]));
            let outer = b.oriented(&[pfu, pfv, pgv, pgu], outward)?;
            let axis = unit(sub(inner[&v], inner[&u]));
            let xdir = normal[f];
            let surface = Surface::Cylinder { origin: inner[&u], axis, xdir, radius };
            b.out.faces.push(Face { surface, same_sense: true, outer, holes: vec![] });
        }
        // Vertex spheres.
        for (v, fs) in &around {
            let cyc: Vec<usize> = fs.iter().map(|f| tangent[&(*f, *v)]).collect();
            let outward = unit(add(add(normal[fs[0]], normal[fs[1]]), normal[fs[2]]));
            let outer = b.oriented(&cyc, outward)?;
            let surface = Surface::Sphere { centre: inner[v], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius };
            b.out.faces.push(Face { surface, same_sense: true, outer, holes: vec![] });
        }
        Ok(b.out)
    }
}

/// The solution of `[a; b; c] x = rhs` (rows a, b, c; `d` their determinant).
fn solve3(a: V3, b: V3, c: V3, rhs: V3, d: f64) -> V3 {
    let col = |k: usize| [a[k], b[k], c[k]];
    let (c0, c1, c2) = (col(0), col(1), col(2));
    [det(rhs, c1, c2) / d, det(c0, rhs, c2) / d, det(c0, c1, rhs) / d]
}
