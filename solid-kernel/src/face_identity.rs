//! Persistent face identity across files (epic #51, "persistent associations").
//!
//! A STEP entity id names a face only inside one physical file: another export of the same part, or an edited version of it,
//! renumbers everything. This module gives every face of an [`AnalyticModel`] an id-independent **signature** (surface kind and
//! its defining invariants, oriented like the face, the extent of its boundary, how many loops and edges it has, and the kinds of
//! the faces around it) and **associates** the faces of two models by those signatures:
//!
//! 1. *exact*: same kind, same invariants within `position_tol` / `angle_tol`, same topology counts, the same neighbour kinds
//!    (a re-export, or an edit that did not touch the face; two faces that tie are `ambiguous`, never guessed);
//! 2. *modified*: the same underlying surface (same kind, invariants and size, within the tolerances) but a different topology
//!    (a hole added to a plate, a neighbour removed): the face whose extent overlaps the old one most, when that is clear;
//! 3. *moved*: same kind and size (radius, angle, sheet orientation), same topology, the same neighbour kinds, but displaced by at
//!    most `search_radius` (an edit that moved or resized a feature around it); reported with the displacement;
//! 4. whatever remains on either side is `lost` or `added`.
//!
//! Signatures encode to a short text line ([`FaceSignature::encode`]) so a selection survives in a project file.

use crate::analytic_step::{AnalyticModel, Surface};
use crate::step::{Error, ErrorKind};
use crate::V3;
use std::collections::BTreeMap;

fn fail(detail: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind: ErrorKind::Geometry,
        detail: detail.into(),
    }
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
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Plane,
    Cylinder,
    Sphere,
    Cone,
    Torus,
    /// A B-spline surface: identified by its extent, loops and neighbours only.
    Spline,
}

/// The id-independent description of one face.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceSignature {
    pub kind: Kind,
    /// The face's outward normal for a plane; the (sign-normalised) axis for a cylinder, cone and torus; unused for a sphere.
    pub direction: V3,
    /// A plane: the point of the plane nearest the origin. A cylinder, cone and torus: the point of the axis nearest the origin.
    /// A sphere: its centre.
    pub anchor: V3,
    /// A plane: 0. A cylinder / sphere: the radius. A cone: the half angle (radians). A torus: the major radius.
    pub size: f64,
    /// A torus: the minor radius; otherwise 0.
    pub size2: f64,
    /// Whether the face's outward normal points away from the axis or centre (a boss, a rounded edge) rather than toward it (a
    /// hole, a pocket); irrelevant for planes.
    pub convex: bool,
    /// Bounding box of the boundary samples, millimetres.
    pub lo: V3,
    pub hi: V3,
    pub loops: usize,
    pub edges: usize,
    /// Kinds of the faces that share an edge with this one, sorted.
    pub neighbours: Vec<Kind>,
}

impl FaceSignature {
    fn centre(&self) -> V3 {
        mul(add(self.lo, self.hi), 0.5)
    }
    /// A one-line text form (`v1`), exact to the printed 17 digits.
    pub fn encode(&self) -> String {
        let v = |p: V3| format!("{:e},{:e},{:e}", p[0], p[1], p[2]);
        format!(
            "face-signature v1;{:?};{};{};{:e};{:e};{};{};{};{};{};{}",
            self.kind,
            v(self.direction),
            v(self.anchor),
            self.size,
            self.size2,
            u8::from(self.convex),
            v(self.lo),
            v(self.hi),
            self.loops,
            self.edges,
            self.neighbours
                .iter()
                .map(|k| format!("{k:?}"))
                .collect::<Vec<_>>()
                .join("+"),
        )
    }
    pub fn decode(text: &str) -> Result<Self, Error> {
        let parts: Vec<&str> = text.trim().split(';').collect();
        if parts.len() != 12 || parts[0] != "face-signature v1" {
            return Err(fail("not a face signature (version 1)"));
        }
        let kind = |s: &str| -> Result<Kind, Error> {
            Ok(match s {
                "Plane" => Kind::Plane,
                "Cylinder" => Kind::Cylinder,
                "Sphere" => Kind::Sphere,
                "Cone" => Kind::Cone,
                "Torus" => Kind::Torus,
                "Spline" => Kind::Spline,
                other => return Err(fail(format!("unknown surface kind {other:?}"))),
            })
        };
        let vec3 = |s: &str| -> Result<V3, Error> {
            let x: Result<Vec<f64>, _> = s.split(',').map(str::parse::<f64>).collect();
            match x.map_err(|e| fail(format!("bad number: {e}")))?.as_slice() {
                [a, b, c] if [a, b, c].iter().all(|v| v.is_finite()) => Ok([*a, *b, *c]),
                _ => Err(fail("three finite numbers expected")),
            }
        };
        let num = |s: &str| -> Result<f64, Error> {
            s.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| fail("bad number"))
        };
        let count =
            |s: &str| -> Result<usize, Error> { s.parse::<usize>().map_err(|_| fail("bad count")) };
        let neighbours = if parts[11].is_empty() {
            Vec::new()
        } else {
            parts[11]
                .split('+')
                .map(kind)
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(Self {
            kind: kind(parts[1])?,
            direction: vec3(parts[2])?,
            anchor: vec3(parts[3])?,
            size: num(parts[4])?,
            size2: num(parts[5])?,
            convex: parts[6] == "1",
            lo: vec3(parts[7])?,
            hi: vec3(parts[8])?,
            loops: count(parts[9])?,
            edges: count(parts[10])?,
            neighbours,
        })
    }
}

/// Make an axis direction unique up to sign: the first non-negligible component positive.
fn canonical_axis(a: V3) -> V3 {
    let flip = a.iter().find(|x| x.abs() > 1e-9).is_some_and(|x| *x < 0.);
    if flip {
        mul(a, -1.)
    } else {
        a
    }
}
/// The point of the line through `p` along unit `d` that is nearest the origin.
fn nearest_to_origin(p: V3, d: V3) -> V3 {
    sub(p, mul(d, dot(p, d)))
}

impl AnalyticModel {
    /// The signature of every face, by source face id.
    pub fn face_signatures(&self) -> Result<BTreeMap<u64, FaceSignature>, Error> {
        let body = self.body();
        let mut owners: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for f in body.faces.values() {
            for b in &f.bounds {
                for u in &b.edges {
                    let list = owners.entry(u.edge).or_default();
                    if !list.contains(&f.id) {
                        list.push(f.id);
                    }
                }
            }
        }
        let kind_of = |id: u64| -> Result<Kind, Error> {
            Ok(
                match self
                    .surface(id)
                    .ok_or_else(|| fail("face without a surface"))?
                {
                    Surface::Plane(_) => Kind::Plane,
                    Surface::Cylinder { .. } => Kind::Cylinder,
                    Surface::Sphere { .. } => Kind::Sphere,
                    Surface::Cone { .. } => Kind::Cone,
                    Surface::Torus { .. } => Kind::Torus,
                Surface::Param(_) => Kind::Spline,
                },
            )
        };
        let mut out = BTreeMap::new();
        for f in body.faces.values() {
            let surface = self.surface(f.id).unwrap();
            let sense = if f.same_sense { 1. } else { -1. };
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            let mut edges = 0;
            let mut neighbours = Vec::new();
            for b in &f.bounds {
                for u in &b.edges {
                    edges += 1;
                    for p in self.edge_points(u.edge).unwrap() {
                        for k in 0..3 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                    for o in &owners[&u.edge] {
                        if *o != f.id {
                            neighbours.push(kind_of(*o)?);
                        }
                    }
                }
            }
            neighbours.sort();
            let probe = f
                .bounds
                .iter()
                .flat_map(|b| &b.edges)
                .find_map(|u| self.edge_points(u.edge).and_then(|p| p.first().copied()))
                .ok_or_else(|| fail("a face without edges"))?;
            let (kind, direction, anchor, size, size2, convex) = match surface {
                Surface::Param(_) => (Kind::Spline, [0.; 3], [0.; 3], 0., 0., true),
                Surface::Plane(fr) => {
                    let n = mul(fr.z, sense);
                    (Kind::Plane, n, mul(n, dot(n, fr.origin)), 0., 0., true)
                }
                Surface::Cylinder { frame, radius } => {
                    let a = canonical_axis(frame.z);
                    let n = surface
                        .normal_at_world(probe)
                        .ok_or_else(|| fail("no normal"))?;
                    // outward normal of the face: away from the axis for a boss; the declared sense flips it
                    let axis_point = nearest_to_origin(frame.origin, a);
                    let radial = sub(
                        probe,
                        add(
                            frame.origin,
                            mul(frame.z, dot(sub(probe, frame.origin), frame.z)),
                        ),
                    );
                    (
                        Kind::Cylinder,
                        a,
                        axis_point,
                        *radius,
                        0.,
                        dot(mul(n, sense), radial) > 0.,
                    )
                }
                Surface::Sphere { frame, radius } => {
                    let n = surface
                        .normal_at_world(probe)
                        .ok_or_else(|| fail("no normal"))?;
                    (
                        Kind::Sphere,
                        [0.; 3],
                        frame.origin,
                        *radius,
                        0.,
                        dot(mul(n, sense), sub(probe, frame.origin)) > 0.,
                    )
                }
                Surface::Cone {
                    frame,
                    semi_angle,
                    radius,
                    ..
                } => {
                    let a = canonical_axis(frame.z);
                    let n = surface
                        .normal_at_world(probe)
                        .ok_or_else(|| fail("no normal"))?;
                    let apex = add(frame.origin, mul(frame.z, -radius / semi_angle.tan()));
                    let radial = sub(
                        probe,
                        add(
                            frame.origin,
                            mul(frame.z, dot(sub(probe, frame.origin), frame.z)),
                        ),
                    );
                    (
                        Kind::Cone,
                        a,
                        nearest_to_origin(apex, a),
                        *semi_angle,
                        0.,
                        dot(mul(n, sense), radial) > 0.,
                    )
                }
                Surface::Torus {
                    frame,
                    major,
                    minor,
                } => {
                    let a = canonical_axis(frame.z);
                    let n = surface
                        .normal_at_world(probe)
                        .ok_or_else(|| fail("no normal"))?;
                    let radial = sub(
                        probe,
                        add(
                            frame.origin,
                            mul(frame.z, dot(sub(probe, frame.origin), frame.z)),
                        ),
                    );
                    (
                        Kind::Torus,
                        a,
                        nearest_to_origin(frame.origin, a),
                        *major,
                        *minor,
                        dot(mul(n, sense), radial) > 0.,
                    )
                }
            };
            out.insert(
                f.id,
                FaceSignature {
                    kind,
                    direction,
                    anchor,
                    size,
                    size2,
                    convex,
                    lo,
                    hi,
                    loops: f.bounds.len(),
                    edges,
                    neighbours,
                },
            );
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerances {
    /// Positions (plane offset, axis point, centre, bounding box) agree within this many millimetres.
    pub position_tol: f64,
    /// Directions agree within this angle (radians); sizes (radius, angle) within `position_tol`.
    pub angle_tol: f64,
    /// A face with the same kind, size and topology within this distance (centre of its extent) is the *moved* face.
    pub search_radius: f64,
}
impl Default for Tolerances {
    fn default() -> Self {
        Self {
            position_tol: 1e-5,
            angle_tol: 1e-7,
            search_radius: 5.,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Association {
    /// Faces whose invariants agree (a re-export, or an untouched face): `(old, new)`.
    pub exact: Vec<(u64, u64)>,
    /// The same surface with a changed topology (a hole added, a neighbour gone): `(old, new)`.
    pub modified: Vec<(u64, u64)>,
    /// Same kind, size and topology but displaced: `(old, new, displacement of the extent's centre)`.
    pub moved: Vec<(u64, u64, f64)>,
    /// Faces of the old model that have no counterpart.
    pub lost: Vec<u64>,
    /// Faces of the new model that no old face claims.
    pub added: Vec<u64>,
    /// Old faces that equally well match several new ones (symmetric features): not guessed.
    pub ambiguous: Vec<(u64, Vec<u64>)>,
}

fn same_direction(a: V3, b: V3, tol: f64) -> bool {
    // parallel or equal; the sign of an axis is already canonical, a plane normal is oriented
    norm(sub(a, b)) <= tol
}

fn strict(a: &FaceSignature, b: &FaceSignature, t: &Tolerances) -> bool {
    a.kind == b.kind
        && a.convex == b.convex
        && a.loops == b.loops
        && a.edges == b.edges
        && a.neighbours == b.neighbours
        && (a.size - b.size).abs() <= t.position_tol
        && (a.size2 - b.size2).abs() <= t.position_tol
        && same_direction(a.direction, b.direction, t.angle_tol.max(1e-12) * 10.)
        && norm(sub(a.anchor, b.anchor)) <= t.position_tol
        && norm(sub(a.lo, b.lo)) <= t.position_tol * 2.
        && norm(sub(a.hi, b.hi)) <= t.position_tol * 2.
}

/// Same underlying surface, whatever the topology.
fn same_surface(a: &FaceSignature, b: &FaceSignature, t: &Tolerances) -> bool {
    a.kind == b.kind
        && a.convex == b.convex
        && (a.size - b.size).abs() <= t.position_tol
        && (a.size2 - b.size2).abs() <= t.position_tol
        && same_direction(a.direction, b.direction, t.angle_tol.max(1e-12) * 10.)
        && norm(sub(a.anchor, b.anchor)) <= t.position_tol
}

/// Overlap of the two extents relative to the smaller one (axes of zero thickness count with a tiny one).
fn overlap(a: &FaceSignature, b: &FaceSignature) -> f64 {
    let (mut inter, mut va, mut vb) = (1.0, 1.0, 1.0);
    for k in 0..3 {
        let (la, lb) = ((a.hi[k] - a.lo[k]).max(1e-9), (b.hi[k] - b.lo[k]).max(1e-9));
        let o = (a.hi[k].min(b.hi[k]) - a.lo[k].max(b.lo[k])).max(0.).max(
            if a.hi[k].min(b.hi[k]) >= a.lo[k].max(b.lo[k]) - 1e-9 {
                1e-9
            } else {
                0.
            },
        );
        inter *= o;
        va *= la;
        vb *= lb;
    }
    inter / va.min(vb)
}

fn relaxed(a: &FaceSignature, b: &FaceSignature, t: &Tolerances) -> Option<f64> {
    let displacement = norm(sub(a.centre(), b.centre()));
    (a.kind == b.kind
        && a.convex == b.convex
        && a.loops == b.loops
        && a.edges == b.edges
        && a.neighbours == b.neighbours
        && (a.size - b.size).abs() <= t.position_tol
        && (a.size2 - b.size2).abs() <= t.position_tol
        && same_direction(a.direction, b.direction, t.angle_tol.max(1e-12) * 10.)
        && displacement <= t.search_radius)
        .then_some(displacement)
}

/// Associate the faces of `old` with those of `new` (see the module documentation). A face is never matched twice; a tie is
/// reported as ambiguous.
pub fn associate(
    old: &BTreeMap<u64, FaceSignature>,
    new: &BTreeMap<u64, FaceSignature>,
    tolerances: Tolerances,
) -> Association {
    let mut taken: BTreeMap<u64, u64> = BTreeMap::new(); // new -> old
    let mut result = Association {
        exact: Vec::new(),
        modified: Vec::new(),
        moved: Vec::new(),
        lost: Vec::new(),
        added: Vec::new(),
        ambiguous: Vec::new(),
    };
    let mut open: Vec<u64> = old.keys().copied().collect();
    // pass 1: exact
    let mut still = Vec::new();
    for o in open.drain(..) {
        let candidates: Vec<u64> = new
            .iter()
            .filter(|(n, s)| !taken.contains_key(n) && strict(&old[&o], s, &tolerances))
            .map(|(n, _)| *n)
            .collect();
        match candidates.as_slice() {
            [n] => {
                // a claim on the same new face by another old face is a tie
                let rivals: Vec<u64> = old
                    .iter()
                    .filter(|(k, s)| **k != o && strict(s, &new[n], &tolerances))
                    .map(|(k, _)| *k)
                    .collect();
                if rivals.is_empty() {
                    taken.insert(*n, o);
                    result.exact.push((o, *n));
                } else {
                    result.ambiguous.push((o, vec![*n]));
                }
            }
            [] => still.push(o),
            many => result.ambiguous.push((o, many.to_vec())),
        }
    }
    // pass 2: modified (the same surface, another topology): the clearest overlap wins, one new face per old face
    let mut candidates: Vec<(f64, u64, u64)> = Vec::new();
    for &o in &still {
        for (n, s) in new {
            if !taken.contains_key(n) && same_surface(&old[&o], s, &tolerances) {
                let ov = overlap(&old[&o], s);
                if ov > 0.0 {
                    candidates.push((ov, o, *n));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut modified_old: BTreeMap<u64, ()> = BTreeMap::new();
    for &(ov, o, n) in &candidates {
        if modified_old.contains_key(&o) || taken.contains_key(&n) {
            continue;
        }
        // not clear when another untaken pair of either face overlaps almost as much
        let rival = candidates.iter().any(|&(ov2, o2, n2)| {
            (o2 == o) != (n2 == n)
                && !modified_old.contains_key(&o2)
                && !taken.contains_key(&n2)
                && ov2 >= 0.8 * ov
        });
        if rival {
            result.ambiguous.push((o, vec![n]));
            modified_old.insert(o, ());
            continue;
        }
        taken.insert(n, o);
        modified_old.insert(o, ());
        result.modified.push((o, n));
    }
    let still: Vec<u64> = still
        .into_iter()
        .filter(|o| !modified_old.contains_key(o))
        .collect();
    // pass 3: moved, nearest first, one new face per old face
    let mut pairs: Vec<(f64, u64, u64)> = Vec::new();
    for &o in &still {
        for (n, s) in new {
            if !taken.contains_key(n) {
                if let Some(d) = relaxed(&old[&o], s, &tolerances) {
                    pairs.push((d, o, *n));
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut matched_old: BTreeMap<u64, ()> = BTreeMap::new();
    // an old face with several equally near candidates (or a new face claimed by several equally near old ones) is a tie
    let tie_tol = tolerances.position_tol;
    let mut tied: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for &(d, o, n) in &pairs {
        let same_old: Vec<u64> = pairs
            .iter()
            .filter(|(d2, o2, _)| *o2 == o && (d2 - d).abs() <= tie_tol)
            .map(|p| p.2)
            .collect();
        let same_new = pairs
            .iter()
            .filter(|(d2, _, n2)| *n2 == n && (d2 - d).abs() <= tie_tol)
            .count();
        let nearest = pairs
            .iter()
            .filter(|(_, o2, _)| *o2 == o)
            .map(|p| p.0)
            .fold(f64::INFINITY, f64::min);
        if (d - nearest).abs() <= tie_tol && (same_old.len() > 1 || same_new > 1) {
            tied.entry(o).or_default().extend(same_old);
        }
    }
    for (o, mut candidates) in tied {
        candidates.sort();
        candidates.dedup();
        matched_old.insert(o, ());
        result.ambiguous.push((o, candidates));
    }
    for (d, o, n) in pairs {
        if matched_old.contains_key(&o)
            || taken.contains_key(&n)
            || result.ambiguous.iter().any(|(_, c)| c.contains(&n))
        {
            continue;
        }
        taken.insert(n, o);
        matched_old.insert(o, ());
        result.moved.push((o, n, d));
    }
    for o in still {
        if !matched_old.contains_key(&o) {
            result.lost.push(o);
        }
    }
    result.added = new
        .keys()
        .copied()
        .filter(|n| !taken.contains_key(n) && !result.ambiguous.iter().any(|(_, c)| c.contains(n)))
        .collect();
    result.exact.sort();
    result.modified.sort();
    result.moved.sort_by_key(|m| (m.0, m.1));
    result.lost.sort();
    result
}
