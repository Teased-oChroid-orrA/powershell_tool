//! Assembly feature detection, geometry only (epic Phase A, `docs/epic-assembly-feature-advice.md`): what the faces of one or
//! more placed bodies are, as plain numbers in the root frame (millimetres). No advice, no solver, no UI: a pure function of the
//! models [`AnalyticModel::read_all`] returns, in a deterministic order (body, then face id).
//!
//! A face key is `(body index, face id)`. STEP face ids repeat across the instances of one part, so the id alone names nothing
//! in an assembly (`docs/epic-assembly-feature-advice.md`, spike point 2).
//!
//! Tolerances are never tuned: a distance comparison uses the models' own `geometry_tolerance_mm` (the larger of the two when two
//! bodies are compared), an angle comparison the angle that distance subtends over the longer length involved. The one table
//! lookup window, [`TABLE_MATCH_MM`], is stated where it is defined.

use crate::analytic_step::{AnalyticModel, Surface};
use crate::face_identity::Kind;
use crate::step::Error;
use crate::V3;

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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

/// One face of one placed body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceKey {
    /// Index into the slice of models, which is the instance order of `read_all`.
    pub body: usize,
    /// The STEP face entity id (unique only inside one body).
    pub face: u64,
}

/// Whether the material of a round face lies outside it (a bore, a pocket) or inside it (a pin, a boss, a rounded edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The outward normal points toward the axis: a hole wall.
    Concave,
    /// The outward normal points away from the axis: a boss.
    Convex,
}

/// What a face is. Anything that is not an analytic plane, cylinder or cone is [`FaceGeometry::Other`], loudly: it is never
/// guessed to be one of them.
#[derive(Debug, Clone, PartialEq)]
pub enum FaceGeometry {
    Plane {
        /// Outward unit normal.
        normal: V3,
        /// `normal . p` for a point `p` of the plane.
        offset: f64,
        /// Area of the face: the outer loop minus its holes (millimetres squared), of the sampled boundary polygon: a curved
        /// boundary reads inside by at most its perimeter times the model's chord tolerance.
        area: f64,
        /// The outer boundary and the hole boundaries as closed polylines (the closing point is not repeated).
        outer: Vec<V3>,
        holes: Vec<Vec<V3>>,
    },
    Cylinder {
        /// A point of the axis and the axis direction (unit, sign fixed by [`crate::face_identity`]'s convention).
        axis_point: V3,
        axis: V3,
        radius: f64,
        side: Side,
        /// Extent of the face along `axis` from `axis_point`.
        range: [f64; 2],
        /// The angles about `axis` the face covers, as merged intervals in `[0, 2 pi]` (see [`stable_basis`] for the zero
        /// direction and the sense). Their total length is the face's wrap: 2 pi for a full ring, pi for a half face of a split
        /// bore, a quarter turn for an inside fillet.
        arcs: Vec<[f64; 2]>,
    },
    Cone {
        axis_point: V3,
        axis: V3,
        /// Half angle in radians.
        half_angle: f64,
        side: Side,
        range: [f64; 2],
        /// Radius at `range[0]` and at `range[1]` (0 at an apex).
        radius_at_ends: [f64; 2],
    },
    Other(Kind),
}

impl FaceGeometry {
    /// The angle a cylinder face wraps about its axis (radians); `None` for other geometry.
    pub fn wrap(&self) -> Option<f64> {
        match self {
            FaceGeometry::Cylinder { arcs, .. } => Some(arcs.iter().map(|a| a[1] - a[0]).sum()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassifiedFace {
    pub key: FaceKey,
    pub geometry: FaceGeometry,
    /// Bounding box of the face's boundary samples.
    pub lo: V3,
    pub hi: V3,
}

/// Closed polyline of one face bound: the edge samples in the direction of use, each joint once.
fn bound_polyline(model: &AnalyticModel, edges: &[crate::step_topology::EdgeUse]) -> Vec<V3> {
    let mut out: Vec<V3> = Vec::new();
    for u in edges {
        let Some(points) = model.edge_points(u.edge) else { continue };
        let ordered: Vec<V3> = if u.forward { points.to_vec() } else { points.iter().rev().copied().collect() };
        for p in ordered {
            if out.last().is_none_or(|q| norm(sub(*q, p)) > 0.0) {
                out.push(p);
            }
        }
    }
    if out.len() > 1 && norm(sub(out[0], out[out.len() - 1])) == 0.0 {
        out.pop();
    }
    out
}

/// Two orthonormal vectors spanning the plane with unit normal `n`.
pub(crate) fn plane_basis(n: V3) -> (V3, V3) {
    let k = if n[0].abs() <= n[1].abs() && n[0].abs() <= n[2].abs() { 0 } else if n[1].abs() <= n[2].abs() { 1 } else { 2 };
    let mut e = [0.0; 3];
    e[k] = 1.0;
    let u = sub(e, mul(n, dot(e, n)));
    let u = mul(u, 1.0 / norm(u));
    (u, cross(n, u))
}

/// Shoelace area (absolute) of a closed polyline projected on the plane basis `(u, v)`.
fn loop_area(points: &[V3], u: V3, v: V3) -> f64 {
    let n = points.len();
    let mut s = 0.0;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        s += dot(a, u) * dot(b, v) - dot(b, u) * dot(a, v);
    }
    0.5 * s.abs()
}

/// Axial extent and the radius at both axial ends of boundary samples about the line `(point, axis)`.
fn axial_profile(points: &[V3], point: V3, axis: V3) -> ([f64; 2], [f64; 2]) {
    let (mut lo, mut hi) = ((f64::INFINITY, 0.0), (f64::NEG_INFINITY, 0.0));
    for p in points {
        let d = sub(*p, point);
        let t = dot(d, axis);
        let r = norm(sub(d, mul(axis, t)));
        if t < lo.0 {
            lo = (t, r);
        }
        if t > hi.0 {
            hi = (t, r);
        }
    }
    ([lo.0, hi.0], [lo.1, hi.1])
}

/// A reference direction and its quarter turn about the unit `axis`, the same for axes that differ by a hair: world x unless
/// the axis is within about 26 degrees of it, then world y. (A basis chosen by the smallest component would jump a quarter turn
/// between two nearly equal axes; the angular intervals of the faces of one bore must agree.)
fn stable_basis(axis: V3) -> (V3, V3) {
    let e = if axis[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = sub(e, mul(axis, dot(e, axis)));
    let u = mul(u, 1.0 / norm(u));
    (u, cross(axis, u))
}

const TAU: f64 = std::f64::consts::TAU;

/// Merge intervals of `[0, 2 pi]` that touch or lie within `gap` of each other.
fn merge_arcs(mut arcs: Vec<[f64; 2]>, gap: f64) -> Vec<[f64; 2]> {
    arcs.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut out: Vec<[f64; 2]> = Vec::new();
    for a in arcs {
        match out.last_mut() {
            Some(last) if a[0] <= last[1] + gap => last[1] = last[1].max(a[1]),
            _ => out.push(a),
        }
    }
    out
}

/// The angular intervals a face covers about the line `(point, axis)`: each pair of consecutive boundary samples of an edge covers
/// the angle between them (the short way round), and the intervals of all edges are merged. Samples on the axis have no angle and
/// are skipped. A full ring is covered once around because its rim chains closed, whatever the rim's shape (an oblique rim is an
/// ellipse); a quarter fillet, whose rim is an open arc, covers a quarter turn.
fn arcs_of(model: &AnalyticModel, edges: &[crate::step_topology::EdgeUse], point: V3, axis: V3, tol: f64, radius: f64) -> Vec<[f64; 2]> {
    let (u, v) = stable_basis(axis);
    let mut arcs = Vec::new();
    for e in edges {
        let Some(points) = model.edge_points(e.edge) else { continue };
        let polar: Vec<Option<f64>> = points
            .iter()
            .map(|p| {
                let d = sub(*p, point);
                let r = norm(sub(d, mul(axis, dot(d, axis))));
                (r > tol).then(|| dot(d, v).atan2(dot(d, u)))
            })
            .collect();
        for w in polar.windows(2) {
            let (Some(a), Some(b)) = (w[0], w[1]) else { continue };
            let mut d = b - a;
            if d > std::f64::consts::PI {
                d -= TAU;
            } else if d <= -std::f64::consts::PI {
                d += TAU;
            }
            let (start, len) = if d >= 0.0 { (a, d) } else { (b, -d) };
            let start = start.rem_euclid(TAU);
            if start + len > TAU {
                arcs.push([start, TAU]);
                arcs.push([0.0, start + len - TAU]);
            } else {
                arcs.push([start, start + len]);
            }
        }
    }
    merge_arcs(arcs, tol / radius.max(tol))
}

/// Classify every face of every body. Errors only when a model's faces cannot be signed (see [`AnalyticModel::face_signatures`]).
pub fn classify_faces(models: &[AnalyticModel]) -> Result<Vec<ClassifiedFace>, Error> {
    let mut out = Vec::new();
    for (body, model) in models.iter().enumerate() {
        let signatures = model.face_signatures()?;
        for (id, face) in &model.body().faces {
            let sig = &signatures[id];
            let samples: Vec<V3> = face
                .bounds
                .iter()
                .flat_map(|b| &b.edges)
                .filter_map(|u| model.edge_points(u.edge))
                .flatten()
                .copied()
                .collect();
            let side = if sig.convex { Side::Convex } else { Side::Concave };
            let geometry = match model.surface(*id) {
                Some(Surface::Plane(_)) => {
                    let (u, v) = plane_basis(sig.direction);
                    let loops: Vec<Vec<V3>> = face.bounds.iter().map(|b| bound_polyline(model, &b.edges)).collect();
                    // FACE_BOUND does not say which loop is outer; the outer one is the one that encloses the most area.
                    let areas: Vec<f64> = loops.iter().map(|l| loop_area(l, u, v)).collect();
                    let outer_index = (0..loops.len()).max_by(|a, b| areas[*a].total_cmp(&areas[*b])).unwrap_or(0);
                    let mut holes = Vec::new();
                    let mut outer = Vec::new();
                    for (i, l) in loops.into_iter().enumerate() {
                        if i == outer_index {
                            outer = l;
                        } else {
                            holes.push(l);
                        }
                    }
                    let area = areas[outer_index] - areas.iter().enumerate().filter(|(i, _)| *i != outer_index).map(|(_, a)| a).sum::<f64>();
                    let offset = dot(sig.direction, sig.anchor);
                    FaceGeometry::Plane { normal: sig.direction, offset, area, outer, holes }
                }
                Some(Surface::Cylinder { .. }) => {
                    let (range, _) = axial_profile(&samples, sig.anchor, sig.direction);
                    let edges: Vec<_> = face.bounds.iter().flat_map(|b| b.edges.iter().cloned()).collect();
                    let arcs = arcs_of(model, &edges, sig.anchor, sig.direction, model.geometry_tolerance_mm(), sig.size);
                    FaceGeometry::Cylinder { axis_point: sig.anchor, axis: sig.direction, radius: sig.size, side, range, arcs }
                }
                Some(Surface::Cone { .. }) => {
                    // the signature's anchor is the axis point nearest the origin; the profile is taken about that line
                    let (range, radius_at_ends) = axial_profile(&samples, sig.anchor, sig.direction);
                    FaceGeometry::Cone { axis_point: sig.anchor, axis: sig.direction, half_angle: sig.size, side, range, radius_at_ends }
                }
                // A rational B-spline (or swept) surface that is a right circular cylinder to within the model's tolerance is a
                // cylinder: Open CASCADE writes every cylinder of AS1 as two rational cubic Bezier halves. The surface sense and the
                // radial direction at a boundary sample give the side, as for an analytic cylinder.
                Some(surface @ Surface::Param(spline)) => match spline.fit_cylinder(model.geometry_tolerance_mm()) {
                    Some(fit) => {
                        let axis = crate::face_identity::canonical_axis(fit.axis);
                        let probe = samples.first().copied().filter(|p| surface.normal_at_world(*p).is_some());
                        match probe.and_then(|p| surface.normal_at_world(p).map(|n| (p, n))) {
                            Some((p, n)) => {
                                let sense = if face.same_sense { 1.0 } else { -1.0 };
                                let d = sub(p, fit.point);
                                let radial = sub(d, mul(axis, dot(d, axis)));
                                let side = if dot(mul(n, sense), radial) > 0.0 { Side::Convex } else { Side::Concave };
                                let (range, _) = axial_profile(&samples, fit.point, axis);
                                let edges: Vec<_> = face.bounds.iter().flat_map(|b| b.edges.iter().cloned()).collect();
                                let arcs = arcs_of(model, &edges, fit.point, axis, model.geometry_tolerance_mm(), fit.radius);
                                FaceGeometry::Cylinder { axis_point: fit.point, axis, radius: fit.radius, side, range, arcs }
                            }
                            None => FaceGeometry::Other(sig.kind),
                        }
                    }
                    None => FaceGeometry::Other(sig.kind),
                },
                _ => FaceGeometry::Other(sig.kind),
            };
            out.push(ClassifiedFace { key: FaceKey { body, face: *id }, geometry, lo: sig.lo, hi: sig.hi });
        }
    }
    Ok(out)
}


// ---------------------------------------------------------------------------------------------------------------------------
// Holes
// ---------------------------------------------------------------------------------------------------------------------------

/// How a hole's section ends at one end of its axial range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoleEnd {
    /// Nothing closes it: the hole continues into free space, or into a part this module does not describe.
    Open,
    /// A flat disk face across the bore (a blind hole's bottom).
    FlatBottom,
    /// A cone that closes to a point (a drill point).
    ConeBottom,
    /// A coaxial concave cylinder of larger radius continues from here (a counterbore).
    Counterbore { radius: f64 },
    /// A coaxial concave cone widening from the bore radius (a countersink); `radius` is its widest radius.
    Countersink { radius: f64 },
    /// A coaxial concave cylinder or cone of smaller radius continues from here.
    Narrows { radius: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closure {
    Through,
    Blind,
    /// Both ends closed: an internal cavity, not a hole to put a fastener in.
    Enclosed,
}

/// One cylindrical bore section of one body: the concave cylinder faces of one radius on one axis that touch end to end (STEP
/// often splits a bore into two half faces). A counterbore or countersink is a second `Hole` on the same axis, linked by
/// [`HoleEnd`].
#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    pub body: usize,
    /// The cylinder faces that make the bore, by id, ascending.
    pub faces: Vec<u64>,
    pub axis_point: V3,
    pub axis: V3,
    pub radius: f64,
    /// Axial extent from `axis_point` along `axis`.
    pub range: [f64; 2],
    /// The ends at `range[0]` and `range[1]`.
    pub ends: [HoleEnd; 2],
}

impl Hole {
    pub fn depth(&self) -> f64 {
        self.range[1] - self.range[0]
    }
    pub fn diameter(&self) -> f64 {
        2.0 * self.radius
    }
    pub fn closure(&self) -> Closure {
        let closed = |e: &HoleEnd| matches!(e, HoleEnd::FlatBottom | HoleEnd::ConeBottom);
        match (closed(&self.ends[0]), closed(&self.ends[1])) {
            (false, false) => Closure::Through,
            (true, true) => Closure::Enclosed,
            _ => Closure::Blind,
        }
    }
}

/// The angle (radians) a distance `tol` subtends over `length`.
fn angle_tol(tol: f64, length: f64) -> f64 {
    tol / length.max(tol).max(1e-9)
}

/// Whether the line `(p, a)` is the line `(p0, a0)` within `tol` (perpendicular offset) and the angle `tol` subtends over `length`.
fn coaxial(p0: V3, a0: V3, p: V3, a: V3, tol: f64, length: f64) -> bool {
    if norm(cross(a0, a)) > angle_tol(tol, length) {
        return false;
    }
    let d = sub(p, p0);
    norm(sub(d, mul(a0, dot(d, a0)))) <= tol
}

/// The two ends `(t, radius)` of a round face, in the axial frame `(p0, a0)`, ordered by `t`.
fn ends_in(p0: V3, a0: V3, p: V3, a: V3, range: [f64; 2], radii: [f64; 2]) -> [(f64, f64); 2] {
    let base = dot(sub(p, p0), a0);
    let k = dot(a, a0);
    let mut e = [(base + range[0] * k, radii[0]), (base + range[1] * k, radii[1])];
    if e[0].0 > e[1].0 {
        e.swap(0, 1);
    }
    e
}

/// A round face's `(axis_point, axis, range, radii at the range ends)` when it is a concave cylinder or cone.
fn concave_round(f: &ClassifiedFace) -> Option<(V3, V3, [f64; 2], [f64; 2])> {
    match &f.geometry {
        FaceGeometry::Cylinder { axis_point, axis, radius, side: Side::Concave, range, .. } => Some((*axis_point, *axis, *range, [*radius, *radius])),
        FaceGeometry::Cone { axis_point, axis, side: Side::Concave, range, radius_at_ends, .. } => Some((*axis_point, *axis, *range, *radius_at_ends)),
        _ => None,
    }
}

fn find_end(hole: &Hole, mine: &[&ClassifiedFace], tol: f64, end: usize) -> HoleEnd {
    let t_end = hole.range[end];
    let outward = if end == 0 { -1.0 } else { 1.0 };
    let length = hole.depth().max(hole.radius);
    for f in mine {
        if hole.faces.contains(&f.key.face) {
            continue;
        }
        if let Some((p, a, range, radii)) = concave_round(f) {
            if !coaxial(hole.axis_point, hole.axis, p, a, tol, length) {
                continue;
            }
            let [lo, hi] = ends_in(hole.axis_point, hole.axis, p, a, range, radii);
            // the end of the neighbour at the hole's end, and its far end, which must lie away from the hole
            let (near, far) = if (lo.0 - t_end).abs() <= tol { (lo, hi) } else if (hi.0 - t_end).abs() <= tol { (hi, lo) } else { continue };
            if (far.0 - t_end) * outward <= tol {
                continue;
            }
            let cone = !matches!(f.geometry, FaceGeometry::Cylinder { .. });
            if !cone {
                if near.1 > hole.radius + tol {
                    return HoleEnd::Counterbore { radius: near.1 };
                }
                if near.1 < hole.radius - tol {
                    return HoleEnd::Narrows { radius: near.1 };
                }
            } else if (near.1 - hole.radius).abs() <= tol {
                return if far.1 > hole.radius + tol {
                    HoleEnd::Countersink { radius: far.1 }
                } else if far.1 <= tol {
                    HoleEnd::ConeBottom
                } else {
                    HoleEnd::Narrows { radius: far.1 }
                };
            }
        }
    }
    // a flat disk across the bore: a plane normal to the axis at this end whose boundary stays inside the bore
    for f in mine {
        if let FaceGeometry::Plane { normal, offset, outer, holes, .. } = &f.geometry {
            let den = dot(*normal, hole.axis);
            if norm(cross(*normal, hole.axis)) > angle_tol(tol, length) || den == 0.0 {
                continue;
            }
            let t_plane = (offset - dot(*normal, hole.axis_point)) / den;
            if (t_plane - t_end).abs() > tol || !holes.is_empty() {
                continue;
            }
            let inside = outer.iter().all(|q| {
                let d = sub(*q, hole.axis_point);
                norm(sub(d, mul(hole.axis, dot(d, hole.axis)))) <= hole.radius + tol
            });
            if inside {
                return HoleEnd::FlatBottom;
            }
        }
    }
    HoleEnd::Open
}

/// Group the concave cylinder faces of every body into holes, ordered by body, then lowest face id. A group of faces is a hole
/// only if together they cover the full circle about the axis (within the angle the geometry tolerance subtends at the radius,
/// twice: one per end of an arc): a slot end or an inside fillet is a concave cylinder that is not a hole and stays visible in `faces`.
pub fn find_holes(models: &[AnalyticModel], faces: &[ClassifiedFace]) -> Vec<Hole> {
    let mut holes = Vec::new();
    for (body, model) in models.iter().enumerate() {
        let tol = model.geometry_tolerance_mm();
        let mine: Vec<&ClassifiedFace> = faces.iter().filter(|f| f.key.body == body).collect();
        let cyls: Vec<(u64, V3, V3, f64, [f64; 2], &[[f64; 2]])> = mine
            .iter()
            .filter_map(|f| match &f.geometry {
                FaceGeometry::Cylinder { axis_point, axis, radius, side: Side::Concave, range, arcs } => Some((f.key.face, *axis_point, *axis, *radius, *range, arcs.as_slice())),
                _ => None,
            })
            .collect();
        // union-find over faces that are the same bore: coaxial, one radius, touching or overlapping axial ranges
        let mut group: Vec<usize> = (0..cyls.len()).collect();
        fn root(g: &mut Vec<usize>, i: usize) -> usize {
            let mut r = i;
            while g[r] != r {
                r = g[r];
            }
            g[i] = r;
            r
        }
        for i in 0..cyls.len() {
            for j in (i + 1)..cyls.len() {
                let (a, b) = (&cyls[i], &cyls[j]);
                let length = (a.4[1] - a.4[0]).max(b.4[1] - b.4[0]).max(a.3);
                if (a.3 - b.3).abs() > tol || !coaxial(a.1, a.2, b.1, b.2, tol, length) {
                    continue;
                }
                let rb = ends_in(a.1, a.2, b.1, b.2, b.4, [b.3, b.3]);
                let gap = a.4[0].max(rb[0].0) - a.4[1].min(rb[1].0);
                if gap <= tol {
                    let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                    group[rj] = ri;
                }
            }
        }
        let mut members: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
        for i in 0..cyls.len() {
            let r = root(&mut group, i);
            members.entry(r).or_default().push(i);
        }
        let mut made: Vec<Hole> = members
            .values()
            // a bore wraps the full circle: a slot end, a counter-sunk half or an inside fillet is a concave cylinder but no hole
            .filter(|m| {
                // the angles the group's faces cover together, measured about the first face's axis (a face whose axis points the
                // other way measures them mirrored)
                let first = &cyls[m[0]];
                let mut all = Vec::new();
                for k in m.iter() {
                    let flip = dot(cyls[*k].2, first.2) < 0.0;
                    all.extend(cyls[*k].5.iter().map(|a| if flip { [TAU - a[1], TAU - a[0]] } else { *a }));
                }
                let covered: f64 = merge_arcs(all, tol / first.3).iter().map(|a| a[1] - a[0]).sum();
                covered >= TAU - 2.0 * tol / first.3
            })
            .map(|m| {
                let first = &cyls[m[0]];
                let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                for &k in m {
                    let e = ends_in(first.1, first.2, cyls[k].1, cyls[k].2, cyls[k].4, [cyls[k].3; 2]);
                    lo = lo.min(e[0].0);
                    hi = hi.max(e[1].0);
                }
                let mut ids: Vec<u64> = m.iter().map(|k| cyls[*k].0).collect();
                ids.sort_unstable();
                Hole { body, faces: ids, axis_point: first.1, axis: first.2, radius: first.3, range: [lo, hi], ends: [HoleEnd::Open; 2] }
            })
            .collect();
        made.sort_by_key(|h| h.faces[0]);
        for h in &mut made {
            let snapshot = h.clone();
            h.ends = [find_end(&snapshot, &mine, tol, 0), find_end(&snapshot, &mine, tol, 1)];
        }
        holes.extend(made);
    }
    holes
}

// ---------------------------------------------------------------------------------------------------------------------------
// Fastener sites: holes on different bodies that share an axis
// ---------------------------------------------------------------------------------------------------------------------------

/// What a bore diameter is, relative to one standard fastener size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitClass {
    /// The tap drill: the bore a thread is cut in (or a self-tapping screw runs in).
    Tap,
    /// The fastener's major diameter: a press, locating or dowel bore.
    Nominal,
    /// A close-fit clearance hole (ISO 273 "fine", Machinery's Handbook "close").
    Close,
    /// A normal-fit clearance hole (ISO 273 "medium"; the inch table has none).
    Normal,
    /// A loose-fit clearance hole (ISO 273 "coarse", Machinery's Handbook "free").
    Loose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeMatch {
    pub size: &'static str,
    pub fit: FitClass,
}

/// A bore diameter matches a table entry within this many millimetres: half of the coarsest step the tables are printed in
/// (0.1 mm for the metric clearance columns), so two different printed entries never both match and a rounded model
/// dimension (0.201 in for the #7 drill) still does.
pub const TABLE_MATCH_MM: f64 = 0.05;

/// `(name, tap, nominal, close, normal, loose)` in millimetres: ISO metric coarse threads, tap drills of ISO 724 and the
/// clearance holes of ISO 273.
const METRIC: [(&str, f64, f64, f64, f64, f64); 10] = [
    ("M3", 2.5, 3.0, 3.2, 3.4, 3.6),
    ("M4", 3.3, 4.0, 4.3, 4.5, 4.8),
    ("M5", 4.2, 5.0, 5.3, 5.5, 5.8),
    ("M6", 5.0, 6.0, 6.4, 6.6, 7.0),
    ("M8", 6.8, 8.0, 8.4, 9.0, 10.0),
    ("M10", 8.5, 10.0, 10.5, 11.0, 12.0),
    ("M12", 10.2, 12.0, 13.0, 13.5, 14.5),
    ("M16", 14.0, 16.0, 17.0, 17.5, 18.5),
    ("M20", 17.5, 20.0, 21.0, 22.0, 24.0),
    ("M24", 21.0, 24.0, 25.0, 26.0, 28.0),
];

/// `(name, tap, nominal, close, loose)` in inches: unified coarse threads, with the tap drill and the close and free clearance
/// drills of the Machinery's Handbook tables as commonly published. Transcribed, not computed. Checked on 2026-11-20 against two
/// published charts (Fractory "Imperial Clearance Hole Sizes", Optimas "Unified Threads Recommended Tapping Size and Clearance
/// Holes"): every entry agrees, the tap drill being the 75 % thread engagement one. The handbook itself was not read, and
/// editions differ on which drill is called "close" and which "free" for some sizes, so an edition-exact claim needs its table 5.
const INCH: [(&str, f64, f64, f64, f64); 8] = [
    ("#4-40", 0.0890, 0.1120, 0.1160, 0.1285),
    ("#6-32", 0.1065, 0.1380, 0.1440, 0.1495),
    ("#8-32", 0.1360, 0.1640, 0.1695, 0.1770),
    ("#10-24", 0.1495, 0.1900, 0.1960, 0.2010),
    ("1/4-20", 0.2010, 0.2500, 0.2570, 0.2660),
    ("5/16-18", 0.2570, 0.3125, 0.3230, 0.3320),
    ("3/8-16", 0.3125, 0.3750, 0.3860, 0.3970),
    ("1/2-13", 0.4219, 0.5000, 0.5156, 0.5312),
];

/// Every standard fastener size and fit a bore of `diameter_mm` matches (within [`TABLE_MATCH_MM`]); one diameter can match
/// several (0.201 in is the #10-24 free clearance and the 1/4-20 tap drill).
pub fn size_matches(diameter_mm: f64) -> Vec<SizeMatch> {
    let near = |x: f64| (diameter_mm - x).abs() <= TABLE_MATCH_MM;
    let mut out = Vec::new();
    for (size, tap, nominal, close, normal, loose) in METRIC {
        for (fit, d) in [(FitClass::Tap, tap), (FitClass::Nominal, nominal), (FitClass::Close, close), (FitClass::Normal, normal), (FitClass::Loose, loose)] {
            if near(d) {
                out.push(SizeMatch { size, fit });
            }
        }
    }
    for (size, tap, nominal, close, loose) in INCH {
        for (fit, d) in [(FitClass::Tap, tap), (FitClass::Nominal, nominal), (FitClass::Close, close), (FitClass::Loose, loose)] {
            if near(d * 25.4) {
                out.push(SizeMatch { size, fit });
            }
        }
    }
    out
}

/// One bore of a fastener site.
#[derive(Debug, Clone, PartialEq)]
pub struct Bore {
    /// Index into [`Features::holes`].
    pub hole: usize,
    pub diameter: f64,
    pub matches: Vec<SizeMatch>,
}

/// Holes of two or more bodies on one axis whose axial ranges touch or overlap: where one fastener can pass through.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerSite {
    pub axis_point: V3,
    pub axis: V3,
    /// The union of the bores' axial ranges along `axis` from `axis_point`.
    pub span: [f64; 2],
    /// Bodies the site passes through, ascending.
    pub bodies: Vec<usize>,
    pub bores: Vec<Bore>,
    /// The smallest standard size every bore matches (as tap, nominal or clearance), if there is one.
    pub fastener: Option<&'static str>,
    /// The bore diameters differ by more than the tolerance: a clearance hole in one part and a tapped, press or smaller hole in
    /// another is normal for a bolted joint, but it is reported, not hidden.
    pub diameter_mismatch: bool,
}

/// Group the holes of different bodies that share an axis and touch or overlap along it into fastener sites, ordered by their
/// lowest hole index.
pub fn find_sites(models: &[AnalyticModel], holes: &[Hole]) -> Vec<FastenerSite> {
    let tol_of = |a: usize, b: usize| models[holes[a].body].geometry_tolerance_mm().max(models[holes[b].body].geometry_tolerance_mm());
    let mut group: Vec<usize> = (0..holes.len()).collect();
    fn root(g: &mut Vec<usize>, i: usize) -> usize {
        let mut r = i;
        while g[r] != r {
            r = g[r];
        }
        g[i] = r;
        r
    }
    for i in 0..holes.len() {
        for j in (i + 1)..holes.len() {
            let (a, b) = (&holes[i], &holes[j]);
            if a.body == b.body {
                continue;
            }
            let tol = tol_of(i, j);
            let length = a.depth().max(b.depth()).max(a.radius).max(b.radius);
            if !coaxial(a.axis_point, a.axis, b.axis_point, b.axis, tol, length) {
                continue;
            }
            let e = ends_in(a.axis_point, a.axis, b.axis_point, b.axis, b.range, [b.radius; 2]);
            if a.range[0].max(e[0].0) - a.range[1].min(e[1].0) <= tol {
                let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                group[rj] = ri;
            }
        }
    }
    let mut members: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..holes.len() {
        let r = root(&mut group, i);
        members.entry(r).or_default().push(i);
    }
    let mut sites = Vec::new();
    for m in members.values().filter(|m| m.len() > 1) {
        let first = &holes[m[0]];
        let mut bodies: Vec<usize> = m.iter().map(|k| holes[*k].body).collect();
        bodies.sort_unstable();
        bodies.dedup();
        if bodies.len() < 2 {
            continue;
        }
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for k in m {
            let h = &holes[*k];
            let e = ends_in(first.axis_point, first.axis, h.axis_point, h.axis, h.range, [h.radius; 2]);
            lo = lo.min(e[0].0);
            hi = hi.max(e[1].0);
        }
        let bores: Vec<Bore> = m.iter().map(|k| Bore { hole: *k, diameter: holes[*k].diameter(), matches: size_matches(holes[*k].diameter()) }).collect();
        let tol = m.iter().map(|k| models[holes[*k].body].geometry_tolerance_mm()).fold(0.0, f64::max);
        let (dmin, dmax) = bores.iter().fold((f64::INFINITY, 0.0f64), |(lo, hi), b| (lo.min(b.diameter), hi.max(b.diameter)));
        let fastener = bores[0]
            .matches
            .iter()
            .map(|c| c.size)
            .find(|s| bores.iter().all(|b| b.matches.iter().any(|c| c.size == *s)));
        sites.push(FastenerSite { axis_point: first.axis_point, axis: first.axis, span: [lo, hi], bodies, bores, fastener, diameter_mismatch: dmax - dmin > tol });
    }
    sites
}

// ---------------------------------------------------------------------------------------------------------------------------
// Interfaces: planar faces of different bodies that touch
// ---------------------------------------------------------------------------------------------------------------------------

/// Two planar faces of different bodies that face each other across a gap no larger than the tolerance and share area.
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    /// The face of the lower-numbered body, then the other.
    pub a: FaceKey,
    pub b: FaceKey,
    /// Outward normal of `a`; `b` faces the other way.
    pub normal: V3,
    /// Signed distance from `a`'s plane to `b`'s along `normal`: positive is a gap, negative an interpenetration.
    pub gap: f64,
    /// Area common to both faces (holes of either excluded), millimetres squared.
    pub overlap_area: f64,
    /// Centroid of that common area, in the plane of `a`.
    pub overlap_centroid: V3,
}

type P2 = [f64; 2];
type Tri = [P2; 3];

fn cross2(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub2(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn signed_area2(p: &[P2]) -> f64 {
    (0..p.len()).map(|i| cross2(p[i], p[(i + 1) % p.len()])).sum::<f64>() * 0.5
}

/// Triangulate a simple polygon by ear clipping (counter-clockwise triangles). `None` when the boundary is not a simple polygon
/// (self-touching or crossing), which is reported by the caller, never papered over.
fn triangulate(poly: &[P2]) -> Option<Vec<Tri>> {
    let n = poly.len();
    if n < 3 {
        return None;
    }
    let scale = poly.iter().fold(0.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs())).max(1.0);
    let eps = 1e-12 * scale * scale;
    let mut idx: Vec<usize> = (0..n).collect();
    if signed_area2(poly) < 0.0 {
        idx.reverse();
    }
    let inside = |p: P2, a: P2, b: P2, c: P2| {
        let (d1, d2, d3) = (cross2(sub2(b, a), sub2(p, a)), cross2(sub2(c, b), sub2(p, b)), cross2(sub2(a, c), sub2(p, c)));
        d1 >= -eps && d2 >= -eps && d3 >= -eps
    };
    let mut tris = Vec::new();
    while idx.len() > 3 {
        let m = idx.len();
        let mut cut = false;
        for k in 0..m {
            let (i0, i1, i2) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (poly[i0], poly[i1], poly[i2]);
            let turn = cross2(sub2(b, a), sub2(c, b));
            if turn.abs() <= eps {
                // a collinear vertex adds no area
                idx.remove(k);
                cut = true;
                break;
            }
            if turn < 0.0 {
                continue;
            }
            let blocked = idx.iter().any(|&j| {
                let q = poly[j];
                j != i0 && j != i1 && j != i2 && q != a && q != b && q != c && inside(q, a, b, c)
            });
            if !blocked {
                tris.push([a, b, c]);
                idx.remove(k);
                cut = true;
                break;
            }
        }
        if !cut {
            return None;
        }
    }
    if idx.len() == 3 {
        let t = [poly[idx[0]], poly[idx[1]], poly[idx[2]]];
        if cross2(sub2(t[1], t[0]), sub2(t[2], t[1])).abs() > eps {
            tris.push(t);
        }
    }
    Some(tris)
}

/// Area and first moments `(A, A cx, A cy)` of the intersection of two counter-clockwise triangles (Sutherland-Hodgman: the
/// clip triangle is convex).
fn triangle_overlap(subject: &Tri, clip: &Tri) -> [f64; 3] {
    let mut poly: Vec<P2> = subject.to_vec();
    for e in 0..3 {
        let (a, b) = (clip[e], clip[(e + 1) % 3]);
        let side = |p: P2| cross2(sub2(b, a), sub2(p, a));
        let input = std::mem::take(&mut poly);
        for i in 0..input.len() {
            let (p, q) = (input[i], input[(i + 1) % input.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                poly.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let t = sp / (sp - sq);
                poly.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            }
        }
        if poly.is_empty() {
            return [0.0; 3];
        }
    }
    let (mut area, mut mx, mut my) = (0.0, 0.0, 0.0);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let w = cross2(p, q);
        area += w * 0.5;
        mx += (p[0] + q[0]) * w / 6.0;
        my += (p[1] + q[1]) * w / 6.0;
    }
    [area, mx, my]
}

fn overlap_of(x: &[Tri], y: &[Tri]) -> [f64; 3] {
    let mut s = [0.0; 3];
    for a in x {
        for b in y {
            let o = triangle_overlap(a, b);
            for k in 0..3 {
                s[k] += o[k];
            }
        }
    }
    s
}

/// A planar face of a body as triangle sets in the 2D basis `(u, v)`: its outer boundary and each hole. `None` when a boundary
/// is not a simple polygon.
fn face_triangles(outer: &[V3], holes: &[Vec<V3>], u: V3, v: V3) -> Option<(Vec<Tri>, Vec<Vec<Tri>>)> {
    let flat = |l: &[V3]| l.iter().map(|p| [dot(*p, u), dot(*p, v)]).collect::<Vec<P2>>();
    Some((triangulate(&flat(outer))?, holes.iter().map(|h| triangulate(&flat(h))).collect::<Option<Vec<_>>>()?))
}

/// Planar faces of different bodies with opposite normals, no more than the tolerance apart along them, and a common area of
/// more than a strip one tolerance wide along the longer face diagonal. Returns the interfaces and the face pairs that passed the geometric screen but whose common area could not be
/// computed (a boundary that is not a simple polygon): those are reported, not dropped.
///
/// Only contact within the tolerance is an interface: a real gap (a washer, a shim, a clearance) is not detected here.
pub fn find_interfaces(models: &[AnalyticModel], faces: &[ClassifiedFace]) -> (Vec<Interface>, Vec<(FaceKey, FaceKey)>) {
    let planes: Vec<&ClassifiedFace> = faces.iter().filter(|f| matches!(f.geometry, FaceGeometry::Plane { .. })).collect();
    let (mut out, mut unresolved) = (Vec::new(), Vec::new());
    for (i, fa) in planes.iter().enumerate() {
        let FaceGeometry::Plane { normal: na, offset: oa, outer: outer_a, holes: holes_a, .. } = &fa.geometry else { continue };
        for fb in &planes[i + 1..] {
            if fa.key.body == fb.key.body {
                continue;
            }
            let FaceGeometry::Plane { normal: nb, offset: ob, outer: outer_b, holes: holes_b, .. } = &fb.geometry else { continue };
            let tol = models[fa.key.body].geometry_tolerance_mm().max(models[fb.key.body].geometry_tolerance_mm());
            let diag = |f: &ClassifiedFace| norm(sub(f.hi, f.lo));
            let length = diag(fa).max(diag(fb));
            if dot(*na, *nb) >= 0.0 || norm(cross(*na, *nb)) > angle_tol(tol, length) {
                continue;
            }
            // `b`'s plane is `nb . p = ob`; along `na` its distance from `a`'s plane is `-ob - oa` (nb is about -na)
            let gap = -ob - oa;
            if gap.abs() > tol {
                continue;
            }
            if (0..3).any(|k| fa.lo[k] > fb.hi[k] + tol || fb.lo[k] > fa.hi[k] + tol) {
                continue;
            }
            let (u, v) = plane_basis(*na);
            let (Some((ta, ha)), Some((tb, hb))) = (face_triangles(outer_a, holes_a, u, v), face_triangles(outer_b, holes_b, u, v)) else {
                unresolved.push((fa.key, fb.key));
                continue;
            };
            // inclusion-exclusion over the faces' holes (disjoint inside each face): |A n B| for A = OA - HA, B = OB - HB
            let mut m = overlap_of(&ta, &tb);
            let mut take = |sign: f64, x: [f64; 3]| {
                for k in 0..3 {
                    m[k] += sign * x[k];
                }
            };
            for h in &ha {
                take(-1.0, overlap_of(h, &tb));
            }
            for h in &hb {
                take(-1.0, overlap_of(&ta, h));
            }
            for x in &ha {
                for y in &hb {
                    take(1.0, overlap_of(x, y));
                }
            }
            // A strip no wider than the tolerance along a shared edge is not a contact: its area is at most `tol` times the
            // length of the edge, which is at most `length`. (`tol * tol` is below the rounding noise of a placed body: an edge
            // contact of two rotated bodies read 6e-14 mm2 against 1e-14.)
            if m[0] <= tol * length {
                continue;
            }
            let centroid = add(add(mul(u, m[1] / m[0]), mul(v, m[2] / m[0])), mul(*na, *oa));
            out.push(Interface { a: fa.key, b: fb.key, normal: *na, gap, overlap_area: m[0], overlap_centroid: centroid });
        }
    }
    (out, unresolved)
}

// ---------------------------------------------------------------------------------------------------------------------------
// Everything at once
// ---------------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    pub faces: Vec<ClassifiedFace>,
    pub holes: Vec<Hole>,
    pub sites: Vec<FastenerSite>,
    pub interfaces: Vec<Interface>,
    /// Face pairs that looked like an interface but whose common area could not be computed.
    pub unresolved: Vec<(FaceKey, FaceKey)>,
}

/// Detect the holes, fastener sites and contact interfaces of placed bodies (the models of `AnalyticModel::read_all`).
pub fn detect(models: &[AnalyticModel]) -> Result<Features, Error> {
    let faces = classify_faces(models)?;
    let holes = find_holes(models, &faces);
    let sites = find_sites(models, &holes);
    let (interfaces, unresolved) = find_interfaces(models, &faces);
    Ok(Features { faces, holes, sites, interfaces, unresolved })
}

#[cfg(test)]
mod table_tests {
    use super::{INCH, METRIC};

    /// Within one size the bores grow tap < nominal < close <= loose (inch) and tap < nominal < close <= normal <= loose (metric),
    /// and the sizes grow: a transcription slip that swaps two columns or two rows breaks this.
    #[test]
    fn the_size_tables_are_monotone_in_every_row_and_down_the_sizes() {
        for (name, tap, nominal, close, loose) in INCH {
            assert!(tap < nominal && nominal < close && close <= loose, "{name}");
        }
        for (name, tap, nominal, close, normal, loose) in METRIC {
            assert!(tap < nominal && nominal < close && close <= normal && normal <= loose, "{name}");
        }
        assert!(INCH.windows(2).all(|w| w[0].2 < w[1].2) && METRIC.windows(2).all(|w| w[0].2 < w[1].2));
    }
}
