//! What the Bushing workbench shows: the inspector's cards for each step, the tree, the ribbon and the menus. Plain data
//! built from the model; the window draws it. The numbers and wording come from the workbench's own model, checks and
//! edge-check report; only the layout is new.
use super::{fe, BushingAct, BushingSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Authoring, Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::toolboxes::bushing::{
    advice::{self, CheckKind, Severity},
    edge_check::{self, TableRow},
    model::{FieldRow, NumberTarget},
    view,
};
use bushing_solver::tolerance::ToleranceStatus;

fn act(a: BushingAct) -> UiAction {
    UiAction::Editor(EditorAction::Bushing(a))
}
fn entry(label: &str, shortcut: &str, a: BushingAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
/// Text of the model, the advice and the edge check, for a window font that lacks arrows and a few symbols.
fn plain(s: &str) -> String {
    s.replace('\u{2192}', "->").replace('\u{2265}', ">=").replace('\u{2264}', "<=").replace('\u{2248}', "~").replace('\u{26a0}', "!").replace('\u{2717}', "x").replace('\u{2713}', "ok").replace('\u{25b8}', ">")
}
fn slug(t: &str) -> String {
    t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}
fn margin(m: f64) -> String {
    if m.is_finite() {
        format!("{m:+.2}")
    } else {
        "n/a".into()
    }
}
fn margin_tone(m: f64) -> Tone {
    if !m.is_finite() || m >= advice::WARN_MARGIN {
        Tone::On
    } else {
        Tone::Warn
    }
}
fn line(text: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::text(text, tone)
}
fn field(label: &str, value: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::field(label, value, tone, None, false)
}

impl BushingSession {
    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = self.input_cards();
        cards.push(self.verdict_card());
        if let Some(c) = self.fixes_card() {
            cards.push(c);
        }
        cards.push(self.edge_card());
        cards.push(self.numbers_card());
        cards.push(self.fe_card());
        cards
    }

    /// One card per section of the input form (`field_rows`), in the form's own order.
    fn input_cards(&self) -> Vec<Card> {
        let m = &self.st.model;
        let related: Vec<NumberTarget> = m.checks.iter().filter(|c| c.severity == Severity::Fail).flat_map(|c| c.kind.related_inputs().iter().copied()).collect();
        let mut sections: Vec<(String, Vec<DockItem>)> = Vec::new();
        for row in self.rows() {
            if let FieldRow::Header(h) = row {
                sections.push((h.to_string(), Vec::new()));
                continue;
            }
            let item = match row {
                FieldRow::AdvancedSection => DockItem::field("Advanced settings", if m.advanced_open { "shown" } else { "hidden" }, Tone::Dim, Some(act(BushingAct::Row(row))), false),
                _ => {
                    let editing = self.prompt.as_ref().filter(|p| p.row == row);
                    let value = editing.map_or_else(|| view::display_value(m, row), |p| format!("{}_", p.text));
                    let flagged = matches!(row, FieldRow::Number(t) if related.contains(&t));
                    let tone = if editing.is_some() { Tone::Warn } else { Tone::Normal };
                    // The window draws an edit box on a Warn value, so a flagged input is marked in its label instead.
                    let label = view::row_text(m.tolerance_mode, row);
                    let (label, tone) = if flagged && editing.is_none() { (format!("! {label}"), Tone::Normal) } else { (label.to_string(), if editing.is_some() { tone } else { Tone::Normal }) };
                    DockItem::field(label, value, tone, Some(act(BushingAct::Row(row))), editing.is_some())
                }
            };
            match sections.last_mut() {
                Some(s) => s.1.push(item),
                None => sections.push((String::new(), vec![item])),
            }
        }
        let editing_hint = self.prompt.is_some();
        sections
            .into_iter()
            .enumerate()
            .map(|(k, (title, mut items))| {
                if editing_hint && items.iter().any(|i| matches!(i, DockItem::Field { focused: true, .. })) {
                    items.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
                }
                let title = if title.is_empty() { "Inputs".to_string() } else { title };
                Card::new(format!("bushing-{}", slug(&title)), title, if k < 2 { Relevance::Primary } else { Relevance::Context }, items).for_steps(&["Inputs"])
            })
            .collect()
    }

    /// Pass / fail at a glance and every check that needs attention; shown while editing the inputs and in the Checks step.
    fn verdict_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let failing = self.failing();
        let mut items = vec![DockItem::row(
            format!("{}  governing: {} ({})", if failing == 0 { "PASS" } else { "REVIEW" }, out.governing.name, margin(out.governing.margin)),
            if failing == 0 { Tone::On } else { Tone::Warn },
            None,
            false,
        )];
        if failing > 0 {
            items.push(line(format!("{failing} check(s) failing"), Tone::Warn));
        }
        for c in m.checks.iter().filter(|c| c.severity != Severity::Pass) {
            let has_fix = m.recommendations.iter().any(|r| r.fixes == c.kind);
            let glyph = if c.severity == Severity::Fail { "x" } else { "!" };
            items.push(DockItem::row(plain(&format!("{glyph} {}: {}{}", c.kind.label(), c.detail, if has_fix { "   fixes" } else { "" })), Tone::Warn, has_fix.then(|| act(BushingAct::ShowFixes(c.kind))), false));
        }
        let advisories = edge_check::advisories(self.st.edge_check.as_ref(), !self.edge_current());
        for a in advisories.iter().take(1) {
            items.push(line(plain(&format!("! {}", a.text)), Tone::Warn));
        }
        if failing == 0 {
            items.push(line("Every check passes.", Tone::Dim));
        }
        if !m.recommendations.is_empty() {
            items.push(DockItem::row(format!("Fixes ({})", m.recommendations.len()), Tone::Title, Some(act(BushingAct::AllFixes)), false));
        }
        if let Some(a) = &self.st.last_applied {
            items.push(line(plain(&format!("ok Applied: {a}")), Tone::On));
        }
        Card::new("bushing-verdict", "Verdict", Relevance::Primary, items).for_steps(&["Inputs", "Checks"])
    }

    /// The verified fixes: pick one to see what it does to everything else, then apply it.
    fn fixes_card(&self) -> Option<Card> {
        let m = &self.st.model;
        let recs: Vec<(usize, &advice::Recommendation)> = m.recommendations.iter().enumerate().filter(|(_, r)| self.fix_kind.is_none_or(|k| r.fixes == k)).collect();
        if recs.is_empty() {
            return None;
        }
        let selected = self.st.rec_selected.min(m.recommendations.len() - 1);
        let mut items = Vec::new();
        for (i, r) in recs {
            let on = i == selected;
            items.push(DockItem::row(plain(&format!("{} {}", if on { ">" } else { " " }, r.summary)), if on { Tone::Title } else { Tone::Normal }, Some(act(BushingAct::SelectFix(i))), on));
            if on {
                items.push(line(plain(&format!("fixes {}: {}", r.fixes.label(), r.outcome)), Tone::Dim));
                for i in &r.impact {
                    items.push(line(plain(&format!("  {i}")), Tone::Dim));
                }
                if let Some(n) = &r.note {
                    items.push(line(plain(&format!("  {n}")), Tone::Warn));
                }
                items.push(if r.is_applicable() { DockItem::row("Apply this fix", Tone::Title, Some(act(BushingAct::ApplyFix(i))), false) } else { line("Needs a manual change.", Tone::Warn) });
            }
        }
        if self.fix_kind.is_some() {
            items.push(DockItem::row("Show every fix", Tone::Dim, Some(act(BushingAct::AllFixes)), false));
        }
        Some(Card::new("bushing-fixes", "Fixes", Relevance::Primary, items).for_steps(&["Checks"]))
    }

    /// The independent edge-distance cross-check: the minimum e/D each model asks for, against what the housing has.
    fn edge_card(&self) -> Card {
        let m = &self.st.model;
        let mut items = Vec::new();
        if let Some(j) = &self.st.edge_job {
            items.push(line(format!("running: {} ({:.0} s)", if j.deep { "quick set + contact FE" } else { "quick set" }, j.started.elapsed().as_secs_f32()), Tone::Warn));
        }
        match &self.st.edge_check {
            None => items.push(line("Not run. The quick check compares the stress models and tabulated allowables; add the contact FE for the bushing and housing together.", Tone::Dim)),
            Some(run) => {
                let r = &run.report;
                if !self.edge_current() {
                    items.push(line("Inputs changed since this run; it re-runs when they settle.", Tone::Warn));
                }
                items.push(line(format!("e/D {:.2}  load {:.0} lbf  bearing limit {:.0} lbf  fit {:.0} psi", r.edge / r.bore_diameter, r.applied_load, r.bearing_limit_load, r.fit_pressure), Tone::Dim));
                items.push(line("minimum e/D to carry it:  strength | bearing | 1st yield | P(fail)", Tone::Dim));
                for TableRow { name, cells, p_fail, error, .. } in edge_check::table_rows(run, m) {
                    match error {
                        Some(e) => items.push(line(format!("{name}: not evaluated ({e})"), Tone::Warn)),
                        None => {
                            let short = cells.iter().any(|c| c.1);
                            items.push(field(&name, format!("{} | {} | {} | {p_fail}", cells[0].0, cells[1].0, cells[2].0), if short { Tone::Warn } else { Tone::Normal }));
                        }
                    }
                }
                for l in edge_check::completion_lines(run).into_iter().skip(1).filter(|l| l.starts_with("Recommended")) {
                    items.push(line(plain(&l), Tone::Dim));
                }
            }
        }
        items.push(DockItem::row("Run quick check", Tone::Title, Some(act(BushingAct::EdgeCheck(false))), false));
        items.push(DockItem::row("Run with contact FE (about 3 s)", Tone::Title, Some(act(BushingAct::EdgeCheck(true))), false));
        Card::new("bushing-edge-check", "Edge-distance cross-check", Relevance::Context, items).for_steps(&["Checks"])
    }

    /// The solver's numbers behind the verdict.
    fn numbers_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let tol = match out.tolerance_status {
            ToleranceStatus::Ok => "OK",
            ToleranceStatus::Clamped => "clamped",
            ToleranceStatus::Infeasible => "INFEASIBLE",
        };
        let sev = |k: CheckKind| advice::severity_of(&m.checks, k);
        let tone = |k: CheckKind| if sev(k) == Severity::Pass { Tone::Normal } else { Tone::Warn };
        let mut items = vec![
            field("OD installed", format!("{:.4} in", out.od_installed), Tone::Normal),
            field("Interference", format!("{:.4} in (target {:.4})", out.delta_total, m.interference), Tone::Normal),
            field("Tolerance", tol, if sev(CheckKind::Tolerance) == Severity::Pass { Tone::Normal } else { Tone::Warn }),
            field("Straight wall", format!("{:.4} in", out.wall_straight), tone(CheckKind::StraightWall)),
            field("Contact pressure", format!("{:.0} psi ({:.0} to {:.0})", out.pressure, out.pressure_range.min, out.pressure_range.max), Tone::Normal),
            field("Housing hoop stress", format!("{:.0} psi, MS {}", out.stress_hoop_housing, margin(out.housing_ms)), if sev(CheckKind::HousingStress) == Severity::Pass { margin_tone(out.housing_ms) } else { Tone::Warn }),
            field("Bushing hoop stress", format!("{:.0} psi, MS {}", out.stress_hoop_bushing, margin(out.bushing_ms)), if sev(CheckKind::BushingStress) == Severity::Pass { margin_tone(out.bushing_ms) } else { Tone::Warn }),
            field("Install force", format!("{:.1} lbf ({:.1} to {:.1})", out.install_force, out.install_force_range.min, out.install_force_range.max), Tone::Normal),
            field("Retained force", format!("{:.1} lbf", out.retained_install_force), Tone::Normal),
            field("Edge e/D", format!("{:.3}", out.ed_actual), tone(CheckKind::EdgeSequencing)),
            field("Needs, sequencing", format!("{:.3}", out.ed_min_sequence), tone(CheckKind::EdgeSequencing)),
            field("Needs, strength", format!("{:.3}", out.ed_min_strength), tone(CheckKind::EdgeStrength)),
        ];
        if m.delta_t != 0.0 {
            items.push(field("Service \u{394}T", format!("{:+.1} \u{b0}F, {:+.5} in", m.delta_t, out.delta_thermal), Tone::Normal));
        }
        for n in &out.tolerance_notes {
            items.push(line(plain(&format!("! {n}")), Tone::Warn));
        }
        Card::new("bushing-numbers", "Fit and stress", Relevance::Info, items).for_steps(&["Checks"])
    }

    /// The finite-element picture: what is drawn, the state of its mesh and solve, and what the solve found.
    fn fe_card(&self) -> Card {
        let m = &self.st.model;
        let mut items = Vec::new();
        match fe::layout(m) {
            Ok(l) => {
                items.push(line(format!("Housing {:.3} x {:.3} in, {:.3} in thick; the bore centre is {:.3} in from the free edge.", l.length, l.height, self.fea.problem.thickness, l.edge), Tone::Normal));
                items.push(line("Held at the far end; the pin pulls the bushing toward the edge. Flanges and countersinks are not drawn.", Tone::Dim));
            }
            Err(why) => items.push(line(format!("No picture yet: {why}."), Tone::Warn)),
        }
        match self.fea.preview_summary() {
            Ok((nodes, elements)) => items.push(field("Mesh", format!("{elements} elements, {nodes} nodes"), Tone::Normal)),
            Err(_) if self.fea.meshing() => items.push(field("Mesh", "generating...", Tone::Warn)),
            Err(_) => items.push(field("Mesh", "not generated", Tone::Dim)),
        }
        if let Some(t) = self.fea.solving_for() {
            items.push(field("Solve", format!("running, {:.0} s", t.as_secs_f32()), Tone::Warn));
        }
        match self.fea.current_solution().filter(|s| s.problem == self.fea.problem) {
            None => items.push(field("Solve", "not run for these inputs", Tone::Dim)),
            Some(s) => {
                let sum = &s.summary;
                items.push(field("Max von Mises", format!("{:.0} psi", sum.max_von_mises.value), Tone::Normal));
                if let Some(i) = sum.interfaces.first() {
                    let analytic = m.output.pressure;
                    items.push(field("Fit pressure, FE", format!("{:.0} psi mean, {:.0} peak", i.mean_pressure, i.peak_pressure), Tone::Normal));
                    if analytic > 0.0 {
                        let rel = 100.0 * (i.mean_pressure - analytic) / analytic;
                        items.push(field("Closed form", format!("{analytic:.0} psi ({rel:+.1} % apart)"), if rel.abs() > 15.0 { Tone::Warn } else { Tone::Dim }));
                    }
                    items.push(field("Open arc", format!("{:.0}\u{b0}", i.open_arc_deg), if i.open_arc_deg > 0.0 { Tone::Warn } else { Tone::Normal }));
                    items.push(field("Friction torque capacity", format!("{:.1} lbf in", i.torque_capacity), Tone::Normal));
                    items.push(field("Slipping share", format!("{:.0} %", 100.0 * i.slip_share), Tone::Normal));
                }
                for n in sum.notes.iter().take(2) {
                    items.push(line(plain(n), Tone::Dim));
                }
            }
        }
        if self.fea.mesh_availability().is_ok() {
            items.push(DockItem::row("Generate mesh", Tone::Title, Some(act(BushingAct::GenerateMesh)), false));
        }
        if self.fea.solve_availability().is_ok() {
            items.push(DockItem::row("Solve fit and load (about 8 s)", Tone::Title, Some(act(BushingAct::Solve)), false));
        }
        if self.fea.solving() {
            items.push(DockItem::row("Cancel solve", Tone::Warn, Some(act(BushingAct::CancelSolve)), false));
        }
        Card::new("bushing-fe", "Finite-element model", Relevance::Primary, items).for_steps(&["Simulation"])
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let m = &self.st.model;
        let step = |s: &'static str| act(BushingAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'B', format!("Bushing: {}", self.name)).selected(false).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, r) in self.rows().into_iter().enumerate() {
            if let FieldRow::Header(h) = r {
                out.push(TreeNode::new(10 + k as u32, 2, '#', h).on_click(step("Inputs")));
            }
        }
        let failing = self.failing();
        out.push(TreeNode::new(3, 1, '+', format!("Checks ({failing} failing)")).folder().selected(self.step == "Checks").on_click(step("Checks")));
        for (k, c) in m.checks.iter().enumerate() {
            let (glyph, tone) = match c.severity {
                Severity::Pass => ('^', Tone::On),
                Severity::Warn => ('>', Tone::Warn),
                Severity::Fail => ('!', Tone::Warn),
            };
            let has_fix = m.recommendations.iter().any(|r| r.fixes == c.kind);
            out.push(TreeNode::new(100 + k as u32, 2, glyph, c.kind.label()).tone(tone).on_click(if has_fix { act(BushingAct::ShowFixes(c.kind)) } else { step("Checks") }));
        }
        out.push(TreeNode::new(4, 1, 'R', format!("Edge-distance cross-check{}", match (&self.st.edge_check, self.edge_current()) { (None, _) => "", (Some(_), true) => "", (Some(_), false) => " (stale)" })).on_click(step("Checks")));
        if !m.recommendations.is_empty() {
            out.push(TreeNode::new(5, 1, 'f', format!("Fixes ({})", m.recommendations.len())).on_click(act(BushingAct::AllFixes)));
        }
        out.push(TreeNode::new(6, 1, '+', "Finite-element model").folder().selected(self.step == "Simulation").on_click(step("Simulation")));
        out.push(TreeNode::new(7, 2, 'M', "Housing").on_click(step("Simulation")));
        out.push(TreeNode::new(8, 2, 'B', "Bushing").on_click(step("Simulation")));
        if self.fea.current_solution().is_some_and(|s| s.problem == self.fea.problem) {
            out.push(TreeNode::new(9, 2, 'f', "Result").on_click(step("Simulation")));
        }
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let row = |label: &str, r: FieldRow| entry(label, "", BushingAct::Row(r));
        let edge = self.edge_availability();
        vec![
            RibbonTab::new("Inputs")
                .group("Fit", vec![row("Fit type", FieldRow::ToggleFitType), row("Bore diameter", FieldRow::Number(NumberTarget::BoreDia)), row("Bushing ID", FieldRow::Number(NumberTarget::IdBushing)), row("Interference", FieldRow::Number(NumberTarget::Interference)), row("Friction", FieldRow::Number(NumberTarget::Friction))])
                .group("Materials", vec![row("Housing material", FieldRow::OpenHousingMaterialPicker), row("Bushing material", FieldRow::OpenBushingMaterialPicker)])
                .group("Housing and load", vec![row("Edge distance", FieldRow::Number(NumberTarget::EdgeDist)), row("Width", FieldRow::Number(NumberTarget::HousingWidth)), row("Length", FieldRow::Number(NumberTarget::HousingLen)), row("Pin load", FieldRow::Number(NumberTarget::Load))])
                .group("FEA workbench", vec![entry("Edit in the FEA workbench", "", BushingAct::OpenInFea).when(self.fe_note.as_ref().map_or(Ok(()), |why| Err(format!("no FE model yet: {why}"))))])
                .group("Edit", vec![entry("Undo", "Ctrl/Cmd+Z", BushingAct::Undo).when(self.undo_availability()), entry("Redo", "Ctrl/Cmd+Shift+Z", BushingAct::Redo).when(self.redo_availability()), entry("Save", "Ctrl/Cmd+S", BushingAct::Save)]),
            RibbonTab::new("Checks")
                .group("Cross-check", vec![entry("Edge check", "", BushingAct::EdgeCheck(false)).when(edge.clone()), entry("Edge check + contact FE", "", BushingAct::EdgeCheck(true)).when(edge), entry("Cancel", "Esc", BushingAct::CancelEdgeCheck).when(self.cancel_edge_availability())])
                .group("Fixes", vec![entry("Fixes", "", BushingAct::AllFixes).when(if self.st.model.recommendations.is_empty() { Err("no check is failing".to_string()) } else { Ok(()) })])
                .group("Report", vec![entry("Export report", "", BushingAct::Export)]),
            RibbonTab::new("Simulation").group(
                "Finite-element model",
                vec![
                    entry("Generate mesh", "", BushingAct::GenerateMesh).when(self.fea.mesh_availability()),
                    entry("Solve fit and load", "Ctrl/Cmd+Enter", BushingAct::Solve).when(self.fea.solve_availability()),
                    entry("Cancel solve", "", BushingAct::CancelSolve).when(if self.fea.solving() { Ok(()) } else { Err("nothing is solving".to_string()) }),
                    entry(if self.fea.show_results { "Show geometry" } else { "Show result" }, "V", BushingAct::ToggleResults).when(self.fea.results_availability()),
                ],
            ),
        ]
    }

    fn edge_availability(&self) -> Result<(), String> {
        if self.st.edge_job.is_some() {
            return Err("an edge check is already running".into());
        }
        edge_check::prepare(&self.st.model, false).map(|_| ())
    }
    fn cancel_edge_availability(&self) -> Result<(), String> {
        if self.st.edge_job.is_some() {
            Ok(())
        } else {
            Err("no edge check is running".into())
        }
    }
    fn undo_availability(&self) -> Result<(), String> {
        if self.undo.is_empty() {
            Err("nothing to undo".into())
        } else {
            Ok(())
        }
    }
    fn redo_availability(&self) -> Result<(), String> {
        if self.redo.is_empty() {
            Err("nothing to redo".into())
        } else {
            Ok(())
        }
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let pick = |tab: &str, labels: &[&str]| all(tab).into_iter().filter(|e| labels.contains(&e.label.as_str())).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: [pick("Inputs", &["Save", "Edit in the FEA workbench"]), pick("Checks", &["Export report"])].concat() },
            Menu { title: "Edit".into(), items: pick("Inputs", &["Undo", "Redo"]) },
            Menu { title: "Tools".into(), items: [pick("Checks", &["Edge check", "Edge check + contact FE", "Cancel", "Fixes"])].concat() },
            Menu { title: "Simulation".into(), items: all("Simulation").into_iter().map(Item::Entry).collect() },
        ]
    }
}
