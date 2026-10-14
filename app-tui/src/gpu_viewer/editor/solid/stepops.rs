//! STEP solids in the editor: opening a `.step` file and the hole / boss / fillet operations on a stored solid
//! (`fea_problem::solid`). All of it needs the OpenCascade build (cargo feature `occt`); a build without it says so.
use super::*;
use fea_problem::solid::SolidOp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptOp {
    Fillet,
    Hole,
    Boss,
}

impl PromptOp {
    fn label(self) -> &'static str {
        match self {
            Self::Fillet => "fillet radius",
            Self::Hole => "hole diameter",
            Self::Boss => "boss: diameter and height (two numbers)",
        }
    }
}

/// `faceN` -> N.
fn face_number(name: &str) -> Option<usize> {
    name.strip_prefix("face")?.parse().ok()
}

impl SolidSession {
    pub(super) fn open_step(&mut self, path: &str) {
        let name = Path::new(path).file_stem().map_or("STEP solid".to_string(), |s| s.to_string_lossy().into_owned());
        match std::fs::read_to_string(path) {
            Ok(text) => match Problem::from_step(&name, &text) {
                Ok(p) => {
                    let faces = p.edge_names().len();
                    self.start_model(p);
                    self.status = format!("Opened {path}: {faces} faces. Pick a face for a support or load, then Generate mesh.");
                }
                Err(e) => self.status = format!("Could not open {path}: {e}"),
            },
            Err(e) => self.status = format!("Could not read {path}: {e}"),
        }
    }

    pub(super) fn solid_op_availability(&self, op: PromptOp) -> Result<(), String> {
        if !cfg!(feature = "occt") {
            return Err("needs the OpenCascade build (cargo build --features occt)".into());
        }
        self.replace_availability()?;
        match &self.problem.geometry {
            Geometry::Solid { step, cache: Some(c) } if c.is_current(step) => {}
            Geometry::Solid { .. } => return Err("the solid's surface mesh is out of date".into()),
            _ => return Err("this model is not a STEP solid (File > Open a .step file)".into()),
        }
        if op != PromptOp::Fillet && self.selected.as_deref().and_then(face_number).is_none() {
            return Err("pick a flat face first".into());
        }
        Ok(())
    }

    pub(super) fn begin_solid_op(&mut self, op: PromptOp) {
        if let Err(e) = self.solid_op_availability(op) {
            self.status = e;
            return;
        }
        self.prompt = Some(Prompt { kind: EditKind::Text, label: op.label().into(), text: String::new(), row: None, open: false, fresh: false, op: Some(op) });
        self.status = "Type the size and press Enter (Esc cancels).".into();
        self.touch();
    }

    pub(super) fn run_prompt_op(&mut self, op: PromptOp, text: &str) {
        let nums: Vec<f64> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
        let face = self.selected.as_deref().and_then(face_number).unwrap_or(0);
        let solid_op = match (op, nums.as_slice()) {
            (PromptOp::Fillet, [r]) => SolidOp::Fillet { radius: *r },
            (PromptOp::Hole, [d]) => SolidOp::Hole { face, diameter: *d },
            (PromptOp::Boss, [d, h]) => SolidOp::Boss { face, diameter: *d, height: *h },
            _ => {
                self.status = format!("Expected {}.", op.label());
                return;
            }
        };
        let mut next = self.problem.clone();
        match next.apply_solid_op(&solid_op) {
            Ok(()) => {
                let faces = next.edge_names().len();
                // Supports and loads follow their faces (`fea_problem::solid::remap_faces`); one whose face the edit removed is
                // renamed `removed:faceN` and shows up as a finding.
                let lost = next.supports.iter().filter_map(|s| s.edge()).map(str::to_string).chain(next.loads.iter().filter_map(|l| l.edge().map(str::to_string))).filter(|e| e.starts_with("removed:")).count();
                self.edit(|p| *p = next);
                self.clear_selection();
                let kept = if lost == 0 { "supports and loads follow their faces".to_string() } else { format!("{lost} support(s) or load(s) lost their face: see the Check card") };
                self.status = format!("{} (undoable). The solid now has {faces} faces; {kept}.", match solid_op {
                    SolidOp::Fillet { radius } => format!("Filleted every edge with radius {radius}"),
                    SolidOp::Hole { diameter, .. } => format!("Cut a hole of diameter {diameter}"),
                    SolidOp::Boss { diameter, height, .. } => format!("Added a boss of diameter {diameter}, height {height}"),
                });
            }
            Err(e) => self.status = e,
        }
    }
}
