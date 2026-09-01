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
use bushing_solver::materials::MATERIALS;
use bushing_solver::reamers::{self, ReamerEntry};
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use bushing_solver::tolerance::{EnforcementPolicy, ToleranceStatus};

use crate::components::{Dropdown, FieldGroup};

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

/// Same thresholds as `margin_class`, as a bare status-dot class for the
/// design-status rail's checklist rows (`DesignStatusRail`) - a compact
/// colored dot rather than `margin_class`'s full text pill, since the
/// margin number already sits right next to it in that layout.
fn margin_dot_class(margin: f64) -> &'static str {
    if !margin.is_finite() {
        "bushing-check-dot neutral"
    } else if margin < 0.0 {
        "bushing-check-dot crit"
    } else if margin < 0.15 {
        "bushing-check-dot warn"
    } else {
        "bushing-check-dot ok"
    }
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
    Report,
}

const STEPS: [Step; 6] = [Step::Repair, Step::Geometry, Step::Material, Step::Fit, Step::Analysis, Step::Report];

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Repair => "Repair",
            Step::Geometry => "Geometry",
            Step::Material => "Material",
            Step::Fit => "Fit",
            Step::Analysis => "Analysis",
            Step::Report => "Report",
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
/// which step is open) - pure aggregation over
/// `bushing_solver::solve::BushingOutput.candidates` (already every named
/// margin check the Results table shows) plus `tolerance_status`, no new
/// engine data. Clicking any row jumps to the Analysis step, where the
/// full Results table and alerts already live - this is the one
/// deliberately coarse piece of cross-step wiring in Phase 1 ("jump to
/// where this is analyzed," not a per-field pinpoint - see the plan's own
/// scoping note on diagram/error cross-linking).
#[component]
fn DesignStatusRail(candidates: Vec<bushing_solver::solve::MarginCandidate>, tolerance_status: ToleranceStatus, on_jump: EventHandler<()>) -> Element {
    let total = candidates.len() + usize::from(tolerance_status == ToleranceStatus::Infeasible);
    let passed = candidates.iter().filter(|c| c.margin.is_finite() && c.margin >= 0.0).count();
    let all_pass = passed == candidates.len() && tolerance_status != ToleranceStatus::Infeasible;
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
                for c in candidates {
                    div {
                        class: if c.margin.is_finite() && c.margin < 0.0 { "bushing-check-row bushing-check-attn" } else { "bushing-check-row" },
                        onclick: move |_| on_jump.call(()),
                        span { class: margin_dot_class(c.margin) }
                        span { class: "bushing-check-name", "{c.name}" }
                        span { class: "bushing-check-val mono", "{fmt_margin(c.margin)}" }
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

/// Same thresholds as `margin_class`, as a short status word for the
/// Results spec table's Status column.
fn margin_status_text(margin: f64) -> &'static str {
    if !margin.is_finite() {
        "\u{2014}"
    } else if margin < 0.0 {
        "Fails"
    } else if margin < 0.15 {
        "Marginal"
    } else {
        "OK"
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
    let mut show_derivation = use_signal(|| false);
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

    let mat_bushing_props = *bushing_solver::materials::get_material(&mat_bushing());
    let mat_housing_props = *bushing_solver::materials::get_material(&mat_housing());

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

            div { class: "bushing-summary-band",
                div { class: "bushing-viz-panes",
                    div { class: "bushing-viz-pane bushing-viz-overview",
                        span { class: "bushing-viz-tag", "Overview" }
                        img { class: "bushing-viz-img", src: bushing_visualizer::geometry_crop_svg_data_uri(&section_input, dark(), None, 90.0) }
                    }
                    div { class: "bushing-viz-pane bushing-viz-detail",
                        span { class: "bushing-viz-tag", "{detail_label}" }
                        img { class: "bushing-viz-img", src: bushing_visualizer::geometry_crop_svg_data_uri(&section_input, dark(), Some(crop), 280.0) }
                        button {
                            class: "bushing-viz-expand",
                            title: "Expand",
                            onclick: move |_| visualizer_lightbox_open.set(true),
                            {icon_expand()}
                        }
                    }
                }
                div { class: "bushing-summary-row",
                    SummaryCard { label: "Contact pressure", value: fmt_ranged(out.pressure_range, 0, "psi") }
                    SummaryCard { label: "Installed OD", value: format!("{:.4} in", out.od_installed) }
                    SummaryCard { label: "Straight wall", value: fmt_ranged(out.wall_straight_range, 4, "in") }
                    SummaryCard { label: "Neck wall", value: format!("{:.4} in", out.wall_neck) }
                    SummaryCard {
                        label: "Governing check",
                        value: out.governing.name.to_string(),
                        badge: Some((fmt_margin(out.governing.margin), margin_class(out.governing.margin))),
                    }
                }
            }

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
                                NumberField { label: "Bushing ID (in)", value: id_bushing, step: "0.001" }
                                NumberField { label: "Housing length (in)", value: housing_len, step: "0.01" }
                                NumberField { label: "Housing width (in)", value: housing_width, step: "0.01" }
                                NumberField { label: "Edge distance (in)", value: edge_dist, step: "0.01" }
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
                                        NumberField { label: "Flange OD (in)", value: flange_od, step: "0.01" }
                                        NumberField { label: "Flange thickness (in)", value: flange_thk, step: "0.001" }
                                    }
                                }
                            }
                        },
                        Step::Material => rsx! {
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "03 \u{00b7} Material" }
                                MaterialField { label: "Housing material", value: mat_housing }
                                MaterialField { label: "Bushing material", value: mat_bushing }
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
                                NumberField { label: "Edge load angle (deg)", value: edge_load_angle_deg, step: "1" }
                                NumberField { label: "Applied edge load (lbf)", value: load, step: "10" }
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
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "05 \u{00b7} Analysis \u{2014} results" }
                                div { class: "spec-table-wrap",
                                    table { class: "spec-table",
                                        thead {
                                            tr {
                                                th { "Quantity" }
                                                th { class: "num", "Nominal" }
                                                th { class: "num", "Min" }
                                                th { class: "num", "Max" }
                                                th { class: "num", "Allowable" }
                                                th { class: "num", "Margin" }
                                                th { "Status" }
                                            }
                                        }
                                        tbody {
                                            ResultSpecRow { label: "Housing hoop stress, psi", decimals: 0, nominal: out.stress_hoop_housing, range: Some((out.stress_hoop_housing_range.min, out.stress_hoop_housing_range.max)), allowable: mat_housing_props.sy_ksi * 1000.0, margin: out.housing_ms }
                                            ResultSpecRow { label: "Bushing hoop stress, psi", decimals: 0, nominal: out.stress_hoop_bushing, range: Some((out.stress_hoop_bushing_range.min, out.stress_hoop_bushing_range.max)), allowable: mat_bushing_props.sy_ksi * 1000.0, margin: out.bushing_ms }
                                            ResultSpecRow { label: "Edge distance (sequencing), in", decimals: 3, nominal: out.ed_actual, range: None, allowable: out.ed_min_sequence, margin: out.sequence_margin }
                                            ResultSpecRow { label: "Edge distance (strength), in", decimals: 3, nominal: out.ed_actual, range: None, allowable: out.ed_min_strength, margin: out.strength_margin }
                                            ResultSpecRow { label: "Straight wall thickness, in", decimals: 4, nominal: out.wall_straight, range: Some((out.wall_straight_range.min, out.wall_straight_range.max)), allowable: min_wall_straight(), margin: out.wall_straight / min_wall_straight() - 1.0 }
                                            ResultSpecRow { label: "Neck wall thickness, in", decimals: 4, nominal: out.wall_neck, range: None, allowable: min_wall_neck(), margin: out.wall_neck / min_wall_neck() - 1.0 }
                                        }
                                    }
                                }
                            }
                        },
                        Step::Report => rsx! {
                            div { class: "bushing-card",
                                h3 { class: "bushing-card-title", "06 \u{00b7} Report \u{2014} derived quantities" }
                                div { class: "bushing-detail-grid",
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
                        },
                    }
                }
                DesignStatusRail {
                    candidates: out.candidates.clone(),
                    tolerance_status: out.tolerance_status,
                    on_jump: move |_| current_step.set(Step::Analysis),
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

/// A spec-table row for one governing/margin check - the same
/// demand/allowable/margin numbers the previous list-row layout showed,
/// laid out as a table row alongside the countersink/dimension tables
/// above it so inputs and outputs read as one continuous sheet. Every row
/// here is a physics-engine output (never a direct or derived dimension),
/// so unlike `CsSpecRow`/`PlainSpecRow` there is no per-row Source badge
/// - the table's own caption says so once instead of repeating an
/// identical "Calc" badge six times.
#[component]
fn ResultSpecRow(label: &'static str, decimals: usize, nominal: f64, range: Option<(f64, f64)>, allowable: f64, margin: f64) -> Element {
    let nominal_text = format!("{nominal:.*}", decimals);
    let min_text = range.map(|(lo, _)| format!("{lo:.*}", decimals)).unwrap_or_else(|| "\u{2014}".to_string());
    let max_text = range.map(|(_, hi)| format!("{hi:.*}", decimals)).unwrap_or_else(|| "\u{2014}".to_string());
    let allowable_text = format!("{allowable:.*}", decimals);
    let margin_text = fmt_margin(margin);
    rsx! {
        tr { class: "spec-row",
            td { "{label}" }
            td { class: "num mono", "{nominal_text}" }
            td { class: "num mono", "{min_text}" }
            td { class: "num mono", "{max_text}" }
            td { class: "num mono", "{allowable_text}" }
            td { class: "num mono", "{margin_text}" }
            td { span { class: margin_class(margin), "{margin_status_text(margin)}" } }
        }
    }
}

#[component]
fn SummaryCard(label: &'static str, value: String, badge: Option<(String, &'static str)>) -> Element {
    rsx! {
        div { class: "summary-card",
            div { class: "summary-card-label-row",
                span { class: "summary-card-label", "{label}" }
                span { class: "src-chip src-calculated", "Calc" }
            }
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
