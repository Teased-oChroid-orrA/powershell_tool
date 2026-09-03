//! Bushing Workbench - egui port of `app/src/bushing_workbench.rs`.
//!
//! `bushing-solver::solve::compute` is pure, framework-agnostic math -
//! reused unchanged. `BushingInputs` derives `Default`, and its own doc
//! comment guarantees every field this pass doesn't expose defaults to
//! "the exact values that reproduce the original straight-bushing-only
//! behavior" (`BushingType::Straight`, `IdType::Straight`,
//! `EnforcementPolicy::default()`) - so leaving them at `..Default::
//! default()` is a real, documented, correct construction, not a guess.
//!
//! Scope of this pass (real, working): the straight-bushing case only -
//! bore/ID/interference, housing length/width/edge distance, both
//! materials, end constraint, delta-T, minimum straight wall, friction,
//! edge load (angle + magnitude), optional thermally-assisted install,
//! and the real Checks/governing margin from `compute()`'s own output.
//! NOT yet ported (tracked, not dropped): flanged/countersink OD
//! geometry, internal countersink ID geometry, the cross-section
//! visualizer + lightbox, the full worst-case-across-tolerance
//! derivation view.

use eframe::egui;
use bushing_solver::solve::{compute, BushingInputs, EndConstraint};
use mechanics_core::materials::MATERIALS;

use crate::components::{checks_tiles, status_rail, CheckRow};
use crate::theme::Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Step {
    #[default]
    Geometry,
    Fit,
    Material,
    Loads,
    Results,
}
const STEPS: [Step; 5] = [Step::Geometry, Step::Fit, Step::Material, Step::Loads, Step::Results];
impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Geometry => "Geometry",
            Step::Fit => "Fit",
            Step::Material => "Material",
            Step::Loads => "Loads",
            Step::Results => "Results",
        }
    }
}

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
        }
    }
}

impl BushingTool {
    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            for s in STEPS {
                if ui.selectable_label(self.step == s, s.label()).clicked() {
                    self.step = s;
                }
            }
        });
        ui.add_space(10.0);

        match self.step {
            Step::Geometry => {
                ui.heading("01 \u{b7} Geometry");
                num_field(ui, "Housing bore diameter (in)", &mut self.bore_dia);
                num_field(ui, "Bushing ID (in)", &mut self.id_bushing);
                num_field(ui, "Housing length (in)", &mut self.housing_len);
                num_field(ui, "Housing width (in)", &mut self.housing_width);
                num_field(ui, "Edge distance (in)", &mut self.edge_dist);
            }
            Step::Fit => {
                ui.heading("02 \u{b7} Fit");
                num_field(ui, "Nominal diametral interference (in)", &mut self.interference);
                num_field(ui, "Minimum straight wall (in)", &mut self.min_wall_straight);
                num_field(ui, "\u{394}T from install, \u{b0}F", &mut self.d_t);
            }
            Step::Material => {
                ui.heading("03 \u{b7} Material");
                material_combo(ui, "Housing material", &mut self.mat_housing);
                material_combo(ui, "Bushing material", &mut self.mat_bushing);
            }
            Step::Loads => {
                ui.heading("04 \u{b7} Loads & install");
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
                num_field(ui, "Edge load angle (deg)", &mut self.edge_load_angle_deg);
                num_field(ui, "Applied edge load (lbf)", &mut self.load);
                ui.checkbox(&mut self.assembly_thermal_assist, "Thermally assisted install (shrink/expand fit)");
                if self.assembly_thermal_assist {
                    num_field(ui, "Housing temp at install (\u{b0}F)", &mut self.assembly_housing_temperature);
                    num_field(ui, "Bushing temp at install (\u{b0}F)", &mut self.assembly_bushing_temperature);
                }
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
            ..BushingInputs::default()
        }
    }

    fn results(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        let out = compute(&self.inputs());
        let all_pass = out.candidates.iter().all(|c| c.margin.is_finite() && c.margin >= 0.0);
        let check_rows: Vec<CheckRow> =
            out.candidates.iter().map(|c| CheckRow { name: c.name, applied: 0.0, allowable: 0.0, margin: c.margin, unit: "" }).collect();

        egui::Frame::default().fill(tokens.bg_raised).stroke(egui::Stroke::new(1.0, tokens.border)).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(if all_pass { tokens.good } else { tokens.warning }, if all_pass { "PASS" } else { "REVIEW" });
                ui.separator();
                ui.label("Governing:");
                ui.colored_label(tokens.fg, format!("{} ({:+.2})", out.governing.name, out.governing.margin));
                ui.separator();
                ui.label(format!("Wall (straight): {:.4} in", out.wall_straight));
            });
        });
        ui.add_space(10.0);

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width() - 250.0);
                egui::Frame::default().fill(tokens.bg_raised).stroke(egui::Stroke::new(1.0, tokens.border)).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
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
                egui::Frame::default().fill(tokens.bg_raised).stroke(egui::Stroke::new(1.0, tokens.border)).rounding(8.0).inner_margin(12.0).show(ui, |ui| {
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
