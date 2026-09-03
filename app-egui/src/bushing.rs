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
//! NOT yet ported (tracked, not dropped): internal countersink ID
//! geometry, the cross-section visualizer + lightbox, the full
//! worst-case-across-tolerance derivation view.

use eframe::egui;
use bushing_solver::geometry::BushingType;
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use mechanics_core::materials::MATERIALS;

use crate::components::{checks_tiles, status_rail, CheckRow};
use crate::sketches::{bushing_head_on, bushing_side_view};
use crate::theme::Tokens;
use crate::widgets::{card, headline, stepper};

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
const STEPS: [(Step, &str); 6] = [
    (Step::Repair, "Repair"),
    (Step::Geometry, "Geometry"),
    (Step::Material, "Material"),
    (Step::Fit, "Fit"),
    (Step::Analysis, "Analysis"),
    (Step::Results, "Results"),
];

pub struct BushingTool {
    step: Step,
    pub bore_dia: f64,
    pub id_bushing: f64,
    pub housing_len: f64,
    pub housing_width: f64,
    pub edge_dist: f64,
    pub interference: f64,
    pub mat_housing: String,
    pub mat_bushing: String,
    pub d_t: f64,
    pub min_wall_straight: f64,
    pub friction: f64,
    pub end_constraint: EndConstraint,
    pub edge_load_angle_deg: f64,
    pub load: f64,
    pub assembly_thermal_assist: bool,
    pub assembly_housing_temperature: f64,
    pub assembly_bushing_temperature: f64,
    pub flanged: bool,
    pub flange_od: f64,
    pub flange_thk: f64,
}

impl Default for BushingTool {
    fn default() -> Self {
        Self {
            step: Step::default(),
            bore_dia: 0.8760,
            id_bushing: 0.500,
            housing_len: 1.25,
            housing_width: 2.00,
            edge_dist: 0.375,
            interference: 0.0015,
            mat_housing: "al2024".to_string(),
            mat_bushing: "steel".to_string(),
            d_t: 0.0,
            min_wall_straight: 0.1875,
            friction: 0.15,
            end_constraint: EndConstraint::Free,
            edge_load_angle_deg: 40.0,
            load: 1000.0,
            assembly_thermal_assist: false,
            assembly_housing_temperature: 70.0,
            assembly_bushing_temperature: 70.0,
            flanged: false,
            flange_od: 1.10,
            flange_thk: 0.062,
        }
    }
}

fn form_and_sketch(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    emph: &str,
    form: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(280.0);
            card(ui, tokens, |ui| {
                ui.heading(title);
                ui.add_space(4.0);
                form(ui);
            });
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            egui::Frame::default().fill(tokens.bg_sunken).stroke(egui::Stroke::new(1.0, tokens.border)).rounding(8.0).inner_margin(14.0).show(ui, |ui| {
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("HEAD-ON CROSS-SECTION").size(9.5).strong());
                bushing_head_on(ui, tokens, emph);
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("SIDE (LONGITUDINAL) VIEW").size(9.5).strong());
                bushing_side_view(ui, tokens, emph);
            });
        });
    });
}

impl BushingTool {
    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        stepper(ui, tokens, &STEPS, &mut self.step);
        ui.add_space(10.0);

        match self.step {
            Step::Repair => form_and_sketch(ui, tokens, "01 \u{b7} Repair", "repair", |ui| {
                num_field(ui, "Housing bore diameter (in)", &mut self.bore_dia);
                num_field(ui, "Bushing ID (in)", &mut self.id_bushing);
                num_field(ui, "Housing length (in)", &mut self.housing_len);
                num_field(ui, "Housing width (in)", &mut self.housing_width);
                num_field(ui, "Edge distance (in)", &mut self.edge_dist);
            }),
            Step::Geometry => form_and_sketch(ui, tokens, "02 \u{b7} Geometry", "geom", |ui| {
                ui.checkbox(&mut self.flanged, "Flanged (external geometry)");
                ui.add_enabled_ui(self.flanged, |ui| {
                    num_field(ui, "Flange OD (in)", &mut self.flange_od);
                    num_field(ui, "Flange thickness (in)", &mut self.flange_thk);
                });
            }),
            Step::Material => form_and_sketch(ui, tokens, "03 \u{b7} Material", "material", |ui| {
                material_combo(ui, "Housing material", &mut self.mat_housing);
                material_combo(ui, "Bushing material", &mut self.mat_bushing);
            }),
            Step::Fit => form_and_sketch(ui, tokens, "04 \u{b7} Fit", "fit", |ui| {
                num_field(ui, "Nominal diametral interference (in)", &mut self.interference);
            }),
            Step::Analysis => {
                card(ui, tokens, |ui| {
                    ui.heading("05 \u{b7} Analysis \u{2014} acceptance criteria");
                    num_field(ui, "Minimum straight wall (in)", &mut self.min_wall_straight);
                });
                ui.add_space(10.0);
                card(ui, tokens, |ui| {
                    ui.heading("05 \u{b7} Analysis \u{2014} environment & install");
                    num_field(ui, "Friction coefficient", &mut self.friction);
                    ui.horizontal(|ui| {
                        ui.label("End constraint");
                        if ui.selectable_label(self.end_constraint == EndConstraint::Free, "Free").clicked() {
                            self.end_constraint = EndConstraint::Free;
                        }
                        if ui.selectable_label(self.end_constraint == EndConstraint::OneEnd, "One end").clicked() {
                            self.end_constraint = EndConstraint::OneEnd;
                        }
                        if ui.selectable_label(self.end_constraint == EndConstraint::BothEnds, "Both ends").clicked() {
                            self.end_constraint = EndConstraint::BothEnds;
                        }
                    });
                    num_field(ui, "\u{394}T from install, \u{b0}F", &mut self.d_t);
                    num_field(ui, "Edge load angle (deg)", &mut self.edge_load_angle_deg);
                    num_field(ui, "Applied edge load (lbf)", &mut self.load);
                    ui.checkbox(&mut self.assembly_thermal_assist, "Thermally assisted install (shrink/expand fit)");
                    if self.assembly_thermal_assist {
                        num_field(ui, "Housing temp at install (\u{b0}F)", &mut self.assembly_housing_temperature);
                        num_field(ui, "Bushing temp at install (\u{b0}F)", &mut self.assembly_bushing_temperature);
                    }
                });
            }
            Step::Results => self.results(ui, tokens),
        }
    }

    fn inputs(&self) -> BushingInputs {
        BushingInputs {
            bore_dia: self.bore_dia,
            id_bushing: self.id_bushing,
            interference: self.interference,
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
            min_wall_neck: self.min_wall_straight,
            bushing_type: if self.flanged { BushingType::Flanged } else { BushingType::Straight },
            flange_od: self.flange_od,
            flange_thk: self.flange_thk,
            ..BushingInputs::default()
        }
    }

    fn results(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        let out = compute(&self.inputs());
        let passed = out.candidates.iter().filter(|c| c.margin.is_finite() && c.margin >= 0.0).count();
        let all_pass = passed == out.candidates.len();
        let check_rows: Vec<CheckRow> =
            out.candidates.iter().map(|c| CheckRow { name: c.name, applied: 0.0, allowable: 0.0, margin: c.margin, unit: "" }).collect();

        headline(
            ui,
            tokens,
            all_pass,
            passed,
            check_rows.len(),
            &[
                ("GOVERNING", format!("{} ({:+.2})", out.governing.name, out.governing.margin), None),
                ("INSTALLED OD", format!("\u{2300}{:.4} in", out.od_installed), None),
                ("WALL (STRAIGHT)", format!("{:.4} in", out.wall_straight), None),
            ],
        );
        ui.add_space(10.0);

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width() - 250.0);
                card(ui, tokens, |ui| {
                    ui.heading("Fabrication & install summary");
                    egui::Grid::new("bushing_spec_grid").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
                        ui.colored_label(tokens.fg_muted, "Installed OD");
                        ui.label(format!("\u{2300}{:.4} in", out.od_installed));
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
                    ui.heading("Checks");
                    checks_tiles(ui, tokens, &check_rows);
                });
            });
            ui.add_space(14.0);
            status_rail(ui, tokens, &check_rows);
        });
    }
}

fn num_field(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.001));
    });
}

fn material_combo(ui: &mut egui::Ui, label: &str, id: &mut String) {
    let current_name = MATERIALS.iter().find(|m| m.id == id.as_str()).map(|m| m.name).unwrap_or("select");
    egui::ComboBox::from_label(label).selected_text(current_name).show_ui(ui, |ui| {
        for m in MATERIALS {
            ui.selectable_value(id, m.id.to_string(), m.name);
        }
    });
}
