//! Pressure Vessel Analyzer - egui port of `app/src/pressure_vessel_workbench.rs`,
//! rebuilt to mirror the approved mockup artifact exactly: pill stepper,
//! form + labeled cross-section sketch per step, frozen headline row,
//! Stat Tiles checks, Ladder/Center-spine status rail.
//!
//! `pressure-vessel-solver`/`mechanics-core` are pure, framework-agnostic
//! math - reused completely unchanged. Everything here recomputes every
//! frame directly from the input fields (cheap: these are closed-form/
//! bounded-bisection solves, not iterative simulation), so there is no
//! separate "run" step and no async bridge needed here, unlike Search.
//!
//! Not yet ported (tracked, not dropped): the 8-step KaTeX derivation
//! view.

use eframe::egui;
use mechanics_core::materials::{get_material, MATERIALS};
use pressure_vessel_solver::buckling::{evaluate_buckling, BucklingApplicability};
use pressure_vessel_solver::failure::{evaluate_failure_modes, governing};
use pressure_vessel_solver::geometry::{classify, CylinderGeometry, GeometryClassification};
use pressure_vessel_solver::pressure::{EndCondition, PressureLoading};
use pressure_vessel_solver::thickness::{solve_minimum_thickness, ThicknessSolverInputs, ThicknessSolverOutcome};

use crate::components::{checks_tiles, status_rail, CheckRow};
use crate::sketches::{pv_head_on, pv_side_view};
use crate::theme::Tokens;
use crate::widgets::{card, headline, stepper};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Step {
    #[default]
    Geometry,
    Pressure,
    Material,
    Buckling,
    Results,
}
const STEPS: [(Step, &str); 5] =
    [(Step::Geometry, "Geometry"), (Step::Pressure, "Pressure"), (Step::Material, "Material"), (Step::Buckling, "Buckling"), (Step::Results, "Results")];

pub struct PressureVesselTool {
    step: Step,
    pub outer_diameter: f64,
    pub wall_thickness: f64,
    pub internal_pressure: f64,
    pub external_pressure: f64,
    pub closed_ends: bool,
    pub material_id: String,
    pub required_ms: f64,
    pub unsupported_length: f64,
}

impl Default for PressureVesselTool {
    fn default() -> Self {
        Self {
            step: Step::default(),
            outer_diameter: 6.0,
            wall_thickness: 1.0,
            internal_pressure: 5000.0,
            external_pressure: 0.0,
            closed_ends: true,
            material_id: "al7075".to_string(),
            required_ms: 0.0,
            unsupported_length: 0.0,
        }
    }
}

/// Form (280px, matches the mockup's `.form-card` width) beside a
/// labeled sketch pair (head-on + side view) flexing to fill the rest -
/// the mockup's `.step-wrap` shape, exactly.
fn form_and_sketch(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    emph: &str,
    head_on: impl Fn(&mut egui::Ui, &Tokens, &str),
    side: impl Fn(&mut egui::Ui, &Tokens, &str),
    form: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(280.0);
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
                head_on(ui, tokens, emph);
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new("SIDE (LONGITUDINAL) VIEW").size(9.5).strong());
                side(ui, tokens, emph);
            });
        });
    });
}

impl PressureVesselTool {
    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        stepper(ui, tokens, &STEPS, &mut self.step);
        ui.add_space(10.0);

        let outer_radius = (self.outer_diameter / 2.0).max(0.0);
        let inner_radius = outer_radius - self.wall_thickness;
        let geometry = CylinderGeometry::new(inner_radius, outer_radius);
        let pressure = PressureLoading::new(
            self.internal_pressure,
            self.external_pressure,
            if self.closed_ends { EndCondition::Closed } else { EndCondition::Open },
        );

        let (geometry, pressure) = match (geometry, pressure) {
            (Ok(g), Ok(p)) => (g, p),
            (g, p) => {
                if let Err(e) = &g {
                    ui.colored_label(tokens.danger, format!("Invalid geometry: {e:?}"));
                }
                if let Err(e) = &p {
                    ui.colored_label(tokens.danger, format!("Invalid pressure: {e:?}"));
                }
                return;
            }
        };
        let material = *get_material(&self.material_id);

        match self.step {
            Step::Geometry => {
                let inner_d = 2.0 * inner_radius;
                form_and_sketch(ui, tokens, "01 \u{b7} Geometry", "od", pv_head_on, pv_side_view, |ui| {
                    num_field(ui, "Outer diameter (in)", &mut self.outer_diameter);
                    num_field(ui, "Wall thickness (in)", &mut self.wall_thickness);
                    ui.colored_label(tokens.fg_subtle, format!("Derived inner diameter: {inner_d:.4} in"));
                });
            }
            Step::Pressure => {
                form_and_sketch(ui, tokens, "02 \u{b7} Pressure & boundary condition", "pressure", pv_head_on, pv_side_view, |ui| {
                    num_field(ui, "Internal pressure (psi)", &mut self.internal_pressure);
                    num_field(ui, "External pressure (psi)", &mut self.external_pressure);
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.closed_ends, "Closed ends").clicked() {
                            self.closed_ends = true;
                        }
                        if ui.selectable_label(!self.closed_ends, "Open ends").clicked() {
                            self.closed_ends = false;
                        }
                    });
                });
            }
            Step::Material => {
                let name = material.name;
                form_and_sketch(ui, tokens, "03 \u{b7} Material & requirement", "material", pv_head_on, pv_side_view, |ui| {
                    egui::ComboBox::from_label("Material").selected_text(name).show_ui(ui, |ui| {
                        for m in MATERIALS {
                            ui.selectable_value(&mut self.material_id, m.id.to_string(), m.name);
                        }
                    });
                    num_field(ui, "Required minimum MS", &mut self.required_ms);
                });
            }
            Step::Buckling => {
                form_and_sketch(ui, tokens, "04 \u{b7} Support spacing (buckling)", "buckling", pv_head_on, pv_side_view, |ui| {
                    num_field(ui, "Unsupported length (in)", &mut self.unsupported_length);
                    ui.colored_label(tokens.fg_subtle, "Only affects the Buckling check, and only when external pressure is present.");
                });
            }
            Step::Results => self.results(ui, tokens, &geometry, &pressure, &material),
        }
    }

    fn results(
        &self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        geometry: &CylinderGeometry,
        pressure: &PressureLoading,
        material: &mechanics_core::materials::Material,
    ) {
        let mut rows = evaluate_failure_modes(geometry, pressure, material);
        let buckling_result = evaluate_buckling(geometry, pressure, material, Some(self.unsupported_length));
        if let BucklingApplicability::Evaluated(ref b) = buckling_result {
            rows.push(b.clone());
        }
        let governing_result = governing(&rows).clone();
        let classification = classify(geometry);
        let thickness_outcome = solve_minimum_thickness(
            &ThicknessSolverInputs { inner_radius: geometry.inner_radius, pressure: *pressure, material: *material, required_minimum_ms: self.required_ms },
            100,
            1e-6,
        );

        let check_rows: Vec<CheckRow> =
            rows.iter().map(|r| CheckRow { name: r.name, applied: r.applied, allowable: r.allowable, margin: r.margin, unit: "psi" }).collect();
        let passed = check_rows.iter().filter(|r| r.margin.is_finite() && r.margin >= 0.0).count();
        let all_pass = passed == check_rows.len();

        headline(
            ui,
            tokens,
            all_pass,
            passed,
            check_rows.len(),
            &[
                ("GOVERNING", format!("{} ({:+.2})", governing_result.name, governing_result.margin), None),
                ("CLASSIFICATION", if classification == GeometryClassification::ThinWall { "Thin-wall".to_string() } else { "Thick-wall".to_string() }, None),
                ("WALL THICKNESS", format!("{:.4} in", geometry.wall_thickness()), None),
            ],
        );
        ui.add_space(10.0);

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width() - 250.0);
                card(ui, tokens, |ui| {
                    crate::widgets::card_title(ui, "Vessel specification summary");
                    egui::Grid::new("pv_spec_grid").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
                        ui.colored_label(tokens.fg_muted, "Outer diameter");
                        ui.label(format!("\u{2300}{:.4} in", 2.0 * geometry.outer_radius));
                        ui.end_row();
                        ui.colored_label(tokens.fg_muted, "Inner diameter");
                        ui.label(format!("\u{2300}{:.4} in", 2.0 * geometry.inner_radius));
                        ui.end_row();
                        ui.colored_label(tokens.fg_muted, "Design pressure");
                        ui.label(format!("{:.0} psi internal, {:.0} psi external", pressure.internal_pressure, pressure.external_pressure));
                        ui.end_row();
                        ui.colored_label(tokens.fg_muted, "Material");
                        ui.label(material.name);
                        ui.end_row();
                    });
                    match thickness_outcome {
                        ThicknessSolverOutcome::Converged(ref sol) => {
                            ui.colored_label(tokens.fg_subtle, format!("Minimum wall thickness solved for the same {:.2} required MS: {:.4} in.", self.required_ms, sol.wall_thickness));
                        }
                        ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found } => {
                            ui.colored_label(tokens.warning, format!("No wall thickness up to {largest_radius_tried:.1} in satisfies the required MS. Best margin found: {best_margin_found:+.2}."));
                        }
                    }
                });
                ui.add_space(10.0);
                card(ui, tokens, |ui| {
                    crate::widgets::card_title(ui, "Checks");
                    checks_tiles(ui, tokens, &check_rows);
                    match buckling_result {
                        BucklingApplicability::NotApplicable => {
                            ui.colored_label(tokens.fg_subtle, "Buckling: not applicable - no external pressure entered.");
                        }
                        BucklingApplicability::InsufficientData => {
                            ui.colored_label(tokens.fg_subtle, "Buckling: insufficient data - enter an unsupported length.");
                        }
                        BucklingApplicability::OutsideValidityRange => {
                            ui.colored_label(tokens.fg_subtle, "Buckling: outside the formulas' validity range (OD/t < 40).");
                        }
                        BucklingApplicability::Evaluated(_) => {}
                    }
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
        ui.add(egui::DragValue::new(value).speed(0.01));
    });
}
