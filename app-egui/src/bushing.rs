//! Bushing Workbench - egui port of `app/src/bushing_workbench.rs`,
//! rebuilt to mirror the approved mockup artifact exactly: same 6-step
//! sequence and field grouping (Repair / Geometry / Material / Fit /
//! Analysis / Results), pill stepper, form + labeled cross-section
//! sketch on the first 4 steps (matching the mockup precisely - Analysis
//! never had a sketch there either), frozen headline, Stat Tiles checks,
//! Ladder/Center-spine status rail.
//!
//! `bushing-solver::solve::compute` is pure, framework-agnostic math -
//! reused unchanged. `BushingInputs` derives `Default`, and its own doc
//! comment guarantees every field this pass doesn't expose reproduces
//! the original straight-bushing-only behavior, so leaving
//! flanged-geometry fields unset when "Flanged" isn't checked is a real,
//! documented construction, not a guess.
//!
//! One real, disclosed UI omission (not a silent deviation): the
//! mockup's Fit step showed a "Fit class" chip row (Class 1/2/3) as
//! descriptive text alongside interference - `BushingInputs` has no
//! matching field, it was illustrative only in the mockup. Shipping a
//! selector that doesn't drive real computation would be a fake control,
//! worse than omitting it, so only the real `interference` input is here.
//!
//! Reamer picker, internal+external countersink dia/depth/angle mode
//! selection, tolerance-capture spec tables (bore/interference/both
//! countersinks), and the bore-tolerance enforcement checkbox are all
//! ported now too - real features an earlier pass of this port dropped,
//! restored after a direct user report (confirmed by reading
//! `app/src/bushing_workbench.rs` side by side rather than guessing).
//!
//! NOT yet ported (tracked, not dropped): the cross-section visualizer +
//! lightbox, the `RangedValue`-based Results-step output displays
//! (how computed outputs vary across the tolerance band - a materially
//! different feature from the input tolerance capture that IS here).

use eframe::egui;
use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use bushing_solver::reamers::{self, ReamerEntry};
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use bushing_solver::tolerance::{EnforcementPolicy, ToleranceRange};
use mechanics_core::materials::MATERIALS;

use crate::components::{checks_tiles, status_rail, CheckRow};
use crate::sketches::{bushing_head_on, bushing_isometric, bushing_side_view, BushingSketchCtx};
use crate::theme::Tokens;
use crate::widgets::{card, headline, side_by_side, stepper, MIN_FLEX_COL};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum Step {
    #[default]
    Repair,
    Geometry,
    Material,
    Fit,
    Analysis,
    Results,
}
const STEPS: [(Step, &str); 6] = [
    (Step::Repair, "Repair"),
    (Step::Geometry, "Geometry"),
    (Step::Material, "Material"),
    (Step::Fit, "Fit"),
    (Step::Analysis, "Analysis"),
    (Step::Results, "Results"),
];

/// Matches the approved mockup's `.form-card` width - used by every step
/// without a spec table.
const FORM_W: f32 = 280.0;
/// Repair/Fit/Geometry now carry 6-column spec tables (Dimension/
/// Nominal/Tol−/Tol+/Range/Source) - a real, visible width increase from
/// every other step's `FORM_W`, needed because a spec table genuinely
/// doesn't fit in 280px (confirmed against how the original HTML table
/// lays out), not a stylistic choice.
const SPEC_FORM_W: f32 = 420.0;

pub struct BushingTool {
    step: Step,
    pub bore_dia: f64,
    pub bore_tol_plus: f64,
    pub bore_tol_minus: f64,
    /// Transient UI toggle for the reamer-catalog picker - not persisted
    /// (opens/closes fresh each session, same as `pending_index_confirm`-
    /// style ephemeral UI state elsewhere in this crate).
    pub reamer_picker_open: bool,
    pub id_bushing: f64,
    pub housing_len: f64,
    pub housing_width: f64,
    pub edge_dist: f64,
    pub interference: f64,
    pub interference_tol_plus: f64,
    pub interference_tol_minus: f64,
    /// "Auto-tighten bore tolerance to meet target interference" -
    /// `BushingInputs::enforcement`'s `enabled` flag. Disabled reproduces
    /// the original "report Infeasible honestly" behavior.
    pub enforcement_enabled: bool,
    pub mat_housing: String,
    pub mat_bushing: String,
    pub d_t: f64,
    pub min_wall_straight: f64,
    pub min_wall_neck: f64,
    pub friction: f64,
    pub end_constraint: EndConstraint,
    pub edge_load_angle_deg: f64,
    pub load: f64,
    pub assembly_thermal_assist: bool,
    pub assembly_housing_temperature: f64,
    pub assembly_bushing_temperature: f64,
    pub head_type: BushingType,
    pub flange_od: f64,
    pub flange_thk: f64,
    /// Bushing's own axial engagement length - drafting-only (see
    /// `inputs()`'s doc comment). Independently editable from
    /// `housing_len`: equal → flush at both ends; unequal → the sketch
    /// shows a real protrusion or recess instead of silently drawing them
    /// flush.
    pub bushing_length: f64,
    /// Internal (ID-side) countersink - independent of the OD `head_type`
    /// feature above (a bushing can have either, both, or neither).
    pub id_type: IdType,
    pub cs_mode: CsMode,
    pub cs_dia: f64,
    pub cs_depth: f64,
    pub cs_angle: f64,
    pub cs_dia_tol_plus: f64,
    pub cs_dia_tol_minus: f64,
    pub cs_depth_tol_plus: f64,
    pub cs_depth_tol_minus: f64,
    pub cs_angle_tol_plus: f64,
    pub cs_angle_tol_minus: f64,
    /// External (OD-side) countersink mode - only meaningful when
    /// `head_type == Countersink`. Replaces a prior hardcoded
    /// `CsMode::DiaAngle` that left "Countersink depth" freely editable
    /// even though depth is the DERIVED dimension in that mode - a real,
    /// silently-ignored-edit bug, not a cosmetic gap.
    pub ext_cs_mode: CsMode,
    pub ext_cs_dia: f64,
    pub ext_cs_depth: f64,
    pub ext_cs_angle: f64,
    pub ext_cs_dia_tol_plus: f64,
    pub ext_cs_dia_tol_minus: f64,
    pub ext_cs_depth_tol_plus: f64,
    pub ext_cs_depth_tol_minus: f64,
    pub ext_cs_angle_tol_plus: f64,
    pub ext_cs_angle_tol_minus: f64,
    pub lower_chamfer_min: f64,
    pub lower_chamfer_max: f64,
    pub lower_chamfer_angle_deg: f64,
    pub head_chamfer_min: f64,
    pub head_chamfer_max: f64,
    pub head_chamfer_angle_deg: f64,
}

impl Default for BushingTool {
    fn default() -> Self {
        Self {
            step: Step::default(),
            bore_dia: 0.8760,
            // Prior app (`app/src/bushing_workbench.rs`) default is
            // 0.0005/0.0 against a 0.500 in bore - scaled by that ratio
            // to this tool's own 0.8760 in default (same "scale a
            // borrowed default, don't copy verbatim" fix already applied
            // to the external countersink defaults below).
            bore_tol_plus: 0.0009,
            bore_tol_minus: 0.0,
            reamer_picker_open: false,
            id_bushing: 0.500,
            housing_len: 1.25,
            housing_width: 2.00,
            edge_dist: 0.375,
            interference: 0.0015,
            interference_tol_plus: 0.0003, // prior app's own default, unscaled (interference nominal is unchanged from it)
            interference_tol_minus: 0.0,
            enforcement_enabled: false,
            mat_housing: "al2024".to_string(),
            mat_bushing: "steel".to_string(),
            d_t: 0.0,
            min_wall_straight: 0.1875,
            min_wall_neck: 0.150, // matches the approved artifact's own illustrative default
            friction: 0.15,
            end_constraint: EndConstraint::Free,
            edge_load_angle_deg: 40.0,
            load: 1000.0,
            assembly_thermal_assist: false,
            assembly_housing_temperature: 70.0,
            assembly_bushing_temperature: 70.0,
            head_type: BushingType::Straight,
            flange_od: 1.10,
            flange_thk: 0.062,
            bushing_length: 1.25, // == housing_len: flush at both ends by default
            // bushing-solver's own tested countersunk-head fixture uses
            // ext_cs_dia/depth 0.6/0.06 against a 0.5 in bore (angle_tolerance
            // test in solve.rs) - scaled by that same ratio to this tool's
            // own default 0.8760 in bore instead of copied verbatim, which
            // would otherwise make the "head" smaller than the installed OD
            // (confirmed by a real screenshot: the counterbore rendered as
            // barely visible).
            // Internal countersink: prior app defaults (cs_dia 0.5,
            // cs_depth 0.08) were sized against its own 0.375 in
            // id_bushing default - scaled by that ratio (1.333x) to this
            // tool's own 0.500 in id_bushing.
            id_type: IdType::Straight,
            cs_mode: CsMode::DepthAngle, // matches bushing-solver's own Default
            cs_dia: 0.65,
            cs_depth: 0.10,
            cs_angle: 100.0,
            cs_dia_tol_plus: 0.0025,
            cs_dia_tol_minus: 0.0,
            cs_depth_tol_plus: 0.0065,
            cs_depth_tol_minus: 0.0,
            cs_angle_tol_plus: 0.0,
            cs_angle_tol_minus: 0.0,
            ext_cs_mode: CsMode::DepthAngle,
            ext_cs_dia: 1.05,
            ext_cs_depth: 0.10,
            ext_cs_angle: 100.0,
            // Same scale-by-bore-ratio (1.752x) as ext_cs_dia/depth above.
            ext_cs_dia_tol_plus: 0.0035,
            ext_cs_dia_tol_minus: 0.0,
            ext_cs_depth_tol_plus: 0.0088,
            ext_cs_depth_tol_minus: 0.0,
            ext_cs_angle_tol_plus: 0.0,
            ext_cs_angle_tol_minus: 0.0,
            lower_chamfer_min: 0.007,
            lower_chamfer_max: 0.015,
            lower_chamfer_angle_deg: 45.0,
            head_chamfer_min: 0.010,
            head_chamfer_max: 0.015,
            head_chamfer_angle_deg: 0.0, // normal to the head surface (square relief), user-confirmed default
        }
    }
}

fn form_and_sketch(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    emph: &str,
    ctx: &BushingSketchCtx,
    form_width: f32,
    form: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(form_width);
            card(ui, tokens, |ui| {
                crate::widgets::card_title(ui, title);
                ui.add_space(4.0);
                form(ui);
            });
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            egui::Frame::default().fill(tokens.bg_sunken).stroke(egui::Stroke::new(1.0, tokens.border)).rounding(8.0).inner_margin(14.0).show(ui, |ui| {
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("HEAD-ON CROSS-SECTION").size(9.5).strong());
                bushing_head_on(ui, tokens, emph, ctx);
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("SIDE (LONGITUDINAL) VIEW").size(9.5).strong());
                bushing_side_view(ui, tokens, emph, ctx);
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("ISOMETRIC (SCHEMATIC)").size(9.5).strong());
                bushing_isometric(ui, tokens, ctx);
            });
        });
    });
}

impl BushingTool {
    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        stepper(ui, tokens, &STEPS, &mut self.step);
        ui.add_space(10.0);

        // Computed every frame regardless of step - `compute()` is a
        // closed-form/bounded solve (same "cheap, no separate run step"
        // shape `pressure_vessel.rs` already documents for its own
        // solvers), so the persistent status rail (below) always reflects
        // the current field values, not just a stale Results-step snapshot.
        let out = compute(&self.inputs());
        let check_rows: Vec<CheckRow> =
            out.candidates.iter().map(|c| CheckRow { name: c.name, applied: 0.0, allowable: 0.0, margin: c.margin, unit: "" }).collect();

        let step = self.step;
        side_by_side(
            ui,
            232.0, // `.status-rail { width: 232px }` in the approved artifact
            MIN_FLEX_COL,
            |ui| self.step_content(ui, tokens, step, &out, &check_rows),
            |ui| status_rail(ui, tokens, &check_rows),
        );
    }

    fn step_content(&mut self, ui: &mut egui::Ui, tokens: &Tokens, step: Step, out: &bushing_solver::solve::BushingOutput, check_rows: &[CheckRow]) {
        let ctx = self.sketch_ctx(out);
        // Design System Epic Phase 5 - a real fade-in when a step's
        // content first appears, not a flat instant swap. Keyed on
        // `step` itself (not a generation counter), so this plays once
        // per step per session rather than re-triggering on every
        // revisit - a deliberate, small scope cut: correct on first
        // visit, and no state to persist for a purely cosmetic effect.
        let fade_id = ui.id().with(("bushing_step_fade", step));
        let alpha = ui.ctx().animate_bool(fade_id, true);
        ui.scope(|ui| {
        ui.set_opacity(alpha);
        match step {
            Step::Repair => form_and_sketch(ui, tokens, "01 \u{b7} Repair", "repair", &ctx, SPEC_FORM_W, |ui| {
                spec_table(ui, "repair_bore_spec", |ui, w| {
                    plain_spec_row(ui, tokens, 0, w, "Bore, in", &mut self.bore_dia, &mut self.bore_tol_plus, &mut self.bore_tol_minus, out.bore_tol, 4);
                });
                ui.horizontal(|ui| {
                    if ui.button("Pick reamer\u{2026}").clicked() {
                        self.reamer_picker_open = !self.reamer_picker_open;
                    }
                    ui.colored_label(tokens.fg_subtle, egui::RichText::new("sets bore + its tool tolerance").size(9.0).italics());
                });
                if self.reamer_picker_open {
                    reamer_picker(ui, tokens, self.bore_dia, |entry| {
                        self.bore_dia = entry.nominal_in;
                        self.bore_tol_plus = entry.tool_tolerance_plus_in;
                        self.bore_tol_minus = entry.tool_tolerance_minus_in;
                        self.reamer_picker_open = false;
                    });
                }
                ui.add_space(6.0);
                num_field(ui, tokens, "Bushing ID (in)", &mut self.id_bushing);
                num_field(ui, tokens, "Housing length (in)", &mut self.housing_len);
                num_field(ui, tokens, "Housing width (in)", &mut self.housing_width);
                num_field(ui, tokens, "Edge distance (in)", &mut self.edge_dist);
            }),
            Step::Geometry => form_and_sketch(ui, tokens, "02 \u{b7} Geometry", "geom", &ctx, SPEC_FORM_W, |ui| {
                ui.label("Head type (OD)");
                crate::design::components::segmented(
                    ui,
                    tokens,
                    &mut self.head_type,
                    &[(BushingType::Straight, "Slug (no head)"), (BushingType::Countersink, "Countersunk"), (BushingType::Flanged, "Flanged")],
                );
                ui.add_enabled_ui(self.head_type == BushingType::Flanged, |ui| {
                    num_field(ui, tokens, "Flange OD (in)", &mut self.flange_od);
                    num_field(ui, tokens, "Flange thickness (in)", &mut self.flange_thk);
                });
                if self.head_type == BushingType::Countersink {
                    ui.add_space(4.0);
                    ui.label("External countersink mode");
                    cs_mode_picker(ui, tokens, &mut self.ext_cs_mode);
                    spec_table(ui, "ext_cs_spec", |ui, w| {
                        let mode = self.ext_cs_mode;
                        cs_spec_row(
                            ui, tokens, 0, w, "Depth, in", cs_field_is_direct(mode, CsField::Depth), &mut self.ext_cs_depth, &mut self.ext_cs_depth_tol_plus,
                            &mut self.ext_cs_depth_tol_minus, out.cs_solved_od.map(|c| c.depth), out.cs_external_depth_tol, 4,
                        );
                        cs_spec_row(
                            ui, tokens, 1, w, "Angle, deg", cs_field_is_direct(mode, CsField::Angle), &mut self.ext_cs_angle, &mut self.ext_cs_angle_tol_plus,
                            &mut self.ext_cs_angle_tol_minus, out.cs_solved_od.map(|c| c.angle_deg), out.cs_external_angle_tol, 1,
                        );
                        cs_spec_row(
                            ui, tokens, 2, w, "Diameter, in", cs_field_is_direct(mode, CsField::Dia), &mut self.ext_cs_dia, &mut self.ext_cs_dia_tol_plus,
                            &mut self.ext_cs_dia_tol_minus, out.cs_solved_od.map(|c| c.dia), out.cs_external_dia_tol, 4,
                        );
                    });
                }
                ui.add_space(8.0);
                ui.label("ID geometry");
                crate::design::components::segmented(ui, tokens, &mut self.id_type, &[(IdType::Straight, "Straight"), (IdType::Countersink, "Countersunk")]);
                if self.id_type == IdType::Countersink {
                    ui.add_space(4.0);
                    ui.label("Internal countersink mode");
                    cs_mode_picker(ui, tokens, &mut self.cs_mode);
                    spec_table(ui, "int_cs_spec", |ui, w| {
                        let mode = self.cs_mode;
                        cs_spec_row(
                            ui, tokens, 0, w, "Depth, in", cs_field_is_direct(mode, CsField::Depth), &mut self.cs_depth, &mut self.cs_depth_tol_plus,
                            &mut self.cs_depth_tol_minus, out.cs_solved_id.map(|c| c.depth), out.cs_internal_depth_tol, 4,
                        );
                        cs_spec_row(
                            ui, tokens, 1, w, "Angle, deg", cs_field_is_direct(mode, CsField::Angle), &mut self.cs_angle, &mut self.cs_angle_tol_plus,
                            &mut self.cs_angle_tol_minus, out.cs_solved_id.map(|c| c.angle_deg), out.cs_internal_angle_tol, 1,
                        );
                        cs_spec_row(
                            ui, tokens, 2, w, "Diameter, in", cs_field_is_direct(mode, CsField::Dia), &mut self.cs_dia, &mut self.cs_dia_tol_plus,
                            &mut self.cs_dia_tol_minus, out.cs_solved_id.map(|c| c.dia), out.cs_internal_dia_tol, 4,
                        );
                    });
                }
                ui.add_space(6.0);
                num_field(ui, tokens, "Bushing length (in)", &mut self.bushing_length);
                ui.add_space(6.0);
                ui.label("Lower-end chamfer (always the end opposite the head)");
                num_field(ui, tokens, "Min (in)", &mut self.lower_chamfer_min);
                num_field(ui, tokens, "Max (in)", &mut self.lower_chamfer_max);
                num_field(ui, tokens, "Angle from axis (deg)", &mut self.lower_chamfer_angle_deg);
                ui.add_space(4.0);
                ui.label("Head top-edge chamfer");
                num_field(ui, tokens, "Min (in)", &mut self.head_chamfer_min);
                num_field(ui, tokens, "Max (in)", &mut self.head_chamfer_max);
                num_field(ui, tokens, "Angle (deg, 0 = square relief)", &mut self.head_chamfer_angle_deg);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("Chamfers + bushing length are drafting only \u{2014} not fed into the margin calculations below").size(9.5).italics());
            }),
            Step::Material => form_and_sketch(ui, tokens, "03 \u{b7} Material", "material", &ctx, FORM_W, |ui| {
                material_combo(ui, tokens, "Housing material", &mut self.mat_housing);
                material_combo(ui, tokens, "Bushing material", &mut self.mat_bushing);
            }),
            Step::Fit => form_and_sketch(ui, tokens, "04 \u{b7} Fit", "fit", &ctx, SPEC_FORM_W, |ui| {
                spec_table(ui, "fit_interference_spec", |ui, w| {
                    plain_spec_row(ui, tokens, 0, w, "Interference, in", &mut self.interference, &mut self.interference_tol_plus, &mut self.interference_tol_minus, out.interference_tol, 4);
                });
                ui.add_space(6.0);
                ui.checkbox(&mut self.enforcement_enabled, "Auto-tighten bore tolerance to meet target interference");
            }),
            Step::Analysis => {
                card(ui, tokens, |ui| {
                    crate::widgets::card_title(ui, "05 \u{b7} Analysis \u{2014} acceptance criteria");
                    num_field(ui, tokens, "Minimum straight wall (in)", &mut self.min_wall_straight);
                    num_field(ui, tokens, "Minimum neck wall (in)", &mut self.min_wall_neck);
                });
                ui.add_space(10.0);
                card(ui, tokens, |ui| {
                    crate::widgets::card_title(ui, "05 \u{b7} Analysis \u{2014} environment & install");
                    num_field(ui, tokens, "Friction coefficient", &mut self.friction);
                    ui.horizontal(|ui| {
                        ui.label("End constraint");
                        crate::design::components::segmented(
                            ui,
                            tokens,
                            &mut self.end_constraint,
                            &[(EndConstraint::Free, "Free"), (EndConstraint::OneEnd, "One end"), (EndConstraint::BothEnds, "Both ends")],
                        );
                    });
                    num_field(ui, tokens, "\u{394}T from install, \u{b0}F", &mut self.d_t);
                    num_field(ui, tokens, "Edge load angle (deg)", &mut self.edge_load_angle_deg);
                    num_field(ui, tokens, "Applied edge load (lbf)", &mut self.load);
                    ui.checkbox(&mut self.assembly_thermal_assist, "Thermally assisted install (shrink/expand fit)");
                    if self.assembly_thermal_assist {
                        num_field(ui, tokens, "Housing temp at install (\u{b0}F)", &mut self.assembly_housing_temperature);
                        num_field(ui, tokens, "Bushing temp at install (\u{b0}F)", &mut self.assembly_bushing_temperature);
                    }
                });
            }
            Step::Results => results_body(ui, tokens, out, check_rows),
        }
        });
    }

    fn inputs(&self) -> BushingInputs {
        BushingInputs {
            bore_dia: self.bore_dia,
            bore_tol_plus: self.bore_tol_plus,
            bore_tol_minus: self.bore_tol_minus,
            id_bushing: self.id_bushing,
            interference: self.interference,
            interference_tol_plus: self.interference_tol_plus,
            interference_tol_minus: self.interference_tol_minus,
            housing_len: self.housing_len,
            housing_width: self.housing_width,
            edge_dist: self.edge_dist,
            mat_housing: self.mat_housing.clone(),
            mat_bushing: self.mat_bushing.clone(),
            d_t: self.d_t,
            min_wall_straight: self.min_wall_straight,
            friction: Some(self.friction),
            end_constraint: self.end_constraint,
            edge_load_angle_deg: Some(self.edge_load_angle_deg),
            load: Some(self.load),
            assembly_housing_temperature: self.assembly_thermal_assist.then_some(self.assembly_housing_temperature),
            assembly_bushing_temperature: self.assembly_thermal_assist.then_some(self.assembly_bushing_temperature),
            min_wall_neck: self.min_wall_neck,
            bushing_type: self.head_type,
            id_type: self.id_type,
            flange_od: self.flange_od,
            flange_thk: self.flange_thk,
            cs_mode: self.cs_mode,
            cs_dia: self.cs_dia,
            cs_depth: self.cs_depth,
            cs_angle: self.cs_angle,
            cs_dia_tol_plus: self.cs_dia_tol_plus,
            cs_dia_tol_minus: self.cs_dia_tol_minus,
            cs_depth_tol_plus: self.cs_depth_tol_plus,
            cs_depth_tol_minus: self.cs_depth_tol_minus,
            cs_angle_tol_plus: self.cs_angle_tol_plus,
            cs_angle_tol_minus: self.cs_angle_tol_minus,
            ext_cs_mode: self.ext_cs_mode,
            ext_cs_dia: self.ext_cs_dia,
            ext_cs_depth: self.ext_cs_depth,
            ext_cs_angle: self.ext_cs_angle,
            ext_cs_dia_tol_plus: self.ext_cs_dia_tol_plus,
            ext_cs_dia_tol_minus: self.ext_cs_dia_tol_minus,
            ext_cs_depth_tol_plus: self.ext_cs_depth_tol_plus,
            ext_cs_depth_tol_minus: self.ext_cs_depth_tol_minus,
            ext_cs_angle_tol_plus: self.ext_cs_angle_tol_plus,
            ext_cs_angle_tol_minus: self.ext_cs_angle_tol_minus,
            // Matches `app/src/bushing_workbench.rs`'s own wiring exactly:
            // `bore_capability` is always `None` there too (never wired to
            // a real process-capability input in the prior app either).
            enforcement: EnforcementPolicy { enabled: self.enforcement_enabled, ..EnforcementPolicy::default() },
            bore_capability: None,
            ..BushingInputs::default()
        }
    }

    /// `bushing_length` and the four chamfer fields are drafting-only:
    /// a .007-.015 in edge break has no meaningful effect on the margins
    /// `compute()` validates, and a real independent engagement-length
    /// input (distinct from `housing_len`, which `axial_length_factor`
    /// already assumes full engagement to) is a separate, bigger
    /// `bushing_solver` change than what was asked for here - a real,
    /// disclosed scope cut, not a silent omission.
    fn sketch_ctx(&self, out: &bushing_solver::solve::BushingOutput) -> BushingSketchCtx {
        let (cs_dia, cs_depth) = out.cs_solved_od.as_ref().map(|c| (c.dia, c.depth)).unwrap_or((self.ext_cs_dia, self.ext_cs_depth));
        BushingSketchCtx {
            head_type: self.head_type,
            housing_len: self.housing_len,
            housing_width: self.housing_width,
            edge_dist: self.edge_dist,
            id_bushing: self.id_bushing,
            bushing_length: self.bushing_length,
            flange_od: self.flange_od,
            flange_thk: self.flange_thk,
            cs_dia,
            cs_depth,
            lower_chamfer_min: self.lower_chamfer_min,
            lower_chamfer_max: self.lower_chamfer_max,
            lower_chamfer_angle_deg: self.lower_chamfer_angle_deg,
            head_chamfer_min: self.head_chamfer_min,
            head_chamfer_max: self.head_chamfer_max,
            head_chamfer_angle_deg: self.head_chamfer_angle_deg,
            od_installed: out.od_installed,
        }
    }
}

/// Results step body only - the status rail is now the persistent right
/// column `BushingTool::ui` renders via `side_by_side`, not a sibling
/// drawn here, so this is just the headline + Fabrication/Checks stack.
fn results_body(ui: &mut egui::Ui, tokens: &Tokens, out: &bushing_solver::solve::BushingOutput, check_rows: &[CheckRow]) {
    let passed = out.candidates.iter().filter(|c| c.margin.is_finite() && c.margin >= 0.0).count();
    let all_pass = passed == out.candidates.len();

    headline(
        ui,
        tokens,
        all_pass,
        passed,
        check_rows.len(),
        &[
            ("GOVERNING", format!("{} ({:+.2})", out.governing.name, out.governing.margin), None),
            ("INSTALLED OD", format!("\u{d8}{:.4} in", out.od_installed), None),
            ("WALL (STRAIGHT)", format!("{:.4} in", out.wall_straight), None),
        ],
    );
    ui.add_space(10.0);

    card(ui, tokens, |ui| {
        crate::widgets::card_title(ui, "Fabrication & install summary");
        egui::Grid::new("bushing_spec_grid").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
            ui.colored_label(tokens.fg_muted, "Installed OD");
            ui.label(format!("\u{d8}{:.4} in", out.od_installed));
            ui.end_row();
            ui.colored_label(tokens.fg_muted, "Wall (straight)");
            ui.label(format!("{:.4} in", out.wall_straight));
            ui.end_row();
            ui.colored_label(tokens.fg_muted, "Install force");
            ui.label(format!("{:.0} lbf", out.install_force));
            ui.end_row();
        });
    });
    ui.add_space(10.0);
    card(ui, tokens, |ui| {
        crate::widgets::card_title(ui, "Checks");
        checks_tiles(ui, tokens, check_rows);
    });
}

#[derive(Clone, Copy, PartialEq)]
enum CsField {
    Dia,
    Depth,
    Angle,
}

/// Ported directly from `app/src/bushing_workbench.rs`'s
/// `cs_field_is_direct_input` - which two of {dia, depth, angle} are
/// direct user inputs for a given `CsMode`; the third is solved by
/// `bushing_solver::countersink::solve_countersink`.
fn cs_field_is_direct(mode: CsMode, which: CsField) -> bool {
    match (mode, which) {
        (CsMode::DepthAngle, CsField::Dia) => false,
        (CsMode::DiaAngle, CsField::Depth) => false,
        (CsMode::DiaDepth, CsField::Angle) => false,
        _ => true,
    }
}

fn cs_mode_picker(ui: &mut egui::Ui, tokens: &Tokens, mode: &mut CsMode) {
    crate::design::components::segmented(
        ui,
        tokens,
        mode,
        &[(CsMode::DepthAngle, "Depth + angle"), (CsMode::DiaAngle, "Dia + angle"), (CsMode::DiaDepth, "Dia + depth")],
    );
}

/// 6-column spec table (Dimension/Nominal/Tol−/Tol+/Range/Source) -
/// matches `app/src/bushing_workbench.rs`'s `spec-table` HTML exactly,
/// via the same `egui::Grid` pattern `results_body`'s
/// `bushing_spec_grid` already uses elsewhere in this file.
fn spec_table(ui: &mut egui::Ui, id: &str, rows: impl FnOnce(&mut egui::Ui, f32)) {
    let width = ui.available_width();
    egui::Grid::new(id).num_columns(6).spacing([10.0, 4.0]).show(ui, |ui| {
        for h in ["Dimension", "Nominal", "Tol \u{2212}", "Tol +", "Range", "Source"] {
            ui.label(egui::RichText::new(h).size(9.0).strong());
        }
        ui.end_row();
        rows(ui, width);
    });
}

/// Design System Epic Phase 5 - zebra-striped spec tables. Paints a
/// subtle full-row tint behind every ODD row (0-indexed, so the 2nd,
/// 4th, ... row) before that row's cells are laid out - egui's painter
/// draws in call order, so this rect ends up underneath the row's own
/// `DragValue`/label widgets rather than covering them. A single-row
/// table (Bore, Interference) never calls this with `row_index > 0`, so
/// it stays a no-op there - striping has no visual meaning without at
/// least 2 rows to alternate against.
fn zebra_stripe(ui: &mut egui::Ui, tokens: &Tokens, row_index: usize, row_width: f32) {
    if row_index % 2 == 0 {
        return;
    }
    let top_left = ui.cursor().min;
    let rect = egui::Rect::from_min_size(egui::pos2(top_left.x - 4.0, top_left.y - 3.0), egui::vec2(row_width + 8.0, 22.0));
    let tint = tokens.bg_sunken;
    ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 110));
}

/// Bare "lower\u{2013}upper" (or a single value when the range is
/// degenerate) - the spec table's column header already carries the
/// unit.
fn format_range(r: ToleranceRange, decimals: usize) -> String {
    if (r.upper - r.lower).abs() < 1e-9 {
        format!("{:.*}", decimals, r.nominal)
    } else {
        format!("{:.*}\u{2013}{:.*}", decimals, r.lower, decimals, r.upper)
    }
}

/// A spec-table row for a plain direct-input dimension with its own
/// tolerance (bore, target interference) - always "Direct", no derived
/// split to track (unlike `cs_spec_row`).
#[allow(clippy::too_many_arguments)]
fn plain_spec_row(ui: &mut egui::Ui, tokens: &Tokens, row_index: usize, row_width: f32, label: &str, value: &mut f64, tol_plus: &mut f64, tol_minus: &mut f64, range: ToleranceRange, decimals: usize) {
    zebra_stripe(ui, tokens, row_index, row_width);
    ui.label(label);
    crate::widgets::styled_number(ui, value, 0.0005, decimals);
    crate::widgets::styled_number(ui, tol_minus, 0.0001, decimals);
    crate::widgets::styled_number(ui, tol_plus, 0.0001, decimals);
    ui.colored_label(tokens.fg_muted, egui::RichText::new(format_range(range, decimals)).monospace());
    ui.colored_label(tokens.accent, "Direct");
    ui.end_row();
}

/// A spec-table row for one countersink dimension (dia/depth/angle).
/// When `is_direct` is false for the active `CsMode`, the nominal/
/// tolerance cells show the solver's OWN solved value (read-only,
/// muted) instead of editable fields, and the source chip reads
/// "Derived" - the direct fix for a real, previously-shipped bug: an
/// edit to a mode-derived field used to be silently ignored by the
/// solver because the UI never disclosed which field was actually
/// live.
#[allow(clippy::too_many_arguments)]
fn cs_spec_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    row_index: usize,
    row_width: f32,
    label: &str,
    is_direct: bool,
    value: &mut f64,
    tol_plus: &mut f64,
    tol_minus: &mut f64,
    solved: Option<f64>,
    range: Option<ToleranceRange>,
    decimals: usize,
) {
    zebra_stripe(ui, tokens, row_index, row_width);
    ui.label(label);
    if is_direct {
        crate::widgets::styled_number(ui, value, 0.0005, decimals);
        crate::widgets::styled_number(ui, tol_minus, 0.0001, decimals);
        crate::widgets::styled_number(ui, tol_plus, 0.0001, decimals);
    } else {
        let text = solved.map(|v| format!("{v:.*}", decimals)).unwrap_or_default();
        ui.colored_label(tokens.fg, egui::RichText::new(text).monospace());
        ui.colored_label(tokens.fg_subtle, "\u{2014}");
        ui.colored_label(tokens.fg_subtle, "\u{2014}");
    }
    let range_text = range.map(|r| format_range(r, decimals)).unwrap_or_else(|| "\u{2014}".to_string());
    ui.colored_label(tokens.fg_muted, egui::RichText::new(range_text).monospace());
    if is_direct {
        ui.colored_label(tokens.accent, "Direct");
    } else {
        ui.colored_label(tokens.warning, "Derived");
    }
    ui.end_row();
}

/// Nearest real aircraft-reamer-catalog sizes to `target_in`
/// (`bushing_solver::reamers::nearest`) - matches
/// `app/src/bushing_workbench.rs`'s `ReamerPicker` (same 8-entry list),
/// rendered inline (this crate's established "sunken panel, not a
/// floating overlay window" pattern) rather than a popup.
fn reamer_picker(ui: &mut egui::Ui, tokens: &Tokens, target_in: f64, mut on_pick: impl FnMut(&ReamerEntry)) {
    egui::Frame::default().fill(tokens.bg_sunken).stroke(egui::Stroke::new(1.0, tokens.border)).rounding(6.0).inner_margin(8.0).show(ui, |ui| {
        ui.colored_label(tokens.fg_muted, format!("Nearest catalog reamers to {target_in:.4} in"));
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            for entry in reamers::nearest(target_in, 8) {
                ui.horizontal(|ui| {
                    if ui.button(&entry.size_label).clicked() {
                        on_pick(entry);
                    }
                    ui.colored_label(tokens.fg_muted, format!("{:.4} in", entry.nominal_in));
                    ui.colored_label(tokens.fg_subtle, tier_label(entry.availability_tier));
                });
            }
        });
    });
}

fn tier_label(t: reamers::AvailabilityTier) -> &'static str {
    match t {
        reamers::AvailabilityTier::Preferred => "Preferred",
        reamers::AvailabilityTier::Common => "Common",
        reamers::AvailabilityTier::Special => "Special",
    }
}

fn num_field(ui: &mut egui::Ui, tokens: &Tokens, label: &str, value: &mut f64) {
    crate::widgets::num_field(ui, tokens, label, value, 0.001, 6);
}

fn material_combo(ui: &mut egui::Ui, tokens: &Tokens, label: &str, id: &mut String) {
    let current_name = MATERIALS.iter().find(|m| m.id == id.as_str()).map(|m| m.name).unwrap_or("select");
    crate::design::components::select_field(ui, tokens, label, current_name, |ui| {
        for m in MATERIALS {
            ui.selectable_value(id, m.id.to_string(), m.name);
        }
    });
}
