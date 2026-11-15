//! The line drawn along the vertical axis, coloured by the bending moment (and, as the second field, the axial force)
//! of the selected case at each station, with a collar at every support. The values are the solver's stations
//! (`CaseResult::stations`); nothing here computes an action. The tube is enlarged for the picture only: a half-inch
//! tube two feet long would otherwise draw as a hairline.
use crate::gpu_viewer::{
    editor::drawing::{self, Mesh},
    scene::ScalarField,
    Scene,
};
use crate::toolboxes::line_run::model::LineRunModel;
use pressure_vessel_solver::line_run::CaseResult;

/// The drawn outer diameter is at least this fraction of the line's length.
const MIN_OD_FRACTION: f64 = 1.0 / 25.0;
pub const FIELDS: [&str; 2] = ["Bending moment", "Axial force"];

/// Enlargement of the tube's radius in the picture (1 when it already reads).
pub fn enlargement(od: f64, length: f64) -> f64 {
    (MIN_OD_FRACTION * length / od).max(1.0)
}

/// Station values by position along the line: one `(x, moment, axial)` per distinct station `x`, ascending. Where
/// actions jump (two stations at one `x`) the larger magnitude is kept.
fn profile(case: &CaseResult) -> Vec<(f64, f64, f64)> {
    let mut out: Vec<(f64, f64, f64)> = Vec::new();
    for s in &case.stations {
        let (m, n) = (s.resultant_moment(), s.axial_force);
        match out.last_mut() {
            Some(last) if (last.0 - s.x).abs() <= 1e-12 * s.x.abs().max(1.0) => {
                last.1 = last.1.max(m);
                if n.abs() > last.2.abs() {
                    last.2 = n;
                }
            }
            _ => out.push((s.x, m, n)),
        }
    }
    out
}

/// The picture of the selected case, or why there is none (no result).
pub fn scene(m: &LineRunModel, name: &str) -> Result<Scene, String> {
    let result = m.outcome.as_ref().map_err(|e| e.clone())?;
    let case = result.cases.get(m.inputs.selected).ok_or("no case selected")?;
    let tube = m.tube().ok_or("the tube section is invalid")?;
    let geometry = tube.geometry().map_err(|e| e.to_string())?;
    let (x0, x1) = (m.inputs.x_start, m.inputs.x_end);
    let k = enlargement(tube.outer_diameter, x1 - x0);
    let (ri, ro) = (geometry.inner_radius * k, geometry.outer_radius * k);
    let prof = profile(case);
    if prof.len() < 2 {
        return Err("no stations".into());
    }
    // The nearest station's value at a position along the line (axis is y).
    let nearest = |y: f64| prof.iter().min_by(|a, b| (a.0 - y).abs().total_cmp(&(b.0 - y).abs())).expect("not empty");
    let moment = |p: &[f64; 3]| nearest(p[1]).1 as f32;
    let mut mesh = Mesh::default();
    let outer: Vec<(f64, f64)> = prof.iter().map(|s| (ro, s.0)).collect();
    let inner: Vec<(f64, f64)> = prof.iter().map(|s| (ri, s.0)).collect();
    mesh.revolve(&outer, moment);
    mesh.revolve(&inner, moment);
    for end in [prof[0].0, prof[prof.len() - 1].0] {
        mesh.revolve(&[(ri, end), (ro, end)], moment);
    }
    let rows: Vec<Vec<(f64, f64)>> = prof.iter().map(|s| vec![(ri, s.0), (ro, s.0)]).collect();
    mesh.cut(&rows, moment);
    // A collar at each support.
    let half = (x1 - x0) / 120.0;
    for s in &m.inputs.supports {
        let c = ro * 1.6;
        mesh.revolve(&[(ro, s.x - half), (c, s.x - half), (c, s.x + half), (ro, s.x + half)], moment);
    }
    let values: [Vec<f64>; 2] = [mesh.positions.iter().map(|p| nearest(p[1]).1).collect(), mesh.positions.iter().map(|p| nearest(p[1]).2).collect()];
    let primary = ScalarField::from_f64(FIELDS[0], &values[0])?;
    mesh.values = primary.values;
    let axial = ScalarField::from_f64(FIELDS[1], &values[1])?;
    mesh.into_scene(format!("{name} | {}", case.name), FIELDS[0], primary.range, vec![axial])
}

/// A scene with nothing visible in it, for a model with no result.
pub fn empty(name: &str) -> Scene {
    drawing::empty(format!("{name} | nothing to draw"), [0.0, 1.0])
}
