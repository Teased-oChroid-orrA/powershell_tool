//! Load allowables by failure mode for a pin or fastener in a hole near a free edge: the table the Plate Edge
//! Workbench shows. Every row is the pin load (lbf) at which ONE mode reaches its criterion, so the lowest row is the
//! mode that governs and no number is a margin in disguise.
//!
//! Plate rows: bearing onset (`Fbry` at the actual e/D, the 2 % offset definition) and bearing ultimate (`Fbru`) from the
//! handbook tables, shear-out (the classical two inclined planes, charged with the fit pressure), tear-out (the
//! ligament in hoop tension against `Ftu`, from the elastic stress field), first yield (peak elastic von Mises, and the
//! contact FE's), and the contact FE's collapse load. Pin rows: shear, bearing and bending of the pin itself.
//!
//! Nothing here is new mechanics: the elastic field and contact FE are the [`crate::models`] the Bushing window uses,
//! asked one mode at a time by [`crate::runner::mode_capacities`]. What is new is the closed forms below and the fit
//! pressure of a solid pin pressed into the plate.
//!
//! A row that cannot be computed carries the reason (`why`), never a guess: a bolt library gives no yield strength, a
//! material without a tabulated `Fbry` has no bearing onset.
//!
//! Units: inches, pounds-force, psi.

use crate::estimate::{prepare, BoreFit, EstimateInput, EstimateOptions, LoadCase, PIN_CORE};
use crate::models::default_models;
use crate::runner::mode_capacities;
use crate::types::{BushingSpec, Mode, Strengths};
use mechanics_core::materials::fbru_at_edge_ratio;

/// The pin or fastener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinSpec {
    /// Nominal diameter of the shank, in. The hole diameter is this minus the diametral interference.
    pub dia: f64,
    pub e: f64,
    pub nu: f64,
    pub ftu: f64,
    /// Ultimate shear strength (the specification rating for a bolt).
    pub fsu: f64,
    /// Tensile yield: the pin bending row needs it. `None` (a bolt's specification gives none) leaves the row unavailable.
    pub fty: Option<f64>,
    /// Bearing ultimate of the pin material on `D t`: `None` leaves the pin bearing row unavailable.
    pub fbru: Option<f64>,
    /// Shear planes carrying the load: 1 (single shear) or 2 (double shear).
    pub planes: u8,
    /// Distance between the bearing resultants of the load transfer (clevis span for double shear, the mean of the two
    /// plate thicknesses for single shear), in. Sets the pin's bending moment.
    pub span: f64,
}

/// Everything the table needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ModesInput {
    /// Hole diameter, in.
    pub bore_dia: f64,
    /// Hole centre to the free edge.
    pub edge: f64,
    pub thickness: f64,
    pub plate: Strengths,
    /// `Fbry` at e/D = 2.0 and 1.5 (psi, `0` = not tabulated): the bearing yield, which the stress rows cannot supply.
    pub fbry: f64,
    pub fbry_e15: f64,
    pub pin: PinSpec,
    /// Diametral interference, in: positive presses the pin in (the hole is smaller), zero is line to line, negative is a
    /// clearance. The pin diameter is `bore_dia + delta`.
    pub delta: f64,
    /// Contact pressure of a custom fit (psi); `None` takes it from `delta`.
    pub custom_pressure: Option<f64>,
    pub friction: f64,
    /// Inclination of the classical shear-out planes, degrees.
    pub plane_angle_deg: f64,
    /// Also run the contact FE (about 3 s).
    pub deep: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Plate,
    Pin,
}

/// One failure mode and the load that reaches it by each method that can say.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeRow {
    pub id: &'static str,
    pub label: &'static str,
    pub basis: &'static str,
    pub group: Group,
    /// Closed form or table (the handbook column).
    pub handbook: Option<f64>,
    /// Elastic stress field.
    pub elastic: Option<f64>,
    /// 2D contact FE.
    pub contact: Option<f64>,
    /// Why no column has a number.
    pub why: Option<String>,
    /// Counts toward the governing mode (the first-yield and collapse rows are informational).
    pub governs: bool,
}

impl ModeRow {
    /// The lowest load any method gives, the number a design check would use.
    pub fn lowest(&self) -> Option<f64> {
        [self.handbook, self.elastic, self.contact].into_iter().flatten().min_by(|a, b| a.total_cmp(b))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModesResult {
    pub rows: Vec<ModeRow>,
    /// Contact pressure of the fit, psi.
    pub fit_pressure: f64,
    /// Index of the governing row, if any row has a finite number.
    pub governing: Option<usize>,
    pub notes: Vec<String>,
}

/// Contact pressure of a solid pin pressed into a hole in a plate that is large against the hole, psi (Lamé: the plate's
/// radial compliance `(1 + nu) / E` and the solid pin's `(1 - nu) / E`). Zero for a clearance or line-to-line fit.
pub fn fit_pressure(bore_dia: f64, delta: f64, plate: &Strengths, pin: &PinSpec) -> f64 {
    if delta <= 0.0 || bore_dia <= 0.0 {
        return 0.0;
    }
    let a = 0.5 * bore_dia;
    (0.5 * delta) / (a * ((1.0 + plate.nu) / plate.e + (1.0 - pin.nu) / pin.e))
}

/// Whether `v` is a usable strength.
fn some(v: f64) -> Option<f64> {
    (v.is_finite() && v > 0.0).then_some(v)
}

fn validate(i: &ModesInput) -> Result<(), String> {
    let pos = |v: f64, what: &str| if v.is_finite() && v > 0.0 { Ok(()) } else { Err(format!("{what} must be positive")) };
    pos(i.bore_dia, "hole diameter")?;
    pos(i.thickness, "plate thickness")?;
    pos(i.pin.dia, "pin diameter")?;
    pos(i.plate.e, "plate modulus")?;
    pos(i.plate.sy, "plate yield strength")?;
    pos(i.plate.ftu, "plate ultimate strength")?;
    pos(i.plate.fsu, "plate shear strength")?;
    pos(i.pin.fsu, "pin shear strength")?;
    pos(i.pin.e, "pin modulus")?;
    if !i.edge.is_finite() || i.edge <= 0.5 * i.bore_dia {
        return Err("the edge distance must exceed the hole radius".into());
    }
    if !i.delta.is_finite() || !i.friction.is_finite() || i.friction < 0.0 {
        return Err("the fit and friction must be finite, friction not negative".into());
    }
    if (i.pin.dia - (i.bore_dia + i.delta)).abs() > 1e-9 * i.bore_dia {
        return Err("the pin diameter must equal the hole diameter plus the diametral interference".into());
    }
    if !(i.pin.planes == 1 || i.pin.planes == 2) {
        return Err("the pin carries its load in one or two shear planes".into());
    }
    Ok(())
}

/// The table. Blocking (about 0.3 s, about 3 s with the contact FE): call from a worker.
pub fn analyse(i: &ModesInput) -> Result<ModesResult, String> {
    validate(i)?;
    let (d, t, a) = (i.bore_dia, i.thickness, 0.5 * i.bore_dia);
    let ed = i.edge / d;
    let p = i.custom_pressure.filter(|p| p.is_finite() && *p >= 0.0).unwrap_or_else(|| fit_pressure(d, i.delta, &i.plate, &i.pin));
    let mut notes = Vec::new();
    if i.custom_pressure.is_none() && i.delta > 0.0 {
        notes.push(format!("fit pressure {p:.0} psi from the diametral interference {:.4} in (solid pin in a large plate)", i.delta));
    }

    // The two stress-based columns from the shared models, one mode at a time.
    let fit = if p > 0.0 || i.delta > 0.0 {
        BoreFit::Bushing(BushingSpec { inner_radius: PIN_CORE * a, interference: 0.5 * i.delta.max(0.0), e: i.pin.e, nu: i.pin.nu, friction: i.friction })
    } else {
        BoreFit::Pin { friction: i.friction }
    };
    let est = EstimateInput {
        bore_radius: a,
        edge: i.edge,
        thickness: t,
        plane_angle_deg: i.plane_angle_deg,
        material: i.plate,
        fit,
        load: LoadCase::PinTowardEdge { load: 1000.0 },
        fit_pressure: p,
        fit_pressure_min: p,
        fit_pressure_max: p,
        legacy: None,
    };
    let (input, cfg) = prepare(&est, &EstimateOptions { deep: i.deep, ..Default::default() })?;
    let caps = mode_capacities(&default_models(&cfg), &input);
    let cap = |model: &str, mode: Mode| caps.iter().find(|c| c.model == model && c.mode == mode).map(|c| c.capacity_lbf);

    // Handbook closed forms.
    let bearing = |strength: Option<f64>| strength.map(|f| f * d * t);
    let fbry_at = fbru_at_edge_ratio(i.fbry, i.fbry_e15, ed).and_then(some);
    let fbru_at = fbru_at_edge_ratio(i.plate.fbru, i.plate.fbru_e15, ed).and_then(some);
    let th = i.plane_angle_deg.to_radians();
    let shear_out = (2.0 * i.plate.fsu * t * (i.edge - a * th.cos()) - p * d * t).max(0.0);

    let no_table = |what: &str| {
        if ed < 1.5 {
            format!("no {what} for e/D {ed:.2}: the handbook tables stop at e/D 1.5, below that needs test substantiation")
        } else {
            format!("no {what} for e/D {ed:.2}: the material has no tabulated value at e/D 1.5, so the table needs e/D 2.0 or more (add the e/D 1.5 value to the material)")
        }
    };
    let mut rows = vec![
        ModeRow {
            id: "bearing_onset",
            label: "Bearing onset (yield)",
            basis: "Fbry(e/D) D t, the load at 2 % D permanent hole elongation",
            group: Group::Plate,
            handbook: bearing(fbry_at),
            elastic: None,
            contact: None,
            why: fbry_at.is_none().then(|| if i.fbry <= 0.0 { "the material has no tabulated Fbry: choose one that does or type it".to_string() } else { no_table("Fbry") }),
            governs: true,
        },
        ModeRow {
            id: "bearing_ultimate",
            label: "Bearing ultimate",
            basis: "Fbru(e/D) D t",
            group: Group::Plate,
            handbook: bearing(fbru_at),
            elastic: None,
            contact: None,
            why: fbru_at.is_none().then(|| no_table("Fbru")),
            governs: true,
        },
        ModeRow {
            id: "shear_out",
            label: "Shear-out",
            basis: "two planes at the shear-out angle: 2 Fsu t (e - a cos) less the fit's p D t; elastic field: mean tangent-plane shear = Fsu",
            group: Group::Plate,
            handbook: Some(shear_out),
            elastic: cap("analytic", Mode::ShearOut),
            contact: None,
            why: None,
            governs: true,
        },
        ModeRow {
            id: "tear_out",
            label: "Tear-out (ligament tension)",
            basis: "mean hoop tension across the ligament between hole and edge = Ftu (elastic field)",
            group: Group::Plate,
            handbook: None,
            elastic: cap("analytic", Mode::Splitting),
            contact: None,
            why: None,
            governs: true,
        },
        ModeRow {
            id: "first_yield",
            label: "First yield",
            basis: "peak elastic von Mises on the hole or the free edge = Fty (informational)",
            group: Group::Plate,
            handbook: None,
            elastic: cap("analytic", Mode::FirstYield),
            contact: cap("contact", Mode::FirstYield),
            why: None,
            governs: false,
        },
        ModeRow {
            id: "collapse",
            label: "Collapse (contact FE, plane strain)",
            basis: "elastic-plastic contact FE, pin driven to 0.1 D travel (plane strain, advisory)",
            group: Group::Plate,
            handbook: None,
            elastic: None,
            contact: cap("contact", Mode::Collapse),
            why: (!i.deep).then(|| "run the contact FE (Run 2D) to get it".to_string()),
            governs: false,
        },
    ];

    // The pin itself.
    let pd = i.pin.dia;
    let shear = i.pin.planes as f64 * i.pin.fsu * std::f64::consts::PI * pd * pd / 4.0;
    let bending = i.pin.fty.and_then(some).filter(|_| i.pin.span > 0.0).map(|fty| {
        let moment_cap = fty * std::f64::consts::PI * pd.powi(3) / 32.0;
        // Uniform bearing over the span, pin simply supported at the ends (double shear): M = P L / 8; single shear: M = P L / 2.
        moment_cap / (i.pin.span * if i.pin.planes == 2 { 0.125 } else { 0.5 })
    });
    rows.push(ModeRow {
        id: "pin_shear",
        label: "Pin shear",
        basis: "planes x Fsu x pi D^2 / 4",
        group: Group::Pin,
        handbook: Some(shear),
        elastic: None,
        contact: None,
        why: None,
        governs: true,
    });
    rows.push(ModeRow {
        id: "pin_bearing",
        label: "Pin bearing",
        basis: "pin Fbru x D x t",
        group: Group::Pin,
        handbook: i.pin.fbru.and_then(some).map(|f| f * pd * t),
        elastic: None,
        contact: None,
        why: i.pin.fbru.and_then(some).is_none().then(|| "the pin has no bearing strength: choose a pin material that has one or type it".to_string()),
        governs: true,
    });
    rows.push(ModeRow {
        id: "pin_bending",
        label: "Pin bending (yield)",
        basis: "Fty pi D^3 / 32 against M = P L / 8 (double shear) or P L / 2 (single), L the span between bearing resultants",
        group: Group::Pin,
        handbook: bending,
        elastic: None,
        contact: None,
        why: bending.is_none().then(|| "needs the pin's yield strength and a span: choose a pin material or type them".to_string()),
        governs: true,
    });

    let governing = rows.iter().enumerate().filter(|(_, r)| r.governs).filter_map(|(k, r)| r.lowest().filter(|v| v.is_finite()).map(|v| (k, v))).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(k, _)| k);
    Ok(ModesResult { rows, fit_pressure: p, governing, notes })
}
