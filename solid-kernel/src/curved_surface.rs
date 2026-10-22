//! Sphere, cone and torus faces: the charts the sampled-surface pipeline triangulates in.
//!
//! Every chart is a plane region in physical millimetres, so the planar machinery (exact trim classification, constrained
//! Delaunay triangulation, quality refinement) applies unchanged, and every chart is *angle preserving* where the surface
//! allows it:
//!
//! * **sphere**: stereographic projection from the antipode of the face's mean direction (conformal). A face must stay
//!   within 120 degrees of that direction, so the pole of the projection and the periodic seam never meet the face.
//! * **cone**: the development of the cone into the plane (isometric): slant distance `rho` from the apex and the angle
//!   `-lon * sin(semi_angle)`. The angle is periodic (`2 pi sin(semi_angle)`), so a full band is a sector with two straight
//!   seam sides, exactly as a cylinder's seam.
//! * **torus**: the conformal coordinates `(R u, R w(v))` with `w(v) = 2 r / sqrt(R^2 - r^2) * atan(m tan(v/2))`,
//!   `m = sqrt((R - r) / (R + r))`; both coordinates are periodic.
//!
//! A chart orientation counter-clockwise in the plane is the *outward* geometric normal of the surface, as for the plane and
//! the cylinder. Faces containing a pole, the apex, or wrapping a full sphere or torus are refused, not approximated.
use crate::{
    analytic_step::{AnalyticModel, Frame, Projection, Surface},
    cylindrical_step::unwrap_period,
    planar_step::{classify, TrimLoop},
    step::{Error, ErrorKind},
    V3,
};
use std::f64::consts::{PI, TAU};

fn fail(kind: ErrorKind, text: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: text.into(),
    }
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn finite(p: V3) -> bool {
    p.iter().all(|x| x.is_finite())
}
/// Faces of a sphere must stay this close (angle from the mean direction) to be charted.
const MAX_SPHERE_REACH: f64 = 120. * PI / 180.;
/// A cone point this close (mm) to the apex is the apex.
const APEX_SLANT: f64 = 1e-9;

/// The torus' conformal tube coordinate scale `2 r / sqrt(R^2 - r^2)`, or `None` for a horn or spindle torus.
/// The point of the bilinear patch through the corners `q` (order (0,0), (1,0), (0,1), (1,1)) nearest `p`: its coordinates in
/// the unit square and its distance. Projected Gauss-Newton from the centre and the four corners.
fn bilinear_foot(q: &[V3; 4], p: V3) -> ([f64; 2], f64) {
    let at = |s: f64, t: f64| {
        add(
            add(mul(q[0], (1. - s) * (1. - t)), mul(q[1], s * (1. - t))),
            add(mul(q[2], (1. - s) * t), mul(q[3], s * t)),
        )
    };
    let mut best = ([0.5, 0.5], f64::INFINITY);
    for start in [[0.5, 0.5], [0., 0.], [1., 0.], [0., 1.], [1., 1.]] {
        let [mut s, mut t] = start;
        for _ in 0..12 {
            let r = sub(at(s, t), p);
            // tangents of the patch
            let ds = add(mul(sub(q[1], q[0]), 1. - t), mul(sub(q[3], q[2]), t));
            let dt = add(mul(sub(q[2], q[0]), 1. - s), mul(sub(q[3], q[1]), s));
            let (a, b, c) = (dot(ds, ds), dot(ds, dt), dot(dt, dt));
            let (g0, g1) = (dot(ds, r), dot(dt, r));
            let det = (a + 1e-12 * (a + c)) * (c + 1e-12 * (a + c)) - b * b;
            if !(det.abs() > 1e-300) {
                break;
            }
            let step = [-(c * g0 - b * g1) / det, -(a * g1 - b * g0) / det];
            let (ns, nt) = ((s + step[0]).clamp(0., 1.), (t + step[1]).clamp(0., 1.));
            if (ns - s).abs() + (nt - t).abs() < 1e-12 {
                s = ns;
                t = nt;
                break;
            }
            s = ns;
            t = nt;
        }
        let d = norm(sub(at(s, t), p));
        if d < best.1 {
            best = ([s, t], d);
        }
    }
    best
}
/// Cells per parameter a surface's search grid is refined to, at most.
const MAX_GRID: usize = 192;
/// A nearest-point search that ends farther than this from the point (mm) is repeated from the grid's local minima of the
/// distance (a point 1e-4 off its surface is on it for every purpose; the repeat costs about a dozen more descents).
const MINIMA_MM: f64 = 1e-4;
/// ... and from a subdivision of the grid when that still ends farther than this (2000 cell expansions, ~10 ms). Foreign CAD
/// files put edge samples up to ~2e-3 mm off their surface legitimately, which must not trigger this dearest search.
const CELLS_MM: f64 = 5e-3;
/// Grid local minima of the distance a repeated search descends from.
const MINIMA_STARTS: usize = 16;
fn tube_scale(major: f64, minor: f64) -> Option<f64> {
    (major > minor && minor > 0.).then(|| 2. * minor / (major * major - minor * minor).sqrt())
}
fn tube_modulus(major: f64, minor: f64) -> f64 {
    ((major - minor) / (major + minor)).sqrt()
}

impl Surface {
    /// The periods of the two raw coordinates (`None`: not periodic).
    pub(crate) fn periods(&self) -> [Option<f64>; 2] {
        match self {
            Self::Plane(_) | Self::Sphere { .. } | Self::Param(_) => [None, None],
            Self::Cylinder { radius, .. } => [Some(radius * TAU), None],
            Self::Cone { semi_angle, .. } => [Some(TAU * semi_angle.sin()), None],
            Self::Torus { major, minor, .. } => [
                Some(TAU * major),
                tube_scale(*major, *minor).map(|c| PI * major * c),
            ],
        }
    }
    /// Raw chart coordinates of a world point, periodic ones in their principal range. For the plane, cylinder and sphere
    /// these are the chart; for the cone `(angle, slant distance)`; for the torus the conformal `(U, W)`.
    pub(crate) fn raw(&self, p: V3) -> Option<[f64; 2]> {
        if !finite(p) {
            return None;
        }
        let r = match self {
            Self::Plane(_) | Self::Cylinder { .. } | Self::Sphere { .. } | Self::Param(_) => {
                return self.project(p).map(|q| q.chart)
            }
            Self::Cone {
                frame,
                radius,
                semi_angle,
                ..
            } => {
                let d = sub(p, frame.origin);
                let t = dot(d, frame.z);
                let slant = (radius + t * semi_angle.tan()) / semi_angle.sin();
                // The apex (slant distance zero, within rounding of the exporter) has no angle: 0 stands in and the
                // caller continues the neighbouring sample's angle.
                if slant.abs() <= APEX_SLANT {
                    return Some([0., 0.]);
                }
                if slant < 0. {
                    return None;
                }
                let lon = dot(d, frame.y).atan2(dot(d, frame.x));
                [-lon * semi_angle.sin(), slant]
            }
            Self::Torus {
                frame,
                major,
                minor,
            } => {
                let d = sub(p, frame.origin);
                let (x, y, z) = (dot(d, frame.x), dot(d, frame.y), dot(d, frame.z));
                let q = x.hypot(y);
                if q == 0. {
                    return None;
                }
                let c = tube_scale(*major, *minor)?;
                let phi = 0.5 * z.atan2(q - major);
                let psi = (tube_modulus(*major, *minor) * phi.sin()).atan2(phi.cos());
                [major * y.atan2(x), major * c * psi]
            }
        };
        (r[0].is_finite() && r[1].is_finite()).then_some(r)
    }
    /// The plane chart of raw coordinates.
    pub(crate) fn plane_of_raw(&self, raw: [f64; 2]) -> [f64; 2] {
        match self {
            Self::Cone { .. } => [raw[1] * raw[0].cos(), raw[1] * raw[0].sin()],
            _ => raw,
        }
    }
    /// The same surface with the branch of its polar chart centred on `centre` (a cone; others unchanged).
    pub(crate) fn with_centre(&self, centre: f64) -> Surface {
        match self {
            Self::Cone {
                frame,
                radius,
                semi_angle,
                ..
            } => Self::Cone {
                frame: frame.clone(),
                radius: *radius,
                semi_angle: *semi_angle,
                centre,
            },
            other => other.clone(),
        }
    }
    /// Distance of a world point from the surface.
    pub(crate) fn residual_curved(&self, p: V3) -> Option<f64> {
        let r = match self {
            Self::Param(n) => n.closest(p)?.1,
            Self::Sphere { frame, radius } => (norm(sub(p, frame.origin)) - radius).abs(),
            Self::Cone {
                frame,
                radius,
                semi_angle,
                ..
            } => {
                let d = sub(p, frame.origin);
                let t = dot(d, frame.z);
                let q = dot(d, frame.x).hypot(dot(d, frame.y));
                (q - (radius + t * semi_angle.tan())).abs() * semi_angle.cos()
            }
            Self::Torus {
                frame,
                major,
                minor,
            } => {
                let d = sub(p, frame.origin);
                let q = dot(d, frame.x).hypot(dot(d, frame.y));
                ((q - major).hypot(dot(d, frame.z)) - minor).abs()
            }
            _ => return None,
        };
        r.is_finite().then_some(r)
    }
    /// Distance of a world point from the surface, for every kind.
    pub(crate) fn distance(&self, p: V3) -> Option<f64> {
        match self {
            Self::Plane(f) => Some(dot(sub(p, f.origin), f.z).abs()),
            Self::Cylinder { frame, radius } => {
                let d = sub(p, frame.origin);
                Some((dot(d, frame.x).hypot(dot(d, frame.y)) - radius).abs())
            }
            _ => self.residual_curved(p),
        }
    }
    pub(crate) fn project_curved(&self, p: V3) -> Option<Projection> {
        if let Self::Param(n) = self {
            let (uv, residual_mm) = n.closest(p)?;
            return Some(Projection {
                chart: n.chart_of_param(uv),
                residual_mm,
            });
        }
        let residual_mm = self.residual_curved(p)?;
        let chart = match self {
            Self::Sphere { frame, radius } => {
                let d = sub(p, frame.origin);
                let n = norm(d);
                if n == 0. {
                    return None;
                }
                let u = mul(d, 1. / n);
                let uz = dot(u, frame.z);
                if uz <= -1. + 1e-9 {
                    return None;
                }
                let k = 2. * radius / (1. + uz);
                [k * dot(u, frame.x), k * dot(u, frame.y)]
            }
            Self::Cone { centre, .. } => {
                let raw = self.raw(p)?;
                let period = self.periods()[0]?;
                let beta = raw[0] + ((centre - raw[0]) / period).round() * period;
                self.plane_of_raw([beta, raw[1]])
            }
            Self::Torus { .. } => self.raw(p)?,
            _ => return None,
        };
        (chart[0].is_finite() && chart[1].is_finite()).then_some(Projection { chart, residual_mm })
    }
    pub(crate) fn point_curved(&self, chart: [f64; 2]) -> Option<V3> {
        let p = match self {
            Self::Param(n) => {
                let uv = n.param_of_chart(chart);
                // a point on the boundary of the parameter rectangle may round a few ulp outside it
                let slack = |k: usize| 1e-9 * (n.domain[k][1] - n.domain[k][0]);
                if (0..2).any(|k| uv[k] < n.domain[k][0] - slack(k) || uv[k] > n.domain[k][1] + slack(k)) {
                    return None;
                }
                n.surface.evaluate(n.clamp(uv)).ok()?
            }
            Self::Sphere { frame, radius } => {
                let rho2 = chart[0] * chart[0] + chart[1] * chart[1];
                let uz = (4. * radius * radius - rho2) / (4. * radius * radius + rho2);
                let k = (1. + uz) / (2. * radius);
                let u = add(
                    add(mul(frame.x, chart[0] * k), mul(frame.y, chart[1] * k)),
                    mul(frame.z, uz),
                );
                add(frame.origin, mul(u, *radius))
            }
            Self::Cone {
                frame,
                radius,
                semi_angle,
                centre,
            } => {
                let slant = chart[0].hypot(chart[1]);
                let period = TAU * semi_angle.sin();
                let principal = chart[1].atan2(chart[0]);
                // The sector is narrower than one turn of the chart angle and `centre` is its middle.
                let beta = principal + ((centre - principal) / TAU).round() * TAU;
                let _ = period;
                let lon = -beta / semi_angle.sin();
                let rad = slant * semi_angle.sin();
                let t = (rad - radius) / semi_angle.tan();
                frame.point([rad * lon.cos(), rad * lon.sin(), t])
            }
            Self::Torus {
                frame,
                major,
                minor,
            } => {
                let c = tube_scale(*major, *minor)?;
                let m = tube_modulus(*major, *minor);
                let u = chart[0] / major;
                let psi = chart[1] / (major * c);
                let phi_p = psi.sin().atan2(m * psi.cos());
                let phi = phi_p + TAU * ((psi - phi_p) / TAU).round();
                let v = 2. * phi;
                let q = major + minor * v.cos();
                frame.point([q * u.cos(), q * u.sin(), minor * v.sin()])
            }
            _ => return None,
        };
        finite(p).then_some(p)
    }
    /// The unit normal `S_u x S_v` at a chart point of a spline surface (the chart's counter-clockwise side).
    pub(crate) fn spline_normal(&self, chart: [f64; 2]) -> Option<V3> {
        let Self::Param(n) = self else { return None };
        let uv = n.clamp(n.param_of_chart(chart));
        let (_, du, dv) = n.partials(uv)?;
        let c = cross(du, dv);
        let l = norm(c);
        (l > 0.).then(|| mul(c, 1. / l))
    }
    /// Geometric (not sense-adjusted) unit normal at a world point on the surface, pointing away from the axis or centre.
    pub fn normal_at_world(&self, p: V3) -> Option<V3> {
        let n = match self {
            Self::Param(n) => {
                let (uv, _) = n.closest(p)?;
                let (_, du, dv) = n.partials(uv)?;
                let c = cross(du, dv);
                let l = norm(c);
                if l == 0. {
                    return None;
                }
                mul(c, 1. / l)
            }
            Self::Plane(f) => f.z,
            Self::Cylinder { frame, .. } => {
                let d = sub(p, frame.origin);
                let radial = sub(d, mul(frame.z, dot(d, frame.z)));
                let n = norm(radial);
                if n == 0. {
                    return None;
                }
                mul(radial, 1. / n)
            }
            Self::Sphere { frame, .. } => {
                let d = sub(p, frame.origin);
                let n = norm(d);
                if n == 0. {
                    return None;
                }
                mul(d, 1. / n)
            }
            Self::Cone {
                frame, semi_angle, ..
            } => {
                let d = sub(p, frame.origin);
                let radial = sub(d, mul(frame.z, dot(d, frame.z)));
                let q = norm(radial);
                if q == 0. {
                    return None;
                }
                let e = mul(radial, 1. / q);
                // Perpendicular to the generator (cos a e + ... ) pointing away from the axis: cos(a) e - sin(a) z... the
                // generator is sin(a) e + cos(a) z, so the outward normal is cos(a) e - sin(a) z.
                add(mul(e, semi_angle.cos()), mul(frame.z, -semi_angle.sin()))
            }
            Self::Torus { frame, major, .. } => {
                let d = sub(p, frame.origin);
                let radial = sub(d, mul(frame.z, dot(d, frame.z)));
                let q = norm(radial);
                if q == 0. {
                    return None;
                }
                let e = mul(radial, 1. / q);
                let v = add(mul(e, q - major), mul(frame.z, dot(d, frame.z)));
                let n = norm(v);
                if n == 0. {
                    return None;
                }
                mul(v, 1. / n)
            }
        };
        finite(n).then_some(n)
    }
    /// A sphere re-framed for one face: `z` is the face's own centre, which puts the stereographic pole opposite the face.
    ///
    /// `loops` are the face's bounds as 3D sample loops in traversal order (the bound's own sense applied). The centre is the
    /// direction of the vector area of the largest loop (a seam traversed twice cancels out), on the side the face occupies:
    /// with the face's normal outward (`same_sense`) the interior is to the left of a loop that runs counter-clockwise seen
    /// from outside, i.e. around `+a`; with the normal inward it is the complement. Refused when the face is not contained
    /// within 120 degrees of that centre (a full sphere, a band around the equator, ...).
    pub(crate) fn localized_sphere(
        &self,
        loops: &[Vec<V3>],
        same_sense: bool,
    ) -> Result<Surface, Error> {
        let Self::Sphere { frame, radius } = self else {
            return Ok(self.clone());
        };
        let direction = |p: V3| -> Result<V3, Error> {
            let d = sub(p, frame.origin);
            let n = norm(d);
            if n == 0. || !n.is_finite() {
                return Err(fail(
                    ErrorKind::Geometry,
                    "spherical boundary sample at the sphere's centre",
                ));
            }
            Ok(mul(d, 1. / n))
        };
        let mut best = ([0.; 3], 0.);
        for l in loops {
            let mut a = [0.; 3];
            for i in 0..l.len() {
                let (p, q) = (direction(l[i])?, direction(l[(i + 1) % l.len()])?);
                a = add(a, mul(cross(p, q), 0.5));
            }
            if norm(a) > best.1 {
                best = (a, norm(a));
            }
        }
        if best.1 < 1e-9 {
            return Err(fail(
                ErrorKind::Geometry,
                "a spherical face without an enclosed area (a full sphere?) is not charted",
            ));
        }
        let mut z = mul(best.0, 1. / best.1);
        if !same_sense {
            z = mul(z, -1.);
        }
        let reach_from = |z: V3| -> Result<f64, Error> {
            let mut reach = 0f64;
            for l in loops {
                for p in l {
                    reach = reach.max(dot(direction(*p)?, z).clamp(-1., 1.).acos());
                }
            }
            Ok(reach)
        };
        let reach = reach_from(z)?;
        let other = reach_from(mul(z, -1.))?;
        // When the other side is clearly the smaller one the orientation convention of the file disagrees with this
        // reading; refuse rather than guess (a hemisphere reaches 90 degrees from either side).
        if other + 0.1 < reach && other <= MAX_SPHERE_REACH {
            return Err(fail(
                ErrorKind::Geometry,
                "spherical face orientation and extent disagree: the face is not on the side its bound encloses",
            ));
        }
        if reach > MAX_SPHERE_REACH {
            return Err(fail(
                ErrorKind::Geometry,
                "a spherical face reaches further than 120 degrees from its centre: not charted",
            ));
        }
        // Any x perpendicular to z; the source x keeps the chart stable when it is usable.
        let mut x = sub(frame.x, mul(z, dot(frame.x, z)));
        if norm(x) < 0.3 {
            x = sub(frame.y, mul(z, dot(frame.y, z)));
        }
        let x = mul(x, 1. / norm(x));
        let y = cross(z, x);
        Ok(Self::Sphere {
            frame: Frame {
                origin: frame.origin,
                x,
                y,
                z,
            },
            radius: *radius,
        })
    }
}

/// How a parametric surface is evaluated.
#[derive(Debug)]
enum Geometry {
    Nurbs(crate::nurbs_surface::NurbsSurface),
    /// `S(a, t)` (STEP's order, which fixes the normal): the curve point at `t` turned by the angle `a` about the axis through
    /// `origin` along the unit `axis`.
    Revolution { curve: crate::nurbs::NurbsCurve, origin: V3, axis: V3 },
    /// `S(t, h)`: the curve point at `t` moved by `h` along the unit `direction`, for `h` in `range`.
    Extrusion { curve: crate::nurbs::NurbsCurve, direction: V3, range: [f64; 2] },
}
impl Geometry {
    fn domain(&self) -> [[f64; 2]; 2] {
        match self {
            Self::Nurbs(s) => s.domain(),
            Self::Revolution { curve, .. } => [[0., TAU], curve.domain()],
            Self::Extrusion { curve, range, .. } => [curve.domain(), *range],
        }
    }
    fn evaluate(&self, uv: [f64; 2]) -> Result<V3, Error> {
        match self {
            Self::Nurbs(s) => s.evaluate(uv),
            Self::Revolution { curve, origin, axis } => {
                let d = sub(curve.evaluate(uv[1])?, *origin);
                // Rodrigues' rotation of `d` about `axis` by the angle `uv[1]`
                let (s, c) = uv[0].sin_cos();
                let turned = add(
                    add(mul(d, c), mul(cross(*axis, d), s)),
                    mul(*axis, dot(*axis, d) * (1. - c)),
                );
                Ok(add(*origin, turned))
            }
            Self::Extrusion { curve, direction, .. } => Ok(add(curve.evaluate(uv[0])?, mul(*direction, uv[1]))),
        }
    }
}

/// A face on a parametric surface: a (rational) B-spline surface, a B-spline curve revolved about an axis, or one moved along
/// a direction. The chart is the parameter rectangle scaled by the surface's mean metric (`X = su (u - u0)`,
/// `Y = sv (v - v0)`), so chart lengths are roughly millimetres; it is not conformal. World points are located on the surface
/// by a damped Gauss-Newton search started from the nearest node of a parameter grid.
#[derive(Debug)]
pub struct SplineSurface {
    surface: Geometry,
    domain: [[f64; 2]; 2],
    scale: [f64; 2],
    grid: Vec<([f64; 2], V3)>,
    /// cells of `grid` along each parameter
    grid_n: [usize; 2],
}

impl SplineSurface {
    pub(crate) fn revolution_profile(&self) -> Option<(&crate::nurbs::NurbsCurve, V3, V3)> {
        match &self.surface {
            Geometry::Revolution {
                curve,
                origin,
                axis,
            } => Some((curve, *origin, *axis)),
            _ => None,
        }
    }
    pub(crate) fn revolution_parameter(&self, p: V3, tolerance: f64) -> Option<f64> {
        self.revolution_profile()?;
        self.candidates(p)?.into_iter().find(|candidate| candidate.1 <= tolerance).map(|candidate| candidate.0[1])
    }
    pub(crate) fn new(surface: crate::nurbs_surface::NurbsSurface) -> Result<Self, Error> {
        Self::from_geometry(Geometry::Nurbs(surface))
    }
    /// The surface swept by turning `curve` about the axis through `origin` along `axis` (any length).
    pub(crate) fn revolution(curve: crate::nurbs::NurbsCurve, origin: V3, axis: V3) -> Result<Self, Error> {
        let n = norm(axis);
        if !(n.is_finite() && n > 0.) {
            return Err(fail(ErrorKind::Geometry, "a revolution axis needs a direction"));
        }
        Self::from_geometry(Geometry::Revolution { curve, origin, axis: mul(axis, 1. / n) })
    }
    /// The surface swept by moving `curve` along `direction` (any length) over the heights `range` along it.
    pub(crate) fn extrusion(curve: crate::nurbs::NurbsCurve, direction: V3, range: [f64; 2]) -> Result<Self, Error> {
        let n = norm(direction);
        if !(n.is_finite() && n > 0. && range[0].is_finite() && range[1].is_finite() && range[1] > range[0]) {
            return Err(fail(ErrorKind::Geometry, "an extrusion needs a direction and a height range"));
        }
        Self::from_geometry(Geometry::Extrusion { curve, direction: mul(direction, 1. / n), range })
    }
    fn from_geometry(surface: Geometry) -> Result<Self, Error> {
        let domain = surface.domain();
        let grid_at = |grid_n: [usize; 2]| -> Result<Vec<([f64; 2], V3)>, Error> {
            let mut grid = Vec::with_capacity((grid_n[0] + 1) * (grid_n[1] + 1));
            for j in 0..=grid_n[1] {
                for i in 0..=grid_n[0] {
                    // the last node is the domain's end exactly: `lo + (hi - lo)` can round a ulp above `hi`
                    let node = |k: usize, at: usize| {
                        if at == grid_n[k] {
                            domain[k][1]
                        } else {
                            domain[k][0] + (domain[k][1] - domain[k][0]) * at as f64 / grid_n[k] as f64
                        }
                    };
                    let uv = [node(0, i), node(1, j)];
                    let p = surface.evaluate(uv).map_err(|e| fail(ErrorKind::Geometry, &e.detail))?;
                    grid.push((uv, p));
                }
            }
            Ok(grid)
        };
        // How aliased the grid is in each direction: the longest segment of its parameter lines and the most sharp turns
        // (over 40 degrees between successive segments) on one line.
        let aliasing = |grid: &[([f64; 2], V3)], grid_n: [usize; 2]| {
            let mut longest = [0.0f64; 2];
            let mut sharp = [0usize; 2];
            for k in 0..2 {
                let (along, across) = (grid_n[k] + 1, grid_n[1 - k] + 1);
                for line in 0..across {
                    let at = |s: usize| if k == 0 { grid[line * along + s].1 } else { grid[s * across + line].1 };
                    let mut turns = 0;
                    for s in 0..grid_n[k] {
                        let a = sub(at(s + 1), at(s));
                        longest[k] = longest[k].max(norm(a));
                        if s > 0 {
                            let b = sub(at(s), at(s - 1));
                            let (na, nb) = (norm(a), norm(b));
                            if na > 0. && nb > 0. && dot(a, b) < 0.766 * na * nb {
                                turns += 1;
                            }
                        }
                    }
                    sharp[k] = sharp[k].max(turns);
                }
            }
            (longest, sharp)
        };
        let mut grid_n = [24usize, 24];
        let mut grid = grid_at(grid_n)?;
        // A parameterization far from arc length (a screw thread wound six to twelve times over the parameter range) is
        // aliased by a fixed grid: its nodes then say nothing about where a point of the surface is. A direction is doubled
        // while some line of it turns sharply at four or more nodes (a corner or two is a kink, not aliasing) or a segment
        // is longer than a twelfth of the surface's extent (about twice the segment of a uniform 24-cell grid).
        loop {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for g in &grid {
                for k in 0..3 {
                    lo[k] = lo[k].min(g.1[k]);
                    hi[k] = hi[k].max(g.1[k]);
                }
            }
            let diagonal = norm(sub(hi, lo));
            let (longest, sharp) = aliasing(&grid, grid_n);
            let mut refined = false;
            for k in 0..2 {
                if (longest[k] > diagonal / 12. || sharp[k] >= 4) && grid_n[k] < MAX_GRID {
                    grid_n[k] *= 2;
                    refined = true;
                }
            }
            if !refined || !diagonal.is_finite() {
                break;
            }
            grid = grid_at(grid_n)?;
        }
        // mean metric along each parameter: the mean length of the grid's parameter-line segments per unit parameter
        let mut scale = [0.0; 2];
        let mut count = [0usize; 2];
        for j in 0..=grid_n[1] {
            for i in 0..=grid_n[0] {
                let here = grid[j * (grid_n[0] + 1) + i];
                if i < grid_n[0] {
                    let next = grid[j * (grid_n[0] + 1) + i + 1];
                    scale[0] += norm(sub(next.1, here.1)) / (next.0[0] - here.0[0]);
                    count[0] += 1;
                }
                if j < grid_n[1] {
                    let next = grid[(j + 1) * (grid_n[0] + 1) + i];
                    scale[1] += norm(sub(next.1, here.1)) / (next.0[1] - here.0[1]);
                    count[1] += 1;
                }
            }
        }
        let scale = [scale[0] / count[0] as f64, scale[1] / count[1] as f64];
        if !(scale[0].is_finite() && scale[1].is_finite() && scale[0] > 0. && scale[1] > 0.) {
            return Err(fail(ErrorKind::Geometry, "a degenerate spline surface (zero metric)"));
        }
        Ok(Self { surface, domain, scale, grid, grid_n })
    }
    fn param_of_chart(&self, c: [f64; 2]) -> [f64; 2] {
        [self.domain[0][0] + c[0] / self.scale[0], self.domain[1][0] + c[1] / self.scale[1]]
    }
    fn chart_of_param(&self, uv: [f64; 2]) -> [f64; 2] {
        [self.scale[0] * (uv[0] - self.domain[0][0]), self.scale[1] * (uv[1] - self.domain[1][0])]
    }
    fn clamp(&self, uv: [f64; 2]) -> [f64; 2] {
        [uv[0].clamp(self.domain[0][0], self.domain[0][1]), uv[1].clamp(self.domain[1][0], self.domain[1][1])]
    }
    /// Point and first partials by central differences (one-sided at the domain's edge): derivatives of the exact evaluator
    /// refuse at C0 knots, and the search must run across them.
    fn partials(&self, uv: [f64; 2]) -> Option<(V3, V3, V3)> {
        let p = self.surface.evaluate(uv).ok()?;
        let mut d = [[0.0; 3]; 2];
        for k in 0..2 {
            let span = self.domain[k][1] - self.domain[k][0];
            let h = 1e-6 * span;
            let (lo, hi) = ((uv[k] - h).max(self.domain[k][0]), (uv[k] + h).min(self.domain[k][1]));
            let (mut a, mut b) = (uv, uv);
            a[k] = lo;
            b[k] = hi;
            let (pa, pb) = (self.surface.evaluate(a).ok()?, self.surface.evaluate(b).ok()?);
            d[k] = mul(sub(pb, pa), 1. / (hi - lo));
        }
        Some((p, d[0], d[1]))
    }
    /// The parameter of the surface point nearest `p` and the distance.
    fn closest(&self, p: V3) -> Option<([f64; 2], f64)> {
        self.candidates(p)?.into_iter().next()
    }
    /// Start parameters (with their distance to `p`) for a descent, found by best-first subdivision of the parameter grid.
    /// The nearest grid *nodes* alone miss the foot of `p` when the surface moves a long way inside one grid cell (a
    /// parameterization far from arc length: a point exactly on its spline surface was found 0.4 to 6 mm off it in 26 of
    /// 300 ABC files). Each cell is modelled by the bilinear patch through its corners; the cell whose patch is nearest `p`
    /// is halved first, so the search dives to the foot, and a cell is dropped once its patch, less twice the patch's
    /// deviation from the surface at the cell centre, cannot come within a micron of the best distance seen.
    fn cell_starts(&self, p: V3) -> Vec<(f64, [f64; 2])> {
        const MAX_DEPTH: u32 = 14;
        const EXPANSIONS: usize = 2000;
        const STARTS: usize = 8;
        #[derive(Clone, Copy)]
        struct Cell {
            lo: [f64; 2],
            hi: [f64; 2],
            /// corners (lo,lo), (hi,lo), (lo,hi), (hi,hi)
            q: [V3; 4],
            depth: u32,
            /// the patch's foot of `p` in the cell's own coordinates and its distance
            foot: [f64; 2],
            est: f64,
            /// the least distance any point of the cell is assumed to have from `p`
            lb: f64,
            /// the cell's largest corner-to-corner span
            span: f64,
        }
        impl PartialEq for Cell {
            fn eq(&self, o: &Self) -> bool {
                self.lb.total_cmp(&o.lb).is_eq()
            }
        }
        impl Eq for Cell {}
        impl PartialOrd for Cell {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Cell {
            // a max-heap pops the cell with the least lower bound first
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.lb.total_cmp(&self.lb)
            }
        }
        let cell = |lo: [f64; 2], hi: [f64; 2], q: [V3; 4], depth: u32| -> Option<Cell> {
            let mid = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.];
            let centre = self.surface.evaluate(mid).ok()?;
            let flat = mul(add(add(q[0], q[1]), add(q[2], q[3])), 0.25);
            let sag = norm(sub(centre, flat));
            let ([s, t], est) = bilinear_foot(&q, p);
            let mut span: f64 = 0.;
            for a in 0..4 {
                for b in a + 1..4 {
                    span = span.max(norm(sub(q[a], q[b])));
                }
            }
            Some(Cell { lo, hi, q, depth, foot: [s, t], est, lb: est - 2. * sag, span })
        };
        let (nu, nv) = (self.grid_n[0], self.grid_n[1]);
        let mut heap = std::collections::BinaryHeap::with_capacity(nu * nv);
        let mut ub = f64::INFINITY;
        for j in 0..nv {
            for i in 0..nu {
                let at = |a: usize, b: usize| self.grid[b * (nu + 1) + a];
                let (c00, c10, c01, c11) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
                if let Some(c) = cell(c00.0, c11.0, [c00.1, c10.1, c01.1, c11.1], 0) {
                    ub = ub.min(c.q.iter().map(|q| norm(sub(*q, p))).fold(f64::INFINITY, f64::min));
                    heap.push(c);
                }
            }
        }
        let mut leaves: Vec<Cell> = Vec::new();
        for _ in 0..EXPANSIONS {
            let Some(c) = heap.pop() else { break };
            if c.lb > ub + 1e-3 {
                // every cell left has a larger bound: none comes within a micron of the best distance seen
                break;
            }
            if c.depth >= MAX_DEPTH || c.span < 1e-7 {
                leaves.push(c);
                continue;
            }
            let mid = [(c.lo[0] + c.hi[0]) / 2., (c.lo[1] + c.hi[1]) / 2.];
            let ev = |uv: [f64; 2]| self.surface.evaluate(uv).ok();
            let (Some(bottom), Some(top), Some(left), Some(right), Some(centre)) = (
                ev([mid[0], c.lo[1]]),
                ev([mid[0], c.hi[1]]),
                ev([c.lo[0], mid[1]]),
                ev([c.hi[0], mid[1]]),
                ev(mid),
            ) else {
                continue;
            };
            let d = c.depth + 1;
            for child in [
                cell(c.lo, mid, [c.q[0], bottom, left, centre], d),
                cell([mid[0], c.lo[1]], [c.hi[0], mid[1]], [bottom, c.q[1], centre, right], d),
                cell([c.lo[0], mid[1]], [mid[0], c.hi[1]], [left, centre, c.q[2], top], d),
                cell(mid, c.hi, [centre, right, top, c.q[3]], d),
            ]
            .into_iter()
            .flatten()
            {
                // the patch is an estimate of the surface, not an upper bound: only evaluated points lower `ub`
                ub = ub.min(child.q.iter().map(|q| norm(sub(*q, p))).fold(f64::INFINITY, f64::min));
                heap.push(child);
            }
        }
        // cells still open are as good a start as a leaf
        leaves.extend(std::iter::from_fn(|| heap.pop()).take(STARTS));
        leaves.sort_by(|a, b| a.est.total_cmp(&b.est));
        leaves.truncate(STARTS);
        leaves
            .iter()
            .filter_map(|c| {
                let uv = self.clamp([c.lo[0] + c.foot[0] * (c.hi[0] - c.lo[0]), c.lo[1] + c.foot[1] * (c.hi[1] - c.lo[1])]);
                let q = self.surface.evaluate(uv).ok()?;
                Some((norm(sub(q, p)), uv))
            })
            .collect()
    }
    /// The distinct nearest points of the surface to `p` (parameter and distance), nearest first, within a micron of the best.
    /// The search refines from several of the nearest grid nodes: a surface closed in a parameter has coincident points on
    /// both sides of its seam, and the clamped search from one side cannot cross it.
    fn candidates(&self, p: V3) -> Option<Vec<([f64; 2], f64)>> {
        const STARTS: usize = 6;
        let mut nodes: Vec<(f64, [f64; 2])> = self.grid.iter().map(|g| (norm(sub(g.1, p)), g.0)).collect();
        let k = STARTS.min(nodes.len());
        nodes.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
        let mut found = Vec::with_capacity(k);
        for &(d, uv) in &nodes[..k] {
            found.push(self.refine(p, uv, d)?);
        }
        found.sort_by(|a, b| a.1.total_cmp(&b.1));
        // The descents from the nearest nodes stall in a wrong basin when they all start in one valley of the distance (a
        // parameterization far from arc length folds the surface over p many times): a point that is not (nearly) on the
        // surface is searched again, first from the grid's local minima of the distance, one per valley, and then from the
        // subdivided grid, before it counts as off the surface.
        if found[0].1 > MINIMA_MM {
            let (nu, nv) = (self.grid_n[0] + 1, self.grid_n[1] + 1);
            let dist: Vec<f64> = self.grid.iter().map(|g| norm(sub(g.1, p))).collect();
            let mut minima: Vec<(f64, [f64; 2])> = Vec::new();
            for j in 0..nv {
                for i in 0..nu {
                    let d = dist[j * nu + i];
                    let is_min = (j.saturating_sub(1)..=(j + 1).min(nv - 1)).all(|jj| {
                        (i.saturating_sub(1)..=(i + 1).min(nu - 1)).all(|ii| {
                            let other = dist[jj * nu + ii];
                            // ties go to the lower index so that a flat valley floor yields one node
                            other > d || (other == d && jj * nu + ii >= j * nu + i)
                        })
                    });
                    if is_min {
                        minima.push((d, self.grid[j * nu + i].0));
                    }
                }
            }
            minima.sort_by(|a, b| a.0.total_cmp(&b.0));
            for &(d, uv) in minima.iter().take(MINIMA_STARTS) {
                found.push(self.refine(p, uv, d)?);
            }
            found.sort_by(|a, b| a.1.total_cmp(&b.1));
            if found[0].1 > CELLS_MM {
                for (d, uv) in self.cell_starts(p) {
                    found.push(self.refine(p, uv, d)?);
                }
                found.sort_by(|a, b| a.1.total_cmp(&b.1));
            }
        }
        let best = found[0].1;
        let span = [self.domain[0][1] - self.domain[0][0], self.domain[1][1] - self.domain[1][0]];
        let mut out: Vec<([f64; 2], f64)> = Vec::new();
        for f in found {
            let distinct = out.iter().all(|o| (o.0[0] - f.0[0]).abs() > 1e-7 * span[0] || (o.0[1] - f.0[1]).abs() > 1e-7 * span[1]);
            if f.1 <= best + 1e-3 && distinct {
                out.push(f);
            }
        }
        Some(out)
    }
    /// The chart points of a closed sequence of world points on the surface (a face boundary loop), continuing each one from
    /// the previous: a point on a seam has one chart image per side of it, and the loop follows the image nearest the sample
    /// before it. The first point has no predecessor, so each of its images is tried and the loop of least chart length kept.
    pub(crate) fn chart_loop(&self, points: &[V3]) -> Option<Vec<[f64; 2]>> {
        let first = self.candidates(*points.first()?)?;
        let mut best: Option<(f64, Vec<[f64; 2]>)> = None;
        for (uv0, _) in first {
            let mut chart = vec![self.chart_of_param(uv0)];
            for p in &points[1..] {
                let hint = *chart.last().unwrap();
                let mut found = self.candidates(*p)?;
                // the continuation of the previous sample's own parameter is a candidate too
                let hint_uv = self.param_of_chart(hint);
                let hd = norm(sub(self.surface.evaluate(self.clamp(hint_uv)).ok()?, *p));
                found.push(self.refine(*p, self.clamp(hint_uv), hd)?);
                let nearest = found[0].1;
                let pick = found
                    .iter()
                    .filter(|f| f.1 <= nearest + 1e-3)
                    .map(|f| self.chart_of_param(f.0))
                    .min_by(|a, b| {
                        (a[0] - hint[0]).hypot(a[1] - hint[1]).total_cmp(&(b[0] - hint[0]).hypot(b[1] - hint[1]))
                    })?;
                chart.push(pick);
            }
            let n = chart.len();
            let length: f64 = (0..n)
                .map(|i| {
                    let (a, b) = (chart[i], chart[(i + 1) % n]);
                    (a[0] - b[0]).hypot(a[1] - b[1])
                })
                .sum();
            if best.as_ref().map_or(true, |b| length < b.0) {
                best = Some((length, chart));
            }
        }
        best.map(|b| b.1)
    }
    /// Damped Gauss-Newton descent of the squared distance to `p` from `start` (at distance `start_d`).
    fn refine(&self, p: V3, start: [f64; 2], start_d: f64) -> Option<([f64; 2], f64)> {
        let mut uv = start;
        let mut f = start_d * start_d;
        for _ in 0..80 {
            let (q, du, dv) = self.partials(uv)?;
            let r = sub(q, p);
            let (a, b, c) = (dot(du, du), dot(du, dv), dot(dv, dv));
            let (g0, g1) = (dot(du, r), dot(dv, r));
            let mu = 1e-9 * (a + c).max(1e-300);
            let det = (a + mu) * (c + mu) - b * b;
            if det.abs() < 1e-300 {
                break;
            }
            let step = [-((c + mu) * g0 - b * g1) / det, -((a + mu) * g1 - b * g0) / det];
            let mut t = 1.0;
            let mut moved = false;
            while t > 1e-6 {
                let trial = self.clamp([uv[0] + t * step[0], uv[1] + t * step[1]]);
                if let Ok(q2) = self.surface.evaluate(trial) {
                    let f2 = norm(sub(q2, p)).powi(2);
                    if f2 < f {
                        let dx = (trial[0] - uv[0]).abs() / (self.domain[0][1] - self.domain[0][0]) + (trial[1] - uv[1]).abs() / (self.domain[1][1] - self.domain[1][0]);
                        uv = trial;
                        let improvement = f - f2;
                        f = f2;
                        moved = dx > 1e-15 && improvement > 1e-30 * (1. + f);
                        break;
                    }
                }
                t *= 0.5;
            }
            if !moved {
                break;
            }
        }
        Some((uv, f.sqrt()))
    }
}

/// The boundary of a cone or torus face in its plane chart, with the middle of the (unwrapped) first raw coordinate.
#[derive(Debug, Clone)]
pub(crate) struct PeriodicTrims {
    pub boundary: TrimLoop,
    pub centre: f64,
}

impl AnalyticModel {
    /// Unwrap the single explicitly seamed boundary of a cone or torus face: every sample continues the previous one in both
    /// periodic raw coordinates, the loop must close without winding, and each coordinate must stay within one period.
    pub(crate) fn sampled_periodic_trims(
        &self,
        face_id: u64,
        max_work: usize,
    ) -> Result<PeriodicTrims, Error> {
        let face = self
            .body()
            .faces
            .get(&face_id)
            .ok_or_else(|| fail(ErrorKind::Geometry, "unknown face"))?;
        let surface = self.surface(face_id).unwrap();
        if !matches!(surface, Surface::Cone { .. } | Surface::Torus { .. }) {
            return Err(fail(ErrorKind::Geometry, "require a cone or torus"));
        }
        let periods = surface.periods();
        if face.bounds.len() != 1 {
            return Err(fail(
                ErrorKind::Geometry,
                "holes and unseamed bands on cones and tori are not implemented",
            ));
        }
        let bound = &face.bounds[0];
        let mut remaining = max_work;
        let mut raws: Vec<[f64; 2]> = Vec::new();
        for usage in &bound.edges {
            let samples = self.edge_points(usage.edge).unwrap();
            remaining = remaining
                .checked_sub(samples.len() - 1)
                .ok_or_else(|| fail(ErrorKind::Limit, "periodic trim expansion budget exceeded"))?;
            for i in 0..samples.len() - 1 {
                let p = samples[if usage.forward {
                    i
                } else {
                    samples.len() - 1 - i
                }];
                let mut raw = surface.raw(p).ok_or_else(|| {
                    fail(
                        ErrorKind::Geometry,
                        "unresolved cone/torus projection (apex or axis in the face?)",
                    )
                })?;
                if let Some(last) = raws.last() {
                    if matches!(surface, Surface::Cone { .. }) && raw[1] == 0. {
                        raw[0] = last[0];
                    } else {
                        for k in 0..2 {
                            if let Some(period) = periods[k] {
                                raw[k] = unwrap_period(raw[k], last[k], period)?;
                            }
                        }
                    }
                }
                raws.push(raw);
            }
        }
        if raws.len() < 3 {
            return Err(fail(
                ErrorKind::Geometry,
                "insufficient cone/torus trim samples",
            ));
        }
        let cone = matches!(surface, Surface::Cone { .. });
        // The apex has no angle: the boundary may turn around it by up to one period without closing in the angle (a full
        // cone's seam runs up to the apex and back along the same line), so the closure test is waived there and the
        // single-turn extent test below, plus the polygon classification, stay in force.
        let apex: Vec<usize> = (0..raws.len())
            .filter(|&i| cone && raws[i][1] == 0.)
            .collect();
        for &i in &apex {
            let neighbour = (0..i)
                .rev()
                .find(|j| raws[*j][1] != 0.)
                .or_else(|| (i + 1..raws.len()).find(|j| raws[*j][1] != 0.))
                .ok_or_else(|| fail(ErrorKind::Geometry, "a cone face reduced to its apex"))?;
            raws[i][0] = raws[neighbour][0];
        }
        for k in 0..2 {
            let Some(period) = periods[k] else { continue };
            let first = raws[0][k];
            let closing = unwrap_period(first, raws.last().unwrap()[k], period)?;
            let roundoff = 64. * f64::EPSILON * period;
            if (closing - first).abs() > roundoff && (k != 0 || apex.is_empty()) {
                return Err(fail(
                    ErrorKind::Geometry,
                    "boundary winds without a chart-closing seam",
                ));
            }
            let min = raws.iter().map(|r| r[k]).fold(f64::INFINITY, f64::min);
            let max = raws.iter().map(|r| r[k]).fold(f64::NEG_INFINITY, f64::max);
            if max - min > period + roundoff {
                return Err(fail(
                    ErrorKind::Geometry,
                    "cannot certify a single-turn chart of a cone/torus face",
                ));
            }
        }
        let centre = {
            let lo = raws.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
            let hi = raws.iter().map(|r| r[0]).fold(f64::NEG_INFINITY, f64::max);
            0.5 * (lo + hi)
        };
        let mut points: Vec<[f64; 2]> = raws.iter().map(|r| surface.plane_of_raw(*r)).collect();
        if !bound.forward {
            points.reverse();
        }
        let mut loops = [TrimLoop {
            bound_id: bound.id,
            points,
            is_outer: false,
        }];
        classify(&mut loops, remaining)?;
        Ok(PeriodicTrims {
            boundary: loops.into_iter().next().unwrap(),
            centre,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame {
        Frame {
            origin: [1., -2., 3.],
            x: [1., 0., 0.],
            y: [0., 1., 0.],
            z: [0., 0., 1.],
        }
    }
    fn surfaces() -> Vec<Surface> {
        vec![
            Surface::Sphere {
                frame: frame(),
                radius: 4.,
            },
            Surface::Cone {
                frame: frame(),
                radius: 5.,
                semi_angle: 0.6,
                centre: 0.4,
            },
            Surface::Torus {
                frame: frame(),
                major: 6.,
                minor: 2.,
            },
        ]
    }
    /// A sawtooth extruded along z: a parameterization with 60 teeth over the parameter range, the shape of a screw thread's
    /// flank. A fixed 24-cell grid aliases it (a node every five curve units), so the nearest nodes are on other teeth.
    fn sawtooth(teeth: usize) -> SplineSurface {
        let n = 2 * teeth + 1;
        let controls: Vec<V3> = (0..n).map(|i| [i as f64, if i % 2 == 1 { 4. } else { 0. }, 0.]).collect();
        let mut knots = vec![0.0];
        knots.extend((0..n).map(|i| i as f64 / (n - 1) as f64));
        knots.push(1.0);
        let curve =
            crate::nurbs::NurbsCurve::new_clamped(1, knots, controls, vec![1.; n], crate::nurbs::Limits::default()).unwrap();
        SplineSurface::extrusion(curve, [0., 0., 1.], [0., 10.]).unwrap()
    }
    #[test]
    fn a_point_on_a_many_toothed_surface_is_found_on_it() {
        let s = sawtooth(60);
        let mut worst: f64 = 0.;
        // 997 and 389 are coprime to the 120 flanks and to 10: the samples fall on every flank at varied heights
        for k in 0..997usize {
            let (t, h) = ((k * 389 % 997) as f64 / 997., (k % 41) as f64 / 4.);
            let p = s.surface.evaluate([t, h]).unwrap();
            let (_, d) = s.closest(p).unwrap();
            worst = worst.max(d);
        }
        assert!(worst < 1e-6, "a point on the surface was found {worst} mm off it");
    }
    #[test]
    fn a_point_off_a_many_toothed_surface_is_still_off_it() {
        // the repeated search must not invent a foot: a point 3 mm above the top edge (height 10) is 3 mm from the surface
        let s = sawtooth(60);
        let p = s.surface.evaluate([0.5, 10.]).unwrap();
        let (_, d) = s.closest([p[0], p[1], p[2] + 3.]).unwrap();
        assert!((d - 3.).abs() < 1e-6, "{d}");
    }
    #[test]
    fn point_and_raw_coordinates_are_inverse_on_each_surface() {
        for s in surfaces() {
            let charts: &[[f64; 2]] = match s {
                Surface::Sphere { .. } => &[[0., 0.], [1.5, -2.], [-3., 4.], [5.5, 0.1]],
                Surface::Cone { .. } => &[[7., 0.2], [9., 0.5], [12., 0.9]],
                _ => &[[0.3, 0.2], [5., -1.], [20., 3.], [-7., -4.]],
            };
            for chart in charts {
                let plane = match s {
                    Surface::Cone { .. } => s.plane_of_raw([chart[1], chart[0]]),
                    _ => *chart,
                };
                let p = s.point(plane).expect("point");
                assert!(s.residual_curved(p).unwrap() < 1e-9, "{s:?} {chart:?}");
                // Periodic raw coordinates come back in their principal range: compare the world points.
                let back = s
                    .point(s.project(p).expect("project").chart)
                    .expect("again");
                assert!(norm(sub(back, p)) < 1e-9, "{s:?}: {plane:?}");
            }
        }
    }
    #[test]
    fn counter_clockwise_chart_triangles_have_the_outward_normal() {
        for s in surfaces() {
            let (a, b, c): ([f64; 2], [f64; 2], [f64; 2]) = match s {
                Surface::Sphere { .. } => ([0.2, 0.1], [1.4, 0.2], [0.6, 1.3]),
                Surface::Cone { .. } => ([7., 0.1], [8., 0.1], [7.5, 0.4]),
                _ => ([0.5, 0.2], [1.0, 0.2], [0.7, 0.6]),
            };
            let plane = |q: [f64; 2]| match s {
                Surface::Cone { .. } => s.plane_of_raw([q[1], q[0]]),
                _ => q,
            };
            let (a, b, c) = (plane(a), plane(b), plane(c));
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            let [pa, pb, pc] = [a, b, c].map(|q| s.point(q).unwrap());
            let n = [
                (pb[1] - pa[1]) * (pc[2] - pa[2]) - (pb[2] - pa[2]) * (pc[1] - pa[1]),
                (pb[2] - pa[2]) * (pc[0] - pa[0]) - (pb[0] - pa[0]) * (pc[2] - pa[2]),
                (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]),
            ];
            let centroid = [
                (pa[0] + pb[0] + pc[0]) / 3.,
                (pa[1] + pb[1] + pc[1]) / 3.,
                (pa[2] + pb[2] + pc[2]) / 3.,
            ];
            let out = s.normal_at_world(centroid).unwrap();
            assert!(
                area.signum() * dot(n, out) > 0.,
                "{s:?}: chart area {area}, normal . outward {}",
                dot(n, out)
            );
        }
    }
    #[test]
    fn sphere_and_torus_charts_are_conformal_and_the_cone_chart_isometric() {
        // Edge lengths ratio of a tiny chart triangle against its surface triangle must be the same for all three sides.
        for s in surfaces() {
            let (o, h) = match s {
                Surface::Sphere { .. } => ([1.5, -1.0], 1e-5),
                Surface::Cone { .. } => (s.plane_of_raw([0.3, 9.]), 1e-5),
                _ => ([4.0, 0.8], 1e-5),
            };
            let pts = [[0., 0.], [h, 0.], [0., h]].map(|d| [o[0] + d[0], o[1] + d[1]]);
            let w = pts.map(|q| s.point(q).unwrap());
            let ratio = |i: usize, j: usize| {
                let c = (pts[i][0] - pts[j][0]).hypot(pts[i][1] - pts[j][1]);
                norm(sub(w[i], w[j])) / c
            };
            let (a, b) = (ratio(0, 1), ratio(0, 2));
            assert!((a / b - 1.).abs() < 1e-3, "{s:?}: {a} vs {b}");
            if matches!(s, Surface::Cone { .. }) {
                assert!((a - 1.).abs() < 1e-4, "isometric development: {a}");
            }
        }
    }
    #[test]
    fn a_sphere_face_is_reframed_around_its_own_centre_or_refused() {
        let s = Surface::Sphere {
            frame: frame(),
            radius: 2.,
        };
        let on = |lat: f64, lon: f64| {
            add(
                frame().origin,
                mul(
                    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()],
                    2.,
                ),
            )
        };
        let ring =
            |lat: f64| -> Vec<V3> { (0..12).map(|i| on(lat, i as f64 * TAU / 12.)).collect() };
        // A cap around the pole of the source frame, loop counter-clockwise seen from outside: charted around its own centre
        // (the pole here; the test then tilts the same cap).
        let Surface::Sphere { frame: f, .. } = s.localized_sphere(&[ring(1.0)], true).unwrap()
        else {
            panic!()
        };
        assert!(f.z[2] > 0.999, "{:?}", f.z);
        // The same loop with the face normal inward bounds the complement: 1.0 rad of latitude leaves 147 degrees of cap.
        assert!(s.localized_sphere(&[ring(1.0)], false).is_err());
        // The equator bounds a hemisphere on either side, 90 degrees from its centre.
        assert!(s.localized_sphere(&[ring(0.)], true).is_ok());
        // A cap wider than 120 degrees (a loop at latitude -60 degrees seen counter-clockwise from above encloses 150).
        assert!(s.localized_sphere(&[ring(-1.05)], true).is_err());
        // A seam traversed twice adds nothing.
        let mut seamed = ring(1.0);
        seamed.extend([on(1.2, 0.), on(1.4, 0.), on(1.2, 0.)]);
        assert!(s.localized_sphere(&[seamed], true).is_ok());
        // No loop at all: a full sphere.
        assert!(s.localized_sphere(&[], true).is_err());
    }
}
