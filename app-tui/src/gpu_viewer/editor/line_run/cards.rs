//! What the Line Run window shows: the inspector's cards, the tree, the ribbon and the menus. Plain data built from the
//! model; the window draws it. Every number is the solver's (`model.outcome`), every label the model's
//! (`Field::label`, `Units::text`); margin text and tone are the shared `theme::{fmt_margin, tone_for_margin}`.
use super::{LineRunAct, LineRunSession, Target};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{Bar, DockItem, Mark, Plot, RibbonTab, Series, TreeNode},
};
use crate::theme::{fmt_margin, tone_for_margin, StatusTone};
use crate::toolboxes::line_run::{
    model::{fmt_number, Class, Field, Qty},
    report::location_text,
};
use pressure_vessel_solver::line_run::{CaseResult, LineMargin};

fn act(a: LineRunAct) -> UiAction {
    UiAction::Editor(EditorAction::LineRun(a))
}
fn entry(label: &str, shortcut: &str, a: LineRunAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn line(text: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::text(text, tone)
}
fn click(text: impl Into<String>, tone: Tone, a: LineRunAct) -> DockItem {
    DockItem::row(text, tone, Some(act(a)), false)
}
fn margin_tone(margin: f64) -> Tone {
    match tone_for_margin(margin) {
        StatusTone::Success => Tone::On,
        StatusTone::Warning | StatusTone::Danger => Tone::Warn,
        StatusTone::Neutral | StatusTone::Info => Tone::Normal,
    }
}
fn yes(on: bool) -> &'static str {
    if on {
        "yes"
    } else {
        "no"
    }
}
/// Utilization (share of the allowable used, with the factor of safety) for a margin: `1 / (1 + MS)`.
fn utilization(margin: f64) -> f64 {
    if margin.is_finite() {
        1.0 / (1.0 + margin)
    } else {
        0.0
    }
}

impl LineRunSession {
    /// Every margin of every case is zero or more.
    pub(super) fn all_pass(&self) -> bool {
        self.model.outcome.as_ref().is_ok_and(|r| r.cases.iter().all(|c| c.margins.iter().all(|m| m.margin >= 0.0)))
    }

    /// One line for the status bar: the verdict and what governs it, or why there is none.
    pub(super) fn verdict(&self) -> String {
        match &self.model.outcome {
            Err(why) => why.clone(),
            Ok(r) => {
                let g = r.governing();
                format!("{}: {} governs in {}, MS {}.", if self.all_pass() { "PASS" } else { "REVIEW" }, g.criterion.label(), g.location.case, fmt_margin(g.margin))
            }
        }
    }

    /// A number row: its label, the value in the shown units (or "not set"), a click types it.
    fn number(&self, f: Field) -> DockItem {
        let m = &self.model;
        let editing = matches!(&self.prompt, Some(p) if p.target == Target::Field(f));
        let text = match (&self.prompt, editing) {
            (Some(p), true) => format!("{}_", p.text),
            _ => match m.shown(f) {
                Some(v) => {
                    let unit = m.units().label(f.qty());
                    if unit.is_empty() {
                        fmt_number(v)
                    } else {
                        format!("{} {unit}", fmt_number(v))
                    }
                }
                None => "not set".into(),
            },
        };
        let tone = if editing || m.get(f).is_none() { Tone::Warn } else { Tone::Normal };
        DockItem::field(f.label(), text, tone, Some(act(LineRunAct::Edit(f))), editing)
    }
    fn toggle(&self, label: impl Into<String>, on: bool, a: LineRunAct) -> DockItem {
        DockItem::field(label, yes(on), Tone::Normal, Some(act(a)), false)
    }
    fn typing_help(&self, items: &mut Vec<DockItem>) {
        if self.prompt.is_some() {
            items.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
        }
    }

    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = vec![self.result_card()];
        cards.extend(self.input_cards());
        cards.extend(self.result_cards());
        cards.push(self.drawing_card());
        cards
    }

    fn input_cards(&self) -> Vec<Card> {
        let m = &self.model;
        let i = &m.inputs;
        let step = &["Inputs"];
        let mut tube = vec![self.number(Field::Od), self.number(Field::Wall), self.number(Field::XStart), self.number(Field::XEnd), self.number(Field::Contents), self.number(Field::Insulation)];
        if let Some(t) = m.tube() {
            let u = m.units();
            tube.push(line(format!("Section A {} \u{b7} I {}", fmt_number(t.area() * u.factor(Qty::Length).powi(2)), fmt_number(t.inertia() * u.factor(Qty::Length).powi(4))), Tone::Dim));
        } else {
            tube.push(line("The wall must be positive and below the radius.", Tone::Warn));
        }
        self.typing_help(&mut tube);

        let mut material = vec![DockItem::field("Class", i.class.label(), Tone::Normal, Some(act(LineRunAct::ToggleClass)), false), self.number(Field::E), self.number(Field::Alpha), self.number(Field::Density)];
        match i.class {
            Class::Ductile => material.extend([self.number(Field::Fty), self.number(Field::Ftu), self.number(Field::FsYield), self.number(Field::FsUltimate)]),
            Class::Brittle => material.extend([self.number(Field::Ftu), self.number(Field::Fcu), self.number(Field::FsBrittle)]),
        }
        material.push(line("Allowables and factors of safety are yours to enter (MMPDS or the spec): none is defaulted. Composite and other classes are refused.", Tone::Dim));

        let mut supports = Vec::new();
        for (k, s) in i.supports.iter().enumerate() {
            supports.push(self.number(Field::SupportX(k)));
            supports.push(self.toggle(format!("Support {} axial anchor", k + 1), s.anchor, LineRunAct::Anchor(k)));
            supports.push(self.toggle(format!("Support {} clamped", k + 1), s.clamp, LineRunAct::Clamp(k)));
            supports.push(click(format!("Remove support {}", k + 1), Tone::Dim, LineRunAct::RemoveSupport(k)));
        }
        supports.push(click("Add support", Tone::Title, LineRunAct::AddSupport));
        supports.push(line("Pinned in both transverse directions; clamped also holds the slope. An anchor holds the line axially.", Tone::Dim));

        let mut cards = vec![
            Card::new("line-tube", "Tube", Relevance::Primary, tube).for_steps(step),
            Card::new("line-material", "Material", Relevance::Primary, material).for_steps(step),
            Card::new("line-supports", "Supports", Relevance::Primary, supports).for_steps(step),
        ];
        for (k, l) in i.loads.iter().enumerate() {
            let mut items = vec![self.number(Field::LoadX(k)), self.number(Field::LoadWeight(k))];
            items.extend((0..3).map(|a| self.number(Field::LoadForce(k, a))));
            items.extend((0..3).map(|a| self.number(Field::LoadOffset(k, a))));
            items.push(self.toggle(format!("Load {} includes pressure thrust", k + 1), l.thrust, LineRunAct::Thrust(k)));
            items.push(DockItem::field(format!("Load {} acts in", k + 1), m.scope_label(l), Tone::Normal, Some(act(LineRunAct::Scope(k))), false));
            items.push(click(format!("Remove load {}", k + 1), Tone::Dim, LineRunAct::RemoveLoad(k)));
            cards.push(Card::new(format!("line-load-{k}"), format!("Point load {} \u{b7} x {}", k + 1, m.units().text(Qty::Length, l.x)), Relevance::Primary, items).for_steps(step));
        }
        let mut add = vec![click("Add point load", Tone::Title, LineRunAct::AddLoad), line("A valve, actuator or a bend's flow thrust. Weight is scaled by the case's load factors; force is applied as given.", Tone::Dim)];
        self.typing_help(&mut add);
        cards.push(Card::new("line-loads", "Point loads", Relevance::Primary, add).for_steps(step));

        let mut cases: Vec<DockItem> = i.cases.iter().enumerate().map(|(k, c)| click(format!("{} {}", if k == i.selected { "\u{25cf}" } else { "\u{25cb}" }, c.name), if k == i.selected { Tone::On } else { Tone::Normal }, LineRunAct::SelectCase(k))).collect();
        cases.push(click("Add load case", Tone::Title, LineRunAct::AddCase));
        let c = m.selected_case();
        let name_editing = matches!(&self.prompt, Some(p) if p.target == Target::CaseName);
        cases.push(DockItem::field("Name", if name_editing { format!("{}_", self.prompt.as_ref().map_or("", |p| p.text.as_str())) } else { c.name.clone() }, if name_editing { Tone::Warn } else { Tone::Normal }, Some(act(LineRunAct::RenameCase)), name_editing));
        cases.extend([self.number(Field::CaseInternal), self.number(Field::CaseExternal), self.number(Field::CaseDeltaT)]);
        cases.extend((0..3).map(|a| self.number(Field::CaseFactor(a))));
        cases.push(self.toggle("Wall carries the pressure end load", c.end_load, LineRunAct::EndLoad));
        cases.push(click(format!("Remove case {}", c.name), Tone::Dim, LineRunAct::RemoveCase));
        cases.push(line("Load factors n multiply weight: one g with z up is (0, 0, -1). Never combine pressure end load with a thrust load that already holds it.", Tone::Dim));
        cards.push(Card::new("line-cases", "Load cases", Relevance::Primary, cases).for_steps(step));
        cards
    }

    /// The verdict (or why there is no result), the equilibrium residual and what governs: shown at every step.
    fn result_card(&self) -> Card {
        let items = match &self.model.outcome {
            Err(why) => vec![line(if why.is_empty() { "No result yet.".to_string() } else { why.clone() }, Tone::Warn)],
            Ok(r) => {
                let g = r.governing();
                let pass = self.all_pass();
                let residual = r.cases.iter().map(|c| c.residual).fold(0.0, f64::max);
                vec![
                    DockItem::Verdict {
                        headline: if pass { "PASS".into() } else { "REVIEW".into() },
                        detail: format!("{} \u{b7} {} \u{b7} MS {}", g.criterion.label(), g.location.case, fmt_margin(g.margin)),
                        tone: if pass { Tone::On } else { Tone::Warn },
                        actions: vec![entry("Export", "", LineRunAct::Export)],
                    },
                    line(location_text(&self.model, &g.location), Tone::Dim),
                    DockItem::field("Equilibrium residual", format!("{residual:.1e}"), Tone::Normal, None, false),
                ]
            }
        };
        Card::new("line-result", "Result", Relevance::Primary, items)
    }

    fn margin_stats(&self, case: &CaseResult) -> Vec<DockItem> {
        let gov = case.governing().criterion;
        case.margins
            .iter()
            .map(|mg: &LineMargin| {
                let u = utilization(mg.margin);
                DockItem::Stat {
                    label: mg.criterion.label().into(),
                    value: format!("MS {}", fmt_margin(mg.margin)),
                    note: format!("{}{:.0}% used", if mg.criterion == gov { "governing \u{b7} " } else { "" }, 100.0 * u),
                    note_tone: margin_tone(mg.margin),
                    detail: format!("{} of {} \u{b7} {}", self.model.units().text(Qty::Stress, mg.applied), self.model.units().text(Qty::Stress, mg.allowable), location_text(&self.model, &mg.location)),
                    action: None,
                }
            })
            .collect()
    }

    fn result_cards(&self) -> Vec<Card> {
        let Ok(result) = &self.model.outcome else { return Vec::new() };
        let Some(case) = result.cases.get(self.model.inputs.selected) else { return Vec::new() };
        let u = self.model.units();
        let steps = &["Results", "Report"];
        let bars = case.margins.iter().map(|mg| Bar { label: mg.criterion.label().into(), value: if mg.margin.is_finite() { mg.margin } else { 1.0 }, text: fmt_margin(mg.margin), tone: margin_tone(mg.margin), tag: if mg.criterion == case.governing().criterion { "governing".into() } else { String::new() }, action: None }).collect();
        let mut stats = self.margin_stats(case);
        stats.push(DockItem::Bars { rows: bars, lo: -0.5, hi: 1.0 });
        stats.push(line("Margin of safety = 1 / (FS \u{b7} demand / allowable) - 1, at the worst station, surface and angle. Pass at zero or more.", Tone::Dim));

        let (lf, mf, ff) = (u.factor(Qty::Length), u.factor(Qty::Moment), u.factor(Qty::Force));
        let mut marks: Vec<Mark> = self.model.inputs.supports.iter().enumerate().map(|(k, s)| Mark { at: [s.x * lf, 0.0], text: format!("S{}{}", k + 1, if s.anchor { " anchor" } else { "" }), color: 2 }).collect();
        let g = case.governing();
        let at_gov = case.stations.iter().min_by(|a, b| (a.x - g.location.x).abs().total_cmp(&(b.x - g.location.x).abs()));
        if let Some(s) = at_gov {
            marks.push(Mark { at: [s.x * lf, s.resultant_moment() * mf], text: "governing".into(), color: 1 });
        }
        let moment = Plot {
            x_label: format!("x ({})", u.label(Qty::Length)),
            y_label: format!("Bending moment ({})", u.label(Qty::Moment)),
            series: vec![Series { label: "Moment resultant".into(), points: case.stations.iter().map(|s| [s.x * lf, s.resultant_moment() * mf]).collect(), color: 0, dashed: false }],
            bands: Vec::new(),
            marks,
        };
        let axial = Plot {
            x_label: format!("x ({})", u.label(Qty::Length)),
            y_label: format!("Axial force ({})", u.label(Qty::Force)),
            series: vec![Series { label: "Axial force (tension +)".into(), points: case.stations.iter().map(|s| [s.x * lf, s.axial_force * ff]).collect(), color: 1, dashed: true }],
            bands: Vec::new(),
            marks: Vec::new(),
        };
        let mut reactions = Vec::new();
        for r in &case.reactions {
            let f = r.force.map(|v| fmt_number(v * ff)).join(", ");
            reactions.push(DockItem::field(format!("x = {}", u.text(Qty::Length, r.x)), format!("F ({f}) {}", u.label(Qty::Force)), Tone::Normal, None, false));
            if r.moment.iter().any(|v| v.abs() > 0.0) {
                reactions.push(line(format!("    M ({}) {}", r.moment.map(|v| fmt_number(v * mf)).join(", "), u.label(Qty::Moment)), Tone::Dim));
            }
        }
        vec![
            Card::new("line-margins", format!("Margins \u{b7} {}", case.name), Relevance::Primary, stats).for_steps(steps),
            Card::new("line-stations", "Stations", Relevance::Primary, vec![DockItem::Plot(moment), DockItem::Plot(axial)]).for_steps(steps),
            Card::new("line-reactions", "Reactions", Relevance::Context, reactions).for_steps(steps).tag("on the line"),
        ]
    }

    fn drawing_card(&self) -> Card {
        let items = match self.model.tube() {
            Some(t) if self.model.outcome.is_ok() => {
                let k = super::drawing::enlargement(t.outer_diameter, self.model.inputs.x_end - self.model.inputs.x_start);
                vec![line(
                    format!("The line, coloured by bending moment (click the legend's title for axial force), a collar at each support. The tube is drawn {k:.1}\u{d7} its radius so it reads; lengths are to scale."),
                    Tone::Dim,
                )]
            }
            _ => vec![line("Nothing to draw until the inputs give a result.", Tone::Warn)],
        };
        Card::new("line-drawing", "Drawing", Relevance::Info, items)
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let i = &self.model.inputs;
        let step = |s: &'static str| act(LineRunAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'L', format!("Line run: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (id, label) in [(10, "Tube".to_string()), (11, format!("Material ({})", i.class.label())), (12, format!("Supports ({})", i.supports.len())), (13, format!("Point loads ({})", i.loads.len())), (14, format!("Load cases ({})", i.cases.len()))] {
            out.push(TreeNode::new(id, 2, '#', label).on_click(step("Inputs")));
        }
        let (pass, review) = (self.all_pass(), self.model.outcome.is_ok());
        let results = if !review { "Results (none)".to_string() } else { format!("Results ({})", if pass { "pass" } else { "review" }) };
        out.push(TreeNode::new(3, 1, '+', results).folder().tone(if pass { Tone::On } else { Tone::Warn }).selected(self.step == "Results").on_click(step("Results")));
        if let Ok(r) = &self.model.outcome {
            for (k, c) in r.cases.iter().enumerate() {
                let g = c.governing();
                out.push(
                    TreeNode::new(40 + k as u32, 2, if g.margin >= 0.0 { '^' } else { '!' }, format!("{} \u{b7} MS {}", c.name, fmt_margin(g.margin)))
                        .tone(margin_tone(g.margin))
                        .selected(k == i.selected)
                        .on_click(act(LineRunAct::SelectCase(k))),
                );
            }
        }
        out.push(TreeNode::new(4, 1, 'R', "Report").selected(self.step == "Report").on_click(step("Report")));
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let never = |what: &str| Err(format!("nothing to {what}"));
        let edit = vec![
            entry("Undo", "Ctrl/Cmd+Z", LineRunAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) }),
            entry("Redo", "Ctrl/Cmd+Shift+Z", LineRunAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) }),
            entry("Save", "Ctrl/Cmd+S", LineRunAct::Save),
        ];
        let export = entry("Export", "", LineRunAct::Export).when(if self.model.outcome.is_ok() { Ok(()) } else { Err("there is no result to export".into()) });
        vec![
            RibbonTab::new("Inputs")
                .group("Line", vec![entry("Add support", "", LineRunAct::AddSupport), entry("Add load", "", LineRunAct::AddLoad), entry("Add case", "", LineRunAct::AddCase)])
                .group("Material", vec![entry("Material class", "", LineRunAct::ToggleClass)])
                .group("View", vec![entry(&format!("Units: {}", self.model.units().other().name()), "", LineRunAct::ToggleUnits)])
                .group("Edit", edit),
            RibbonTab::new("Results").group("Results", vec![export.clone()]),
            RibbonTab::new("Report").group("Report", vec![export, entry("Save", "Ctrl/Cmd+S", LineRunAct::Save)]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let items = |tab: &str, keep: &dyn Fn(&str) -> bool| all(tab).into_iter().filter(|e| keep(&e.label)).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: [items("Report", &|l| l == "Save" || l == "Export")].concat() },
            Menu { title: "Edit".into(), items: items("Inputs", &|l| l == "Undo" || l == "Redo") },
            Menu { title: "Tools".into(), items: items("Inputs", &|l| l.starts_with("Add") || l.starts_with("Units") || l == "Material class") },
        ]
    }
}
