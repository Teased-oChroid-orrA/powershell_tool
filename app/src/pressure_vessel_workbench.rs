//! Pressure Vessel Stress, Failure Mode & Minimum Thickness Analyzer UI -
//! issue #11 v1 (see `docs/issue-11-status.md` in the repo root for the
//! full gap analysis, scope decision, and explicit backlog).
//!
//! **Bushing Workbench visual language, exactly** - a guided step
//! sequence (`PvStep`/`PvStepperNav`, mirroring `bushing_workbench.rs`'s
//! own `Step`/`StepperNav`), a persistent design-status rail
//! (`PvStatusRail`), a PASS/REVIEW headline, a fabrication-style vessel-
//! specification summary card, value/whisker Checks gauges
//! (`crate::components::CheckGauge`, already shared), and a full
//! step-by-step derivation view with real KaTeX-rendered formula images
//! - approved from a mockup round (`docs/issue-11-phase-10.md`) after the
//! user asked for the Bushing Workbench's look and feel to be matched
//! exactly, with a minimum of 5 real derivation steps (this ships 8).
//!
//! `PvStep`/`PvStepperNav`/`PvStatusRail` are small, separate types from
//! `bushing_workbench.rs`'s own `Step`/`StepperNav`/`DesignStatusRail`
//! rather than genericized versions of them - those are tied to
//! bushing-specific types (`Step`'s own variants, `ToleranceStatus`) and
//! genericizing them risks the Bushing Workbench's own already-verified
//! behavior for a saving of a few dozen lines. `crate::components`
//! already holds what was safe and valuable to share
//! (`NumberField`/`MaterialField`/`CheckGauge`/`CheckRowData`).

use dioxus::prelude::*;

use mechanics_core::materials::get_material;
use pressure_vessel_solver::buckling::{evaluate_buckling, BucklingApplicability};
use pressure_vessel_solver::failure::{evaluate_failure_modes, governing, tresca_stress, von_mises_stress, MarginResult};
use pressure_vessel_solver::geometry::{classify, CylinderGeometry, GeometryClassification};
use pressure_vessel_solver::pressure::{EndCondition, PressureLoading};
use pressure_vessel_solver::stress::stress_at_inner_surface;
use pressure_vessel_solver::thickness::{solve_minimum_thickness, ThicknessSolverInputs, ThicknessSolverOutcome};

use crate::components::{fmt_margin, margin_class, CheckGauge, CheckRowData, MaterialField, NumberField};

fn margin_result_to_row(r: &MarginResult) -> CheckRowData {
    CheckRowData {
        label: r.name,
        at_least: false,
        decimals: 0,
        unit: "psi",
        nominal: r.applied,
        range: None,
        allowable: r.allowable,
        margin: r.margin,
    }
}

/// The 5 workflow steps - mirrors how a pressure-vessel design actually
/// gets specified (geometry, then loading, then material, then the
/// buckling-relevant support spacing, then the results/checks), not the
/// engine's own internal call order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum PvStep {
    #[default]
    Geometry,
    Pressure,
    Material,
    Buckling,
    Results,
}

const PV_STEPS: [PvStep; 5] = [PvStep::Geometry, PvStep::Pressure, PvStep::Material, PvStep::Buckling, PvStep::Results];

impl PvStep {
    fn label(self) -> &'static str {
        match self {
            PvStep::Geometry => "Geometry",
            PvStep::Pressure => "Pressure",
            PvStep::Material => "Material",
            PvStep::Buckling => "Buckling",
            PvStep::Results => "Results",
        }
    }
    fn number(self) -> u8 {
        PV_STEPS.iter().position(|s| *s == self).unwrap() as u8 + 1
    }
}

#[component]
fn PvStepperNav(current: Signal<PvStep>) -> Element {
    let mut current = current;
    rsx! {
        div { class: "bushing-stepper",
            for s in PV_STEPS {
                div {
                    class: if current() == s { "bushing-step-pill bushing-step-current" } else { "bushing-step-pill" },
                    onclick: move |_| current.set(s),
                    span { class: "bushing-step-num", "{s.number()}" }
                    span { "{s.label()}" }
                }
            }
        }
    }
}

/// Persistent design-status rail, visible regardless of which step is
/// open - same role as the Bushing Workbench's own `DesignStatusRail`,
/// simplified: this tool has no tolerance-band concept, so it's just a
/// plain `(name, margin)` checklist with a PASS/REVIEW header.
///
/// `on_jump` carries the clicked check's name (not just `()`) so the
/// Results step can highlight that specific row - it can't scroll to it:
/// this renderer has no JS engine at all (`dioxus-native`/Blitz, not a
/// WebView), so a `scrollIntoView`-style call isn't available here the
/// way it would be on `dioxus-desktop`. Highlighting is the closest
/// renderer-safe equivalent to "jump to".
#[component]
fn PvStatusRail(checks: Vec<(&'static str, f64)>, on_jump: EventHandler<&'static str>) -> Element {
    let total = checks.len();
    let passed = checks.iter().filter(|(_, margin)| margin.is_finite() && *margin >= 0.0).count();
    let all_pass = passed == total;
    rsx! {
        div { class: "bushing-status-rail",
            div { class: if all_pass { "bushing-status-head bushing-status-pass" } else { "bushing-status-head bushing-status-review" },
                span { class: "bushing-status-dot" }
                div {
                    div { class: "bushing-status-text", if all_pass { "PASS" } else { "REVIEW" } }
                    div { class: "bushing-status-sub", "{passed} / {total} checks passed" }
                }
            }
            div { class: "bushing-checklist",
                for (name , margin) in checks {
                    div {
                        class: if margin.is_finite() && margin < 0.0 { "bushing-check-row bushing-check-attn" } else { "bushing-check-row" },
                        onclick: move |_| on_jump.call(name),
                        span { class: if !margin.is_finite() { "bushing-check-dot neutral" } else if margin < 0.0 { "bushing-check-dot crit" } else if margin < 0.15 { "bushing-check-dot warn" } else { "bushing-check-dot ok" } }
                        span { class: "bushing-check-name", "{name}" }
                        span { class: "bushing-check-val mono", "{fmt_margin(margin)}" }
                    }
                }
            }
        }
    }
}

/// One derivation-view formula image + its id, matching
/// `bushing_workbench.rs`'s own `Formula`/`formula!` shape exactly but
/// kept as this file's own small copy rather than shared - see this
/// module's own top doc comment for why. `dir` lets the same macro pull
/// from either this tool's own `pv_formulas/` assets (5 new formulas) or
/// the Bushing Workbench's existing `bushing_formulas/` assets (3 steps
/// here are the exact same general Lame physics already rendered there -
/// real reuse of the actual asset files, not just the concept).
struct PvFormula {
    id: &'static str,
    dark_png: &'static [u8],
    light_png: &'static [u8],
}

macro_rules! pv_formula {
    ($dir:literal, $id:literal) => {
        PvFormula {
            id: $id,
            dark_png: include_bytes!(concat!("../assets/", $dir, "/", $id, "_dark.png")),
            light_png: include_bytes!(concat!("../assets/", $dir, "/", $id, "_light.png")),
        }
    };
}

static PV_FORMULAS: &[PvFormula] = &[
    pv_formula!("bushing_formulas", "radial_equilibrium_ode"),
    pv_formula!("bushing_formulas", "lame_trial_form"),
    pv_formula!("bushing_formulas", "lame_constants_solved"),
    pv_formula!("pv_formulas", "pv_hoop_at_inner_surface"),
    pv_formula!("pv_formulas", "pv_closed_end_axial_stress"),
    pv_formula!("pv_formulas", "pv_von_mises_stress"),
    pv_formula!("pv_formulas", "pv_tresca_stress"),
    pv_formula!("pv_formulas", "pv_windenburg_trilling"),
];

fn pv_formula_img_src(f: &PvFormula, dark: bool) -> String {
    use base64::Engine;
    let bytes = if dark { f.dark_png } else { f.light_png };
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[component]
pub fn PressureVesselWorkbench(dark: Signal<bool>) -> Element {
    let current_step = use_signal(PvStep::default);
    let outer_diameter = use_signal(|| 6.0_f64);
    let wall_thickness = use_signal(|| 1.0_f64);
    let internal_pressure = use_signal(|| 5000.0_f64);
    let external_pressure = use_signal(|| 0.0_f64);
    let mut closed_ends = use_signal(|| true);
    let material_id = use_signal(|| "al7075".to_string());
    let required_ms = use_signal(|| 0.0_f64);
    let unsupported_length = use_signal(|| 0.0_f64);
    let show_more_detail = use_signal(|| false);
    let highlighted_check = use_signal(|| None::<&'static str>);

    let outer_radius = (outer_diameter() / 2.0).max(0.0);
    let inner_radius = outer_radius - wall_thickness();

    let geometry = CylinderGeometry::new(inner_radius, outer_radius);
    let pressure = PressureLoading::new(
        internal_pressure(),
        external_pressure(),
        if closed_ends() { EndCondition::Closed } else { EndCondition::Open },
    );
    let material = *get_material(&material_id());

    rsx! {
        div { class: "panel bushing-page",
            PvStepperNav { current: current_step }
            match (geometry, pressure) {
                (Ok(geometry), Ok(pressure)) => {
                    let mut rows: Vec<MarginResult> = evaluate_failure_modes(&geometry, &pressure, &material);
                    let buckling_result = evaluate_buckling(&geometry, &pressure, &material, Some(unsupported_length()));
                    if let BucklingApplicability::Evaluated(ref b) = buckling_result {
                        rows.push(b.clone());
                    }
                    let governing_result = governing(&rows).clone();
                    let classification = classify(&geometry);
                    let thickness_outcome = solve_minimum_thickness(
                        &ThicknessSolverInputs { inner_radius: geometry.inner_radius, pressure, material, required_minimum_ms: required_ms() },
                        100,
                        1e-6,
                    );
                    let checks: Vec<(&'static str, f64)> = rows.iter().map(|r| (r.name, r.margin)).collect();

                    // Live derivation values - current inputs, not a
                    // worst-case-across-tolerance-band derivation like
                    // the Bushing Workbench's own (this tool has no
                    // tolerance-band concept in v1).
                    let inner = stress_at_inner_surface(&geometry, &pressure);
                    let (c1, c2) = mechanics_core::lame::lame_constants(geometry.inner_radius, geometry.outer_radius, pressure.internal_pressure, pressure.external_pressure);
                    let vm = von_mises_stress(&inner);
                    let tresca = tresca_stress(&inner);

                    rsx! {
                        div { class: "bushing-workspace-split",
                            div { class: "bushing-workspace",
                                match current_step() {
                                    PvStep::Geometry => rsx! {
                                        div { class: "bushing-card",
                                            h3 { class: "bushing-card-title", "01 \u{00b7} Geometry" }
                                            NumberField { label: "Outer diameter (in)", value: outer_diameter, step: "0.01" }
                                            NumberField { label: "Wall thickness (in)", value: wall_thickness, step: "0.01" }
                                            span { class: "derivation-note", style: "margin:0;", "Derived inner diameter: {(2.0 * inner_radius):.4} in" }
                                        }
                                    },
                                    PvStep::Pressure => rsx! {
                                        div { class: "bushing-card",
                                            h3 { class: "bushing-card-title", "02 \u{00b7} Pressure \u{0026} boundary condition" }
                                            NumberField { label: "Internal pressure (psi)", value: internal_pressure, step: "10" }
                                            NumberField { label: "External pressure (psi)", value: external_pressure, step: "10" }
                                            div { class: "field",
                                                span { class: "field-label", "End condition" }
                                                div { class: "chip-row",
                                                    span { class: if closed_ends() { "chip selected" } else { "chip" }, onclick: move |_| closed_ends.set(true), "Closed ends" }
                                                    span { class: if !closed_ends() { "chip selected" } else { "chip" }, onclick: move |_| closed_ends.set(false), "Open ends" }
                                                }
                                            }
                                            p { class: "derivation-note", style: "margin:0;",
                                                "Closed: a real end cap reacts pressure thrust here - or a downstream expansion joint absorbs it before reaching the far end, but the thrust is still fully present in this segment. Open: no cap and nothing downstream to react thrust - e.g. analyzing the run "
                                                strong { "past" }
                                                " an expansion joint. A cap on one end doesn't become \"open\" just because the member is long - the thrust travels through the wall wherever it's actually reacted, not less so with distance."
                                            }
                                        }
                                    },
                                    PvStep::Material => rsx! {
                                        div { class: "bushing-card",
                                            h3 { class: "bushing-card-title", "03 \u{00b7} Material \u{0026} requirement" }
                                            MaterialField { label: "Material", value: material_id }
                                            NumberField { label: "Required minimum MS", value: required_ms, step: "0.05" }
                                        }
                                    },
                                    PvStep::Buckling => rsx! {
                                        div { class: "bushing-card",
                                            h3 { class: "bushing-card-title", "04 \u{00b7} Support spacing (buckling)" }
                                            NumberField { label: "Unsupported length (in)", value: unsupported_length, step: "1" }
                                            p { class: "derivation-note", style: "margin:0;",
                                                "Distance between stiffening rings/supports. Only affects the Buckling check, and only when external pressure is present - leave at 0 if not applicable."
                                            }
                                        }
                                    },
                                    PvStep::Results => rsx! {
                                        div { class: if rows.iter().all(|r| r.margin.is_finite() && r.margin >= 0.0) { "bushing-headline pass" } else { "bushing-headline review" },
                                            div { class: "bushing-headline-status",
                                                span { class: "bushing-headline-dot" }
                                                div {
                                                    span { class: "bushing-headline-text", if rows.iter().all(|r| r.margin.is_finite() && r.margin >= 0.0) { "PASS" } else { "REVIEW" } }
                                                    span { class: "bushing-headline-sub", "{rows.iter().filter(|r| r.margin.is_finite() && r.margin >= 0.0).count()} / {rows.len()} checks passed" }
                                                }
                                            }
                                            div { class: "bushing-mini-stats",
                                                div { class: "bushing-mini-stat",
                                                    span { class: "bushing-mini-label", "Governing" }
                                                    span { class: margin_class(governing_result.margin), "{governing_result.name} \u{00b7} {fmt_margin(governing_result.margin)}" }
                                                }
                                                div { class: "bushing-mini-stat",
                                                    span { class: "bushing-mini-label", "Classification" }
                                                    span { class: "bushing-mini-val", if classification == GeometryClassification::ThinWall { "Thin-wall" } else { "Thick-wall" } }
                                                }
                                                div { class: "bushing-mini-stat",
                                                    span { class: "bushing-mini-label", "Wall thickness" }
                                                    span { class: "bushing-mini-val", "{geometry.wall_thickness():.4} in" }
                                                }
                                            }
                                        }

                                        div { class: "bushing-card fab-card",
                                            div { class: "bushing-card-head",
                                                h3 { class: "bushing-card-title", "Vessel specification summary" }
                                                span { class: "fab-badge ready", "Ready to fabricate" }
                                            }
                                            div { class: "fab-grid",
                                                div { class: "detail-field", span { class: "detail-field-label", "Outer diameter (finish to)" } span { class: "detail-field-value", "\u{2300}{outer_diameter():.4} in" } }
                                                div { class: "detail-field", span { class: "detail-field-label", "Inner diameter (bore to)" } span { class: "detail-field-value", "\u{2300}{(2.0 * inner_radius):.4} in" } }
                                                div { class: "detail-field", span { class: "detail-field-label", "Wall thickness" } span { class: "detail-field-value", "{wall_thickness():.4} in" } }
                                                div { class: "detail-field", span { class: "detail-field-label", "Design pressure" } span { class: "detail-field-value", "{internal_pressure():.0} psi internal, {external_pressure():.0} psi external" } }
                                                div { class: "detail-field", span { class: "detail-field-label", "End condition" } span { class: "detail-field-value", if closed_ends() { "Closed ends" } else { "Open ends" } } }
                                                div { class: "detail-field", span { class: "detail-field-label", "Material" } span { class: "detail-field-value", "{material.name}" } }
                                                div { class: "detail-field", span { class: "detail-field-label", "Governing failure mode" } span { class: "detail-field-value", "{governing_result.name} ({governing_result.critical_location.label()})" } }
                                            }
                                            match thickness_outcome {
                                                ThicknessSolverOutcome::Converged(ref sol) => rsx! {
                                                    p { class: "fab-note", "Minimum wall thickness solved for the same {required_ms():.2} required MS: {sol.wall_thickness:.4} in." }
                                                },
                                                ThicknessSolverOutcome::Infeasible { .. } => rsx! {
                                                    p { class: "fab-note", "Minimum-thickness solve: infeasible at this required MS - see the Minimum wall thickness detail below." }
                                                },
                                            }
                                        }

                                        div { class: "bushing-card",
                                            h3 { class: "bushing-card-title", "Checks" }
                                            div { class: "checks-list",
                                                for r in rows.iter() {
                                                    div {
                                                        class: if highlighted_check() == Some(r.name) { "check-row-highlight" } else { "" },
                                                        CheckGauge { row: margin_result_to_row(r) }
                                                    }
                                                }
                                            }
                                            match buckling_result {
                                                BucklingApplicability::NotApplicable => rsx! {
                                                    p { class: "check-note", "Buckling (external pressure): not applicable - no external pressure entered." }
                                                },
                                                BucklingApplicability::InsufficientData => rsx! {
                                                    p { class: "check-note", "Buckling (external pressure): insufficient data - enter an unsupported length in the Buckling step to evaluate." }
                                                },
                                                BucklingApplicability::OutsideValidityRange => rsx! {
                                                    p { class: "check-note", "Buckling (external pressure): outside the formulas' validity range (outer diameter / thickness \u{003c} 40)." }
                                                },
                                                BucklingApplicability::Evaluated(_) => rsx! {},
                                            }
                                        }

                                        match thickness_outcome {
                                            ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found } => rsx! {
                                                div { class: "bushing-alert bushing-alert-warn",
                                                    "No wall thickness up to {largest_radius_tried:.1} in (a {(largest_radius_tried / geometry.inner_radius):.0}\u{00d7} the inner radius) satisfies the required minimum MS. Best margin found: {fmt_margin(best_margin_found)}. Internal pressure alone bounds the achievable margin - see the Analysis note in docs/issue-11-phase-5.md."
                                                }
                                            },
                                            ThicknessSolverOutcome::Converged(_) => rsx! {},
                                        }

                                        div { class: "bushing-derivation-toggle",
                                            button {
                                                class: "link-button",
                                                onclick: {
                                                    let mut show_more_detail = show_more_detail;
                                                    move |_| show_more_detail.set(!show_more_detail())
                                                },
                                                if show_more_detail() { "Hide detail (derivation) \u{25b4}" } else { "Show more detail (derivation) \u{25be}" }
                                            }
                                        }
                                        if show_more_detail() {
                                            div { class: "bushing-card",
                                                h3 { class: "bushing-card-title", "Derivation" }
                                                p { class: "derivation-note",
                                                    "Steps 1\u{2013}4 derive the full thick-wall Lam\u{e9} solution from first principles (same physics, same rendered formulas, as the Bushing Workbench's own derivation view); steps 5\u{2013}7 specialize it to this vessel's axial stress and equivalent-stress failure criteria; step 8 is the finite-length buckling correction. Every step is evaluated at this vessel's real, current inputs - the "
                                                    strong { "Cited" }
                                                    " tag marks the one step (Windenburg-Trilling) that's a published closed-form result rather than re-derived from scratch here - see "
                                                    code { "buckling.rs" }
                                                    "'s own module doc for exactly why."
                                                }
                                                div { class: "derivation-block",
                                                    for f in PV_FORMULAS {
                                                        div { class: "derivation-row",
                                                            img { class: "derivation-formula", src: pv_formula_img_src(f, dark()) }
                                                            span { class: "derivation-value", "{pv_derivation_value(f.id, geometry.inner_radius, geometry.outer_radius, pressure.internal_pressure, pressure.external_pressure, c1, c2, &inner, vm, tresca, &governing_result, &buckling_result)}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                }
                            }
                            PvStatusRail { checks, on_jump: {
                                let mut current_step = current_step;
                                let mut highlighted_check = highlighted_check;
                                move |name| {
                                    current_step.set(PvStep::Results);
                                    highlighted_check.set(Some(name));
                                }
                            } }
                        }
                    }
                }
                (geometry_result, pressure_result) => rsx! {
                    div { class: "bushing-alert bushing-alert-fail",
                        if let Err(e) = geometry_result { "Invalid geometry: {e:?}." }
                        if let Err(e) = pressure_result { " Invalid pressure: {e:?}." }
                    }
                    div { class: "bushing-card",
                        h3 { class: "bushing-card-title", "Geometry" }
                        NumberField { label: "Outer diameter (in)", value: outer_diameter, step: "0.01" }
                        NumberField { label: "Wall thickness (in)", value: wall_thickness, step: "0.01" }
                    }
                    div { class: "bushing-card",
                        h3 { class: "bushing-card-title", "Pressure \u{0026} boundary condition" }
                        NumberField { label: "Internal pressure (psi)", value: internal_pressure, step: "10" }
                        NumberField { label: "External pressure (psi)", value: external_pressure, step: "10" }
                    }
                },
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn pv_derivation_value(
    id: &str,
    a: f64,
    b: f64,
    p_i: f64,
    p_o: f64,
    c1: f64,
    c2: f64,
    inner: &pressure_vessel_solver::stress::StressState,
    vm: f64,
    tresca: f64,
    governing_result: &MarginResult,
    buckling_result: &BucklingApplicability,
) -> String {
    match id {
        "radial_equilibrium_ode" => "Governing ODE for axisymmetric radial equilibrium - no numeric substitution, always true.".to_string(),
        "lame_trial_form" => "General trial form assumed for a thick-wall ring under uniform boundary pressure.".to_string(),
        "lame_constants_solved" => format!(
            "a = {a:.4} in, b = {b:.4} in, p_i = {p_i:.0} psi, p_o = {p_o:.0} psi \u{2192} C\u{2081} = {c1:.1} psi, C\u{2082} = {c2:.1} psi\u{00b7}in\u{00b2}"
        ),
        "pv_hoop_at_inner_surface" => format!(
            "Inner-surface hoop stress = {c1:.1} + {c2:.1}/{a:.4}\u{00b2} = {:.0} psi. Radial at inner surface = C\u{2081} \u{2212} C\u{2082}/a\u{00b2} = {:.0} psi (boundary condition satisfied exactly).",
            inner.hoop, inner.radial
        ),
        "pv_closed_end_axial_stress" => format!(
            "Closed-end axial stress, from force equilibrium on the end cap - exactly the first Lam\u{e9} constant, not a coincidence (see mechanics-core::lame doc). = {:.0} psi.",
            inner.axial
        ),
        "pv_von_mises_stress" => format!(
            "\u{03c3}\u{2081}={:.0} (radial), \u{03c3}\u{2082}={:.0} (hoop), \u{03c3}\u{2083}={:.0} (axial) \u{2192} \u{03c3}_vm = {vm:.0} psi.",
            inner.radial, inner.hoop, inner.axial
        ),
        "pv_tresca_stress" => format!(
            "{:.0} \u{2212} ({:.0}) = {tresca:.0} psi. {}",
            inner.hoop.max(inner.axial).max(inner.radial),
            inner.hoop.min(inner.axial).min(inner.radial),
            if governing_result.name == "Tresca (max shear)" { "Governs (lowest margin) for this vessel." } else { "" }
        ),
        "pv_windenburg_trilling" => match buckling_result {
            BucklingApplicability::Evaluated(b) => format!(
                "Combined with the fully-derived n=2 ring limit, p_cr = D(n\u{00b2}\u{2212}1)/r\u{00b3}, via max(). Governing critical pressure = {:.0} psi (applied external pressure {:.0} psi, MS = {}).",
                b.allowable, b.applied, fmt_margin(b.margin)
            ),
            BucklingApplicability::NotApplicable => "Combined with the fully-derived n=2 ring limit, p_cr = D(n\u{00b2}\u{2212}1)/r\u{00b3}, via max(). Not evaluated here: no external pressure entered.".to_string(),
            BucklingApplicability::InsufficientData => "Combined with the fully-derived n=2 ring limit, p_cr = D(n\u{00b2}\u{2212}1)/r\u{00b3}, via max(). Not evaluated here: no unsupported length entered.".to_string(),
            BucklingApplicability::OutsideValidityRange => "Combined with the fully-derived n=2 ring limit, p_cr = D(n\u{00b2}\u{2212}1)/r\u{00b3}, via max(). Not evaluated here: outside the formulas' thin-shell validity range.".to_string(),
        },
        _ => String::new(),
    }
}
