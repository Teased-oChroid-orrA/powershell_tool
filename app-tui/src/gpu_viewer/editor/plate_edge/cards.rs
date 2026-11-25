//! What the Plate Edge workbench shows: the three panes' cards (inputs, results, the plot under the view), the top bar's chips
//! and actions, the menus, the status line and the text report. Plain data built from the inputs and the last run; the window
//! draws it (`gui/workbench.rs`, `Look::Boxed`).
use super::inputs::{self, Field, FitKind, MeshMode, PinSource, Prop, Source};
use super::run::{FeLine, Kind, Outcome};
use super::{Choice, Dim, Layer, PlateEdgeAct, PlateEdgeSession, Pick, Setting, Target};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Authoring, Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    page::{Cell, Column, DockTable, TableRow},
    ui::{Compare, CompareRow, DockItem, Look, Mark, Plot, Series, Workbench, WorkbenchView},
};
use crate::toolboxes::fastener_hole::model::format_for_edit;
use edge_check::modes::ModesResult;

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
/// A load for a table cell: whole pounds with thousands grouped, one decimal under 100, "never" for an unreachable mode, no unit
/// (the head says lb).
pub(super) fn grp(v: f64) -> String {
    if v.is_infinite() {
        return "never".into();
    }
    let s = if v.abs() >= 100.0 { format!("{:.0}", v.abs()) } else { format!("{:.1}", v.abs()) };
    let (int, frac) = s.split_once('.').map_or((s.as_str(), ""), |(i, f)| (i, f));
    let mut g = String::new();
    for (k, c) in int.chars().enumerate() {
        if k > 0 && (int.len() - k) % 3 == 0 {
            g.push(',');
        }
        g.push(c);
    }
    format!("{}{g}{}", if v < 0.0 { "-" } else { "" }, if frac.is_empty() { String::new() } else { format!(".{frac}") })
}
fn cell(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), grp)
}
fn pct(v: f64) -> String {
    format!("{:.2} %", 100.0 * v)
}

impl PlateEdgeSession {
    // ------------------------------------------------------------------ the three panes

    pub(super) fn workbench_data(&self) -> Workbench {
        let busy = self.solving();
        let shown = self.solved();
        let on_geometry = !self.shows_result();
        let have_3d = self.outcome.as_ref().is_some_and(|o| o.lines.iter().any(|l| l.label.starts_with("3D") && l.fields.is_some()));
        let three = shown.is_some_and(|l| l.label.starts_with("3D")) || (shown.is_none() && self.dim == Dim::Three);
        let run = |label: &str, k: Kind| entry(label, "", PlateEdgeAct::Run(k)).when(if busy { Err("a run is going; cancel it first".to_string()) } else { Ok(()) });
        let mut actions = vec![entry("Save case", "Ctrl/Cmd+S", PlateEdgeAct::Save), entry("Report", "", PlateEdgeAct::Export)];
        if busy {
            actions.push(entry("Cancel", "Esc", PlateEdgeAct::Cancel));
        } else {
            actions.extend([run("Run 2D", Kind::TwoD), run("Run 2D + 3D", Kind::ThreeD)]);
        }
        let d = self.inputs.hole_diameter();
        let mut title = vec![shown.filter(|_| !on_geometry).map_or("Geometry", |l| l.label).to_string(), format!("e {:.3} in (e/D {:.2})", self.inputs.edge, self.inputs.edge / d)];
        if !on_geometry {
            title.push("plastic strain at 0.1 D travel".into());
        }
        let mut cards_under = Vec::new();
        if let Some(o) = self.outcome.as_ref().filter(|o| !o.lines.is_empty()) {
            cards_under.push(self.plot_card(o));
        }
        Workbench {
            chips: self.chips(),
            actions,
            inputs: self.input_cards(),
            inputs_note: "US customary inputs only. A value is typed: click it, Enter applies, Esc cancels.".into(),
            views: vec![
                entry("2D plane", "", PlateEdgeAct::View(Dim::Two)).checked(!three),
                entry("3D solid", "", PlateEdgeAct::View(Dim::Three)).checked(three).when(if have_3d { Ok(()) } else { Err("no 3D solid yet: Run 2D + 3D".to_string()) }),
            ],
            show: WorkbenchView::Model,
            fields: vec![
                entry("Geometry", "", PlateEdgeAct::Layer(Layer::Geometry)).checked(on_geometry),
                entry("Plastic strain", "", PlateEdgeAct::Layer(Layer::Strain)).checked(!on_geometry).when(if shown.is_some() { Ok(()) } else { Err("no solved model for these inputs: run the FE".to_string()) }),
            ],
            switches: Vec::new(),
            view_note: String::new(),
            model_title: title.join(" \u{b7} "),
            model_hint: String::new(),
            sketches: Vec::new(),
            results: self.result_cards(),
            dialog: None,
            look: Look::Boxed,
            under: cards_under,
            view_status: self.view_status(),
        }
    }

    /// The solved model the centre view would draw for the current inputs (`dim` first, else the other family).
    fn solved(&self) -> Option<&FeLine> {
        self.outcome.as_ref().filter(|_| self.current()).and_then(|o| super::scene::pick_fields(o, self.dim))
    }

    /// The units, the verdict and the advisory mark in the top bar.
    fn chips(&self) -> Vec<(String, Tone)> {
        let mut chips = vec![("in \u{b7} lbf \u{b7} ksi".to_string(), Tone::Normal)];
        chips.push(match self.outcome.as_ref().filter(|_| self.current()) {
            None => (if self.job.is_some() { "Computing..." } else { "Not computed" }.to_string(), if self.job.is_some() { Tone::Dim } else { Tone::Warn }),
            Some(o) => match o.modes.as_ref().map(|m| m.governing.map(|g| &m.rows[g])) {
                Ok(Some(r)) => (format!("Governing: {} {}", r.label, opt(r.lowest())), Tone::On),
                Ok(None) => ("No mode has a number".to_string(), Tone::Warn),
                Err(_) => ("Not computed".to_string(), Tone::Warn),
            },
        });
        chips.push(("advisory".to_string(), Tone::Dim));
        chips
    }

    /// What the last FE run did, or what is going on now: the line between the view and the plot.
    fn view_status(&self) -> Option<(String, Tone)> {
        if let Some(j) = self.job.as_ref().filter(|j| j.kind != Kind::Quick) {
            let last = j.progress.last().map(|(l, r)| format!(" \u{b7} {l}: {} around{}, collapse {}", r.around, if r.layers > 1 { format!(", {} layers", r.layers) } else { String::new() }, lb(r.collapse))).unwrap_or_default();
            return Some((format!("running {:.0} s{last}", j.started.elapsed().as_secs_f32()), Tone::Warn));
        }
        let Some(o) = &self.outcome else { return Some(("Computing the allowables...".into(), Tone::Dim)) };
        if o.lines.is_empty() {
            return Some(match &o.fe_note {
                Some(note) => (format!("FE not run: {note}."), Tone::Warn),
                None => ("Allowables only: Run 2D or Run 2D + 3D for the FE bearing response.".into(), Tone::Dim),
            });
        }
        if !self.current() {
            return Some(("Inputs changed since this run; run it again.".into(), Tone::Warn));
        }
        let per: Vec<String> = o
            .lines
            .iter()
            .map(|l| {
                let mesh = match (l.meshes.len(), l.converged) {
                    (1, _) => "one mesh".to_string(),
                    (n, true) => format!("{n} meshes, converged"),
                    (n, false) => format!("{n} meshes, not converged"),
                };
                format!("{}: {mesh}, {} dof, {:.0} s", short_title(l.label), grp(l.last().dofs as f64), l.last().secs)
            })
            .collect();
        let converged = o.lines.iter().any(|l| l.meshes.len() > 1) && o.lines.iter().all(|l| l.converged);
        Some((per.join(" \u{b7} "), if converged { Tone::On } else { Tone::Dim }))
    }

    // ------------------------------------------------------------------ inputs

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
    /// A read-only figure the inputs give (the hole diameter, e/D).
    fn derived(items: &mut Vec<DockItem>, label: &str, value: impl Into<String>, tone: Tone) {
        items.push(DockItem::field(label, value, tone, None, false));
    }
    /// One choice of a few as a segmented switch across the card.
    fn switch(items: &mut Vec<DockItem>, options: &[(&str, Setting, bool)]) {
        items.push(DockItem::Segmented(options.iter().map(|(label, s, on)| entry(label, "", PlateEdgeAct::Set(*s)).checked(*on)).collect()));
    }
    /// A library material row: a click types part of its name; Enter picks the first match, Enter again the next.
    fn material_row(&self, items: &mut Vec<DockItem>, label: &str, m: &mechanics_core::materials::Material, pick: Pick) {
        let editing = self.editing(Target::Search(pick));
        let text = editing.map_or_else(|| m.name.to_string(), |p| format!("{}_", p.text));
        items.push(DockItem::field(label, text, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(PlateEdgeAct::Search(pick))), editing.is_some()));
        if editing.is_some() {
            items.push(line("Type part of an alloy, temper or id (7075, 4340, T651), Enter picks the first match, Enter again the next.", Tone::Dim));
        } else {
            items.push(line(inputs::describe(m), Tone::Dim));
        }
    }

    pub(super) fn input_cards(&self) -> Vec<Card> {
        vec![self.plate_card(), self.pin_card(), self.fit_card(), self.geometry_card(), self.mesh_card()]
    }

    fn plate_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        Self::switch(&mut items, &[("Library", Setting::Plate(Source::Library), i.plate_source == Source::Library), ("Custom", Setting::Plate(Source::Custom), i.plate_source == Source::Custom)]);
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
        Card::new("plate-edge-plate", "Plate material", Relevance::Primary, items)
    }

    fn pin_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        let at = |s: PinSource| i.pin_source == s;
        Self::switch(&mut items, &[("Class", Setting::Pin(PinSource::Class), at(PinSource::Class)), ("Material", Setting::Pin(PinSource::Material), at(PinSource::Material)), ("Custom", Setting::Pin(PinSource::Custom), at(PinSource::Custom))]);
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
        Card::new("plate-edge-pin", "Fastener / pin", Relevance::Primary, items)
    }

    fn fit_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        let at = |k: FitKind| i.fit == k;
        Self::switch(&mut items, &[("Clearance", Setting::Fit(FitKind::Clearance), at(FitKind::Clearance)), ("Press-fit", Setting::Fit(FitKind::PressFit), at(FitKind::PressFit)), ("Interference", Setting::Fit(FitKind::Interference), at(FitKind::Interference)), ("Pressure", Setting::Fit(FitKind::Custom), at(FitKind::Custom))]);
        Self::derived(&mut items, "Hole diameter D", format!("{:.4} in", i.hole_diameter()), Tone::Normal);
        let value = Field::FitValue;
        let editing = self.editing(Target::Field(value));
        let text = editing.map_or_else(|| format!("{}{}", format_for_edit(i.fit_value), i.fit.unit()), |p| format!("{}_", p.text));
        items.push(DockItem::field(i.fit.value_label(), text, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(PlateEdgeAct::Field(value))), editing.is_some()));
        if editing.is_some() {
            items.push(line(value.hint(), Tone::Dim));
        }
        items.push(line(
            match i.fit {
                FitKind::Clearance => "The hole is larger than the pin by the clearance. The FE bracket (2D and 3D) models this fit: a rigid pin, no contact pressure.",
                FitKind::PressFit => "Light interference: the hole is smaller than the pin. The fit's pressure is charged against shear-out and tear-out. The FE bracket models the pin as an elastic body pressed into the hole, then pushed toward the edge.",
                FitKind::Interference => "Heavy interference: the hole is smaller than the pin. The fit's pressure is charged against shear-out and tear-out. The FE bracket models the pin as an elastic body pressed into the hole, then pushed toward the edge.",
                FitKind::Custom => "Only the contact pressure is known: the hole equals the pin. The FE bracket skips it (it imposes a diametral interference, which a pressure does not give).",
            },
            Tone::Dim,
        ));
        if let Some(Ok(m)) = self.outcome.as_ref().filter(|_| self.current()).map(|o| &o.modes) {
            if m.fit_pressure > 0.0 {
                items.push(line(format!("Contact pressure {:.0} psi (charged against shear-out and tear-out at 100 %)", m.fit_pressure), Tone::Dim));
            }
        }
        Card::new("plate-edge-fit", "Fit", Relevance::Primary, items)
    }

    fn geometry_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        self.number(&mut items, Field::Edge);
        self.number(&mut items, Field::Thickness);
        let d = i.hole_diameter();
        let (ed, td) = (i.edge / d, i.thickness / d);
        Self::derived(&mut items, if ed < 1.5 { "! e / D" } else { "e / D" }, format!("{ed:.2}"), if ed < 1.5 { Tone::Warn } else { Tone::Normal });
        Self::derived(&mut items, "t / D", format!("{td:.2}"), Tone::Normal);
        self.number(&mut items, Field::Friction);
        Self::derived(&mut items, "Load direction", "to free edge", Tone::Normal);
        if ed < 1.5 {
            items.push(line("e/D under 1.5: the handbook bearing tables are read at the actual e/D and extrapolate poorly there.", Tone::Warn));
        }
        Card::new("plate-edge-geometry", "Plate & load", Relevance::Primary, items)
    }

    fn mesh_card(&self) -> Card {
        let i = &self.inputs;
        let mut items = Vec::new();
        let at = |m: MeshMode| i.mesh == m;
        Self::switch(&mut items, &[("Graded", Setting::Mesh(MeshMode::Graded), at(MeshMode::Graded)), ("Converge", Setting::Mesh(MeshMode::Converge), at(MeshMode::Converge)), ("Fixed fine", Setting::Mesh(MeshMode::Fine), at(MeshMode::Fine))]);
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
        Card::new("plate-edge-mesh", "Mesh", Relevance::Context, items)
    }

    // ------------------------------------------------------------------ results

    pub(super) fn result_cards(&self) -> Vec<Card> {
        let Some(o) = &self.outcome else {
            return vec![Card::new("plate-edge-allowables", "Allowable loads", Relevance::Info, vec![line("Not computed yet: enter a valid case.", Tone::Dim)])];
        };
        let mut cards = Vec::new();
        let stale = !self.current();
        match &o.modes {
            Err(why) => cards.push(Card::new("plate-edge-allowables", "Allowable loads", Relevance::Info, vec![line(format!("Not computed: {why}."), Tone::Warn)])),
            Ok(m) => {
                cards.push(self.allowables_card(o, m, stale));
                cards.push(self.shear_card(o, m));
                cards.push(self.check_card(o, m));
            }
        }
        if !o.lines.is_empty() {
            cards.push(self.convergence_card(o));
        }
        if o.kind != Kind::Quick || !o.lines.is_empty() {
            cards.push(self.fe_card(o, stale));
        }
        cards.push(self.notes_card());
        cards
    }

    /// Every failure mode with the lowest load any method gives, then the FE's load for the modes it also reaches: plane strain
    /// (the 2D column) and the 3D solid. The FE bracket's own loads are the 2 % offset yield (the bearing yield row) and the
    /// collapse at 0.1 D (the collapse row); no other row has an FE number, and none is made up.
    fn allowables_card(&self, o: &Outcome, m: &ModesResult, stale: bool) -> Card {
        let fe = |label: &str, id: &str| -> Option<f64> {
            let l = o.lines.iter().find(|l| l.label == label)?;
            match id {
                "bearing_onset" => l.last().onset,
                "collapse" => Some(l.last().collapse),
                _ => None,
            }
        };
        let eligible: Vec<f64> = m.rows.iter().filter(|r| r.governs).filter_map(|r| r.lowest()).filter(|v| v.is_finite()).collect();
        let most = eligible.iter().copied().fold(0.0, f64::max);
        let rows = m
            .rows
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let names = if r.id == "shear_out" { ["handbook 40\u{b0} planes", "elastic field, derived plane", "contact FE"] } else { ["handbook", "elastic field", "contact FE"] };
                let methods: Vec<String> = [(names[0], r.handbook), (names[1], r.elastic), (names[2], r.contact)].into_iter().filter_map(|(n, v)| v.map(|v| format!("{n} {}", grp(v)))).collect();
                let governing = m.governing == Some(k);
                CompareRow {
                    title: r.label.to_string(),
                    detail: r.why.clone().unwrap_or_else(|| methods.join(" \u{b7} ")),
                    values: if r.id == "shear_out" { vec![cell(r.lowest()), shear_fe_cell(o, "2D plane strain"), shear_fe_cell(o, "3D solid")] } else { vec![cell(r.lowest()), cell(fe("2D plane strain", r.id)), cell(fe("3D solid", r.id))] },
                    governing,
                    bar: r.lowest().filter(|v| governing && v.is_finite() && most > 0.0).map(|v| (v / most) as f32),
                    chip: None,
                    dim: !r.governs,
                }
            })
            .collect();
        let mut footer = "lb. Allowable is the lowest of the handbook, the elastic field and the contact FE. 2D FE is plane strain.".to_string();
        if !o.lines.is_empty() {
            footer.push_str(" FE loads: bearing yield at 2 % D offset, collapse at 0.1 D travel.");
        }
        let mut items = Vec::new();
        if stale {
            items.push(line("Inputs changed since this result; it is being recomputed.", Tone::Warn));
        }
        items.push(DockItem::Compare(Box::new(Compare { heads: vec!["Mode".into(), "Allowable".into(), "2D FE".into(), "3D FE".into()], rows, footer })));
        for n in &m.notes {
            items.push(line(n.clone(), Tone::Dim));
        }
        let (tag, tone) = match m.governing.map(|g| &m.rows[g]) {
            Some(r) => (format!("governing: {}", r.label), Tone::On),
            None => ("incomplete".to_string(), Tone::Warn),
        };
        Card::new("plate-edge-allowables", "Allowable loads", Relevance::Info, items).tag(tag).tag_tone(tone)
    }

    /// The shear-out plane three ways: the handbook's assumed 40 degree planes, the plane of least capacity found on the elastic
    /// field, and the ridge of plastic strain of each FE line with the load at which the mean shear along it reaches `Fsu`.
    fn shear_card(&self, o: &Outcome, m: &ModesResult) -> Card {
        let fsu = o.inputs.plate_strengths().0.fsu;
        let angles = |start: f64, incline: f64| format!("{start:.0} / {incline:+.0}\u{b0}");
        let mut rows = Vec::new();
        if let Some(h) = m.rows.iter().find(|r| r.id == "shear_out").and_then(|r| r.handbook) {
            rows.push(CompareRow {
                title: "Handbook (assumed)".into(),
                detail: "two planes parallel to the load, leaving the hole at 40\u{b0}: 2 Fsu t (e - a cos 40\u{b0}) less the fit's p D t. An assumption, kept as the handbook column.".into(),
                values: vec![angles(40.0, 0.0), grp(h)],
                dim: true,
                ..CompareRow::default()
            });
        }
        match &m.shear_scan {
            Ok(s) => {
                let a = 0.5 * o.inputs.hole_diameter();
                let pinned = s.best.edge_y < 0.05 * a;
                rows.push(CompareRow {
                    title: "Derived from elastic field".into(),
                    detail: format!(
                        "plane of least capacity over where it leaves the wall and its inclination, mean shear along it = Fsu; {:.3} in long, ends {:.3} in from the load line at the free edge{}. Same field on the 40\u{b0} parallel planes: {}; on the tangent planes: {}.",
                        s.best.length,
                        s.best.edge_y,
                        if pinned { " (the planes of both halves meet there: the family's limit, a plate cannot shear out along planes that cross)" } else { "" },
                        grp(s.classical.capacity),
                        grp(s.tangent.capacity)
                    ),
                    values: vec![angles(s.best.start_deg, s.best.incline_deg), grp(s.best.capacity)],
                    governing: true,
                    ..CompareRow::default()
                });
            }
            Err(why) => rows.push(CompareRow { title: "Derived from elastic field".into(), detail: format!("not derived: {why}"), values: vec!["-".into(), "-".into()], dim: true, ..CompareRow::default() }),
        }
        for l in &o.lines {
            let title = format!("Derived from FE, {}", short_title(l.label));
            match &l.last().shear {
                Ok(p) => {
                    let reached = p.load_at(fsu);
                    rows.push(CompareRow {
                        title,
                        detail: format!(
                            "ridge of equivalent plastic strain (peak {:.3}), {:.3} in long; mean shear along it {:.0} psi = {:.0} % of Fsu at 0.1 D travel{}",
                            p.peak_plastic,
                            p.length,
                            p.mean_at_end(),
                            100.0 * p.mean_at_end() / fsu,
                            if reached.is_some() { "" } else { ": not reached, bearing governs the FE" }
                        ),
                        values: vec![angles(p.start_deg, p.incline_deg), reached.map_or_else(|| "not reached".into(), grp)],
                        ..CompareRow::default()
                    });
                }
                Err(why) => rows.push(CompareRow { title, detail: format!("no path: {why}"), values: vec!["-".into(), "-".into()], dim: true, ..CompareRow::default() }),
            }
        }
        let footer = "Angles: where the plane leaves the hole wall, from the load line at the hole centre / its inclination to the load line (+ opens toward the edge). Load: pin load, lb, at which the mean shear along the plane reaches Fsu. The elastic figure is a full-redistribution criterion (lowest over the planes); the FE path is read from the solve and its stress state at every step. Advisory.";
        Card::new("plate-edge-shear", "Shear-out plane", Relevance::Info, vec![DockItem::Compare(Box::new(Compare { heads: vec!["Plane".into(), "Angle".into(), "Load".into()], rows, footer: footer.into() }))])
    }

    /// The legacy handbook figure beside what the FE says: advisory, nothing promoted to the governing check.
    fn check_card(&self, o: &Outcome, m: &ModesResult) -> Card {
        let ultimate = m.rows.iter().find(|r| r.id == "bearing_ultimate").and_then(|r| r.handbook);
        let mut rows = vec![CompareRow {
            title: "Handbook bearing ultimate".into(),
            detail: "Fbru at this e/D times D t. The legacy Fbru + 0.8 p check stays governing; nothing here is promoted to it.".into(),
            values: vec![ultimate.map_or_else(|| "-".into(), |v| format!("{} lb", grp(v)))],
            ..CompareRow::default()
        }];
        let get = |label: &str| o.lines.iter().find(|l| l.label == label).map(|l| l.last().collapse);
        if let (Some(strain), Some(solid)) = (get("2D plane strain"), get("3D solid")) {
            rows.push(CompareRow {
                title: "3D vs plane strain".into(),
                detail: "Spread of the two FE collapse loads. Plane strain brackets the 3D answer from above, plane stress from below.".into(),
                values: vec![format!("{:+.1} %", 100.0 * (solid / strain - 1.0))],
                dim: true,
                ..CompareRow::default()
            });
        }
        let mut footer = "Advisory: the FE and the allowables do not replace the legacy handbook check.".to_string();
        if m.fit_pressure > 0.0 {
            footer = format!("Fit contact pressure {:.0} psi, charged against shear-out and tear-out at 100 %. {footer}", m.fit_pressure);
        }
        Card::new("plate-edge-check", "Check against the handbook rule", Relevance::Info, vec![DockItem::Compare(Box::new(Compare { heads: vec!["Check".into(), "Value".into()], rows, footer }))])
    }

    /// Every mesh of every FE line: its collapse load and how much it moved from the previous mesh. One mesh (the graded
    /// mode) has no change to show; the card says so.
    fn convergence_card(&self, o: &Outcome) -> Card {
        let mut rows = Vec::new();
        for l in &o.lines {
            for r in &l.meshes {
                rows.push(CompareRow {
                    title: format!("{} \u{b7} {} x {}", l.label, r.around, r.layers),
                    detail: format!("{} dof \u{b7} {:.1} s{}", grp(r.dofs as f64), r.secs, if r.collapse_change.is_none() { " \u{b7} first mesh" } else { "" }),
                    values: vec![grp(r.collapse), r.collapse_change.map_or_else(|| "-".to_string(), pct)],
                    ..CompareRow::default()
                });
            }
        }
        let multi = o.lines.iter().any(|l| l.meshes.len() > 1);
        let all_converged = o.lines.iter().all(|l| l.converged);
        let (footer, tag, tone) = if !multi {
            ("One mesh per model: no convergence check. Choose Mesh: Converge to repeat on finer meshes.", "single mesh", Tone::Dim)
        } else if all_converged {
            ("Converged: the last change of every model is under the tolerance.", "converged", Tone::On)
        } else {
            ("Not converged within the meshes allowed: the last change of a model is above the tolerance.", "not converged", Tone::Warn)
        };
        let compare = Compare { heads: vec!["Mesh".into(), "Collapse, lb".into(), "Change".into()], rows, footer: footer.into() };
        Card::new("plate-edge-convergence", "Mesh convergence", Relevance::Info, vec![DockItem::Compare(Box::new(compare))]).tag(tag).tag_tone(tone)
    }

    /// The FE lines' loads side by side, the contact FE beside them. The load-travel plot is under the view.
    fn fe_card(&self, o: &Outcome, stale: bool) -> Card {
        let mut items = Vec::new();
        if stale {
            items.push(line("Inputs changed since this run; run it again.", Tone::Warn));
        }
        if let Some(note) = &o.fe_note {
            items.push(line(format!("{}: {note}.", if o.lines.is_empty() { "FE not run" } else { "FE incomplete" }), Tone::Warn));
        }
        for l in &o.lines {
            items.push(DockItem::field(fe_name(l.label), format!("collapse {}", lb(l.last().collapse)), Tone::Normal, None, false));
        }
        if !o.lines.is_empty() {
            items.push(DockItem::Table(Box::new(line_table(o))));
            items.push(line("Pin load toward the edge. Onset: where the load-travel line has lost 5 % of its initial stiffness. Yield: its 2 % D offset. Collapse: the load at 0.1 D of pin travel. Dots on the curves: onset. The housing is elastic-perfectly-plastic, so yield and collapse sit close together: the table's bearing-onset row is the handbook Fbry, not this.", Tone::Dim));
            items.push(line(FE_MODELS_NOTE, Tone::Dim));
        }
        if let Some(c2) = o.modes.as_ref().ok().and_then(|m| m.rows.iter().find(|r| r.id == "collapse")).and_then(|r| r.contact) {
            items.push(DockItem::field("Collapse, edge-check contact FE (plane strain)", lb(c2), Tone::Normal, None, false));
            if let Some(b) = o.lines.iter().find(|l| l.label == "2D plane strain") {
                items.push(line(format!("The FE bracket's plane strain collapse is {:+.1} % against it.", 100.0 * (b.last().collapse / c2 - 1.0)), Tone::Dim));
            }
        }
        Card::new("plate-edge-fe", "FE bearing response", Relevance::Info, items).tag(if o.lines.iter().any(|l| l.label.starts_with("3D")) { "2D + 3D" } else { "2D" })
    }

    /// The load-travel plot under the view.
    fn plot_card(&self, o: &Outcome) -> Card {
        let tag = if o.lines.iter().any(|l| l.label.starts_with("3D")) { "2D and 3D overlay" } else { "2D" };
        Card::new("plate-edge-plot", "Load vs. pin travel", Relevance::Info, vec![curve_plot(o)]).tag(tag)
    }

    fn notes_card(&self) -> Card {
        let items = vec![
            line("Plane strain runs above the 3D model at e/D 1.5 to 2.0 (12 to 36 % in the recorded bracket) and plane stress below it: read the 2D numbers as a bracket, and the 3D as the better single estimate for a bare pin.", Tone::Dim),
            line("The 2D and 3D FE model a rigid pin in a clearance or line-to-line hole, or an elastic pin (a ring of the pin's modulus around a rigid core, as edge-check models it) pressed into an interference hole by frictional contact and then pushed toward the edge. A custom contact pressure is not modelled by the FE.", Tone::Dim),
            line("Handbook rows read the table values of the chosen material at the actual e/D; a row with no table value says why.", Tone::Dim),
            line("The FE bracket computes the mesh, the displacement and the plastic strain only: von Mises, shear and bearing-pressure fields are not available. The load is along the free-edge direction; plate width and an inclined load are not inputs of these calculations.", Tone::Dim),
        ];
        Card::new("plate-edge-notes", "Limits", Relevance::Info, items)
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
                    Some(l) => format!("{gov}. FE {} collapse {}.", fe_name(l.label), lb(l.last().collapse)),
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
                        match &m.shear_scan {
                            Ok(s) => t += &format!("Shear-out plane derived from the elastic field: leaves the wall at {:.1} deg, inclined {:+.1} deg to the load line, {:.3} in long, ends {:.3} in from the load line at the free edge; capacity {} (the handbook's 40 deg parallel planes on the same field: {}; tangent planes: {})\n", s.best.start_deg, s.best.incline_deg, s.best.length, s.best.edge_y, opt(Some(s.best.capacity)), opt(Some(s.classical.capacity)), opt(Some(s.tangent.capacity))),
                            Err(why) => t += &format!("Shear-out plane not derived: {why}\n"),
                        }
                        for n in &m.notes {
                            t += &format!("Note: {n}\n");
                        }
                    }
                }
                if let Some(n) = &o.fe_note {
                    t += &format!("\n{}: {n}.\n", if o.lines.is_empty() { "FE not run" } else { "FE incomplete" });
                }
                for l in &o.lines {
                    t += &format!("\n{}\n", fe_name(l.label));
                    for r in &l.meshes {
                        t += &format!("  {} around, {} layers, {} dof: onset {}, yield {}, collapse {}, {:.1} s\n", r.around, r.layers, r.dofs, opt(r.departure), opt(r.onset), lb(r.collapse), r.secs);
                    }
                    t += &format!("  {}\n", if l.meshes.len() > 1 { if l.converged { "converged" } else { "not converged" } } else { "one mesh" });
                    let fsu = o.inputs.plate_strengths().0.fsu;
                    match &l.last().shear {
                        Ok(p) => t += &format!("  shear-out path from the plastic strain: leaves the wall at {:.1} deg, inclined {:+.1} deg; mean shear at 0.1 D {:.0} psi = {:.0} % of Fsu; {}\n", p.start_deg, p.incline_deg, p.mean_at_end(), 100.0 * p.mean_at_end() / fsu, p.load_at(fsu).map_or_else(|| "Fsu not reached".to_string(), |v| format!("reaches Fsu at {} lbf", opt(Some(v))))),
                        Err(why) => t += &format!("  no shear-out path: {why}\n"),
                    }
                }
            }
        }
        t += "\nAdvisory: the legacy handbook check stays governing.\n";
        t
    }

    // ------------------------------------------------------------------ menus

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let busy = self.solving();
        let never = |what: &str| Err(format!("nothing to {what}"));
        let run = |label: &str, k: Kind| entry(label, "", PlateEdgeAct::Run(k)).when(if busy { Err("a run is going; cancel it first".to_string()) } else { Ok(()) });
        vec![
            Menu { title: "File".into(), items: vec![Item::Entry(entry("Save case", "Ctrl/Cmd+S", PlateEdgeAct::Save)), Item::Entry(entry("Export report", "", PlateEdgeAct::Export))] },
            Menu {
                title: "Edit".into(),
                items: vec![
                    Item::Entry(entry("Undo", "Ctrl/Cmd+Z", PlateEdgeAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) })),
                    Item::Entry(entry("Redo", "Ctrl/Cmd+Shift+Z", PlateEdgeAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) })),
                ],
            },
            Menu { title: "Run".into(), items: vec![Item::Entry(run("Run 2D", Kind::TwoD)), Item::Entry(run("Run 2D + 3D", Kind::ThreeD)), Item::Entry(entry("Cancel", "Esc", PlateEdgeAct::Cancel).when(if busy { Ok(()) } else { never("cancel") }))] },
        ]
    }
}

/// The FE line's figure for the shear-out row: the pin load at which the mean shear along its plastic-strain path reaches `Fsu`,
/// or how far the shear got when the travel ended, or nothing when the solve showed no path.
fn shear_fe_cell(o: &Outcome, label: &str) -> String {
    let Some(l) = o.lines.iter().find(|l| l.label == label) else { return "-".into() };
    let fsu = o.inputs.plate_strengths().0.fsu;
    match &l.last().shear {
        Ok(p) => p.load_at(fsu).map_or_else(|| format!("{:.0} % Fsu", 100.0 * p.mean_at_end() / fsu), grp),
        Err(_) => "-".into(),
    }
}

/// What the FE lines are and how they differ from the contact FE row of the allowables table.
const FE_MODELS_NOTE: &str = "FE bracket: lug-solver on the general kernel, a rigid pin, the plate's flow stress, friction, load read at 0.1 D. The contact FE row of the allowables is edge-check's own independent plane strain solver; the two share the physics but not the mesh or the contact algorithm, so they differ by their discretisation.";

fn text(t: impl Into<String>, tone: Tone) -> Cell {
    Cell::Text { text: t.into(), tone, action: None, editing: false }
}

/// The plot colour of an FE line: three different palette colours (accent, bad, good: the amber, salmon and olive of the warm theme are the furthest apart).
fn line_color(label: &str) -> u8 {
    match label {
        "2D plane stress" => 4,
        "3D solid" => 2,
        _ => 0,
    }
}

/// A FE line's column title: "2D strain", "2D stress", "3D solid".
fn short_title(label: &str) -> String {
    label.replace(" plane", "")
}

/// A FE line's name where both FE solvers appear: "2D plane strain (FE bracket)".
fn fe_name(label: &str) -> String {
    format!("{label} (FE bracket)")
}

/// Pin travel (in D) at which a curve first reaches `load`, by linear interpolation.
fn travel_at(curve: &[(f64, f64)], load: f64) -> Option<f64> {
    let mut prev = (0.0, 0.0);
    for &(t, p) in curve {
        if p >= load {
            let w = if p > prev.1 { (load - prev.1) / (p - prev.1) } else { 1.0 };
            return Some(prev.0 + w * (t - prev.0));
        }
        prev = (t, p);
    }
    None
}

/// Load against pin travel of every FE line (last mesh), the onset of yielding marked on each, and the contact FE's collapse
/// where the allowables have it.
fn curve_plot(o: &Outcome) -> DockItem {
    let mut plot = Plot { x_label: "Pin travel / D".into(), y_label: "Load, lbf".into(), ..Plot::default() };
    for l in &o.lines {
        let r = l.last();
        let color = line_color(l.label);
        plot.series.push(Series { label: l.label.to_string(), points: r.curve.iter().map(|&(t, p)| [t, p]).collect(), color, dashed: l.label.starts_with("2D plane stress") });
        // The onset of yielding as a dot in the line's colour; the key below names the lines, so the dots carry no label.
        if let Some(at) = r.departure.and_then(|d| travel_at(&r.curve, d).map(|t| [t, d])) {
            plot.marks.push(Mark { at, text: String::new(), color });
        }
    }
    if let Some(c) = o.modes.as_ref().ok().and_then(|m| m.rows.iter().find(|r| r.id == "collapse")).and_then(|r| r.contact) {
        plot.marks.push(Mark { at: [lug_solver::bracket::D_OVER_D, c], text: "edge-check contact FE".into(), color: 1 });
    }
    DockItem::Plot(plot)
}

/// The FE lines side by side: one column per idealisation, the three loads, the size and the spread against plane strain.
fn line_table(o: &Outcome) -> DockTable {
    let mut columns = vec![Column::new("", 2.4)];
    columns.extend(o.lines.iter().map(|l| Column::new(&short_title(l.label), 1.2).right()));
    let per = |f: &dyn Fn(&FeLine) -> String| -> Vec<Cell> { o.lines.iter().map(|l| text(f(l), Tone::Normal)).collect() };
    let row = |title: &str, tone: Tone, cells: Vec<Cell>, governing: bool| {
        let mut all = vec![text(title, tone)];
        all.extend(cells);
        TableRow { cells: all, governing }
    };
    let base = o.lines.iter().find(|l| l.label == "2D plane strain").map(|l| l.last().collapse);
    let mut rows = vec![
        row("Onset (5 % loss)", Tone::Normal, per(&|l| opt(l.last().departure)), false),
        row("Yield (2 % D)", Tone::Normal, per(&|l| opt(l.last().onset)), false),
        row("Collapse (0.1 D)", Tone::Title, per(&|l| lb(l.last().collapse)), true),
    ];
    if let Some(b) = base.filter(|_| o.lines.len() > 1) {
        rows.push(row("vs plane strain", Tone::Dim, per(&|l| if l.label == "2D plane strain" { "-".into() } else { format!("{:+.1} %", 100.0 * (l.last().collapse / b - 1.0)) }), false));
    }
    rows.push(row("Mesh", Tone::Dim, per(&|l| format!("{} x {}", l.last().around, l.last().layers)), false));
    rows.push(row("Dof", Tone::Dim, per(&|l| l.last().dofs.to_string()), false));
    rows.push(row("Time", Tone::Dim, per(&|l| format!("{:.1} s", l.last().secs)), false));
    DockTable { columns, rows, add: None, footer: String::new(), footer_tone: None, hint: String::new() }
}
