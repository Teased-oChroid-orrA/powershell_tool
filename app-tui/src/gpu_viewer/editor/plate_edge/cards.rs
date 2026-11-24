//! What the Plate Edge workbench shows: the inspector's cards, the tree, the ribbon, the menus, the status line and the text
//! report. Plain data built from the inputs and the last run; the window draws it.
use super::inputs::{self, Field, FitKind, MeshMode, PinSource, Prop, Source, PIN_CLASSES};
use super::run::{FeLine, Kind, Outcome};
use super::{Choice, PlateEdgeAct, PlateEdgeSession, Pick, Target};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::toolboxes::fastener_hole::model::format_for_edit;
use edge_check::modes::{Group, ModeRow, ModesResult};

fn act(a: PlateEdgeAct) -> UiAction {
    UiAction::Editor(EditorAction::PlateEdge(a))
}
fn entry(label: &str, shortcut: &str, a: PlateEdgeAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn line(text: impl Into<String>, tone: Tone) -> DockItem {
    DockItem::text(text, tone)
}

/// A load in lbf: whole pounds, "never" for an unreachable mode, "no" for a failure at any load.
fn lb(v: f64) -> String {
    if v.is_infinite() {
        "not reached".into()
    } else if v >= 100.0 {
        format!("{v:.0} lb")
    } else {
        format!("{v:.1} lb")
    }
}
fn opt(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), lb)
}
/// `s` cut to `n` characters with an ellipsis (the inspector clips a long value at its edge).
fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(n - 3).collect::<String>())
    }
}
fn pct(v: f64) -> String {
    format!("{:.2} %", 100.0 * v)
}

impl PlateEdgeSession {
    fn editing(&self, t: Target) -> Option<&super::Prompt> {
        self.prompt.as_ref().filter(|p| p.target == t)
    }

    /// A numeric input row; while it is being typed its hint follows.
    fn number(&self, items: &mut Vec<DockItem>, f: Field) {
        let editing = self.editing(Target::Field(f));
        let text = editing.map_or_else(|| format!("{}{}", format_for_edit(self.inputs.get(f)), f.unit()), |p| format!("{}_", p.text));
        items.push(DockItem::field(f.label(), text, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(PlateEdgeAct::Field(f))), editing.is_some()));
        if editing.is_some() {
            items.push(line(f.hint(), Tone::Dim));
            items.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
        }
    }
    fn choice(&self, items: &mut Vec<DockItem>, label: &str, value: impl Into<String>, c: Choice) {
        items.push(DockItem::field(label, value, Tone::Normal, Some(act(PlateEdgeAct::Cycle(c))), false));
    }
    /// A library material row: a click types part of its name; Enter picks the first match, Enter again the next.
    fn material_row(&self, items: &mut Vec<DockItem>, label: &str, m: &mechanics_core::materials::Material, pick: Pick) {
        let editing = self.editing(Target::Search(pick));
        let text = editing.map_or_else(|| short(m.name, 26), |p| format!("{}_", p.text));
        items.push(DockItem::field(label, text, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(PlateEdgeAct::Search(pick))), editing.is_some()));
        if editing.is_some() {
            items.push(line("Type part of an alloy, temper or id (7075, 4340, T651), Enter picks the first match, Enter again the next.", Tone::Dim));
        } else {
            items.push(line(inputs::describe(m), Tone::Dim));
        }
    }

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = vec![self.plate_card(), self.pin_card(), self.fit_card(), self.geometry_card(), self.mesh_card(), self.run_card(), self.summary_card()];
        cards.extend(self.result_cards());
        cards
    }

    fn plate_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        self.choice(&mut items, "Source", i.plate_source.label(), Choice::PlateSource);
        match i.plate_source {
            Source::Library => {
                let m = i.plate_library();
                self.material_row(&mut items, "Material", m, Pick::Plate);
                let (s, fbry, fbry15) = i.plate_strengths();
                items.push(line(format!("E {:.0} ksi  Fty {:.0}  Ftu {:.0}  Fsu {:.0}  Fbru {:.0}  Fbry {}", s.e / 1e3, s.sy / 1e3, s.ftu / 1e3, s.fsu / 1e3, s.fbru / 1e3, if fbry > 0.0 { format!("{:.0}", fbry / 1e3) } else { "not tabulated".into() }), Tone::Dim));
                if fbry15 > 0.0 || s.fbru_e15 > 0.0 {
                    items.push(line(format!("At e/D 1.5: Fbru {:.0}, Fbry {:.0} ksi", s.fbru_e15 / 1e3, fbry15 / 1e3), Tone::Dim));
                }
            }
            Source::Custom => Prop::ALL.iter().for_each(|&p| self.number(&mut items, Field::Plate(p))),
        }
        Card::new("plate-edge-plate", "Plate material", Relevance::Primary, items).for_steps(&["Inputs"]).tag(i.plate_source.label())
    }

    fn pin_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        self.choice(&mut items, "Source", i.pin_source.label(), Choice::PinSource);
        match i.pin_source {
            PinSource::Class => {
                self.choice(&mut items, "Fastener class", i.pin_class_spec().label, Choice::PinClass);
                self.choice(&mut items, "Nominal size", i.size_entry().0, Choice::Size);
                items.push(line("Specification ratings: tension and double shear only. Pin bending and bearing need a yield and a bearing strength: pick a pin material or type them.", Tone::Dim));
            }
            PinSource::Material => {
                self.choice(&mut items, "Nominal size", i.size_entry().0, Choice::Size);
                self.material_row(&mut items, "Pin material", i.pin_library(), Pick::Pin);
            }
            PinSource::Custom => {
                self.number(&mut items, Field::PinDia);
                for p in [Prop::E, Prop::Nu, Prop::Ftu, Prop::Fsu, Prop::Fty, Prop::Fbru] {
                    self.number(&mut items, Field::Pin(p));
                }
            }
        }
        self.number(&mut items, Field::Planes);
        self.number(&mut items, Field::Span);
        let s = i.pin_spec();
        items.push(line(format!("Pin diameter {:.4} in  Ftu {:.0}  Fsu {:.0} ksi", s.dia, s.ftu / 1e3, s.fsu / 1e3), Tone::Dim));
        Card::new("plate-edge-pin", "Fastener / pin", Relevance::Primary, items).for_steps(&["Inputs"]).tag(i.pin_source.label())
    }

    fn fit_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        self.choice(&mut items, "Fit", i.fit.label(), Choice::Fit);
        let value = Field::FitValue;
        let editing = self.editing(Target::Field(value));
        let text = editing.map_or_else(|| format!("{}{}", format_for_edit(i.fit_value), i.fit.unit()), |p| format!("{}_", p.text));
        items.push(DockItem::field(i.fit.value_label(), text, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(PlateEdgeAct::Field(value))), editing.is_some()));
        if editing.is_some() {
            items.push(line(value.hint(), Tone::Dim));
        }
        self.number(&mut items, Field::Friction);
        items.push(line(format!("Hole {:.4} in  ({})", i.hole_diameter(), match i.fit {
            FitKind::Clearance => "larger than the pin".to_string(),
            FitKind::Custom => "equal to the pin; the pressure is given".to_string(),
            _ => "smaller than the pin".to_string(),
        }), Tone::Dim));
        if let Some(Ok(m)) = self.outcome.as_ref().filter(|_| self.current()).map(|o| &o.modes) {
            if m.fit_pressure > 0.0 {
                items.push(line(format!("Contact pressure {:.0} psi (charged against shear-out and tear-out at 100 %)", m.fit_pressure), Tone::Dim));
            }
        }
        Card::new("plate-edge-fit", "Fit", Relevance::Primary, items).for_steps(&["Inputs"]).tag(i.fit.label())
    }

    fn geometry_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        self.number(&mut items, Field::Edge);
        self.number(&mut items, Field::Thickness);
        let d = i.hole_diameter();
        items.push(line(format!("e/D {:.2}   t/D {:.2}", i.edge / d, i.thickness / d), if i.edge / d < 1.5 { Tone::Warn } else { Tone::Dim }));
        Card::new("plate-edge-geometry", "Plate and edge", Relevance::Primary, items).for_steps(&["Inputs"])
    }

    fn mesh_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        self.choice(&mut items, "Mesh", i.mesh.label(), Choice::Mesh);
        self.number(&mut items, Field::Around);
        self.number(&mut items, Field::Layers);
        if i.mesh == MeshMode::Converge {
            self.number(&mut items, Field::Tolerance);
        }
        items.push(line(match i.mesh {
            MeshMode::Graded => "One mesh, fine at the hole wall and coarse away from it. The first 3D mesh is a few minutes.",
            MeshMode::Converge => "Repeats the FE on finer meshes until the collapse load changes by less than the tolerance, then stops (2D up to 4 meshes, 3D up to 3).",
            MeshMode::Fine => "One fixed fine mesh (72 around, 4 layers) as a reference. Slow in 3D.",
        }, Tone::Dim));
        Card::new("plate-edge-mesh", "Mesh", Relevance::Context, items).for_steps(&["Inputs"]).folded()
    }

    fn run_card(&self) -> Card {
        let mut items = Vec::new();
        match &self.job {
            Some(j) if j.kind != Kind::Quick => {
                items.push(line(format!("running ({:.0} s)", j.started.elapsed().as_secs_f32()), Tone::Warn));
                for (label, r) in &j.progress {
                    items.push(line(format!("{label}: {} around, {} layers: collapse {}", r.around, r.layers, lb(r.collapse)), Tone::Dim));
                }
                items.push(DockItem::row("Cancel", Tone::Warn, Some(act(PlateEdgeAct::Cancel)), false));
            }
            _ => {
                items.push(DockItem::row("Run 2D (contact FE and 2D bracket, seconds)", Tone::Title, Some(act(PlateEdgeAct::Run(Kind::TwoD))), false));
                items.push(DockItem::row("Run 2D + 3D (minutes)", Tone::Title, Some(act(PlateEdgeAct::Run(Kind::ThreeD))), false));
                if let Some(why) = self.inputs.bracket_refusal() {
                    items.push(line(format!("The FE bracket is skipped: {why}."), Tone::Warn));
                }
            }
        }
        let tag = match (&self.job, &self.outcome) {
            (Some(j), _) if j.kind != Kind::Quick => "running",
            (_, None) => "not run",
            (_, Some(o)) if !self.current() => if o.kind == Kind::Quick { "stale" } else { "stale" },
            (_, Some(o)) => match o.kind {
                Kind::Quick => "allowables only",
                Kind::TwoD => "2D done",
                Kind::ThreeD => "2D + 3D done",
            },
        };
        Card::new("plate-edge-run", "Run", Relevance::Primary, items).for_steps(&["Inputs", "Results"]).tag(tag)
    }

    // ------------------------------------------------------------------ results

    /// The answer in one glance on the Inputs step: the governing mode and each FE collapse load (the full tables are on Results).
    fn summary_card(&self) -> Card {
        let mut items = Vec::new();
        match self.outcome.as_ref().filter(|_| self.current()) {
            None => items.push(line(if self.job.is_some() { "Computing..." } else { "Not computed yet." }, Tone::Dim)),
            Some(o) => {
                match o.modes.as_ref().ok().and_then(|m| m.governing.map(|g| &m.rows[g])) {
                    Some(r) => {
                        items.push(DockItem::field("Governing", r.label, Tone::On, None, false));
                        items.push(DockItem::field("Allowable load", opt(r.lowest()), Tone::On, None, false));
                    }
                    None => items.push(line("No mode has a number: see Results.", Tone::Warn)),
                }
                for l in &o.lines {
                    items.push(DockItem::field(l.label, format!("collapse {}", lb(l.last().collapse)), Tone::Normal, None, false));
                }
                if let Some(note) = &o.fe_note {
                    items.push(line(format!("FE not run: {note}."), Tone::Warn));
                }
                items.push(line("Tables and the FE detail are on the Results step.", Tone::Dim));
            }
        }
        Card::new("plate-edge-summary", "Answer", Relevance::Info, items).for_steps(&["Inputs"])
    }

    fn result_cards(&self) -> Vec<Card> {
        let Some(o) = &self.outcome else {
            return vec![Card::new("plate-edge-allowables", "Allowable loads", Relevance::Info, vec![line("Not computed yet: enter a valid case.", Tone::Dim)]).for_steps(&["Results"])];
        };
        let mut cards = Vec::new();
        let stale = !self.current();
        match &o.modes {
            Err(why) => cards.push(Card::new("plate-edge-allowables", "Allowable loads", Relevance::Info, vec![line(format!("Not computed: {why}."), Tone::Warn)]).for_steps(&["Results"])),
            Ok(m) => cards.push(self.allowables_card(m, stale)),
        }
        if o.kind != Kind::Quick || !o.lines.is_empty() {
            cards.push(self.fe_card(o, stale));
        }
        if o.lines.iter().any(|l| l.meshes.len() > 1) {
            cards.push(self.convergence_card(o));
        }
        cards.push(self.notes_card());
        cards
    }

    fn allowables_card(&self, m: &ModesResult, stale: bool) -> Card {
        let mut items = Vec::new();
        if stale {
            items.push(line("Inputs changed since this result; it is being recomputed.", Tone::Warn));
        }
        match m.governing.map(|g| &m.rows[g]) {
            Some(r) => {
                items.push(DockItem::field("Governing", r.label, Tone::On, None, false));
                items.push(DockItem::field("Allowable load", opt(r.lowest()), Tone::On, None, false));
            }
            None => items.push(line("No mode has a number: see the reasons below.", Tone::Warn)),
        }
        for group in [Group::Plate, Group::Pin] {
            items.push(line(if group == Group::Plate { "Plate" } else { "Pin / fastener" }, Tone::Title));
            for (k, r) in m.rows.iter().enumerate().filter(|(_, r)| r.group == group) {
                items.extend(row_items(r, m.governing == Some(k)));
            }
        }
        for n in &m.notes {
            items.push(line(n.clone(), Tone::Dim));
        }
        Card::new("plate-edge-allowables", "Allowable loads", Relevance::Info, items).for_steps(&["Results"]).tag(if m.governing.is_some() { "advisory" } else { "incomplete" })
    }

    fn fe_card(&self, o: &Outcome, stale: bool) -> Card {
        let mut items = Vec::new();
        if stale {
            items.push(line("Inputs changed since this run; run it again.", Tone::Warn));
        }
        if let Some(note) = &o.fe_note {
            items.push(line(format!("FE not run: {note}."), Tone::Warn));
        }
        for l in &o.lines {
            items.extend(fe_items(l));
        }
        if !o.lines.is_empty() {
            items.push(line("Pin load toward the edge. Onset: where the load-travel line has lost 5 % of its initial stiffness. Yield: its 2 % D offset. Collapse: the load at 0.1 D of pin travel. The housing is elastic-perfectly-plastic, so yield and collapse sit close together: the table's bearing-onset row is the handbook Fbry, not this.", Tone::Dim));
        }
        if let Some(c2) = o.modes.as_ref().ok().and_then(|m| m.rows.iter().find(|r| r.id == "collapse")).and_then(|r| r.contact) {
            items.push(DockItem::field("Contact FE collapse (plane strain, advisory)", lb(c2), Tone::Normal, None, false));
        }
        Card::new("plate-edge-fe", "FE bearing response", Relevance::Info, items).for_steps(&["Results"]).tag(if o.lines.iter().any(|l| l.label.starts_with("3D")) { "2D + 3D" } else { "2D" })
    }

    fn convergence_card(&self, o: &Outcome) -> Card {
        let mut items = Vec::new();
        for l in o.lines.iter().filter(|l| l.meshes.len() > 1) {
            items.push(line(l.label, Tone::Title));
            for r in &l.meshes {
                items.push(DockItem::field(
                    format!("{} around, {} layers", r.around, r.layers),
                    format!("{}  {}", lb(r.collapse), r.collapse_change.map_or_else(|| "first mesh".to_string(), |c| format!("change {}", pct(c)))),
                    Tone::Normal,
                    None,
                    false,
                ));
            }
            items.push(line(if l.converged { "Converged: the last change is under the tolerance." } else { "Not converged within the meshes allowed: the last change is above the tolerance." }, if l.converged { Tone::On } else { Tone::Warn }));
        }
        Card::new("plate-edge-convergence", "Mesh convergence", Relevance::Info, items).for_steps(&["Results"])
    }

    fn notes_card(&self) -> Card {
        let items = vec![
            line("Advisory. The legacy handbook check (Fbru + 0.8 p in the Bushing workbench) stays the governing check; nothing here is promoted to it.", Tone::Dim),
            line("Plane strain runs above the 3D model at e/D 1.5 to 2.0 (12 to 36 % in the recorded bracket) and plane stress below it: read the 2D numbers as a bracket, and the 3D as the better single estimate for a bare pin.", Tone::Dim),
            line("The 3D and 2D FE model a rigid pin in a clearance or line-to-line hole. An interference fit is in the 2D contact FE; its 3D collapse did not converge in trials.", Tone::Dim),
            line("Handbook rows read the table values of the chosen material at the actual e/D; a row with no table value says why.", Tone::Dim),
        ];
        Card::new("plate-edge-notes", "Limits", Relevance::Info, items).for_steps(&["Results"]).folded()
    }

    // ------------------------------------------------------------------ status and report

    /// One line for the status bar.
    pub(super) fn verdict(&self) -> String {
        let Some(o) = self.outcome.as_ref().filter(|_| self.current()) else {
            return if self.job.is_some() { "Computing the allowables...".into() } else { "Not computed.".into() };
        };
        match &o.modes {
            Err(why) => format!("Not computed: {why}."),
            Ok(m) => {
                let gov = m.governing.map(|g| &m.rows[g]).map_or_else(|| "no mode has a number".to_string(), |r| format!("governing {}: {}", r.label.to_lowercase(), opt(r.lowest())));
                match o.lines.iter().find(|l| l.label.starts_with("3D")).or_else(|| o.lines.first()) {
                    Some(l) => format!("{gov}. FE {} collapse {}.", l.label, lb(l.last().collapse)),
                    None => format!("{gov}."),
                }
            }
        }
    }

    /// The report text written beside the case file.
    pub(super) fn report_text(&self) -> String {
        let i = &self.inputs;
        let mut t = format!("Plate edge: {}\n\n", self.name);
        t += &format!("Plate: {} ({})\n", match i.plate_source { Source::Library => i.plate_library().name.to_string(), Source::Custom => "custom".into() }, i.plate_source.label());
        t += &format!("Pin: dia {:.4} in, {}\n", i.pin_diameter(), i.pin_source.label());
        t += &format!("Fit: {} {}{}; hole {:.4} in; friction {}\n", i.fit.label(), format_for_edit(i.fit_value), i.fit.unit(), i.hole_diameter(), i.friction);
        t += &format!("Edge {:.4} in (e/D {:.2}), thickness {:.4} in\n\n", i.edge, i.edge / i.hole_diameter(), i.thickness);
        match self.outcome.as_ref().filter(|_| self.current()) {
            None => t += "No current result: run the case first.\n",
            Some(o) => {
                match &o.modes {
                    Err(why) => t += &format!("Allowables not computed: {why}.\n"),
                    Ok(m) => {
                        t += "Allowable loads (lbf, the load at which each mode is reached)\n";
                        for r in &m.rows {
                            t += &format!("  {:<30} lowest {:>12}  handbook {:>12}  elastic {:>12}  contact FE {:>12}", r.label, opt(r.lowest()), opt(r.handbook), opt(r.elastic), opt(r.contact));
                            if let Some(why) = &r.why {
                                t += &format!("  [{why}]");
                            }
                            t.push('\n');
                        }
                        if let Some(g) = m.governing {
                            t += &format!("Governing: {} {}\n", m.rows[g].label, opt(m.rows[g].lowest()));
                        }
                        for n in &m.notes {
                            t += &format!("Note: {n}\n");
                        }
                    }
                }
                if let Some(n) = &o.fe_note {
                    t += &format!("\nFE not run: {n}.\n");
                }
                for l in &o.lines {
                    t += &format!("\n{}\n", l.label);
                    for r in &l.meshes {
                        t += &format!("  {} around, {} layers, {} dof: onset {}, yield {}, collapse {}, {:.1} s\n", r.around, r.layers, r.dofs, opt(r.departure), opt(r.onset), lb(r.collapse), r.secs);
                    }
                    t += &format!("  {}\n", if l.meshes.len() > 1 { if l.converged { "converged" } else { "not converged" } } else { "one mesh" });
                }
            }
        }
        t += "\nAdvisory: the legacy handbook check stays governing.\n";
        t
    }

    // ------------------------------------------------------------------ tree, ribbon, menus

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let step = |s: &'static str| act(PlateEdgeAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'P', format!("Plate edge: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, label) in ["Plate material", "Fastener / pin", "Fit", "Plate and edge", "Mesh"].into_iter().enumerate() {
            out.push(TreeNode::new(10 + k as u32, 2, '#', label).on_click(step("Inputs")));
        }
        out.push(TreeNode::new(3, 1, '+', "Results").folder().selected(self.step == "Results").on_click(step("Results")));
        if let Some(Ok(m)) = self.outcome.as_ref().filter(|_| self.current()).map(|o| &o.modes) {
            for (k, r) in m.rows.iter().enumerate() {
                let gov = m.governing == Some(k);
                out.push(TreeNode::new(30 + k as u32, 2, if gov { '!' } else { '^' }, format!("{}: {}", r.label, opt(r.lowest()))).tone(if gov { Tone::Warn } else { Tone::Normal }).on_click(step("Results")));
            }
        }
        out
    }

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let never = |what: &str| Err(format!("nothing to {what}"));
        let busy = self.job.as_ref().is_some_and(|j| j.kind != Kind::Quick);
        let run = |label: &str, k: Kind| entry(label, "", PlateEdgeAct::Run(k)).when(if busy { Err("a run is going; cancel it first".to_string()) } else { Ok(()) });
        vec![
            RibbonTab::new("Inputs")
                .group("Case", vec![entry("Fit type", "", PlateEdgeAct::Cycle(Choice::Fit)), entry("Mesh mode", "", PlateEdgeAct::Cycle(Choice::Mesh)), entry("Plate source", "", PlateEdgeAct::Cycle(Choice::PlateSource)), entry("Pin source", "", PlateEdgeAct::Cycle(Choice::PinSource))])
                .group(
                    "Edit",
                    vec![
                        entry("Undo", "Ctrl/Cmd+Z", PlateEdgeAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) }),
                        entry("Redo", "Ctrl/Cmd+Shift+Z", PlateEdgeAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) }),
                        entry("Save", "Ctrl/Cmd+S", PlateEdgeAct::Save),
                    ],
                ),
            RibbonTab::new("Results")
                .group("Run", vec![run("Run 2D", Kind::TwoD), run("Run 2D + 3D", Kind::ThreeD), entry("Cancel", "Esc", PlateEdgeAct::Cancel).when(if busy { Ok(()) } else { never("cancel") })])
                .group("Report", vec![entry("Export report", "", PlateEdgeAct::Export), entry("Save", "Ctrl/Cmd+S", PlateEdgeAct::Save)]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let pick = |tab: &str, labels: &[&str]| all(tab).into_iter().filter(|e| labels.contains(&e.label.as_str())).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: pick("Results", &["Save", "Export report"]) },
            Menu { title: "Edit".into(), items: pick("Inputs", &["Undo", "Redo"]) },
            Menu { title: "Run".into(), items: pick("Results", &["Run 2D", "Run 2D + 3D", "Cancel"]) },
        ]
    }
}

/// A mode's rows: the lowest load as the field, then how each method reached its number (or why there is none).
fn row_items(r: &ModeRow, governing: bool) -> Vec<DockItem> {
    let tone = if governing { Tone::On } else { Tone::Normal };
    let mut items = vec![DockItem::field(r.label, opt(r.lowest()), tone, None, false)];
    let methods: Vec<String> = [("handbook", r.handbook), ("elastic field", r.elastic), ("contact FE", r.contact)].into_iter().filter_map(|(n, v)| v.map(|v| format!("{n} {}", lb(v)))).collect();
    if !methods.is_empty() {
        items.push(line(methods.join("  |  "), Tone::Dim));
    }
    if let Some(why) = &r.why {
        items.push(line(why.clone(), Tone::Warn));
    }
    items
}

/// One FE idealisation: its last mesh's three loads and size.
fn fe_items(l: &FeLine) -> Vec<DockItem> {
    let r = l.last();
    vec![
        DockItem::field(l.label, format!("collapse {}", lb(r.collapse)), Tone::Normal, None, false),
        line(format!("onset {}  |  yield (2 % D) {}  |  {} dof, {:.1} s", opt(r.departure), opt(r.onset), r.dofs, r.secs), Tone::Dim),
    ]
}

#[allow(dead_code)]
fn pin_classes() -> usize {
    PIN_CLASSES.len()
}
