//! Bridges the pure `pressure-vessel-solver`/`mechanics-core` engine into
//! UI-facing state, the same split `toolboxes/fastener_hole/model.rs` uses:
//! zero `ratatui`/`crossterm` here - only `mod.rs` (key routing) and
//! `view.rs` (rendering) know about the terminal.
//!
//! Unlike Fastener Holes' toleranced dimensions, every input here is a
//! plain `f64` (neither `app`'s nor `app-egui`'s Pressure Vessel Analyzer
//! has a tolerance-band concept), so there is no `NumberPart`/toleranced-value
//! machinery to carry over - `field_rows` is a fixed row list, not a
//! hole-type-dependent one.
//!
//! **Full, non-reduced Lame equations only**: every stress value here comes
//! from `pressure_vessel_solver::stress`/`failure`, which in turn call
//! straight into `mechanics_core::lame` - the exact same authoritative
//! thick-wall Lame solution `pressure-vessel-solver/src/geometry.rs`'s own
//! doc comment guarantees is used "for both thin-wall and thick-wall
//! scenarios." This module never computes a reduced/thin-wall shortcut
//! (e.g. `hoop = p*r/t`) itself - it only reads results out of that crate.
//! Thermal stress (`pressure_vessel_solver::thermal`) is superposed the
//! same way - `evaluate_failure_modes_with_thermal` is a strict superset of
//! `evaluate_failure_modes`, proven so in that crate's own tests.

use mechanics_core::materials::{Material, MATERIALS};
use pressure_vessel_solver::buckling::{evaluate_buckling, BucklingApplicability};
use pressure_vessel_solver::failure::{evaluate_failure_modes_with_thermal, MarginResult};
use pressure_vessel_solver::geometry::{CylinderGeometry, GeometryError};
use pressure_vessel_solver::pressure::{EndCondition, PressureError, PressureLoading};
use pressure_vessel_solver::thermal::ThermalLoading;
use pressure_vessel_solver::thickness::{solve_minimum_thickness, ThicknessSolverInputs, ThicknessSolverOutcome};

/// Which physical quantity a numeric row edits - drives both display
/// precision/unit and, on commit, which `PressureVesselModel` field is
/// updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberTarget {
    OuterDiameter,
    WallThickness,
    InternalPressure,
    ExternalPressure,
    RequiredMs,
    UnsupportedLength,
    TemperatureDifferential,
}

impl NumberTarget {
    pub fn label(self) -> &'static str {
        match self {
            NumberTarget::OuterDiameter => "Outer Diameter",
            NumberTarget::WallThickness => "Wall Thickness",
            NumberTarget::InternalPressure => "Internal Pressure",
            NumberTarget::ExternalPressure => "External Pressure",
            NumberTarget::RequiredMs => "Required Minimum MS",
            NumberTarget::UnsupportedLength => "Unsupported Length",
            NumberTarget::TemperatureDifferential => "Temp Differential",
        }
    }

    /// Display text for one value of this target - centralizes unit +
    /// precision per field rather than scattering format strings through
    /// `view.rs` (same reasoning as `fastener_hole/model.rs`'s own
    /// `LINEAR_DECIMALS`/`ANGLE_DECIMALS` constants).
    pub fn format_value(self, value: f64) -> String {
        match self {
            NumberTarget::OuterDiameter | NumberTarget::WallThickness | NumberTarget::UnsupportedLength => format!("{value:.4} in"),
            NumberTarget::InternalPressure | NumberTarget::ExternalPressure => format!("{value:.0} psi"),
            NumberTarget::RequiredMs => format!("{value:.2}"),
            NumberTarget::TemperatureDifferential => format!("{value:+.1} \u{b0}F"),
        }
    }

    /// Per-field validation hint, shown inline on the field row itself
    /// (not just surfaced later in the results pane) - `None` when the
    /// current value is fine on its own. Deliberately independent,
    /// single-field checks (never a full cross-field geometry solve) so
    /// each row can explain itself without needing every other field's
    /// current value - the one exception is `WallThickness`, which also
    /// needs `outer_diameter` to state the one cross-field invariant a
    /// user is most likely to trip (wall thicker than the vessel itself).
    pub fn validation_hint(self, model: &PressureVesselModel) -> Option<String> {
        match self {
            NumberTarget::OuterDiameter => (model.outer_diameter <= 0.0).then(|| "must be > 0".to_string()),
            NumberTarget::WallThickness => {
                if model.wall_thickness <= 0.0 {
                    Some("must be > 0".to_string())
                } else if model.wall_thickness >= model.outer_diameter / 2.0 {
                    // Short by design - the outer diameter that makes this
                    // invalid is already visible two rows up; a long
                    // restatement of it here only pushes this row (and
                    // `fields_required_width`'s own worst-case) wider than
                    // it needs to be.
                    Some("wall exceeds OD".to_string())
                } else {
                    None
                }
            }
            NumberTarget::InternalPressure => (model.internal_pressure < 0.0).then(|| "must be \u{2265} 0".to_string()),
            NumberTarget::ExternalPressure => (model.external_pressure < 0.0).then(|| "must be \u{2265} 0".to_string()),
            NumberTarget::UnsupportedLength => (model.unsupported_length < 0.0).then(|| "must be \u{2265} 0".to_string()),
            // A negative required MS and any temperature differential
            // (positive, negative, or zero) are all legitimate inputs -
            // no invalid range exists for either.
            NumberTarget::RequiredMs | NumberTarget::TemperatureDifferential => None,
        }
    }
}

/// One navigable/editable row - the row list is fixed content-wise (unlike
/// Fastener Holes' row list, which varies with hole type/solve-for, every
/// Pressure Vessel input is always relevant regardless of the others'
/// values), but is still built by a function rather than a `const` array so
/// non-selectable `Header` dividers can be inserted (see `field_rows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    /// A non-selectable section divider - see
    /// `toolboxes/bushing/model.rs::FieldRow::Header`'s doc comment for the
    /// same reasoning, applied here.
    Header(&'static str),
    ToggleEndCondition,
    /// Opens the material picker overlay (`material_picker.rs`) rather
    /// than cycling in place - once custom materials exist alongside the
    /// 17 built-in ones, a filterable list is strictly more usable than
    /// stepping through one at a time.
    OpenMaterialPicker,
    Number(NumberTarget),
}

/// The complete navigable row list, grouped under `Header` dividers - same
/// pattern `toolboxes/bushing/model.rs`/`toolboxes/preload_analysis/model.rs`
/// already use. Content-fixed (always the same 9 real rows, every input
/// always relevant), so this takes no `model` argument - unlike Fastener
/// Holes/Bushing/Preload Analysis, whose row lists vary with toolbox state.
pub fn field_rows() -> Vec<FieldRow> {
    vec![
        FieldRow::Header("Geometry"),
        FieldRow::Number(NumberTarget::OuterDiameter),
        FieldRow::Number(NumberTarget::WallThickness),
        FieldRow::Header("Pressure & End Condition"),
        FieldRow::Number(NumberTarget::InternalPressure),
        FieldRow::Number(NumberTarget::ExternalPressure),
        FieldRow::ToggleEndCondition,
        FieldRow::Header("Material"),
        FieldRow::OpenMaterialPicker,
        FieldRow::Header("Analysis Options"),
        FieldRow::Number(NumberTarget::RequiredMs),
        FieldRow::Number(NumberTarget::UnsupportedLength),
        FieldRow::Number(NumberTarget::TemperatureDifferential),
    ]
}

pub fn row_label(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(text) => text,
        FieldRow::ToggleEndCondition => "End Condition",
        FieldRow::OpenMaterialPicker => "Material",
        FieldRow::Number(target) => target.label(),
    }
}

/// One-line description shown in the bottom "Hint" panel while this row is
/// selected - same purpose as `toolboxes/bushing/model.rs::field_hint`.
/// Distinct from `NumberTarget::validation_hint` below, which is a
/// transient, state-dependent inline `⚠` warning rendered directly on the
/// row itself - this is a static, always-present description.
pub fn field_hint(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(_) => "",
        FieldRow::ToggleEndCondition => "Open ends (e.g. a pipe run) or closed ends (a capped vessel) - changes the axial stress term in the failure-mode solve.",
        FieldRow::OpenMaterialPicker => "Pick from the built-in material catalog or a previously-added custom material - Enter opens a filterable list.",
        FieldRow::Number(NumberTarget::OuterDiameter) => "Vessel outer diameter.",
        FieldRow::Number(NumberTarget::WallThickness) => "Wall thickness - must be less than half the outer diameter (else the inner radius would be non-positive).",
        FieldRow::Number(NumberTarget::InternalPressure) => "Internal pressure.",
        FieldRow::Number(NumberTarget::ExternalPressure) => "External pressure - nonzero together with an unsupported length enables the buckling check.",
        FieldRow::Number(NumberTarget::RequiredMs) => "Required minimum margin of safety - the minimum-thickness solve targets this margin, not just zero.",
        FieldRow::Number(NumberTarget::UnsupportedLength) => "Unsupported axial length between stiffening rings/supports - needed for the buckling check alongside external pressure.",
        FieldRow::Number(NumberTarget::TemperatureDifferential) => "Inner-to-outer temperature difference - 0 means thermal stress is not evaluated at all, not evaluated at zero.",
    }
}

/// Builds a fresh `&'static` [`Material`] for a user-entered custom
/// material by leaking its owned `name`/`id` strings exactly once, at
/// creation time - not per frame, not per recompute. `mechanics_core`'s
/// `Material`/every `pressure-vessel-solver` function that consumes it
/// requires `&'static str` for `id`/`name` (matching the crate's existing
/// built-in `MATERIALS` table, which is a `'static` array), and neither of
/// those crates were changed to accommodate this - leaking here, entirely
/// contained to this toolbox's own code, is the surgical option; teaching
/// a shared crate two other GUI heads also depend on to accept owned
/// `String`s instead would be a far larger, riskier change for a feature
/// this narrow. The leak is bounded by how many materials a user manually
/// adds in a session (a handful, realistically), not by anything that
/// scales with runtime or input size.
fn leak_custom_material(name: String, e_ksi: f64, sy_ksi: f64, ftu_ksi: f64, nu: f64, alpha_u_f: f64) -> &'static Material {
    let id: &'static str = Box::leak(format!("custom:{name}").into_boxed_str());
    let name: &'static str = Box::leak(name.into_boxed_str());
    // `fbru_ksi`/`fsu_ksi` (bearing/shear ultimate) are bushing-specific
    // fields `pressure-vessel-solver` never reads - zeroed rather than
    // guessed, since a wrong nonzero value here could look load-bearing
    // to a future reader when it structurally cannot be for this toolbox.
    Box::leak(Box::new(Material { id, name, e_ksi, sy_ksi, fbru_ksi: 0.0, fbru_e15_ksi: 0.0, fsu_ksi: 0.0, ftu_ksi, nu, alpha_u_f, extra: None }))
}

/// The whole toolbox's engineering state, plus every derived result -
/// recomputed fresh on every field mutation via [`PressureVesselModel::recompute`],
/// mirroring `FastenerHoleModel`'s own "recompute on every mutation, never
/// from a render path" discipline. Defaults match both existing GUI heads'
/// own `PressureVesselTool`/`PressureVesselWorkbench` defaults exactly.
pub struct PressureVesselModel {
    pub outer_diameter: f64,
    pub wall_thickness: f64,
    pub internal_pressure: f64,
    pub external_pressure: f64,
    pub closed_ends: bool,
    pub material_index: usize,
    pub required_ms: f64,
    pub unsupported_length: f64,
    /// `T_inner - T_outer`, °F. Zero means "not applicable" - no thermal
    /// contribution is evaluated at all (see `recompute`).
    pub temperature_differential: f64,
    /// User-added materials, appended to the built-in `MATERIALS` table by
    /// [`PressureVesselModel::material_catalog`] - `material_index` indexes
    /// into that combined sequence, built-ins first.
    pub custom_materials: Vec<&'static Material>,

    pub geometry: Result<CylinderGeometry, GeometryError>,
    pub pressure: Result<PressureLoading, PressureError>,
    /// Every evaluated failure mode (the four v1 stress modes, thermal
    /// contribution folded directly into their stress state - not a
    /// separate row - plus buckling appended when
    /// [`BucklingApplicability::Evaluated`]) - empty only when
    /// `geometry`/`pressure` is itself invalid.
    pub rows: Vec<MarginResult>,
    pub buckling: BucklingApplicability,
    /// `None` only when `geometry`/`pressure` is invalid - the minimum-
    /// thickness solve otherwise always runs, same as both existing GUI
    /// heads' own "recomputed every frame" behavior. Note this solve does
    /// NOT account for thermal stress (`pressure-vessel-solver::thickness`
    /// has no thermal-aware variant) - documented at its call site in
    /// `view.rs`, not silently presented as thermal-inclusive.
    pub thickness_outcome: Option<ThicknessSolverOutcome>,
}

impl Default for PressureVesselModel {
    fn default() -> Self {
        let custom_materials = super::persistence::load()
            .into_iter()
            .map(|m| leak_custom_material(m.name, m.e_ksi, m.sy_ksi, m.ftu_ksi, m.nu, m.alpha_u_f))
            .collect();
        let mut model = Self {
            outer_diameter: 6.0,
            wall_thickness: 1.0,
            internal_pressure: 5000.0,
            external_pressure: 0.0,
            closed_ends: true,
            material_index: 0, // "al7075" - MATERIALS[0], matching both existing heads' default material_id
            required_ms: 0.0,
            unsupported_length: 0.0,
            temperature_differential: 0.0,
            custom_materials,
            geometry: Err(GeometryError::NonPositiveInnerRadius),
            pressure: Err(PressureError::NegativePressure),
            rows: Vec::new(),
            buckling: BucklingApplicability::NotApplicable,
            thickness_outcome: None,
        };
        model.recompute();
        model
    }
}

impl PressureVesselModel {
    /// Built-in materials first, then user-added custom ones, in the order
    /// they were added - `material_index` is an index into exactly this
    /// sequence.
    pub fn material_catalog(&self) -> Vec<&'static Material> {
        mechanics_core::materials::builtin_catalog().chain(self.custom_materials.iter().copied()).collect()
    }

    pub fn material(&self) -> &'static Material {
        self.material_catalog().get(self.material_index).copied().unwrap_or(&MATERIALS[0])
    }

    pub fn select_material(&mut self, index: usize) {
        if index < self.material_catalog().len() {
            self.material_index = index;
            self.recompute();
        }
    }

    /// Adds a user-entered material to the catalog and selects it
    /// immediately - validation (non-empty name, finite/positive numeric
    /// fields) is the caller's job (`material_picker.rs`'s own
    /// `validate_and_build`), not this method's; it trusts its inputs.
    pub fn add_custom_material(&mut self, name: String, e_ksi: f64, sy_ksi: f64, ftu_ksi: f64, nu: f64, alpha_u_f: f64) {
        let material = leak_custom_material(name, e_ksi, sy_ksi, ftu_ksi, nu, alpha_u_f);
        self.custom_materials.push(material);
        self.material_index = self.material_catalog().len() - 1;
        self.recompute();
    }

    pub fn number_value(&self, target: NumberTarget) -> f64 {
        match target {
            NumberTarget::OuterDiameter => self.outer_diameter,
            NumberTarget::WallThickness => self.wall_thickness,
            NumberTarget::InternalPressure => self.internal_pressure,
            NumberTarget::ExternalPressure => self.external_pressure,
            NumberTarget::RequiredMs => self.required_ms,
            NumberTarget::UnsupportedLength => self.unsupported_length,
            NumberTarget::TemperatureDifferential => self.temperature_differential,
        }
    }

    /// Commits one edited numeric field. A non-finite result is silently
    /// ignored, leaving the previous value in place - the same convention
    /// `FastenerHoleModel::commit_number`/`settings_view.rs` already use;
    /// an out-of-range-but-finite value (e.g. a wall thickness that makes
    /// the inner radius non-positive) is accepted here and surfaces both
    /// as an inline "Invalid geometry" state in `view.rs` AND as this
    /// field's own [`NumberTarget::validation_hint`] - not silently
    /// swallowed either way.
    pub fn commit_number(&mut self, target: NumberTarget, raw: f64) {
        if !raw.is_finite() {
            return;
        }
        match target {
            NumberTarget::OuterDiameter => self.outer_diameter = raw,
            NumberTarget::WallThickness => self.wall_thickness = raw,
            NumberTarget::InternalPressure => self.internal_pressure = raw,
            NumberTarget::ExternalPressure => self.external_pressure = raw,
            NumberTarget::RequiredMs => self.required_ms = raw,
            NumberTarget::UnsupportedLength => self.unsupported_length = raw,
            NumberTarget::TemperatureDifferential => self.temperature_differential = raw,
        }
        self.recompute();
    }

    pub fn toggle_end_condition(&mut self) {
        self.closed_ends = !self.closed_ends;
        self.recompute();
    }

    /// Recomputes every derived result from current input state - the full
    /// applicable failure-mode set (thermal-superposed when a temperature
    /// differential is entered), buckling (if applicable), and the
    /// minimum-thickness solve, exactly mirroring `PressureVesselTool::ui`'s
    /// own "computed every frame regardless of step" behavior in
    /// `app-egui/src/pressure_vessel.rs`.
    pub fn recompute(&mut self) {
        let outer_radius = (self.outer_diameter / 2.0).max(0.0);
        let inner_radius = outer_radius - self.wall_thickness;
        self.geometry = CylinderGeometry::new(inner_radius, outer_radius);
        let end_condition = if self.closed_ends { EndCondition::Closed } else { EndCondition::Open };
        self.pressure = PressureLoading::new(self.internal_pressure, self.external_pressure, end_condition);

        let (Ok(geometry), Ok(pressure)) = (self.geometry, self.pressure) else {
            self.rows = Vec::new();
            self.buckling = BucklingApplicability::NotApplicable;
            self.thickness_outcome = None;
            return;
        };

        let material = *self.material();
        let thermal = self.thermal_loading(&material);
        let mut rows = evaluate_failure_modes_with_thermal(&geometry, &pressure, &material, thermal.as_ref());
        let buckling = evaluate_buckling(&geometry, &pressure, &material, Some(self.unsupported_length));
        if let BucklingApplicability::Evaluated(ref b) = buckling {
            rows.push(b.clone());
        }
        self.rows = rows;
        self.buckling = buckling;
        // Not thermal-aware - see this struct's own field doc comment.
        self.thickness_outcome = Some(solve_minimum_thickness(
            &ThicknessSolverInputs { inner_radius: geometry.inner_radius, pressure, material, required_minimum_ms: self.required_ms },
            100,
            1e-6,
        ));
    }

    /// `None` when the temperature differential is exactly zero - "not
    /// applicable", not a zero-valued thermal load evaluated anyway (same
    /// convention `evaluate_buckling` uses for "no external pressure").
    pub fn thermal_loading(&self, material: &Material) -> Option<ThermalLoading> {
        if self.temperature_differential == 0.0 {
            return None;
        }
        Some(ThermalLoading {
            delta_t: self.temperature_differential,
            alpha_per_f: material.alpha_u_f * 1e-6,
            e_psi: material.e_ksi * 1000.0,
            nu: material.nu,
        })
    }
}

/// Trims a fixed-decimal formatted number down for the edit buffer, the
/// same helper `fastener_hole/model.rs::format_for_edit` provides - kept as
/// its own small copy rather than shared, matching that module's own
/// "small, self-contained toolbox" precedent rather than introducing a
/// shared-utility module for a two-line function.
pub fn format_for_edit(value: f64) -> String {
    let s = format!("{value:.6}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() { "0".to_string() } else { trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_matches_both_existing_gui_heads_defaults() {
        let model = PressureVesselModel::default();
        assert_eq!(model.outer_diameter, 6.0);
        assert_eq!(model.wall_thickness, 1.0);
        assert_eq!(model.internal_pressure, 5000.0);
        assert_eq!(model.external_pressure, 0.0);
        assert!(model.closed_ends);
        assert_eq!(model.material().id, "al7075");
        assert_eq!(model.required_ms, 0.0);
        assert_eq!(model.unsupported_length, 0.0);
        assert_eq!(model.temperature_differential, 0.0);
    }

    #[test]
    fn default_model_solves_a_valid_geometry_and_evaluates_four_failure_modes() {
        let model = PressureVesselModel::default();
        assert!(model.geometry.is_ok());
        assert!(model.pressure.is_ok());
        assert_eq!(model.rows.len(), 4, "no external pressure by default - buckling must not be appended");
        assert!(model.thickness_outcome.is_some());
    }

    #[test]
    fn default_model_matches_the_shigley_thick_wall_reference_case() {
        // Same reference case `pressure-vessel-solver/src/stress.rs`'s own
        // test proves against Shigley's worked example - the default inputs
        // here (OD 6, wall 1 -> a=2,b=3; 5000 psi internal; closed ends)
        // are chosen to BE that case, so a UI-level regression that quietly
        // stopped calling the real solver (e.g. a hand-rolled reduced/
        // thin-wall formula) would be caught here too, not just in
        // `pressure-vessel-solver`'s own test suite.
        let model = PressureVesselModel::default();
        let yield_mode = model.rows.iter().find(|r| r.name == "Yield (maximum stress)").unwrap();
        assert!((yield_mode.applied - 13000.0).abs() < 1e-6, "expected the full Lame hoop stress (13000 psi), got {}", yield_mode.applied);
    }

    #[test]
    fn commit_number_updates_the_field_and_recomputes() {
        let mut model = PressureVesselModel::default();
        model.commit_number(NumberTarget::WallThickness, 0.5);
        assert_eq!(model.wall_thickness, 0.5);
        assert_eq!(model.geometry.unwrap().wall_thickness(), 0.5);
    }

    #[test]
    fn commit_number_silently_ignores_a_non_finite_result() {
        let mut model = PressureVesselModel::default();
        let before = model.outer_diameter;
        model.commit_number(NumberTarget::OuterDiameter, f64::NAN);
        assert_eq!(model.outer_diameter, before);
    }

    #[test]
    fn invalid_geometry_clears_rows_and_thickness_outcome_instead_of_panicking() {
        let mut model = PressureVesselModel::default();
        model.commit_number(NumberTarget::WallThickness, 10.0); // wall > outer radius -> inner_radius <= 0
        assert!(model.geometry.is_err());
        assert!(model.rows.is_empty());
        assert!(model.thickness_outcome.is_none());
    }

    #[test]
    fn toggle_end_condition_flips_and_recomputes() {
        let mut model = PressureVesselModel::default();
        assert!(model.closed_ends);
        model.toggle_end_condition();
        assert!(!model.closed_ends);
        assert_eq!(model.pressure.unwrap().end_condition, EndCondition::Open);
    }

    #[test]
    fn select_material_switches_and_recomputes() {
        let mut model = PressureVesselModel::default();
        let steel_index = MATERIALS.iter().position(|m| m.id == "steel").unwrap();
        model.select_material(steel_index);
        assert_eq!(model.material().id, "steel");
    }

    #[test]
    fn select_material_out_of_range_is_ignored() {
        let mut model = PressureVesselModel::default();
        let before = model.material_index;
        model.select_material(9999);
        assert_eq!(model.material_index, before);
    }

    #[test]
    fn external_pressure_with_unsupported_length_appends_a_fifth_buckling_row() {
        let mut model = PressureVesselModel::default();
        model.commit_number(NumberTarget::WallThickness, 0.05); // thin, D/t well above 40
        model.commit_number(NumberTarget::ExternalPressure, 50.0);
        model.commit_number(NumberTarget::UnsupportedLength, 10.0);
        assert_eq!(model.rows.len(), 5, "expected buckling to be appended once genuinely applicable");
        assert!(matches!(model.buckling, BucklingApplicability::Evaluated(_)));
    }

    #[test]
    fn zero_temperature_differential_is_not_applicable() {
        let model = PressureVesselModel::default();
        assert!(model.thermal_loading(model.material()).is_none());
    }

    #[test]
    fn nonzero_temperature_differential_changes_the_evaluated_rows() {
        let mut model = PressureVesselModel::default();
        let without = model.rows.clone();
        model.commit_number(NumberTarget::TemperatureDifferential, 300.0);
        assert!(model.thermal_loading(model.material()).is_some());
        assert_ne!(model.rows, without, "a real 300F differential must change the evaluated margins");
    }

    #[test]
    fn add_custom_material_appends_selects_and_is_used_by_recompute() {
        let mut model = PressureVesselModel::default();
        let builtin_count = mechanics_core::materials::builtin_len();
        model.add_custom_material("Unobtainium".to_string(), 99999.0, 5000.0, 6000.0, 0.25, 3.0);
        assert_eq!(model.material_catalog().len(), builtin_count + 1);
        assert_eq!(model.material_index, builtin_count);
        assert_eq!(model.material().name, "Unobtainium");
        // Recompute actually used it - the absurdly high yield strength
        // must produce a very different (much larger) margin than al7075's.
        let yield_mode = model.rows.iter().find(|r| r.name == "Yield (maximum stress)").unwrap();
        assert!(yield_mode.margin > 50.0, "expected a huge margin from the absurdly strong custom material, got {}", yield_mode.margin);
    }

    #[test]
    fn validation_hint_flags_non_positive_outer_diameter() {
        let mut model = PressureVesselModel::default();
        model.outer_diameter = 0.0;
        assert!(NumberTarget::OuterDiameter.validation_hint(&model).is_some());
        model.outer_diameter = 6.0;
        assert!(NumberTarget::OuterDiameter.validation_hint(&model).is_none());
    }

    #[test]
    fn validation_hint_flags_wall_thickness_too_large_for_outer_diameter() {
        let mut model = PressureVesselModel::default();
        assert!(NumberTarget::WallThickness.validation_hint(&model).is_none());
        model.wall_thickness = 10.0;
        assert!(NumberTarget::WallThickness.validation_hint(&model).is_some());
    }

    #[test]
    fn validation_hint_flags_negative_pressures_and_length() {
        let mut model = PressureVesselModel::default();
        model.internal_pressure = -1.0;
        assert!(NumberTarget::InternalPressure.validation_hint(&model).is_some());
        model.external_pressure = -1.0;
        assert!(NumberTarget::ExternalPressure.validation_hint(&model).is_some());
        model.unsupported_length = -1.0;
        assert!(NumberTarget::UnsupportedLength.validation_hint(&model).is_some());
    }

    #[test]
    fn validation_hint_never_flags_required_ms_or_temperature_differential() {
        let mut model = PressureVesselModel::default();
        model.required_ms = -5.0;
        model.temperature_differential = -500.0;
        assert!(NumberTarget::RequiredMs.validation_hint(&model).is_none());
        assert!(NumberTarget::TemperatureDifferential.validation_hint(&model).is_none());
    }

    #[test]
    fn format_for_edit_round_trips_through_parse() {
        let s = format_for_edit(0.25);
        assert_eq!(s.parse::<f64>().unwrap(), 0.25);
        assert_eq!(format_for_edit(0.0), "0");
    }
}
