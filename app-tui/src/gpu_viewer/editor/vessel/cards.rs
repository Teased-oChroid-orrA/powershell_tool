//! What the Pressure Vessel workbench shows: the inspector's cards, the tree, the ribbon and the menus. Plain data built
//! from the model; the window draws it. Row labels, values, margins and their tones are the model's
//! (`model::row_label`, `report::display_value`, `theme::{fmt_margin, tone_for_margin}`), every number comes from
//! `pressure-vessel-solver`.
use super::{drawing, VesselAct, VesselSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::theme::{fmt_margin, tone_for_margin, StatusTone};
use crate::toolboxes::pressure_vessel::{
    model::{self, FieldRow},
    report::{display_value, passes},
};
use mechanics_core::lame::lame_constants;
use pressure_vessel_solver::{
    buckling::BucklingApplicability,
    failure::{governing, tresca_stress, von_mises_stress},
    geometry::{classify, GeometryClassification},
    stress::{stress_at_inner_surface_with_thermal, stress_at_outer_surface_with_thermal},
    thickness::ThicknessSolverOutcome,
};

fn act(a: VesselAct) -> UiAction {
    UiAction::Editor(EditorAction::Vessel(a))
}
fn entry(label: &str, shortcut: &str, a: VesselAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn slug(t: &str) -> String {
    t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}
fn line(text: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::text(text, tone)
}
fn value(label: impl Into<String>, text: impl Into<String>) -> DockItem {
    DockItem::field(label, text, Tone::Normal, None, false)
}
fn psi(v: f64) -> String {
    format!("{v:.0} psi")
}
/// The window's tone for a margin's (`theme::tone_for_margin`): a pass reads on, a failure or a thin margin warns.
fn margin_tone(margin: f64) -> Tone {
    match tone_for_margin(margin) {
        StatusTone::Success => Tone::On,
        StatusTone::Warning | StatusTone::Danger => Tone::Warn,
        StatusTone::Neutral | StatusTone::Info => Tone::Normal,
    }
}
fn ok_tone(ok: bool) -> Tone {
    if ok {
        Tone::On
    } else {
        Tone::Warn
    }
}

impl VesselSession {
    /// Every check passes (`report::passes`).
    pub(super) fn all_pass(&self) -> bool {
        let rows = &self.model.rows;
        !rows.is_empty() && rows.iter().all(passes)
    }

    /// One line for the status bar: the verdict and what governs it, or why there is none.
    pub(super) fn verdict(&self) -> String {
        let m = &self.model;
        match (&m.geometry, &m.pressure) {
            (Err(e), _) => format!("Invalid geometry: {e:?}."),
            (_, Err(e)) => format!("Invalid pressure: {e:?}."),
            _ if m.rows.is_empty() => String::new(),
            _ => {
                let passed = m.rows.iter().filter(|r| passes(r)).count();
                let g = governing(&m.rows);
                format!("{}: {passed} / {} checks pass; {} governs, MS {}.", if self.all_pass() { "PASS" } else { "REVIEW" }, m.rows.len(), g.name, fmt_margin(g.margin))
            }
        }
    }

    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = self.input_cards();
        cards.push(self.result_card());
        cards.extend(self.check_cards());
        cards.push(self.drawing_card());
        cards
    }

    /// One card per section of the input rows (`field_rows`), in the form's own order, each row's warning under it.
    fn input_cards(&self) -> Vec<Card> {
        let m = &self.model;
        let mut sections: Vec<(String, Vec<DockItem>)> = Vec::new();
        for row in model::field_rows() {
            if let FieldRow::Header(h) = row {
                sections.push((h.to_string(), Vec::new()));
                continue;
            }
            let editing = self.prompt.as_ref().filter(|p| p.row == row);
            let text = editing.map_or_else(|| display_value(m, row), |p| format!("{}_", p.text));
            // The window draws a Warn value as an edit box: only the row being typed is one.
            let tone = if editing.is_some() { Tone::Warn } else { Tone::Normal };
            let mut items = vec![DockItem::field(model::row_label(row), text, tone, Some(act(VesselAct::Row(row))), editing.is_some())];
            if let FieldRow::Number(t) = row {
                if let Some(hint) = t.validation_hint(m) {
                    items.push(line(format!("{}: {hint}", model::row_label(row)), Tone::Warn));
                }
            }
            if editing.is_some() {
                items.push(line(model::field_hint(row), Tone::Dim));
                items.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
            }
            match sections.last_mut() {
                Some(s) => s.1.extend(items),
                None => sections.push((String::new(), items)),
            }
        }
        sections
            .into_iter()
            .map(|(title, items)| {
                let title = if title.is_empty() { "Inputs".to_string() } else { title };
                Card::new(format!("vessel-{}", slug(&title)), title, Relevance::Primary, items).for_steps(&["Inputs"])
            })
            .collect()
    }

    /// The verdict, what governs it and how the wall classifies: shown at every step.
    fn result_card(&self) -> Card {
        let m = &self.model;
        let items = match (&m.geometry, &m.pressure) {
            (Err(e), _) => vec![line(format!("Invalid geometry: {e:?}."), Tone::Warn), line("Use a positive nominal bore and remaining wall, nonnegative corrosion, and mill undertolerance below 100%.", Tone::Dim)],
            (_, Err(e)) => vec![line(format!("Invalid pressure: {e:?}."), Tone::Warn)],
            (Ok(g), Ok(_)) => {
                let passed = m.rows.iter().filter(|r| passes(r)).count();
                let gov = governing(&m.rows);
                vec![
                    line(format!("{}  {passed} / {} checks pass", if self.all_pass() { "PASS" } else { "REVIEW" }, m.rows.len()), ok_tone(self.all_pass())),
                    value("Governing", gov.name),
                    DockItem::field("Margin of safety", fmt_margin(gov.margin), margin_tone(gov.margin), None, false),
                    value("Classification", if classify(g) == GeometryClassification::ThinWall { "Thin-wall" } else { "Thick-wall" }),
                    value("Inner diameter", format!("{:.4} in", 2.0 * g.inner_radius)),
                    value("Remaining wall", format!("{:.4} in", g.wall_thickness())),
                    value("Material", m.material().name),
                ]
            }
        };
        Card::new("vessel-result", "Result", Relevance::Primary, items)
    }

    /// Every check with its demand, capacity and margin; the minimum wall; the through-wall numbers.
    fn check_cards(&self) -> Vec<Card> {
        let m = &self.model;
        let (Ok(geometry), Ok(pressure)) = (m.geometry, m.pressure) else { return Vec::new() };
        let mut checks: Vec<DockItem> = m.rows.iter().map(|r| DockItem::field(r.name, format!("{} / {}  MS {}", psi(r.applied), psi(r.allowable), fmt_margin(r.margin)), margin_tone(r.margin), None, false)).collect();
        checks.push(line("Applied / allowable stress, margin of safety. Pass at MS of zero or more.", Tone::Dim));
        checks.push(match &m.buckling {
            BucklingApplicability::NotApplicable => line("Buckling: not applicable, no external pressure entered.", Tone::Dim),
            BucklingApplicability::InsufficientData => line("Buckling: insufficient data, enter an unsupported length.", Tone::Dim),
            BucklingApplicability::OutsideValidityRange => line("Buckling: outside the formulas' validity range (OD/t < 40).", Tone::Dim),
            BucklingApplicability::Evaluated(_) => line("Buckling: ring and Windenburg-Trilling short-span critical pressure, the larger governs.", Tone::Dim),
        });
        checks.push(if m.temperature_differential == 0.0 {
            line("Thermal: not applicable, no temperature differential entered.", Tone::Dim)
        } else {
            line(format!("Thermal: {:+.1} \u{b0}F (inner - outer) superposed into the stress checks.", m.temperature_differential), Tone::Normal)
        });
        let thickness = match &m.thickness_outcome {
            Some(ThicknessSolverOutcome::Converged(sol)) => vec![
                value(format!("Minimum remaining wall for MS {:.2}", m.required_ms), format!("{:.4} in", sol.wall_thickness)),
                value("Minimum nominal wall", format!("{:.4} in", m.wall_allowance().unwrap().nominal_thickness(sol.wall_thickness))),
                value("Sizing outer diameter", format!("{:.4} in", 2.0 * sol.outer_radius)),
                line(format!("Entered nominal wall {:.4} in; remaining wall {:.4} in.", m.wall_thickness, geometry.wall_thickness()), ok_tone(geometry.wall_thickness() >= sol.wall_thickness)),
                line("Pressure only, without thermal stress or buckling. Sizing holds the analyzed bore fixed and grows OD.", Tone::Dim),
            ],
            Some(ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found }) => vec![
                line(format!("No wall up to {largest_radius_tried:.1} in reaches MS {:.2} (best {}).", m.required_ms, fmt_margin(*best_margin_found)), Tone::Warn),
            ],
            None => Vec::new(),
        };
        let thermal = m.thermal_loading(m.material());
        let (a, b) = (geometry.inner_radius, geometry.outer_radius);
        let (c1, c2) = lame_constants(a, b, pressure.internal_pressure, pressure.external_pressure);
        let mut wall = vec![
            line("sigma_theta = C1 + C2/r^2, sigma_r = C1 - C2/r^2", Tone::Dim),
            value("C1", psi(c1)),
            value("C2", format!("{c2:.1} psi\u{b7}in\u{b2}")),
        ];
        for (label, s) in [("Inner surface", stress_at_inner_surface_with_thermal(&geometry, &pressure, thermal.as_ref())), ("Outer surface", stress_at_outer_surface_with_thermal(&geometry, &pressure, thermal.as_ref()))] {
            wall.push(line(label, Tone::Title));
            wall.push(value("Radial / hoop / axial", format!("{:.0} / {:.0} / {:.0} psi", s.radial, s.hoop, s.axial)));
            wall.push(value("von Mises / Tresca", format!("{:.0} / {:.0} psi", von_mises_stress(&s), tresca_stress(&s))));
        }
        let mut cards = vec![Card::new("vessel-checks", "Failure checks", Relevance::Primary, checks).for_steps(&["Results", "Report"])];
        if !thickness.is_empty() {
            cards.push(Card::new("vessel-thickness", "Minimum wall thickness", Relevance::Context, thickness).for_steps(&["Results", "Report"]));
        }
        cards.push(Card::new("vessel-wall", "Stress through the wall", Relevance::Context, wall).for_steps(&["Results"]));
        cards
    }

    fn drawing_card(&self) -> Card {
        let items = match (&self.model.geometry, &self.model.pressure) {
            (Ok(g), Ok(_)) => vec![line(
                format!("Remaining wall, to scale, with a quarter cut away, coloured by stress. Nominal OD stays fixed; mill loss and internal corrosion enlarge the bore. The drawn length ({:.2} in, {} outer diameters) is only for the picture. Click the legend's title for the next stress.", drawing::LENGTH_PER_OD * 2.0 * g.outer_radius, drawing::LENGTH_PER_OD),
                Tone::Dim,
            )],
            _ => vec![line("Nothing to draw: the inputs make no vessel.", Tone::Warn)],
        };
        Card::new("vessel-drawing", "Drawing", Relevance::Info, items)
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let m = &self.model;
        let step = |s: &'static str| act(VesselAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'V', format!("Pressure vessel: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, r) in model::field_rows().into_iter().enumerate() {
            if let FieldRow::Header(h) = r {
                out.push(TreeNode::new(10 + k as u32, 2, '#', h).on_click(step("Inputs")));
            }
        }
        out.push(TreeNode::new(3, 1, '+', format!("Results ({})", if self.all_pass() { "pass" } else { "review" })).folder().tone(ok_tone(self.all_pass())).selected(self.step == "Results").on_click(step("Results")));
        for (k, r) in m.rows.iter().enumerate() {
            let ok = passes(r);
            out.push(TreeNode::new(40 + k as u32, 2, if ok { '^' } else { '!' }, r.name).tone(ok_tone(ok)).on_click(step("Results")));
        }
        out.push(TreeNode::new(4, 1, 'R', "Report").selected(self.step == "Report").on_click(step("Report")));
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let never = |what: &str| Err(format!("nothing to {what}"));
        let edit = vec![
            entry("Undo", "Ctrl/Cmd+Z", VesselAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) }),
            entry("Redo", "Ctrl/Cmd+Shift+Z", VesselAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) }),
            entry("Save", "Ctrl/Cmd+S", VesselAct::Save),
        ];
        vec![
            RibbonTab::new("Inputs").group("Vessel", vec![entry("End condition", "", VesselAct::Row(FieldRow::ToggleEndCondition)), entry("Material", "", VesselAct::Row(FieldRow::OpenMaterialPicker))]).group("Edit", edit),
            RibbonTab::new("Results").group("Results", vec![entry("Export report", "", VesselAct::Export)]),
            RibbonTab::new("Report").group("Report", vec![entry("Export report", "", VesselAct::Export), entry("Save", "Ctrl/Cmd+S", VesselAct::Save)]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let pick = |tab: &str, labels: &[&str]| all(tab).into_iter().filter(|e| labels.contains(&e.label.as_str())).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: pick("Report", &["Save", "Export report"]) },
            Menu { title: "Edit".into(), items: pick("Inputs", &["Undo", "Redo"]) },
            Menu { title: "Tools".into(), items: pick("Inputs", &["End condition", "Material"]) },
        ]
    }
}
