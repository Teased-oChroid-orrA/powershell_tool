//! Precision Studio page. Sizing and solid coordinates come from the solver.
use super::*;
use crate::gpu_viewer::editor::{
    session::EditorAction,
    stackup::{PrecisionPage, StackupAct},
};
use bushing_solver::stackup::{ResultSet, SolidLayer, Status};
#[derive(Clone, Copy, Default, PartialEq)]
enum ResultTab {
    #[default]
    Sizing,
    Wall,
    Details,
}
pub(super) struct PrecisionState {
    exploded: bool,
    spacing: f32,
    tab: ResultTab,
    zoom: f32,
}
impl Default for PrecisionState {
    fn default() -> Self {
        Self {
            exploded: true,
            spacing: 0.55,
            tab: ResultTab::Sizing,
            zoom: 1.0,
        }
    }
}
fn send(out: &mut Vec<Outcome>, a: StackupAct) {
    out.push(Outcome::Action(UiAction::Editor(EditorAction::Stackup(a))));
}
fn surface(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(10)
        .inner_margin(14)
}
impl Gui {
    pub(super) fn precision_page(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        doc: &Document,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let page = doc.precision.as_ref().unwrap();
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(area.shrink2(Vec2::new(16.0, 5.0)))
                .id_salt("precision-studio"),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("precision-page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("GS ENGINEERING  /  TOOLBENCH")
                                        .size(10.0)
                                        .color(p.dim),
                                );
                                ui.label(RichText::new("Bushing Stackup").size(26.0).strong());
                                ui.label(
                                    RichText::new(format!(
                                        "{} members  ·  Inches / degrees  ·  Common finished axis",
                                        page.inputs.members.len()
                                    ))
                                    .small()
                                    .color(p.dim),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for label in ["Save", "Recalculate"] {
                                    if let Some(e) = doc.buttons.iter().find(|e| e.label == label) {
                                        self.doc_button(
                                            ui,
                                            e,
                                            p,
                                            out,
                                            hint,
                                            label == "Recalculate",
                                        );
                                    }
                                }
                            });
                        });
                        ui.horizontal_wrapped(|ui| {
                            self.panel_buttons(ui, &doc.panels, p, out, hint);
                            for chip in &doc.chips {
                                let applicable = match chip.action {
                                    UiAction::Editor(EditorAction::Stackup(
                                        StackupAct::ToggleBushing | StackupAct::Mode,
                                    )) => true,
                                    _ => page.inputs.bushing_enabled,
                                };
                                if chip.enabled && applicable {
                                    self.chip(ui, chip, p, out, hint);
                                }
                            }
                        });
                        if let Some(error) = &page.error {
                            ui.label(RichText::new(error).color(p.warn));
                        }
                        if !page.current || page.result.is_none() {
                            ui.label(
                                RichText::new(
                                    doc.notice
                                        .as_ref()
                                        .map(|n| n.text.as_str())
                                        .unwrap_or("Recalculate to update geometry"),
                                )
                                .color(p.warn),
                            );
                        }
                        let width = ui.available_width();
                        let wide = width >= 1000.0;
                        let compact = width < 720.0;
                        if !wide {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new("MEMBERS").small().color(p.dim));
                                for (i, m) in page.inputs.members.iter().enumerate() {
                                    if ui
                                        .selectable_label(
                                            i == page.selected,
                                            format!("{} · {}", i + 1, m.name),
                                        )
                                        .clicked()
                                    {
                                        send(out, StackupAct::Select(i));
                                    }
                                }
                            });
                        }
                        if !wide {
                            ui.horizontal_wrapped(|ui| {
                                for (label, a) in [
                                    ("+ Add member", StackupAct::Add),
                                    ("Remove", StackupAct::Remove),
                                    ("Move up", StackupAct::Up),
                                    ("Move down", StackupAct::Down),
                                ] {
                                    if ui.small_button(label).clicked() {
                                        send(out, a);
                                    }
                                }
                            });
                        }
                        let h = (area.height() - 290.0).clamp(430.0, 620.0);
                        let inspector_w = if compact { width } else { 248.0 };
                        let member_w = if wide { 172.0 } else { 0.0 };
                        let canvas_w = if compact {
                            width
                        } else {
                            (width - inspector_w - member_w - if wide { 24.0 } else { 12.0 })
                                .max(200.0)
                        };
                        ui.horizontal_top(|ui| {
                            if wide {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(member_w, h),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        surface(p).show(ui, |ui| {
                                            ui.set_width(member_w - 28.0);
                                            ui.set_min_height(h - 28.0);
                                            ui.label(
                                                RichText::new("STACK MEMBERS")
                                                    .size(10.0)
                                                    .strong()
                                                    .color(p.dim),
                                            );
                                            ui.add_space(10.0);
                                            egui::ScrollArea::vertical()
                                                .id_salt("precision-members")
                                                .max_height(h - 145.0)
                                                .show(ui, |ui| {
                                                    for (i, m) in
                                                        page.inputs.members.iter().enumerate()
                                                    {
                                                        let response = ui.add_sized(
                                                            [ui.available_width(), 48.0],
                                                            egui::Button::new(format!(
                                                                "{:02}   {}\n        {:.4} in",
                                                                i + 1,
                                                                m.name,
                                                                m.thickness
                                                            ))
                                                            .selected(i == page.selected),
                                                        );
                                                        self.targets.push((
                                                            format!("Member {}", m.id),
                                                            response.rect,
                                                        ));
                                                        if response.clicked() {
                                                            send(out, StackupAct::Select(i));
                                                        }
                                                    }
                                                });
                                            ui.add_space(10.0);
                                            ui.horizontal_wrapped(|ui| {
                                                for (label, a) in [
                                                    ("+ Add", StackupAct::Add),
                                                    ("Remove", StackupAct::Remove),
                                                    ("Up", StackupAct::Up),
                                                    ("Down", StackupAct::Down),
                                                ] {
                                                    if ui.small_button(label).clicked() {
                                                        send(out, a);
                                                    }
                                                }
                                            });
                                        });
                                    },
                                );
                            }
                            ui.allocate_ui_with_layout(
                                Vec2::new(canvas_w, h),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    surface(p).show(ui, |ui| {
                                        ui.set_width(canvas_w - 28.0);
                                        self.precision_canvas(ui, page, p, out, h - 28.0);
                                    });
                                },
                            );
                            if !compact {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(inspector_w, h),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        surface(p).show(ui, |ui| {
                                            ui.set_width(inspector_w - 28.0);
                                            ui.set_height(h - 28.0);
                                            self.precision_inspector(
                                                ui,
                                                doc,
                                                page,
                                                p,
                                                out,
                                                hint,
                                                h - 28.0,
                                            );
                                        });
                                    },
                                );
                            }
                        });
                        if compact {
                            surface(p).show(ui, |ui| {
                                self.precision_inspector(ui, doc, page, p, out, hint, 500.0)
                            });
                        }
                        self.precision_results(ui, doc, page, p, out);
                    });
            },
        );
    }
    fn precision_inspector(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        page: &PrecisionPage,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
        height: f32,
    ) {
        let m = &page.inputs.members[page.selected];
        egui::ScrollArea::vertical()
            .id_salt("precision-selected")
            .max_height(height)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("SELECTED MEMBER  {:02}", page.selected + 1))
                        .size(10.0)
                        .color(p.dim),
                );
                ui.label(RichText::new(&m.name).size(18.0).strong());
                ui.add_space(12.0);
                let input = |id| {
                    doc.panels
                        .iter()
                        .flat_map(|panel| &panel.inputs)
                        .find(|field| field.id == id)
                };
                if let Some(field) = input(12) {
                    self.labelled_input(ui, field, ui.available_width(), p, out, hint);
                }
                if page.current {
                    if let Some(r) = page
                        .result
                        .as_ref()
                        .and_then(|r| r.layers.get(page.selected))
                    {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("FINAL CLEANED MEMBER HOLE")
                                .size(10.0)
                                .color(p.dim),
                        );
                        ui.label(
                            RichText::new(format!("{:.5} in", r.final_hole.min))
                                .size(27.0)
                                .strong()
                                .color(
                                    if matches!(r.status, Status::Feasible | Status::Informational)
                                    {
                                        p.accent
                                    } else {
                                        p.warn
                                    },
                                ),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Angularity-only cleanup  {:.5} in",
                                r.cleanup_diameter
                            ))
                            .small()
                            .color(p.dim),
                        );
                        if let Some(reason) = &r.enlargement_reason {
                            ui.label(
                                RichText::new(format!("ENLARGED · {reason}"))
                                    .small()
                                    .color(p.warn),
                            );
                        }
                        ui.label(
                            RichText::new(format!("{:?} · {}", r.status, r.governing))
                                .small()
                                .color(
                                    if matches!(r.status, Status::Feasible | Status::Informational)
                                    {
                                        p.good
                                    } else {
                                        p.warn
                                    },
                                ),
                        );
                        ui.add_space(12.0);
                        ui.separator();
                        if page.inputs.bushing_enabled {
                            for (label, value) in [
                                (
                                    "Bushing ID",
                                    format!("{:.5} in", page.inputs.finished_id.nominal),
                                ),
                                ("Installed bushing OD", format!("{:.5} in", r.od.nominal)),
                                ("Minimum wall", format!("{:.5} in", r.wall_min)),
                                ("Maximum wall", format!("{:.5} in", r.wall_max)),
                                (
                                    "Required wall",
                                    crate::gpu_viewer::editor::stackup::optional(
                                        r.policy.effective,
                                    ),
                                ),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(label).small().color(p.dim));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(RichText::new(value).small().monospace());
                                    });
                                });
                            }
                            ui.label(RichText::new(&r.comparison).small().color(p.dim));
                        }
                    }
                } else {
                    ui.label(RichText::new("Results stale · Recalculate").color(p.warn));
                }
                ui.add_space(12.0);
                ui.collapsing("Member dimensions & tolerances", |ui| {
                    for id in [
                        10, 11, 13, 14, 30, 31, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 28, 29,
                        26, 27,
                    ] {
                        if let Some(field) = input(id) {
                            self.labelled_input(ui, field, ui.available_width(), p, out, hint);
                            ui.add_space(7.0);
                        }
                    }
                });
                if let Some(entry) = doc.buttons.iter().find(|e| e.label == "Send to Bushing") {
                    ui.add_space(12.0);
                    self.doc_button(ui, entry, p, out, hint, false);
                }
            });
    }
    fn precision_results(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        page: &PrecisionPage,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        if !page.inputs.bushing_enabled && self.doc.precision.tab == ResultTab::Wall {
            self.doc.precision.tab = ResultTab::Sizing;
        }
        surface(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                for (label, tab) in [
                    ("Hole sizing", ResultTab::Sizing),
                    ("Wall policy", ResultTab::Wall),
                    ("Full results", ResultTab::Details),
                ] {
                    if ui
                        .selectable_label(self.doc.precision.tab == tab, label)
                        .clicked()
                    {
                        self.doc.precision.tab = tab;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(if page.current {
                            "Current calculation"
                        } else {
                            "STALE"
                        })
                        .small()
                        .color(if page.current { p.good } else { p.warn }),
                    );
                });
            });
            ui.add_space(8.0);
            if self.doc.precision.tab == ResultTab::Details {
                let table = &doc.table;
                egui::ScrollArea::both()
                    .id_salt("precision-full")
                    .max_height(250.0)
                    .show(ui, |ui| {
                        egui::Grid::new("precision-full-grid")
                            .striped(true)
                            .show(ui, |ui| {
                                for c in &table.columns {
                                    ui.label(RichText::new(&c.title).small().color(p.dim));
                                }
                                ui.end_row();
                                for row in &table.rows {
                                    for cell in &row.cells {
                                        ui.label(RichText::new(cell).small());
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                return;
            }
            egui::ScrollArea::both()
                .id_salt("precision-results")
                .max_height(225.0)
                .show(ui, |ui| {
                    egui::Grid::new("precision-result-grid")
                        .striped(true)
                        .min_col_width(110.0)
                        .spacing([22.0, 12.0])
                        .show(ui, |ui| {
                            let headers = if self.doc.precision.tab == ResultTab::Wall {
                                vec![
                                    "Member",
                                    "Effective minimum",
                                    "Source / mode",
                                    "Calculated min",
                                    "Comparison",
                                ]
                            } else {
                                let mut h = vec![
                                    "Member",
                                    "Current hole",
                                    "Angularity cleanup",
                                    "Final adopted hole",
                                    "Enlargement",
                                ];
                                if page.inputs.bushing_enabled {
                                    h.extend(["Bushing ID", "Installed OD", "Min wall"]);
                                }
                                h.push("Status");
                                h
                            };
                            for h in headers {
                                ui.label(RichText::new(h).small().color(p.dim));
                            }
                            ui.end_row();
                            for (i, m) in page.inputs.members.iter().enumerate() {
                                if ui.selectable_label(i == page.selected, &m.name).clicked() {
                                    send(out, StackupAct::Select(i));
                                }
                                if let Some(r) = page
                                    .result
                                    .as_ref()
                                    .filter(|_| page.current)
                                    .and_then(|r| r.layers.get(i))
                                {
                                    let cells = if self.doc.precision.tab == ResultTab::Wall {
                                        vec![
                                            r.policy
                                                .effective
                                                .map(|v| format!("{v:.5} in"))
                                                .unwrap_or_else(|| "Not configured".into()),
                                            format!("{} / {:?}", r.policy.source, r.policy.mode),
                                            if page.inputs.bushing_enabled {
                                                format!("{:.5} in", r.wall_min)
                                            } else {
                                                "Not applicable".into()
                                            },
                                            r.comparison.clone(),
                                        ]
                                    } else {
                                        let mut c = vec![
                                            format!("{:.5} in", m.hole.nominal),
                                            format!("{:.5} in", r.cleanup_diameter),
                                            format!("{:.5} in", r.final_hole.min),
                                            r.enlargement_reason
                                                .clone()
                                                .unwrap_or_else(|| "None".into()),
                                        ];
                                        if page.inputs.bushing_enabled {
                                            c.extend([
                                                format!("{:.5}", page.inputs.finished_id.nominal),
                                                format!("{:.5}", r.od.nominal),
                                                format!("{:.5}", r.wall_min),
                                            ]);
                                        }
                                        c.push(format!("{:?}", r.status));
                                        c
                                    };
                                    for c in cells {
                                        ui.label(RichText::new(c).small());
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Recalculate to update this member")
                                            .small()
                                            .color(p.warn),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
        });
    }
    fn precision_canvas(
        &mut self,
        ui: &mut egui::Ui,
        page: &PrecisionPage,
        p: &Palette,
        out: &mut Vec<Outcome>,
        height: f32,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Stack geometry").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Fit").clicked() {
                    self.doc.precision.zoom = 1.0;
                }
                if ui.small_button("+").clicked() {
                    self.doc.precision.zoom = (self.doc.precision.zoom * 1.2).min(4.0);
                }
                if ui.small_button("−").clicked() {
                    self.doc.precision.zoom = (self.doc.precision.zoom / 1.2).max(0.5);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.doc.precision.exploded, false, "Fitted");
            ui.selectable_value(&mut self.doc.precision.exploded, true, "Exploded");
            ui.add(
                egui::Slider::new(&mut self.doc.precision.spacing, 0.15..=1.0)
                    .show_value(false)
                    .text("Separation"),
            );
        });
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), (height - 85.0).max(150.0)),
            Sense::click(),
        );
        let Some(result) = page.result.as_ref().filter(|_| page.current) else {
            ui.painter_at(rect).text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Recalculate to update the geometry",
                FontId::proportional(14.0),
                p.dim,
            );
            return;
        };
        let progress = ui
            .ctx()
            .animate_bool(Id::new("precision-explode"), self.doc.precision.exploded)
            as f64;
        paint_solids(
            ui,
            rect,
            response.hover_pos(),
            response.clicked(),
            page,
            result,
            progress,
            self.doc.precision.spacing as f64,
            self.doc.precision.zoom,
            p,
            out,
        );
        ui.label(
            RichText::new("Translucent members · amber original hole · blue cleaned hole · display offsets only")
                .size(10.0)
                .color(p.dim),
        );
    }
}
fn explosion_offsets(
    solids: &[SolidLayer],
    bushing: bool,
    gap: f64,
    progress: f64,
) -> Vec<[f64; 2]> {
    let mut cursor = 0.0;
    solids
        .iter()
        .map(|solid| {
            let t = solid.z[1] - solid.z[0];
            let bush = cursor - solid.z[0];
            if bushing {
                cursor += t + gap;
            }
            let member = cursor - solid.z[0];
            cursor += t + gap;
            let carry = member * (progress * 2.0).min(1.0);
            let extraction = (progress * 2.0 - 1.0).max(0.0);
            [carry, carry + (bush - member) * extraction]
        })
        .collect()
}

fn paint_solids(
    ui: &mut egui::Ui,
    rect: Rect,
    hover: Option<Pos2>,
    clicked: bool,
    page: &PrecisionPage,
    result: &ResultSet,
    progress: f64,
    spacing: f64,
    zoom: f32,
    p: &Palette,
    out: &mut Vec<Outcome>,
) {
    let gap = page
        .inputs
        .members
        .iter()
        .map(|m| m.member_depth * 0.25 + m.member_width * 0.12 + 0.1 + spacing * 0.25)
        .fold(0.0, f64::max);
    let offsets = explosion_offsets(&result.solids, page.inputs.bushing_enabled, gap, progress);
    let project = |v: [f64; 3]| [v[0] * 0.9 + v[1] * 0.45, v[2] + v[1] * 0.25 - v[0] * 0.12];
    let moved = |v: [f64; 3], solid: &SolidLayer, dz: f64| {
        [v[0] + dz * solid.axis[0] / solid.axis[2], v[1], v[2] + dz]
    };
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for (solid, offset) in result.solids.iter().zip(&offsets) {
        for (points, dz) in [
            (solid.body.as_slice(), offset[0]),
            (solid.hole[0].as_slice(), offset[1]),
            (solid.hole[1].as_slice(), offset[1]),
        ] {
            for &point in points {
                let q = project(moved(point, solid, dz));
                for k in 0..2 {
                    low[k] = low[k].min(q[k]);
                    high[k] = high[k].max(q[k]);
                }
            }
        }
    }
    let scale = ((rect.width() - 55.0) as f64 / (high[0] - low[0]).max(0.001))
        .min((rect.height() - 32.0) as f64 / (high[1] - low[1]).max(0.001))
        * zoom as f64;
    let center = [low[0] / 2.0 + high[0] / 2.0, low[1] / 2.0 + high[1] / 2.0];
    let pixel = |v: [f64; 3], solid: &SolidLayer, dz: f64| {
        let a = project(moved(v, solid, dz));
        Pos2::new(
            rect.center().x + ((a[0] - center[0]) * scale) as f32,
            rect.center().y + ((a[1] - center[1]) * scale) as f32,
        )
    };
    let painter = ui.painter_at(rect);
    for (index, (solid, offset)) in result.solids.iter().zip(&offsets).enumerate() {
        let selected = index == page.selected;
        let edge = if selected { p.accent } else { p.dim };
        let body: Vec<_> = solid
            .body
            .iter()
            .map(|&v| pixel(v, solid, offset[0]))
            .collect();
        let body_bounds = Rect::from_points(&body);
        if body_bounds.height() < 3.0 && !selected {
            painter.line_segment(
                [body_bounds.left_center(), body_bounds.right_center()],
                Stroke::new(0.7, with_alpha(p.dim, 90)),
            );
            continue;
        }
        for face in [
            [0, 1, 2, 3],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
            [4, 5, 6, 7],
        ] {
            painter.add(Shape::convex_polygon(
                face.map(|i| body[i]).to_vec(),
                with_alpha(
                    if selected { p.accent } else { p.dim },
                    if selected { 22 } else { 13 },
                ),
                Stroke::new(0.8, with_alpha(edge, 110)),
            ));
        }
        let hole: Vec<Vec<Pos2>> = solid
            .hole
            .iter()
            .map(|ring| ring.iter().map(|&v| pixel(v, solid, offset[0])).collect())
            .collect();
        for ring in &solid.original_hole {
            let points: Vec<_> = ring.iter().map(|&v| pixel(v, solid, offset[0])).collect();
            for j in (0..points.len()).step_by(2) {
                painter.line_segment(
                    [points[j], points[(j + 1) % points.len()]],
                    Stroke::new(0.8, with_alpha(p.warn, 160)),
                );
            }
        }
        for ring in &hole {
            let mut points = ring.clone();
            points.push(points[0]);
            painter.add(Shape::line(
                points,
                Stroke::new(1.0, with_alpha(p.accent, 190)),
            ));
        }
        for j in (0..hole[0].len()).step_by(8) {
            painter.line_segment(
                [hole[0][j], hole[1][j]],
                Stroke::new(0.6, with_alpha(p.accent, 90)),
            );
        }
        if let Some(inner) = &solid.bushing_id {
            let outer: Vec<Vec<Pos2>> = solid
                .hole
                .iter()
                .map(|ring| ring.iter().map(|&v| pixel(v, solid, offset[1])).collect())
                .collect();
            let inside: Vec<Vec<Pos2>> = inner
                .iter()
                .map(|ring| ring.iter().map(|&v| pixel(v, solid, offset[1])).collect())
                .collect();
            let bush_bounds =
                Rect::from_points(&outer.iter().flatten().copied().collect::<Vec<_>>());
            if clicked && hover.is_some_and(|at| bush_bounds.contains(at)) {
                send(out, StackupAct::Select(index));
            }
            let n = outer[0].len();
            for j in 0..n {
                let k = (j + 1) % n;
                let tint =
                    (130.0 + 55.0 * (j as f64 * std::f64::consts::TAU / n as f64).cos()) as u8;
                painter.add(Shape::convex_polygon(
                    vec![outer[0][j], outer[0][k], outer[1][k], outer[1][j]],
                    with_alpha(p.accent, tint),
                    Stroke::NONE,
                ));
                painter.add(Shape::convex_polygon(
                    vec![inside[0][j], inside[1][j], inside[1][k], inside[0][k]],
                    with_alpha(p.dim, 130),
                    Stroke::NONE,
                ));
                for end in [0, 1] {
                    painter.add(Shape::convex_polygon(
                        vec![outer[end][j], outer[end][k], inside[end][k], inside[end][j]],
                        with_alpha(p.accent, 210),
                        Stroke::NONE,
                    ));
                }
            }
            for ring in outer.iter().chain(&inside) {
                let mut ring = ring.clone();
                ring.push(ring[0]);
                painter.add(Shape::line(
                    ring,
                    Stroke::new(
                        0.8,
                        with_alpha(if selected { p.accent } else { p.dim }, 220),
                    ),
                ));
            }
            if progress > 0.5 {
                let first = solid.hole[0][0];
                let center3 = [
                    first[0] - (solid.hole[0][0][0] - solid.hole[0][24][0]) / 2.0,
                    0.0,
                    solid.z[0],
                ];
                painter.line_segment(
                    [
                        pixel(center3, solid, offset[0]),
                        pixel(
                            [
                                center3[0]
                                    + (solid.z[1] - solid.z[0]) * solid.axis[0] / solid.axis[2],
                                0.0,
                                solid.z[1],
                            ],
                            solid,
                            offset[1],
                        ),
                    ],
                    Stroke::new(0.6, with_alpha(p.dim, 110)),
                );
            }
        }
        let bounds = Rect::from_points(&body);
        if hover.is_some_and(|at| bounds.contains(at)) {
            if clicked {
                send(out, StackupAct::Select(index));
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let name = &page.inputs.members[index].name;
        painter.text(
            Pos2::new(bounds.right() + 8.0, bounds.center().y),
            Align2::LEFT_CENTER,
            format!("{:02} {}", index + 1, name),
            FontId::proportional(10.0),
            if selected { p.text } else { p.dim },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exploded_parts_never_cross_other_members() {
        let mut input = bushing_solver::stackup::Inputs::default();
        input.bushing_enabled = true;
        input.members.push(bushing_solver::stackup::Member::new(3));
        let r = bushing_solver::stackup::calculate(&input).unwrap();
        for gap in [0.1, 0.5, 1.0] {
            for step in 0..=100 {
                let progress = step as f64 / 100.0;
                let offsets = explosion_offsets(&r.solids, true, gap, progress);
                let mut parts = Vec::new();
                for (i, (s, o)) in r.solids.iter().zip(&offsets).enumerate() {
                    for kind in 0..2 {
                        parts.push((i, kind, s.z[0] + o[kind], s.z[1] + o[kind]));
                    }
                }
                for (i, a) in parts.iter().enumerate() {
                    for b in &parts[i + 1..] {
                        if a.0 == b.0 && progress < 1.0 {
                            continue;
                        }
                        assert!(
                            a.3 <= b.2 + 1e-12 || b.3 <= a.2 + 1e-12,
                            "{a:?} intersects {b:?} at {progress}"
                        );
                    }
                }
            }
        }
    }
}
