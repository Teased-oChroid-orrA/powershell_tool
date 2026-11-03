//! What the Pressure Vessel results read as, independent of any front end: each input row's display text, the margin's
//! text and tone, and the plain-text report. The terminal view (`view.rs`) and the native window
//! (`gpu_viewer/editor/vessel`) both read them here so the two never word a result differently.

use crate::theme::fmt_margin;

use pressure_vessel_solver::buckling::BucklingApplicability;
use pressure_vessel_solver::failure::governing;
use pressure_vessel_solver::geometry::{classify, GeometryClassification};
use pressure_vessel_solver::thickness::ThicknessSolverOutcome;

use super::model::{FieldRow, PressureVesselModel};

/// The value an input row shows (unit and precision per field, `NumberTarget::format_value`).
pub fn display_value(model: &PressureVesselModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleEndCondition => if model.closed_ends { "Closed ends" } else { "Open ends" }.to_string(),
        FieldRow::OpenMaterialPicker => model.material().name.to_string(),
        FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
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

    let (geometry, pressure) = match (&model.geometry, &model.pressure) {
        (Ok(g), Ok(p)) => (*g, *p),
        (g, p) => {
            if let Err(e) = g {
                out.push_str(&format!("Invalid geometry: {e:?}\n"));
            }
            if let Err(e) = p {
                out.push_str(&format!("Invalid pressure: {e:?}\n"));
            }
            return out;
        }
    };

    out.push_str(&format!("Outer diameter:   {:.4} in\n", 2.0 * geometry.outer_radius));
    out.push_str(&format!("Inner diameter:   {:.4} in\n", 2.0 * geometry.inner_radius));
    out.push_str(&format!("Wall thickness:   {:.4} in\n", geometry.wall_thickness()));
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
            out.push_str(&format!("Minimum wall thickness for required MS {:.2} (pressure only, not thermal-aware): {:.4} in\n", model.required_ms, sol.wall_thickness));
        }
        Some(ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found }) => {
            out.push_str(&format!("No wall thickness up to {largest_radius_tried:.1} in satisfies the required MS (best margin found: {}).\n", fmt_margin(*best_margin_found)));
        }
        None => {}
    }

    out
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
