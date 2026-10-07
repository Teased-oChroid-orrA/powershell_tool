//! Pure UI-facing model of the Eccentric Bushing toolbox: its own few inputs, the bridge from the Bushing toolbox's
//! live model to `eccentric_bushing::Inputs`, and the report text. It computes nothing itself.

pub use eccentric_bushing::Inputs;
use eccentric_bushing::{Analysis, Elasticity, OffsetLimit};

use crate::toolboxes::bushing::model::BushingModel;

/// What a worker run computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// The offset as entered: pressure map, torque capacity, demand, margin.
    Analyze,
    /// The largest offset that still holds, at the entered load.
    MaxOffset,
    /// The largest load that is held, at the entered offset.
    MaxLoad,
}

/// A finished worker run.
#[derive(Debug, Clone)]
pub enum Output {
    Analysis(Box<Analysis>),
    MaxOffset(OffsetLimit),
    MaxLoad(OffsetLimit),
}

/// Run one task (blocking: seconds to about a minute).
pub fn run(task: Task, input: &Inputs) -> Result<Output, String> {
    Ok(match task {
        Task::Analyze => Output::Analysis(Box::new(eccentric_bushing::analyze(input)?)),
        Task::MaxOffset => Output::MaxOffset(eccentric_bushing::max_offset(input, 0.02)?),
        Task::MaxLoad => Output::MaxLoad(eccentric_bushing::max_load(input, 0.02)?),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberTarget {
    Offset,
    LoadAngle,
    BossFactor,
    PinClearance,
    PinFriction,
    PinModulus,
    PinNu,
}

impl NumberTarget {
    pub fn label(self) -> &'static str {
        match self {
            NumberTarget::Offset => "Bore Offset e",
            NumberTarget::LoadAngle => "Load Angle",
            NumberTarget::BossFactor => "Boss OD / Bore",
            NumberTarget::PinClearance => "Pin Clearance (dia)",
            NumberTarget::PinFriction => "Pin Friction",
            NumberTarget::PinModulus => "Pin Modulus E",
            NumberTarget::PinNu => "Pin Poisson Ratio",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            NumberTarget::Offset => " in",
            NumberTarget::LoadAngle => " deg",
            NumberTarget::BossFactor => " x",
            NumberTarget::PinClearance => " in",
            NumberTarget::PinFriction => "",
            NumberTarget::PinModulus => " Msi",
            NumberTarget::PinNu => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    Header(&'static str),
    Number(NumberTarget),
    TogglePlane,
    ToggleHousing,
    TogglePinCredit,
    ToggleDirectOnset,
    Run(Task),
    /// Opens and closes the Advanced section (rows after it in `field_rows`).
    AdvancedSection,
}

/// Basic inputs and the three actions first; the modelling switches sit in the Advanced section.
pub fn field_rows(advanced: bool) -> Vec<FieldRow> {
    let mut rows = vec![
        FieldRow::Header("Eccentricity"),
        FieldRow::Number(NumberTarget::Offset),
        FieldRow::Number(NumberTarget::LoadAngle),
        FieldRow::Header("Housing & Pin"),
        FieldRow::Number(NumberTarget::BossFactor),
        FieldRow::Number(NumberTarget::PinClearance),
        FieldRow::Number(NumberTarget::PinFriction),
        FieldRow::Header("Run"),
        FieldRow::Run(Task::Analyze),
        FieldRow::Run(Task::MaxOffset),
        FieldRow::Run(Task::MaxLoad),
        FieldRow::AdvancedSection,
    ];
    if advanced {
        rows.extend([
            FieldRow::TogglePlane,
            FieldRow::ToggleHousing,
            FieldRow::Number(NumberTarget::PinModulus),
            FieldRow::Number(NumberTarget::PinNu),
            FieldRow::TogglePinCredit,
            FieldRow::ToggleDirectOnset,
        ]);
    }
    rows
}

pub fn row_label(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(h) => h,
        FieldRow::Number(t) => t.label(),
        FieldRow::TogglePlane => "Axial Condition",
        FieldRow::ToggleHousing => "Housing",
        FieldRow::TogglePinCredit => "Capacity Basis",
        FieldRow::ToggleDirectOnset => "Direct Spin Check",
        FieldRow::Run(Task::Analyze) => "Analyse this offset",
        FieldRow::Run(Task::MaxOffset) => "Find maximum offset",
        FieldRow::Run(Task::MaxLoad) => "Find maximum load",
        FieldRow::AdvancedSection => "Advanced settings",
    }
}

pub fn field_hint(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(_) => "",
        FieldRow::AdvancedSection => "Axial condition, housing type, pin modulus and Poisson ratio, capacity basis and the direct spin check. The analysis uses these values whether the section is open or not. Enter, Space or a click opens and closes it.",
        FieldRow::Number(NumberTarget::Offset) => "Distance between the bushing's bore centre and its outer-diameter centre. The wall on the thin side is (bore - ID)/2 minus this. The model needs it above 3 % of the bore; the Bushing Workbench's minimum wall is reported against it separately.",
        FieldRow::Number(NumberTarget::LoadAngle) => "Pin load direction from the offset line (the line from the bushing's OD centre through its bore centre). 90 is transverse: the worst case for spin; 0 or 180 push along the offset line and cause no spin torque.",
        FieldRow::Number(NumberTarget::BossFactor) => "Outer diameter of the round boss around the bore, as a multiple of the bore diameter (at least 1.5). The boss surface is free; only two supports prevent rigid motion.",
        FieldRow::TogglePlane => "Free ends (plane stress) is the Lame assumption of the Bushing Workbench, so the fit pressure agrees with it. Constrained ends (plane strain) models a bushing axially held by its housing: stiffer, higher pressure.",
        FieldRow::ToggleHousing => "Round boss: a free circular boss around the bore (Boss OD / Bore). Edge-limited: a plate whose free edge is the Bushing Workbench's Edge Distance from the bore centre, on the side opposite the offset; a thin ligament there makes the fit pressure uneven even when the bushing is concentric.",
        FieldRow::Number(NumberTarget::PinClearance) => "Diametral clearance between the pin and the bushing bore as fitted (the fit closes the bore a little). The pin is a meshed elastic body, loaded through its length and free to rotate.",
        FieldRow::Number(NumberTarget::PinFriction) => "Coulomb friction between the pin and the bushing bore. The pin is free to rotate, so it adds no net torque on the bushing; it only changes how the pin's pressure is distributed.",
        FieldRow::Number(NumberTarget::PinModulus) => "Pin Young's modulus in Msi (steel 29, titanium 16, aluminium 10.4).",
        FieldRow::Number(NumberTarget::PinNu) => "Pin Poisson's ratio.",
        FieldRow::TogglePinCredit => "With pin load: the margin uses the interface pressure the loaded pin actually produces (its mean pressure squeezes the bushing and raises the capacity). Fit alone: the conservative basis, no credit for the pin's squeeze.",
        FieldRow::ToggleDirectOnset => "On: also simulate the spin directly (a pure torque raised until the interface slips all round, 15-30 s more) and judge the margin on the smaller capacity; the integral capacity overstates the onset by 3-4 % when the wall varies 3:1. Off: the integral only.",
        FieldRow::Run(Task::Analyze) => "Enter or r: solve the fit and the pin load on the entered offset (a few seconds).",
        FieldRow::Run(Task::MaxOffset) => "Enter or m: the largest offset whose friction torque still carries load x offset x sin(angle). Bisection on the fit alone, confirmed with the loaded run (tens of seconds).",
        FieldRow::Run(Task::MaxLoad) => "Enter or l: the largest pin load held at the entered offset and angle.",
    }
}

/// The toolbox's own inputs (everything else is read from the Bushing Workbench).
#[derive(Debug, Clone, PartialEq)]
pub struct EccentricUi {
    /// The Advanced section of the field list is open.
    pub advanced_open: bool,
    pub offset: f64,
    pub load_angle: f64,
    pub boss_factor: f64,
    pub plane_strain: bool,
    pub pin_clearance: f64,
    pub pin_friction: f64,
    /// Msi.
    pub pin_modulus: f64,
    pub pin_nu: f64,
    pub credit_pin_load: bool,
    pub direct_onset: bool,
    /// Housing is a plate with the Bushing Workbench's edge distance instead of a round boss.
    pub edge_limited: bool,
}

impl Default for EccentricUi {
    fn default() -> Self {
        Self { advanced_open: false, offset: 0.02, load_angle: 90.0, boss_factor: 2.5, plane_strain: false, pin_clearance: 0.001, pin_friction: 0.1, pin_modulus: 29.0, pin_nu: 0.30, credit_pin_load: true, direct_onset: false, edge_limited: false }
    }
}

impl EccentricUi {
    pub fn number_value(&self, t: NumberTarget) -> f64 {
        match t {
            NumberTarget::Offset => self.offset,
            NumberTarget::LoadAngle => self.load_angle,
            NumberTarget::BossFactor => self.boss_factor,
            NumberTarget::PinClearance => self.pin_clearance,
            NumberTarget::PinFriction => self.pin_friction,
            NumberTarget::PinModulus => self.pin_modulus,
            NumberTarget::PinNu => self.pin_nu,
        }
    }

    /// Store an edited value, clamped to a sensible range; returns whether it was accepted.
    pub fn commit_number(&mut self, t: NumberTarget, raw: f64) -> bool {
        if !raw.is_finite() {
            return false;
        }
        match t {
            NumberTarget::Offset => self.offset = raw.max(0.0),
            NumberTarget::LoadAngle => self.load_angle = raw.rem_euclid(360.0),
            NumberTarget::BossFactor => self.boss_factor = raw.max(1.5),
            NumberTarget::PinClearance => self.pin_clearance = raw.max(0.0),
            NumberTarget::PinFriction => self.pin_friction = raw.clamp(0.0, 2.0),
            NumberTarget::PinModulus => self.pin_modulus = raw.max(0.1),
            NumberTarget::PinNu => self.pin_nu = raw.clamp(0.0, 0.49),
        }
        true
    }
}

pub fn format_for_edit(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn format_value(t: NumberTarget, v: f64) -> String {
    match t {
        NumberTarget::Offset => format!("{v:.4}{}", t.unit()),
        NumberTarget::LoadAngle => format!("{v:.1}{}", t.unit()),
        NumberTarget::BossFactor => format!("{v:.2}{}", t.unit()),
        NumberTarget::PinClearance => format!("{v:.4}{}", t.unit()),
        NumberTarget::PinFriction | NumberTarget::PinNu => format!("{v:.2}"),
        NumberTarget::PinModulus => format!("{v:.1}{}", t.unit()),
    }
}

/// The solver input for the live Bushing Workbench model and this toolbox's inputs, or why it cannot be built.
pub fn build_input(bushing: &BushingModel, ui: &EccentricUi) -> Result<Inputs, String> {
    let out = &bushing.output;
    let bore = out.bore_tol.nominal;
    let finite = [bore, bushing.id_bushing, out.delta_total, bushing.housing_len, bushing.load, bushing.friction];
    if finite.iter().any(|v| !v.is_finite()) {
        return Err("the Bushing Workbench inputs are incomplete (bore, ID, interference, length, load)".into());
    }
    let inputs = Inputs {
        bore_dia: bore,
        housing_od: ui.boss_factor * bore,
        edge_distance: ui.edge_limited.then_some(bushing.edge_dist),
        bushing_id: bushing.id_bushing,
        offset: ui.offset,
        interference_dia: out.delta_total,
        thickness: bushing.housing_len,
        housing: Elasticity::from(bushing.housing_material()),
        bushing: Elasticity::from(bushing.bushing_material()),
        friction: bushing.friction,
        load_lbf: bushing.load,
        load_angle_deg: ui.load_angle,
        direct_onset: ui.direct_onset,
        min_wall: bushing.min_wall_straight,
        pin: Elasticity::iso(ui.pin_modulus * 1.0e6, ui.pin_nu),
        pin_friction: ui.pin_friction,
        pin_clearance_dia: ui.pin_clearance,
        credit_pin_load: ui.credit_pin_load,
        plane_strain: ui.plane_strain,
        mesh_size: None,
    };
    inputs.validate()?;
    Ok(inputs)
}

/// Plain-text report of everything computed for `input`.
pub fn report_text(input: &Inputs, analysis: Option<&Analysis>, max_offset: Option<&OffsetLimit>, max_load: Option<&OffsetLimit>) -> String {
    let mut s = String::from("ECCENTRIC BUSHING - SPIN CAPACITY (design guide, not a certification value)\n\n");
    s.push_str(&format!(
        "Bore {:.4} in, bushing ID {:.4} in, interference {:.4} in (diametral), length {:.3} in, {}\nFriction {:.2}, pin load {:.0} lbf at {:.1} deg to the offset line, {}\nPin: E {:.1} Msi, clearance {:.4} in, friction {:.2}; capacity judged {}\n\n",
        input.bore_dia, input.bushing_id, input.interference_dia, input.thickness, input.edge_distance.map_or(format!("boss OD {:.3} in", input.housing_od), |e| format!("plate, edge {e:.3} in")), input.friction, input.load_lbf, input.load_angle_deg,
        if input.plane_strain { "constrained ends (plane strain)" } else { "free ends (plane stress)" },
        input.pin.e_psi / 1.0e6, input.pin_clearance_dia, input.pin_friction, if input.credit_pin_load { "with the pin load" } else { "on the fit alone (conservative)" }
    ));
    if let Some(a) = analysis {
        s.push_str(&format!(
            "Offset {:.4} in: wall {:.4} in thin / {:.4} in thick{}\nTorque required  F e sin(angle) {:>10.2} lbf in\nCapacity, fit alone             {:>10.2} lbf in\nCapacity, with the pin load     {:>10.2} lbf in\nDesign capacity (the smaller)   {:>10.2} lbf in\nMargin {}\nFit pressure {:.0} to {:.0} psi around the interface; contact lost over {:.0} deg under load\nPin bears on {:.0} deg of the bore, peak {:.0} psi\nFriction at its limit on {:.0} % of the normal force\n\n",
            a.offset, a.wall_thin, a.wall_thick, if a.wall_ok { "" } else { " - BELOW THE MINIMUM WALL" }, a.torque_required, a.torque_capacity_fit, a.torque_capacity, a.design_capacity,
            if a.margin.is_finite() { format!("{:+.1} %  ({})", a.margin * 100.0, if a.margin >= 0.0 { "holds" } else { "SPINS" }) } else { "no spin torque at this angle".into() },
            a.fit_pressure_min, a.fit_pressure_max, a.contact_lost_deg, a.pin_arc_deg, a.pin_peak_pressure, a.slip_share * 100.0
        ));
    }
    if let Some(l) = max_offset {
        s.push_str(&format!("Offset the spin check allows: up to {:.4} in{}\n", l.value, if l.bounded_by_wall { " (spin never limits it: the numerical wall floor does)" } else { "" }));
        s.push_str(&format!("Offset the minimum wall allows: up to {:.4} in{}\n", l.wall_limit, if l.wall_limit < l.value { " (the wall governs, not spin)" } else { "" }));
    }
    if let Some(l) = max_load {
        s.push_str(&if l.value.is_finite() { format!("Maximum pin load that is held: {:.0} lbf\n", l.value) } else { "No spin torque at this angle: any load is held.\n".to_string() });
    }
    s
}
