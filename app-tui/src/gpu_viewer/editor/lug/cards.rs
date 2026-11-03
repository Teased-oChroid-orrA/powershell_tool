//! What the Lug Analysis workbench shows: the inspector's cards, the tree, the ribbon and the menus. Plain data built from
//! the model and the last run; the window draws it. Row labels, values and hints are the model's
//! (`model::{row_label, display_value, field_hint}`, `LugUiModel::validation_hint`); every number comes from `lug-solver`
//! through `model::run` (`LugRun`: solution, checks, collapse, bushing, pin bending, comparison, notes).
use super::{drawing, LugAct, LugSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Authoring, Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::theme::fmt_margin;
use crate::toolboxes::lug_analysis::model::{self, Check, Comparison, FieldRow, LugRun, SolverChoice, Status};

fn act(a: LugAct) -> UiAction {
    UiAction::Editor(EditorAction::Lug(a))
}
fn entry(label: &str, shortcut: &str, a: LugAct) -> Entry {
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
fn ksi(v: f64) -> String {
    format!("{:.1} ksi", v / 1000.0)
}
/// The window's tone for a check: a pass reads on, a warning or a failure warns.
fn tone(s: Status) -> Tone {
    match s {
        Status::Pass => Tone::On,
        Status::Warn | Status::Fail => Tone::Warn,
        Status::Info => Tone::Normal,
    }
}
fn ok_tone(ok: bool) -> Tone {
    if ok {
        Tone::On
    } else {
        Tone::Warn
    }
}
/// The relative difference of `a` from `b`, or a dash when `b` is zero.
fn diff(a: f64, b: f64) -> String {
    if b.abs() > 1e-12 {
        format!("{:+.1} %", 100.0 * (a / b - 1.0))
    } else {
        "-".into()
    }
}

/// The one-line answer above the checks: one fails, some need a look, or every check is met; and the lowest margin.
fn verdict(checks: &[Check]) -> (String, bool) {
    let failing: Vec<&Check> = checks.iter().filter(|c| c.status == Status::Fail).collect();
    let warning = checks.iter().filter(|c| c.status == Status::Warn).count();
    let lowest = checks.iter().filter_map(|c| c.margin.map(|m| (m, c.name))).min_by(|a, b| a.0.total_cmp(&b.0));
    let text = if let Some(first) = failing.first() {
        format!("FAIL: {} check{} not met, first {}", failing.len(), if failing.len() == 1 { "" } else { "s" }, first.name)
    } else if warning > 0 {
        format!("REVIEW: {warning} check{} with a warning", if warning == 1 { "" } else { "s" })
    } else {
        "PASS: every check is met".into()
    };
    let margin = lowest.map_or(String::new(), |(m, name)| format!("; lowest MS {} ({name})", fmt_margin(m)));
    (format!("{text}{margin}"), failing.is_empty() && warning == 0)
}

impl LugSession {
    /// The analysis of the inputs shown is in and every check is met.
    pub(super) fn all_pass(&self) -> bool {
        self.current_run().is_some_and(|r| verdict(&r.checks).1)
    }

    /// One line for the status bar: the verdict and lowest margin, or why there is none yet.
    pub(super) fn verdict(&self) -> String {
        if let Some(e) = self.error() {
            return format!("Cannot analyse: {e}.");
        }
        match self.current_run() {
            Some(run) => format!("{}.", verdict(&run.checks).0),
            None if self.running() => "Analysing the lug...".into(),
            None => "Waiting for the analysis.".into(),
        }
    }

    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = self.input_cards();
        cards.push(self.result_card());
        if let Some(run) = &self.run {
            cards.extend(result_cards(run));
        }
        cards.push(self.drawing_card());
        cards
    }

    /// The window's text for an input row: the form's, with the mesh test's progress and the section's state.
    fn row_value(&self, row: FieldRow) -> String {
        match row {
            FieldRow::MeshAdvice => self.mesh_advice_text(),
            FieldRow::MeshSection => format!("{}, {}", if self.model.mesh_open { "Hide" } else { "Show" }, model::display_value(&self.model, row)),
            r => model::display_value(&self.model, r),
        }
    }

    /// One card per section of the input rows (`field_rows`), in the form's own order; the Advanced settings row
    /// starts its own card and its sections follow it.
    fn input_cards(&self) -> Vec<Card> {
        let mut sections: Vec<(String, Vec<DockItem>)> = Vec::new();
        for row in model::field_rows(&self.model) {
            match row {
                FieldRow::Header(h) => {
                    sections.push((h.to_string(), Vec::new()));
                    continue;
                }
                FieldRow::MeshSection => sections.push((model::row_label(row).to_string(), Vec::new())),
                _ => {}
            }
            let editing = self.prompt.as_ref().filter(|p| p.row == row);
            let text = editing.map_or_else(|| self.row_value(row), |p| format!("{}_", p.text));
            // The window draws a Warn value as an edit box: only the row being typed is one.
            let tone = if editing.is_some() { Tone::Warn } else { Tone::Normal };
            // A catalog name is too long for the value column: it reads on its own line under the row.
            let picker = matches!(row, FieldRow::OpenMaterialPicker | FieldRow::OpenBushingMaterialPicker);
            let shown = if picker { "Browse".to_string() } else { text.clone() };
            let mut items = vec![DockItem::field(model::row_label(row), shown, tone, Some(act(LugAct::Row(row))), editing.is_some())];
            if picker {
                items.push(line(text, Tone::Normal));
            }
            if let FieldRow::Number(t) = row {
                if let Some(why) = self.model.validation_hint(t) {
                    items.push(line(format!("{} {why}.", t.label()), Tone::Warn));
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
                Card::new(format!("lug-{}", slug(&title)), title, Relevance::Primary, items).for_steps(&["Inputs"])
            })
            .collect()
    }

    /// The verdict and every check, or why there is none yet: shown at every step.
    fn result_card(&self) -> Card {
        let mut items = Vec::new();
        if let Some(e) = self.error() {
            items.push(line(format!("Cannot analyse: {e}."), Tone::Warn));
        }
        match (&self.run, self.current_run().is_some()) {
            (None, _) if self.running() => items.push(line("Analysing the lug...", Tone::Dim)),
            (None, _) => items.push(line(if self.error().is_some() { "Every result needs inputs the solver accepts." } else { "Starts a moment after the inputs stop changing." }, Tone::Dim)),
            (Some(run), current) => {
                if !current {
                    items.push(line(if self.running() { "Analysing the new inputs; the last completed analysis is shown." } else { "Showing the last completed analysis; the inputs have changed." }, Tone::Dim));
                }
                let (text, pass) = verdict(&run.checks);
                items.push(line(text, ok_tone(pass)));
                for c in &run.checks {
                    let ms = c.margin.map_or("-".to_string(), |m| format!("MS {}", fmt_margin(m)));
                    items.push(DockItem::field(c.name, ms, tone(c.status), None, false));
                    if !c.detail.is_empty() {
                        items.push(line(c.detail.clone(), Tone::Dim));
                    }
                }
            }
        }
        Card::new("lug-result", "Result", Relevance::Primary, items)
    }

    fn drawing_card(&self) -> Card {
        let text = if self.model.input().is_err() {
            return Card::new("lug-drawing", "Drawing", Relevance::Info, vec![line("Nothing to draw: the inputs make no lug.", Tone::Warn)]);
        } else if self.shows_result() {
            "The lug on its finite-element mesh, contoured by the kernel's nodal field (a half model is mirrored about the lug axis). Click the legend's title for the next field.".to_string()
        } else if self.current_run().is_some() {
            "The mesh coloured by part: the legacy solver gives no nodal field to contour.".to_string()
        } else if self.error().is_some() {
            "The mesh coloured by part: the analysis failed, so no stress to show.".to_string()
        } else {
            "The mesh coloured by part, the pin on the side the load drives it to, while the analysis runs; its stress follows.".to_string()
        };
        let mut items = vec![line(text, Tone::Dim)];
        if self.shows_result() {
            items.push(value("Fields", drawing::FIELDS.join(", ").to_lowercase()));
        }
        Card::new("lug-drawing", "Drawing", Relevance::Info, items)
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let step = |s: &'static str| act(LugAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'L', format!("Lug: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, r) in model::field_rows(&self.model).into_iter().enumerate() {
            if let FieldRow::Header(h) = r {
                out.push(TreeNode::new(10 + k as u32, 2, '#', h).on_click(step("Inputs")));
            }
        }
        let run = self.current_run();
        let state = match run {
            Some(_) if self.all_pass() => "pass",
            Some(_) => "review",
            None => "pending",
        };
        out.push(TreeNode::new(3, 1, '+', format!("Results ({state})")).folder().tone(ok_tone(self.all_pass())).selected(self.step == "Results").on_click(step("Results")));
        for (k, c) in run.iter().flat_map(|r| &r.checks).enumerate() {
            let ok = c.status != Status::Fail && c.status != Status::Warn;
            let ms = c.margin.map_or(String::new(), |m| format!(" MS {}", fmt_margin(m)));
            out.push(TreeNode::new(200 + k as u32, 2, if ok { '^' } else { '!' }, format!("{}{ms}", c.name)).tone(tone(c.status)).on_click(step("Results")));
        }
        out.push(TreeNode::new(4, 1, 'R', "Report").selected(self.step == "Report").on_click(step("Report")));
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let never = |what: &str| Err(format!("nothing to {what}"));
        let edit = vec![
            entry("Undo", "Ctrl/Cmd+Z", LugAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) }),
            entry("Redo", "Ctrl/Cmd+Shift+Z", LugAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) }),
            entry("Save", "Ctrl/Cmd+S", LugAct::Save),
        ];
        let lug = vec![
            entry("Lug material", "", LugAct::Row(FieldRow::OpenMaterialPicker)),
            entry("Head shape", "", LugAct::Row(FieldRow::ToggleHeadShape)),
            entry("Bushing", "", LugAct::Row(FieldRow::ToggleBushing)),
        ];
        let analysis = vec![entry("Solver", "", LugAct::Row(FieldRow::ToggleSolver)), entry("Plastic limit load", "", LugAct::Row(FieldRow::TogglePlastic)), entry("Mesh size test", "", LugAct::Row(FieldRow::MeshAdvice))];
        let export = || entry("Export report", "", LugAct::Export).when(if self.current_run().is_some() { Ok(()) } else { Err("the analysis of these inputs has not finished".into()) });
        vec![
            RibbonTab::new("Inputs").group("Lug", lug).group("Analysis", analysis).group("Edit", edit),
            RibbonTab::new("Results").group("Results", vec![export()]),
            RibbonTab::new("Report").group("Report", vec![export(), entry("Save", "Ctrl/Cmd+S", LugAct::Save)]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let pick = |tab: &str, labels: &[&str]| all(tab).into_iter().filter(|e| labels.contains(&e.label.as_str())).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: pick("Report", &["Save", "Export report"]) },
            Menu { title: "Edit".into(), items: pick("Inputs", &["Undo", "Redo"]) },
            Menu { title: "Tools".into(), items: pick("Inputs", &["Lug material", "Head shape", "Bushing", "Solver", "Plastic limit load", "Mesh size test"]) },
        ]
    }
}

/// Every result section the terminal shows, one card each, only those the run has.
fn result_cards(run: &LugRun) -> Vec<Card> {
    let results = |c: Card| c.for_steps(&["Results", "Report"]);
    let (s, i) = (&run.solution, &run.input);
    let mut cards = Vec::new();

    let mut contact = vec![value("Load", format!("{:.1} lbf at {:.1} deg", i.case.load_lbf, i.case.angle_deg)), value("Pin travel", format!("{:.5} in", s.bearing_deflection))];
    if s.peak_pressure > 0.0 {
        let arc = if s.contact_arc_deg > 359.0 { "all round".to_string() } else { format!("{:.1} deg about {:.0} deg", s.contact_arc_deg, s.contact_centre_deg.rem_euclid(360.0)) };
        contact.push(value("Contact patch", arc));
        contact.push(value("Peak pressure", ksi(s.peak_pressure)));
        contact.push(value("Bearing P/(D t)", ksi(s.bearing_stress)));
        if i.pin.friction > 0.0 {
            let slipping = s.contact.iter().filter(|p| p.slipping).count();
            let touching = s.contact.iter().filter(|p| p.pressure > 0.0).count();
            contact.push(line(format!("Friction {:.2}: {slipping} of {touching} contact points slipping.", i.pin.friction), Tone::Dim));
        }
    } else {
        contact.push(line("The pin is not touching the bore (clearance).", Tone::Dim));
    }
    cards.push(results(Card::new("lug-contact", "Contact", Relevance::Primary, contact)));

    cards.push(results(Card::new(
        "lug-stress",
        "Stress (elastic FE)",
        Relevance::Primary,
        vec![
            value("Peak hoop", format!("{} at {:.0} deg", ksi(s.peak_hoop), s.peak_hoop_angle_deg)),
            value("Peak von Mises", ksi(s.peak_von_mises)),
            value("Net section", ksi(s.net_section_stress)),
            value("Kt (net)", format!("{:.2}", s.kt_net)),
        ],
    )));

    if let Some(l) = &run.limit {
        let mut items = vec![
            value("Collapse load", format!("{:.0} lbf{}", l.limit_load_lbf, if l.plateau { "" } else { " (lower bound)" })),
            value("Flow stress", format!("{} ({})", ksi(l.flow_stress), i.flow_rule.label())),
        ];
        if i.case.load_lbf > 0.0 {
            let zone = l.curve.iter().max_by(|a, b| a.load_lbf.total_cmp(&b.load_lbf)).map_or(0.0, |c| c.plastic_fraction);
            items.push(value("Applied / collapse", format!("{:.0} %", 100.0 * i.case.load_lbf / l.limit_load_lbf)));
            items.push(value("Plastic zone", format!("{:.0} % of the lug at collapse", 100.0 * zone)));
        }
        cards.push(results(Card::new("lug-ultimate", "Ultimate (plastic collapse)", Relevance::Primary, items)));
    }
    if let (Some(f), Some(e)) = (&run.finite, i.ductility) {
        let mut items = vec![value("Collapse load", format!("{:.0} lbf{}", f.collapse_lbf, if f.strain_limited || f.peak_reached { "" } else { " (lower bound)" })), value("Failure strain", format!("{e:.2}"))];
        if !(f.strain_limited || f.peak_reached) {
            items.push(line("The travel cap ended the run before the peak.", Tone::Dim));
        }
        if i.case.load_lbf > 0.0 {
            items.push(value("Applied / collapse", format!("{:.0} %", 100.0 * i.case.load_lbf / f.collapse_lbf)));
            items.push(value("Travel at collapse", format!("{:.4} in", f.curve.last().map_or(0.0, |c| c.travel))));
        }
        items.push(value("Run", format!("{} steps, {:.1} s", f.curve.len(), f.elapsed_ms / 1e3)));
        cards.push(results(Card::new("lug-finite", "Ultimate (finite strain)", Relevance::Primary, items)));
    }
    if let (Some(b), Some(bi)) = (&s.bushing, &i.bushing) {
        cards.push(results(Card::new(
            "lug-bushing",
            "Bushing and fit",
            Relevance::Primary,
            vec![
                value("Fit pressure", format!("{} unloaded", ksi(b.fit_pressure_unloaded))),
                value("Fit", format!("{:+.4} in, friction {:.2}", bi.interference_dia, bi.friction)),
                value("Interface at load", format!("mean {}, peak {}", ksi(b.interface_mean_pressure), ksi(b.interface_peak_pressure))),
                value("Contact lost", format!("{:.0} % of the interface", 100.0 * b.separated_fraction)),
                value("Bushing hoop", format!("{} at {:.0} deg", ksi(b.peak_hoop), b.peak_hoop_angle_deg)),
                value("Bushing von Mises", ksi(b.peak_von_mises)),
                value("Bearing P/(ID t)", ksi(b.bearing_stress)),
            ],
        )));
    }
    if let Some((t, hoop)) = &run.thickness {
        cards.push(results(Card::new(
            "lug-pin-bending",
            "Pin bending",
            Relevance::Primary,
            vec![
                value("Peaking", format!("{:.2} x the mean over {} slices", t.peaking, t.slice_load.len())),
                value("Pin moment", format!("{:.0} lbf-in", t.max_moment)),
                value("Pin bending", ksi(t.pin_bending_stress)),
                value("Pin shear", ksi(t.pin_shear_stress)),
                value("Hoop, worst slice", ksi(*hoop)),
            ],
        )));
    }
    if let Some(c) = &run.comparison {
        cards.push(results(comparison_card(run, c)));
    }
    if !run.notes.is_empty() {
        cards.push(results(Card::new("lug-notes", "Notes", Relevance::Primary, run.notes.iter().map(|n| line(n.clone(), Tone::Warn)).collect())));
    }
    cards.push(Card::new("lug-bore", "Bore profile", Relevance::Context, bore_rows(run)).for_steps(&["Results"]));
    if let Some(l) = &run.limit {
        let mut items = vec![line("Pin travel from its force-free position: load and plastic fraction.", Tone::Dim)];
        items.extend(l.curve.iter().map(|c| value(format!("{:.4} in", c.travel), format!("{:.0} lbf, {:.0} % plastic", c.load_lbf, 100.0 * c.plastic_fraction))));
        cards.push(Card::new("lug-collapse-curve", "Load-travel curve", Relevance::Context, items).for_steps(&["Results"]));
    }
    cards.push(model_card(run));
    cards
}

/// The legacy solver's numbers beside the kernel's, with the difference of each.
fn comparison_card(run: &LugRun, c: &Comparison) -> Card {
    let (k, l) = (&run.solution, &c.solution);
    let row = |name: &str, a: String, b: String, d: String| value(name, format!("{a} / {b} ({d})"));
    let mut items = vec![
        line("Kernel / legacy (difference):", Tone::Dim),
        row("Peak hoop", ksi(k.peak_hoop), ksi(l.peak_hoop), diff(k.peak_hoop, l.peak_hoop)),
        row("Peak von Mises", ksi(k.peak_von_mises), ksi(l.peak_von_mises), diff(k.peak_von_mises, l.peak_von_mises)),
        row("Peak pressure", ksi(k.peak_pressure), ksi(l.peak_pressure), diff(k.peak_pressure, l.peak_pressure)),
        row("Pin travel", format!("{:.5}", k.bearing_deflection), format!("{:.5} in", l.bearing_deflection), diff(k.bearing_deflection, l.bearing_deflection)),
        row("Contact patch", format!("{:.1}", k.contact_arc_deg), format!("{:.1} deg", l.contact_arc_deg), format!("{:+.1} deg", k.contact_arc_deg - l.contact_arc_deg)),
    ];
    if let (Some(a), Some(b)) = (&run.limit, &c.limit) {
        items.push(row("Collapse load", format!("{:.0}", a.limit_load_lbf), format!("{:.0} lbf", b.limit_load_lbf), diff(a.limit_load_lbf, b.limit_load_lbf)));
    }
    if let (Some(a), Some(b)) = (&run.finite, &c.finite) {
        items.push(row("Collapse (finite)", format!("{:.0}", a.collapse_lbf), format!("{:.0} lbf", b.collapse_lbf), diff(a.collapse_lbf, b.collapse_lbf)));
    }
    items.push(line(format!("Legacy solver {:.0} ms; kernel {:.0} ms.", c.ms, run.total_ms), Tone::Dim));
    Card::new("lug-comparison", "Solver comparison", Relevance::Primary, items)
}

/// Bore angles every 15 degrees: contact pressure, friction and hoop stress (and the fit pressure with a bushing), read
/// from the nearest bore and contact points; a half model's other side is its mirror image.
fn bore_rows(run: &LugRun) -> Vec<DockItem> {
    let s = &run.solution;
    let symmetric = run.input.case.is_axial();
    let fold = |a: f64| {
        let a = a.rem_euclid(360.0);
        if symmetric && a > 180.0 {
            360.0 - a
        } else {
            a
        }
    };
    let apart = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    };
    let mut items = vec![line(
        if s.bushing.is_some() { "Pressure / friction / hoop on the pin-bearing surface (the bushing's bore), then the bushing-to-lug fit pressure, ksi." } else { "Pressure / friction / hoop stress on the bore surface, ksi; the angle is about the hole centre." },
        Tone::Dim,
    )];
    for k in 0..24 {
        let ang = k as f64 * 15.0;
        let want = fold(ang);
        let bore = s.bore.iter().min_by(|a, b| apart(a.angle_deg, want).total_cmp(&apart(b.angle_deg, want)));
        let cp = s.contact.iter().min_by(|a, b| apart(a.angle_deg, want).total_cmp(&apart(b.angle_deg, want)));
        let (Some(bore), Some(cp)) = (bore, cp) else { continue };
        let shear = if symmetric && ang > 180.0 { -cp.shear } else { cp.shear };
        let fit = s.bushing.as_ref().map_or(String::new(), |b| {
            let p = b.interface.iter().min_by(|x, y| apart(x.angle_deg, want).total_cmp(&apart(y.angle_deg, want))).map_or(0.0, |p| p.pressure);
            format!(", fit {:.1}", p / 1000.0)
        });
        items.push(value(format!("{ang:.0} deg"), format!("{:.1} / {:.1} / {:.1}{fit}", cp.pressure / 1000.0, shear / 1000.0, bore.hoop / 1000.0)));
    }
    items
}

/// What was solved and how: the solver, the mesh, the iterations and the time.
fn model_card(run: &LugRun) -> Card {
    let s = &run.solution;
    let collapse = match (&run.limit, run.limit_reused) {
        (Some(_), true) => ", collapse reused",
        (Some(_), false) => ", collapse computed",
        _ => "",
    };
    Card::new(
        "lug-model",
        "Model",
        Relevance::Context,
        vec![
            value("Solver", if run.solver == SolverChoice::Kernel { "kernel (fea-core)" } else { "legacy condensed" }),
            value("Mesh", format!("{} nodes, {} dofs, {}", s.mesh.nodes, s.dofs, if run.input.case.is_axial() { "half lug" } else { "full lug" })),
            value("Aspect", format!("max {:.1}, bore {:.1}", s.mesh.aspect_max, s.mesh.bore_aspect_max)),
            value("Newton iterations", s.contact_iterations.to_string()),
            value("Time", format!("{:.0} ms ({}{collapse})", run.total_ms, if run.reused_model { "model reused" } else { "model built" })),
        ],
    )
    .for_steps(&["Results", "Report"])
}
