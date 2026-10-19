//! An owned analytic B-rep and its constructors (extrusion of a sketch profile, revolution of a polyline), no OpenCASCADE.
//!
//! This is the native counterpart of the modelling half of `occt::Brep` (ADR-019, epic #46/#63): the shape is built here, written
//! by `native_brep_step`, and read back by the native importer (`AnalyticModel`). Surfaces are the analytic ones the importer
//! understands (plane, cylinder, cone, sphere, torus); curves are lines and circles. A solid is a single closed shell whose face
//! loops are consistently oriented: every edge is used once forward and once reversed, and each loop runs counter-clockwise seen
//! from outside the material (a hole boundary clockwise), which is the STEP convention.
//!
//! Closed-form volumes ([`extrude_volume`], [`lathe_volume`]) are the proof anchors of the tests; they do not read the shape.
use crate::V3;

#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Line { origin: V3, direction: V3 },
    /// A full circle (or the arc from its first to its last vertex; the constructors here only make full circles).
    Circle { centre: V3, axis: V3, xdir: V3, radius: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Plane { origin: V3, normal: V3, xdir: V3 },
    Cylinder { origin: V3, axis: V3, xdir: V3, radius: f64 },
    /// Radius `radius` at `origin`, opening toward +axis with half-angle `semi_angle`.
    Cone { origin: V3, axis: V3, xdir: V3, radius: f64, semi_angle: f64 },
    Sphere { centre: V3, axis: V3, xdir: V3, radius: f64 },
    Torus { centre: V3, axis: V3, xdir: V3, major: f64, minor: f64 },
}

/// An edge from vertex `start` to vertex `end` along `curve` (a circle with `start == end` is the full circle).
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub curve: Curve,
    pub start: usize,
    pub end: usize,
}

/// An edge of a face loop and the sense in which the loop runs along it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Use {
    pub edge: usize,
    pub forward: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub surface: Surface,
    /// Whether the face's outward normal is the surface's own normal (`ADVANCED_FACE` same-sense flag).
    pub same_sense: bool,
    pub outer: Vec<Use>,
    pub holes: Vec<Vec<Use>>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct NativeBrep {
    pub vertices: Vec<V3>,
    pub edges: Vec<Edge>,
    pub faces: Vec<Face>,
}

/// A closed sketch loop in the plane of the profile (the same contract as `cad::Loop`).
pub use crate::cad::{Loop, Profile};

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// Twice the signed area of a polygon (positive when counter-clockwise).
fn signed_area2(p: &[[f64; 2]]) -> f64 {
    (0..p.len()).map(|i| {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        a[0] * b[1] - a[1] * b[0]
    }).sum()
}

fn check_polygon(p: &[[f64; 2]]) -> Result<(), String> {
    if p.len() < 3 {
        return Err("a polygon loop needs at least three points".into());
    }
    if p.iter().any(|q| !q[0].is_finite() || !q[1].is_finite()) {
        return Err("a polygon loop has a non-finite coordinate".into());
    }
    let scale = p.iter().flat_map(|q| q.iter()).fold(0.0f64, |m, v| m.max(v.abs())).max(1.0);
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        if ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() <= 1e-9 * scale {
            return Err("a polygon loop has a zero-length side".into());
        }
    }
    if signed_area2(p).abs() <= 1e-12 * scale * scale {
        return Err("a polygon loop encloses no area".into());
    }
    Ok(())
}

/// The area of a profile: the outer loop minus the holes.
pub fn profile_area(profile: &Profile) -> f64 {
    let a = |l: &Loop| match l {
        Loop::Polygon(p) => signed_area2(p).abs() / 2.0,
        Loop::Circle { radius, .. } => std::f64::consts::PI * radius * radius,
    };
    a(&profile.outer) - profile.holes.iter().map(a).sum::<f64>()
}

/// The volume of `Profile` extruded by `depth`: area times depth (closed form).
pub fn extrude_volume(profile: &Profile, depth: f64) -> f64 {
    profile_area(profile) * depth.abs()
}

/// The volume of the solid of revolution about the z axis of a closed (r, z) polyline (any orientation), by
/// `V = pi |oint r^2 dz|` with `int r^2 dz = dz (r0^2 + r0 r1 + r1^2) / 3` per side (closed form).
pub fn lathe_volume(points: &[[f64; 2]]) -> f64 {
    let mut v = 0.0;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        v += (b[1] - a[1]) * (a[0] * a[0] + a[0] * b[0] + b[0] * b[0]) / 3.0;
    }
    std::f64::consts::PI * v.abs()
}

impl NativeBrep {
    fn vertex(&mut self, p: V3) -> usize {
        self.vertices.push(p);
        self.vertices.len() - 1
    }
    fn edge(&mut self, curve: Curve, start: usize, end: usize) -> usize {
        self.edges.push(Edge { curve, start, end });
        self.edges.len() - 1
    }
    fn line(&mut self, a: usize, b: usize) -> Result<usize, String> {
        let (p, q) = (self.vertices[a], self.vertices[b]);
        let d = sub(q, p);
        let l = len(d);
        if l <= 0.0 {
            return Err("a degenerate edge".into());
        }
        Ok(self.edge(Curve::Line { origin: p, direction: [d[0] / l, d[1] / l, d[2] / l] }, a, b))
    }
    /// Extrude `profile` (in the plane `z = z0`) by `depth` along +z (a negative depth extrudes downward). The outer loop is
    /// oriented counter-clockwise and the holes clockwise whatever the order they are given in.
    pub fn extrude(profile: &Profile, z0: f64, depth: f64) -> Result<Self, String> {
        if !(depth.abs() > 0.0 && depth.is_finite() && z0.is_finite()) {
            return Err("the extrusion depth must be a non-zero number".into());
        }
        let (zb, zt) = if depth > 0.0 { (z0, z0 + depth) } else { (z0 + depth, z0) };
        let mut b = Self::default();
        // Per loop: (bottom edges, top edges, vertical edges, lateral faces) are built; the caps take the bottom / top edges.
        let mut bottom: Vec<Vec<Use>> = Vec::new();
        let mut top: Vec<Vec<Use>> = Vec::new();
        for (k, l) in std::iter::once(&profile.outer).chain(profile.holes.iter()).enumerate() {
            let outer = k == 0;
            match l {
                Loop::Polygon(raw) => {
                    check_polygon(raw)?;
                    let mut p = raw.clone();
                    // Outer counter-clockwise, hole clockwise, seen from +z.
                    if (signed_area2(&p) > 0.0) != outer {
                        p.reverse();
                    }
                    let n = p.len();
                    let vb: Vec<usize> = p.iter().map(|q| b.vertex([q[0], q[1], zb])).collect();
                    let vt: Vec<usize> = p.iter().map(|q| b.vertex([q[0], q[1], zt])).collect();
                    let eb: Vec<usize> = (0..n).map(|i| b.line(vb[i], vb[(i + 1) % n])).collect::<Result<_, _>>()?;
                    let et: Vec<usize> = (0..n).map(|i| b.line(vt[i], vt[(i + 1) % n])).collect::<Result<_, _>>()?;
                    let ev: Vec<usize> = (0..n).map(|i| b.line(vb[i], vt[i])).collect::<Result<_, _>>()?;
                    for i in 0..n {
                        let j = (i + 1) % n;
                        let (dx, dy) = (p[j][0] - p[i][0], p[j][1] - p[i][1]);
                        let l = (dx * dx + dy * dy).sqrt();
                        // Material is on the left of the travel direction, so the outward normal is to its right.
                        let normal = [dy / l, -dx / l, 0.0];
                        let surface = Surface::Plane { origin: [p[i][0], p[i][1], zb], normal, xdir: [dx / l, dy / l, 0.0] };
                        let outer_loop = vec![
                            Use { edge: eb[i], forward: true },
                            Use { edge: ev[j], forward: true },
                            Use { edge: et[i], forward: false },
                            Use { edge: ev[i], forward: false },
                        ];
                        b.faces.push(Face { surface, same_sense: true, outer: outer_loop, holes: vec![] });
                    }
                    bottom.push((0..n).map(|i| Use { edge: eb[i], forward: true }).collect());
                    top.push((0..n).map(|i| Use { edge: et[i], forward: true }).collect());
                }
                Loop::Circle { centre, radius } => {
                    if radius.is_nan() || *radius <= 0.0 || !radius.is_finite() {
                        return Err("a circle loop needs a positive radius".into());
                    }
                    let (cx, cy, r) = (centre[0], centre[1], *radius);
                    let (vb, vt) = (b.vertex([cx + r, cy, zb]), b.vertex([cx + r, cy, zt]));
                    let circle = |z: f64| Curve::Circle { centre: [cx, cy, z], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius: r };
                    let (eb, et) = (b.edge(circle(zb), vb, vb), b.edge(circle(zt), vt, vt));
                    let ev = b.line(vb, vt)?;
                    // The wall's normal is radial outward for an outer circle (the surface's own) and inward for a hole.
                    let surface = Surface::Cylinder { origin: [cx, cy, zb], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius: r };
                    // Bottom edge runs counter-clockwise for the outer circle, clockwise for a hole.
                    let f = outer;
                    let wall = vec![
                        Use { edge: eb, forward: f },
                        Use { edge: ev, forward: true },
                        Use { edge: et, forward: !f },
                        Use { edge: ev, forward: false },
                    ];
                    b.faces.push(Face { surface, same_sense: outer, outer: wall, holes: vec![] });
                    bottom.push(vec![Use { edge: eb, forward: true }]);
                    top.push(vec![Use { edge: et, forward: true }]);
                }
            }
        }
        // Caps. A loop's stored `forward: true` is its counter-clockwise (outer) or clockwise (hole) order seen from +z, which is
        // the order the top cap runs (outward normal +z); the bottom cap (normal -z) runs every loop the other way round.
        // Circle edges are always stored counter-clockwise, so a circle hole is reversed on the top cap.
        let circle_hole: Vec<bool> = std::iter::once(&profile.outer).chain(profile.holes.iter()).enumerate().map(|(k, l)| k > 0 && matches!(l, Loop::Circle { .. })).collect();
        let cap = |loops: &[Vec<Use>], flip: bool| -> (Vec<Use>, Vec<Vec<Use>>) {
            let run = |k: usize, u: &Vec<Use>| -> Vec<Use> {
                let reverse = flip != circle_hole[k];
                if reverse {
                    u.iter().rev().map(|x| Use { edge: x.edge, forward: !x.forward }).collect()
                } else {
                    u.clone()
                }
            };
            (run(0, &loops[0]), loops.iter().enumerate().skip(1).map(|(k, u)| run(k, u)).collect())
        };
        // A circle outer loop is counter-clockwise already; a polygon outer loop was oriented counter-clockwise above.
        let (bo, bh) = cap(&bottom, true);
        let (to, th) = cap(&top, false);
        b.faces.push(Face { surface: Surface::Plane { origin: [0., 0., zb], normal: [0., 0., -1.], xdir: [1., 0., 0.] }, same_sense: true, outer: bo, holes: bh });
        b.faces.push(Face { surface: Surface::Plane { origin: [0., 0., zt], normal: [0., 0., 1.], xdir: [1., 0., 0.] }, same_sense: true, outer: to, holes: th });
        Ok(b)
    }

    /// An axis-aligned box.
    pub fn box_from_corners(lo: V3, hi: V3) -> Result<Self, String> {
        let p = Profile { outer: Loop::Polygon(vec![[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]]), holes: vec![] };
        Self::extrude(&p, lo[2], hi[2] - lo[2])
    }
    /// A cylinder with its axis along +z through `base`.
    pub fn cylinder_z(base: V3, radius: f64, height: f64) -> Result<Self, String> {
        Self::extrude(&Profile { outer: Loop::Circle { centre: [base[0], base[1]], radius }, holes: vec![] }, base[2], height)
    }

    /// Revolve a closed polyline of (radius, z) points about the z axis. Each side becomes a cylinder (constant radius), a
    /// plane (constant z, a disk or an annulus) or a cone; sides on the axis vanish. A vertex on the axis is only allowed
    /// between sides that lie on the axis or in a plane (a cone apex or a sphere pole is not built here: refused).
    pub fn lathe(points: &[[f64; 2]]) -> Result<Self, String> {
        let n = points.len();
        if n < 3 || points.iter().any(|p| !p[0].is_finite() || !p[1].is_finite() || p[0] < 0.0) {
            return Err("a lathe profile needs at least three finite points with a non-negative radius".into());
        }
        let mut p = points.to_vec();
        if signed_area2(&p).abs() <= 1e-12 {
            return Err("a lathe profile encloses no area".into());
        }
        // Material on the left of the travel direction: counter-clockwise in the (r, z) plane.
        if signed_area2(&p) < 0.0 {
            p.reverse();
        }
        let n = p.len();
        let tol = 1e-12 * p.iter().flat_map(|q| q.iter()).fold(1.0f64, |m, v| m.max(v.abs()));
        let on_axis = |q: [f64; 2]| q[0] <= tol;
        let mut b = Self::default();
        // One circle edge (with its vertex at angle 0) per off-axis profile vertex.
        let mut circle: Vec<Option<(usize, usize)>> = vec![None; n];
        for (i, q) in p.iter().enumerate() {
            if !on_axis(*q) {
                let v = b.vertex([q[0], 0.0, q[1]]);
                let e = b.edge(Curve::Circle { centre: [0., 0., q[1]], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius: q[0] }, v, v);
                circle[i] = Some((v, e));
            }
        }
        // Planar sides: (segment, outward normal sign), collected so annuli can take both circles.
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, c) = (p[i], p[j]);
            let (dr, dz) = (c[0] - a[0], c[1] - a[1]);
            let (ai, ci) = (on_axis(a), on_axis(c));
            if ai && ci {
                continue; // lies on the axis
            }
            if (ai || ci) && dz.abs() > tol {
                return Err("a lathe profile may touch the axis only along a plane or the axis itself (cone apex / pole not supported)".into());
            }
            if dz.abs() <= tol {
                if dr.abs() <= tol {
                    return Err("a lathe profile has a zero-length side".into());
                }
                // Plane z = const. Outward normal (dz, -dr) = (0, -dr): down when moving outward.
                let down = dr > 0.0;
                let normal = [0., 0., if down { -1. } else { 1. }];
                let surface = Surface::Plane { origin: [0., 0., a[1]], normal, xdir: [1., 0., 0.] };
                // Bounds: the larger circle is the outer boundary. Outer counter-clockwise around the normal: forward when the
                // normal is +z, reversed when -z. The inner circle (annulus) is the opposite.
                let up = !down;
                let (outer_i, inner_i) = if a[0] > c[0] { (i, Some(j)) } else { (j, Some(i)) };
                let outer_e = circle[outer_i].ok_or("internal: missing circle")?.1;
                let outer = vec![Use { edge: outer_e, forward: up }];
                let holes = match inner_i.and_then(|k| circle[k]) {
                    Some((_, e)) => vec![vec![Use { edge: e, forward: !up }]],
                    None => vec![],
                };
                b.faces.push(Face { surface, same_sense: true, outer, holes });
                continue;
            }
            // Cylinder (dr = 0) or cone: both circles exist (neither end on the axis).
            let (v0, e0) = circle[i].ok_or("internal: missing circle")?;
            let (v1, e1) = circle[j].ok_or("internal: missing circle")?;
            let seam = b.line(v0, v1)?;
            // Outward normal in the (r, z) plane is (dz, -dr)/l; the surface normal of a cylinder is +radial, of a cone
            // (opening toward +z, semi-angle t > 0 when the radius grows with z) cos t * radial - sin t * axis.
            let l = (dr * dr + dz * dz).sqrt();
            let (nr, nz) = (dz / l, -dr / l);
            let (surface, own) = if dr.abs() <= tol {
                (Surface::Cylinder { origin: [0., 0., a[1]], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius: a[0] }, (1.0, 0.0))
            } else {
                // Radius as a function of z: r(z) = a.r + (z - a.z) dr/dz, semi-angle t = atan(dr/dz).
                // STEP wants a positive half-angle: when the radius shrinks with z the cone opens toward -z.
                let t = (dr / dz).atan();
                if t > 0.0 {
                    (Surface::Cone { origin: [0., 0., a[1]], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius: a[0], semi_angle: t }, (t.cos(), -t.sin()))
                } else {
                    (Surface::Cone { origin: [0., 0., a[1]], axis: [0., 0., -1.], xdir: [1., 0., 0.], radius: a[0], semi_angle: -t }, (t.cos(), -t.sin()))
                }
            };
            let same = nr * own.0 + nz * own.1 > 0.0;
            let wall = vec![
                Use { edge: e0, forward: true },
                Use { edge: seam, forward: true },
                Use { edge: e1, forward: false },
                Use { edge: seam, forward: false },
            ];
            b.faces.push(Face { surface, same_sense: same, outer: wall, holes: vec![] });
        }
        if b.faces.is_empty() {
            return Err("the lathe profile has no surface".into());
        }
        Ok(b)
    }
}
