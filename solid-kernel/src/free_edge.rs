//! The in-plane distance from a hole's axis to the nearest **free edge** of the plate face it is drilled in: the geometry the
//! edge-distance estimate (`edge-check`) needs from a B-rep part (`docs/edge-distance-crosscheck.md`, "Assembly Joints").
//!
//! Geometry only, millimetres, a pure function of the models [`AnalyticModel::read_all`] returns and the output of
//! [`crate::features`]: [`free_edge_distance`] takes a [`Hole`] found there. No tolerance is tuned: lengths are compared with the
//! model's `geometry_tolerance_mm`, directions with the angle that distance subtends over the face's size.
//!
//! **What is read.** The hole's end faces are the planar faces across the rim edges of its bore, square to its axis, that hold
//! the rim as a closed circular inner loop (the rim as the *outer* loop is a blind bore's flat bottom, skipped). On each such face
//! `F` (outward normal `n`) every edge of the outer loop is classified by the one face `N` across it:
//!
//! * **free**: `N` is a plane square to `F` or a cylinder with its axis along `n` (a straight side, a round outline, a notch) *and*
//!   it runs through to the opposite face `G` of the same plate: a plane facing the other way that holds the hole's axis inside
//!   its outline. The material ends at the edge; `|F - G|` is the thickness there.
//! * **continues**: `N` is square to `F` (or a cylinder along `n`) but runs on beyond the plate: a bend into another flange, the
//!   wall of a step or a counterbore. The material goes on, so the edge is not free whatever its distance.
//! * **unread**: anything else (a rounded or chamfered edge, a bend radius, a cone, a spline, a drafted side, a coplanar split
//!   neighbour, an edge not shared by exactly two faces). It is not guessed: if one lies nearer than the nearest free edge the
//!   reading is refused ([`FreeEdgeError::UnreadBoundary`]), because the true free edge could be that one.
//!
//! The distance is measured in the plane of `F` from the point where the axis meets it, to a straight edge or exactly to a circular
//! one (centre and radius fitted to the edge's samples), never through the thickness. Other holes of the plate are inner loops and
//! are not edges. When the hole opens onto two faces the nearer reading is kept.

use std::collections::{BTreeMap, BTreeSet};

use crate::analytic_step::AnalyticModel;
use crate::features::{add, angle_tol, bound_polyline, cross, dot, loop_area, merge_arcs, mul, norm, plane_basis, sub, ClassifiedFace, FaceGeometry, FaceKey, Hole, Side};
use crate::step_topology::EdgeUse;
use crate::V3;

const TAU: f64 = std::f64::consts::TAU;

/// The kind of boundary edge the hole is nearest to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeKind {
    /// A straight edge.
    Line,
    /// A circular edge (a round outline, a notch): its radius in millimetres.
    Arc { radius: f64 },
}

/// The nearest free edge of a hole's plate face.
#[derive(Debug, Clone, PartialEq)]
pub struct FreeEdge {
    /// The planar face the distance was read on.
    pub face: FaceKey,
    /// The STEP id of the edge.
    pub edge: u64,
    /// The face across that edge (the side of the plate).
    pub rim: FaceKey,
    pub kind: EdgeKind,
    /// Millimetres, in the plane of `face`, from where the hole's axis meets it to the nearest point of the edge. Measured from
    /// the axis, not the bore wall: subtract the radius for the ligament.
    pub distance: f64,
    /// The nearest point of the edge.
    pub point: V3,
    /// The plate thickness at the hole: the distance from `face` to the opposite face the rim runs to.
    pub thickness: f64,
}

/// Why no free edge distance could be read; `Display` states the reason for a person.
#[derive(Debug, Clone, PartialEq)]
pub enum FreeEdgeError {
    /// The hole's body is not among the models.
    UnknownBody,
    /// The bore ends on no flat face square to its axis (a countersink cone, a curved wall, a tilted face); `found` names what it
    /// meets.
    NotPlanar { found: String },
    /// The bore does not close a full circle on its face: a slit or notch, or a bore whose faces cover less than a turn.
    NotFullCircle,
    /// Every boundary edge of the face joins material that goes on: there is no free edge to measure to.
    NoFreeEdge,
    /// A boundary nearer than the nearest free edge is not a kind this reading understands.
    UnreadBoundary { distance: f64, reason: String },
}

impl std::fmt::Display for FreeEdgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownBody => write!(f, "the hole's body is not among the models"),
            Self::NotPlanar { found } => write!(f, "the bore does not end on a flat face square to its axis (it meets {found}): no in-plane edge distance"),
            Self::NotFullCircle => write!(f, "the bore is not a full circle on its face (a slit or notch open to the edge)"),
            Self::NoFreeEdge => write!(f, "the face has no free edge: every boundary edge joins material that continues (a bend, a step or a counterbore wall)"),
            Self::UnreadBoundary { distance, reason } => write!(f, "the boundary {distance:.3} mm from the axis is {reason}: not read, and it is nearer than any free edge"),
        }
    }
}

impl std::error::Error for FreeEdgeError {}

/// What lies across one boundary edge of the face.
enum Across {
    Free { rim: u64, thickness: f64 },
    Continues,
    Unread(String),
}

/// A boundary edge as geometry in the face plane.
enum Shape {
    Line { a: V3, b: V3 },
    Arc { centre: V3, radius: f64, e1: V3, e2: V3, start: f64, span: f64 },
}

struct Ctx<'a> {
    model: &'a AnalyticModel,
    faces: BTreeMap<u64, &'a ClassifiedFace>,
    /// Faces using each edge (an edge used twice by one face is listed twice).
    adjacent: BTreeMap<u64, Vec<u64>>,
    tol: f64,
}

impl<'a> Ctx<'a> {
    fn new(model: &'a AnalyticModel, faces: &'a [ClassifiedFace], body: usize) -> Self {
        let mut adjacent: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for (id, face) in &model.body().faces {
            for b in &face.bounds {
                for u in &b.edges {
                    adjacent.entry(u.edge).or_default().push(*id);
                }
            }
        }
        Ctx { model, faces: faces.iter().filter(|f| f.key.body == body).map(|f| (f.key.face, f)).collect(), adjacent, tol: model.geometry_tolerance_mm() }
    }

    /// The faces across `edge` from `face`, each once.
    fn across(&self, face: u64, edge: u64) -> Vec<u64> {
        let set: BTreeSet<u64> = self.adjacent.get(&edge).into_iter().flatten().copied().filter(|g| *g != face).collect();
        set.into_iter().collect()
    }

    /// The faces across any edge of `face` (other than `except`).
    fn neighbours_of(&self, face: u64, except: &[u64]) -> BTreeSet<u64> {
        let mut out = BTreeSet::new();
        if let Some(f) = self.model.body().faces.get(&face) {
            for u in f.bounds.iter().flat_map(|b| &b.edges) {
                out.extend(self.across(face, u.edge).into_iter().filter(|g| !except.contains(g)));
            }
        }
        out
    }

    /// Whether the point `p` lies inside the outer outline of plane face `g`, seen along its normal.
    fn inside_outline(&self, g: &ClassifiedFace, p: V3) -> bool {
        let FaceGeometry::Plane { normal, outer, .. } = &g.geometry else { return false };
        let (u, v) = plane_basis(*normal);
        let (x, y) = (dot(p, u), dot(p, v));
        let mut inside = false;
        for i in 0..outer.len() {
            let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
            let (ax, ay, bx, by) = (dot(a, u), dot(a, v), dot(b, u), dot(b, v));
            if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                inside = !inside;
            }
        }
        inside
    }

    /// What lies across `edge` of the plane face `f` (normal `n`, offset `offset`), for a hole whose axis meets `f` at `axis_at`.
    fn classify(&self, f: &ClassifiedFace, n: V3, offset: f64, axis_at: V3, edge: u64) -> Across {
        let ang = angle_tol(self.tol, norm(sub(f.hi, f.lo)));
        let across = self.across(f.key.face, edge);
        let [other] = across.as_slice() else { return Across::Unread("an edge not shared by exactly two faces".into()) };
        let Some(nf) = self.faces.get(other) else { return Across::Unread("an edge to an unclassified face".into()) };
        match &nf.geometry {
            FaceGeometry::Plane { normal: m, .. } => {
                if norm(cross(*m, n)) <= ang {
                    return Across::Unread("a neighbouring face in the same plane (merged coplanar faces are not read)".into());
                }
                if dot(*m, n).abs() > ang {
                    return Across::Unread("an inclined neighbouring face (a chamfer or a drafted side)".into());
                }
            }
            FaceGeometry::Cylinder { axis, .. } => {
                if norm(cross(*axis, n)) > ang {
                    return Across::Unread("a rounded or bent neighbouring face (a cylinder with its axis in the plane: a fillet or a bend radius)".into());
                }
            }
            other => return Across::Unread(format!("a neighbouring {} face", describe(other))),
        }
        // The opposite face of the same plate: across another edge of the neighbour, facing away, holding the axis.
        for g in self.neighbours_of(*other, &[f.key.face, *other]) {
            let Some(gf) = self.faces.get(&g) else { continue };
            let FaceGeometry::Plane { normal: m, offset: og, .. } = &gf.geometry else { continue };
            if norm(cross(*m, n)) <= ang && dot(*m, n) < 0.0 && self.inside_outline(gf, axis_at) {
                let thickness = offset + og;
                if thickness > self.tol {
                    return Across::Free { rim: *other, thickness };
                }
            }
        }
        Across::Continues
    }
}

fn describe(g: &FaceGeometry) -> String {
    match g {
        FaceGeometry::Plane { .. } => "plane".into(),
        FaceGeometry::Cylinder { side: Side::Concave, .. } => "concave cylinder".into(),
        FaceGeometry::Cylinder { .. } => "convex cylinder".into(),
        FaceGeometry::Cone { .. } => "cone".into(),
        FaceGeometry::Other(kind) => format!("{kind:?}").to_lowercase(),
    }
}

/// Distance from `p` to the segment `a b`, and the nearest point.
fn segment_nearest(p: V3, a: V3, b: V3) -> (f64, V3) {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    let t = if len2 > 0.0 { (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let q = add(a, mul(ab, t));
    (norm(sub(p, q)), q)
}

fn polyline_nearest(p: V3, points: &[V3]) -> (f64, V3) {
    let mut best = (f64::INFINITY, p);
    for w in points.windows(2) {
        let r = segment_nearest(p, w[0], w[1]);
        if r.0 < best.0 {
            best = r;
        }
    }
    best
}

/// The straight line or circular arc through an edge's samples (within `tol`), `None` for any other curve. `n` is the plane normal
/// the edge lies in.
fn read_shape(points: &[V3], n: V3, tol: f64) -> Option<Shape> {
    let (first, last) = (*points.first()?, *points.last()?);
    if points.len() >= 2 && norm(sub(last, first)) > tol {
        let d = mul(sub(last, first), 1.0 / norm(sub(last, first)));
        if points.iter().all(|p| {
            let r = sub(*p, first);
            norm(sub(r, mul(d, dot(r, d)))) <= tol
        }) {
            return Some(Shape::Line { a: first, b: last });
        }
    }
    if points.len() < 4 {
        return None;
    }
    let (a, b, c) = (points[0], points[points.len() / 3], points[2 * points.len() / 3]);
    let (u, v) = (sub(b, a), sub(c, a));
    let w = cross(u, v);
    let w2 = dot(w, w);
    if w2 <= tol * tol * tol * tol {
        return None;
    }
    let centre = add(a, mul(add(mul(cross(w, u), dot(v, v)), mul(cross(v, w), dot(u, u))), 1.0 / (2.0 * w2)));
    let radius = norm(sub(a, centre));
    if radius <= tol || points.iter().any(|p| (norm(sub(*p, centre)) - radius).abs() > tol) {
        return None;
    }
    let e1 = mul(sub(a, centre), 1.0 / radius);
    let e2 = cross(n, e1);
    let angle = |p: V3| dot(sub(p, centre), e2).atan2(dot(sub(p, centre), e1));
    let (mut previous, mut span) = (angle(points[0]), 0.0);
    for p in &points[1..] {
        let mut d = angle(*p) - previous;
        if d > std::f64::consts::PI {
            d -= TAU;
        } else if d <= -std::f64::consts::PI {
            d += TAU;
        }
        span += d;
        previous = angle(*p);
    }
    Some(Shape::Arc { centre, radius, e1, e2, start: angle(points[0]), span })
}

/// Distance from `p` (in the plane) to the shape and the nearest point.
fn shape_nearest(p: V3, shape: &Shape) -> (f64, V3) {
    match shape {
        Shape::Line { a, b } => segment_nearest(p, *a, *b),
        Shape::Arc { centre, radius, e1, e2, start, span } => {
            let d = sub(p, *centre);
            let phi = dot(d, *e2).atan2(dot(d, *e1));
            let along = ((phi - start) * span.signum()).rem_euclid(TAU);
            let at = |a: f64| add(*centre, mul(add(mul(*e1, a.cos()), mul(*e2, a.sin())), *radius));
            if span.abs() >= TAU - 1e-9 || along <= span.abs() {
                let q = at(phi);
                (norm(sub(p, q)), q)
            } else {
                let (s, e) = (at(*start), at(start + span));
                let (ds, de) = (norm(sub(p, s)), norm(sub(p, e)));
                if ds <= de {
                    (ds, s)
                } else {
                    (de, e)
                }
            }
        }
    }
}

fn kind_of(shape: &Shape) -> EdgeKind {
    match shape {
        Shape::Line { .. } => EdgeKind::Line,
        Shape::Arc { radius, .. } => EdgeKind::Arc { radius: *radius },
    }
}

/// Read the nearest free edge on plane face `f`, for `hole`. `None`: `f` is the flat bottom of a blind bore (the rim is its outer
/// loop), which is no plate face.
fn read_face(cx: &Ctx, hole: &Hole, f: &ClassifiedFace) -> Option<Result<FreeEdge, FreeEdgeError>> {
    let FaceGeometry::Plane { normal: n, offset, .. } = &f.geometry else { return None };
    let (n, offset) = (*n, *offset);
    let face = cx.model.body().faces.get(&f.key.face)?;
    let (u, v) = plane_basis(n);
    let loops: Vec<Vec<V3>> = face.bounds.iter().map(|b| bound_polyline(cx.model, &b.edges)).collect();
    let outer = (0..loops.len()).max_by(|a, b| loop_area(&loops[*a], u, v).total_cmp(&loop_area(&loops[*b], u, v)))?;
    let on_rim = |l: &Vec<V3>| {
        !l.is_empty()
            && l.iter().all(|p| {
                let d = sub(*p, hole.axis_point);
                (norm(sub(d, mul(hole.axis, dot(d, hole.axis)))) - hole.radius).abs() <= cx.tol
            })
    };
    match loops.iter().position(on_rim) {
        Some(i) if i == outer => return None,
        Some(_) => {}
        None => return Some(Err(FreeEdgeError::NotFullCircle)),
    }
    // where the axis meets the face plane
    let s = (offset - dot(n, hole.axis_point)) / dot(n, hole.axis);
    let axis_at = add(hole.axis_point, mul(hole.axis, s));
    let mut free: Option<FreeEdge> = None;
    let mut unread: Option<(f64, String)> = None;
    let edges: &[EdgeUse] = &face.bounds[outer].edges;
    for u in edges {
        let points = cx.model.edge_points(u.edge).unwrap_or(&[]);
        match cx.classify(f, n, offset, axis_at, u.edge) {
            Across::Continues => {}
            Across::Free { rim, thickness } => match read_shape(points, n, cx.tol) {
                Some(shape) => {
                    let (distance, point) = shape_nearest(axis_at, &shape);
                    if free.as_ref().is_none_or(|b| distance < b.distance) {
                        free = Some(FreeEdge { face: f.key, edge: u.edge, rim: FaceKey { body: f.key.body, face: rim }, kind: kind_of(&shape), distance, point, thickness });
                    }
                }
                None => {
                    let d = polyline_nearest(axis_at, points).0;
                    if unread.as_ref().is_none_or(|(b, _)| d < *b) {
                        unread = Some((d, "an edge that is neither a line nor a circular arc".into()));
                    }
                }
            },
            Across::Unread(reason) => {
                let d = polyline_nearest(axis_at, points).0;
                if unread.as_ref().is_none_or(|(b, _)| d < *b) {
                    unread = Some((d, reason));
                }
            }
        }
    }
    Some(match (free, unread) {
        (Some(f), Some((d, reason))) if d < f.distance => Err(FreeEdgeError::UnreadBoundary { distance: d, reason }),
        (Some(f), _) => Ok(f),
        (None, Some((distance, reason))) => Err(FreeEdgeError::UnreadBoundary { distance, reason }),
        (None, None) => Err(FreeEdgeError::NoFreeEdge),
    })
}

/// The in-plane distance from `hole`'s axis to the nearest free edge of the plate face it is drilled in (see the module notes for
/// what a free edge is). `models` and `faces` are the models and [`crate::features::classify_faces`] output `hole` was found in.
///
/// A bore that opens onto two plate faces is read on both and the nearer kept; a refusal on either face that is not merely "no free
/// edge here" (a bore bottom or a counterbore floor has none) is returned, since the other face's answer could then be wrong.
pub fn free_edge_distance(models: &[AnalyticModel], faces: &[ClassifiedFace], hole: &Hole) -> Result<FreeEdge, FreeEdgeError> {
    let model = models.get(hole.body).ok_or(FreeEdgeError::UnknownBody)?;
    read_hole(&Ctx::new(model, faces, hole.body), hole)
}

/// [`free_edge_distance`] for every hole, in the order of `holes`; the topology of each body is indexed once, not once per hole.
pub fn free_edges(models: &[AnalyticModel], faces: &[ClassifiedFace], holes: &[Hole]) -> Vec<Result<FreeEdge, FreeEdgeError>> {
    let contexts: Vec<Ctx> = models.iter().enumerate().map(|(body, m)| Ctx::new(m, faces, body)).collect();
    holes.iter().map(|h| contexts.get(h.body).ok_or(FreeEdgeError::UnknownBody).and_then(|cx| read_hole(cx, h))).collect()
}

fn read_hole(cx: &Ctx, hole: &Hole) -> Result<FreeEdge, FreeEdgeError> {
    let model = cx.model;
    // the bore must wrap the full circle (the faces' angular intervals, merged)
    let arcs: Vec<[f64; 2]> = hole
        .faces
        .iter()
        .filter_map(|id| cx.faces.get(id))
        .filter_map(|f| match &f.geometry {
            FaceGeometry::Cylinder { arcs, .. } => Some(arcs.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let covered: f64 = merge_arcs(arcs, cx.tol / hole.radius.max(cx.tol)).iter().map(|a| a[1] - a[0]).sum();
    if covered < TAU - 2.0 * cx.tol / hole.radius.max(cx.tol) {
        return Err(FreeEdgeError::NotFullCircle);
    }
    // the faces across the bore's rim edges
    let mut ends = BTreeSet::new();
    for id in &hole.faces {
        let Some(bore) = model.body().faces.get(id) else { continue };
        for u in bore.bounds.iter().flat_map(|b| &b.edges) {
            ends.extend(cx.across(*id, u.edge).into_iter().filter(|g| !hole.faces.contains(g)));
        }
    }
    let mut candidates: Vec<&ClassifiedFace> = Vec::new();
    let mut other: Option<String> = None;
    for id in ends {
        let Some(f) = cx.faces.get(&id) else { continue };
        match &f.geometry {
            FaceGeometry::Plane { normal, .. } if norm(cross(*normal, hole.axis)) <= angle_tol(cx.tol, norm(sub(f.hi, f.lo))) => candidates.push(f),
            FaceGeometry::Plane { .. } => other = other.or(Some("a plane not square to the axis".into())),
            g => other = other.or(Some(format!("a {} face", describe(g)))),
        }
    }
    let mut best: Option<FreeEdge> = None;
    let mut none = false;
    let mut seen = false;
    for f in candidates {
        match read_face(cx, hole, f) {
            None => {}
            Some(Ok(r)) => {
                seen = true;
                if best.as_ref().is_none_or(|b| r.distance < b.distance) {
                    best = Some(r);
                }
            }
            Some(Err(FreeEdgeError::NoFreeEdge)) => {
                seen = true;
                none = true;
            }
            Some(Err(e)) => return Err(e),
        }
    }
    match best {
        Some(b) => Ok(b),
        None if seen && none => Err(FreeEdgeError::NoFreeEdge),
        None => Err(FreeEdgeError::NotPlanar { found: other.unwrap_or_else(|| "no face but its own bottom".into()) }),
    }
}
