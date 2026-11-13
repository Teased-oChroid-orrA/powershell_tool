//! Text of the Bushing workbench shared by the native window and the report: the label and shown value of each input row,
//! and the plain-text report.
use super::model::{self, BushingModel, FieldRow};
use super::advice::Severity;
use crate::theme::fmt_margin;

/// Label of a row; tolerance rows read "... Tol" or "... Limits" by entry mode.
pub fn row_text(mode: model::ToleranceMode, row: FieldRow) -> &'static str {
    match row {
        FieldRow::Tol(group) => group.label(mode),
        other => model::row_label(other),
    }
}

pub fn display_value(model: &BushingModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) | FieldRow::AdvancedSection => String::new(),
        FieldRow::ToggleFitType => model::label_fit_type(model.fit_type).to_string(),
        FieldRow::ToggleToleranceMode => model.tolerance_mode.label().to_string(),
        FieldRow::Tol(group) => model.tolerance_display(group),
        FieldRow::ToggleBushingType => model::label_bushing_type(model.bushing_type).to_string(),
        FieldRow::ToggleIdType => model::label_id_type(model.id_type).to_string(),
        FieldRow::ToggleEndConstraint => model::label_end_constraint(model.end_constraint).to_string(),
        FieldRow::ToggleCsMode => model::label_cs_mode(model.cs_mode).to_string(),
        FieldRow::ToggleExtCsMode => model::label_cs_mode(model.ext_cs_mode).to_string(),
        FieldRow::ToggleEnforcementEnabled => if model.enforcement_enabled { "Enabled" } else { "Disabled" }.to_string(),
        FieldRow::ToggleLockBore => bool_label(model.lock_bore),
        FieldRow::TogglePreserveBoreNominal => bool_label(model.preserve_bore_nominal),
        FieldRow::ToggleAllowBoreNominalShift => bool_label(model.allow_bore_nominal_shift),
        FieldRow::ToggleAssemblyThermalEnabled => if model.assembly_thermal_enabled { "Enabled" } else { "Disabled" }.to_string(),
        FieldRow::OpenHousingMaterialPicker => model.housing_material().name.to_string(),
        FieldRow::OpenBushingMaterialPicker => model.bushing_material().name.to_string(),
        FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
}

fn bool_label(b: bool) -> String {
    if b { "Yes" } else { "No" }.to_string()
}

/// The plain-text report (Export report): pure and synchronous, no filesystem access.
pub fn build_report_text(model: &BushingModel) -> String {
    let out = &model.output;
    let mut s = String::new();
    s.push_str("Bushing Workbench Report\n");
    s.push_str("========================\n\n");
    s.push_str(&format!("OD geometry:      {}\n", model::label_bushing_type(model.bushing_type)));
    s.push_str(&format!("ID geometry:      {}\n", model::label_id_type(model.id_type)));
    s.push_str(&format!("Housing material: {}\n", model.housing_material().name));
    s.push_str(&format!("Bushing material: {}\n", model.bushing_material().name));
    s.push_str(&format!("Bore diameter:    {:.4} in\n", model.bore_dia));
    s.push_str(&format!("Bushing ID:       {:.4} in\n", model.id_bushing));
    s.push_str(&format!("Target interference: {:.4} in\n", model.interference));
    s.push_str(&format!("Delta interference, service temp change {:+.1} F: {:+.5} in (in service {:.5} in)\n", model.delta_t, out.delta_thermal, out.delta_total));
    if model.assembly_thermal_enabled {
        s.push_str(&format!("Delta interference, install thermal assist: {:+.5} in (at install {:.5} in)\n", out.assembly_thermal_delta, out.install_delta));
    }
    s.push('\n');

    s.push_str(&format!("OD installed:     {:.4} in\n", out.od_installed));
    s.push_str(&format!("Straight wall:    {:.4} in ({})\n", out.wall_straight, if out.fail_straight { "FAIL" } else { "OK" }));
    s.push_str(&format!("Neck wall:        {:.4} in ({})\n", out.wall_neck, if out.fail_neck { "FAIL" } else { "OK" }));
    s.push_str(&format!("Contact pressure: {:.0} psi\n", out.pressure));
    s.push_str(&format!("Housing hoop stress: {:.0} psi, MS {}\n", out.stress_hoop_housing, fmt_margin(out.housing_ms)));
    s.push_str(&format!("Bushing hoop stress: {:.0} psi, MS {}\n", out.stress_hoop_bushing, fmt_margin(out.bushing_ms)));
    s.push_str(&format!("Install force:    {:.1} lbf\n", out.install_force));
    s.push_str(&format!("Retained force:   {:.1} lbf\n\n", out.retained_install_force));

    s.push_str(&format!("Governing: {} ({})\n", out.governing.name, fmt_margin(out.governing.margin)));
    s.push_str("Candidates:\n");
    for c in &out.candidates {
        s.push_str(&format!("  {:<28} {}\n", c.name, fmt_margin(c.margin)));
    }
    s.push('\n');
    let failing: Vec<_> = model.checks.iter().filter(|c| c.severity != Severity::Pass).collect();
    if !failing.is_empty() {
        s.push_str("Checks needing attention:\n");
        for c in failing {
            s.push_str(&format!("  [{}] {}: {}\n", if c.severity == Severity::Fail { "FAIL" } else { "WARN" }, c.kind.label(), c.detail));
        }
        for (i, r) in model.recommendations.iter().enumerate() {
            s.push_str(&format!("  recommendation {}: {} ({})\n", i + 1, r.summary, r.outcome));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "Tolerance status: {:?}\n",
        out.tolerance_status
    ));
    for note in &out.tolerance_notes {
        s.push_str(&format!("  note: {note}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_report_text_mentions_the_governing_mode() {
        let model = BushingModel::default();
        let text = build_report_text(&model);
        assert!(text.contains("Governing:"));
        assert!(text.contains(model.output.governing.name));
    }

    #[test]
    fn the_report_names_the_service_temperature_change_and_the_install_thermal_assist() {
        let mut model = BushingModel::default();
        model.commit_number(model::NumberTarget::DeltaT, 100.0);
        assert!(!build_report_text(&model).contains("install thermal assist"));
        model.toggle_assembly_thermal_enabled();
        model.commit_number(model::NumberTarget::AssemblyBushingTemp, -100.0);
        let report = build_report_text(&model);
        assert!(report.contains("Delta interference, service temp change") && report.contains("install thermal assist"), "{report}");
    }
}
