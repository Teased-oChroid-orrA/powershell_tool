//! Sketch tools of the solid editor: pick a tool in the ribbon or Insert menu, then click the model.
//! The Hole tool places a circular hole where the cursor meets the part's front face.
use super::*;
use fea_problem::Shape;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SolidTool {
    #[default]
    Select,
    Hole,
}
impl SolidTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select surface",
            Self::Hole => "Hole (click the part)",
        }
    }
}

/// Round `v` to a step of about 1/100 of `span`, a power of ten times 1, 2 or 5.
pub(super) fn nice_step(span: f64) -> f64 {
    let raw = span / 100.0;
    if !(raw.is_finite() && raw > 0.0) {
        return 0.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(raw)
}

impl SolidSession {
    pub fn hole_availability(&self) -> Result<(), String> {
        if self.problem.history.is_some() {
            return Err("this model is built from a feature history: draw the hole in its sketch".into());
        }
        match &self.problem.geometry {
            Geometry::Sketch { .. } => Ok(()),
            Geometry::Imported { .. } => Err("holes are sketched; an imported mesh has none to add".into()),
            Geometry::Solid { .. } => Err("holes are cut from a solid with a boolean in the feature history; a stored STEP solid has no sketch".into()),
        }
    }
    pub fn set_tool(&mut self, tool: SolidTool) {
        if tool == SolidTool::Hole {
            if let Err(e) = self.hole_availability() {
                self.status = e;
                return;
            }
        }
        self.tool = tool;
        self.status = match tool {
            SolidTool::Hole => "Hole: click the part where the hole's centre goes (Esc stops).".into(),
            SolidTool::Select => String::new(),
        };
        self.touch();
    }
    /// Model-space point under the pixel on the part's front face (`z` = top of the extrusion, or 0 for a 2D model).
    fn front_face_point(&self, cam: &Camera, px: [f32; 2], size: [f32; 2]) -> Option<[f64; 2]> {
        let mesh = self.shown_mesh()?;
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in &mesh.nodes {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let span = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        if !(span.is_finite() && span > 0.0) {
            return None;
        }
        let mid: [f64; 3] = std::array::from_fn(|a| lo[a] + (hi[a] - lo[a]) / 2.0);
        let ndc = Camera::ndc(px[0] as f64, px[1] as f64, size[0], size[1]);
        let plane = ((hi[2] - mid[2]) / span) as f32;
        let q = cam.unproject_to_plane(ndc, 2, plane)?;
        let step = nice_step(span);
        let snap = |v: f64| if step > 0.0 { (v / step).round() * step + 0.0 } else { v };
        Some([snap(mid[0] + q[0] as f64 * span), snap(mid[1] + q[1] as f64 * span)])
    }
    /// Hole tool click: add a hole centred at the pixel with the default radius.
    pub(super) fn place_hole(&mut self, cam: &Camera, px: [f32; 2], size: [f32; 2]) {
        let Some([cx, cy]) = self.front_face_point(cam, px, size) else {
            self.status = "wait for the mesh preview, or turn the view so the part's front face is visible".into();
            return;
        };
        let Geometry::Sketch { outer, holes, .. } = &self.problem.geometry else { return };
        let Shape::Circle { r, .. } = model::new_hole(&self.problem) else { return };
        let (lo, hi) = outer.bounds();
        if cx - r < lo[0] || cx + r > hi[0] || cy - r < lo[1] || cy + r > hi[1] {
            self.status = "the hole would leave the part's outline: click nearer the middle".into();
            return;
        }
        let index = holes.len();
        self.edit(|p| {
            if let Geometry::Sketch { holes, .. } = &mut p.geometry {
                holes.push(Shape::Circle { cx, cy, r });
            }
        });
        self.do_action(SolidAction::FocusFeature(Feature::Hole(index)));
        self.status = format!("Hole {} at ({cx:.4}, {cy:.4}), radius {r:.4}: edit its size in the property panel.", index + 1);
    }
}
