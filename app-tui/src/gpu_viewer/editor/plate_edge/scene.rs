//! The viewport of the Plate Edge window: the plate as the FE meshes it (graded about the hole, half of the plate by symmetry
//! about the load line) with the pin, the free edge, the shear-out planes and the edge distance drawn on it, then the deformed
//! model of the last solve coloured by equivalent plastic strain.
//!
//! Both views are framed on the same region, the hole, the ligament and the free edge (`Frame`), so the picture does not jump
//! when a result replaces it; the plate beyond the region continues past the viewport.
use super::inputs::PlateEdgeInputs;
use super::run::{FeLine, Outcome};
use crate::gpu_viewer::{scene::SceneSource, Scene};
use fea_core::{sweep::extrude, Mesh};
use lug_solver::PlaneMode;
use std::sync::Arc;

pub const KEY_LABELS: [&str; 4] = ["Plate", "Hole wall", "Pin", "Free edge, planes"];
/// The colour key's codes of what the picture draws (`KEY_LABELS` in order).
const PLATE: f32 = 0.0;
const HOLE: f32 = 1.0;
const PIN: f32 = 2.0;
const EDGE: f32 = 3.0;

/// How much of the viewport's unit span the framed region fills (the docks and cards cover the rest).
const FILL: f64 = 0.75;
/// Annotations sit this far in front of the plate (scene units), and their lines are this wide.
const LIFT: f32 = 0.0006;
const LINE: f64 = 0.006;

/// The FE line the picture shows: the 3D solid if it ran, else plane strain.
pub fn pick_fields(o: &Outcome) -> Option<&FeLine> {
    ["3D solid", "2D plane strain", "2D plane stress"].iter().find_map(|l| o.lines.iter().find(|x| x.label == *l && x.fields.is_some()))
}

/// The region both views are framed on, in the model's inches (hole centre at the origin, the free edge at `x = -e`, the load
/// line along `y = 0`): the free edge and a little beyond, two hole diameters past the hole, and room under the load line for
/// the edge dimension.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub d: f64,
    pub e: f64,
    centre: [f64; 2],
    span: f64,
}

impl Frame {
    pub fn of(inputs: &PlateEdgeInputs) -> Self {
        let (d, e) = (inputs.hole_diameter(), inputs.edge);
        let (x0, x1, y0, y1) = (-(e + 0.3 * d), 2.2 * d, -0.8 * d, 2.2 * d);
        Self { d, e, centre: [0.5 * (x0 + x1), 0.5 * (y0 + y1)], span: (x1 - x0).max(y1 - y0) }
    }
    /// A model point as a scene point (the scene's unit span is the framed region at `FILL`).
    pub fn xy(&self, x: f64, y: f64) -> [f32; 2] {
        [((x - self.centre[0]) / self.span * FILL) as f32, ((y - self.centre[1]) / self.span * FILL) as f32]
    }
    pub fn point(&self, x: f64, y: f64) -> [f32; 3] {
        let [a, b] = self.xy(x, y);
        [a, b, 0.0]
    }
    /// Scene units to model units.
    fn len(&self, scene_len: f64) -> f64 {
        scene_len * self.span / FILL
    }
    /// The pin as drawn: its radius (never more than the hole's) and its centre on the load line, resting on the loaded side.
    pub fn pin(&self, pin_dia: f64) -> (f64, f64) {
        let r = 0.5 * self.d;
        let rp = (0.5 * pin_dia).min(0.999 * r);
        (rp, -(r - rp))
    }
}

/// Map a scene built from `mesh` (normalised on its own bounding box by `Scene::from_frames`) onto the framed region.
fn reframe(scene: &mut Scene, mesh: &Mesh, f: &Frame) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &mesh.nodes {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let span = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
    let mid: [f64; 3] = std::array::from_fn(|a| 0.5 * (lo[a] + hi[a]));
    let k = span * FILL / f.span;
    let off = [(mid[0] - f.centre[0]) * FILL / f.span, (mid[1] - f.centre[1]) * FILL / f.span, mid[2] * FILL / f.span];
    for p in &mut scene.positions {
        *p = std::array::from_fn(|a| (f64::from(p[a]) * k + off[a]) as f32);
    }
    for s in &mut scene.samples {
        for a in 0..3 {
            s[a] = (f64::from(s[a]) * k) as f32;
        }
    }
    // Deformation drawn to scale (the scene's automatic gain exaggerates it several times, which made the hole look larger
    // than the picture's); the gain keys still scale it.
    scene.gain = 1.0;
}

pub fn build(inputs: &PlateEdgeInputs, name: &str, outcome: Option<&Outcome>) -> Result<Scene, String> {
    if let Some(line) = outcome.and_then(pick_fields) {
        let f = line.fields.as_ref().expect("picked for its fields");
        let last = line.last();
        let label = format!("{name} | {} | {} elements around, {} dof | pin load at 0.1 D travel {:.0} lbf | deformation to scale", line.label, last.around, last.dofs, last.collapse);
        let mut scene = SceneSource { mesh: Arc::new(f.mesh.clone()), displacement: f.u.clone(), values: f.plastic.clone(), period: None, label, field_label: "Equivalent plastic strain".into(), fields: Vec::new() }.build()?;
        reframe(&mut scene, &f.mesh, &Frame::of(inputs));
        return Ok(scene);
    }
    picture(inputs, name)
}

/// The plate before any solve: the hole wall in one colour, the pin, the free edge, the shear-out planes and the edge distance.
fn picture(inputs: &PlateEdgeInputs, name: &str) -> Result<Scene, String> {
    let case = inputs.picture_case()?;
    let fe = case.lug(PlaneMode::Strain);
    let half_t = 0.5 * case.thickness();
    let mesh = extrude(&fe.model.mesh, &[0.0, half_t])?;
    let r = 0.5 * case.dia;
    let values: Vec<f64> = mesh.nodes.iter().map(|p| if p[0].hypot(p[1]) < r * (1.0 + 1e-6) { f64::from(HOLE) } else { f64::from(PLATE) }).collect();
    let n = mesh.nodes.len();
    let label = format!("{name} | half model, symmetric about the load line, hole {:.4} in, edge {:.3} in", case.dia, inputs.edge);
    let mut scene = SceneSource { mesh: Arc::new(mesh.clone()), displacement: vec![0.0; 3 * n], values, period: None, label, field_label: "part (see color key)".into(), fields: Vec::new() }.build()?;
    // The geometry view reads the values as the colour key's codes 0..3 (`KEY_LABELS`), not as a contour.
    scene.range = [0.0, 3.0];
    let frame = Frame::of(inputs);
    reframe(&mut scene, &mesh, &frame);
    draw_annotations(&mut scene, &frame, inputs);
    Ok(scene)
}

/// Flat shapes in front of the plate, appended to the scene (one frame: a sample per vertex).
struct Ink<'a> {
    scene: &'a mut Scene,
    f: &'a Frame,
}

impl Ink<'_> {
    fn vertex(&mut self, p: [f64; 2], code: f32) -> u32 {
        let [x, y] = self.f.xy(p[0], p[1]);
        self.scene.positions.push([x, y, -LIFT]);
        self.scene.samples.push([0.0, 0.0, 0.0, code]);
        (self.scene.positions.len() - 1) as u32
    }
    fn triangle(&mut self, a: [f64; 2], b: [f64; 2], c: [f64; 2], code: f32) {
        let ids = [self.vertex(a, code), self.vertex(b, code), self.vertex(c, code)];
        self.scene.triangles.extend(ids);
    }
    /// A line of constant on-screen width from `a` to `b`.
    fn bar(&mut self, a: [f64; 2], b: [f64; 2], code: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        if l <= 0.0 {
            return;
        }
        let w = 0.5 * self.f.len(LINE);
        let n = [-dy / l * w, dx / l * w];
        let (p, q, r, s) = ([a[0] + n[0], a[1] + n[1]], [a[0] - n[0], a[1] - n[1]], [b[0] - n[0], b[1] - n[1]], [b[0] + n[0], b[1] + n[1]]);
        self.triangle(p, q, r, code);
        self.triangle(p, r, s, code);
    }
    /// A dashed line.
    fn dashes(&mut self, a: [f64; 2], b: [f64; 2], dash: f64, gap: f64, code: f32) {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        let at = |t: f64| [a[0] + (b[0] - a[0]) * t / l, a[1] + (b[1] - a[1]) * t / l];
        let mut t = 0.0;
        while t < l {
            self.bar(at(t), at((t + dash).min(l)), code);
            t += dash + gap;
        }
    }
    /// The half of a disc above the load line.
    fn half_disc(&mut self, c: [f64; 2], r: f64, code: f32) {
        const N: usize = 48;
        let at = |k: usize| {
            let a = std::f64::consts::PI * k as f64 / N as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        };
        for k in 0..N {
            self.triangle(c, at(k), at(k + 1), code);
        }
    }
    /// An arrowhead at `tip` pointing along `dir` (unit), `len` long.
    fn arrow_head(&mut self, tip: [f64; 2], dir: [f64; 2], len: f64, code: f32) {
        let side = [-dir[1], dir[0]];
        let base = [tip[0] - dir[0] * len, tip[1] - dir[1] * len];
        self.triangle(tip, [base[0] + side[0] * 0.3 * len, base[1] + side[1] * 0.3 * len], [base[0] - side[0] * 0.3 * len, base[1] - side[1] * 0.3 * len], code);
    }
}

/// The pin, the free edge, the shear-out planes (the tangents to the hole toward the edge) and the edge distance `e`, measured
/// below the load line.
fn draw_annotations(scene: &mut Scene, f: &Frame, inputs: &PlateEdgeInputs) {
    let (d, e) = (f.d, f.e);
    let r = 0.5 * d;
    let (rp, cx) = f.pin(inputs.pin_diameter());
    let top = 2.2 * d;
    let mut ink = Ink { scene, f };
    ink.half_disc([cx, 0.0], rp, PIN);
    ink.bar([-e, 0.0], [-e, top], EDGE);
    ink.dashes([-e, r], [0.0, r], 0.3 * d, 0.2 * d, EDGE);
    let y = dimension_y(d);
    ink.bar([-e, y], [0.0, y], EDGE);
    for x in [-e, 0.0] {
        ink.bar([x, y - 0.15 * d], [x, y + 0.15 * d], EDGE);
    }
    let head = (0.3 * d).min(0.4 * e);
    ink.arrow_head([-e, y], [-1.0, 0.0], head, EDGE);
    ink.arrow_head([0.0, y], [1.0, 0.0], head, EDGE);
}

/// Where the edge dimension runs: under the load line.
pub fn dimension_y(d: f64) -> f64 {
    -0.45 * d
}
