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
}

impl FeatureKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sketch { .. } => "Sketch",
            Self::Extrude { .. } => "Extrude",
            Self::Revolve { .. } => "Revolve",
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

/// What the active features produce.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// A sketch only (a plane problem lives in the XY plane).
    Region,
    Extrude { depth: f64, layers: usize },
    Revolve { angle_deg: f64, segments: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub outer: crate::Shape,
    pub holes: Vec<crate::Shape>,
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
        let crate::Geometry::Sketch { outer, holes, depth, layers } = &p.geometry else { return Err("an imported mesh has no sketch to turn into a history".into()) };
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
        self.features.retain(|f| f.id != id && !matches!(f.kind, FeatureKind::Extrude { profile, .. } | FeatureKind::Revolve { profile, .. } if profile == id));
        self.rollback = self.rollback.map(|n| n.min(self.features.len()));
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
        Ok(Built { outer: prof.outer, holes: prof.holes, body, frame })
    }
}
