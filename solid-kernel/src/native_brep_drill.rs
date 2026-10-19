//! Native cylinder booleans on a [`NativeBrep`]: a round hole drilled from a planar face (blind or through, across cavities) and a
//! round boss standing on a planar face. These are the two booleans the solid editor performs (`SolidOp::Hole` / `Boss`).
//!
//! Method (no general surface-surface intersection): the tool is a cylinder whose axis is perpendicular to the start face, so its
//! intersection with the solid is decided on the axis. Every other face is either a plane perpendicular to the axis (which the
//! tool circle crosses cleanly, misses, or partly overlaps) or must be provably clear of the tool; anything else is refused with
//! an explicit error, never a wrong shape. Faces are appended after the existing ones; the existing faces keep their indices.
use crate::native_brep::{Curve, Edge, Face, NativeBrep, Surface, Use};
use crate::V3;
use std::f64::consts::TAU;

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
/// A unit vector perpendicular to the unit vector `a`.
fn perpendicular(a: V3) -> V3 {
    let k = if a[0].abs() <= a[1].abs() && a[0].abs() <= a[2].abs() { 0 } else if a[1].abs() <= a[2].abs() { 1 } else { 2 };
    let mut e = [0.0; 3];
    e[k] = 1.0;
    unit(sub(e, mul(a, dot(e, a))))
}

/// The sampled boundary polyline (3D) of one edge use, in the direction of use.
pub(crate) fn polyline(b: &NativeBrep, u: &Use) -> Vec<V3> {
    let e = &b.edges[u.edge];
    let (p0, p1) = (b.vertices[e.start], b.vertices[e.end]);
    let mut pts = match &e.curve {
        Curve::Line { .. } => vec![p0, p1],
        Curve::Circle { centre, axis, xdir, radius } => {
            let y = cross(*axis, *xdir);
            let rel = sub(p0, *centre);
            let a0 = dot(rel, y).atan2(dot(rel, *xdir));
            let rel1 = sub(p1, *centre);
            let mut sweep = dot(rel1, y).atan2(dot(rel1, *xdir)) - a0;
            if e.start == e.end || sweep <= 1e-12 {
                sweep += TAU;
            }
            let n = 256;
            (0..=n)
                .map(|i| {
                    let a = a0 + sweep * i as f64 / n as f64;
                    add(*centre, add(mul(*xdir, radius * a.cos()), mul(y, radius * a.sin())))
                })
                .collect()
        }
    };
    if !u.forward {
        pts.reverse();
    }
    pts
}

fn seg_distance(p: V3, a: V3, b: V3) -> f64 {
    let d = sub(b, a);
    let l2 = dot(d, d);
    let t = if l2 > 0.0 { (dot(sub(p, a), d) / l2).clamp(0.0, 1.0) } else { 0.0 };
    norm(sub(p, add(a, mul(d, t))))
}

#[derive(Debug, PartialEq)]
enum Fit {
    /// The circle lies in the face's material region, clear of its boundary.
    Inside,
    /// The circle lies in the plane but outside the face (beyond its outer loop, or within one of its holes).
    Outside,
    /// The circle touches or crosses the face's boundary.
    Partial,
}

/// Where the circle (`centre` in the plane of planar face `f`, `radius`) lies against the face.
fn fit(b: &NativeBrep, f: &Face, centre: V3, radius: f64, tol: f64) -> Fit {
    let Surface::Plane { origin, normal, xdir } = &f.surface else { return Fit::Partial };
    let y = cross(*normal, *xdir);
    let to2 = |p: V3| {
        let d = sub(p, *origin);
        [dot(d, *xdir), dot(d, y)]
    };
    let c2 = to2(centre);
    let mut clearance = f64::INFINITY;
    let mut crossings = 0usize;
    for l in std::iter::once(&f.outer).chain(f.holes.iter()) {
        for u in l {
            let e = &b.edges[u.edge];
            if let (Curve::Circle { centre: cc, radius: rr, .. }, true) = (&e.curve, e.start == e.end) {
                clearance = clearance.min(((norm(sub(centre, *cc))) - rr).abs());
            }
            let pts = polyline(b, u);
            for w in pts.windows(2) {
                if !matches!(e.curve, Curve::Circle { .. }) || e.start != e.end {
                    clearance = clearance.min(seg_distance(centre, w[0], w[1]));
                }
                let (p, q) = (to2(w[0]), to2(w[1]));
                if (p[1] > c2[1]) != (q[1] > c2[1]) && c2[0] < p[0] + (c2[1] - p[1]) * (q[0] - p[0]) / (q[1] - p[1]) {
                    crossings += 1;
                }
            }
        }
    }
    if clearance < radius + tol {
        Fit::Partial
    } else if crossings % 2 == 1 {
        Fit::Inside
    } else {
        Fit::Outside
    }
}

fn extent_box(b: &NativeBrep, f: &Face) -> (V3, V3) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut grow = |p: V3, r: V3| {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k] - r[k]);
            hi[k] = hi[k].max(p[k] + r[k]);
        }
    };
    for l in std::iter::once(&f.outer).chain(f.holes.iter()) {
        for u in l {
            let e = &b.edges[u.edge];
            grow(b.vertices[e.start], [0.0; 3]);
            grow(b.vertices[e.end], [0.0; 3]);
            if let Curve::Circle { centre, axis, radius, .. } = &e.curve {
                grow(*centre, [0, 1, 2].map(|k| radius * (1.0 - axis[k] * axis[k]).max(0.0).sqrt()));
            }
        }
    }
    match &f.surface {
        Surface::Sphere { centre, radius, .. } => grow(*centre, [*radius; 3]),
        Surface::Torus { centre, major, minor, .. } => grow(*centre, [major + minor; 3]),
        _ => {}
    }
    (lo, hi)
}

/// The tool: a cylinder of `radius` on the axis through `base` with direction `axis` (unit), from `t = 0` to `t = length`.
struct Tool {
    base: V3,
    axis: V3,
    radius: f64,
    length: f64,
}
impl Tool {
    fn at(&self, t: f64) -> V3 {
        add(self.base, mul(self.axis, t))
    }
    fn bbox(&self, pad: f64) -> (V3, V3) {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for t in [0.0, self.length] {
            let c = self.at(t);
            for k in 0..3 {
                let r = self.radius * (1.0 - self.axis[k] * self.axis[k]).max(0.0).sqrt() + pad;
                lo[k] = lo[k].min(c[k] - r);
                hi[k] = hi[k].max(c[k] + r);
            }
        }
        (lo, hi)
    }
}

impl NativeBrep {
    fn bounds(&self) -> (V3, V3) {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for f in &self.faces {
            let (a, b) = extent_box(self, f);
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
            }
        }
        (lo, hi)
    }

    /// Whether the tool may touch face `g` (a face that is not a perpendicular plane): conservative, a `true` is refused.
    fn tool_may_touch(&self, g: &Face, tool: &Tool, tol: f64) -> bool {
        // Cylinders parallel to the tool: exact (two coaxial-direction circles meet iff |R - r| <= gap <= R + r).
        if let Surface::Cylinder { origin, axis, radius, .. } = &g.surface {
            if dot(*axis, tool.axis).abs() >= 1.0 - 1e-9 {
                let d = sub(*origin, tool.base);
                let gap = norm(sub(d, mul(tool.axis, dot(d, tool.axis))));
                let (big, small) = (radius.max(tool.radius), radius.min(tool.radius));
                return !(gap > radius + tool.radius + tol || gap < big - small - tol);
            }
        }
        let (tl, th) = tool.bbox(tol);
        let (fl, fh) = extent_box(self, g);
        (0..3).all(|k| fl[k] <= th[k] && fh[k] >= tl[k])
    }

    fn circle_edge(&mut self, t: f64, tool: &Tool, x0: V3) -> usize {
        let c = tool.at(t);
        let v = self.vertices.len();
        self.vertices.push(add(c, mul(x0, tool.radius)));
        self.edges.push(Edge { curve: Curve::Circle { centre: c, axis: tool.axis, xdir: x0, radius: tool.radius }, start: v, end: v });
        self.edges.len() - 1
    }
    fn seam_edge(&mut self, from: usize, to: usize) -> usize {
        let (a, b) = (self.vertices[self.edges[from].start], self.vertices[self.edges[to].start]);
        let (s, e) = (self.edges[from].start, self.edges[to].start);
        self.edges.push(Edge { curve: Curve::Line { origin: a, direction: unit(sub(b, a)) }, start: s, end: e });
        self.edges.len() - 1
    }
    fn wall(&mut self, tool: &Tool, x0: V3, from: usize, to: usize, t_from: f64, outward: bool) {
        let seam = self.seam_edge(from, to);
        // Outward-facing wall (boss): the bottom circle runs counter-clockwise about the axis; an inward-facing wall (hole), clockwise.
        let loop_ = vec![
            Use { edge: from, forward: outward },
            Use { edge: seam, forward: true },
            Use { edge: to, forward: !outward },
            Use { edge: seam, forward: false },
        ];
        let surface = Surface::Cylinder { origin: tool.at(t_from), axis: tool.axis, xdir: x0, radius: tool.radius };
        self.faces.push(Face { surface, same_sense: outward, outer: loop_, holes: vec![] });
    }

    /// The planar face `face` and its outward unit normal.
    fn planar(&self, face: usize) -> Result<(V3, V3), String> {
        let f = self.faces.get(face).ok_or_else(|| format!("the solid has no face {face}"))?;
        match &f.surface {
            Surface::Plane { origin, normal, .. } => Ok((*origin, if f.same_sense { unit(*normal) } else { mul(unit(*normal), -1.0) })),
            _ => Err(format!("face {face} is not flat")),
        }
    }

    fn scale_tol(&self) -> f64 {
        let (lo, hi) = self.bounds();
        1e-9 * norm(sub(hi, lo)).max(1e-9)
    }

    /// Perpendicular planar faces (other than `skip`) met by the tool's axis within `(tol, length + tol]`: `(t, face, fit)`.
    fn crossings(&self, skip: usize, tool: &Tool, p0: V3, tol: f64) -> Result<Vec<(f64, usize, bool)>, String> {
        let mut out = Vec::new();
        for (i, g) in self.faces.iter().enumerate() {
            if i == skip {
                continue;
            }
            if let Surface::Plane { origin, normal, .. } = &g.surface {
                if dot(unit(*normal), tool.axis).abs() >= 1.0 - 1e-9 {
                    let t = dot(sub(*origin, p0), tool.axis);
                    if t <= tol && t >= -tol {
                        return Err(format!("face {i} lies in the plane of the start face"));
                    }
                    if t > tol && t <= tool.length + tol {
                        let c = tool.at(t);
                        match fit(self, g, c, tool.radius, tol) {
                            Fit::Inside => {
                                let n = if g.same_sense { unit(*normal) } else { mul(unit(*normal), -1.0) };
                                out.push((t, i, dot(n, tool.axis) > 0.0));
                            }
                            Fit::Outside => {}
                            Fit::Partial => return Err(format!("the tool circle meets the edge of face {i}: not supported by the native cylinder boolean")),
                        }
                    }
                    continue;
                }
            }
            if self.tool_may_touch(g, tool, tol) {
                return Err(format!("the tool may touch face {i}: not supported by the native cylinder boolean"));
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(out)
    }

    /// Drill a round hole of `radius` into the solid from planar face `face`, perpendicular to it, with its axis through `centre`
    /// (projected onto the face plane). `depth: None` goes through every wall the axis meets; `Some(d)` stops at depth `d` (a flat
    /// bottom, or through if the depth reaches the far face). The new faces are appended after the existing ones.
    pub fn drill(&self, face: usize, centre: V3, radius: f64, depth: Option<f64>) -> Result<NativeBrep, String> {
        if !(radius.is_finite() && radius > 0.0) {
            return Err("the hole radius must be positive".into());
        }
        if depth.is_some_and(|d| !(d.is_finite() && d > 0.0)) {
            return Err("the hole depth must be positive".into());
        }
        let (origin, n) = self.planar(face)?;
        let tol = self.scale_tol();
        let axis = mul(n, -1.0);
        let p0 = add(centre, mul(n, -dot(sub(centre, origin), n)));
        let (lo, hi) = self.bounds();
        let length = depth.unwrap_or(norm(sub(hi, lo)) * 1.5 + 1.0);
        let tool = Tool { base: p0, axis, radius, length };
        match fit(self, &self.faces[face], p0, radius, tol) {
            Fit::Inside => {}
            _ => return Err("the hole does not lie inside the flat face".into()),
        }
        let cross = self.crossings(face, &tool, p0, tol)?;
        let mut out = self.clone();
        let x0 = perpendicular(axis);
        // `segment` is the start (t, circle edge) of the stretch of material the hole is currently in; None while it crosses a void.
        let e0 = out.circle_edge(0.0, &tool, x0);
        // The start face is entered, so its hole loop runs counter-clockwise about the axis (clockwise about its own normal).
        out.faces[face].holes.push(vec![Use { edge: e0, forward: true }]);
        let mut segment = Some((0.0, e0));
        for (t, g, exits) in &cross {
            if *exits != segment.is_some() {
                return Err(format!("face {g} does not alternate with the material along the axis: the solid is not a consistent shell here"));
            }
            let e = out.circle_edge(*t, &tool, x0);
            if let Some((s, es)) = segment.take() {
                out.wall(&tool, x0, es, e, s, false);
                out.faces[*g].holes.push(vec![Use { edge: e, forward: false }]);
            } else {
                out.faces[*g].holes.push(vec![Use { edge: e, forward: true }]);
                segment = Some((*t, e));
            }
        }
        if let Some((s, es)) = segment {
            let Some(d) = depth else { return Err("the hole never leaves the solid".into()) };
            let bottom = out.circle_edge(d, &tool, x0);
            out.wall(&tool, x0, es, bottom, s, false);
            out.faces.push(Face {
                surface: Surface::Plane { origin: tool.at(d), normal: mul(axis, -1.0), xdir: x0 },
                same_sense: true,
                outer: vec![Use { edge: bottom, forward: false }],
                holes: vec![],
            });
        }
        Ok(out)
    }

    /// Stand a round boss of `radius` and `height` on planar face `face`, its axis through `centre` (projected onto the plane).
    pub fn boss(&self, face: usize, centre: V3, radius: f64, height: f64) -> Result<NativeBrep, String> {
        if !(radius.is_finite() && radius > 0.0 && height.is_finite() && height > 0.0) {
            return Err("the boss radius and height must be positive".into());
        }
        let (origin, n) = self.planar(face)?;
        let tol = self.scale_tol();
        let p0 = add(centre, mul(n, -dot(sub(centre, origin), n)));
        let tool = Tool { base: p0, axis: n, radius, length: height };
        match fit(self, &self.faces[face], p0, radius, tol) {
            Fit::Inside => {}
            _ => return Err("the boss does not lie inside the flat face".into()),
        }
        if let Some((t, g, _)) = self.crossings(face, &tool, p0, tol)?.first() {
            return Err(format!("the boss would meet face {g} at height {t}"));
        }
        let mut out = self.clone();
        let x0 = perpendicular(n);
        let base = out.circle_edge(0.0, &tool, x0);
        let top = out.circle_edge(height, &tool, x0);
        out.faces[face].holes.push(vec![Use { edge: base, forward: false }]);
        out.wall(&tool, x0, base, top, 0.0, true);
        out.faces.push(Face {
            surface: Surface::Plane { origin: tool.at(height), normal: n, xdir: x0 },
            same_sense: true,
            outer: vec![Use { edge: top, forward: true }],
            holes: vec![],
        });
        Ok(out)
    }
}
