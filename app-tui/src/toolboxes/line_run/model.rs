//! Line Run model: the inputs of a supported pressure tube (tube, material, supports, point loads, load cases), the
//! unit system they are shown in, and the bridge to `pressure_vessel_solver::line_run`. No ratatui, no window code.
//!
//! Inputs are stored in the solver's units (in, lbf, psi, degF) and shown in `Units`; a typed value is converted back
//! once, so the solve never sees display units. Material constants and allowables are `Option`: nothing is defaulted
//! (owner rule), and a run without them says which one is missing instead of producing a number.
use pressure_vessel_solver::line_run::{self as lr, LineResult, LoadCase, MaterialClass, PointLoad, Support, Tube};
use serde::{Deserialize, Serialize};

/// Stations per segment handed to the solver (peak moments are exact for any value).
pub const SUBDIVISIONS: usize = lr::DEFAULT_SUBDIVISIONS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Units {
    /// in, lbf, psi, degF.
    #[default]
    Us,
    /// mm, N, MPa, degC.
    Si,
}

/// What a number measures: picks the display factor and label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qty {
    Length,
    Force,
    Stress,
    Moment,
    ForcePerLength,
    Density,
    Alpha,
    /// A temperature difference (not an absolute temperature).
    DeltaT,
    Factor,
}

const LBF: f64 = 4.448_221_615_260_5;
const INCH: f64 = 25.4;

impl Units {
    pub fn other(self) -> Self {
        match self {
            Units::Us => Units::Si,
            Units::Si => Units::Us,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Units::Us => "in, lbf, psi",
            Units::Si => "mm, N, MPa",
        }
    }
    /// Display value per solver unit.
    pub fn factor(self, q: Qty) -> f64 {
        match self {
            Units::Us => 1.0,
            Units::Si => match q {
                Qty::Length => INCH,
                Qty::Force => LBF,
                Qty::Stress => 6.894_757_293_168_361e-3,
                Qty::Moment => LBF * INCH,
                Qty::ForcePerLength => LBF / INCH,
                Qty::Density => LBF / (INCH * INCH * INCH),
                Qty::Alpha => 1.8,
                Qty::DeltaT => 5.0 / 9.0,
                Qty::Factor => 1.0,
            },
        }
    }
    pub fn label(self, q: Qty) -> &'static str {
        match (self, q) {
            (_, Qty::Factor) => "",
            (Units::Us, Qty::Length) => "in",
            (Units::Us, Qty::Force) => "lbf",
            (Units::Us, Qty::Stress) => "psi",
            (Units::Us, Qty::Moment) => "in\u{b7}lbf",
            (Units::Us, Qty::ForcePerLength) => "lbf/in",
            (Units::Us, Qty::Density) => "lbf/in\u{b3}",
            (Units::Us, Qty::Alpha) => "1/\u{b0}F",
            (Units::Us, Qty::DeltaT) => "\u{b0}F",
            (Units::Si, Qty::Length) => "mm",
            (Units::Si, Qty::Force) => "N",
            (Units::Si, Qty::Stress) => "MPa",
            (Units::Si, Qty::Moment) => "N\u{b7}mm",
            (Units::Si, Qty::ForcePerLength) => "N/mm",
            (Units::Si, Qty::Density) => "N/mm\u{b3}",
            (Units::Si, Qty::Alpha) => "1/\u{b0}C",
            (Units::Si, Qty::DeltaT) => "\u{b0}C",
        }
    }
    /// `v` (solver units) as display text with its unit, e.g. `12.7 mm`.
    pub fn text(self, q: Qty, v: f64) -> String {
        let unit = self.label(q);
        let n = fmt_number(v * self.factor(q));
        if unit.is_empty() {
            n
        } else {
            format!("{n} {unit}")
        }
    }
}

/// A number for reading (five significant digits): plain when moderate, scientific when tiny or huge, trailing zeros dropped.
pub fn fmt_number(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if !v.is_finite() {
        "\u{2014}".into()
    } else if v.abs() < 1e-3 || v.abs() >= 1e7 {
        format!("{v:.4e}")
    } else {
        // Five significant digits.
        let decimals = if v.abs() >= 1.0 { (4 - v.abs().log10().floor() as i32).max(0) as usize } else { 5 };
        let s = format!("{v:.decimals$}");
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
    }
}

/// A number as the text an edit box starts with: shortest form that parses back to (nearly) the same value.
pub fn format_for_edit(v: f64) -> String {
    if v != 0.0 && (v.abs() < 1e-3 || v.abs() >= 1e7) {
        format!("{v:.6e}")
    } else {
        let s = format!("{v:.8}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Class {
    #[default]
    Ductile,
    Brittle,
}
impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Class::Ductile => "Ductile",
            Class::Brittle => "Brittle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupportRow {
    pub x: f64,
    pub anchor: bool,
    pub clamp: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LoadRow {
    pub x: f64,
    /// Lumped weight, scaled by the case's load factors.
    pub weight: f64,
    pub force: [f64; 3],
    /// Arm from the line axis to the load's line of action (valve centre of gravity).
    pub offset: [f64; 3],
    /// The force already contains the pressure thrust (a flow-thrust result).
    pub thrust: bool,
    /// The load case it belongs to; `None`: every case (hardware).
    pub case: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseRow {
    pub name: String,
    pub internal_pressure: f64,
    pub external_pressure: f64,
    pub end_load: bool,
    pub delta_t: f64,
    pub factors: [f64; 3],
}

/// Everything the person enters (what a case file stores). Results are never stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inputs {
    pub units: Units,
    pub od: f64,
    pub wall: f64,
    pub e: Option<f64>,
    pub alpha: Option<f64>,
    pub density: Option<f64>,
    pub contents: f64,
    pub insulation: f64,
    pub x_start: f64,
    pub x_end: f64,
    pub class: Class,
    pub fty: Option<f64>,
    pub ftu: Option<f64>,
    pub fcu: Option<f64>,
    pub fs_yield: Option<f64>,
    pub fs_ultimate: Option<f64>,
    pub fs_brittle: Option<f64>,
    pub supports: Vec<SupportRow>,
    pub loads: Vec<LoadRow>,
    pub cases: Vec<CaseRow>,
    pub selected: usize,
}

impl Default for Inputs {
    /// A neutral starting layout: a half-inch tube on two supports under one g. No material constant or allowable.
    fn default() -> Self {
        Self {
            units: Units::Us,
            od: 0.5,
            wall: 0.035,
            e: None,
            alpha: None,
            density: None,
            contents: 0.0,
            insulation: 0.0,
            x_start: 0.0,
            x_end: 24.0,
            class: Class::Ductile,
            fty: None,
            ftu: None,
            fcu: None,
            fs_yield: None,
            fs_ultimate: None,
            fs_brittle: None,
            supports: vec![SupportRow { x: 0.0, anchor: true, clamp: false }, SupportRow { x: 24.0, anchor: false, clamp: false }],
            loads: Vec::new(),
            cases: vec![CaseRow::one_g()],
            selected: 0,
        }
    }
}

impl CaseRow {
    pub fn one_g() -> Self {
        Self { name: "1 g down".into(), internal_pressure: 0.0, external_pressure: 0.0, end_load: false, delta_t: 0.0, factors: [0.0, 0.0, -1.0] }
    }
}

/// One editable number. Indexed variants name a table row; the case ones act on the selected case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Od,
    Wall,
    E,
    Alpha,
    Density,
    Contents,
    Insulation,
    XStart,
    XEnd,
    Fty,
    Ftu,
    Fcu,
    FsYield,
    FsUltimate,
    FsBrittle,
    SupportX(usize),
    LoadX(usize),
    LoadWeight(usize),
    LoadForce(usize, u8),
    LoadOffset(usize, u8),
    CaseInternal,
    CaseExternal,
    CaseDeltaT,
    CaseFactor(u8),
}

const AXES: [&str; 3] = ["x", "y", "z"];

impl Field {
    pub fn qty(self) -> Qty {
        match self {
            Field::Od | Field::Wall | Field::XStart | Field::XEnd | Field::SupportX(_) | Field::LoadX(_) | Field::LoadOffset(..) => Qty::Length,
            Field::E | Field::Fty | Field::Ftu | Field::Fcu | Field::CaseInternal | Field::CaseExternal => Qty::Stress,
            Field::Alpha => Qty::Alpha,
            Field::Density => Qty::Density,
            Field::Contents | Field::Insulation => Qty::ForcePerLength,
            Field::FsYield | Field::FsUltimate | Field::FsBrittle | Field::CaseFactor(_) => Qty::Factor,
            Field::LoadWeight(_) | Field::LoadForce(..) => Qty::Force,
            Field::CaseDeltaT => Qty::DeltaT,
        }
    }
    pub fn label(self) -> String {
        let axis = |a: u8| AXES[a.min(2) as usize];
        match self {
            Field::Od => "Outer diameter".into(),
            Field::Wall => "Wall".into(),
            Field::E => "Elastic modulus E".into(),
            Field::Alpha => "Expansion coefficient".into(),
            Field::Density => "Weight density".into(),
            Field::Contents => "Contents weight per length".into(),
            Field::Insulation => "Insulation weight per length".into(),
            Field::XStart => "Line start x".into(),
            Field::XEnd => "Line end x".into(),
            Field::Fty => "Yield Fty".into(),
            Field::Ftu => "Ultimate Ftu".into(),
            Field::Fcu => "Compressive ultimate Fcu".into(),
            Field::FsYield => "Factor of safety, yield".into(),
            Field::FsUltimate => "Factor of safety, ultimate".into(),
            Field::FsBrittle => "Factor of safety".into(),
            Field::SupportX(i) => format!("Support {} x", i + 1),
            Field::LoadX(i) => format!("Load {} x", i + 1),
            Field::LoadWeight(i) => format!("Load {} weight", i + 1),
            Field::LoadForce(i, a) => format!("Load {} force {}", i + 1, axis(a)),
            Field::LoadOffset(i, a) => format!("Load {} offset {}", i + 1, axis(a)),
            Field::CaseInternal => "Internal pressure".into(),
            Field::CaseExternal => "External pressure".into(),
            Field::CaseDeltaT => "Temperature change".into(),
            Field::CaseFactor(a) => format!("Load factor {}", axis(a)),
        }
    }
}

pub struct LineRunModel {
    pub inputs: Inputs,
    /// The solve of `inputs`, or why there is none (a missing constant, a refused request, a mechanism).
    pub outcome: Result<LineResult, String>,
}

impl Default for LineRunModel {
    fn default() -> Self {
        Self::new(Inputs::default())
    }
}

impl LineRunModel {
    pub fn new(inputs: Inputs) -> Self {
        let mut m = Self { outcome: Err(String::new()), inputs };
        m.fix_selection();
        m.recompute();
        m
    }
    pub fn recompute(&mut self) {
        self.outcome = self.solve();
    }
    pub fn set_inputs(&mut self, inputs: Inputs) {
        self.inputs = inputs;
        self.fix_selection();
        self.recompute();
    }
    fn fix_selection(&mut self) {
        if self.inputs.cases.is_empty() {
            self.inputs.cases.push(CaseRow::one_g());
        }
        self.inputs.selected = self.inputs.selected.min(self.inputs.cases.len() - 1);
    }
    pub fn units(&self) -> Units {
        self.inputs.units
    }
    pub fn selected_case(&self) -> &CaseRow {
        &self.inputs.cases[self.inputs.selected]
    }

    // ------------------------------------------------------------------ numbers

    /// The value in solver units, or `None` for an optional constant not entered yet (or a stale index).
    pub fn get(&self, f: Field) -> Option<f64> {
        let i = &self.inputs;
        let c = self.selected_case();
        match f {
            Field::Od => Some(i.od),
            Field::Wall => Some(i.wall),
            Field::E => i.e,
            Field::Alpha => i.alpha,
            Field::Density => i.density,
            Field::Contents => Some(i.contents),
            Field::Insulation => Some(i.insulation),
            Field::XStart => Some(i.x_start),
            Field::XEnd => Some(i.x_end),
            Field::Fty => i.fty,
            Field::Ftu => i.ftu,
            Field::Fcu => i.fcu,
            Field::FsYield => i.fs_yield,
            Field::FsUltimate => i.fs_ultimate,
            Field::FsBrittle => i.fs_brittle,
            Field::SupportX(k) => i.supports.get(k).map(|s| s.x),
            Field::LoadX(k) => i.loads.get(k).map(|l| l.x),
            Field::LoadWeight(k) => i.loads.get(k).map(|l| l.weight),
            Field::LoadForce(k, a) => i.loads.get(k).map(|l| l.force[a.min(2) as usize]),
            Field::LoadOffset(k, a) => i.loads.get(k).map(|l| l.offset[a.min(2) as usize]),
            Field::CaseInternal => Some(c.internal_pressure),
            Field::CaseExternal => Some(c.external_pressure),
            Field::CaseDeltaT => Some(c.delta_t),
            Field::CaseFactor(a) => Some(c.factors[a.min(2) as usize]),
        }
    }
    /// Whether `f` names something that exists (a table row still there).
    pub fn exists(&self, f: Field) -> bool {
        match f {
            Field::SupportX(k) => k < self.inputs.supports.len(),
            Field::LoadX(k) | Field::LoadWeight(k) | Field::LoadForce(k, _) | Field::LoadOffset(k, _) => k < self.inputs.loads.len(),
            _ => true,
        }
    }
    /// The value as shown (display units), `None` when not entered.
    pub fn shown(&self, f: Field) -> Option<f64> {
        self.get(f).map(|v| v * self.units().factor(f.qty()))
    }
    /// Store `display` (typed in the shown units). False for a non-finite number or a missing row.
    pub fn set(&mut self, f: Field, display: f64) -> bool {
        if !display.is_finite() || !self.exists(f) {
            return false;
        }
        let v = display / self.units().factor(f.qty());
        let sel = self.inputs.selected;
        let i = &mut self.inputs;
        match f {
            Field::Od => i.od = v,
            Field::Wall => i.wall = v,
            Field::E => i.e = Some(v),
            Field::Alpha => i.alpha = Some(v),
            Field::Density => i.density = Some(v),
            Field::Contents => i.contents = v,
            Field::Insulation => i.insulation = v,
            Field::XStart => i.x_start = v,
            Field::XEnd => i.x_end = v,
            Field::Fty => i.fty = Some(v),
            Field::Ftu => i.ftu = Some(v),
            Field::Fcu => i.fcu = Some(v),
            Field::FsYield => i.fs_yield = Some(v),
            Field::FsUltimate => i.fs_ultimate = Some(v),
            Field::FsBrittle => i.fs_brittle = Some(v),
            Field::SupportX(k) => i.supports[k].x = v,
            Field::LoadX(k) => i.loads[k].x = v,
            Field::LoadWeight(k) => i.loads[k].weight = v,
            Field::LoadForce(k, a) => i.loads[k].force[a.min(2) as usize] = v,
            Field::LoadOffset(k, a) => i.loads[k].offset[a.min(2) as usize] = v,
            Field::CaseInternal => i.cases[sel].internal_pressure = v,
            Field::CaseExternal => i.cases[sel].external_pressure = v,
            Field::CaseDeltaT => i.cases[sel].delta_t = v,
            Field::CaseFactor(a) => i.cases[sel].factors[a.min(2) as usize] = v,
        }
        self.recompute();
        true
    }
    /// A typed value that equals the current one (to the precision the box shows) is not an edit.
    pub fn same(&self, f: Field, display: f64) -> bool {
        self.shown(f).is_some_and(|s| (s - display).abs() <= 1e-9 * s.abs().max(display.abs()).max(1e-300))
    }

    // ------------------------------------------------------------------ tables and toggles

    pub fn toggle_class(&mut self) {
        self.inputs.class = match self.inputs.class {
            Class::Ductile => Class::Brittle,
            Class::Brittle => Class::Ductile,
        };
        self.recompute();
    }
    pub fn toggle_units(&mut self) {
        self.inputs.units = self.inputs.units.other();
    }
    pub fn toggle_anchor(&mut self, k: usize) {
        if let Some(s) = self.inputs.supports.get_mut(k) {
            s.anchor = !s.anchor;
            self.recompute();
        }
    }
    pub fn toggle_clamp(&mut self, k: usize) {
        if let Some(s) = self.inputs.supports.get_mut(k) {
            s.clamp = !s.clamp;
            self.recompute();
        }
    }
    /// A support past the last one (one tenth of the line further, or on the end).
    pub fn add_support(&mut self) {
        let i = &mut self.inputs;
        let x = i.supports.iter().map(|s| s.x).fold(i.x_start, f64::max);
        i.supports.push(SupportRow { x: (x + 0.1 * (i.x_end - i.x_start)).min(i.x_end), anchor: false, clamp: false });
        self.recompute();
    }
    pub fn remove_support(&mut self, k: usize) {
        if k < self.inputs.supports.len() {
            self.inputs.supports.remove(k);
            self.recompute();
        }
    }
    /// A load at midspan, applied in every case until its scope is set (`cycle_scope`).
    pub fn add_load(&mut self) {
        let x = 0.5 * (self.inputs.x_start + self.inputs.x_end);
        self.inputs.loads.push(LoadRow { x, ..LoadRow::default() });
        self.recompute();
    }
    pub fn remove_load(&mut self, k: usize) {
        if k < self.inputs.loads.len() {
            self.inputs.loads.remove(k);
            self.recompute();
        }
    }
    pub fn toggle_thrust(&mut self, k: usize) {
        if let Some(l) = self.inputs.loads.get_mut(k) {
            l.thrust = !l.thrust;
            self.recompute();
        }
    }
    /// Every case, then case 1, case 2, ... and back.
    pub fn cycle_scope(&mut self, k: usize) {
        let n = self.inputs.cases.len();
        if let Some(l) = self.inputs.loads.get_mut(k) {
            l.case = match l.case {
                None => Some(0),
                Some(c) if c + 1 < n => Some(c + 1),
                Some(_) => None,
            };
            self.recompute();
        }
    }
    pub fn scope_label(&self, l: &LoadRow) -> String {
        match l.case.and_then(|c| self.inputs.cases.get(c)) {
            Some(c) => c.name.clone(),
            None => "every case".into(),
        }
    }
    pub fn toggle_end_load(&mut self) {
        let sel = self.inputs.selected;
        self.inputs.cases[sel].end_load = !self.inputs.cases[sel].end_load;
        self.recompute();
    }
    pub fn select_case(&mut self, k: usize) {
        if k < self.inputs.cases.len() {
            self.inputs.selected = k;
        }
    }
    pub fn add_case(&mut self) {
        let n = self.inputs.cases.len() + 1;
        let mut c = CaseRow::one_g();
        c.name = format!("Case {n}");
        self.inputs.cases.push(c);
        self.inputs.selected = self.inputs.cases.len() - 1;
        self.recompute();
    }
    /// Remove a case and its own loads; the last case cannot go.
    pub fn remove_case(&mut self, k: usize) {
        if self.inputs.cases.len() < 2 || k >= self.inputs.cases.len() {
            return;
        }
        self.inputs.cases.remove(k);
        self.inputs.loads.retain(|l| l.case != Some(k));
        for l in &mut self.inputs.loads {
            if let Some(c) = &mut l.case {
                if *c > k {
                    *c -= 1;
                }
            }
        }
        self.fix_selection();
        self.recompute();
    }
    pub fn rename_case(&mut self, name: &str) {
        let sel = self.inputs.selected;
        self.inputs.cases[sel].name = name.trim().to_string();
        self.recompute();
    }

    // ------------------------------------------------------------------ solve

    fn solve(&self) -> Result<LineResult, String> {
        let i = &self.inputs;
        let need = |v: Option<f64>, what: &str| v.ok_or_else(|| format!("Enter {what}: material constants and allowables are never defaulted."));
        let tube = Tube { outer_diameter: i.od, wall: i.wall, e: need(i.e, "the elastic modulus E")?, alpha: need(i.alpha, "the expansion coefficient")?, weight_density: need(i.density, "the weight density")? };
        let class = match i.class {
            Class::Ductile => MaterialClass::Ductile {
                yield_strength: need(i.fty, "the yield strength Fty")?,
                ultimate_strength: need(i.ftu, "the ultimate strength Ftu")?,
                yield_factor_of_safety: need(i.fs_yield, "the yield factor of safety")?,
                ultimate_factor_of_safety: need(i.fs_ultimate, "the ultimate factor of safety")?,
            },
            Class::Brittle => MaterialClass::Brittle {
                ultimate_tension: need(i.ftu, "the tension ultimate Ftu")?,
                ultimate_compression: need(i.fcu, "the compressive ultimate Fcu")?,
                factor_of_safety: need(i.fs_brittle, "the factor of safety")?,
            },
        };
        let load = |l: &LoadRow| PointLoad { x: l.x, force: l.force, weight: l.weight, moment: [0.0; 3], offset: l.offset, pressure_thrust: l.thrust };
        let run = lr::LineRun {
            tube,
            x_start: i.x_start,
            x_end: i.x_end,
            supports: i.supports.iter().map(|s| Support { x: s.x, axial_anchor: s.anchor, rotation_fixed: s.clamp }).collect(),
            contents_weight_per_length: i.contents,
            insulation_weight_per_length: i.insulation,
            point_loads: i.loads.iter().filter(|l| l.case.is_none()).map(load).collect(),
            subdivisions: SUBDIVISIONS,
        };
        let cases: Vec<LoadCase> = i
            .cases
            .iter()
            .enumerate()
            .map(|(k, c)| LoadCase {
                name: c.name.clone(),
                internal_pressure: c.internal_pressure,
                external_pressure: c.external_pressure,
                pressure_end_load: c.end_load,
                delta_t: c.delta_t,
                load_factors: c.factors,
                point_loads: i.loads.iter().filter(|l| l.case == Some(k)).map(load).collect(),
            })
            .collect();
        lr::analyze(&run, &class, &cases).map_err(|e| e.to_string())
    }

    /// The tube section for display (area, inertia); `None` for a section the solver refuses.
    pub fn tube(&self) -> Option<Tube> {
        let t = Tube { outer_diameter: self.inputs.od, wall: self.inputs.wall, e: self.inputs.e.unwrap_or(1.0), alpha: self.inputs.alpha.unwrap_or(0.0), weight_density: self.inputs.density.unwrap_or(0.0) };
        t.geometry().ok().map(|_| t)
    }
}

#[cfg(test)]
mod tests;
