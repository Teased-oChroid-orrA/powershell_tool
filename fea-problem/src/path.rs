//! A sweep path in 3D: a tangent-continuous chain of straight lines and circular arcs, with the rotation-minimizing
//! frame carried along it. A profile swept along the path keeps its orientation relative to that frame, so the sweep of a
//! rectangle along a quarter circle is a bent bar and along a line is the same solid an extrusion gives.
//!
//! For circular arcs and lines the parallel-transport frame is closed-form: constant along a line, and a rigid rotation
//! about the arc's axis along an arc (the twist rate about the tangent is the axis component of the tangent, which is zero
//! for a planar arc). No numerical integration is involved, so a sweep along an arc lands exactly on the circle.
use crate::problem::Seg;

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
/// `v` rotated by `phi` about the unit `axis` (Rodrigues).
fn rotate(v: V3, axis: V3, phi: f64) -> V3 {
    let (c, s) = (phi.cos(), phi.sin());
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Line { a: V3, t: V3 },
    /// Rotation by `angle` (signed, about `axis`) of the radial vector `r0` from the centre `c`.
    Arc { c: V3, axis: V3, r0: V3, angle: f64 },
}

#[derive(Debug, Clone, PartialEq)]
struct PathSeg {
    kind: Kind,
    len: f64,
    /// Frame (`u`, `v`; the tangent is `u x v`) at the start of the segment.
    u: V3,
    v: V3,
}

/// A frame on the path: position and the rotation-minimizing `u`, `v` (tangent `u x v`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathFrame {
    pub p: V3,
    pub u: V3,
    pub v: V3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path3 {
    segs: Vec<PathSeg>,
    length: f64,
}

impl Path3 {
    /// The path of sketch edges `segs` (in sketch coordinates, in walk order) placed in 3D by `place` (the sketch plane's
    /// frame: `place([x, y]) -> point`, with `w` its normal). `u0`, `v0` is the frame at the start: it must be orthonormal,
    /// with `u0 x v0` along the path's first tangent, and the path must be tangent-continuous.
    pub fn new(segs: &[Seg], place: &dyn Fn([f64; 2]) -> V3, w: V3, u0: V3, v0: V3) -> Result<Self, String> {
        if segs.is_empty() {
            return Err("the path has no edges".into());
        }
        let (mut u, mut v) = (u0, v0);
        let mut out: Vec<PathSeg> = Vec::new();
        let mut length = 0.0;
        let dir3 = |a: [f64; 2], b: [f64; 2]| {
            let (pa, pb) = (place(a), place(b));
            [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]]
        };
        for (i, sg) in segs.iter().enumerate() {
            let (kind, len, t0) = match *sg {
                Seg::Spline { .. } => return Err(format!("path edge {} is a spline; a swept path is lines and arcs", i + 1)),
                Seg::Line { a, b } => {
                    let d = dir3(a, b);
                    let len = norm(d);
                    if !(len > 1e-12) {
                        return Err(format!("path edge {} has no length", i + 1));
                    }
                    let t = scale(d, 1.0 / len);
                    (Kind::Line { a: place(a), t }, len, t)
                }
                Seg::Arc { c, r, a0, a1 } => {
                    let angle = a1 - a0;
                    if !(r > 1e-12 && angle.abs() > 1e-12) {
                        return Err(format!("path edge {} has no length", i + 1));
                    }
                    let centre = place(c);
                    let r0 = dir3(c, [c[0] + r * a0.cos(), c[1] + r * a0.sin()]);
                    let axis = scale(w, angle.signum());
                    // Rotating about `axis` by `|angle|` is a counter-clockwise turn about it.
                    let t = scale(cross(axis, r0), 1.0 / norm(r0));
                    (Kind::Arc { c: centre, axis, r0, angle: angle.abs() }, r * angle.abs(), t)
                }
            };
            // The frame entering this edge must have this edge's tangent as its own: tangent continuity.
            let tangent = cross(u, v);
            if dot(tangent, t0) < 1.0 - 1e-9 {
                return Err(if i == 0 {
                    "the path must leave the profile along the normal of the profile's sketch plane".to_string()
                } else {
                    format!("the path has a corner where edge {i} meets edge {}; a swept path must be smooth (use a tangent arc at the bend)", i + 1)
                });
            }
            out.push(PathSeg { kind, len, u, v });
            length += len;
            // Carry the frame to the end of the edge.
            if let Kind::Arc { axis, angle, .. } = &out.last().expect("pushed").kind {
                u = rotate(u, *axis, *angle);
                v = rotate(v, *axis, *angle);
            }
        }
        Ok(Self { segs: out, length })
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    /// Start point of the path.
    pub fn start(&self) -> V3 {
        self.at(0.0).p
    }

    /// Frame at arc length `s` (clamped to the path).
    pub fn at(&self, s: f64) -> PathFrame {
        let mut rest = s.clamp(0.0, self.length);
        for (i, sg) in self.segs.iter().enumerate() {
            if rest <= sg.len || i + 1 == self.segs.len() {
                let t = (rest / sg.len).clamp(0.0, 1.0);
                return match sg.kind {
                    Kind::Line { a, t: dir } => PathFrame { p: add(a, scale(dir, t * sg.len)), u: sg.u, v: sg.v },
                    Kind::Arc { c, axis, r0, angle } => {
                        let phi = angle * t;
                        PathFrame { p: add(c, rotate(r0, axis, phi)), u: rotate(sg.u, axis, phi), v: rotate(sg.v, axis, phi) }
                    }
                };
            }
            rest -= sg.len;
        }
        unreachable!("a path has at least one edge")
    }

    /// Layer boundaries (arc lengths, strictly increasing from 0 to the length) for about `layers` layers: shared out
    /// over the edges by length, at least one per edge and at least one per 22.5 degrees of an arc.
    pub fn levels(&self, layers: usize) -> Vec<f64> {
        let mut out = vec![0.0];
        let mut base = 0.0;
        for sg in &self.segs {
            let mut n = ((layers.max(1) as f64 * sg.len / self.length).round() as usize).max(1);
            if let Kind::Arc { angle, .. } = sg.kind {
                n = n.max((angle / (std::f64::consts::PI / 8.0)).ceil() as usize);
            }
            for k in 1..=n {
                out.push(base + sg.len * k as f64 / n as f64);
            }
            base += sg.len;
        }
        out
    }
}
