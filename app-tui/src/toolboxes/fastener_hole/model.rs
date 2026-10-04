//! Bridges the pure `domain/` engine into UI-facing state: which
//! representation the user is currently editing in, which fields are
//! visible for the current hole type/solve-for selection, and the cached
//! derived results `view.rs` renders. Zero `ratatui`/`crossterm` here -
//! only `mod.rs` (key routing) and `view.rs` (rendering) know about the
//! terminal.

use super::domain::countersink::{self, AreaPreservationCheck, CountersinkGeometry, CountersinkInputs, CountersinkSolveFor, SecondaryCountersinkMethod};
use super::domain::regular_hole::{self, FitEnvelope, FitPreservationCheck};
use super::domain::surface_area;
use super::domain::{GeometryError, TolerancedValue};

/// Display precision (decimal places) for each dimension family - centralized
/// here rather than scattered through rendering code (spec section 5:
/// "make the display precision configurable or centralized rather than
/// hard-coded throughout the domain model"). Internal calculation values
/// are never rounded; only these constants govern how many digits a
/// rendered/edit-prefill string shows.
pub const LINEAR_DECIMALS: usize = 4;
pub const ANGLE_DECIMALS: usize = 3;
pub const AREA_DECIMALS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToleranceInputMode {
    #[default]
    NominalTol,
    MinMax,
}

impl ToleranceInputMode {
    pub fn cycle(self) -> Self {
        match self {
            ToleranceInputMode::NominalTol => ToleranceInputMode::MinMax,
            ToleranceInputMode::MinMax => ToleranceInputMode::NominalTol,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ToleranceInputMode::NominalTol => "Nominal +/-Tol",
            ToleranceInputMode::MinMax => "Min / Max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HoleType {
    #[default]
    Regular,
    Countersunk,
}

impl HoleType {
    pub fn cycle(self) -> Self {
        match self {
            HoleType::Regular => HoleType::Countersunk,
            HoleType::Countersunk => HoleType::Regular,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HoleType::Regular => "Regular",
            HoleType::Countersunk => "Countersunk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberTarget {
    RegularHole1,
    RegularHole2,
    RegularReference,
    CsOuterDiameter,
    CsHoleDiameter,
    CsDepth,
    CsAngle,
    CsSecondaryHoleDiameter,
}

impl NumberTarget {
    /// Whether this dimension is an angle (degrees) rather than a linear
    /// (inches) quantity - governs both display precision and which unit
    /// suffix `view.rs` shows (spec section 4: never conflate the two).
    pub fn is_angle(self) -> bool {
        matches!(self, NumberTarget::CsAngle)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberPart {
    Nominal,
    TolMinus,
    TolPlus,
    Min,
    Max,
}

impl NumberPart {
    pub fn suffix(self) -> &'static str {
        match self {
            NumberPart::Nominal => "Nominal",
            NumberPart::TolMinus => "-Tol",
            NumberPart::TolPlus => "+Tol",
            NumberPart::Min => "Min",
            NumberPart::Max => "Max",
        }
    }
}

/// One navigable/editable row, or a toggle row that cycles an enum on
/// Space/Enter - the shape `mod.rs`'s key routing and `view.rs`'s
/// rendering both walk. Rebuilt fresh from current state on every
/// keystroke/render rather than cached, mirroring how `settings_view.rs`
/// treats its own (static, in that case) `FIELDS` list as the single
/// source of truth for both concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    /// A non-selectable section divider - see `toolboxes/bushing/model.rs::FieldRow::Header`'s
    /// doc comment for the same reasoning, applied here.
    Header(&'static str),
    ToggleHoleType,
    ToggleToleranceMode,
    ToggleSolveFor,
    ToggleSecondaryMethod,
    Number(NumberTarget, NumberPart, &'static str),
}

/// One-line description shown in the bottom "Hint" panel while this row is
/// selected - same purpose as `toolboxes/bushing/model.rs::field_hint`.
pub fn field_hint(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(_) => "",
        FieldRow::ToggleHoleType => "Regular (two plain round holes) or Countersunk (a flared entry over a straight hole).",
        FieldRow::ToggleToleranceMode => "How each dimension below is entered - a nominal value with +/- tolerances, or an explicit min/max pair.",
        FieldRow::ToggleSolveFor => "Which countersink dimension is calculated from the other three - that dimension never appears as an editable row.",
        FieldRow::ToggleSecondaryMethod => "How the secondary countersink's depth/angle is derived from the primary: preserve depth (recompute angle) or preserve lateral surface area (recompute depth and angle together).",
        FieldRow::Number(NumberTarget::RegularHole1, _, _) => "First hole's diameter.",
        FieldRow::Number(NumberTarget::RegularHole2, _, _) => "Second hole's diameter - compared against the first to determine the resulting fit.",
        FieldRow::Number(NumberTarget::RegularReference, _, _) => "Secondary reference hole diameter, checked against the derived fit for consistency.",
        FieldRow::Number(NumberTarget::CsOuterDiameter, _, _) => "Countersink outer (flared) diameter.",
        FieldRow::Number(NumberTarget::CsHoleDiameter, _, _) => "Straight-hole diameter below the countersink.",
        FieldRow::Number(NumberTarget::CsDepth, _, _) => "Countersink depth from the surface to the straight-hole transition.",
        FieldRow::Number(NumberTarget::CsAngle, _, _) => "Countersink included angle.",
        FieldRow::Number(NumberTarget::CsSecondaryHoleDiameter, _, _) => "Secondary (opposite-face) countersink's hole diameter - its depth/angle are derived per the Secondary Method above.",
    }
}

fn dimension_rows(mode: ToleranceInputMode, target: NumberTarget, label: &'static str) -> Vec<FieldRow> {
    match mode {
        ToleranceInputMode::NominalTol => {
            vec![FieldRow::Number(target, NumberPart::Nominal, label), FieldRow::Number(target, NumberPart::TolMinus, label), FieldRow::Number(target, NumberPart::TolPlus, label)]
        }
        ToleranceInputMode::MinMax => vec![FieldRow::Number(target, NumberPart::Min, label), FieldRow::Number(target, NumberPart::Max, label)],
    }
}

/// The complete navigable row list for the current hole type/solve-for
/// selection. Only INPUT dimensions ever appear here - a CALCULATED/
/// TRANSFERRED/PRESERVED value is never added to this list, so it can
/// never receive editing focus (spec section 32).
pub fn field_rows(model: &FastenerHoleModel) -> Vec<FieldRow> {
    let mut rows = vec![FieldRow::Header("Hole Setup"), FieldRow::ToggleHoleType, FieldRow::ToggleToleranceMode];
    match model.hole_type {
        HoleType::Regular => {
            rows.push(FieldRow::Header("Regular Hole Dimensions"));
            rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::RegularHole1, "Hole 1 Diameter"));
            rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::RegularHole2, "Hole 2 Diameter"));
            rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::RegularReference, "Secondary Reference Hole"));
        }
        HoleType::Countersunk => {
            rows.push(FieldRow::Header("Countersink Geometry"));
            rows.push(FieldRow::ToggleSolveFor);
            let cs = &model.countersink;
            if cs.solve_for != CountersinkSolveFor::OuterDiameter {
                rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::CsOuterDiameter, "Outer Diameter"));
            }
            if cs.solve_for != CountersinkSolveFor::HoleDiameter {
                rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::CsHoleDiameter, "Hole Diameter"));
            }
            if cs.solve_for != CountersinkSolveFor::Depth {
                rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::CsDepth, "Depth"));
            }
            if cs.solve_for != CountersinkSolveFor::Angle {
                rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::CsAngle, "Angle"));
            }
            rows.push(FieldRow::Header("Secondary Countersink"));
            rows.push(FieldRow::ToggleSecondaryMethod);
            rows.extend(dimension_rows(model.tolerance_mode, NumberTarget::CsSecondaryHoleDiameter, "Secondary Hole Diameter"));
        }
    }
    rows
}

fn apply_part(current: TolerancedValue, part: NumberPart, raw: f64) -> Result<TolerancedValue, GeometryError> {
    match part {
        NumberPart::Nominal => TolerancedValue::from_nominal_tol(raw, current.tol_minus(), current.tol_plus()),
        NumberPart::TolMinus => TolerancedValue::from_nominal_tol(current.nominal, raw, current.tol_plus()),
        NumberPart::TolPlus => TolerancedValue::from_nominal_tol(current.nominal, current.tol_minus(), raw),
        NumberPart::Min => TolerancedValue::from_min_max(raw, current.max),
        NumberPart::Max => TolerancedValue::from_min_max(current.min, raw),
    }
}

pub fn part_value(value: &TolerancedValue, part: NumberPart) -> f64 {
    match part {
        NumberPart::Nominal => value.nominal,
        NumberPart::TolMinus => value.tol_minus(),
        NumberPart::TolPlus => value.tol_plus(),
        NumberPart::Min => value.min,
        NumberPart::Max => value.max,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegularHoleUi {
    pub hole1: TolerancedValue,
    pub hole2: TolerancedValue,
    pub reference: TolerancedValue,
}

impl Default for RegularHoleUi {
    fn default() -> Self {
        let quarter_inch = TolerancedValue::exact(0.250).unwrap();
        Self { hole1: quarter_inch, hole2: quarter_inch, reference: quarter_inch }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CountersinkUi {
    pub solve_for: CountersinkSolveFor,
    pub outer: TolerancedValue,
    pub hole: TolerancedValue,
    pub depth: TolerancedValue,
    pub angle: TolerancedValue,
    pub secondary_method: SecondaryCountersinkMethod,
    pub secondary_hole: TolerancedValue,
}

impl Default for CountersinkUi {
    fn default() -> Self {
        Self {
            solve_for: CountersinkSolveFor::Angle,
            outer: TolerancedValue::exact(0.500).unwrap(),
            hole: TolerancedValue::exact(0.250).unwrap(),
            depth: TolerancedValue::exact(0.125).unwrap(),
            angle: TolerancedValue::exact(90.0).unwrap(),
            secondary_method: SecondaryCountersinkMethod::default(),
            secondary_hole: TolerancedValue::exact(0.3125).unwrap(),
        }
    }
}

fn countersink_inputs(cs: &CountersinkUi) -> CountersinkInputs {
    let mut inputs =
        CountersinkInputs { solve_for: cs.solve_for, outer_diameter: Some(cs.outer), hole_diameter: Some(cs.hole), depth: Some(cs.depth), angle_deg: Some(cs.angle) };
    match cs.solve_for {
        CountersinkSolveFor::OuterDiameter => inputs.outer_diameter = None,
        CountersinkSolveFor::HoleDiameter => inputs.hole_diameter = None,
        CountersinkSolveFor::Depth => inputs.depth = None,
        CountersinkSolveFor::Angle => inputs.angle_deg = None,
    }
    inputs
}

/// The whole toolbox's engineering state, plus every derived result -
/// recomputed fresh on every field mutation via [`FastenerHoleModel::recompute`]
/// (cheap pure-function calculation, no reason to cache staler results -
/// spec section 48's performance guidance is about avoiding recalculation
/// *from rendering functions*, not about avoiding it on genuine state
/// changes).
pub struct FastenerHoleModel {
    pub hole_type: HoleType,
    pub tolerance_mode: ToleranceInputMode,
    pub regular: RegularHoleUi,
    pub countersink: CountersinkUi,

    pub regular_fit: FitEnvelope,
    pub regular_secondary: Result<(TolerancedValue, FitPreservationCheck), GeometryError>,

    pub countersink_primary: Result<CountersinkGeometry, GeometryError>,
    pub countersink_primary_area: Result<TolerancedValue, GeometryError>,
    pub countersink_secondary: Result<CountersinkGeometry, GeometryError>,
    pub countersink_secondary_area: Result<TolerancedValue, GeometryError>,
    pub countersink_area_check: Option<AreaPreservationCheck>,
}

impl Default for FastenerHoleModel {
    fn default() -> Self {
        let mut model = Self {
            hole_type: HoleType::default(),
            tolerance_mode: ToleranceInputMode::default(),
            regular: RegularHoleUi::default(),
            countersink: CountersinkUi::default(),
            regular_fit: regular_hole::compute_fit(&RegularHoleUi::default().hole1, &RegularHoleUi::default().hole2),
            regular_secondary: Err(GeometryError::UnderdeterminedGeometry),
            countersink_primary: Err(GeometryError::UnderdeterminedGeometry),
            countersink_primary_area: Err(GeometryError::UnderdeterminedGeometry),
            countersink_secondary: Err(GeometryError::UnderdeterminedGeometry),
            countersink_secondary_area: Err(GeometryError::UnderdeterminedGeometry),
            countersink_area_check: None,
        };
        model.recompute();
        model
    }
}

impl FastenerHoleModel {
    pub fn get_toleranced(&self, target: NumberTarget) -> TolerancedValue {
        match target {
            NumberTarget::RegularHole1 => self.regular.hole1,
            NumberTarget::RegularHole2 => self.regular.hole2,
            NumberTarget::RegularReference => self.regular.reference,
            NumberTarget::CsOuterDiameter => self.countersink.outer,
            NumberTarget::CsHoleDiameter => self.countersink.hole,
            NumberTarget::CsDepth => self.countersink.depth,
            NumberTarget::CsAngle => self.countersink.angle,
            NumberTarget::CsSecondaryHoleDiameter => self.countersink.secondary_hole,
        }
    }

    fn set_toleranced(&mut self, target: NumberTarget, value: TolerancedValue) {
        match target {
            NumberTarget::RegularHole1 => self.regular.hole1 = value,
            NumberTarget::RegularHole2 => self.regular.hole2 = value,
            NumberTarget::RegularReference => self.regular.reference = value,
            NumberTarget::CsOuterDiameter => self.countersink.outer = value,
            NumberTarget::CsHoleDiameter => self.countersink.hole = value,
            NumberTarget::CsDepth => self.countersink.depth = value,
            NumberTarget::CsAngle => self.countersink.angle = value,
            NumberTarget::CsSecondaryHoleDiameter => self.countersink.secondary_hole = value,
        }
    }

    /// Commits one edited numeric sub-value. Malformed input never reaches
    /// here at all (the caller only calls this after a successful
    /// `str::parse::<f64>()`); a value that parses but produces an invalid
    /// `TolerancedValue` (e.g. non-finite) is silently ignored, leaving the
    /// previous value in place - the same "invalid numeric input is
    /// silently ignored" convention `settings_view.rs` already uses.
    pub fn commit_number(&mut self, target: NumberTarget, part: NumberPart, raw: f64) {
        let current = self.get_toleranced(target);
        if let Ok(updated) = apply_part(current, part, raw) {
            self.set_toleranced(target, updated);
            self.recompute();
        }
    }

    /// Recomputes every derived result from current input state. Called
    /// after every mutation, never from a render path (spec section 48).
    pub fn recompute(&mut self) {
        self.regular_fit = regular_hole::compute_fit(&self.regular.hole1, &self.regular.hole2);
        self.regular_secondary = regular_hole::derive_and_verify(&self.regular_fit, &self.regular.reference);

        self.countersink_primary = countersink::solve(&countersink_inputs(&self.countersink));
        self.countersink_primary_area = match &self.countersink_primary {
            Ok(geometry) => surface_area::lateral_surface_area_toleranced(&geometry.outer_diameter, &geometry.hole_diameter, &geometry.angle_deg),
            Err(e) => Err(*e),
        };

        match &self.countersink_primary {
            Ok(primary) => match self.countersink.secondary_method {
                SecondaryCountersinkMethod::PreserveDepth => {
                    self.countersink_secondary = countersink::secondary_preserve_depth(primary, self.countersink.secondary_hole);
                    self.countersink_area_check = None;
                    self.countersink_secondary_area = match &self.countersink_secondary {
                        Ok(geometry) => surface_area::lateral_surface_area_toleranced(&geometry.outer_diameter, &geometry.hole_diameter, &geometry.angle_deg),
                        Err(e) => Err(*e),
                    };
                }
                SecondaryCountersinkMethod::PreserveLateralArea => {
                    match countersink::secondary_preserve_area(primary, &self.countersink.secondary_hole) {
                        Ok((geometry, check)) => {
                            self.countersink_secondary = Ok(geometry);
                            self.countersink_secondary_area = surface_area::lateral_surface_area_toleranced(&geometry.outer_diameter, &geometry.hole_diameter, &geometry.angle_deg);
                            self.countersink_area_check = Some(check);
                        }
                        Err(e) => {
                            self.countersink_secondary = Err(e);
                            self.countersink_secondary_area = Err(e);
                            self.countersink_area_check = None;
                        }
                    }
                }
            },
            Err(e) => {
                self.countersink_secondary = Err(*e);
                self.countersink_secondary_area = Err(*e);
                self.countersink_area_check = None;
            }
        }
    }
}

/// Trims a fixed-decimal formatted number down for the edit buffer so
/// re-editing a value doesn't show a wall of trailing zeros, while still
/// round-tripping exactly through `str::parse::<f64>()`.
pub fn format_for_edit(value: f64) -> String {
    let s = format!("{value:.6}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() { "0".to_string() } else { trimmed.to_string() }
}

pub fn format_linear(value: f64) -> String {
    format!("{value:.LINEAR_DECIMALS$}")
}

pub fn format_angle(value: f64) -> String {
    format!("{value:.ANGLE_DECIMALS$}")
}

pub fn format_area(value: f64) -> String {
    format!("{value:.AREA_DECIMALS$}")
}

/// Compact toleranced form `nominal -minus/+plus` (e.g. `0.2500 -0.0020/+0.0040`),
/// formatting every part with `fmt`. Used for every calculated value so the
/// tolerance band sits next to the nominal instead of on extra lines.
pub fn format_band(nominal: f64, min: f64, max: f64, fmt: impl Fn(f64) -> String) -> String {
    format!("{} -{}/+{}", fmt(nominal), fmt((nominal - min).max(0.0)), fmt((max - nominal).max(0.0)))
}

pub fn format_linear_band(v: &TolerancedValue) -> String {
    format_band(v.nominal, v.min, v.max, format_linear)
}

pub fn format_angle_band(v: &TolerancedValue) -> String {
    format_band(v.nominal, v.min, v.max, format_angle)
}

pub fn format_area_band(v: &TolerancedValue) -> String {
    format_band(v.nominal, v.min, v.max, format_area)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_computes_a_transition_fit_for_two_equal_default_holes() {
        let model = FastenerHoleModel::default();
        assert_eq!(model.regular_fit.nominal, 0.0);
    }

    #[test]
    fn default_model_solves_a_valid_countersink_geometry() {
        let model = FastenerHoleModel::default();
        let geometry = model.countersink_primary.expect("default countersink geometry must be valid");
        assert!((geometry.angle_deg.nominal - 90.0).abs() < 1e-6);
        assert!(model.countersink_primary_area.is_ok());
    }

    #[test]
    fn field_rows_only_lists_the_three_direct_inputs_for_the_current_solve_for() {
        let mut model = FastenerHoleModel::default();
        model.hole_type = HoleType::Countersunk;
        model.countersink.solve_for = CountersinkSolveFor::Depth;
        let rows = field_rows(&model);
        let numbers: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                FieldRow::Number(target, _, _) => Some(*target),
                _ => None,
            })
            .collect();
        assert!(numbers.contains(&NumberTarget::CsOuterDiameter));
        assert!(numbers.contains(&NumberTarget::CsHoleDiameter));
        assert!(numbers.contains(&NumberTarget::CsAngle));
        assert!(!numbers.contains(&NumberTarget::CsDepth), "the solved dimension must never be an editable row");
    }

    #[test]
    fn min_max_mode_produces_two_rows_per_dimension_instead_of_three() {
        let mut model = FastenerHoleModel::default();
        model.tolerance_mode = ToleranceInputMode::MinMax;
        let rows = field_rows(&model);
        let hole1_rows = rows.iter().filter(|r| matches!(r, FieldRow::Number(NumberTarget::RegularHole1, _, _))).count();
        assert_eq!(hole1_rows, 2);
    }

    #[test]
    fn commit_number_updates_nominal_and_recomputes_the_fit() {
        let mut model = FastenerHoleModel::default();
        model.commit_number(NumberTarget::RegularHole2, NumberPart::Nominal, 0.260);
        assert!((model.regular.hole2.nominal - 0.260).abs() < 1e-9);
        assert!((model.regular_fit.nominal - 0.010).abs() < 1e-9);
    }

    #[test]
    fn commit_number_silently_ignores_a_non_finite_result() {
        let mut model = FastenerHoleModel::default();
        let before = model.regular.hole1;
        model.commit_number(NumberTarget::RegularHole1, NumberPart::Nominal, f64::NAN);
        assert_eq!(model.regular.hole1, before);
    }

    #[test]
    fn toggling_secondary_method_switches_between_preserve_depth_and_preserve_area_behavior() {
        let mut model = FastenerHoleModel::default();
        model.hole_type = HoleType::Countersunk;
        model.countersink.secondary_method = SecondaryCountersinkMethod::PreserveDepth;
        model.recompute();
        assert!(model.countersink_area_check.is_none());

        model.countersink.secondary_method = SecondaryCountersinkMethod::PreserveLateralArea;
        model.recompute();
        assert!(model.countersink_area_check.is_some());
    }

    #[test]
    fn format_band_puts_minus_and_plus_tolerance_next_to_nominal() {
        assert_eq!(format_band(0.25, 0.248, 0.254, format_linear), "0.2500 -0.0020/+0.0040");
    }

    #[test]
    fn format_for_edit_round_trips_through_parse() {
        let s = format_for_edit(0.25);
        assert_eq!(s.parse::<f64>().unwrap(), 0.25);
        assert_eq!(format_for_edit(0.0), "0");
    }
}
