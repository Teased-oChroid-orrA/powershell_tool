//! The serialisable problem description.
//!
//! Edge naming (what supports and loads refer to): an outer `Rect` has `bottom`, `right`, `top`,
//! `left`; an outer `Polygon` has `edge1..edgeN`; an outer `Circle` / `Slot` is `outer`; hole `i`
//! (1-based, in list order) is `hole{i}` whatever its shape. An extruded problem adds `start`
//! (`z = 0`) and `end` (`z = depth`). An imported mesh uses the names its file defines.

use serde::{Deserialize, Serialize};

/// Format version written to JSON; bump when a field changes meaning.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Analysis {
    PlaneStress,
    PlaneStrain,
    /// `x` = radius, `y` = axis; loads are totals over 360 degrees.
    Axisymmetric,
    /// 3D solid: an extruded 2D sketch or an imported volume mesh.
    Solid,
}

impl Analysis {
    pub const ALL: [Analysis; 4] = [Analysis::PlaneStress, Analysis::PlaneStrain, Analysis::Axisymmetric, Analysis::Solid];

    pub fn label(self) -> &'static str {
        match self {
            Analysis::PlaneStress => "Plane stress",
            Analysis::PlaneStrain => "Plane strain",
            Analysis::Axisymmetric => "Axisymmetric",
            Analysis::Solid => "3D solid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElementChoice {
    Tri3,
    Tri6,
    Quad4,
    Quad8,
    Quad9,
}

impl ElementChoice {
    pub const ALL: [ElementChoice; 5] = [ElementChoice::Quad8, ElementChoice::Quad9, ElementChoice::Quad4, ElementChoice::Tri6, ElementChoice::Tri3];

    pub fn label(self) -> &'static str {
        match self {
            ElementChoice::Tri3 => "Tri3 (linear triangle)",
            ElementChoice::Tri6 => "Tri6 (quadratic triangle)",
            ElementChoice::Quad4 => "Quad4 (bilinear)",
            ElementChoice::Quad8 => "Quad8 (serendipity)",
            ElementChoice::Quad9 => "Quad9 (Lagrange)",
        }
    }

    pub fn is_quad(self) -> bool {
        matches!(self, ElementChoice::Quad4 | ElementChoice::Quad8 | ElementChoice::Quad9)
    }

    pub fn is_quadratic(self) -> bool {
        matches!(self, ElementChoice::Tri6 | ElementChoice::Quad8 | ElementChoice::Quad9)
    }

    pub fn kind(self) -> fea_core::ElementKind {
        use fea_core::ElementKind as K;
        match self {
            ElementChoice::Tri3 => K::Tri3,
            ElementChoice::Tri6 => K::Tri6,
            ElementChoice::Quad4 => K::Quad4,
            ElementChoice::Quad8 => K::Quad8,
            ElementChoice::Quad9 => K::Quad9,
        }
    }
}

/// Isotropic linear material. `yield_stress` only feeds the margin readout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialSpec {
    pub name: String,
    pub e: f64,
    pub nu: f64,
    /// Coefficient of thermal expansion (per degree).
    #[serde(default)]
    pub alpha: f64,
    #[serde(default)]
    pub yield_stress: Option<f64>,
    /// Mass density (mass per volume, in the problem's consistent units: with inch / psi, lbf s^2 / in^4 = weight density / 386.09).
    /// Only the natural-frequency analysis uses it; `0` = not given.
    #[serde(default)]
    pub density: f64,
}

impl MaterialSpec {
    pub fn elastic(&self) -> fea_core::Elastic {
        fea_core::Elastic::new(self.e, self.nu).with_alpha(self.alpha)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Shape {
    Rect { x0: f64, y0: f64, x1: f64, y1: f64 },
    Circle { cx: f64, cy: f64, r: f64 },
    /// Stadium: overall `length` x `width`, centred at `(cx, cy)`, long axis `angle` degrees from `x`.
    Slot { cx: f64, cy: f64, length: f64, width: f64, angle: f64 },
    Polygon { pts: Vec<[f64; 2]> },
}

impl Shape {
    pub fn label(&self) -> &'static str {
        match self {
            Shape::Rect { .. } => "Rectangle",
            Shape::Circle { .. } => "Circle",
            Shape::Slot { .. } => "Slot",
            Shape::Polygon { .. } => "Polygon",
        }
    }

    /// Bounding box `(min, max)`.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        match self {
            Shape::Rect { x0, y0, x1, y1 } => ([x0.min(*x1), y0.min(*y1)], [x0.max(*x1), y0.max(*y1)]),
            Shape::Circle { cx, cy, r } => ([cx - r, cy - r], [cx + r, cy + r]),
            Shape::Slot { cx, cy, length, width, angle } => {
                let (c, s) = (angle.to_radians().cos().abs(), angle.to_radians().sin().abs());
                let (hx, hy) = (0.5 * (length - width) * c + 0.5 * width, 0.5 * (length - width) * s + 0.5 * width);
                ([cx - hx, cy - hy], [cx + hx, cy + hy])
            }
            Shape::Polygon { pts } => {
                let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                for p in pts {
                    for i in 0..2 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
                (lo, hi)
            }
        }
    }

    /// Centre used to aim a bearing load and size the mesh refinement: the circle / slot centre,
    /// else the middle of the bounding box.
    pub fn centre(&self) -> [f64; 2] {
        match self {
            Shape::Circle { cx, cy, .. } | Shape::Slot { cx, cy, .. } => [*cx, *cy],
            _ => {
                let (lo, hi) = self.bounds();
                [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])]
            }
        }
    }

    /// Radius of the circle through the bounding box corners' farthest point from the centre.
    pub fn reach(&self) -> f64 {
        match self {
            Shape::Circle { r, .. } => *r,
            _ => {
                let (lo, hi) = self.bounds();
                0.5 * (hi[0] - lo[0]).hypot(hi[1] - lo[1])
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshSpec {
    pub element: ElementChoice,
    /// Target element edge length away from refinement.
    pub size: f64,
    /// Size multiplier at holes (1 = uniform, 0.25 = four times finer at the hole edge, blending
    /// back to `size` about two hole radii away).
    pub hole_factor: f64,
    /// Remeshing passes driven by the ZZ error estimate (0 = none): refine where the error is above `target_error`, coarsen
    /// where it is below; 2D parametric problems only (bushed plates included: the fit is re-solved on every mesh).
    pub adapt_passes: u8,
    /// Target relative energy-norm error of an adaptive pass.
    pub target_error: f64,
}

impl Default for MeshSpec {
    fn default() -> Self {
        Self { element: ElementChoice::Quad8, size: 0.25, hole_factor: 0.35, adapt_passes: 0, target_error: 0.02 }
    }
}

/// Where the geometry comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    /// A 2D sketch (outer boundary minus holes); extruded when the analysis is [`Analysis::Solid`].
    Sketch { outer: Shape, holes: Vec<Shape>, depth: f64, layers: usize },
    /// A mesh read from a Gmsh `.msh` or Abaqus `.inp` file (the app supplies its text).
    Imported { path: String },
}

/// One support: each component that is `Some(v)` is fixed to the displacement `v`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Support {
    Edge { edge: String, ux: Option<f64>, uy: Option<f64>, uz: Option<f64> },
    /// The node nearest `(x, y, z)`.
    Point { x: f64, y: f64, z: f64, ux: Option<f64>, uy: Option<f64>, uz: Option<f64> },
}

impl Support {
    pub fn fixed(edge: &str) -> Self {
        Support::Edge { edge: edge.into(), ux: Some(0.0), uy: Some(0.0), uz: Some(0.0) }
    }

    /// Symmetry / roller: only the component `comp` (0, 1, 2) is held at zero.
    pub fn roller(edge: &str, comp: usize) -> Self {
        let z = |c| if c == comp { Some(0.0) } else { None };
        Support::Edge { edge: edge.into(), ux: z(0), uy: z(1), uz: z(2) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Load {
    /// Normal pressure acting into the body.
    Pressure { edge: String, p: f64 },
    /// Force per unit area on an edge / face.
    Traction { edge: String, tx: f64, ty: f64, tz: f64 },
    /// Total force spread uniformly over an edge / face.
    Force { edge: String, fx: f64, fy: f64, fz: f64 },
    /// Cosine-distributed bearing load of resultant `(fx, fy)` on hole `hole` (1-based): the pin
    /// pushes on the half of the bore facing the load.
    Bearing { hole: usize, fx: f64, fy: f64 },
    /// Concentrated force on the node nearest `(x, y, z)`.
    Point { x: f64, y: f64, z: f64, fx: f64, fy: f64, fz: f64 },
    /// Body force per unit volume (weight density times acceleration direction).
    Body { bx: f64, by: f64, bz: f64 },
}

/// A bushing pressed into a circular hole of a plane problem: an annulus of its own material whose outer diameter is the
/// hole's plus `interference`, in frictional contact with the plate (the interference fit is installed first, then the
/// loads are applied). A load or support on `holeN` then acts on the bushing's bore. The bore may be offset from the
/// hole's centre (an eccentric bushing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bushing {
    /// Hole number (1-based, as in [`Load::Bearing`]).
    pub hole: usize,
    pub inner_diameter: f64,
    /// Bore centre relative to the hole centre.
    #[serde(default)]
    pub offset: [f64; 2],
    /// Diametral interference: bushing outer diameter minus the hole diameter.
    pub interference: f64,
    pub friction: f64,
    pub material: MaterialSpec,
}

impl Bushing {
    /// Editable numbers, in a fixed order.
    pub fn params(&self) -> Vec<(&'static str, f64)> {
        vec![("Hole number", self.hole as f64), ("Bore diameter", self.inner_diameter), ("Offset x", self.offset[0]), ("Offset y", self.offset[1]), ("Interference (dia)", self.interference), ("Friction", self.friction), ("Bushing E", self.material.e), ("Bushing nu", self.material.nu)]
    }

    pub fn set_param(&mut self, i: usize, v: f64) {
        match i {
            0 => self.hole = v.round().max(1.0) as usize,
            1 => self.inner_diameter = v,
            2 => self.offset[0] = v,
            3 => self.offset[1] = v,
            4 => self.interference = v,
            5 => self.friction = v,
            6 => self.material.e = v,
            _ => self.material.nu = v,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    #[serde(default = "version")]
    pub version: u32,
    pub name: String,
    pub analysis: Analysis,
    /// Plane problems: out-of-plane thickness (loads are per this thickness).
    pub thickness: f64,
    pub material: MaterialSpec,
    pub geometry: Geometry,
    pub mesh: MeshSpec,
    pub supports: Vec<Support>,
    pub loads: Vec<Load>,
    /// Uniform temperature change from the stress-free state.
    #[serde(default)]
    pub delta_t: f64,
    /// Interference-fit bushings in circular holes (plane analyses only).
    #[serde(default)]
    pub bushings: Vec<Bushing>,
}

fn version() -> u32 {
    FORMAT_VERSION
}

impl Problem {
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let p: Problem = serde_json::from_str(text).map_err(|e| format!("not a problem file: {e}"))?;
        if p.version > FORMAT_VERSION {
            return Err(format!("problem file version {} is newer than this program understands ({FORMAT_VERSION})", p.version));
        }
        Ok(p)
    }

    /// Names of the edges a support or load can refer to (sketch geometry only; an imported mesh's
    /// names are only known once its file is read).
    pub fn edge_names(&self) -> Vec<String> {
        let Geometry::Sketch { outer, holes, depth, .. } = &self.geometry else { return Vec::new() };
        let mut names: Vec<String> = match outer {
            Shape::Rect { .. } => ["bottom", "right", "top", "left"].iter().map(|s| s.to_string()).collect(),
            Shape::Polygon { pts } => (1..=pts.len()).map(|i| format!("edge{i}")).collect(),
            Shape::Circle { .. } | Shape::Slot { .. } => vec!["outer".into()],
        };
        names.extend((1..=holes.len()).map(|i| format!("hole{i}")));
        if self.analysis == Analysis::Solid && *depth > 0.0 {
            names.push("start".into());
            names.push("end".into());
        }
        names
    }

    /// Everything needed to mesh the problem (no supports or loads yet).
    pub fn validate_geometry(&self) -> Result<(), String> {
        self.material.elastic().validate()?;
        if !(self.thickness.is_finite() && self.thickness > 0.0) && matches!(self.analysis, Analysis::PlaneStress | Analysis::PlaneStrain) {
            return Err("thickness must be positive".into());
        }
        if !(self.mesh.size.is_finite() && self.mesh.size > 0.0) {
            return Err("mesh size must be positive".into());
        }
        if !(self.mesh.hole_factor.is_finite() && self.mesh.hole_factor > 0.0 && self.mesh.hole_factor <= 1.0) {
            return Err("hole size factor must be in (0, 1]".into());
        }
        match &self.geometry {
            Geometry::Sketch { outer, holes, depth, layers } => {
                if self.analysis == Analysis::Solid {
                    if !self.mesh.element.is_quad() {
                        return Err("a 3D extrusion needs quadrilateral elements (Quad4 / Quad8 / Quad9)".into());
                    }
                    if !(depth.is_finite() && *depth > 0.0) || *layers == 0 {
                        return Err("an extrusion needs a positive depth and at least one layer".into());
                    }
                }
                for s in std::iter::once(outer).chain(holes.iter()) {
                    check_shape(s)?;
                }
            }
            Geometry::Imported { path } => {
                if path.trim().is_empty() {
                    return Err("no mesh file chosen".into());
                }
            }
        }
        self.validate_bushings()?;
        Ok(())
    }

    fn validate_bushings(&self) -> Result<(), String> {
        if self.bushings.is_empty() {
            return Ok(());
        }
        let Geometry::Sketch { holes, .. } = &self.geometry else { return Err("a bushing needs a sketch hole (not an imported mesh)".into()) };
        if !matches!(self.analysis, Analysis::PlaneStress | Analysis::PlaneStrain) {
            return Err("bushings are for plane stress and plane strain analyses".into());
        }
        for (i, b) in self.bushings.iter().enumerate() {
            let n = i + 1;
            let Some(Shape::Circle { r, .. }) = holes.get(b.hole.wrapping_sub(1)) else { return Err(format!("bushing {n}: hole {} is not a circle", b.hole)) };
            if self.bushings.iter().take(i).any(|o| o.hole == b.hole) {
                return Err(format!("bushing {n}: hole {} already has a bushing", b.hole));
            }
            let (ri, off) = (0.5 * b.inner_diameter, b.offset[0].hypot(b.offset[1]));
            if !(b.inner_diameter.is_finite() && ri > 0.0 && ri + off < 0.97 * r) {
                return Err(format!("bushing {n}: the bore (radius {ri:.4} offset {off:.4}) must leave a wall inside the hole radius {r:.4}"));
            }
            if !(b.interference.is_finite() && b.interference > 0.0 && b.interference < 0.2 * r) {
                return Err(format!("bushing {n}: the diametral interference must be positive (and below 20 % of the hole radius)"));
            }
            if !(0.0..=2.0).contains(&b.friction) {
                return Err(format!("bushing {n}: friction must be in 0..2"));
            }
            b.material.elastic().validate().map_err(|e| format!("bushing {n}: {e}"))?;
        }
        Ok(())
    }

    /// Everything needed to solve: a meshable geometry and at least one support.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_geometry()?;
        if self.supports.is_empty() {
            return Err("add at least one support: the model is free to move".into());
        }
        Ok(())
    }
}

fn check_shape(s: &Shape) -> Result<(), String> {
    let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
    match s {
        Shape::Rect { x0, y0, x1, y1 } => {
            if !finite(&[*x0, *y0, *x1, *y1]) || x1 <= x0 || y1 <= y0 {
                return Err("a rectangle needs x1 > x0 and y1 > y0".into());
            }
        }
        Shape::Circle { cx, cy, r } => {
            if !finite(&[*cx, *cy, *r]) || *r <= 0.0 {
                return Err("a circle needs a positive radius".into());
            }
        }
        Shape::Slot { cx, cy, length, width, angle } => {
            if !finite(&[*cx, *cy, *length, *width, *angle]) || *width <= 0.0 || length <= width {
                return Err("a slot needs 0 < width < length".into());
            }
        }
        Shape::Polygon { pts } => {
            if pts.len() < 3 || pts.iter().any(|p| !finite(p)) {
                return Err("a polygon needs at least three finite points".into());
            }
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------- editing helpers
// Parameter access by index so an editor can list and change a shape, support or load without
// knowing its variants.

impl Shape {
    /// Editable numbers, in a fixed order. A polygon's points are edited through [`Shape::Polygon`] directly.
    pub fn params(&self) -> Vec<(&'static str, f64)> {
        match self {
            Shape::Rect { x0, y0, x1, y1 } => vec![("X min", *x0), ("Y min", *y0), ("X max", *x1), ("Y max", *y1)],
            Shape::Circle { cx, cy, r } => vec![("Centre X", *cx), ("Centre Y", *cy), ("Radius", *r)],
            Shape::Slot { cx, cy, length, width, angle } => vec![("Centre X", *cx), ("Centre Y", *cy), ("Length", *length), ("Width", *width), ("Angle (deg)", *angle)],
            Shape::Polygon { .. } => Vec::new(),
        }
    }

    pub fn set_param(&mut self, i: usize, v: f64) {
        let slot: Option<&mut f64> = match (self, i) {
            (Shape::Rect { x0, .. }, 0) => Some(x0),
            (Shape::Rect { y0, .. }, 1) => Some(y0),
            (Shape::Rect { x1, .. }, 2) => Some(x1),
            (Shape::Rect { y1, .. }, 3) => Some(y1),
            (Shape::Circle { cx, .. }, 0) | (Shape::Slot { cx, .. }, 0) => Some(cx),
            (Shape::Circle { cy, .. }, 1) | (Shape::Slot { cy, .. }, 1) => Some(cy),
            (Shape::Circle { r, .. }, 2) => Some(r),
            (Shape::Slot { length, .. }, 2) => Some(length),
            (Shape::Slot { width, .. }, 3) => Some(width),
            (Shape::Slot { angle, .. }, 4) => Some(angle),
            _ => None,
        };
        if let Some(s) = slot {
            *s = v;
        }
    }

    /// The next shape kind in `outer` (Rect, Circle, Slot, Polygon) or hole (Circle, Slot, Rect)
    /// order, keeping the footprint.
    pub fn next_kind(&self, outer: bool) -> Shape {
        let (lo, hi) = self.bounds();
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        let c = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
        let rect = Shape::Rect { x0: lo[0], y0: lo[1], x1: hi[0], y1: hi[1] };
        let circle = Shape::Circle { cx: c[0], cy: c[1], r: 0.5 * w.min(h) };
        let slot = Shape::Slot { cx: c[0], cy: c[1], length: w.max(h), width: 0.5 * w.min(h), angle: if h > w { 90.0 } else { 0.0 } };
        let poly = Shape::Polygon { pts: vec![[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]] };
        match (self, outer) {
            (Shape::Rect { .. }, true) => circle,
            (Shape::Circle { .. }, true) => slot,
            (Shape::Slot { .. }, true) => poly,
            (Shape::Polygon { .. }, true) => rect,
            (Shape::Circle { .. }, false) => slot,
            (Shape::Slot { .. }, false) => rect,
            (Shape::Rect { .. } | Shape::Polygon { .. }, false) => circle,
        }
    }
}

impl Support {
    /// Edge name, for an edge support.
    pub fn edge(&self) -> Option<&str> {
        match self {
            Support::Edge { edge, .. } => Some(edge),
            Support::Point { .. } => None,
        }
    }

    pub fn set_edge(&mut self, name: &str) {
        if let Support::Edge { edge, .. } = self {
            *edge = name.to_string();
        }
    }

    pub fn comps(&self) -> [Option<f64>; 3] {
        match self {
            Support::Edge { ux, uy, uz, .. } | Support::Point { ux, uy, uz, .. } => [*ux, *uy, *uz],
        }
    }

    pub fn set_comp(&mut self, c: usize, v: Option<f64>) {
        let (Support::Edge { ux, uy, uz, .. } | Support::Point { ux, uy, uz, .. }) = self;
        match c {
            0 => *ux = v,
            1 => *uy = v,
            _ => *uz = v,
        }
    }

    /// Point coordinates (empty for an edge support).
    pub fn coords(&self) -> Vec<(&'static str, f64)> {
        match self {
            Support::Point { x, y, z, .. } => vec![("X", *x), ("Y", *y), ("Z", *z)],
            Support::Edge { .. } => Vec::new(),
        }
    }

    pub fn set_coord(&mut self, i: usize, v: f64) {
        if let Support::Point { x, y, z, .. } = self {
            match i {
                0 => *x = v,
                1 => *y = v,
                _ => *z = v,
            }
        }
    }

    /// Toggle between an edge support and a point support.
    pub fn toggled(&self, first_edge: &str) -> Support {
        match self {
            Support::Edge { ux, uy, uz, .. } => Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: *ux, uy: *uy, uz: *uz },
            Support::Point { ux, uy, uz, .. } => Support::Edge { edge: first_edge.into(), ux: *ux, uy: *uy, uz: *uz },
        }
    }
}

impl Load {
    pub fn label(&self) -> &'static str {
        match self {
            Load::Pressure { .. } => "Pressure",
            Load::Traction { .. } => "Traction",
            Load::Force { .. } => "Edge force",
            Load::Bearing { .. } => "Bearing (pin) load",
            Load::Point { .. } => "Point force",
            Load::Body { .. } => "Body force",
        }
    }

    pub fn edge(&self) -> Option<&str> {
        match self {
            Load::Pressure { edge, .. } | Load::Traction { edge, .. } | Load::Force { edge, .. } => Some(edge),
            _ => None,
        }
    }

    pub fn set_edge(&mut self, name: &str) {
        if let Load::Pressure { edge, .. } | Load::Traction { edge, .. } | Load::Force { edge, .. } = self {
            *edge = name.to_string();
        }
    }

    /// Editable numbers; the `z` ones only when `dim == 3`.
    pub fn params(&self, dim: usize) -> Vec<(&'static str, f64)> {
        let mut v = match self {
            Load::Pressure { p, .. } => vec![("Pressure", *p)],
            Load::Traction { tx, ty, tz, .. } => vec![("Traction x", *tx), ("Traction y", *ty), ("Traction z", *tz)],
            Load::Force { fx, fy, fz, .. } => vec![("Force x", *fx), ("Force y", *fy), ("Force z", *fz)],
            Load::Bearing { hole, fx, fy } => vec![("Hole number", *hole as f64), ("Force x", *fx), ("Force y", *fy)],
            Load::Point { x, y, z, fx, fy, fz } => vec![("X", *x), ("Y", *y), ("Z", *z), ("Force x", *fx), ("Force y", *fy), ("Force z", *fz)],
            Load::Body { bx, by, bz } => vec![("Body x", *bx), ("Body y", *by), ("Body z", *bz)],
        };
        if dim == 2 {
            v.retain(|(l, _)| !l.ends_with(" z") && *l != "Z");
        }
        v
    }

    /// Set the `i`-th entry of [`Load::params`] for the same `dim`.
    pub fn set_param(&mut self, dim: usize, i: usize, value: f64) {
        let labels: Vec<&'static str> = self.params(dim).iter().map(|(l, _)| *l).collect();
        let Some(&label) = labels.get(i) else { return };
        let slot: Option<&mut f64> = match (self, label) {
            (Load::Pressure { p, .. }, _) => Some(p),
            (Load::Traction { tx, .. }, "Traction x") => Some(tx),
            (Load::Traction { ty, .. }, "Traction y") => Some(ty),
            (Load::Traction { tz, .. }, "Traction z") => Some(tz),
            (Load::Force { fx, .. }, "Force x") | (Load::Bearing { fx, .. }, "Force x") | (Load::Point { fx, .. }, "Force x") => Some(fx),
            (Load::Force { fy, .. }, "Force y") | (Load::Bearing { fy, .. }, "Force y") | (Load::Point { fy, .. }, "Force y") => Some(fy),
            (Load::Force { fz, .. }, "Force z") | (Load::Point { fz, .. }, "Force z") => Some(fz),
            (Load::Bearing { hole, .. }, "Hole number") => {
                *hole = value.round().max(1.0) as usize;
                None
            }
            (Load::Point { x, .. }, "X") => Some(x),
            (Load::Point { y, .. }, "Y") => Some(y),
            (Load::Point { z, .. }, "Z") => Some(z),
            (Load::Body { bx, .. }, "Body x") => Some(bx),
            (Load::Body { by, .. }, "Body y") => Some(by),
            (Load::Body { bz, .. }, "Body z") => Some(bz),
            _ => None,
        };
        if let Some(s) = slot {
            *s = value;
        }
    }

    /// The next load kind with sensible defaults (keeping the edge when both use one).
    pub fn next_kind(&self, first_edge: &str) -> Load {
        let edge = self.edge().unwrap_or(first_edge).to_string();
        match self {
            Load::Pressure { .. } => Load::Traction { edge, tx: 1000.0, ty: 0.0, tz: 0.0 },
            Load::Traction { .. } => Load::Force { edge, fx: 100.0, fy: 0.0, fz: 0.0 },
            Load::Force { .. } => Load::Bearing { hole: 1, fx: 1000.0, fy: 0.0 },
            Load::Bearing { .. } => Load::Point { x: 0.0, y: 0.0, z: 0.0, fx: 100.0, fy: 0.0, fz: 0.0 },
            Load::Point { .. } => Load::Body { bx: 0.0, by: -0.1, bz: 0.0 },
            Load::Body { .. } => Load::Pressure { edge, p: 1000.0 },
        }
    }
}
