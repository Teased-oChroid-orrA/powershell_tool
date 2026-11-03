//! What the Preload Analysis results read as, independent of any front end: each input row's display text, the
//! margins and the verdict they give, and the plain-text report. The terminal view (`view.rs`) and the native window
//! (`gpu_viewer/editor/preload`) both read them here so the two never word a result differently.

use fastened_joint_solver::solve::{JointSolution, SolverStatus};

use super::fe_check::FeResult;
use super::model::{label_bearing_model, label_tightening_from, FieldRow, PreloadModel, MAX_MEMBERS};
use crate::theme::{fmt_margin, StatusTone};

/// The value an input row shows (unit and precision per field, `NumberTarget::format_value`).
pub fn display_value(model: &PreloadModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleMode => model.mode.label().to_string(),
        FieldRow::OpenBoltPicker => model.matching_bolt().map(|b| b.designation.to_string()).unwrap_or_else(|| "Custom".to_string()),
        FieldRow::OpenTemplatePicker => format!("{} member(s) - Enter to choose", model.members.len()),
        FieldRow::ToggleTighteningFrom => label_tightening_from(model.tightening_from).to_string(),
        FieldRow::AdvancedSection => String::new(),
        FieldRow::ToggleBearingModel => label_bearing_model(model.bearing_model).to_string(),
        FieldRow::ToggleMemberStiffness => match (model.member_stiffness_fe, model.fe_angle_deg) {
            (false, _) => "Pressure Cone".to_string(),
            (true, Some(a)) => format!("Finite Element (cone {a:.1} deg)"),
            (true, None) => "Finite Element (pending, using cone)".to_string(),
        },
        FieldRow::ToggleExternalLoadEnabled => bool_label(model.external_load_enabled),
        FieldRow::ToggleSlipEnabled => bool_label(model.slip_enabled),
        FieldRow::ToggleStrengthLimitsEnabled => bool_label(model.strength_limits_enabled),
        FieldRow::ToggleAddMember => format!("Enter ({}/{})", model.members.len(), MAX_MEMBERS),
        FieldRow::ToggleRemoveMember => "Enter".to_string(),
        FieldRow::ToggleUncertaintyEnabled => bool_label(model.uncertainty_enabled),
        FieldRow::ToggleMonteCarloEnabled => bool_label(model.monte_carlo_enabled),
        FieldRow::ToggleThreadLoadDistributionEnabled => bool_label(model.thread_load_distribution_enabled),
        FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
}

fn bool_label(b: bool) -> String {
    if b { "Enabled" } else { "Disabled" }.to_string()
}

/// The solver's status as a short word and its tone.
pub fn status_label(status: SolverStatus) -> (&'static str, StatusTone) {
    match status {
        SolverStatus::Converged => ("CONVERGED", StatusTone::Success),
        SolverStatus::PrevailingTorqueExceedsApplied => ("PREVAILING TORQUE EXCEEDS APPLIED", StatusTone::Warning),
        SolverStatus::NotBracketed => ("NOT BRACKETED", StatusTone::Danger),
        SolverStatus::MaxIterationsExceeded => ("MAX ITERATIONS EXCEEDED", StatusTone::Danger),
    }
}

/// Every margin of safety the solution has, by name: yield and ultimate on each stress section (with strength limits
/// entered), thread shear, separation under the service load, slip. Unbounded margins (nothing loads them) are left out.
pub fn margins(s: &JointSolution) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for (label, section) in [("Shank", &s.stress.shank), ("Tensile area", &s.stress.tensile_stress_area), ("Thread root", &s.stress.thread_root)] {
        out.push((format!("{label} yield"), section.yield_margin));
        out.push((format!("{label} ultimate"), section.ultimate_margin));
    }
    out.extend(s.thread_shear_margin.map(|m| ("Thread shear".to_string(), m)));
    out.extend(s.service_load.as_ref().map(|l| ("Separation".to_string(), l.separation_margin)));
    out.extend(s.slip_margin.map(|m| ("Slip".to_string(), m)));
    out.retain(|(_, m)| m.is_finite());
    out
}

/// The joint passes when the solve converged and no margin is negative.
pub fn passes(s: &JointSolution) -> bool {
    s.status == SolverStatus::Converged && margins(s).iter().all(|(_, m)| *m >= 0.0) && !s.service_load.as_ref().is_some_and(|l| l.separated)
}

/// Plain-text export, pure and synchronous (no filesystem access). `fe` is the member-compliance cross-check for exactly
/// this joint, when there is one.
pub fn build_report_text(model: &PreloadModel, fe: Option<&Result<FeResult, String>>) -> String {
    let mut s = String::new();
    s.push_str("Preload Analysis Report\n");
    s.push_str("========================\n\n");
    s.push_str(&format!("Mode: {}\n", model.mode.label()));
    s.push_str(&format!("Tightening from: {}\n\n", label_tightening_from(model.tightening_from)));

    let solution = match &model.output {
        Ok(s) => s,
        Err(e) => {
            s.push_str(&format!("Invalid input: {e:?}\n"));
            return s;
        }
    };

    s.push_str(&format!("Solver status: {:?}\n", solution.status));
    s.push_str(&format!("Solved preload: {:.4}\n", solution.preload));
    s.push_str(&format!(
        "Torque breakdown: applied {:.4}, thread {:.4} ({:.1}%), bearing {:.4} ({:.1}%), prevailing {:.4} ({:.1}%), residual {:.4}\n\n",
        solution.torque.applied,
        solution.torque.thread,
        solution.torque.thread_fraction() * 100.0,
        solution.torque.bearing,
        solution.torque.bearing_fraction() * 100.0,
        solution.torque.prevailing,
        solution.torque.prevailing_fraction() * 100.0,
        solution.torque.residual
    ));
    s.push_str(&format!(
        "Deformation: fastener elongation {:.6}, member compression {:.6}, total {:.6}\n",
        solution.deformation.fastener_elongation, solution.deformation.member_compression, solution.deformation.total_closure
    ));
    s.push_str(&format!(
        "Nut rotation: {:.4} rev ({:.2} deg), fastener twist {:.3} deg, est. tool rotation {:.2} deg\n\n",
        solution.rotation.revolutions, solution.rotation.degrees, solution.rotation.torsional_twist_degrees, solution.rotation.estimated_tool_rotation_degrees
    ));
    for (label, section) in [("Shank", &solution.stress.shank), ("Tensile area", &solution.stress.tensile_stress_area), ("Thread root", &solution.stress.thread_root)] {
        s.push_str(&format!(
            "{label}: axial {:.2}, shear {:.2}, von Mises {:.2}, yield MS {}, ult MS {}\n",
            section.axial,
            section.torsional_shear,
            section.von_mises,
            fmt_margin(section.yield_margin),
            fmt_margin(section.ultimate_margin)
        ));
    }
    if let Some(service) = &solution.service_load {
        s.push_str(&format!(
            "\nService load: external {:.4}, bolt load {:.4}, clamp load {:.4}, separation load {:.4}, margin {}\n",
            service.external_load,
            service.bolt_load,
            service.member_load,
            service.separation_load,
            fmt_margin(service.separation_margin)
        ));
    }
    if let (Some(capacity), Some(margin)) = (solution.slip_capacity, solution.slip_margin) {
        s.push_str(&format!("Slip capacity {capacity:.4}, margin {}\n", fmt_margin(margin)));
    }
    if let Some(u) = solution.uncertainty {
        s.push_str(&format!("\nUncertainty (worst case): preload {:.1} .. {:.1} .. {:.1} lbf (min/nominal/max)\n", u.preload_min, u.preload_nominal, u.preload_max));
    }
    if let Some(mc) = solution.monte_carlo {
        s.push_str(&format!(
            "Uncertainty (Monte Carlo, seed {}, {} samples): preload mean {:.1} +/- {:.1} lbf, range {:.1} .. {:.1} lbf\n",
            mc.seed, mc.samples, mc.preload_mean, mc.preload_std_dev, mc.preload_min, mc.preload_max
        ));
    }
    if !solution.warnings.is_empty() {
        s.push_str("\nWarnings:\n");
        for w in &solution.warnings {
            s.push_str(&format!("  - {w}\n"));
        }
    }
    if let Some(Ok(r)) = fe {
        s.push_str(&format!(
            "\nMember compliance cross-check (FE, axisymmetric): cone C_m {:.4e}, FE C_m {:.4e} ({:+.1}%); joint load fraction cone {:.3}, FE {:.3}{}\n",
            r.cone,
            r.fe.compliance,
            r.difference() * 100.0,
            r.fraction_cone,
            r.fraction_fe,
            r.equivalent_angle_deg.map(|a| format!("; equivalent cone half angle {a:.1} deg")).unwrap_or_default()
        ));
    }
    s
}

