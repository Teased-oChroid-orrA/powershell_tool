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
use bushing_solver::geometry::{BushingSectionInput, BushingType, IdType};

use crate::bushing_visualizer;
use mechanics_core::materials::MATERIALS;
use bushing_solver::reamers::{self, ReamerEntry};
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use bushing_solver::tolerance::{EnforcementPolicy, ToleranceStatus};

use crate::components::{fmt_margin, margin_class, margin_dot_class, CheckGauge, CheckRowData, Dropdown, FieldGroup};

/// Hand-drawn inline SVG, same convention `main.rs`'s nav-rail
/// `icon_*` functions already use - not a Unicode glyph. A prior version
/// of the visualizer's expand button used `"\u{29B2}"` (an Arrows-B
/// symbol), which the actual system UI font has no glyph for and
/// rendered as an unrelated fallback icon in the real app, even though it
/// looked fine as plain text here in the editor.
fn icon_expand() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" }
        }
    }
}

fn icon_close() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M18 6L6 18M6 6l12 12" }
        }
    }
}

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
    formula!("radial_equilibrium_ode"),
    formula!("lame_trial_form"),
    formula!("lame_boundary_conditions"),
    formula!("lame_constants_solved"),
    formula!("lame_radial_stress_field"),
    formula!("lame_hoop_stress_field"),
    formula!("lame_axial_stress"),
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

/// The exact 6 checks the Results view computes margins for - factored
/// into one place so `DesignStatusRail` and the Checks gauges can never
/// silently disagree about which checks exist or what their margins are.
/// This exact bug already happened once: an earlier version of the rail
/// read `out.candidates` (a real engine field, but a different and
/// narrower "governing check" set - edge distance and wall thickness only)
/// and showed "PASS" while the Results table, built from these same 6
/// values independently, showed a real failing hoop-stress margin
/// `out.candidates` never included at all.
fn check_rows(
    out: &bushing_solver::solve::BushingOutput,
    mat_housing_sy_psi: f64,
    mat_bushing_sy_psi: f64,
    min_wall_straight: f64,
    min_wall_neck: f64,
) -> Vec<CheckRowData> {
    vec![
        CheckRowData {
            label: "Housing hoop stress",
            at_least: false,
            decimals: 0,
            unit: "psi",
            nominal: out.stress_hoop_housing,
            range: Some((out.stress_hoop_housing_range.min, out.stress_hoop_housing_range.max)),
            allowable: mat_housing_sy_psi,
            margin: out.housing_ms,
        },
        CheckRowData {
            label: "Bushing hoop stress",
            at_least: false,
            decimals: 0,
            unit: "psi",
            nominal: out.stress_hoop_bushing,
            range: Some((out.stress_hoop_bushing_range.min, out.stress_hoop_bushing_range.max)),
            allowable: mat_bushing_sy_psi,
            margin: out.bushing_ms,
        },
        CheckRowData {
            label: "Edge distance (sequencing)",
            at_least: true,
            decimals: 3,
            unit: "\u{00d7}D",
            nominal: out.ed_actual,
            range: None,
            allowable: out.ed_min_sequence,
            margin: out.sequence_margin,
        },
        CheckRowData {
            label: "Edge distance (strength)",
            at_least: true,
            decimals: 3,
            unit: "\u{00d7}D",
            nominal: out.ed_actual,
            range: None,
            allowable: out.ed_min_strength,
            margin: out.strength_margin,
        },
        CheckRowData {
            label: "Straight wall thickness",
            at_least: true,
            decimals: 4,
            unit: "in",
            nominal: out.wall_straight,
            range: Some((out.wall_straight_range.min, out.wall_straight_range.max)),
            allowable: min_wall_straight,
            margin: out.wall_straight / min_wall_straight - 1.0,
        },
        CheckRowData {
            label: "Neck wall thickness",
            at_least: true,
            decimals: 4,
            unit: "in",
            nominal: out.wall_neck,
            range: None,
            allowable: min_wall_neck,
            margin: out.wall_neck / min_wall_neck - 1.0,
        },
    ]
}

fn design_checks(rows: &[CheckRowData]) -> Vec<(&'static str, f64)> {
    rows.iter().map(|r| (r.label, r.margin)).collect()
}

/// The workflow steps the workspace is organized around - mirrors how a
/// repair-bushing design actually gets made (define the repair/housing,
/// then the bushing's own geometry, then material, then the fit, then
/// check the design, then the finalize/report step), not the engine's
/// internal `BushingInputs` field grouping. Only the current step's cards
/// render in the workspace at a time - see `BushingWorkbench`'s `match
/// current_step()` - so the whole page never has to show more than one
/// step's worth of fields, cards, and tables at once.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Step {
    #[default]
    Repair,
    Geometry,
    Material,
    Fit,
    Analysis,
    Results,
}

const STEPS: [Step; 6] = [Step::Repair, Step::Geometry, Step::Material, Step::Fit, Step::Analysis, Step::Results];

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Repair => "Repair",
            Step::Geometry => "Geometry",
            Step::Material => "Material",
            Step::Fit => "Fit",
            Step::Analysis => "Analysis",
            Step::Results => "Results",
        }
    }
    fn number(self) -> u8 {
        STEPS.iter().position(|s| *s == self).unwrap() as u8 + 1
    }
}

#[component]
fn StepperNav(current: Signal<Step>) -> Element {
    let mut current = current;
    rsx! {
        div { class: "bushing-stepper",
            for s in STEPS {
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

/// Persistent design-health rail (per the source UX brief's \u{00a7}7: "a
/// constant sense of where am I with this design," visible regardless of
/// which step is open). Takes a plain `(name, margin)` list the caller
/// builds - NOT `bushing_solver::solve::BushingOutput.candidates`
/// directly. That field is a real, but narrower, thing: the engine's own
/// "governing check" reduction (`solve.rs` - edge distance and wall-
/// thickness margins only, matching the TS reference's own `governing`
/// concept), not every check the Results table shows. A prior version of
/// this rail used `.candidates` directly and was caught claiming PASS
/// while the Results table showed a real failing hoop-stress margin
/// (housing/bushing hoop stress margins were never in `.candidates` at
/// all) - this rail must show the exact same 6 checks the Results table
/// does, built from the same source values, or the two can silently
/// disagree again. Clicking any row jumps to the Results step - this is
/// the one deliberately coarse piece of cross-step wiring in Phase 1
/// ("jump to where this is analyzed," not a per-field pinpoint).
#[component]
fn DesignStatusRail(checks: Vec<(&'static str, f64)>, tolerance_status: ToleranceStatus, on_jump: EventHandler<()>) -> Element {
    let total = checks.len() + usize::from(tolerance_status == ToleranceStatus::Infeasible);
    let passed = checks.iter().filter(|(_, margin)| margin.is_finite() && *margin >= 0.0).count();
    let all_pass = passed == checks.len() && tolerance_status != ToleranceStatus::Infeasible;
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
                        onclick: move |_| on_jump.call(()),
                        span { class: margin_dot_class(margin) }
                        span { class: "bushing-check-name", "{name}" }
                        span { class: "bushing-check-val mono", "{fmt_margin(margin)}" }
                    }
                }
                if tolerance_status == ToleranceStatus::Infeasible {
                    div {
                        class: "bushing-check-row bushing-check-attn",
                        onclick: move |_| on_jump.call(()),
                        span { class: "bushing-check-dot crit" }
                        span { class: "bushing-check-name", "Tolerance feasibility" }
                        span { class: "bushing-check-val mono", "\u{2014}" }
                    }
                }
            }
        }
    }
}

/// A warning/failure banner that's actually actionable, not just
/// informational text: names the step that owns the field most likely to
/// fix it and jumps there on click - per explicit request ("provide
/// options to fix the issue... takes me to the required section"). This
/// doesn't compute a corrective *value* (that needs a closed-form inverse
/// per check, real future work) - it gets you to the right place to fix
/// it yourself, which is most of the value for a first pass.
#[component]
fn ActionAlert(is_fail: bool, message: String, action_label: &'static str, on_jump: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: if is_fail { "bushing-alert bushing-alert-fail bushing-alert-action" } else { "bushing-alert bushing-alert-warn bushing-alert-action" },
            span { class: "bushing-alert-msg", "{message}" }
            button { class: "bushing-alert-action-btn", onclick: move |_| on_jump.call(()), "{action_label}" }
        }
    }
}

/// Presents a result the same way its driving inputs were entered -
/// nominal plus the range the tolerance bands actually propagate into
/// (`bushing_solver::solve::RangedValue`) - not a bare point value, per
/// explicit request. Collapses to just the nominal (no redundant
/// "(X-X)") when the range is degenerate (zero-width tolerance bands, or
/// this specific result genuinely doesn't vary - both real, not bugs).
fn fmt_ranged(r: bushing_solver::solve::RangedValue, decimals: usize, unit: &str) -> String {
    if (r.max - r.min).abs() < 1e-9 {
        format!("{:.*} {unit}", decimals, r.nominal)
    } else {
        format!("{:.*} {unit} ({:.*}\u{2013}{:.*})", decimals, r.nominal, decimals, r.min, decimals, r.max)
    }
}

/// Same presentation as `fmt_ranged`, for the engine's other range type
/// (`bushing_solver::tolerance::ToleranceRange` - a resolved tolerance
/// band, `.lower`/`.upper`/`.nominal`, as opposed to `RangedValue`'s
/// `.min`/`.max`/`.nominal` for a propagated result range). Two structs,
/// not one, because they come from genuinely different places in the
/// engine (direct tolerance resolution vs. re-evaluating a formula at
/// two extremes) - see each type's own doc comment.
fn fmt_tol_range(r: bushing_solver::tolerance::ToleranceRange, decimals: usize, unit: &str) -> String {
    if (r.upper - r.lower).abs() < 1e-9 {
        format!("{:.*} {unit}", decimals, r.nominal)
    } else {
        format!("{:.*} {unit} ({:.*}\u{2013}{:.*})", decimals, r.nominal, decimals, r.lower, decimals, r.upper)
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
    let mut visualizer_lightbox_open = use_signal(|| false);
    let mut show_more_detail = use_signal(|| false);
    let mut current_step = use_signal(Step::default);

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
    let cs_angle_tol_plus = use_signal(|| 0.0_f64);
    let cs_angle_tol_minus = use_signal(|| 0.0_f64);

    let ext_cs_mode = use_signal(CsMode::default);
    let ext_cs_dia = use_signal(|| 0.6_f64);
    let ext_cs_depth = use_signal(|| 0.06_f64);
    let ext_cs_angle = use_signal(|| 100.0_f64);
    let ext_cs_dia_tol_plus = use_signal(|| 0.002_f64);
    let ext_cs_dia_tol_minus = use_signal(|| 0.0_f64);
    let ext_cs_depth_tol_plus = use_signal(|| 0.005_f64);
    let ext_cs_depth_tol_minus = use_signal(|| 0.0_f64);
    let ext_cs_angle_tol_plus = use_signal(|| 0.0_f64);
    let ext_cs_angle_tol_minus = use_signal(|| 0.0_f64);

    let mut enforcement_enabled = use_signal(|| false);

    let mut assembly_thermal_assist_enabled = use_signal(|| false);
    let assembly_housing_temperature = use_signal(|| 70.0_f64);
    let assembly_bushing_temperature = use_signal(|| -20.0_f64);

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
        cs_angle_tol_plus: cs_angle_tol_plus(),
        cs_angle_tol_minus: cs_angle_tol_minus(),
        ext_cs_mode: ext_cs_mode(),
        ext_cs_dia: ext_cs_dia(),
        ext_cs_depth: ext_cs_depth(),
        ext_cs_angle: ext_cs_angle(),
        ext_cs_dia_tol_plus: ext_cs_dia_tol_plus(),
        ext_cs_dia_tol_minus: ext_cs_dia_tol_minus(),
        ext_cs_depth_tol_plus: ext_cs_depth_tol_plus(),
        ext_cs_depth_tol_minus: ext_cs_depth_tol_minus(),
        ext_cs_angle_tol_plus: ext_cs_angle_tol_plus(),
        ext_cs_angle_tol_minus: ext_cs_angle_tol_minus(),
        enforcement: EnforcementPolicy { enabled: enforcement_enabled(), ..EnforcementPolicy::default() },
        bore_capability: None,
        assembly_housing_temperature: assembly_thermal_assist_enabled().then_some(assembly_housing_temperature()),
        assembly_bushing_temperature: assembly_thermal_assist_enabled().then_some(assembly_bushing_temperature()),
    };
    let out = compute(&input);

    let mat_bushing_props = *mechanics_core::materials::get_material(&mat_bushing());
    let mat_housing_props = *mechanics_core::materials::get_material(&mat_housing());
    let rows = check_rows(
        &out,
        mat_housing_props.sy_ksi * 1000.0,
        mat_bushing_props.sy_ksi * 1000.0,
        min_wall_straight(),
        min_wall_neck(),
    );
    let checks_passed = rows.iter().filter(|r| r.margin.is_finite() && r.margin >= 0.0).count();
    let checks_total = rows.len();
    let all_checks_pass = checks_passed == checks_total;
    let governing_str = format!("{} \u{00b7} {}", out.governing.name, fmt_margin(out.governing.margin));
    let install_force_str = fmt_ranged(out.install_force_range, 0, "lbf");
    let achieved_interference_str = fmt_tol_range(out.achieved_interference_tol, 4, "in");
    let contact_pressure_str = fmt_ranged(out.pressure_range, 0, "psi");
    let install_delta_label: &'static str = if out.install_delta > 0.0 { "Interference at install" } else { "Clearance at install" };
    let install_delta_note = if out.install_delta > 0.0 { "still needs force to seat" } else { "slip fit, no press force needed" };
    let install_method_str = if assembly_thermal_assist_enabled() {
        format!(
            "Shrink fit \u{2014} chill bushing to {:.0}\u{00b0}F, heat housing to {:.0}\u{00b0}F before assembly",
            assembly_bushing_temperature(),
            assembly_housing_temperature()
        )
    } else {
        "Press fit \u{2014} no thermal assist".to_string()
    };

    let section_input = BushingSectionInput {
        bore_dia: bore_dia(),
        housing_len: housing_len(),
        housing_width: housing_width(),
        id_bushing: id_bushing(),
        bushing_type: bushing_type(),
        id_type: id_type(),
        flange_od: flange_od(),
        flange_thk: flange_thk(),
        od_bushing: out.od_installed,
        cs_external: out.cs_solved_od.map(|c| (c.dia, c.depth)),
        cs_internal: out.cs_solved_id.map(|c| (c.dia, c.depth)),
    };
    let stress_overlay = bushing_visualizer::StressOverlay {
        housing_stress_psi: out.stress_hoop_housing,
        housing_ms: out.housing_ms,
        bushing_stress_psi: out.stress_hoop_bushing,
        bushing_ms: out.bushing_ms,
        bushing_field: out.bushing_stress_field.clone(),
        housing_field: out.housing_stress_field.clone(),
    };
    let neck_governs = id_type() == IdType::Countersink && out.wall_neck < out.wall_straight;
    let section_params = bushing_solver::geometry::resolve_bushing_section_params(&section_input);
    let crop = bushing_visualizer::detail_crop(&section_input, &section_params, neck_governs);
    let detail_label = if neck_governs { "Detail \u{2014} neck wall (governing)" } else { "Detail \u{2014} straight wall" };

    rsx! {
        // Single flowing column, matching the approved mockup exactly - no
        // fixed-width sidebar, no independent inner scroll regions, no
        // accordion/collapse chrome. `.stage`'s own page-level scroll
        // (main.rs) is the only scrollbar for the whole tool.
        div { class: "panel bushing-page",

            if visualizer_lightbox_open() {
                div {
                    class: "bushing-viz-lightbox-backdrop",
                    div {
                        class: "bushing-viz-lightbox-card",
                        button {
                            class: "bushing-viz-lightbox-close",
                            onclick: move |_| visualizer_lightbox_open.set(false),
                            {icon_close()}
                        }
                        img {
                            class: "bushing-viz-lightbox-img",
                            src: bushing_visualizer::section_svg_data_uri(&section_input, out.achieved_interference_tol.nominal, stress_overlay, dark()),
                        }
                    }
                }
            }

            StepperNav { current: current_step }

            div { class: "bushing-workspace-split",
                div { class: "bushing-workspace",
                    match current_step() {
                        Step::Repair => rsx! {
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "01 \u{00b7} Repair" }
                                p { class: "bushing-card-sub", "The housing bore/repair this bushing goes into." }
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
                                div { class: "field-row",
                                    NumberField { label: "Bushing ID (in)", value: id_bushing, step: "0.001" }
                                    NumberField { label: "Housing length (in)", value: housing_len, step: "0.01" }
                                    NumberField { label: "Housing width (in)", value: housing_width, step: "0.01" }
                                    NumberField { label: "Edge distance (in)", value: edge_dist, step: "0.01" }
                                }
                            }
                        },
                        Step::Geometry => rsx! {
                            if id_type() != IdType::Countersink && bushing_type() != BushingType::Countersink && bushing_type() != BushingType::Flanged {
                                div { class: "bushing-card",
                                    h3 { class: "bushing-card-title", "02 \u{00b7} Geometry" }
                                    p { class: "bushing-card-sub", "This is a straight bushing on both ID and OD - no countersink or flange geometry to define. Change OD/ID geometry on the Repair step to add one." }
                                }
                            }
                            if id_type() == IdType::Countersink {
                                div { class: "bushing-card",
                                    div { class: "bushing-card-head",
                                        h3 { class: "bushing-card-title", "02 \u{00b7} Internal countersink (ID)" }
                                        CsModeField { mode: cs_mode }
                                    }
                                    div { class: "spec-table-wrap",
                                        table { class: "spec-table",
                                            thead {
                                                tr {
                                                    th { "Dimension" }
                                                    th { class: "num", "Nominal" }
                                                    th { class: "num", "Tol \u{2212}" }
                                                    th { class: "num", "Tol +" }
                                                    th { class: "num", "Range" }
                                                    th { "Source" }
                                                }
                                            }
                                            tbody {
                                                CsSpecRow { mode: cs_mode(), which: CsField::Depth, label: "Depth, in", decimals: 4, step: "0.001", value: cs_depth, tol_plus: cs_depth_tol_plus, tol_minus: cs_depth_tol_minus, solved: out.cs_solved_id, range: out.cs_internal_depth_tol }
                                                CsSpecRow { mode: cs_mode(), which: CsField::Angle, label: "Angle, deg", decimals: 1, step: "0.1", value: cs_angle, tol_plus: cs_angle_tol_plus, tol_minus: cs_angle_tol_minus, solved: out.cs_solved_id, range: out.cs_internal_angle_tol }
                                                CsSpecRow { mode: cs_mode(), which: CsField::Dia, label: "Diameter, in", decimals: 4, step: "0.001", value: cs_dia, tol_plus: cs_dia_tol_plus, tol_minus: cs_dia_tol_minus, solved: out.cs_solved_id, range: out.cs_internal_dia_tol }
                                            }
                                        }
                                    }
                                }
                            }
                            if bushing_type() == BushingType::Countersink || bushing_type() == BushingType::Flanged {
                                div { class: "bushing-card",
                                    div { class: "bushing-card-head",
                                        h3 { class: "bushing-card-title", "02 \u{00b7} External geometry (OD)" }
                                        if bushing_type() == BushingType::Countersink {
                                            CsModeField { mode: ext_cs_mode }
                                        }
                                    }
                                    if bushing_type() == BushingType::Countersink {
                                        div { class: "spec-table-wrap",
                                            table { class: "spec-table",
                                                thead {
                                                    tr {
                                                        th { "Dimension" }
                                                        th { class: "num", "Nominal" }
                                                        th { class: "num", "Tol \u{2212}" }
                                                        th { class: "num", "Tol +" }
                                                        th { class: "num", "Range" }
                                                        th { "Source" }
                                                    }
                                                }
                                                tbody {
                                                    CsSpecRow { mode: ext_cs_mode(), which: CsField::Depth, label: "Depth, in", decimals: 4, step: "0.001", value: ext_cs_depth, tol_plus: ext_cs_depth_tol_plus, tol_minus: ext_cs_depth_tol_minus, solved: out.cs_solved_od, range: out.cs_external_depth_tol }
                                                    CsSpecRow { mode: ext_cs_mode(), which: CsField::Angle, label: "Angle, deg", decimals: 1, step: "0.1", value: ext_cs_angle, tol_plus: ext_cs_angle_tol_plus, tol_minus: ext_cs_angle_tol_minus, solved: out.cs_solved_od, range: out.cs_external_angle_tol }
                                                    CsSpecRow { mode: ext_cs_mode(), which: CsField::Dia, label: "Diameter, in", decimals: 4, step: "0.001", value: ext_cs_dia, tol_plus: ext_cs_dia_tol_plus, tol_minus: ext_cs_dia_tol_minus, solved: out.cs_solved_od, range: out.cs_external_dia_tol }
                                                }
                                            }
                                        }
                                    }
                                    if bushing_type() == BushingType::Flanged {
                                        div { class: "field-row",
                                            NumberField { label: "Flange OD (in)", value: flange_od, step: "0.01" }
                                            NumberField { label: "Flange thickness (in)", value: flange_thk, step: "0.001" }
                                        }
                                    }
                                }
                            }
                        },
                        Step::Material => rsx! {
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "03 \u{00b7} Material" }
                                div { class: "field-row",
                                    MaterialField { label: "Housing material", value: mat_housing }
                                    MaterialField { label: "Bushing material", value: mat_bushing }
                                }
                            }
                        },
                        Step::Fit => rsx! {
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "04 \u{00b7} Fit" }
                                p { class: "bushing-card-sub", "Bore and target interference, with the tolerance each is held to." }
                                div { class: "spec-table-wrap",
                                    table { class: "spec-table",
                                        thead {
                                            tr {
                                                th { "Dimension" }
                                                th { class: "num", "Nominal" }
                                                th { class: "num", "Tol \u{2212}" }
                                                th { class: "num", "Tol +" }
                                                th { class: "num", "Range" }
                                                th { "Source" }
                                            }
                                        }
                                        tbody {
                                            PlainSpecRow { label: "Bore, in", decimals: 4, step: "0.0001", value: bore_dia, tol_plus: bore_tol_plus, tol_minus: bore_tol_minus, range: out.bore_tol }
                                            PlainSpecRow { label: "Interference, in", decimals: 4, step: "0.0001", value: interference, tol_plus: interference_tol_plus, tol_minus: interference_tol_minus, range: out.interference_tol }
                                        }
                                    }
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
                                label { class: "field field-checkbox",
                                    input {
                                        r#type: "checkbox",
                                        checked: enforcement_enabled(),
                                        oninput: move |e| enforcement_enabled.set(e.checked()),
                                    }
                                    span { "Auto-tighten bore tolerance to meet target interference" }
                                }
                            }
                        },
                        Step::Analysis => rsx! {
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "05 \u{00b7} Analysis \u{2014} acceptance criteria" }
                                FieldGroup { label: "Minimum wall thickness",
                                    div { class: "field-row",
                                        NumberField { label: "Straight wall (in)", value: min_wall_straight, step: "0.001" }
                                        NumberField { label: "Neck wall (in)", value: min_wall_neck, step: "0.001" }
                                    }
                                }
                            }
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "05 \u{00b7} Analysis \u{2014} environment & install" }
                                div { class: "field-row",
                                    NumberField { label: "Friction coefficient", value: friction, step: "0.01" }
                                    NumberField { label: "Temperature change, \u{0394}T (\u{00b0}F)", value: d_t, step: "1" }
                                }
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
                                div { class: "field-row",
                                    NumberField { label: "Edge load angle (deg)", value: edge_load_angle_deg, step: "1" }
                                    NumberField { label: "Applied edge load (lbf)", value: load, step: "10" }
                                }
                                FieldGroup { label: "Shrink-fit install assist",
                                    label { class: "field field-checkbox",
                                        input {
                                            r#type: "checkbox",
                                            checked: assembly_thermal_assist_enabled(),
                                            oninput: move |e| assembly_thermal_assist_enabled.set(e.checked()),
                                        }
                                        span { "Bushing chilled / housing heated at install (distinct from in-service \u{0394}T above)" }
                                    }
                                    if assembly_thermal_assist_enabled() {
                                        div { class: "field-row",
                                            NumberField { label: "Housing temp at install (\u{00b0}F)", value: assembly_housing_temperature, step: "1" }
                                            NumberField { label: "Bushing temp at install (\u{00b0}F)", value: assembly_bushing_temperature, step: "1" }
                                        }
                                    }
                                }
                            }
                        },
                        Step::Results => rsx! {
                            div { class: if all_checks_pass { "bushing-headline pass" } else { "bushing-headline review" },
                                div { class: "bushing-headline-status",
                                    span { class: "bushing-headline-dot" }
                                    div {
                                        span { class: "bushing-headline-text", if all_checks_pass { "PASS" } else { "REVIEW" } }
                                        span { class: "bushing-headline-sub", "{checks_passed} / {checks_total} checks passed" }
                                    }
                                }
                                div { class: "bushing-mini-stats",
                                    div { class: "bushing-mini-stat",
                                        span { class: "bushing-mini-label", "Governing" }
                                        span { class: margin_class(out.governing.margin), "{governing_str}" }
                                    }
                                    div { class: "bushing-mini-stat",
                                        span { class: "bushing-mini-label", "Install force" }
                                        span { class: "bushing-mini-val", "{install_force_str}" }
                                    }
                                    div { class: "bushing-mini-stat",
                                        span { class: "bushing-mini-label", "Achieved interference" }
                                        span { class: "bushing-mini-val", "{achieved_interference_str}" }
                                    }
                                    div { class: "bushing-mini-stat",
                                        span { class: "bushing-mini-label", "Contact pressure" }
                                        span { class: "bushing-mini-val", "{contact_pressure_str}" }
                                    }
                                }
                            }
                            if out.fail_straight {
                                ActionAlert {
                                    is_fail: true,
                                    message: format!("Straight wall thickness ({:.4} in) is below the minimum ({:.4} in).", out.wall_straight, min_wall_straight()),
                                    action_label: "Adjust bushing ID \u{2192} Repair",
                                    on_jump: move |_| current_step.set(Step::Repair),
                                }
                            }
                            if out.fail_neck {
                                ActionAlert {
                                    is_fail: true,
                                    message: format!("Neck wall thickness ({:.4} in) is below the minimum ({:.4} in).", out.wall_neck, min_wall_neck()),
                                    action_label: "Adjust countersink \u{2192} Geometry",
                                    on_jump: move |_| current_step.set(Step::Geometry),
                                }
                            }
                            if out.sequence_margin < 0.0 {
                                ActionAlert {
                                    is_fail: true,
                                    message: format!("Edge distance sequencing margin is {} - housing width needs to grow by roughly {:.3} in.", fmt_margin(out.sequence_margin), (out.ed_min_sequence - out.ed_actual) * out.bore_tol.nominal),
                                    action_label: "Adjust housing width \u{2192} Repair",
                                    on_jump: move |_| current_step.set(Step::Repair),
                                }
                            }
                            if out.delta_total <= 0.0 {
                                ActionAlert {
                                    is_fail: false,
                                    message: "Net interference is zero or negative after thermal correction - this is a clearance fit, not a press fit.".to_string(),
                                    action_label: "Adjust interference \u{2192} Fit",
                                    on_jump: move |_| current_step.set(Step::Fit),
                                }
                            }
                            if out.tolerance_status == ToleranceStatus::Infeasible {
                                ActionAlert {
                                    is_fail: false,
                                    message: "Bore and interference tolerance bands don't fully overlap - the achieved-interference range shown is collapsed to a point estimate, not a real tolerance band.".to_string(),
                                    action_label: "Review tolerances \u{2192} Fit",
                                    on_jump: move |_| current_step.set(Step::Fit),
                                }
                            }
                            if out.tolerance_status == ToleranceStatus::Clamped {
                                ActionAlert {
                                    is_fail: false,
                                    message: "OD nominal was clamped to keep the fit inside the requested interference tolerance window (bore tolerance auto-adjustment was applied).".to_string(),
                                    action_label: "Review auto-tighten \u{2192} Fit",
                                    on_jump: move |_| current_step.set(Step::Fit),
                                }
                            }
                            div { class: "bushing-card fab-card",
                                div { class: "bushing-card-head",
                                    h3 { class: "bushing-card-title", "Fabrication \u{0026} install summary" }
                                    span { class: if all_checks_pass { "fab-badge ready" } else { "fab-badge review" }, if all_checks_pass { "Ready to machine" } else { "Needs review" } }
                                }
                                div { class: "fab-grid",
                                    DetailField { label: "Housing bore (ream to)", value: format!("\u{2300}{:.4} in ({:.4}\u{2013}{:.4})", out.bore_tol.nominal, out.bore_tol.lower, out.bore_tol.upper) }
                                    DetailField { label: "Bushing OD (finish to)", value: format!("\u{2300}{:.4} in ({:.4}\u{2013}{:.4})", out.od_tol.nominal, out.od_tol.lower, out.od_tol.upper) }
                                    DetailField { label: "Diametral interference (in-service)", value: fmt_tol_range(out.achieved_interference_tol, 4, "in") }
                                    DetailField {
                                        label: install_delta_label,
                                        value: format!("{:.4} in \u{2014} {}", out.install_delta, install_delta_note),
                                    }
                                    DetailField {
                                        label: "Edge distance",
                                        value: format!("{:.3} in (needs \u{2265} {:.3} in)", edge_dist(), out.ed_min_sequence.max(out.ed_min_strength) * out.bore_tol.nominal),
                                    }
                                    div { class: "detail-field fab-item wide",
                                        span { class: "detail-field-label", "Install method" }
                                        span { class: "detail-field-value", "{install_method_str}" }
                                    }
                                    DetailField { label: "Install force", value: format!("{} at assembly temp", fmt_ranged(out.install_force_range, 0, "lbf")) }
                                    DetailField { label: "Retained force (in-service)", value: fmt_ranged(out.retained_install_force_range, 0, "lbf") }
                                    DetailField { label: "Housing material", value: mat_housing_props.name.to_string() }
                                    DetailField { label: "Bushing material", value: mat_bushing_props.name.to_string() }
                                }
                                p { class: "fab-note", "Install figures assume a friction coefficient of {friction()}. Reamer-pick the housing bore to the band above before pressing; interference at install accounts for any shrink-fit thermal assist configured in Analysis." }
                            }
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "Checks" }
                                div { class: "checks-list",
                                    for r in rows.iter() {
                                        CheckGauge { row: r.clone() }
                                    }
                                }
                            }
                            div { class: "bushing-derivation-toggle",
                                button {
                                    class: "link-button",
                                    onclick: move |_| show_more_detail.set(!show_more_detail()),
                                    if show_more_detail() { "Hide detail (derived quantities + derivation) \u{25b4}" } else { "Show more detail (derived quantities + derivation) \u{25be}" }
                                }
                            }
                            if show_more_detail() {
                            div { class: "bushing-card",
                                div { class: "bushing-viz-panes",
                                    div { class: "bushing-viz-pane bushing-viz-overview",
                                        span { class: "bushing-viz-tag", "Overview" }
                                        img { class: "bushing-viz-img", src: bushing_visualizer::geometry_crop_svg_data_uri(&section_input, dark(), None, 130.0) }
                                    }
                                    div { class: "bushing-viz-pane bushing-viz-detail",
                                        span { class: "bushing-viz-tag", "{detail_label}" }
                                        img { class: "bushing-viz-img", src: bushing_visualizer::geometry_crop_svg_data_uri(&section_input, dark(), Some(crop), 340.0) }
                                        button {
                                            class: "bushing-viz-expand",
                                            title: "Expand",
                                            onclick: move |_| visualizer_lightbox_open.set(true),
                                            {icon_expand()}
                                        }
                                    }
                                }
                            }
                                div { class: "bushing-detail-grid",
                                    DetailField { label: "Contact pressure", value: fmt_ranged(out.pressure_range, 0, "psi") }
                                    DetailField { label: "Minimum straight wall (input)", value: format!("{:.4} in", min_wall_straight()) }
                                    DetailField { label: "Minimum neck wall (input)", value: format!("{:.4} in", min_wall_neck()) }
                                    DetailField { label: "Neck wall (worst-case)", value: format!("{:.4} in", out.wall_neck) }
                                    DetailField { label: "Neck wall (nominal)", value: format!("{:.4} in", out.wall_neck_nominal) }
                                    DetailField { label: "Retained install force (in-service)", value: fmt_ranged(out.retained_install_force_range, 0, "lbf") }
                                    DetailField { label: "Install force (at assembly)", value: fmt_ranged(out.install_force_range, 0, "lbf") }
                                    DetailField { label: "Install pressure (at assembly)", value: format!("{:.0} psi", out.install_pressure) }
                                    if assembly_thermal_assist_enabled() {
                                        DetailField { label: "Assembly thermal delta", value: format!("{:+.5} in", out.assembly_thermal_delta) }
                                    }
                                    DetailField { label: "Axial stress, housing", value: format!("{:.0} psi", out.stress_axial_housing) }
                                    DetailField { label: "Axial stress, bushing", value: format!("{:.0} psi", out.stress_axial_bushing) }
                                    DetailField { label: "Effective housing OD", value: format!("{:.4} in", out.effective_od_housing) }
                                    DetailField { label: "Finite-plate factor \u{03c8}", value: format!("{:.3}", out.psi) }
                                    DetailField { label: "Edge-distance ratio \u{03bb}", value: format!("{:.3}", out.lambda) }
                                    DetailField { label: "Achieved interference", value: fmt_tol_range(out.achieved_interference_tol, 4, "in") }
                                    DetailField { label: "Solved OD band", value: format!("{:.4}\u{2013}{:.4} in", out.od_tol.lower, out.od_tol.upper) }
                                    if let Some(id) = out.cs_solved_id {
                                        DetailField { label: "Internal CS (solved)", value: format!("\u{2300}{:.4} \u{00d7} {:.4} deep, {:.1}\u{00b0}", id.dia, id.depth, id.angle_deg) }
                                    }
                                    if let Some(od) = out.cs_solved_od {
                                        DetailField { label: "External CS (solved)", value: format!("\u{2300}{:.4} \u{00d7} {:.4} deep, {:.1}\u{00b0}", od.dia, od.depth, od.angle_deg) }
                                    }
                                }
                                DerivationBlock { out: out.clone(), dark: dark(), nu_housing: mat_housing_props.nu, nu_bushing: mat_bushing_props.nu }
                            }
                        },
                    }
                }
                DesignStatusRail {
                    checks: design_checks(&rows),
                    tolerance_status: out.tolerance_status,
                    on_jump: move |_| current_step.set(Step::Results),
                }
            }
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
    let selected_label = mechanics_core::materials::get_material(&value()).name.to_string();
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
/// given `CsSpecRow` renders - the mode determines which two are direct
/// user inputs and which one is derived (see `countersink.rs`'s
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

/// Bare "lower\u{2013}upper" (or a single value when the range is
/// degenerate), no unit - the spec table's column header already carries
/// the unit, so repeating it on every row like `fmt_tol_range` does would
/// just be noise.
fn fmt_range_bare(r: bushing_solver::tolerance::ToleranceRange, decimals: usize) -> String {
    if (r.upper - r.lower).abs() < 1e-9 {
        format!("{:.*}", decimals, r.nominal)
    } else {
        format!("{:.*}\u{2013}{:.*}", decimals, r.lower, decimals, r.upper)
    }
}

/// Same controlled-input discipline as `NumberField` (a shadow `text`
/// signal so a user can type transiently-invalid text like "1." or "-"
/// without a snap-back, `value` only ever set on a successful parse -
/// `CLAUDE.md`'s documented numeric-input rule), rendered as a bare table
/// cell input instead of a labeled `field` block.
#[component]
fn SpecNumberInput(value: Signal<f64>, step: &'static str) -> Element {
    let mut value = value;
    let mut text = use_signal(|| format!("{}", value()));
    use_effect(move || {
        let v = value();
        if text.peek().parse::<f64>().ok() != Some(v) {
            text.set(format!("{v}"));
        }
    });
    rsx! {
        input {
            class: "spec-input",
            r#type: "number",
            step: "{step}",
            value: "{text}",
            oninput: move |e| {
                let s = e.value();
                if let Ok(v) = s.parse::<f64>() {
                    value.set(v);
                }
                text.set(s);
            },
        }
    }
}

/// One row of a countersink spec-sheet table: Dimension / Nominal / Tol
/// \u{2212} / Tol + / Range / Source. `which` decides, via
/// `cs_field_is_direct_input`, whether this row is editable (Source:
/// Direct) or a read-only solved value (Source: Derived) - the same
/// mode-vs-field logic decides which fields are direct vs. derived, laid
/// out as a table row with the propagated tolerance `range` (already
/// correctly resolved for both cases in `bushing-solver::solve::compute`)
/// always visible instead of requiring a separate detail lookup.
#[component]
fn CsSpecRow(mode: CsMode, which: CsField, label: &'static str, decimals: usize, step: &'static str, value: Signal<f64>, tol_plus: Signal<f64>, tol_minus: Signal<f64>, solved: Option<bushing_solver::countersink::CsCorner>, range: Option<bushing_solver::tolerance::ToleranceRange>) -> Element {
    let is_direct = cs_field_is_direct_input(mode, which);
    let solved_value = solved.map(|c| match which {
        CsField::Dia => c.dia,
        CsField::Depth => c.depth,
        CsField::Angle => c.angle_deg,
    });
    let solved_text = solved_value.map(|v| format!("{v:.*}", decimals)).unwrap_or_default();
    let range_text = range.map(|r| fmt_range_bare(r, decimals)).unwrap_or_else(|| "\u{2014}".to_string());
    rsx! {
        tr { class: if is_direct { "spec-row" } else { "spec-row spec-row-derived" },
            td { "{label}" }
            if is_direct {
                td { class: "num", SpecNumberInput { value, step } }
                td { class: "num", SpecNumberInput { value: tol_minus, step } }
                td { class: "num", SpecNumberInput { value: tol_plus, step } }
            } else {
                td { class: "num mono", "{solved_text}" }
                td { class: "num mono", "\u{2014}" }
                td { class: "num mono", "\u{2014}" }
            }
            td { class: "num range-cell mono", "{range_text}" }
            td {
                span {
                    class: if is_direct { "src-chip src-direct" } else { "src-chip src-derived" },
                    if is_direct { "Direct" } else { "Derived" }
                }
            }
        }
    }
}

/// A spec-table row for a plain direct-input dimension with its own
/// tolerance (bore, target interference) - no derived/direct split to
/// track since these are never solved from other dimensions, unlike a
/// countersink's `CsSpecRow`.
#[component]
fn PlainSpecRow(label: &'static str, decimals: usize, step: &'static str, value: Signal<f64>, tol_plus: Signal<f64>, tol_minus: Signal<f64>, range: bushing_solver::tolerance::ToleranceRange) -> Element {
    let range_text = fmt_range_bare(range, decimals);
    rsx! {
        tr { class: "spec-row",
            td { "{label}" }
            td { class: "num", SpecNumberInput { value, step } }
            td { class: "num", SpecNumberInput { value: tol_minus, step } }
            td { class: "num", SpecNumberInput { value: tol_plus, step } }
            td { class: "num range-cell mono", "{range_text}" }
            td { span { class: "src-chip src-direct", "Direct" } }
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
fn DerivationBlock(out: bushing_solver::solve::BushingOutput, dark: bool, nu_housing: f64, nu_bushing: f64) -> Element {
    let worst = WorstCaseLame::compute(&out, nu_housing, nu_bushing);
    rsx! {
        div { class: "derivation-block",
            p { class: "derivation-note",
                "Steps 3\u{2013}9 below derive the full thick-wall (Lam\u{e9}) cylinder solution from first principles - radial equilibrium through the substituted stress fields - then evaluate it at the "
                strong { "worst-case interference" }
                " within the achieved-interference tolerance band (\u{0394} = {worst.delta_worst:.5} in, contact pressure p = {worst.p_worst:.0} psi), the same extreme the Checks gauges' upper whisker already shows, so the numbers here are never a separate, potentially-inconsistent worst case."
            }
            for f in FORMULAS {
                div { class: "derivation-row",
                    img { class: "derivation-formula", src: formula_img_src(f, dark) }
                    span { class: "derivation-value", "{derivation_value(f.id, &out, &worst)}" }
                }
            }
        }
    }
}

/// The full Lamé thick-wall derivation evaluated at the worst-case
/// (maximum) interference within the achieved-interference tolerance
/// band - the same extreme `out.pressure_range.max`/
/// `out.stress_hoop_*_range.max` already represent, computed once here so
/// every derivation step's displayed number is consistent with those
/// existing fields rather than a separately re-derived worst case.
/// Region radii come straight from the already-computed
/// `bushing_stress_field`/`housing_stress_field` endpoints (their first/
/// last sample's own `.r`) rather than a new input, so this can't drift
/// from the geometry the rest of the app already displays.
struct WorstCaseLame {
    delta_worst: f64,
    p_worst: f64,
    a_bushing: f64,
    b_bushing: f64,
    a_housing: f64,
    b_housing: f64,
    c1_bushing: f64,
    c2_bushing: f64,
    c1_housing: f64,
    c2_housing: f64,
    sigma_theta_bushing_worst: f64,
    sigma_theta_housing_worst: f64,
    sigma_z_bushing_worst: f64,
    sigma_z_housing_worst: f64,
}

impl WorstCaseLame {
    fn compute(out: &bushing_solver::solve::BushingOutput, nu_housing: f64, nu_bushing: f64) -> Self {
        let a_bushing = out.bushing_stress_field.first().map(|s| s.r).unwrap_or(0.0);
        let b_bushing = out.bushing_stress_field.last().map(|s| s.r).unwrap_or(0.0);
        let a_housing = out.housing_stress_field.first().map(|s| s.r).unwrap_or(0.0);
        let b_housing = out.housing_stress_field.last().map(|s| s.r).unwrap_or(0.0);
        let p_worst = out.pressure_range.max;
        let (c1_bushing, c2_bushing) = mechanics_core::lame::lame_constants(a_bushing, b_bushing, 0.0, p_worst);
        let (c1_housing, c2_housing) = mechanics_core::lame::lame_constants(a_housing, b_housing, p_worst, 0.0);
        // Housing hoop stress is tensile (>=0), so its worst case is the
        // numerically LARGEST value - `.max`. Bushing hoop stress is
        // compressive (<=0), so its worst case (largest magnitude) is
        // the numerically SMALLEST (most negative) value - `.min`, not
        // `.max`. Both correspond to the exact same physical extreme
        // (maximum achieved interference, i.e. `p_worst` above) - only
        // which RangedValue field holds it differs, because `ranged()`
        // orders by numeric value, not by which achieved-interference
        // extreme produced it (see solve.rs's own `ranged()` doc).
        let sigma_theta_bushing_worst = out.stress_hoop_bushing_range.min;
        let sigma_theta_housing_worst = out.stress_hoop_housing_range.max;
        let axial_scale = out.axial_constraint_factor * out.axial_length_factor;
        // sigma_r at the loaded interface equals -p_worst by the very
        // boundary condition that solved C1/C2 (step 3) - not
        // independently computed, so this can never disagree with it.
        let sigma_z_bushing_worst = axial_scale * nu_bushing * (-p_worst + sigma_theta_bushing_worst);
        let sigma_z_housing_worst = axial_scale * nu_housing * (-p_worst + sigma_theta_housing_worst);
        Self {
            delta_worst: out.achieved_interference_tol.upper + out.delta_thermal,
            p_worst,
            a_bushing,
            b_bushing,
            a_housing,
            b_housing,
            c1_bushing,
            c2_bushing,
            c1_housing,
            c2_housing,
            sigma_theta_bushing_worst,
            sigma_theta_housing_worst,
            sigma_z_bushing_worst,
            sigma_z_housing_worst,
        }
    }
}

fn derivation_value(id: &str, out: &bushing_solver::solve::BushingOutput, worst: &WorstCaseLame) -> String {
    match id {
        "thermal_delta_interference" => format!("= {:.6} in", out.delta_thermal),
        "installed_outer_diameter" => format!("= {:.4} in", out.od_installed),
        "contact_pressure" => format!("= {:.0} psi", out.pressure),
        "radial_equilibrium_ode" => "governing ODE - no numeric substitution, always true.".to_string(),
        "lame_trial_form" => "general form assumed for a thick-wall ring under uniform boundary pressure.".to_string(),
        "lame_boundary_conditions" => format!(
            "worst case: \u{0394} = {:.5} in \u{2192} p = {:.0} psi. Bushing ring: a={:.4} in, b={:.4} in, p_i=0, p_o=p. Housing ring: a={:.4} in, b={:.4} in, p_i=p, p_o=0.",
            worst.delta_worst, worst.p_worst, worst.a_bushing, worst.b_bushing, worst.a_housing, worst.b_housing
        ),
        "lame_constants_solved" => format!(
            "worst case: bushing C\u{2081}={:.1} psi, C\u{2082}={:.6} psi\u{00b7}in\u{00b2}. housing C\u{2081}={:.1} psi, C\u{2082}={:.6} psi\u{00b7}in\u{00b2}.",
            worst.c1_bushing, worst.c2_bushing, worst.c1_housing, worst.c2_housing
        ),
        "lame_radial_stress_field" => format!("worst case: \u{03c3}r(interface) = \u{2212}{:.0} psi for both rings (boundary condition satisfied exactly).", worst.p_worst),
        "lame_hoop_stress_field" => format!(
            "worst case: \u{03c3}\u{03b8},housing = {:.0} psi, \u{03c3}\u{03b8},bushing = {:.0} psi.",
            worst.sigma_theta_housing_worst, worst.sigma_theta_bushing_worst
        ),
        "lame_axial_stress" => format!(
            "worst case: \u{03c3}z,housing = {:.0} psi, \u{03c3}z,bushing = {:.0} psi (k_constraint={:.2}, k_length={:.2}).",
            worst.sigma_z_housing_worst, worst.sigma_z_bushing_worst, out.axial_constraint_factor, out.axial_length_factor
        ),
        "hoop_stress_housing" => format!("nominal = {:.0} psi (worst case = {:.0} psi)", out.stress_hoop_housing, worst.sigma_theta_housing_worst),
        "hoop_stress_bushing" => format!("nominal = {:.0} psi (worst case = {:.0} psi)", out.stress_hoop_bushing, worst.sigma_theta_bushing_worst),
        "install_force" => format!("= {:.0} lbf", out.install_force),
        "margin_of_safety" => format!("governing: {} = {}", out.governing.name, fmt_margin(out.governing.margin)),
        _ => String::new(),
    }
}

#[cfg(test)]
mod worst_case_lame_tests {
    use super::WorstCaseLame;
    use bushing_solver::solve::{compute, BushingInputs, EndConstraint};

    fn fixture(end_constraint: EndConstraint) -> bushing_solver::solve::BushingOutput {
        compute(&BushingInputs {
            bore_dia: 0.5,
            id_bushing: 0.375,
            interference: 0.0015,
            interference_tol_plus: 0.0008,
            interference_tol_minus: 0.0008,
            bore_tol_plus: 0.0002,
            bore_tol_minus: 0.0002,
            housing_len: 0.5,
            housing_width: 1.5,
            edge_dist: 0.75,
            mat_housing: "al7075".to_string(),
            mat_bushing: "bronze".to_string(),
            friction: Some(0.15),
            d_t: 0.0,
            end_constraint,
            min_wall_straight: 0.05,
            ..Default::default()
        })
    }

    /// The worst-case hoop stress this struct reports must be the exact
    /// same number the Checks gauges' own worst-magnitude whisker end
    /// already shows - a passthrough, but the one that matters most: if
    /// this ever drifted, the derivation view would show a "worst case"
    /// that disagreed with the rest of the app. Housing stress is
    /// tensile (>=0), so its worst case is the range's numeric `.max`;
    /// bushing stress is compressive (<=0), so its worst case (largest
    /// magnitude) is the range's numeric `.min` - `ranged()` orders by
    /// value, not by which interference extreme produced it, so these
    /// are genuinely different fields, not a typo.
    #[test]
    fn worst_case_hoop_stress_matches_the_existing_range_fields() {
        let out = fixture(EndConstraint::Free);
        let worst = WorstCaseLame::compute(&out, 0.33, 0.34);
        assert_eq!(worst.sigma_theta_housing_worst, out.stress_hoop_housing_range.max);
        assert_eq!(worst.sigma_theta_bushing_worst, out.stress_hoop_bushing_range.min);
        assert_eq!(worst.p_worst, out.pressure_range.max);
    }

    /// C1/C2 must satisfy the same Lamé trial form the derivation PNG
    /// shows (`sigma_theta(r) = C1 + C2/r^2`) at each region's own
    /// interface radius - proves the displayed constants aren't just
    /// plausible-looking numbers but actually solve the equation the
    /// adjacent formula image states.
    #[test]
    fn solved_constants_satisfy_the_lame_trial_form_at_the_interface() {
        let out = fixture(EndConstraint::Free);
        let worst = WorstCaseLame::compute(&out, 0.33, 0.34);
        let sigma_theta_bushing_from_constants = worst.c1_bushing + worst.c2_bushing / worst.b_bushing.powi(2);
        let sigma_theta_housing_from_constants = worst.c1_housing + worst.c2_housing / worst.a_housing.powi(2);
        assert!((sigma_theta_bushing_from_constants - worst.sigma_theta_bushing_worst).abs() < 1e-6);
        assert!((sigma_theta_housing_from_constants - worst.sigma_theta_housing_worst).abs() < 1e-6);
    }

    #[test]
    fn zero_axial_scale_for_free_end_constraint_zeroes_worst_case_axial_stress() {
        let out = fixture(EndConstraint::Free);
        let worst = WorstCaseLame::compute(&out, 0.33, 0.34);
        assert_eq!(worst.sigma_z_housing_worst, 0.0);
        assert_eq!(worst.sigma_z_bushing_worst, 0.0);
    }

    /// With a real axial constraint, worst-case axial stress must be
    /// nonzero and carry the sign Poisson coupling implies: housing hoop
    /// stress is tensile (positive) so its axial companion is tensile
    /// too; bushing hoop stress is compressive (negative) so its axial
    /// companion is compressive too.
    #[test]
    fn both_ends_constrained_produces_nonzero_worst_case_axial_stress_with_the_expected_sign() {
        let out = fixture(EndConstraint::BothEnds);
        let worst = WorstCaseLame::compute(&out, 0.33, 0.34);
        assert!(worst.sigma_z_housing_worst > 0.0, "expected tensile housing axial stress, got {}", worst.sigma_z_housing_worst);
        assert!(worst.sigma_z_bushing_worst < 0.0, "expected compressive bushing axial stress, got {}", worst.sigma_z_bushing_worst);
    }
}
