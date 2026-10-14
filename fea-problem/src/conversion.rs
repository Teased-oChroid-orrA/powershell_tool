//! Changing a support or load to another kind without losing what it means: a traction becomes the force it adds up to, a
//! point force becomes a pressure on a face, a fixed face becomes a roller. The resultant is kept; what a new kind cannot
//! hold (the tangential part of a force as a pressure) is dropped and reported. The measurements of the surfaces come from
//! [`Facts`].
use crate::facts::Facts;
use crate::problem::{Load, Problem, Support};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadKind {
    Pressure,
    Traction,
    Force,
    /// Pin load on a hole.
    Bearing,
    /// Concentrated force at a point.
    Point,
}
impl LoadKind {
    pub const ALL: [LoadKind; 5] = [Self::Pressure, Self::Traction, Self::Force, Self::Bearing, Self::Point];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pressure => "Pressure",
            Self::Traction => "Traction",
            Self::Force => "Force",
            Self::Bearing => "Pin load",
            Self::Point => "Point force",
        }
    }
    pub fn of(l: &Load) -> Option<LoadKind> {
        Some(match l {
            Load::Pressure { .. } => Self::Pressure,
            Load::Traction { .. } => Self::Traction,
            Load::Force { .. } => Self::Force,
            Load::Bearing { .. } => Self::Bearing,
            Load::Point { .. } => Self::Point,
            Load::Body { .. } => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportKind {
    /// Every direction held.
    Fixed,
    /// Only the direction normal to the face held: the face slides in the others.
    Slides,
    /// The directions held at a point instead of a face.
    Point,
}
impl SupportKind {
    pub const ALL: [SupportKind; 3] = [Self::Fixed, Self::Slides, Self::Point];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Slides => "Slides on the face",
            Self::Point => "Held at a point",
        }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// The face of `facts` nearest to a point, by centroid.
pub fn nearest_face(facts: &Facts, at: [f64; 3]) -> Option<String> {
    facts
        .faces
        .iter()
        .filter(|(_, f)| f.area > 0.0)
        .min_by(|a, b| {
            let d = |c: [f64; 3]| (c[0] - at[0]).powi(2) + (c[1] - at[1]).powi(2) + (c[2] - at[2]).powi(2);
            d(a.1.centroid).total_cmp(&d(b.1.centroid))
        })
        .map(|(n, _)| n.clone())
}

impl Problem {
    /// Change load `i` to `to`, acting on `surface` (the load's own surface when it has one). Returns what was lost.
    pub fn convert_load(&mut self, i: usize, to: LoadKind, surface: Option<&str>, facts: &Facts) -> Result<Vec<String>, String> {
        let load = self.loads.get(i).cloned().ok_or("that load no longer exists")?;
        let from = LoadKind::of(&load).ok_or("a body force has no other kind: it acts on the whole part")?;
        if from == to {
            return Err(format!("it already is a {}", to.label().to_lowercase()));
        }
        let face = |name: &str| facts.faces.get(name).ok_or_else(|| format!("the size of '{name}' is not known: generate the mesh first"));
        // The resultant force vector, and the surface it acts on when it has one.
        let (resultant, own): ([f64; 3], Option<String>) = match &load {
            Load::Pressure { edge, p } => {
                let f = face(edge)?;
                if norm(f.normal) == 0.0 {
                    return Err(format!("'{edge}' has no single direction (a hole or a curved face): its pressure adds up to nothing to convert"));
                }
                ([-p * f.area * f.normal[0], -p * f.area * f.normal[1], -p * f.area * f.normal[2]], Some(edge.clone()))
            }
            Load::Traction { edge, tx, ty, tz } => {
                let f = face(edge)?;
                ([tx * f.area, ty * f.area, tz * f.area], Some(edge.clone()))
            }
            Load::Force { edge, fx, fy, fz } => ([*fx, *fy, *fz], Some(edge.clone())),
            Load::Bearing { hole, fx, fy } => ([*fx, *fy, 0.0], Some(format!("hole{hole}"))),
            Load::Point { fx, fy, fz, x, y, z } => ([*fx, *fy, *fz], nearest_face(facts, [*x, *y, *z])),
            Load::Body { .. } => unreachable!("handled above"),
        };
        let target = surface.map(str::to_string).or(own).ok_or("pick the surface the new load acts on")?;
        let mut notes = Vec::new();
        let new = match to {
            LoadKind::Force => Load::Force { edge: target.clone(), fx: resultant[0], fy: resultant[1], fz: resultant[2] },
            LoadKind::Traction => {
                let a = face(&target)?.area;
                if a <= 0.0 {
                    return Err(format!("'{target}' has no area"));
                }
                Load::Traction { edge: target.clone(), tx: resultant[0] / a, ty: resultant[1] / a, tz: resultant[2] / a }
            }
            LoadKind::Pressure => {
                let f = face(&target)?;
                if norm(f.normal) == 0.0 || f.area <= 0.0 {
                    return Err(format!("'{target}' has no single direction: a pressure needs a flat face"));
                }
                let normal = dot(resultant, f.normal);
                let tangential = (norm(resultant).powi(2) - normal * normal).max(0.0).sqrt();
                if tangential > 0.01 * norm(resultant) {
                    notes.push(format!("the part of the force along the face ({tangential:.4e}) cannot be a pressure and was dropped"));
                }
                Load::Pressure { edge: target.clone(), p: -normal / f.area }
            }
            LoadKind::Bearing => {
                let Some(n) = target.strip_prefix("hole").and_then(|d| d.parse::<usize>().ok()) else { return Err("a pin load acts on a hole: pick a hole".into()) };
                if !matches!(self.analysis, crate::problem::Analysis::PlaneStress | crate::problem::Analysis::PlaneStrain) {
                    return Err("a pin load is for plane problems".into());
                }
                if resultant[2].abs() > 0.01 * norm(resultant) {
                    notes.push("the out-of-plane part of the force was dropped".into());
                }
                Load::Bearing { hole: n, fx: resultant[0], fy: resultant[1] }
            }
            LoadKind::Point => {
                let at = match &load {
                    Load::Point { x, y, z, .. } if surface.is_none() => [*x, *y, *z],
                    _ => face(&target)?.centroid,
                };
                Load::Point { x: at[0], y: at[1], z: at[2], fx: resultant[0], fy: resultant[1], fz: resultant[2] }
            }
        };
        self.loads[i] = new;
        Ok(notes)
    }

    /// Change support `i` to `to`, on `surface` (the support's own surface when it has one).
    pub fn convert_support(&mut self, i: usize, to: SupportKind, surface: Option<&str>, facts: &Facts) -> Result<Vec<String>, String> {
        let sup = self.supports.get(i).cloned().ok_or("that support no longer exists")?;
        let dim = self.analysis_dim();
        let comps = sup.comps();
        let own = match &sup {
            Support::Edge { edge, .. } => Some(edge.clone()),
            Support::Point { x, y, z, .. } => nearest_face(facts, [*x, *y, *z]),
        };
        let target = surface.map(str::to_string).or(own).ok_or("pick the surface the new support acts on")?;
        let face = facts.faces.get(&target);
        let mut notes = Vec::new();
        let held: [Option<f64>; 3] = match to {
            SupportKind::Fixed => std::array::from_fn(|k| if k < dim { Some(comps[k].unwrap_or(0.0)) } else { None }),
            SupportKind::Slides => {
                let f = face.filter(|f| norm(f.normal) > 0.0).ok_or_else(|| format!("'{target}' has no single direction: a roller needs a flat face"))?;
                let k = (0..dim).max_by(|a, b| f.normal[*a].abs().total_cmp(&f.normal[*b].abs())).unwrap_or(0);
                if f.normal[k].abs() < 0.95 {
                    notes.push(format!("'{target}' is inclined: the roller holds the nearest axis ({})", ["x", "y", "z"][k]));
                }
                std::array::from_fn(|c| if c == k { Some(comps[c].unwrap_or(0.0)) } else { None })
            }
            SupportKind::Point => comps,
        };
        self.supports[i] = match to {
            SupportKind::Point => {
                let at = face.map(|f| f.centroid).ok_or_else(|| format!("the position of '{target}' is not known: generate the mesh first"))?;
                Support::Point { x: at[0], y: at[1], z: at[2], ux: held[0], uy: held[1], uz: held[2] }
            }
            _ => Support::Edge { edge: target, ux: held[0], uy: held[1], uz: held[2] },
        };
        Ok(notes)
    }
}
