//! Bushing Workbench - straight-bushing interference-fit calculator.
//! Ported from ~/Claude/Projects/engineering.toolbox's TypeScript bushing
//! workbench (see `bushing-solver`'s `Cargo.toml` doc comment for the
//! exact scope decision - straight bushings only, no countersink/flange/
//! duty-process-approval layers - and `docs/bushing-workbench-status.md`
//! in this repo for the full writeup). This file owns the UI only; all
//! the actual physics lives in `bushing-solver`, verified against the
//! real production TS engine's own output (`bushing-solver/tests/differential.rs`).

use dioxus::prelude::*;

use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use bushing_solver::materials::MATERIALS;
use bushing_solver::reamers::{self, ReamerEntry};
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use bushing_solver::tolerance::{EnforcementPolicy, ToleranceStatus};

use crate::components::Dropdown;

/// The 7 core derivation formulas, in display order - ids match the PNG
/// asset filenames (`{id}_dark.png`/`{id}_light.png`, pre-rendered via
/// KaTeX+Playwright from the exact same LaTeX source strings
/// engineering.toolbox's own `BushingInformationPage.svelte` uses - see
/// `docs/bushing-workbench-status.md` for the render pipeline).
struct Formula {
    id: &'static str,
    dark_png: &'static [u8],
    light_png: &'static [u8],
}

macro_rules! formula {
    ($id:literal) => {
        Formula {
            id: $id,
            dark_png: include_bytes!(concat!("../assets/bushing_formulas/", $id, "_dark.png")),
            light_png: include_bytes!(concat!("../assets/bushing_formulas/", $id, "_light.png")),
        }
    };
}

static FORMULAS: &[Formula] = &[
    formula!("thermal_delta_interference"),
    formula!("installed_outer_diameter"),
    formula!("contact_pressure"),
    formula!("hoop_stress_housing"),
    formula!("hoop_stress_bushing"),
    formula!("install_force"),
    formula!("margin_of_safety"),
];

fn formula_img_src(f: &Formula, dark: bool) -> String {
    use base64::Engine;
    let bytes = if dark { f.dark_png } else { f.light_png };
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn margin_class(margin: f64) -> &'static str {
    if !margin.is_finite() {
        "ms-pill ms-neutral"
    } else if margin < 0.0 {
        "ms-pill ms-fail"
    } else if margin < 0.15 {
        "ms-pill ms-marginal"
    } else {
        "ms-pill ms-pass"
    }
}

fn fmt_margin(margin: f64) -> String {
    if margin.is_infinite() {
        "\u{2014}".to_string() // em dash - no governing demand for this check
    } else {
        format!("{:+.2}", margin)
    }
}

#[component]
pub fn BushingWorkbench(dark: Signal<bool>) -> Element {
    let mut bore_dia = use_signal(|| 0.500_f64);
    let mut bore_tol_plus = use_signal(|| 0.0005_f64);
    let mut bore_tol_minus = use_signal(|| 0.0_f64);
    let id_bushing = use_signal(|| 0.375_f64);
    let interference = use_signal(|| 0.0015_f64);
    let interference_tol_plus = use_signal(|| 0.0003_f64);
    let interference_tol_minus = use_signal(|| 0.0_f64);
    let housing_len = use_signal(|| 0.5_f64);
    let housing_width = use_signal(|| 1.5_f64);
    let edge_dist = use_signal(|| 1.0_f64);
    let mat_housing = use_signal(|| "al7075".to_string());
    let mat_bushing = use_signal(|| "bronze".to_string());
    let friction = use_signal(|| 0.15_f64);
    let d_t = use_signal(|| 0.0_f64);
    let mut end_constraint = use_signal(EndConstraint::default);
    let min_wall_straight = use_signal(|| 0.05_f64);
    let edge_load_angle_deg = use_signal(|| 40.0_f64);
    let load = use_signal(|| 1000.0_f64);
    let mut reamer_picker_open = use_signal(|| false);
    let mut show_derivation = use_signal(|| false);

    let mut bushing_type = use_signal(BushingType::default);
    let mut id_type = use_signal(IdType::default);
    let flange_od = use_signal(|| 0.75_f64);
    let flange_thk = use_signal(|| 0.063_f64);
    let min_wall_neck = use_signal(|| 0.04_f64);

    let cs_mode = use_signal(CsMode::default);
    let cs_dia = use_signal(|| 0.5_f64);
    let cs_depth = use_signal(|| 0.08_f64);
    let cs_angle = use_signal(|| 100.0_f64);
    let cs_dia_tol_plus = use_signal(|| 0.002_f64);
    let cs_dia_tol_minus = use_signal(|| 0.0_f64);
    let cs_depth_tol_plus = use_signal(|| 0.005_f64);
    let cs_depth_tol_minus = use_signal(|| 0.0_f64);

    let ext_cs_mode = use_signal(CsMode::default);
    let ext_cs_dia = use_signal(|| 0.6_f64);
    let ext_cs_depth = use_signal(|| 0.06_f64);
    let ext_cs_angle = use_signal(|| 100.0_f64);
    let ext_cs_dia_tol_plus = use_signal(|| 0.002_f64);
    let ext_cs_dia_tol_minus = use_signal(|| 0.0_f64);
    let ext_cs_depth_tol_plus = use_signal(|| 0.005_f64);
    let ext_cs_depth_tol_minus = use_signal(|| 0.0_f64);

    let mut enforcement_enabled = use_signal(|| false);

    let input = BushingInputs {
        bore_dia: bore_dia(),
        bore_tol_plus: bore_tol_plus(),
        bore_tol_minus: bore_tol_minus(),
        id_bushing: id_bushing(),
        interference: interference(),
        interference_tol_plus: interference_tol_plus(),
        interference_tol_minus: interference_tol_minus(),
        housing_len: housing_len(),
        housing_width: housing_width(),
        edge_dist: edge_dist(),
        mat_housing: mat_housing(),
        mat_bushing: mat_bushing(),
        friction: Some(friction()),
        d_t: d_t(),
        end_constraint: end_constraint(),
        min_wall_straight: min_wall_straight(),
        edge_load_angle_deg: Some(edge_load_angle_deg()),
        load: Some(load()),
        bushing_type: bushing_type(),
        id_type: id_type(),
        flange_od: flange_od(),
        flange_thk: flange_thk(),
        min_wall_neck: min_wall_neck(),
        cs_mode: cs_mode(),
        cs_dia: cs_dia(),
        cs_depth: cs_depth(),
        cs_angle: cs_angle(),
        cs_dia_tol_plus: cs_dia_tol_plus(),
        cs_dia_tol_minus: cs_dia_tol_minus(),
        cs_depth_tol_plus: cs_depth_tol_plus(),
        cs_depth_tol_minus: cs_depth_tol_minus(),
        ext_cs_mode: ext_cs_mode(),
        ext_cs_dia: ext_cs_dia(),
        ext_cs_depth: ext_cs_depth(),
        ext_cs_angle: ext_cs_angle(),
        ext_cs_dia_tol_plus: ext_cs_dia_tol_plus(),
        ext_cs_dia_tol_minus: ext_cs_dia_tol_minus(),
        ext_cs_depth_tol_plus: ext_cs_depth_tol_plus(),
        ext_cs_depth_tol_minus: ext_cs_depth_tol_minus(),
        enforcement: EnforcementPolicy { enabled: enforcement_enabled(), ..EnforcementPolicy::default() },
        bore_capability: None,
    };
    let out = compute(&input);

    let mat_bushing_props = *bushing_solver::materials::get_material(&mat_bushing());
    let mat_housing_props = *bushing_solver::materials::get_material(&mat_housing());

    rsx! {
        div { class: "panel bushing-layout",
            div { class: "bushing-inputs",
                BushingSection { title: "Geometry",
                    div { class: "field",
                        span { class: "field-label", "OD geometry" }
                        div { class: "chip-row",
                            for (t , label) in [(BushingType::Straight, "Straight"), (BushingType::Flanged, "Flanged"), (BushingType::Countersink, "Countersink")] {
                                span {
                                    class: if bushing_type() == t { "chip selected" } else { "chip" },
                                    onclick: move |_| bushing_type.set(t),
                                    "{label}"
                                }
                            }
                        }
                    }
                    div { class: "field",
                        span { class: "field-label", "ID geometry" }
                        div { class: "chip-row",
                            for (t , label) in [(IdType::Straight, "Straight"), (IdType::Countersink, "Countersink")] {
                                span {
                                    class: if id_type() == t { "chip selected" } else { "chip" },
                                    onclick: move |_| id_type.set(t),
                                    "{label}"
                                }
                            }
                        }
                    }
                    NumberField { label: "Housing bore, nominal (in)", value: bore_dia, step: "0.0005" }
                    div { class: "field-row",
                        NumberField { label: "Bore tol +", value: bore_tol_plus, step: "0.0001" }
                        NumberField { label: "Bore tol \u{2212}", value: bore_tol_minus, step: "0.0001" }
                    }
                    div { class: "field-inline-row",
                        button {
                            class: "reamer-picker-trigger",
                            onclick: move |_| reamer_picker_open.set(!reamer_picker_open()),
                            "Pick reamer\u{2026}"
                        }
                        if reamer_picker_open() {
                            ReamerPicker {
                                target_in: bore_dia(),
                                on_pick: move |entry: ReamerEntry| {
                                    bore_dia.set(entry.nominal_in);
                                    bore_tol_plus.set(entry.tool_tolerance_plus_in);
                                    bore_tol_minus.set(entry.tool_tolerance_minus_in);
                                    reamer_picker_open.set(false);
                                },
                                on_close: move |_| reamer_picker_open.set(false),
                            }
                        }
                    }
                    NumberField { label: "Bushing ID (in)", value: id_bushing, step: "0.001" }
                    NumberField { label: "Housing length (in)", value: housing_len, step: "0.01" }
                    NumberField { label: "Housing width (in)", value: housing_width, step: "0.01" }
                    NumberField { label: "Edge distance (in)", value: edge_dist, step: "0.01" }
                }
                if id_type() == IdType::Countersink {
                    BushingSection { title: "Internal countersink (ID)",
                        CsModeField { mode: cs_mode }
                        CsGeometryField { mode: cs_mode(), which: CsField::Dia, label: "Countersink diameter (in)", value: cs_dia, step: "0.001", solved: out.cs_solved_id }
                        CsGeometryField { mode: cs_mode(), which: CsField::Depth, label: "Countersink depth (in)", value: cs_depth, step: "0.001", solved: out.cs_solved_id }
                        CsGeometryField { mode: cs_mode(), which: CsField::Angle, label: "Countersink angle (deg)", value: cs_angle, step: "1", solved: out.cs_solved_id }
                        div { class: "field-row",
                            NumberField { label: "Dia tol +", value: cs_dia_tol_plus, step: "0.0005" }
                            NumberField { label: "Dia tol \u{2212}", value: cs_dia_tol_minus, step: "0.0005" }
                        }
                        div { class: "field-row",
                            NumberField { label: "Depth tol +", value: cs_depth_tol_plus, step: "0.0005" }
                            NumberField { label: "Depth tol \u{2212}", value: cs_depth_tol_minus, step: "0.0005" }
                        }
                    }
                }
                if bushing_type() == BushingType::Countersink || bushing_type() == BushingType::Flanged {
                    BushingSection { title: "External geometry (OD)",
                        if bushing_type() == BushingType::Countersink {
                            CsModeField { mode: ext_cs_mode }
                            CsGeometryField { mode: ext_cs_mode(), which: CsField::Dia, label: "Countersink diameter (in)", value: ext_cs_dia, step: "0.001", solved: out.cs_solved_od }
                            CsGeometryField { mode: ext_cs_mode(), which: CsField::Depth, label: "Countersink depth (in)", value: ext_cs_depth, step: "0.001", solved: out.cs_solved_od }
                            CsGeometryField { mode: ext_cs_mode(), which: CsField::Angle, label: "Countersink angle (deg)", value: ext_cs_angle, step: "1", solved: out.cs_solved_od }
                            div { class: "field-row",
                                NumberField { label: "Dia tol +", value: ext_cs_dia_tol_plus, step: "0.0005" }
                                NumberField { label: "Dia tol \u{2212}", value: ext_cs_dia_tol_minus, step: "0.0005" }
                            }
                            div { class: "field-row",
                                NumberField { label: "Depth tol +", value: ext_cs_depth_tol_plus, step: "0.0005" }
                                NumberField { label: "Depth tol \u{2212}", value: ext_cs_depth_tol_minus, step: "0.0005" }
                            }
                        }
                        if bushing_type() == BushingType::Flanged {
                            NumberField { label: "Flange OD (in)", value: flange_od, step: "0.01" }
                            NumberField { label: "Flange thickness (in)", value: flange_thk, step: "0.001" }
                        }
                    }
                }
                BushingSection { title: "Interference & tolerance",
                    NumberField { label: "Target diametral interference (in)", value: interference, step: "0.0001" }
                    div { class: "field-row",
                        NumberField { label: "Interference tol +", value: interference_tol_plus, step: "0.0001" }
                        NumberField { label: "Interference tol \u{2212}", value: interference_tol_minus, step: "0.0001" }
                    }
                    label { class: "field field-checkbox",
                        input {
                            r#type: "checkbox",
                            checked: enforcement_enabled(),
                            oninput: move |e| enforcement_enabled.set(e.checked()),
                        }
                        span { "Auto-tighten bore tolerance to meet target interference" }
                    }
                }
                BushingSection { title: "Neck wall",
                    NumberField { label: "Minimum neck wall (in)", value: min_wall_neck, step: "0.001" }
                }
                BushingSection { title: "Materials",
                    MaterialField { label: "Housing material", value: mat_housing }
                    MaterialField { label: "Bushing material", value: mat_bushing }
                }
                BushingSection { title: "Environment & install",
                    NumberField { label: "Friction coefficient", value: friction, step: "0.01" }
                    NumberField { label: "Temperature change, \u{0394}T (\u{00b0}F)", value: d_t, step: "1" }
                    div { class: "field",
                        span { class: "field-label", "End constraint" }
                        div { class: "chip-row",
                            for (ec , label) in [(EndConstraint::Free, "Free"), (EndConstraint::OneEnd, "One end"), (EndConstraint::BothEnds, "Both ends")] {
                                span {
                                    class: if end_constraint() == ec { "chip selected" } else { "chip" },
                                    onclick: move |_| end_constraint.set(ec),
                                    "{label}"
                                }
                            }
                        }
                    }
                    NumberField { label: "Minimum straight wall (in)", value: min_wall_straight, step: "0.001" }
                    NumberField { label: "Edge load angle (deg)", value: edge_load_angle_deg, step: "1" }
                    NumberField { label: "Applied edge load (lbf)", value: load, step: "10" }
                }
            }

            div { class: "bushing-results",
                div { class: "bushing-results-scroll",
                    div { class: "bushing-summary-row",
                        SummaryCard { label: "Contact pressure", value: format!("{:.0} psi", out.pressure) }
                        SummaryCard { label: "Installed OD", value: format!("{:.4} in", out.od_installed) }
                        SummaryCard { label: "Straight wall", value: format!("{:.4} in", out.wall_straight) }
                        SummaryCard {
                            label: "Governing check",
                            value: out.governing.name.to_string(),
                            badge: Some((fmt_margin(out.governing.margin), margin_class(out.governing.margin))),
                        }
                    }

                    if out.fail_straight {
                        div { class: "bushing-alert bushing-alert-fail",
                            "Straight wall thickness ({out.wall_straight:.4} in) is below the minimum ({min_wall_straight():.4} in)."
                        }
                    }
                    if out.fail_neck {
                        div { class: "bushing-alert bushing-alert-fail",
                            "Neck wall thickness ({out.wall_neck:.4} in) is below the minimum ({min_wall_neck():.4} in)."
                        }
                    }
                    if out.delta_total <= 0.0 {
                        div { class: "bushing-alert bushing-alert-warn",
                            "Net interference is zero or negative after thermal correction - this is a clearance fit, not a press fit."
                        }
                    }
                    if out.tolerance_status == ToleranceStatus::Infeasible {
                        div { class: "bushing-alert bushing-alert-warn",
                            "Bore and interference tolerance bands don't fully overlap - the achieved-interference range shown is collapsed to a point estimate, not a real tolerance band."
                        }
                    }
                    if out.tolerance_status == ToleranceStatus::Clamped {
                        div { class: "bushing-alert bushing-alert-warn",
                            "OD nominal was clamped to keep the fit inside the requested interference tolerance window (bore tolerance auto-adjustment was applied)."
                        }
                    }

                    div { class: "bushing-margins",
                        MarginRow { label: "Housing hoop stress", stress: out.stress_hoop_housing, ms: out.housing_ms, allowable: mat_housing_props.sy_ksi * 1000.0 }
                        MarginRow { label: "Bushing hoop stress", stress: out.stress_hoop_bushing, ms: out.bushing_ms, allowable: mat_bushing_props.sy_ksi * 1000.0 }
                        MarginRow { label: "Edge distance (sequencing)", stress: out.ed_actual, ms: out.sequence_margin, allowable: out.ed_min_sequence }
                        MarginRow { label: "Edge distance (strength)", stress: out.ed_actual, ms: out.strength_margin, allowable: out.ed_min_strength }
                        MarginRow { label: "Neck wall thickness", stress: out.wall_neck, ms: out.wall_neck / min_wall_neck() - 1.0, allowable: min_wall_neck() }
                    }

                    div { class: "bushing-detail-grid",
                        DetailField { label: "Retained install force", value: format!("{:.0} lbf", out.retained_install_force) }
                        DetailField { label: "Effective housing OD", value: format!("{:.4} in", out.effective_od_housing) }
                        DetailField { label: "Finite-plate factor \u{03c8}", value: format!("{:.3}", out.psi) }
                        DetailField { label: "Edge-distance ratio \u{03bb}", value: format!("{:.3}", out.lambda) }
                        DetailField { label: "Achieved interference", value: format!("{:.4} in", out.achieved_interference_tol.nominal) }
                        DetailField { label: "Solved OD band", value: format!("{:.4}\u{2013}{:.4} in", out.od_tol.lower, out.od_tol.upper) }
                        DetailField { label: "Neck wall (nominal)", value: format!("{:.4} in", out.wall_neck_nominal) }
                        if let Some(id) = out.cs_solved_id {
                            DetailField { label: "Internal CS (solved)", value: format!("\u{2300}{:.4} \u{00d7} {:.4} deep, {:.1}\u{00b0}", id.dia, id.depth, id.angle_deg) }
                        }
                        if let Some(od) = out.cs_solved_od {
                            DetailField { label: "External CS (solved)", value: format!("\u{2300}{:.4} \u{00d7} {:.4} deep, {:.1}\u{00b0}", od.dia, od.depth, od.angle_deg) }
                        }
                    }

                    div { class: "bushing-derivation-toggle",
                        button {
                            class: "link-button",
                            onclick: move |_| show_derivation.set(!show_derivation()),
                            if show_derivation() { "Hide derivation" } else { "Show derivation" }
                        }
                    }
                    if show_derivation() {
                        DerivationBlock { out: out.clone(), dark: dark() }
                    }
                }
            }
        }
    }
}

#[component]
fn BushingSection(title: &'static str, children: Element) -> Element {
    rsx! {
        details { class: "bushing-section", open: true,
            summary { "{title}" }
            div { class: "bushing-section-body", {children} }
        }
    }
}

/// A numeric field with its own display-text signal, decoupled from the
/// numeric `value` signal it reads/writes - NOT the naive
/// `value: "{value}"` pattern used elsewhere in this app (see CLAUDE.md's
/// "numeric `<input>` snap-back" note), because that pattern has a second
/// failure mode beyond the one already documented there: even a
/// *successful* parse can still clobber what the user is mid-typing.
/// Bushing dimensions are routinely < 1 (e.g. `0.375`), so typing one
/// means typing "0" then "." before any nonzero digit - "0".parse() and
/// "0.".parse() both succeed as `Ok(0.0)`, and Rust's `f64` `Display`
/// formats `0.0` back as `"0"`, not `"0."` - reformatting the controlled
/// value from the parsed float on every keystroke silently deletes the
/// decimal point the instant it's typed, making any value below 1
/// unenterable. Keeping a separate `text` signal that only ever gets
/// overwritten by what the user actually typed (never by reformatting a
/// successful parse) fixes this for every fractional value, not just this
/// one case. `text` is still resynced from `value` when it changes from
/// *outside* this field (e.g. the reamer picker setting `bore_dia`) - the
/// guard compares the current text's own parse against the new value so a
/// self-triggered update (this field's own `oninput`) never clobbers
/// itself, only a genuinely external change does.
#[component]
fn NumberField(label: &'static str, value: Signal<f64>, step: &'static str) -> Element {
    let mut value = value;
    let mut text = use_signal(|| format!("{}", value()));
    use_effect(move || {
        let v = value();
        if text.peek().parse::<f64>().ok() != Some(v) {
            text.set(format!("{v}"));
        }
    });
    rsx! {
        label { class: "field",
            span { class: "field-label", "{label}" }
            input {
                r#type: "number",
                step: "{step}",
                value: "{text}",
                oninput: move |e| {
                    let s = e.value();
                    if let Ok(v) = s.parse::<f64>() { value.set(v); }
                    text.set(s);
                },
            }
        }
    }
}

/// `select`/`option` renders on `blitz-dom` as every option's text
/// flattened together with no popup at all (see `components.rs`'s own
/// `Dropdown` doc comment and `docs/epic-ui-performance-and-design.md`'s
/// "Verified platform constraints" table) - the exact bug a user reported
/// here ("hardcoded text of all materials" instead of a real picker).
/// Reuses the same `Dropdown` component every other picker in this app
/// already uses, rather than reinventing it.
#[component]
fn MaterialField(label: &'static str, value: Signal<String>) -> Element {
    let mut value = value;
    let selected_label = bushing_solver::materials::get_material(&value()).name.to_string();
    let options: Vec<(&'static str, &'static str)> = MATERIALS.iter().map(|m| (m.id, m.name)).collect();
    rsx! {
        Dropdown {
            field_label: label.to_string(),
            selected_label,
            options,
            on_select: move |v: String| value.set(v),
        }
    }
}

/// Which of a countersink's three dimensions (diameter/depth/angle) a
/// given `CsGeometryField` renders - the mode determines which two are
/// direct user inputs and which one is derived (see `countersink.rs`'s
/// `CsMode`).
#[derive(Clone, Copy, PartialEq)]
enum CsField {
    Dia,
    Depth,
    Angle,
}

fn cs_field_is_direct_input(mode: CsMode, which: CsField) -> bool {
    match (mode, which) {
        (CsMode::DepthAngle, CsField::Dia) => false,
        (CsMode::DiaAngle, CsField::Depth) => false,
        (CsMode::DiaDepth, CsField::Angle) => false,
        _ => true,
    }
}

#[component]
fn CsModeField(mode: Signal<CsMode>) -> Element {
    let mut mode = mode;
    rsx! {
        div { class: "field",
            span { class: "field-label", "Countersink mode" }
            div { class: "chip-row",
                for (m , label) in [(CsMode::DepthAngle, "Depth + angle"), (CsMode::DiaAngle, "Dia + angle"), (CsMode::DiaDepth, "Dia + depth")] {
                    span {
                        class: if mode() == m { "chip selected" } else { "chip" },
                        onclick: move |_| mode.set(m),
                        "{label}"
                    }
                }
            }
        }
    }
}

/// One countersink dimension - an editable `NumberField` when `mode`
/// treats it as a direct input, or a read-only `DetailField` showing the
/// solved value when `mode` derives it from the other two.
#[component]
fn CsGeometryField(mode: CsMode, which: CsField, label: &'static str, value: Signal<f64>, step: &'static str, solved: Option<bushing_solver::countersink::CsCorner>) -> Element {
    if cs_field_is_direct_input(mode, which) {
        rsx! { NumberField { label, value, step } }
    } else {
        let derived = solved.map(|c| match which {
            CsField::Dia => c.dia,
            CsField::Depth => c.depth,
            CsField::Angle => c.angle_deg,
        });
        rsx! { DetailField { label, value: derived.map(|v| format!("{v:.4} (derived)")).unwrap_or_default() } }
    }
}

#[component]
fn SummaryCard(label: &'static str, value: String, badge: Option<(String, &'static str)>) -> Element {
    rsx! {
        div { class: "summary-card",
            span { class: "summary-card-label", "{label}" }
            span { class: "summary-card-value", "{value}" }
            if let Some((text, class)) = badge {
                span { class: "{class}", "{text}" }
            }
        }
    }
}

#[component]
fn DetailField(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "detail-field",
            span { class: "detail-field-label", "{label}" }
            span { class: "detail-field-value", "{value}" }
        }
    }
}

#[component]
fn MarginRow(label: &'static str, stress: f64, ms: f64, allowable: f64) -> Element {
    rsx! {
        div { class: "margin-row",
            span { class: "margin-row-label", "{label}" }
            span { class: "margin-row-demand", "demand {stress:.0}, allowable {allowable:.0}" }
            span { class: margin_class(ms), "MS {fmt_margin(ms)}" }
        }
    }
}

#[component]
fn ReamerPicker(target_in: f64, on_pick: EventHandler<ReamerEntry>, on_close: EventHandler<()>) -> Element {
    let matches: Vec<ReamerEntry> = reamers::nearest(target_in, 8).into_iter().cloned().collect();
    rsx! {
        div { class: "reamer-picker",
            div { class: "reamer-picker-header",
                span { "Nearest catalog reamers to {target_in:.4} in" }
                button { class: "reamer-picker-close", onclick: move |_| on_close.call(()), "\u{2715}" }
            }
            div { class: "reamer-picker-list",
                for entry in matches {
                    button {
                        class: "reamer-picker-row",
                        onclick: {
                            let entry = entry.clone();
                            move |_| on_pick.call(entry.clone())
                        },
                        span { class: "reamer-picker-size", "{entry.size_label}" }
                        span { class: "reamer-picker-nominal", "{entry.nominal_in:.4} in" }
                        span {
                            class: if entry.availability_tier == reamers::AvailabilityTier::Preferred { "soon-pill" } else { "reamer-picker-tier" },
                            "{tier_label(entry.availability_tier)}"
                        }
                    }
                }
            }
        }
    }
}

fn tier_label(tier: reamers::AvailabilityTier) -> &'static str {
    match tier {
        reamers::AvailabilityTier::Preferred => "Preferred",
        reamers::AvailabilityTier::Common => "Common",
        reamers::AvailabilityTier::Special => "Special",
    }
}

#[component]
fn DerivationBlock(out: bushing_solver::solve::BushingOutput, dark: bool) -> Element {
    rsx! {
        div { class: "derivation-block",
            for f in FORMULAS {
                div { class: "derivation-row",
                    img { class: "derivation-formula", src: formula_img_src(f, dark) }
                    span { class: "derivation-value", "{derivation_value(f.id, &out)}" }
                }
            }
        }
    }
}

fn derivation_value(id: &str, out: &bushing_solver::solve::BushingOutput) -> String {
    match id {
        "thermal_delta_interference" => format!("= {:.6} in", out.delta_thermal),
        "installed_outer_diameter" => format!("= {:.4} in", out.od_installed),
        "contact_pressure" => format!("= {:.0} psi", out.pressure),
        "hoop_stress_housing" => format!("= {:.0} psi", out.stress_hoop_housing),
        "hoop_stress_bushing" => format!("= {:.0} psi", out.stress_hoop_bushing),
        "install_force" => format!("= {:.0} lbf", out.install_force),
        "margin_of_safety" => format!("governing: {} = {}", out.governing.name, fmt_margin(out.governing.margin)),
        _ => String::new(),
    }
}
