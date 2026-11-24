//! The viewport of the Plate Edge window: the plate as the FE meshes it (graded about the hole, a quarter of the plate), then
//! the deformed model of the last solve coloured by equivalent plastic strain.
use super::inputs::PlateEdgeInputs;
use super::run::{FeLine, Outcome};
use crate::gpu_viewer::{scene::SceneSource, Scene};
use fea_core::sweep::extrude;
use lug_solver::PlaneMode;
use std::sync::Arc;

pub const KEY_LABELS: [&str; 4] = ["Plate", "Hole wall", "Free edge", "Cut face"];

/// The FE line the picture shows: the 3D solid if it ran, else plane strain.
pub fn pick_fields(o: &Outcome) -> Option<&FeLine> {
    ["3D solid", "2D plane strain", "2D plane stress"].iter().find_map(|l| o.lines.iter().find(|x| x.label == *l && x.fields.is_some()))
}

pub fn build(inputs: &PlateEdgeInputs, name: &str, outcome: Option<&Outcome>) -> Result<Scene, String> {
    if let Some(line) = outcome.and_then(pick_fields) {
        let f = line.fields.as_ref().expect("picked for its fields");
        let last = line.last();
        let label = format!("{name} | {} | {} elements around, {} dof | pin load at 0.1 D travel {:.0} lbf", line.label, last.around, last.dofs, last.collapse);
        return SceneSource { mesh: Arc::new(f.mesh.clone()), displacement: f.u.clone(), values: f.plastic.clone(), period: None, label, field_label: "Equivalent plastic strain".into(), fields: Vec::new() }.build();
    }
    picture(inputs, name)
}

/// The plate before any solve: the hole wall in one colour.
fn picture(inputs: &PlateEdgeInputs, name: &str) -> Result<Scene, String> {
    let case = inputs.picture_case()?;
    let fe = case.lug(PlaneMode::Strain);
    let half_t = 0.5 * case.thickness();
    let mesh = extrude(&fe.model.mesh, &[0.0, half_t])?;
    let r = 0.5 * case.dia;
    let values: Vec<f64> = mesh.nodes.iter().map(|p| if p[0].hypot(p[1]) < r * (1.0 + 1e-6) { 1.0 } else { 0.0 }).collect();
    let n = mesh.nodes.len();
    SceneSource { mesh: Arc::new(mesh), displacement: vec![0.0; 3 * n], values, period: None, label: format!("{name} | a quarter of the plate, hole {:.4} in, edge {:.3} in", case.dia, inputs.edge), field_label: "part (see color key)".into(), fields: Vec::new() }.build()
}
