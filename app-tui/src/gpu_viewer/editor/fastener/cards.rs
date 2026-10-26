//! What the Fastener Hole workbench shows: the inspector's cards, the tree, the ribbon and the menus. Plain data built from
//! the model; the window draws it. Row labels and values are the terminal form's (`view::row_label` / `display_value`),
//! every calculated value is shown as `nominal -minus/+plus` (`model::format_*_band`) with where it came from.
use super::{drawing, FastenerAct, FastenerSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::toolboxes::fastener_hole::{
    domain::{CountersinkSolveFor, FitClassification, SecondaryCountersinkMethod, TolerancedValue},
    model::{self, FieldRow, HoleType},
    view,
};

fn act(a: FastenerAct) -> UiAction {
    UiAction::Editor(EditorAction::Fastener(a))
}
fn entry(label: &str, shortcut: &str, a: FastenerAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn slug(t: &str) -> String {
    t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}
fn line(text: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::text(text, tone)
}
/// A calculated value: its label says where it came from (calculated, derived, transferred, preserved).
fn value(label: &str, origin: &str, text: String) -> DockItem {
    DockItem::field(format!("{label} ({origin})"), text, Tone::Normal, None, false)
}
fn linear(v: &TolerancedValue) -> String {
    format!("{} in", model::format_linear_band(v))
}
fn angle(v: &TolerancedValue) -> String {
    format!("{} deg", model::format_angle_band(v))
}
fn area(v: &TolerancedValue) -> String {
    format!("{} in^2", model::format_area_band(v))
}
fn ok_tone(ok: bool) -> Tone {
    if ok {
        Tone::On
    } else {
        Tone::Warn
    }
}

impl FastenerSession {
    /// Every result the inputs give is valid (nothing reads INVALID).
    pub(super) fn valid(&self) -> bool {
        let m = &self.st.model;
        match m.hole_type {
            HoleType::Regular => m.regular_secondary.is_ok(),
            HoleType::Countersunk => m.countersink_primary.is_ok() && m.countersink_secondary.is_ok(),
        }
    }

    /// One line for the status bar: the answer to the question the hole type asks.
    pub(super) fn verdict(&self) -> String {
        let m = &self.st.model;
        match m.hole_type {
            HoleType::Regular => {
                let fit = &m.regular_fit;
                format!("{}: fit {} in.", classification(fit.classification).0, model::format_band(fit.nominal, fit.min, fit.max, model::format_linear))
            }
            HoleType::Countersunk => match &m.countersink_primary {
                Ok(g) => {
                    let (label, v) = solved(g.solve_for, g);
                    format!("Countersink {}: {}.", label.to_lowercase(), if g.solve_for == CountersinkSolveFor::Angle { angle(&v) } else { linear(&v) })
                }
                Err(e) => format!("The countersink does not solve: {e}."),
            },
        }
    }

    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = self.input_cards();
        cards.extend(match self.st.model.hole_type {
            HoleType::Regular => self.regular_cards(),
            HoleType::Countersunk => self.countersink_cards(),
        });
        cards.push(self.drawing_card());
        cards
    }

    /// One card per section of the terminal form (`field_rows`), in the form's own order.
    fn input_cards(&self) -> Vec<Card> {
        let m = &self.st.model;
        let mut sections: Vec<(String, Vec<DockItem>)> = Vec::new();
        for row in self.rows() {
            if let FieldRow::Header(h) = row {
                sections.push((h.to_string(), Vec::new()));
                continue;
            }
            let editing = self.prompt.as_ref().filter(|p| p.row == row);
            let text = editing.map_or_else(|| view::display_value(m, row), |p| format!("{}_", p.text));
            // The window draws a Warn value as an edit box: only the row being typed is one.
            let tone = if editing.is_some() { Tone::Warn } else { Tone::Normal };
            let item = DockItem::field(view::row_label(row), text, tone, Some(act(FastenerAct::Row(row))), editing.is_some());
            match sections.last_mut() {
                Some(s) => s.1.push(item),
                None => sections.push((String::new(), vec![item])),
            }
            if editing.is_some() {
                let s = sections.last_mut().expect("just pushed");
                s.1.push(line(model::field_hint(row), Tone::Dim));
                s.1.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
            }
        }
        sections
            .into_iter()
            .map(|(title, items)| {
                let title = if title.is_empty() { "Inputs".to_string() } else { title };
                Card::new(format!("fastener-{}", slug(&title)), title, Relevance::Primary, items).for_steps(&["Inputs"])
            })
            .collect()
    }

    fn regular_cards(&self) -> Vec<Card> {
        let m = &self.st.model;
        let fit = &m.regular_fit;
        let (class, ok) = classification(fit.classification);
        let fit_items = vec![
            DockItem::field("Fit", format!("{} in", model::format_band(fit.nominal, fit.min, fit.max, model::format_linear)), Tone::Normal, None, false),
            line(class, ok_tone(ok)),
            line("Fit = Hole 2 - Hole 1: positive is clearance, negative interference, over every tolerance corner.", Tone::Dim),
        ];
        let companion = match &m.regular_secondary {
            Ok((hole, check)) => vec![
                value("Diameter", "derived", linear(hole)),
                line(if check.preserved { "Fit envelope preserved" } else { "Fit envelope NOT preserved" }, ok_tone(check.preserved)),
                line("The hole that gives the same fit envelope against the secondary reference hole.", Tone::Dim),
            ],
            Err(e) => vec![line(format!("Companion hole: INVALID, {e}."), Tone::Warn)],
        };
        vec![Card::new("fastener-fit", "Fit analysis", Relevance::Primary, fit_items), Card::new("fastener-companion", "Secondary companion hole", Relevance::Context, companion)]
    }

    fn countersink_cards(&self) -> Vec<Card> {
        let m = &self.st.model;
        let mut primary = Vec::new();
        match &m.countersink_primary {
            Ok(g) => {
                let (label, v) = solved(g.solve_for, g);
                primary.push(value(label, "calculated", if g.solve_for == CountersinkSolveFor::Angle { angle(&v) } else { linear(&v) }));
            }
            Err(e) => primary.push(line(format!("Geometry: INVALID, {e}."), Tone::Warn)),
        }
        match &m.countersink_primary_area {
            Ok(a) => primary.push(value("Lateral surface area", "calculated", area(a))),
            Err(_) => primary.push(line("Lateral surface area: incomplete", Tone::Dim)),
        }
        let method = m.countersink.secondary_method;
        let mut secondary = Vec::new();
        match &m.countersink_secondary {
            Ok(g) => {
                let depth_origin = if method == SecondaryCountersinkMethod::PreserveDepth { "transferred" } else { "calculated" };
                secondary.push(value("Outer diameter", "calculated", linear(&g.outer_diameter)));
                secondary.push(value("Depth", depth_origin, linear(&g.depth)));
                secondary.push(value("Angle", "transferred", angle(&g.angle_deg)));
                match &m.countersink_secondary_area {
                    Ok(a) => secondary.push(value("Lateral surface area", if method == SecondaryCountersinkMethod::PreserveDepth { "calculated" } else { "preserved" }, area(a))),
                    Err(_) => secondary.push(line("Lateral surface area: incomplete", Tone::Dim)),
                }
            }
            Err(e) => secondary.push(line(format!("Geometry: INVALID, {e}."), Tone::Warn)),
        }
        if let Some(check) = &m.countersink_area_check {
            secondary.push(DockItem::field("Area, primary / secondary", format!("{} / {} in^2", model::format_area(check.primary_area), model::format_area(check.secondary_area)), Tone::Normal, None, false));
            secondary.push(DockItem::field("Area difference", format!("{} in^2", model::format_area(check.delta)), Tone::Normal, None, false));
            secondary.push(line(if check.preserved { "Area preserved" } else { "Area NOT preserved" }, ok_tone(check.preserved)));
        }
        vec![
            Card::new("fastener-primary", "Primary countersink", Relevance::Primary, primary),
            Card::new("fastener-secondary", format!("Secondary countersink ({})", method.label()), Relevance::Context, secondary),
        ]
    }

    fn drawing_card(&self) -> Card {
        let items = match drawing::layout(&self.st.model) {
            Ok(l) => vec![
                line(format!("To scale at the nominal dimensions, a quarter cut away. The plate ({:.3} in across, {:.3} in thick) is only for the picture.", 2.0 * l.outer_radius, l.thickness), Tone::Dim),
            ],
            Err(why) => vec![line(format!("Nothing to draw: {why}."), Tone::Warn)],
        };
        Card::new("fastener-drawing", "Drawing", Relevance::Info, items).for_steps(&["Inputs"])
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let m = &self.st.model;
        let step = |s: &'static str| act(FastenerAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'H', format!("Fastener hole: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', format!("Inputs ({})", m.hole_type.label())).folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, r) in self.rows().into_iter().enumerate() {
            if let FieldRow::Header(h) = r {
                out.push(TreeNode::new(10 + k as u32, 2, '#', h).on_click(step("Inputs")));
            }
        }
        out.push(TreeNode::new(3, 1, '+', "Results").folder().on_click(step("Inputs")));
        let mut result = |id: u32, label: String, ok: bool| out.push(TreeNode::new(id, 2, if ok { '^' } else { '!' }, label).tone(ok_tone(ok)).on_click(step("Inputs")));
        match m.hole_type {
            HoleType::Regular => {
                let (class, ok) = classification(m.regular_fit.classification);
                result(4, format!("Fit: {class}"), ok);
                result(5, "Companion hole".into(), m.regular_secondary.as_ref().is_ok_and(|(_, c)| c.preserved));
            }
            HoleType::Countersunk => {
                result(4, "Primary countersink".into(), m.countersink_primary.is_ok());
                result(5, "Secondary countersink".into(), m.countersink_secondary.is_ok() && m.countersink_area_check.is_none_or(|c| c.preserved));
            }
        }
        out.push(TreeNode::new(6, 1, 'R', "Report").selected(self.step == "Report").on_click(step("Report")));
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let row = |label: &str, r: FieldRow| entry(label, "", FastenerAct::Row(r));
        let mut hole = vec![row("Hole type", FieldRow::ToggleHoleType), row("Tolerance input", FieldRow::ToggleToleranceMode)];
        if self.st.model.hole_type == HoleType::Countersunk {
            hole.push(row("Solve for", FieldRow::ToggleSolveFor));
            hole.push(row("Secondary method", FieldRow::ToggleSecondaryMethod));
        }
        let never = |what: &str| Err(format!("nothing to {what}"));
        vec![
            RibbonTab::new("Inputs").group("Hole", hole).group(
                "Edit",
                vec![
                    entry("Undo", "Ctrl/Cmd+Z", FastenerAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) }),
                    entry("Redo", "Ctrl/Cmd+Shift+Z", FastenerAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) }),
                    entry("Save", "Ctrl/Cmd+S", FastenerAct::Save),
                ],
            ),
            RibbonTab::new("Report").group("Report", vec![entry("Export report", "", FastenerAct::Export), entry("Save", "Ctrl/Cmd+S", FastenerAct::Save)]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let pick = |tab: &str, labels: &[&str]| all(tab).into_iter().filter(|e| labels.contains(&e.label.as_str())).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: pick("Report", &["Save", "Export report"]) },
            Menu { title: "Edit".into(), items: pick("Inputs", &["Undo", "Redo"]) },
            Menu { title: "Tools".into(), items: pick("Inputs", &["Hole type", "Tolerance input", "Solve for", "Secondary method"]) },
        ]
    }
}

/// The fit's name and whether it reads as a pass (clearance and interference are what was designed; a transition fit
/// can be either, so it is flagged).
fn classification(c: FitClassification) -> (&'static str, bool) {
    match c {
        FitClassification::Clearance => ("CLEARANCE", true),
        FitClassification::Transition => ("TRANSITION", false),
        FitClassification::Interference => ("INTERFERENCE", true),
    }
}

/// The dimension the countersink was solved for, with its value.
fn solved(s: CountersinkSolveFor, g: &crate::toolboxes::fastener_hole::domain::CountersinkGeometry) -> (&'static str, TolerancedValue) {
    match s {
        CountersinkSolveFor::OuterDiameter => ("Outer diameter", g.outer_diameter),
        CountersinkSolveFor::HoleDiameter => ("Hole diameter", g.hole_diameter),
        CountersinkSolveFor::Depth => ("Depth", g.depth),
        CountersinkSolveFor::Angle => ("Angle", g.angle_deg),
    }
}
