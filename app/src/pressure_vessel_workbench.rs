//! Pressure Vessel Stress, Failure Mode & Minimum Thickness Analyzer UI -
//! issue #11 v1 (see `docs/issue-11-status.md` in the repo root for the
//! full gap analysis, scope decision, and explicit backlog).
//!
//! **Single-page layout, not the Bushing Workbench's multi-step wizard** -
//! a deliberate scope decision, not a shortcut: v1's whole input set
//! (two radii, two pressures, an end condition, a material, and a
//! required margin - seven fields) is small enough that a stepper would
//! be ceremony without benefit. The Bushing Workbench's own stepper
//! exists because that tool has 30+ inputs across genuinely distinct
//! concerns (geometry, countersink, material, fit, environment). Reuses
//! the same visual language and components either way -
//! `crate::components`'s `NumberField`/`MaterialField`/`CheckGauge`/
//! `CheckRowData` were extracted specifically so this tool could depend
//! on them without duplicating the Bushing Workbench's own code.

use dioxus::prelude::*;

use mechanics_core::materials::get_material;
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
    let inner_radius = use_signal(|| 2.0_f64);
    let mut outer_radius = use_signal(|| 3.0_f64);
    let internal_pressure = use_signal(|| 5000.0_f64);
    let external_pressure = use_signal(|| 0.0_f64);
    let mut closed_ends = use_signal(|| true);
    let material_id = use_signal(|| "al7075".to_string());
    let required_ms = use_signal(|| 0.0_f64);

    let geometry = CylinderGeometry::new(inner_radius(), outer_radius());
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
                    let rows: Vec<MarginResult> = evaluate_failure_modes(&geometry, &pressure, &material);
                    let governing_result = governing(&rows).clone();
                    let all_pass = rows.iter().all(|r| r.margin.is_finite() && r.margin >= 0.0);
                    let classification = classify(&geometry);
                    let thickness_outcome = solve_minimum_thickness(
                        &ThicknessSolverInputs { inner_radius: geometry.inner_radius, pressure, material, required_minimum_ms: required_ms() },
                        100,
                        1e-6,
                    );
                    rsx! {
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
                                    span { class: "bushing-mini-label", "Geometry classification" }
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
                            h3 { class: "bushing-card-title", "Geometry" }
                            div { class: "field-row",
                                NumberField { label: "Inner radius (in)", value: inner_radius, step: "0.01" }
                                NumberField { label: "Outer radius (in)", value: outer_radius, step: "0.01" }
                            }
                        }

                        div { class: "bushing-card",
                            h3 { class: "bushing-card-title", "Pressure \u{0026} boundary condition" }
                            div { class: "field-row",
                                NumberField { label: "Internal pressure (psi)", value: internal_pressure, step: "10" }
                                NumberField { label: "External pressure (psi)", value: external_pressure, step: "10" }
                            }
                            div { class: "field",
                                span { class: "field-label", "End condition" }
                                div { class: "chip-row",
                                    span { class: if closed_ends() { "chip selected" } else { "chip" }, onclick: move |_| closed_ends.set(true), "Closed ends" }
                                    span { class: if !closed_ends() { "chip selected" } else { "chip" }, onclick: move |_| closed_ends.set(false), "Open ends" }
                                }
                            }
                        }

                        div { class: "bushing-card",
                            h3 { class: "bushing-card-title", "Material \u{0026} requirement" }
                            div { class: "field-row",
                                MaterialField { label: "Material", value: material_id }
                                NumberField { label: "Required minimum MS", value: required_ms, step: "0.05" }
                            }
                        }

                        div { class: "bushing-card",
                            h3 { class: "bushing-card-title", "Checks" }
                            div { class: "checks-list",
                                for r in rows.iter() {
                                    CheckGauge { row: margin_result_to_row(r) }
                                }
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
                                        onclick: move |_| outer_radius.set(sol.outer_radius),
                                        "Apply solved outer radius \u{2192} Geometry"
                                    }
                                },
                                ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found } => rsx! {
                                    div { class: "bushing-alert bushing-alert-warn",
                                        "No wall thickness up to {largest_radius_tried:.1} in (a {(largest_radius_tried / geometry.inner_radius):.0}\u{00d7} the inner radius) satisfies the required minimum MS. Best margin found: {fmt_margin(best_margin_found)}. Internal pressure alone bounds the achievable margin - see the Analysis note in docs/issue-11-phase-5.md."
                                    }
                                },
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
                        div { class: "field-row",
                            NumberField { label: "Inner radius (in)", value: inner_radius, step: "0.01" }
                            NumberField { label: "Outer radius (in)", value: outer_radius, step: "0.01" }
                        }
                    }
                    div { class: "bushing-card",
                        h3 { class: "bushing-card-title", "Pressure \u{0026} boundary condition" }
                        div { class: "field-row",
                            NumberField { label: "Internal pressure (psi)", value: internal_pressure, step: "10" }
                            NumberField { label: "External pressure (psi)", value: external_pressure, step: "10" }
                        }
                    }
                },
            }
        }
    }
}
