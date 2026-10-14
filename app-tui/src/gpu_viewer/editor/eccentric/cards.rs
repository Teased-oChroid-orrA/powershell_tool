//! What the Eccentric Bushing workbench shows: inspector cards for the Inputs and Analysis steps, the tree, the ribbon and
//! the menus. Plain data built from the toolbox's state; the numbers and wording are the terminal toolbox's.
use super::{EccentricAct, EccentricSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::toolboxes::eccentric_bushing::{
    model::{self, FieldRow, NumberTarget, Task},
    view::display_value,
};

fn act(a: EccentricAct) -> UiAction {
    UiAction::Editor(EditorAction::Eccentric(a))
}
fn entry(label: &str, shortcut: &str, a: EccentricAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn line(text: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::text(text, tone)
}
fn field(label: &str, value: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::field(label, value, tone, None, false)
}
fn button(text: &str, a: EccentricAct) -> DockItem {
    DockItem::row(text, Tone::Title, Some(act(a)), false)
}
/// The window font lacks arrows and several symbols.
fn plain(s: &str) -> String {
    s.replace('\u{2192}', "->").replace('\u{2265}', ">=").replace('\u{2264}', "<=").replace('\u{26a0}', "!").replace('\u{2717}', "x").replace('\u{2713}', "ok").replace('\u{2026}', "...")
}
fn slug(t: &str) -> String {
    t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}

impl EccentricSession {
    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = self.input_cards();
        cards.push(self.bushing_card());
        cards.extend(self.analysis_cards());
        cards
    }

    fn input_cards(&self) -> Vec<Card> {
        let mut sections: Vec<(String, Vec<DockItem>)> = Vec::new();
        for row in self.rows() {
            match row {
                FieldRow::Header("Run") | FieldRow::Run(_) => continue,
                FieldRow::Header(h) => sections.push((h.to_string(), Vec::new())),
                FieldRow::AdvancedSection => {
                    let item = DockItem::field("Advanced settings", if self.st.ui.advanced_open { "shown" } else { "hidden" }, Tone::Dim, Some(act(EccentricAct::Row(row))), false);
                    match sections.last_mut() {
                        Some(s) => s.1.push(item),
                        None => sections.push((String::new(), vec![item])),
                    }
                }
                _ => {
                    let editing = self.prompt.as_ref().filter(|p| p.row == row);
                    let value = editing.map_or_else(|| display_value(&self.st, row), |p| format!("{}_", p.text));
                    let item = DockItem::field(model::row_label(row), value, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(EccentricAct::Row(row))), editing.is_some());
                    match sections.last_mut() {
                        Some(s) => s.1.push(item),
                        None => sections.push((String::new(), vec![item])),
                    }
                }
            }
        }
        let editing = self.prompt.is_some();
        sections
            .into_iter()
            .enumerate()
            .map(|(k, (title, mut items))| {
                if editing && items.iter().any(|i| matches!(i, DockItem::Field { focused: true, .. })) {
                    items.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
                }
                let title = if title.is_empty() { "Inputs".to_string() } else { title };
                Card::new(format!("eccentric-{}", slug(&title)), title, if k < 2 { Relevance::Primary } else { Relevance::Context }, items).for_steps(&["Inputs"])
            })
            .collect()
    }

    /// What is read from the Bushing case, and the two actions around it.
    fn bushing_card(&self) -> Card {
        let b = &self.bushing;
        let o = &b.output;
        let items = vec![
            line(plain(&self.bushing_note), Tone::Dim),
            field("Bore", format!("{:.4} in", o.bore_tol.nominal), Tone::Normal),
            field("Bushing ID", format!("{:.4} in", b.id_bushing), Tone::Normal),
            field("Interference (dia)", format!("{:.4} in", o.delta_total), Tone::Normal),
            field("Friction", format!("{:.2}", b.friction), Tone::Normal),
            field("Length", format!("{:.3} in", b.housing_len), Tone::Normal),
            field("Pin load", format!("{:.0} lbf", b.load), Tone::Normal),
            field("Housing", b.housing_material().name, Tone::Normal),
            field("Bushing", b.bushing_material().name, Tone::Normal),
            field("Edge distance", format!("{:.3} in", b.edge_dist), Tone::Normal),
            field("Minimum wall", format!("{:.4} in", b.min_wall_straight), Tone::Normal),
            button("Reload the Bushing case", EccentricAct::ReloadBushing),
            button("Edit it in the Bushing workbench", EccentricAct::OpenBushing),
        ];
        Card::new("eccentric-bushing-case", "From the Bushing case", Relevance::Context, items).for_steps(&["Inputs"])
    }

    fn analysis_cards(&self) -> Vec<Card> {
        let steps = &["Analysis"];
        let mut cards = Vec::new();
        let input = self.fresh_input();
        let fresh = |sig: &model::Inputs| input.as_ref().is_some_and(|i| i == sig);

        // The run in flight, its progress and the queue.
        let mut run = Vec::new();
        if let Some(job) = &self.st.job {
            run.push(line(format!("{} ({:.0} s)", model::task_name(job.task), job.started.elapsed().as_secs_f32()), Tone::Warn));
            if let Some(p) = job.control.progress.as_ref().map(|p| p.snapshot()) {
                if !p.detail.is_empty() {
                    run.push(line(plain(&p.detail), Tone::Dim));
                }
                let mut sofar = format!("{} solve{} done", p.solves_done, if p.solves_done == 1 { "" } else { "s" });
                if let Some((lo, hi)) = p.bracket {
                    sofar.push_str(&format!(", the answer lies in {lo:.4} to {hi:.4}"));
                }
                run.push(line(sofar, Tone::Dim));
            }
            if !self.st.queue.is_empty() {
                run.push(line(format!("queued next: {}", self.st.queue.iter().map(|t| model::task_name(*t)).collect::<Vec<_>>().join(", ")), Tone::Dim));
            }
            run.push(DockItem::row("Cancel", Tone::Warn, Some(act(EccentricAct::Cancel)), false));
        }
        if let Some(e) = &self.st.error {
            run.push(line(plain(e), Tone::Warn));
        }
        if let Some(why) = &self.fe_note {
            run.push(line(plain(&format!("No picture: {why}")), Tone::Warn));
        }
        if !run.is_empty() {
            cards.push(Card::new("eccentric-run", "Run", Relevance::Primary, run).for_steps(steps));
        }

        if let Some((sig, a)) = &self.st.analysis {
            let spins = a.margin.is_finite() && a.margin < 0.0;
            let spin = if a.margin.is_finite() { format!("{} at e = {:.4} in, margin {:+.1} %", if spins { "SPINS" } else { "HOLDS" }, a.offset, a.margin * 100.0) } else { format!("no spin torque at this load angle (e = {:.4} in)", a.offset) };
            let (head, tone) = if !a.wall_ok { (format!("THIN WALL below the minimum; spin: {spin}"), Tone::Warn) } else if spins { (spin, Tone::Warn) } else if a.margin.is_finite() { (spin, Tone::On) } else { (spin, Tone::Dim) };
            let mut items = vec![DockItem::row(head, tone, None, false)];
            if !fresh(sig) {
                items.push(line("Inputs changed since this run: analyse again.", Tone::Warn));
            }
            let credited = input.as_ref().is_some_and(|i| i.credit_pin_load);
            items.push(field("Torque required F e sin(a)", format!("{:.2} lbf in", a.torque_required), Tone::Normal));
            items.push(field("Capacity, fit alone", format!("{:.2} lbf in", a.torque_capacity_fit), Tone::Normal));
            items.push(field("Capacity, with pin load", format!("{:.2} lbf in", a.torque_capacity), Tone::Normal));
            items.push(field(if credited { "Design capacity (pin loaded)" } else { "Design capacity (fit alone)" }, format!("{:.2} lbf in", a.design_capacity), Tone::Normal));
            if let Some(onset) = a.onset_torque {
                items.push(field("Direct spin onset", format!("{onset:.2} lbf in ({:+.1} % vs integral)", 100.0 * (onset / a.torque_capacity_fit - 1.0)), Tone::Normal));
            }
            if a.pin_peak_pressure > 0.0 {
                items.push(field("Pin on the bore", format!("{:.0} deg, peak {:.0} psi", a.pin_arc_deg, a.pin_peak_pressure), Tone::Normal));
            }
            items.push(field("Wall thin / thick", format!("{:.4} / {:.4} in", a.wall_thin, a.wall_thick), if a.wall_ok { Tone::Normal } else { Tone::Warn }));
            items.push(field("Fit pressure", format!("{:.0} to {:.0} psi", a.fit_pressure_min, a.fit_pressure_max), Tone::Normal));
            if a.contact_lost_deg > 0.0 {
                items.push(line(format!("! contact lost over {:.0} deg of the interface under the pin load", a.contact_lost_deg), Tone::Warn));
            }
            items.push(line(format!("FE check: interface force ({:.0}, {:.0}) lbf, friction moment {:.2} lbf in", a.net_force[0], a.net_force[1], a.friction_torque), Tone::Dim));
            if a.mesh_error.is_finite() {
                items.push(line(format!("estimated mesh error {:.1} % of the energy norm{}", 100.0 * a.mesh_error, if a.mesh_error > 0.10 { ": local pressures (pin peak) are uncertain, the capacity integral is not" } else { "" }), if a.mesh_error > 0.10 { Tone::Warn } else { Tone::Dim }));
            }
            if let Some(why) = &a.loaded_failure {
                items.push(line(plain(&format!("! the pin load could not be solved ({why}): capacity and margin are the fit alone (conservative), the pin results are blank")), Tone::Warn));
            }
            if self.result_is_current() {
                items.push(DockItem::row(if self.show_result { "Show the inputs picture" } else { "Show the solved model" }, Tone::Title, Some(act(EccentricAct::ToggleResult)), false));
            }
            cards.push(Card::new("eccentric-verdict", "Spin check", Relevance::Primary, items).for_steps(steps));
            if self.st.show_profile {
                let mut p = vec![line("psi, fit alone | with the pin load; the wall is thinnest at 0 degrees and thickest at 180", Tone::Dim)];
                let peak = a.profile.iter().map(|b| b.fit.max(b.loaded)).fold(1.0, f64::max);
                let per = (a.profile.len() / 24).max(1);
                for k in 0..24 {
                    let Some(bins) = a.profile.get(k * per..(k + 1) * per) else { break };
                    let mean = |f: fn(&eccentric_bushing::ProfileBin) -> f64| bins.iter().map(f).sum::<f64>() / bins.len() as f64;
                    let (fit, loaded) = (mean(|b| b.fit), mean(|b| b.loaded));
                    p.push(field(&format!("{:>3} deg", k * 15), format!("{fit:.0} | {loaded:.0}  {}", "|".repeat(((loaded / peak) * 16.0).round() as usize)), Tone::Normal));
                }
                cards.push(Card::new("eccentric-profile", "Interface pressure", Relevance::Context, p).for_steps(steps));
            }
        }
        if let Some((sig, l)) = &self.st.max_offset {
            let mut items = vec![];
            let note = if l.bounded_by_wall { " (spin never limits it: the numerical wall floor does)" } else if l.set_by_loaded_run { " (set by contact loss under load)" } else { "" };
            items.push(line(format!("spin allows e up to {:.4} in{note}", l.value), Tone::Normal));
            items.push(line(format!("the minimum wall allows e up to {:.4} in{}", l.wall_limit, if l.wall_limit < l.value { ": the wall governs, not spin" } else { "" }), if l.wall_limit < l.value { Tone::Warn } else { Tone::Dim }));
            if let Some(n) = model::halted_note(l, "in", 4).or_else(|| model::caveat_note(l)) {
                items.push(line(plain(&format!("! {n}")), Tone::Warn));
            }
            if !fresh(sig) {
                items.push(line("Inputs changed since this run.", Tone::Warn));
            }
            cards.push(Card::new("eccentric-max-offset", "Maximum offset that holds", Relevance::Primary, items).for_steps(steps));
        }
        if let Some((sig, l)) = &self.st.max_load {
            let mut items = vec![line(if l.value.is_finite() { format!("F max = {:.0} lbf", l.value) } else { "no spin torque at this angle: any load is held".to_string() }, Tone::Normal)];
            if let Some(n) = model::halted_note(l, "lbf", 0).or_else(|| model::caveat_note(l)) {
                items.push(line(plain(&format!("! {n}")), Tone::Warn));
            }
            if !fresh(sig) {
                items.push(line("Inputs changed since this run.", Tone::Warn));
            }
            cards.push(Card::new("eccentric-max-load", "Maximum pin load that is held", Relevance::Primary, items).for_steps(steps));
        }
        if let Some((sig, points)) = &self.st.sweep {
            let mut items = Vec::new();
            if !fresh(sig) {
                items.push(line("Inputs changed since this run.", Tone::Warn));
            }
            for p in points {
                let (text, tone) = match (p.margin, &p.error) {
                    (Some(m), _) if !m.is_finite() => ("no spin torque".to_string(), Tone::Dim),
                    (Some(m), _) if m < 0.0 => (format!("{:+.0} %  SPINS", m * 100.0), Tone::Warn),
                    (Some(m), _) => (format!("{:+.0} %  {}", m * 100.0, "|".repeat((m.min(10.0) / 10.0 * 16.0).ceil().max(1.0) as usize)), if m < 0.25 { Tone::Warn } else { Tone::On }),
                    (None, why) => (why.clone().unwrap_or_else(|| "no result".into()), Tone::Warn),
                };
                items.push(field(&format!("e {:.4} in", p.offset), text, tone));
            }
            cards.push(Card::new("eccentric-sweep", "Margin versus offset", Relevance::Context, items).for_steps(steps));
        }
        if self.st.history.len() > 1 {
            let latest = self.st.history.last().cloned();
            let mut items = Vec::new();
            for h in self.st.history.iter().rev().skip(1) {
                let delta = latest.as_ref().map_or(String::new(), |l| if h.margin.is_finite() && l.margin.is_finite() { format!(", latest {:+.0} points", (l.margin - h.margin) * 100.0) } else { String::new() });
                let margin = if h.margin.is_finite() { format!("{:+.0} %", h.margin * 100.0) } else { "no spin torque".into() };
                items.push(line(format!("e {:.4} in, {:.0} lbf at {:.0} deg: margin {margin}, capacity {:.1}{delta}", h.offset, h.load_lbf, h.load_angle_deg, h.design_capacity), Tone::Dim));
            }
            cards.push(Card::new("eccentric-history", "Earlier analyses", Relevance::Info, items).for_steps(steps));
        }
        if self.st.analysis.is_none() && self.st.max_offset.is_none() && self.st.max_load.is_none() && self.st.job.is_none() {
            cards.push(Card::new("eccentric-start", "Analysis", Relevance::Primary, vec![line("Analyse this offset, find the largest offset or pin load the fit holds, or sweep the margin over the offsets. Runs take seconds to tens of seconds and never start by themselves.", Tone::Dim), button("Analyse this offset", EccentricAct::Analyse)]).for_steps(steps));
        }
        cards.push(Card::new("eccentric-note", "Basis", Relevance::Info, vec![line("Plane contact FE (fea-core); friction capacity only (lock plates and teeth are not credited); a design guide, not a certification value.", Tone::Dim)]).for_steps(steps));
        cards
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let step = |s: &'static str| act(EccentricAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'B', format!("Eccentric bushing: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, r) in self.rows().into_iter().enumerate() {
            if let FieldRow::Header(h) = r {
                if h != "Run" {
                    out.push(TreeNode::new(10 + k as u32, 2, '#', h).on_click(step("Inputs")));
                }
            }
        }
        out.push(TreeNode::new(3, 2, 'R', "From the Bushing case").on_click(step("Inputs")));
        out.push(TreeNode::new(4, 1, '+', "Analysis").folder().selected(self.step == "Analysis").on_click(step("Analysis")));
        if let Some((_, a)) = &self.st.analysis {
            let bad = !a.wall_ok || (a.margin.is_finite() && a.margin < 0.0);
            let label = if a.margin.is_finite() { format!("{} {:+.0} %", if a.margin < 0.0 { "Spins" } else { "Holds" }, a.margin * 100.0) } else { "No spin torque".to_string() };
            out.push(TreeNode::new(5, 2, if bad { '!' } else { '^' }, label).tone(if bad { Tone::Warn } else { Tone::On }).on_click(step("Analysis")));
        }
        if let Some((_, l)) = &self.st.max_offset {
            out.push(TreeNode::new(6, 2, 'f', format!("Max offset {:.4} in", l.value)).on_click(step("Analysis")));
        }
        if let Some((_, l)) = &self.st.max_load {
            out.push(TreeNode::new(7, 2, 'f', if l.value.is_finite() { format!("Max load {:.0} lbf", l.value) } else { "Max load: any".to_string() }).on_click(step("Analysis")));
        }
        if self.st.sweep.is_some() {
            out.push(TreeNode::new(8, 2, 'f', "Margin sweep").on_click(step("Analysis")));
        }
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let row = |label: &str, r: FieldRow| entry(label, "", EccentricAct::Row(r));
        let ready = || -> Result<(), String> { model::build_input(&self.bushing, &self.st.ui).map(|_| ()) };
        let run = |label: &str, shortcut: &str, a: EccentricAct| entry(label, shortcut, a).when(ready());
        let result_shown = self.result_is_current();
        vec![
            RibbonTab::new("Inputs")
                .group("Eccentricity", vec![row("Offset", FieldRow::Number(NumberTarget::Offset)), row("Load angle", FieldRow::Number(NumberTarget::LoadAngle))])
                .group("Housing and pin", vec![row("Boss OD / bore", FieldRow::Number(NumberTarget::BossFactor)), row("Housing", FieldRow::ToggleHousing), row("Pin clearance", FieldRow::Number(NumberTarget::PinClearance)), row("Pin friction", FieldRow::Number(NumberTarget::PinFriction))])
                .group("Bushing case", vec![entry("Reload", "", EccentricAct::ReloadBushing), entry("Edit in the Bushing workbench", "", EccentricAct::OpenBushing)])
                .group("Edit", vec![entry("Undo", "Ctrl/Cmd+Z", EccentricAct::Undo).when(if self.undo.is_empty() { Err("nothing to undo".to_string()) } else { Ok(()) }), entry("Redo", "Ctrl/Cmd+Shift+Z", EccentricAct::Redo).when(if self.redo.is_empty() { Err("nothing to redo".to_string()) } else { Ok(()) }), entry("Save", "Ctrl/Cmd+S", EccentricAct::Save)]),
            RibbonTab::new("Analysis")
                .group(
                    "Run",
                    vec![
                        run("Analyse", "Ctrl/Cmd+Enter", EccentricAct::Analyse),
                        run("Maximum offset", "", EccentricAct::Row(FieldRow::Run(Task::MaxOffset))),
                        run("Maximum load", "", EccentricAct::Row(FieldRow::Run(Task::MaxLoad))),
                        run("Sweep", "", EccentricAct::Row(FieldRow::Run(Task::Sweep))),
                        entry("Cancel", "Esc", EccentricAct::Cancel).when(if self.st.job.is_some() { Ok(()) } else { Err("nothing is running".to_string()) }),
                    ],
                )
                .group(
                    "Show",
                    vec![
                        entry(if self.show_result { "Inputs picture" } else { "Solved model" }, "", EccentricAct::ToggleResult).when(if result_shown { Ok(()) } else { Err("analyse these inputs first".to_string()) }),
                        entry("Pressure profile", "", EccentricAct::ToggleProfile).checked(self.st.show_profile),
                    ],
                )
                .group("Export", vec![entry("Report", "", EccentricAct::ExportReport), entry("CSV", "", EccentricAct::ExportCsv), entry("FE fields (.vtu)", "", EccentricAct::ExportFields).when(ready())]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str, group: Option<&str>| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().filter(|g| group.is_none_or(|n| g.title == n)).flat_map(|g| g.buttons.clone()).map(Item::Entry).collect::<Vec<_>>()).unwrap_or_default();
        let save: Vec<Item> = all("Inputs", Some("Edit")).into_iter().filter(|i| matches!(i, Item::Entry(e) if e.label == "Save")).collect();
        let undo: Vec<Item> = all("Inputs", Some("Edit")).into_iter().filter(|i| matches!(i, Item::Entry(e) if e.label != "Save")).collect();
        vec![
            Menu { title: "File".into(), items: [save, all("Analysis", Some("Export"))].concat() },
            Menu { title: "Edit".into(), items: undo },
            Menu { title: "Analysis".into(), items: all("Analysis", None) },
        ]
    }
}
