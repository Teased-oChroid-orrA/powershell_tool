//! What the Pressure Vessel results read as, independent of any front end: each input row's display text, the margin's
//! text and tone, and the plain-text report. The native window
//! (`gpu_viewer/editor/vessel`) reads them here, so no front end words a result differently.

use crate::theme::fmt_margin;

use pressure_vessel_solver::buckling::BucklingApplicability;
use pressure_vessel_solver::failure::governing;
use pressure_vessel_solver::geometry::{classify, GeometryClassification};
use pressure_vessel_solver::heads::HeadError;
use pressure_vessel_solver::thickness::ThicknessSolverOutcome;

use super::model::{FieldRow, PressureVesselModel};

/// The value an input row shows (unit and precision per field, `NumberTarget::format_value`).
pub fn display_value(model: &PressureVesselModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleEndCondition => if model.closed_ends { "Closed ends" } else { "Open ends" }.to_string(),
        FieldRow::ToggleHeadComponent => model.head.component.label().to_string(),
        FieldRow::OpenMaterialPicker => model.material().name.to_string(),
        FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
}

/// Why the code check gave no number, in the words both the window and the report use.
pub fn head_error_text(e: &HeadError) -> String {
    match e {
        HeadError::InvalidInput(what) => format!("Invalid input: {what}."),
        HeadError::OutsideRules(why) => format!("Outside the code rules: {why}."),
    }
}

/// The code check as report lines (empty text never: an error is stated, not omitted).
fn heads_report(model: &PressureVesselModel, out: &mut String) {
    let h = &model.head;
    let c = &model.heads;
    out.push_str(&format!("\nASME VIII-1 code check (UG-27 / UG-32): {}\n", h.component.label()));
    out.push_str(&format!("  R {:.4} in, L {:.4} in, r {:.4} in, S {:.0} psi, E {:.2}, P {:.0} psi (corroded condition)\n", h.radius, h.crown_radius, h.knuckle_radius, h.allowable_stress, h.efficiency, h.pressure));
    match &c.required {
        Ok(t) => out.push_str(&format!("  Required thickness: {t:.4} in\n")),
        Err(e) => out.push_str(&format!("  Required thickness: {}\n", head_error_text(e))),
    }
    match (&c.provided, &c.mawp) {
        (Some(t), Some(Ok(p))) => out.push_str(&format!("  Provided (remaining wall): {t:.4} in, MAWP {p:.0} psi\n")),
        (Some(t), Some(Err(e))) => out.push_str(&format!("  Provided (remaining wall): {t:.4} in, MAWP: {}\n", head_error_text(e))),
        _ => out.push_str("  Provided thickness: none, the vessel geometry is invalid.\n"),
    }
    if let Some(m) = c.margin() {
        out.push_str(&format!("  Margin: {}\n", fmt_margin(m)));
    }
    out.push_str("  Allowable stress and efficiency are entered, not looked up; check the coefficients against your code edition.\n");
}

/// A check passes at a margin of zero or more. An unbounded margin (nothing loads that check, e.g. no pressure) passes.
pub fn passes(r: &pressure_vessel_solver::failure::MarginResult) -> bool {
    r.margin >= 0.0
}

/// Plain-text export - vessel spec, governing/classification, every check,
/// buckling/thermal notes, and the minimum-thickness solve outcome. Pure
/// and synchronous (no filesystem access) so `mod.rs`'s `e` key can build
/// it directly in the reducer and hand the finished string to `main.rs`
/// via `Effect::ExportPressureVesselReport` - the reducer itself still
/// never touches the filesystem (see this crate's own `Contracts` in
/// `AGENTS.md`).
pub fn build_report_text(model: &PressureVesselModel) -> String {
    let mut out = String::new();
    out.push_str("Pressure Vessel Analyzer Report\n");
    out.push_str("================================\n\n");
    out.push_str(&format!("Nominal wall thickness: {:.4} in\n", model.wall_thickness));
    out.push_str(&format!("Corrosion allowance: {:.4} in\n", model.corrosion_allowance));
    out.push_str(&format!("Mill undertolerance: {:.2} %\n", model.mill_undertolerance));
    out.push_str("Remaining wall = nominal wall * (1 - mill undertolerance / 100) - corrosion allowance.\n");
    out.push_str("Nominal OD stays fixed; all loss enlarges the bore. Checks and drawing use remaining wall.\n\n");

    let (geometry, pressure) = match (&model.geometry, &model.pressure) {
        (Ok(g), Ok(p)) => (*g, *p),
        (g, p) => {
            if let Err(e) = g {
                out.push_str(&format!("Invalid geometry: {e:?}\n"));
            }
            if let Err(e) = p {
                out.push_str(&format!("Invalid pressure: {e:?}\n"));
            }
            heads_report(model, &mut out);
            return out;
        }
    };

    out.push_str(&format!("Outer diameter:   {:.4} in\n", 2.0 * geometry.outer_radius));
    out.push_str(&format!("Inner diameter:   {:.4} in\n", 2.0 * geometry.inner_radius));
    out.push_str(&format!("Remaining wall thickness: {:.4} in\n", geometry.wall_thickness()));
    out.push_str(&format!("Internal pressure: {:.0} psi\n", pressure.internal_pressure));
    out.push_str(&format!("External pressure: {:.0} psi\n", pressure.external_pressure));
    out.push_str(&format!("End condition:    {}\n", if model.closed_ends { "Closed ends" } else { "Open ends" }));
    out.push_str(&format!("Material:         {}\n", model.material().name));
    if model.temperature_differential != 0.0 {
        out.push_str(&format!("Temperature differential (inner - outer): {:+.1} degF\n", model.temperature_differential));
    }
    out.push_str(&format!("Classification:   {}\n", if classify(&geometry) == GeometryClassification::ThinWall { "Thin-wall" } else { "Thick-wall" }));
    out.push('\n');

    if !model.rows.is_empty() {
        let governing_result = governing(&model.rows);
        out.push_str(&format!("Governing: {} ({})\n\n", governing_result.name, fmt_margin(governing_result.margin)));
        out.push_str("Checks:\n");
        for r in &model.rows {
            out.push_str(&format!("  {:<28} applied {:>10.0} psi  allowable {:>10.0} psi  MS {}\n", r.name, r.applied, r.allowable, fmt_margin(r.margin)));
        }
        out.push('\n');
    }

    match &model.buckling {
        BucklingApplicability::NotApplicable => out.push_str("Buckling: not applicable - no external pressure entered.\n"),
        BucklingApplicability::InsufficientData => out.push_str("Buckling: insufficient data - enter an unsupported length.\n"),
        BucklingApplicability::OutsideValidityRange => out.push_str("Buckling: outside the formulas' validity range (OD/t < 40).\n"),
        BucklingApplicability::Evaluated(_) => {}
    }

    match &model.thickness_outcome {
        Some(ThicknessSolverOutcome::Converged(sol)) => {
            out.push_str(&format!("Minimum remaining wall thickness for required MS {:.2}: {:.4} in\n", model.required_ms, sol.wall_thickness));
            out.push_str(&format!("Minimum nominal wall thickness with allowances: {:.4} in\n", model.wall_allowance().unwrap().nominal_thickness(sol.wall_thickness)));
            out.push_str(&format!("Sizing outer diameter: {:.4} in\n", 2.0 * sol.outer_radius));
            out.push_str("Pressure only, without thermal stress or buckling. Sizing holds the analyzed bore fixed and grows OD.\n");
        }
        Some(ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found }) => {
            out.push_str(&format!("No wall thickness up to {largest_radius_tried:.1} in satisfies the required MS (best margin found: {}).\n", fmt_margin(*best_margin_found)));
        }
        None => {}
    }

    walls_report(model, &mut out);
    heads_report(model, &mut out);
    out
}

/// "Minimum wall by requirement" (and, with the line run on, its cases, reactions and governing station).
fn walls_report(model: &PressureVesselModel, out: &mut String) {
    use super::walls::WallOutcome;
    if model.requirements.is_empty() {
        return;
    }
    out.push_str("\nMinimum wall by requirement");
    out.push_str(if model.line.enabled { " (supported line run, every case):\n" } else { " (cylinder, remaining wall):\n" });
    let walls = match &model.walls {
        Some(Ok(w)) => w,
        Some(Err(e)) => {
            out.push_str(&format!("  Not solved: {e}\n"));
            return;
        }
        None => {
            out.push_str("  Not solved yet.\n");
            return;
        }
    };
    out.push_str(&format!("  Material class {}; yield {:.1} ksi, ultimate {:.1} ksi{}.\n", model.kind().label(), model.effective_material().sy_ksi, model.effective_material().ftu_ksi, if model.allowable_override.iter().any(Option::is_some) { " (overrides entered)" } else { "" }));
    out.push_str(&format!("  {:<28} {:<26} {:>7} {:>8} {:>16} {:>18}\n", "Requirement", "Criterion", "Factor", "Req. MS", "Minimum wall in", "Margin at wall"));
    let governing = walls.governing();
    for (i, (r, w)) in model.requirements.iter().zip(&walls.rows).enumerate() {
        let wall = match &w.outcome {
            WallOutcome::Wall { wall, .. } => format!("{wall:.4}"),
            WallOutcome::Infeasible { .. } => "no wall".to_string(),
            WallOutcome::NotEvaluable(_) => "not evaluable".to_string(),
        };
        let margin = w.margin_at_wall.map_or("-".to_string(), |ms| format!("{} {}", fmt_margin(ms), if ms >= r.required_ms { "met" } else { "short" }));
        out.push_str(&format!("{} {:<28} {:<26} {:>7} {:>8} {:>16} {:>18}\n", if governing == Some(i) { '*' } else { ' ' }, r.label, r.criterion.label(), r.factor, r.required_ms, wall, margin));
        if let WallOutcome::NotEvaluable(why) = &w.outcome {
            out.push_str(&format!("      {why}\n"));
        }
    }
    if let Some(g) = governing {
        match walls.rows[g].outcome {
            WallOutcome::Wall { wall, .. } => out.push_str(&format!("  Governing wall {wall:.4} in remaining ({}); the wall entered is {:.4} in.\n", model.requirements[g].label, walls.current_wall)),
            WallOutcome::Infeasible { best_margin } => out.push_str(&format!("  No wall meets {} (best margin {}).\n", model.requirements[g].label, fmt_margin(best_margin))),
            WallOutcome::NotEvaluable(_) => {}
        }
    }
    if model.requirements.iter().any(|r| r.is_neutral()) {
        out.push_str("  Rows at factor 1 and required MS 0 are the unedited seeds, not a program's values.\n");
    }
    if model.line.enabled {
        out.push_str(&format!("  Run length {} in; {} support(s), {} point load(s), {} case(s).\n", model.line.length, model.line.supports.len(), model.line.loads.len(), model.line.cases.len()));
        let sg = &model.line.segment;
        if sg.enabled {
            let end = |e: &super::line::EndIn, x: f64| match e.fixity {
                super::line::FixityIn::Spring => format!("x {x}: spring clip {} lbf-in/rad{}", e.spring, if e.axial_anchor { ", axial anchor" } else { "" }),
                f => format!("x {x}: {}{}", f.label().to_lowercase(), if e.axial_anchor { ", axial anchor" } else { "" }),
            };
            out.push_str(&format!("  Segment between two supports: {}; {}.\n", end(&sg.ends[0], 0.0), end(&sg.ends[1], model.line.length)));
            out.push_str("  A cut segment is an idealisation: the real end restraint depends on the neighbouring spans and clips.\n");
            if sg.bound {
                out.push_str("  Bound (both ends pinned, then both clamped; the stated end conditions are not used), worse margin per requirement:\n");
                for r in model.bound_rows() {
                    let b = if r.bound == pressure_vessel_solver::line_run::Bound::Clamped { "clamped" } else { "pinned" };
                    out.push_str(&format!("    {}: pinned {:+.3}, clamped {:+.3}, worse {:+.3} ({b}); required {:+.3}.\n", r.label, r.pinned, r.clamped, r.worse(), r.required_ms));
                }
                if let Some(Err(e)) = &model.line_result {
                    out.push_str(&format!("    Not solved: {e}\n"));
                }
            }
        }
        if let Some(Ok(res)) = &model.line_result {
            for case in &res.cases {
                let sum: Vec<String> = (0..3).map(|k| format!("{:.1}", case.reactions.iter().map(|r| r.force[k]).sum::<f64>())).collect();
                out.push_str(&format!("  Case {}: reactions sum to ({}) lbf.\n", case.name, sum.join(", ")));
            }
        }
        if let Some(x) = model.worst_point() {
            out.push_str(&format!("  Lowest margin over the requirements, selected case: {:+.3} at x {:.2} in.\n", x[1], x[0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::model::NumberTarget;

    #[test]
    fn build_report_text_contains_the_governing_mode_and_every_check() {
        let model = PressureVesselModel::default();
        let text = build_report_text(&model);
        assert!(text.contains("Governing:"));
        for r in &model.rows {
            assert!(text.contains(r.name), "report must mention check `{}`", r.name);
        }
    }

    #[test]
    fn the_report_lists_every_requirement_and_marks_the_governing_one() {
        let model = PressureVesselModel::default();
        let text = build_report_text(&model);
        assert!(text.contains("Minimum wall by requirement"), "{text}");
        for r in &model.requirements {
            assert!(text.contains(&r.label), "{}", r.label);
        }
        assert!(text.contains("\n* "), "the governing row is starred: {text}");
        assert!(text.contains("unedited seeds"));
    }

    #[test]
    fn an_unloaded_check_has_an_unbounded_margin_and_passes() {
        let mut model = PressureVesselModel::default();
        model.commit_number(NumberTarget::InternalPressure, 0.0);
        assert!(model.rows.iter().all(|r| r.margin.is_infinite() && passes(r)), "{:?}", model.rows);
    }

    #[test]
    fn build_report_text_on_invalid_geometry_still_produces_readable_output() {
        let mut model = PressureVesselModel::default();
        model.commit_number(NumberTarget::WallThickness, 10.0);
        let text = build_report_text(&model);
        assert!(text.contains("Invalid geometry"));
    }
}
