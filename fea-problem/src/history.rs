//! Feature history: the ordered, editable recipe a solid is rebuilt from (sketch on a plane, then
//! extrude / revolve), with suppress and roll-back like a CAD feature tree. `Problem::apply_history` turns the
//! active features into the `Geometry` the mesher takes; supports and loads keep referring to the same
//! `edgeN` / `holeN` / `start` / `end` names because the sketch profile is extracted in a deterministic order.
//! Design: `docs/adr/ADR-015-parametric-sketch-history.md`.
use crate::sketch::{Id, Sketch};
use serde::{Deserialize, Serialize};

/// Where a sketch lives in 3D. Standard planes follow SolidWorks: Front = XY (normal +Z), Top = XZ (normal +Y),
/// Right = YZ (normal +X); `offset` moves the plane along its normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Plane {
    Front { offset: f64 },
    Top { offset: f64 },
    Right { offset: f64 },
    /// User datum plane through `origin` with `normal`; `x_dir` (projected into the plane) is the sketch x axis.
    Datum { origin: [f64; 3], normal: [f64; 3], x_dir: [f64; 3] },
}

impl Plane {
    pub fn label(&self) -> &'static str {
        match self {
            Plane::Front { .. } => "Front plane (XY)",
            Plane::Top { .. } => "Top plane (XZ)",
            Plane::Right { .. } => "Right plane (YZ)",
            Plane::Datum { .. } => "Datum plane",
        }
    }
    pub fn frame(&self) -> Result<Frame, String> {
        let unit = |v: [f64; 3]| -> Result<[f64; 3], String> {
            let n = v[0].hypot(v[1]).hypot(v[2]);
            if n.is_finite() && n > 1e-12 { Ok([v[0] / n, v[1] / n, v[2] / n]) } else { Err("a datum plane needs a non-zero normal and x direction".into()) }
        };
        Ok(match self {
            Plane::Front { offset } => Frame { origin: [0.0, 0.0, *offset], u: [1.0, 0.0, 0.0], v: [0.0, 1.0, 0.0] },
            Plane::Top { offset } => Frame { origin: [0.0, *offset, 0.0], u: [1.0, 0.0, 0.0], v: [0.0, 0.0, -1.0] },
            Plane::Right { offset } => Frame { origin: [*offset, 0.0, 0.0], u: [0.0, 1.0, 0.0], v: [0.0, 0.0, 1.0] },
            Plane::Datum { origin, normal, x_dir } => {
                let w = unit(*normal)?;
                let d = unit(*x_dir)?;
                let k = d[0] * w[0] + d[1] * w[1] + d[2] * w[2];
                let u = unit([d[0] - k * w[0], d[1] - k * w[1], d[2] - k * w[2]]).map_err(|_| "the datum x direction is parallel to its normal".to_string())?;
                let v = [w[1] * u[2] - w[2] * u[1], w[2] * u[0] - w[0] * u[2], w[0] * u[1] - w[1] * u[0]];
                Frame { origin: *origin, u, v }
            }
        })
    }
}

/// Orthonormal sketch frame: local `(x, y, z)` maps to `origin + x u + y v + z w`, `w = u x v`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
}

impl Frame {
    pub fn w(&self) -> [f64; 3] {
        let (u, v) = (self.u, self.v);
        [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    }
    pub fn is_identity(&self) -> bool {
        *self == Frame { origin: [0.0; 3], u: [1.0, 0.0, 0.0], v: [0.0, 1.0, 0.0] }
    }
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let w = self.w();
        std::array::from_fn(|i| self.origin[i] + p[0] * self.u[i] + p[1] * self.v[i] + p[2] * w[i])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FeatureKind {
    Sketch { plane: Plane, sketch: Sketch },
    /// Extrude the profile of sketch feature `profile` by `depth` along the plane normal, in `layers` element layers.
    Extrude { profile: Id, depth: f64, layers: usize },
    /// Revolve the profile about the sketch's y axis (x = radius, x >= 0) through `angle_deg`, in `segments` layers.
    Revolve { profile: Id, angle_deg: f64, segments: usize },
    /// Sweep the profile of sketch feature `profile` along the open path of sketch feature `path` (tangent-continuous lines
    /// and arcs that start on the profile's plane and leave it along its normal), in about `layers` element layers.
    Sweep { profile: Id, path: Id, layers: usize },
}

impl FeatureKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sketch { .. } => "Sketch",
            Self::Extrude { .. } => "Extrude",
            Self::Revolve { .. } => "Revolve",
            Self::Sweep { .. } => "Sweep",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub suppressed: bool,
    #[serde(flatten)]
    pub kind: FeatureKind,
}

/// A flat face of the body the active features build, as a plane a new sketch can be placed on (the datum-plane picker).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanarFace {
    /// The mesh surface name of the face (`start`, `end`, `bottom`, `edge3`, ...).
    pub name: String,
    /// Datum plane whose normal points out of the body, `x_dir` along the face's first direction.
    pub plane: Plane,
}

/// What the active features produce.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// A sketch only (a plane problem lives in the XY plane).
    Region,
    Extrude { depth: f64, layers: usize },
    Revolve { angle_deg: f64, segments: usize },
    /// The profile moved along `path`; `origin` is the path's start point in the profile sketch's coordinates.
    Sweep { path: crate::path::Path3, layers: usize, origin: [f64; 2] },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub outer: crate::Shape,
    pub holes: Vec<crate::Shape>,
    pub regions: Vec<crate::SketchRegion>,
    pub body: Body,
    pub frame: Frame,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub next_id: Id,
    pub features: Vec<Feature>,
    /// Roll-back bar: only the first `n` features are active (None = all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<usize>,
}

impl History {
    /// The history equivalent to a sketch problem's outline and holes (Front plane; an extrude when it is a solid).
    pub fn from_problem(p: &crate::Problem) -> Result<History, String> {
        let crate::Geometry::Sketch { outer, holes, depth, layers, .. } = &p.geometry else { return Err("an imported mesh has no sketch to turn into a history".into()) };
        let mut h = History::default();
        let sk = h.add("Sketch1", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: Sketch::from_shapes(outer, holes) });
        if p.analysis == crate::Analysis::Solid {
            h.add("Extrude1", FeatureKind::Extrude { profile: sk, depth: *depth, layers: *layers });
        }
        Ok(h)
    }

    fn take_id(&mut self) -> Id {
        let id = self.next_id.max(self.features.iter().map(|f| f.id + 1).max().unwrap_or(0));
        self.next_id = id + 1;
        id
    }
    pub fn add(&mut self, name: &str, kind: FeatureKind) -> Id {
        // New features go in at the roll-back bar (as in CAD systems) and the bar moves past them.
        let at = self.active_len();
        let id = self.take_id();
        self.features.insert(at, Feature { id, name: name.into(), suppressed: false, kind });
        if self.rollback.is_some() {
            self.rollback = Some(at + 1);
        }
        id
    }
    pub fn feature(&self, id: Id) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }
    pub fn sketch_mut(&mut self, id: Id) -> Option<(&mut Plane, &mut Sketch)> {
        self.features.iter_mut().find(|f| f.id == id).and_then(|f| match &mut f.kind {
            FeatureKind::Sketch { plane, sketch } => Some((plane, sketch)),
            _ => None,
        })
    }
    pub fn active_len(&self) -> usize {
        self.rollback.map_or(self.features.len(), |n| n.min(self.features.len()))
    }
    /// Delete a feature and the features that depend on it.
    pub fn remove(&mut self, id: Id) {
        self.features.retain(|f| {
            f.id != id
                && match f.kind {
                    FeatureKind::Extrude { profile, .. } | FeatureKind::Revolve { profile, .. } => profile != id,
                    FeatureKind::Sweep { profile, path, .. } => profile != id && path != id,
                    FeatureKind::Sketch { .. } => true,
                }
        });
        self.rollback = self.rollback.map(|n| n.min(self.features.len()));
    }

    /// The flat faces of the extruded body: the two caps and every straight side of the outline, each as an outward-facing
    /// datum plane. A revolve and a plain sketch have none (their faces are curved or not a body yet).
    pub fn planar_faces(&self) -> Result<Vec<PlanarFace>, String> {
        use crate::problem::{Seg, Shape};
        let built = self.rebuild()?;
        let Body::Extrude { depth, .. } = built.body else { return Ok(Vec::new()) };
        let (f, w) = (built.frame, built.frame.w());
        let datum = |origin: [f64; 3], normal: [f64; 3], x_dir: [f64; 3]| Plane::Datum { origin, normal, x_dir };
        let neg = |v: [f64; 3]| v.map(|c| -c);
        let mut out = vec![
            PlanarFace { name: "start".into(), plane: datum(f.origin, neg(w), f.u) },
            PlanarFace { name: "end".into(), plane: datum(f.apply([0.0, 0.0, depth]), w, f.u) },
        ];
        // Straight sides: `(name, from, to)` in sketch coordinates, in the order the mesher names them.
        let sides: Vec<(String, [f64; 2], [f64; 2])> = match &built.outer {
            Shape::Rect { x0, y0, x1, y1 } => vec![
                ("bottom".into(), [*x0, *y0], [*x1, *y0]),
                ("right".into(), [*x1, *y0], [*x1, *y1]),
                ("top".into(), [*x1, *y1], [*x0, *y1]),
                ("left".into(), [*x0, *y1], [*x0, *y0]),
            ],
            Shape::Polygon { pts } => (0..pts.len()).map(|i| (format!("edge{}", i + 1), pts[i], pts[(i + 1) % pts.len()])).collect(),
            Shape::Outline { segs } => segs.iter().enumerate().filter(|(_, sg)| matches!(sg, Seg::Line { .. })).map(|(i, sg)| (format!("edge{}", i + 1), sg.start(), sg.end())).collect(),
            Shape::Circle { .. } | Shape::Slot { .. } => Vec::new(),
        };
        // Outward is to the right of a counter-clockwise walk and to the left of a clockwise one.
        let pts: Vec<[f64; 2]> = match &built.outer {
            Shape::Outline { segs } => segs.iter().flat_map(|sg| sg.sample(48.0)).collect(),
            Shape::Polygon { pts } => pts.clone(),
            other => {
                let (lo, hi) = other.bounds();
                vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]
            }
        };
        let area2: f64 = (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[i][1] * pts[(i + 1) % pts.len()][0]).sum();
        let side = if area2 >= 0.0 { 1.0 } else { -1.0 };
        for (name, a, b) in sides {
            let d = [b[0] - a[0], b[1] - a[1]];
            let len = d[0].hypot(d[1]);
            if len.is_nan() || len <= 0.0 {
                continue;
            }
            let (d, n) = ([d[0] / len, d[1] / len], [side * d[1] / len, -side * d[0] / len]);
            let dir = |c: [f64; 2]| -> [f64; 3] { std::array::from_fn(|i| c[0] * f.u[i] + c[1] * f.v[i]) };
            out.push(PlanarFace { name, plane: datum(f.apply([a[0], a[1], 0.0]), dir(n), dir(d)) });
        }
        Ok(out)
    }

    /// The sweep body: the path of sketch `path` placed in 3D and checked against the plane of sketch `profile`.
    fn sweep_body(&self, active: &[&Feature], profile: Id, path: Id, layers: usize) -> Result<Body, String> {
        if layers == 0 {
            return Err("a sweep needs at least one layer".into());
        }
        let find = |id: Id, what: &str| -> Result<(&str, &Plane, &Sketch), String> {
            let f = active.iter().find(|f| f.id == id).ok_or_else(|| format!("the sweep's {what} sketch is missing, suppressed or rolled back"))?;
            match &f.kind {
                FeatureKind::Sketch { plane, sketch } => Ok((f.name.as_str(), plane, sketch)),
                _ => Err(format!("the sweep's {what} must be a sketch")),
            }
        };
        let (pname, pplane, _) = find(profile, "profile")?;
        let (qname, qplane, qsketch) = find(path, "path")?;
        if profile == path {
            return Err("a sweep needs two sketches: the profile and the path".into());
        }
        let pf = pplane.frame().map_err(|e| format!("sketch '{pname}': {e}"))?;
        let qf = qplane.frame().map_err(|e| format!("sketch '{qname}': {e}"))?;
        let mut qs = qsketch.clone();
        if !qs.solve().violated.is_empty() {
            return Err(format!("sketch '{qname}' has conflicting constraints"));
        }
        let segs = qs.open_path().map_err(|e| format!("path sketch '{qname}': {e}"))?;
        // Walk the path from the end that lies on the profile plane; a path drawn from the other end is turned around.
        let w = pf.w();
        let on_plane = |p: [f64; 3]| (0..3).map(|i| (p[i] - pf.origin[i]) * w[i]).sum::<f64>().abs();
        let place = |q: [f64; 2]| qf.apply([q[0], q[1], 0.0]);
        let scale = segs.iter().map(|s| (s.end()[0] - s.start()[0]).hypot(s.end()[1] - s.start()[1])).fold(1e-9, f64::max);
        let tol = 1e-7 * scale.max(1.0);
        let segs = if on_plane(place(segs[0].start())) <= tol {
            segs
        } else if on_plane(place(segs.last().expect("non-empty").end())) <= tol {
            segs.iter().rev().map(|s| s.reversed()).collect()
        } else {
            return Err(format!("the path of '{qname}' does not start on the plane of '{pname}'; a swept path begins on the profile's plane"));
        };
        let start = place(segs[0].start());
        let path3 = crate::path::Path3::new(&segs, &place, qf.w(), pf.u, pf.v).map_err(|e| format!("path sketch '{qname}': {e}"))?;
        let d = [start[0] - pf.origin[0], start[1] - pf.origin[1], start[2] - pf.origin[2]];
        let origin = [(0..3).map(|i| d[i] * pf.u[i]).sum(), (0..3).map(|i| d[i] * pf.v[i]).sum()];
        Ok(Body::Sweep { path: path3, layers, origin })
    }

    /// Rebuild the profile and body from the active, unsuppressed features.
    pub fn rebuild(&self) -> Result<Built, String> {
        let active: Vec<&Feature> = self.features[..self.active_len()].iter().filter(|f| !f.suppressed).collect();
        let bodies: Vec<&&Feature> = active.iter().filter(|f| !matches!(f.kind, FeatureKind::Sketch { .. })).collect();
        if bodies.len() > 1 {
            return Err("only one solid feature (extrude or revolve) is supported; put every outline and hole in its sketch".into());
        }
        let (profile_id, body) = match bodies.first().map(|f| &f.kind) {
            Some(FeatureKind::Extrude { profile, depth, layers }) => (*profile, Body::Extrude { depth: *depth, layers: *layers }),
            Some(FeatureKind::Revolve { profile, angle_deg, segments }) => (*profile, Body::Revolve { angle_deg: *angle_deg, segments: *segments }),
            Some(FeatureKind::Sweep { profile, path, layers }) => (*profile, self.sweep_body(&active, *profile, *path, *layers)?),
            Some(FeatureKind::Sketch { .. }) | None => {
                let last = active.iter().rev().find(|f| matches!(f.kind, FeatureKind::Sketch { .. })).ok_or("the history has no sketch")?;
                (last.id, Body::Region)
            }
        };
        let f = active.iter().find(|f| f.id == profile_id).ok_or("the feature refers to a sketch that is missing, suppressed or rolled back")?;
        let FeatureKind::Sketch { plane, sketch } = &f.kind else { return Err("the feature must refer to a sketch".into()) };
        let frame = plane.frame()?;
        let mut sketch = sketch.clone();
        let report = sketch.solve();
        if !report.violated.is_empty() {
            return Err(format!("sketch '{}' has conflicting constraints", f.name));
        }
        let prof = sketch.profile().map_err(|e| format!("sketch '{}': {e}", f.name))?;
        match &body {
            Body::Extrude { depth, layers } if !(depth.is_finite() && *depth > 0.0) || *layers == 0 => return Err("an extrude needs a positive depth and at least one layer".into()),
            Body::Revolve { angle_deg, segments } => {
                if !(angle_deg.is_finite() && *angle_deg > 0.0 && *angle_deg <= 360.0) || *segments == 0 {
                    return Err("a revolve needs an angle in (0, 360] degrees and at least one segment".into());
                }
                let (lo, _) = prof.outer.bounds();
                if lo[0] < -1e-9 {
                    return Err("a revolve profile must lie on one side of the axis (x >= 0 in the sketch)".into());
                }
            }
            _ => {}
        }
        if matches!(body, Body::Region) && !frame.is_identity() && !matches!(plane, Plane::Front { offset } if *offset == 0.0) {
            // A plane problem is solved in XY; a sketch elsewhere only becomes a part once it is extruded.
            return Err("a plane problem needs its sketch on the Front plane; extrude or revolve to place a part elsewhere".into());
        }
        Ok(Built { outer: prof.outer, holes: prof.holes, regions: prof.regions, body, frame })
    }
}
