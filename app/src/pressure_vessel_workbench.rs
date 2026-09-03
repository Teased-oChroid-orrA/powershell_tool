//! Pressure Vessel Stress, Failure Mode & Minimum Thickness Analyzer UI -
//! issue #11 v1 (see `docs/issue-11-status.md` in the repo root for the
//! full gap analysis, scope decision, and explicit backlog).
//!
//! **Input-rail layout** (`.input-rail-layout`, `main.rs`) - a fixed-
//! width left rail holding every input, one field per row, with the
//! headline/checks/thickness-solve results in a flexible right column.
//! Chosen (mockup round, three options presented) specifically because it
//! **cannot overflow** the way a `.field-row` of several fields side by
//! side can at narrow widths: `.bushing-card .field-row .field {
//! flex: none }` (tuned for the Bushing Workbench, where it's still
//! correct) forces each field to its natural width with no shrinking, and
//! this page's longer labels ("Required minimum MS", "Internal pressure
//! (psi)") could exceed the card at anything less than a wide window.
//! Rather than touch that shared rule and risk the Bushing Workbench's
//! own already-verified layout, this page just never puts more than one
//! field in a row - see `main.rs`'s `.input-rail-layout` doc comment.

use dioxus::prelude::*;

use mechanics_core::materials::get_material;
use pressure_vessel_solver::buckling::{evaluate_buckling, BucklingApplicability};
use pressure_vessel_solver::failure::{evaluate_failure_modes, governing, MarginResult};
use pressure_vessel_solver::geometry::{classify, CylinderGeometry, GeometryClassification};
use pressure_vessel_solver::pressure::{EndCondition, PressureLoading};
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

#[component]
pub fn PressureVesselWorkbench(dark: Signal<bool>) -> Element {
    let _ = dark;
    // Real-world convention: a vessel is specified by outer diameter and
    // wall thickness (what a drawing/spec sheet actually calls out), not
    // inner/outer radius - inner radius is derived and shown as a
    // read-only caption, matching the "input vs. derived" visual
    // distinction this app already uses elsewhere (`src-chip`/`derived`
    // badges in the Bushing Workbench).
    let mut outer_diameter = use_signal(|| 6.0_f64);
    let mut wall_thickness = use_signal(|| 1.0_f64);
    let internal_pressure = use_signal(|| 5000.0_f64);
    let external_pressure = use_signal(|| 0.0_f64);
    let mut closed_ends = use_signal(|| true);
    let material_id = use_signal(|| "al7075".to_string());
    let required_ms = use_signal(|| 0.0_f64);
    // 0.0 means "not specified" - evaluate_buckling treats a non-positive
    // unsupported length the same as "no length given" (InsufficientData),
    // so this field doubles as its own presence flag without needing an
    // Option-aware input widget.
    let unsupported_length = use_signal(|| 0.0_f64);

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
            match (geometry, pressure) {
                (Ok(geometry), Ok(pressure)) => {
                    let mut rows: Vec<MarginResult> = evaluate_failure_modes(&geometry, &pressure, &material);
                    let buckling_result = evaluate_buckling(&geometry, &pressure, &material, Some(unsupported_length()));
                    if let BucklingApplicability::Evaluated(ref b) = buckling_result {
                        rows.push(b.clone());
                    }
                    let governing_result = governing(&rows).clone();
                    let all_pass = rows.iter().all(|r| r.margin.is_finite() && r.margin >= 0.0);
                    let classification = classify(&geometry);
                    let thickness_outcome = solve_minimum_thickness(
                        &ThicknessSolverInputs { inner_radius: geometry.inner_radius, pressure, material, required_minimum_ms: required_ms() },
                        100,
                        1e-6,
                    );
                    rsx! {
                        div { class: "input-rail-layout",
                            div { class: "input-rail",
                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "Geometry" }
                                    NumberField { label: "Outer diameter (in)", value: outer_diameter, step: "0.01" }
                                    NumberField { label: "Wall thickness (in)", value: wall_thickness, step: "0.01" }
                                    span { class: "derivation-note", style: "margin:0;",
                                        "Derived inner diameter: {(2.0 * inner_radius):.4} in"
                                    }
                                }
                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "Pressure \u{0026} boundary condition" }
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
                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "Support spacing (buckling)" }
                                    NumberField { label: "Unsupported length (in)", value: unsupported_length, step: "1" }
                                    p { class: "derivation-note", style: "margin:0;",
                                        "Distance between stiffening rings/supports. Only affects the Buckling check below, and only when external pressure is present - leave at 0 if not applicable."
                                    }
                                }
                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "Material \u{0026} requirement" }
                                    MaterialField { label: "Material", value: material_id }
                                    NumberField { label: "Required minimum MS", value: required_ms, step: "0.05" }
                                }
                            }
                            div { class: "results-column",
                                div { class: if all_pass { "bushing-headline pass" } else { "bushing-headline review" },
                                    div { class: "bushing-headline-status",
                                        span { class: "bushing-headline-dot" }
                                        div {
                                            span { class: "bushing-headline-text", if all_pass { "PASS" } else { "REVIEW" } }
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
                                            span { class: "bushing-mini-val",
                                                if classification == GeometryClassification::ThinWall { "Thin-wall" } else { "Thick-wall" }
                                            }
                                        }
                                        div { class: "bushing-mini-stat",
                                            span { class: "bushing-mini-label", "Wall thickness" }
                                            span { class: "bushing-mini-val", "{geometry.wall_thickness():.4} in" }
                                        }
                                    }
                                }
                                p { class: "derivation-note",
                                    "Geometry classification is for engineering interpretation only - both classifications use the exact same full Lam\u{e9} thick-wall stress solution below, never a simplified thin-wall approximation."
                                }

                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "Checks" }
                                    div { class: "checks-list",
                                        for r in rows.iter() {
                                            CheckGauge { row: margin_result_to_row(r) }
                                        }
                                    }
                                    match buckling_result {
                                        BucklingApplicability::NotApplicable => rsx! {
                                            p { class: "derivation-note", style: "margin:0;", "Buckling: not applicable - no external pressure." }
                                        },
                                        BucklingApplicability::InsufficientData => rsx! {
                                            p { class: "derivation-note", style: "margin:0;", "Buckling: insufficient data - enter an unsupported length above to evaluate." }
                                        },
                                        BucklingApplicability::OutsideValidityRange => rsx! {
                                            p { class: "derivation-note", style: "margin:0;", "Buckling: outside the formulas' validity range (outer diameter / thickness < 40 - too thick for thin-shell buckling theory)." }
                                        },
                                        BucklingApplicability::Evaluated(_) => rsx! {},
                                    }
                                }

                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "Minimum wall thickness" }
                                    match thickness_outcome {
                                        ThicknessSolverOutcome::Converged(sol) => rsx! {
                                            div { class: "bushing-detail-grid",
                                                div { class: "detail-field",
                                                    span { class: "detail-field-label", "Minimum outer radius" }
                                                    span { class: "detail-field-value", "{sol.outer_radius:.4} in" }
                                                }
                                                div { class: "detail-field",
                                                    span { class: "detail-field-label", "Minimum wall thickness" }
                                                    span { class: "detail-field-value", "{sol.wall_thickness:.4} in" }
                                                }
                                                div { class: "detail-field",
                                                    span { class: "detail-field-label", "Governing mode at solution" }
                                                    span { class: "detail-field-value", "{sol.governing.name} ({sol.governing.critical_location.label()})" }
                                                }
                                            }
                                            button {
                                                class: "link-button",
                                                onclick: move |_| {
                                                    outer_diameter.set(2.0 * sol.outer_radius);
                                                    wall_thickness.set(sol.wall_thickness);
                                                },
                                                "Apply solved thickness \u{2192} Geometry"
                                            }
                                        },
                                        ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found } => rsx! {
                                            div { class: "bushing-alert bushing-alert-warn",
                                                "No wall thickness up to {largest_radius_tried:.1} in (a {(largest_radius_tried / geometry.inner_radius):.0}\u{00d7} the inner radius) satisfies the required minimum MS. Best margin found: {fmt_margin(best_margin_found)}. Internal pressure alone bounds the achievable margin - see the Analysis note in docs/issue-11-phase-5.md."
                                            }
                                        },
                                    }
                                    p { class: "derivation-note", style: "margin:0;",
                                        "Note: the minimum-thickness solve considers the four stress-based checks only, not buckling - buckling depends on the unsupported length, not the wall thickness/inner-radius relationship this solver varies."
                                    }
                                }
                            }
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
