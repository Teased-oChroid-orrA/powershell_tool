//! Bushing Workbench - straight-bushing interference-fit calculator.
//! Ported from ~/Claude/Projects/engineering.toolbox's TypeScript bushing
//! workbench (see `bushing-solver`'s `Cargo.toml` doc comment for the
//! exact scope decision - straight bushings only, no countersink/flange/
//! duty-process-approval layers - and `docs/bushing-workbench-status.md`
//! in this repo for the full writeup). This file owns the UI only; all
//! the actual physics lives in `bushing-solver`, verified against the
//! real production TS engine's own output (`bushing-solver/tests/differential.rs`).

use dioxus::prelude::*;

use bushing_solver::materials::MATERIALS;
use bushing_solver::reamers::{self, ReamerEntry};
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use bushing_solver::tolerance::ToleranceStatus;

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
    };
    let out = compute(&input);

    let mat_bushing_props = *bushing_solver::materials::get_material(&mat_bushing());
    let mat_housing_props = *bushing_solver::materials::get_material(&mat_housing());

    rsx! {
        div { class: "panel bushing-layout",
            div { class: "bushing-inputs",
                BushingSection { title: "Geometry",
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
                BushingSection { title: "Interference & tolerance",
                    NumberField { label: "Target diametral interference (in)", value: interference, step: "0.0001" }
                    div { class: "field-row",
                        NumberField { label: "Interference tol +", value: interference_tol_plus, step: "0.0001" }
                        NumberField { label: "Interference tol \u{2212}", value: interference_tol_minus, step: "0.0001" }
                    }
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

                    div { class: "bushing-margins",
                        MarginRow { label: "Housing hoop stress", stress: out.stress_hoop_housing, ms: out.housing_ms, allowable: mat_housing_props.sy_ksi * 1000.0 }
                        MarginRow { label: "Bushing hoop stress", stress: out.stress_hoop_bushing, ms: out.bushing_ms, allowable: mat_bushing_props.sy_ksi * 1000.0 }
                        MarginRow { label: "Edge distance (sequencing)", stress: out.ed_actual, ms: out.sequence_margin, allowable: out.ed_min_sequence }
                        MarginRow { label: "Edge distance (strength)", stress: out.ed_actual, ms: out.strength_margin, allowable: out.ed_min_strength }
                    }

                    div { class: "bushing-detail-grid",
                        DetailField { label: "Retained install force", value: format!("{:.0} lbf", out.retained_install_force) }
                        DetailField { label: "Effective housing OD", value: format!("{:.4} in", out.effective_od_housing) }
                        DetailField { label: "Finite-plate factor \u{03c8}", value: format!("{:.3}", out.psi) }
                        DetailField { label: "Edge-distance ratio \u{03bb}", value: format!("{:.3}", out.lambda) }
                        DetailField { label: "Achieved interference", value: format!("{:.4} in", out.achieved_interference_tol.nominal) }
                        DetailField { label: "Solved OD band", value: format!("{:.4}\u{2013}{:.4} in", out.od_tol.lower, out.od_tol.upper) }
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

#[component]
fn NumberField(label: &'static str, value: Signal<f64>, step: &'static str) -> Element {
    let mut value = value;
    rsx! {
        label { class: "field",
            span { class: "field-label", "{label}" }
            input {
                r#type: "number",
                step: "{step}",
                value: "{value}",
                oninput: move |e| { if let Ok(v) = e.value().parse::<f64>() { value.set(v); } },
            }
        }
    }
}

#[component]
fn MaterialField(label: &'static str, value: Signal<String>) -> Element {
    let mut value = value;
    rsx! {
        label { class: "field",
            span { class: "field-label", "{label}" }
            select {
                value: "{value}",
                oninput: move |e| value.set(e.value()),
                for m in MATERIALS {
                    option { value: "{m.id}", "{m.name}" }
                }
            }
        }
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
