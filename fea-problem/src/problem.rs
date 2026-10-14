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
    /// Elastic-plastic response: J2 plasticity with this yield stress and `hardening` (small strain, loads applied in steps).
    /// Off, `yield_stress` only feeds the margin readout.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plastic: bool,
    /// Tangent modulus after yield (stress per unit plastic strain); `0` is perfectly plastic.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hardening: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
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
    /// Closed chain of straight and circular-arc edges (a sketch outline with arcs), each edge one named surface
    /// (`edgeN` for an outer outline, the whole chain `holeN` for a hole). Arcs are exact: the mesher places nodes on them.
    Outline { segs: Vec<Seg> },
}

/// One edge of a [`Shape::Outline`]; each starts where the previous one ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Seg {
    Line { a: [f64; 2], b: [f64; 2] },
    /// Arc about `c` from angle `a0` to `a1` (radians); `a1 < a0` runs clockwise. At most one full turn.
    Arc { c: [f64; 2], r: f64, a0: f64, a1: f64 },
    /// Smooth curve through `pts` (at least two): a centripetal Catmull-Rom spline, C1 through every point, with no cusp
    /// or loop inside a span. It is meshed as a fine chain of straight pieces of one surface (the mesher has no spline).
    Spline { pts: Vec<[f64; 2]> },
}

/// Straight pieces per spline span when a spline is meshed or sampled.
const SPLINE_PIECES: usize = 16;

/// Point of the centripetal Catmull-Rom spline through `pts` at `u` in `[0, 1]` of span `span` (between `pts[span]` and
/// `pts[span + 1]`); the missing neighbours at the ends are reflections, which keeps the end tangents natural.
fn spline_at(pts: &[[f64; 2]], span: usize, u: f64) -> [f64; 2] {
    let n = pts.len();
    let at = |i: isize| -> [f64; 2] {
        if i < 0 {
            [2.0 * pts[0][0] - pts[1][0], 2.0 * pts[0][1] - pts[1][1]]
        } else if i as usize >= n {
            [2.0 * pts[n - 1][0] - pts[n - 2][0], 2.0 * pts[n - 1][1] - pts[n - 2][1]]
        } else {
            pts[i as usize]
        }
    };
    let (p0, p1, p2, p3) = (at(span as isize - 1), pts[span], pts[span + 1], at(span as isize + 2));
    // Centripetal knots t_{i+1} = t_i + |P_{i+1} - P_i|^0.5.
    let knot = |a: [f64; 2], b: [f64; 2]| ((b[0] - a[0]).hypot(b[1] - a[1])).sqrt().max(1e-12);
    let (t0, t1) = (0.0, knot(p0, p1));
    let (t2, t3) = (t1 + knot(p1, p2), t1 + knot(p1, p2) + knot(p2, p3));
    let t = t1 + u * (t2 - t1);
    let lerp = |a: [f64; 2], b: [f64; 2], ta: f64, tb: f64| -> [f64; 2] {
        let w = (t - ta) / (tb - ta);
        [(1.0 - w) * a[0] + w * b[0], (1.0 - w) * a[1] + w * b[1]]
    };
    let (a1, a2, a3) = (lerp(p0, p1, t0, t1), lerp(p1, p2, t1, t2), lerp(p2, p3, t2, t3));
    let (b1, b2) = (lerp(a1, a2, t0, t2), lerp(a2, a3, t1, t3));
    lerp(b1, b2, t1, t2)
}

impl Seg {
    /// Point at parameter `t` in `[0, 1]` along the edge (a spline: equal parameter per span).
    pub fn point(&self, t: f64) -> [f64; 2] {
        match self {
            Seg::Line { a, b } => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
            Seg::Arc { c, r, a0, a1 } => {
                let th = a0 + t * (a1 - a0);
                [c[0] + r * th.cos(), c[1] + r * th.sin()]
            }
            Seg::Spline { pts } => {
                let spans = pts.len() - 1;
                let x = t.clamp(0.0, 1.0) * spans as f64;
                let span = (x.floor() as usize).min(spans - 1);
                spline_at(pts, span, x - span as f64)
            }
        }
    }
    pub fn start(&self) -> [f64; 2] {
        self.point(0.0)
    }
    pub fn end(&self) -> [f64; 2] {
        self.point(1.0)
    }
    /// Points along the edge, `start` inclusive and `end` exclusive; an arc gets about `per_turn` per full turn.
    pub fn sample(&self, per_turn: f64) -> Vec<[f64; 2]> {
        match self {
            Seg::Line { a, .. } => vec![*a],
            Seg::Arc { a0, a1, .. } => {
                let n = (((a1 - a0).abs() / std::f64::consts::TAU * per_turn).ceil() as usize).max(2);
                (0..n).map(|i| self.point(i as f64 / n as f64)).collect()
            }
            Seg::Spline { pts } => {
                let n = (pts.len() - 1) * SPLINE_PIECES;
                (0..n).map(|i| self.point(i as f64 / n as f64)).collect()
            }
        }
    }
    /// The curve of the spline through `pts` as a polyline of `per_span` pieces per span, both ends included.
    pub fn spline_polyline(pts: &[[f64; 2]], per_span: usize) -> Vec<[f64; 2]> {
        if pts.len() < 2 {
            return pts.to_vec();
        }
        let per_span = per_span.max(1);
        let mut v: Vec<[f64; 2]> = Vec::with_capacity((pts.len() - 1) * per_span + 1);
        for span in 0..pts.len() - 1 {
            v.extend((0..per_span).map(|i| spline_at(pts, span, i as f64 / per_span as f64)));
        }
        v.push(pts[pts.len() - 1]);
        v
    }

    /// The same edge walked the other way.
    pub fn reversed(&self) -> Seg {
        match self {
            Seg::Line { a, b } => Seg::Line { a: *b, b: *a },
            Seg::Arc { c, r, a0, a1 } => Seg::Arc { c: *c, r: *r, a0: *a1, a1: *a0 },
            Seg::Spline { pts } => Seg::Spline { pts: pts.iter().rev().copied().collect() },
        }
    }
    /// The mesher's curves for this edge: one for a line or an arc, a chain of short lines for a spline.
    pub(crate) fn curves(&self) -> Vec<fea_core::geometry::Curve> {
        use fea_core::geometry::Curve;
        match self {
            Seg::Line { a, b } => vec![Curve::line(*a, *b)],
            Seg::Arc { c, r, a0, a1 } => vec![Curve::arc(*c, *r, *a0, *a1)],
            Seg::Spline { .. } => {
                let mut v = self.sample(0.0);
                v.push(self.end());
                v.windows(2).map(|w| Curve::line(w[0], w[1])).collect()
            }
        }
    }
}

impl Shape {
    pub fn label(&self) -> &'static str {
        match self {
            Shape::Rect { .. } => "Rectangle",
            Shape::Circle { .. } => "Circle",
            Shape::Slot { .. } => "Slot",
            Shape::Polygon { .. } => "Polygon",
            Shape::Outline { .. } => "Outline",
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
            Shape::Outline { segs } => {
                // Corners plus the arc extremes that fall inside each arc's sweep.
                let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                let mut grow = |p: [f64; 2]| {
                    for i in 0..2 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                };
                for sg in segs {
                    grow(sg.start());
                    grow(sg.end());
                    if let Seg::Spline { .. } = sg {
                        sg.sample(0.0).into_iter().for_each(&mut grow);
                    }
                    if let Seg::Arc { c, r, a0, a1 } = *sg {
                        let (from, to) = (a0.min(a1), a0.max(a1));
                        for k in 0..4 {
                            let q = k as f64 * std::f64::consts::FRAC_PI_2;
                            let n0 = ((from - q) / std::f64::consts::TAU).ceil();
                            if q + n0 * std::f64::consts::TAU <= to {
                                grow([c[0] + r * q.cos(), c[1] + r * q.sin()]);
                            }
                        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MeshStrategy {
    #[default]
    Unstructured,
    Hybrid,
}

impl MeshStrategy {
    pub const ALL: [MeshStrategy; 2] = [MeshStrategy::Unstructured, MeshStrategy::Hybrid];

    pub fn label(self) -> &'static str {
        match self {
            MeshStrategy::Unstructured => "Unstructured",
            MeshStrategy::Hybrid => "Hybrid (structured + unstructured)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AdaptMode {
    #[default]
    Zz,
    /// The element error comes from an equilibrated (H(div)) stress reconstruction instead of the ZZ recovery: a statically
    /// admissible stress field, so the estimate is an energy-norm measure that does not rely on superconvergence. Triangle
    /// meshes of plane problems only (`Unstructured` strategy, Tri3 or Tri6).
    Equilibrated,
    GoalOriented,
}

impl AdaptMode {
    pub const ALL: [AdaptMode; 3] = [AdaptMode::Zz, AdaptMode::Equilibrated, AdaptMode::GoalOriented];

    pub fn label(self) -> &'static str {
        match self {
            AdaptMode::Zz => "ZZ error estimate",
            AdaptMode::Equilibrated => "Equilibrated stress (triangles)",
            AdaptMode::GoalOriented => "Goal-oriented (compliance)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshSpec {
    /// How a sketch region is meshed: all-unstructured (default) or hybrid structured blocks with unstructured cells around the
    /// features (a mixed Quad/Tri mesh, with the unstructured mesh as the explicit fallback when the region is unsuitable).
    #[serde(default)]
    pub strategy: MeshStrategy,
    /// What drives the adaptive passes: the ZZ error estimate (default) or goal-oriented adaptivity on the compliance (adjoint
    /// error estimate fused with the ZZ physics criterion, Dorfler marking; structural 2D sketches without bushings).
    #[serde(default)]
    pub adapt_mode: AdaptMode,
    /// Join the sketch's regions along the boundaries they share (one welded, conforming mesh with each region's own
    /// material); otherwise every region is its own body. Needs matching boundaries: the mesh refuses a mismatch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bond_regions: bool,
    /// Predict where the mesh must be fine before any solution exists (`meshing::predictor`): curved boundaries, re-entrant
    /// corners and point loads / restraints get a finer size that grows back to `size` with distance. Never coarser than the
    /// size field without it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub predict_sizes: bool,
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
    /// Extra refinement zones in the sketch plane (also kept through adaptive passes); 2D profile meshes only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refine: Vec<RefineRegion>,
    /// Imported meshes: turn elements with the opposite node orientation right side out when the mesh is read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reorient: bool,
}

impl Default for MeshSpec {
    fn default() -> Self {
        Self { strategy: MeshStrategy::default(), adapt_mode: AdaptMode::default(), bond_regions: false, predict_sizes: false, element: ElementChoice::Quad8, size: 0.25, hole_factor: 0.35, adapt_passes: 2, target_error: 0.02, refine: Vec::new(), reorient: false }
    }
}

/// Most refinement zones a problem may carry.
pub const MAX_REFINE: usize = 16;
/// Finest allowed refinement level.
pub const MAX_LEVEL: u8 = 3;

/// Element size multiplier of refinement `level`: 1 halves the size, 2 quarters it, 3 divides it by eight.
pub fn level_factor(level: u8) -> f64 {
    0.5f64.powi(i32::from(level))
}

/// A zone where the element size is `size * level_factor(level)` within `radius` of `centre` (or of the segment
/// `centre`-`to`, a band along an edge), blending back to `size` over `blend` more (default: one more `radius`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefineRegion {
    pub centre: [f64; 2],
    pub radius: f64,
    pub level: u8,
    /// Other end of a band; absent for a round zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<[f64; 2]>,
    /// Width of the blend back to the base size; absent means `radius`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<f64>,
}

impl RefineRegion {
    pub fn circle(centre: [f64; 2], radius: f64, level: u8) -> Self {
        Self { centre, radius, level, to: None, blend: None }
    }
    /// Distance from `x` to the zone's centre (a point, or the segment of a band).
    pub fn distance(&self, x: [f64; 2]) -> f64 {
        let Some(b) = self.to else { return (x[0] - self.centre[0]).hypot(x[1] - self.centre[1]) };
        let a = self.centre;
        let (ab, ax) = ([b[0] - a[0], b[1] - a[1]], [x[0] - a[0], x[1] - a[1]]);
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if len2 > 0.0 { ((ax[0] * ab[0] + ax[1] * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
        (x[0] - a[0] - t * ab[0]).hypot(x[1] - a[1] - t * ab[1])
    }
    pub fn blend_width(&self) -> f64 {
        self.blend.unwrap_or(self.radius)
    }
    /// Element size multiplier at `x`: the level's factor inside `radius`, rising to 1 over the blend.
    pub fn factor(&self, x: [f64; 2]) -> f64 {
        let f = level_factor(self.level);
        let t = ((self.distance(x) - self.radius) / self.blend_width()).clamp(0.0, 1.0);
        f + (1.0 - f) * t
    }
}

/// One more region of a sketch: an outline and its holes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchRegion {
    pub outer: Shape,
    #[serde(default)]
    pub holes: Vec<Shape>,
}

/// Where the geometry comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    /// A 2D sketch (outer boundary minus holes); extruded when the analysis is [`Analysis::Solid`]. `regions` are the
    /// sketch's other disjoint regions (a separate outline, an island inside a hole): meshed on their own and appended as
    /// separate bodies whose surfaces are named `r2/...`, `r3/...` (the first region has no prefix).
    Sketch {
        outer: Shape,
        holes: Vec<Shape>,
        depth: f64,
        layers: usize,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        regions: Vec<SketchRegion>,
    },
    /// A mesh read from a Gmsh `.msh` or Abaqus `.inp` file (the app supplies its text).
    Imported { path: String },
    /// A B-rep solid: its STEP text (the source of truth, edited with the OpenCascade kernel) and the cached face-tagged
    /// surface mesh of it (all a solve needs, so a build without the kernel still solves). The cache is valid only while
    /// its hash matches `step`; faces are named `face1`.. in the cache's face order, like sketch edges.
    Solid {
        step: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<crate::solid::SolidCache>,
    },
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
    /// Materials of the sketch's other regions (the second onwards, in region order); a missing or empty entry is `material`.
    /// Regions are separate bodies unless `mesh.bond_regions` joins them along their shared boundaries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub region_materials: Vec<Option<MaterialSpec>>,
    pub geometry: Geometry,
    pub mesh: MeshSpec,
    pub supports: Vec<Support>,
    pub loads: Vec<Load>,
    /// Load cases (see `cases`): empty means every support and load acts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<crate::cases::LoadCase>,
    /// Index of the case a solve uses.
    #[serde(default, skip_serializing_if = "is_zero_index")]
    pub active_case: usize,
    /// Uniform temperature change from the stress-free state.
    #[serde(default)]
    pub delta_t: f64,
    /// Interference-fit bushings in circular holes (plane analyses only).
    #[serde(default)]
    pub bushings: Vec<Bushing>,
    /// What a solve computes: stresses, a steady temperature field, or the thermal stress of that field.
    #[serde(default)]
    pub study: crate::thermal::Study,
    /// Conductivity, source and thermal boundary conditions (used by the thermal studies).
    #[serde(default, skip_serializing_if = "crate::thermal::ThermalSpec::is_default")]
    pub thermal: crate::thermal::ThermalSpec,
    /// Parametric feature history the geometry was built from (sketch on a plane, extrude / revolve). Absent for
    /// plain `Geometry` problems and for files written before the history existed; `apply_history` rewrites
    /// `geometry` (and `analysis` for a solid) from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<crate::history::History>,
}

fn is_zero_index(n: &usize) -> bool {
    *n == 0
}

fn version() -> u32 {
    FORMAT_VERSION
}

impl Problem {
    /// Degrees of freedom per node of the analysis (a plane or axisymmetric problem has 2, a solid 3).
    pub fn analysis_dim(&self) -> usize {
        if self.analysis == Analysis::Solid { 3 } else { 2 }
    }

    /// Material of region `k` (0 = the outline and holes of `geometry`, 1.. = its `regions`).
    pub fn region_material(&self, k: usize) -> &MaterialSpec {
        k.checked_sub(1).and_then(|i| self.region_materials.get(i)).and_then(Option::as_ref).unwrap_or(&self.material)
    }

    /// Whether any region has a material of its own.
    pub fn has_region_materials(&self) -> bool {
        self.region_materials.iter().any(|m| m.as_ref().is_some_and(|m| *m != self.material))
    }

    /// Rebuild `geometry` (and `analysis`, for a solid feature) from the feature history. Supports and loads are
    /// untouched: they refer to names (`edgeN`, `holeN`, `start`, `end`) that the rebuild reproduces.
    pub fn apply_history(&mut self) -> Result<(), String> {
        use crate::history::Body;
        let Some(h) = &self.history else { return Ok(()) };
        let b = h.rebuild()?;
        let (depth, layers) = match b.body {
            // A plain sketch keeps the extrusion settings the problem already had (they matter once it is extruded).
            Body::Region => match &self.geometry {
                Geometry::Sketch { depth, layers, .. } => (*depth, *layers),
                Geometry::Imported { .. } | Geometry::Solid { .. } => (1.0, 1),
            },
            Body::Extrude { depth, layers } => (depth, layers),
            Body::Revolve { segments, .. } => (1.0, segments),
            Body::Sweep { layers, .. } => (1.0, layers),
        };
        if b.body != Body::Region {
            self.analysis = Analysis::Solid;
            if !self.mesh.element.is_quad() {
                self.mesh.element = ElementChoice::Quad4;
            }
        }
        self.geometry = Geometry::Sketch { outer: b.outer, holes: b.holes, depth, layers, regions: b.regions };
        Ok(())
    }

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
        if let Geometry::Solid { cache: Some(c), .. } = &self.geometry {
            return c.face_names();
        }
        let Geometry::Sketch { outer, holes, depth, regions, .. } = &self.geometry else { return Vec::new() };
        let region_names = |outer: &Shape, holes: &[Shape], prefix: &str| -> Vec<String> {
            let mut names: Vec<String> = match outer {
                Shape::Rect { .. } => ["bottom", "right", "top", "left"].iter().map(|s| s.to_string()).collect(),
                Shape::Polygon { pts } => (1..=pts.len()).map(|i| format!("edge{i}")).collect(),
                Shape::Outline { segs } => (1..=segs.len()).map(|i| format!("edge{i}")).collect(),
                Shape::Circle { .. } | Shape::Slot { .. } => vec!["outer".into()],
            };
            names.extend((1..=holes.len()).map(|i| format!("hole{i}")));
            names.into_iter().map(|n| format!("{prefix}{n}")).collect()
        };
        let mut names = region_names(outer, holes, "");
        for (k, r) in regions.iter().enumerate() {
            names.extend(region_names(&r.outer, &r.holes, &format!("r{}/", k + 2)));
        }
        if self.analysis == Analysis::Solid && *depth > 0.0 {
            names.push("start".into());
            names.push("end".into());
        }
        names
    }

    /// Remove all geometry: an empty feature history, no boundary conditions (they named surfaces that are gone) and
    /// a unit placeholder outline that is never meshed (`validate_geometry` refuses until a sketch has a profile).
    pub fn clear_geometry(&mut self) {
        self.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, holes: Vec::new(), depth: 1.0, layers: 4, regions: Vec::new() };
        self.history = Some(crate::history::History::default());
        self.supports.clear();
        self.loads.clear();
        self.bushings.clear();
    }

    /// Everything needed to mesh the problem (no supports or loads yet).
    pub fn validate_geometry(&self) -> Result<(), String> {
        if let Some(h) = &self.history {
            if h.features.is_empty() {
                return Err("the model has no geometry: add a sketch".into());
            }
            h.rebuild().map_err(|e| format!("the feature history has no valid body: {e}"))?;
        }
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
        if self.mesh.refine.len() > MAX_REFINE {
            return Err(format!("at most {MAX_REFINE} refinement zones"));
        }
        for r in &self.mesh.refine {
            let ok = r.centre.iter().all(|c| c.is_finite())
                && r.to.is_none_or(|t| t.iter().all(|c| c.is_finite()))
                && r.radius.is_finite()
                && r.radius > 0.0
                && r.blend.is_none_or(|b| b.is_finite() && b > 0.0)
                && (1..=MAX_LEVEL).contains(&r.level);
            if !ok {
                return Err(format!("a refinement zone needs a finite centre, a positive radius and blend, and a level 1 to {MAX_LEVEL}"));
            }
        }
        match &self.geometry {
            Geometry::Sketch { outer, holes, depth, layers, regions } => {
                if self.analysis == Analysis::Solid {
                    if !self.mesh.element.is_quad() {
                        return Err("a 3D extrusion needs quadrilateral elements (Quad4 / Quad8 / Quad9)".into());
                    }
                    if !(depth.is_finite() && *depth > 0.0) || *layers == 0 {
                        return Err("an extrusion needs a positive depth and at least one layer".into());
                    }
                }
                for s in std::iter::once(outer).chain(holes.iter()).chain(regions.iter().flat_map(|r| std::iter::once(&r.outer).chain(r.holes.iter()))) {
                    check_shape(s)?;
                }
                if self.region_materials.len() > regions.len() {
                    return Err(format!("{} region materials for {} extra regions", self.region_materials.len(), regions.len()));
                }
                for (i, m) in self.region_materials.iter().enumerate() {
                    if let Some(m) = m {
                        m.elastic().validate().map_err(|e| format!("region {} material: {e}", i + 2))?;
                    }
                }
                if !regions.is_empty() && !self.bushings.is_empty() {
                    return Err("a bushing is placed in a hole of the first region; a sketch with several regions cannot have bushings".into());
                }
            }
            Geometry::Imported { path } => {
                if path.trim().is_empty() {
                    return Err("no mesh file chosen".into());
                }
            }
            Geometry::Solid { step, cache } => {
                if self.analysis != Analysis::Solid {
                    return Err("a B-rep solid is analysed as a 3D solid: set the analysis to 3D solid".into());
                }
                if step.trim().is_empty() {
                    return Err("the solid has no STEP text".into());
                }
                match cache {
                    None => return Err("the solid has not been tessellated yet: open it with the OpenCascade build to cache its surface mesh".into()),
                    Some(c) if !c.is_current(step) => return Err("the solid's cached surface mesh is out of date (the STEP text changed): re-tessellate it with the OpenCascade build".into()),
                    Some(_) => {}
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

    /// The elastic-plastic request is consistent: a yield stress, a plain structural study, and no features that need other solvers.
    fn validate_plasticity(&self) -> Result<(), String> {
        if !self.material.plastic {
            return Ok(());
        }
        match self.material.yield_stress {
            Some(y) if y.is_finite() && y > 0.0 => {}
            _ => return Err("an elastic-plastic material needs a positive yield stress".into()),
        }
        if !(self.material.hardening.is_finite() && self.material.hardening >= 0.0) {
            return Err("the hardening modulus must be zero or positive".into());
        }
        if !self.bushings.is_empty() {
            return Err("an elastic-plastic material does not combine with bushings (the contact fit has its own solver)".into());
        }
        if self.study != crate::thermal::Study::Structural {
            return Err("an elastic-plastic material is for the structural study (not thermal)".into());
        }
        if self.has_region_materials() {
            return Err("an elastic-plastic material applies to one material; regions with materials of their own are not supported with it".into());
        }
        Ok(())
    }

    /// Everything needed to solve: a meshable geometry and at least one support.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_geometry()?;
        self.validate_plasticity()?;
        if self.study.is_thermal() {
            self.thermal.validate()?;
            if self.thermal.supports.is_empty() && !self.thermal.loads.iter().any(|l| matches!(l, crate::thermal::HeatLoad::Convection { .. })) {
                return Err("add a fixed temperature or a convection surface: without one the temperature level is undetermined".into());
            }
        }
        if self.study.is_structural() && self.supports.is_empty() {
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
        Shape::Outline { segs } => {
            let ok = |sg: &Seg| match sg {
                Seg::Line { a, b } => finite(a) && finite(b),
                Seg::Arc { c, r, a0, a1 } => finite(c) && finite(&[*r, *a0, *a1]) && *r > 0.0 && (a1 - a0).abs() <= std::f64::consts::TAU + 1e-12,
                Seg::Spline { pts } => pts.len() >= 2 && pts.iter().all(|p| finite(p)),
            };
            if segs.len() < 2 || !segs.iter().all(ok) {
                return Err("an outline needs at least two finite edges and arcs with a positive radius and at most a full turn".into());
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
            Shape::Polygon { .. } | Shape::Outline { .. } => Vec::new(),
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
            (Shape::Polygon { .. } | Shape::Outline { .. }, true) => rect,
            (Shape::Circle { .. }, false) => slot,
            (Shape::Slot { .. }, false) => rect,
            (Shape::Rect { .. } | Shape::Polygon { .. } | Shape::Outline { .. }, false) => circle,
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
