//! The Pressure Vessel window's "Minimum wall by requirement" table and the supported line run: the page drawn over the
//! viewport while the line run is on, the table the Results step shows for the cylinder, and the inspector cards for the
//! requirement rows, the allowables and the line run's lists. Plain data from the model; every number is the solver's
//! (`PressureVesselModel::walls`, `line_result`).
use super::{VesselAct, VesselSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    page::{Cell, Column, DockTable, LineDrawing, Pill, TableRow},
    ui::DockItem,
};
use crate::theme::fmt_margin;
use crate::toolboxes::pressure_vessel::{
    edit::{Allowable, Edit, LineField, ReqField},
    walls::{WallOutcome, Walls},
};

fn act(a: VesselAct) -> UiAction {
    UiAction::Editor(EditorAction::Vessel(a))
}
fn trim(v: f64) -> String {
    crate::toolboxes::pressure_vessel::model::format_for_edit(v)
}
fn norm(v: [f64; 3]) -> f64 {
    v.iter().map(|c| c * c).sum::<f64>().sqrt()
}

/// The seeded rows say nothing about a program: the table tells the person so.
const NEUTRAL_HINT: &str = "Factors and required margins are yours: enter your program's factors and margins (rows are seeded 1.0 and 0.0, which asks only for the stress to stay inside the allowable).";

impl VesselSession {
    fn editing(&self, e: Edit) -> bool {
        self.prompt.as_ref().is_some_and(|p| p.edit == e)
    }
    /// A field's text, or what is being typed in it.
    fn field_text(&self, e: Edit, shown: String) -> String {
        self.prompt.as_ref().filter(|p| p.edit == e).map_or(shown, |p| format!("{}_", p.text))
    }
    /// A labelled row that types into `e` when clicked.
    fn edit_row(&self, label: &str, e: Edit, shown: String) -> DockItem {
        let editing = self.editing(e);
        DockItem::field(label, self.field_text(e, shown), if editing { Tone::Warn } else { Tone::Normal }, Some(act(VesselAct::Edit(e))), editing)
    }
    fn switch_row(&self, label: &str, shown: &str, a: VesselAct) -> DockItem {
        DockItem::field(label, shown, Tone::Normal, Some(act(a)), false)
    }
    fn text_cell(&self, e: Edit, shown: String) -> Cell {
        Cell::Text { text: self.field_text(e, shown), tone: Tone::Normal, action: Some(act(VesselAct::Edit(e))), editing: self.editing(e) }
    }

    // ------------------------------------------------------------------ the table

    /// The requirements with their minimum walls. `compact` (the inspector) leaves the editable columns to the Requirements
    /// card, where each field is its own row.
    pub(super) fn requirements_table(&self, compact: bool) -> DockItem {
        let m = &self.model;
        let walls: Option<&Walls> = match &m.walls {
            Some(Ok(w)) => Some(w),
            _ => None,
        };
        let governing = walls.and_then(Walls::governing);
        let scale = walls.map_or(0.0, |w| w.rows.iter().filter_map(|r| if let WallOutcome::Wall { wall, .. } = r.outcome { Some(wall) } else { None }).fold(w.current_wall, f64::max) * 1.15);
        let mut reasons = Vec::new();
        let rows: Vec<TableRow> = m
            .requirements
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let result = walls.and_then(|w| w.rows.get(i));
                let wall_cell = match (result.map(|x| &x.outcome), &m.walls) {
                    (Some(WallOutcome::Wall { wall, .. }), _) => Cell::Wall { text: format!("{wall:.4}"), value: *wall, current: walls.map_or(0.0, |w| w.current_wall), max: scale, tone: if governing == Some(i) { Tone::Title } else { Tone::Normal } },
                    (Some(WallOutcome::Infeasible { .. }), _) => Cell::Text { text: "no wall meets it".into(), tone: Tone::Warn, action: None, editing: false },
                    (Some(WallOutcome::NotEvaluable(why)), _) => {
                        reasons.push(format!("{}: {why}", r.label));
                        Cell::Text { text: "not evaluable".into(), tone: Tone::Dim, action: None, editing: false }
                    }
                    (None, Some(Err(_))) => Cell::Text { text: "-".into(), tone: Tone::Dim, action: None, editing: false },
                    (None, _) => Cell::Text { text: if m.line.enabled && m.walls.is_none() && m.geometry.is_ok() { "solving".into() } else { "-".into() }, tone: Tone::Dim, action: None, editing: false },
                };
                let margin_cell = match result.and_then(|x| x.margin_at_wall) {
                    Some(ms) => {
                        let met = ms >= r.required_ms;
                        Cell::Chip { text: format!("{} {}", if met { "met" } else { "short" }, fmt_margin(ms)), tone: if met { Tone::On } else { Tone::Warn } }
                    }
                    None => Cell::Chip { text: "-".into(), tone: Tone::Dim },
                };
                let cells = if compact {
                    vec![Cell::Text { text: r.label.clone(), tone: Tone::Normal, action: None, editing: false }, wall_cell, margin_cell]
                } else {
                    vec![
                        self.text_cell(Edit::Req(i, ReqField::Label), r.label.clone()),
                        Cell::Text { text: r.criterion.label().into(), tone: Tone::Dim, action: Some(act(VesselAct::CycleRequirement(i))), editing: false },
                        self.text_cell(Edit::Req(i, ReqField::Factor), trim(r.factor)),
                        self.text_cell(Edit::Req(i, ReqField::RequiredMs), trim(r.required_ms)),
                        wall_cell,
                        margin_cell,
                        Cell::Button { text: r.label.clone(), action: act(VesselAct::RemoveRequirement(i)) },
                    ]
                };
                TableRow { cells, governing: governing == Some(i) }
            })
            .collect();
        let columns = if compact {
            vec![Column::new("Requirement", 2.0), Column::new("Min wall (in)", 2.2), Column::new("Margin at wall", 2.0).right()]
        } else {
            vec![
                Column::new("Requirement", 2.4),
                Column::new("Criterion", 2.6).optional(),
                Column::new("Factor", 1.0).right().optional(),
                Column::new("Required MS", 1.5).right().optional(),
                Column::new("Minimum wall (in)", 3.2),
                Column::new("Margin at wall", 2.0),
                Column::new("", 0.5),
            ]
        };
        let (footer, footer_tone) = self.walls_footer(walls, governing);
        let mut hint = Vec::new();
        if m.requirements.iter().any(|r| r.is_neutral()) {
            hint.push(NEUTRAL_HINT.to_string());
        }
        if let Some(Err(e)) = &m.walls {
            hint.push(format!("Not solved: {e}"));
        }
        hint.extend(reasons);
        if m.line.enabled && m.kind() == crate::toolboxes::pressure_vessel::requirements::MaterialKind::Brittle && m.allowable_override[2].is_none() {
            hint.push("Ultimate compression is taken as the ultimate tension unless you enter it.".into());
        }
        DockItem::Table(Box::new(DockTable {
            columns,
            rows,
            add: (!compact).then(|| ("Add requirement".to_string(), act(VesselAct::AddRequirement))),
            footer,
            footer_tone,
            hint: hint.join("\n"),
        }))
    }

    fn walls_footer(&self, walls: Option<&Walls>, governing: Option<usize>) -> (String, Option<Tone>) {
        let m = &self.model;
        let (Some(w), Some(g)) = (walls, governing) else {
            return (if m.line.enabled && m.walls.is_none() && m.geometry.is_ok() { "Solving the minimum walls over every case...".into() } else { String::new() }, Some(Tone::Dim));
        };
        let label = &m.requirements[g].label;
        match w.rows[g].outcome {
            WallOutcome::Wall { wall, .. } => {
                let nominal = m.wall_allowance().map(|a| a.nominal_thickness(wall));
                let tail = nominal.map_or(String::new(), |n| format!(", nominal {n:.4} in with the allowances"));
                (format!("Governing wall {wall:.4} in remaining ({label}){tail}; yours is {:.4} in.", w.current_wall), Some(if w.current_wall >= wall { Tone::On } else { Tone::Warn }))
            }
            WallOutcome::Infeasible { best_margin } => (format!("No wall meets {label} (best margin {}).", fmt_margin(best_margin)), Some(Tone::Warn)),
            WallOutcome::NotEvaluable(_) => (String::new(), None),
        }
    }

    // ------------------------------------------------------------------ the line run's page

    /// The page over the viewport: only while the line run is on (the 3D vessel stays the picture otherwise).
    pub(super) fn page_items(&self) -> Vec<DockItem> {
        let m = &self.model;
        if !m.line.enabled {
            return Vec::new();
        }
        let mut header = Vec::new();
        match m.equilibrium() {
            Some(e) => {
                let ok = e.balanced();
                header.push(Pill::new(format!("{} Sum of reactions {:.1} lbf vs load {:.1} lbf", if ok { "Balanced:" } else { "Out of balance:" }, norm(e.reacted), norm(e.applied)), if ok { Tone::On } else { Tone::Warn }, None, false));
            }
            None => header.push(Pill::new("No solution yet", Tone::Dim, None, false)),
        }
        header.push(Pill::new("Export report", Tone::Normal, Some(act(VesselAct::Export)), false));
        header.push(Pill::new("Line run off", Tone::Dim, Some(act(VesselAct::ToggleLineRun)), false));
        let mut cases: Vec<Pill> = m.line.cases.iter().enumerate().map(|(i, c)| Pill::new(c.name.clone(), Tone::Normal, Some(act(VesselAct::SelectCase(i))), m.line.selected() == i)).collect();
        cases.push(Pill::new("+ Case", Tone::Normal, Some(act(VesselAct::AddCase)), false));
        let drawing = LineDrawing {
            length: m.line.length,
            supports: m.line.supports.iter().map(|s| (s.x, format!("{}", trim(s.x)))).collect(),
            loads: m
                .line
                .loads
                .iter()
                .map(|l| {
                    let f = norm(l.force);
                    (l.x, l.label.clone(), if l.weight != 0.0 { format!("{} lbf", trim(l.weight)) } else { format!("{f:.0} lbf") })
                })
                .collect(),
            curve: m.margin_curve(),
            worst: m.worst_point(),
            curve_label: format!("Margin over the requirement along the run, {}", m.line.cases.get(m.line.selected()).map_or("", |c| c.name.as_str())),
            note: match &m.line_result {
                Some(Err(e)) => e.clone(),
                None => "The vessel inputs make no tube.".into(),
                _ => String::new(),
            },
        };
        vec![
            DockItem::Pills(header),
            DockItem::Pills(cases),
            DockItem::Run(Box::new(drawing)),
            DockItem::Heading("Minimum wall by requirement".into()),
            self.requirements_table(false),
        ]
    }

    // ------------------------------------------------------------------ inspector cards

    /// The Results step's table card for the cylinder (the line run shows it on the page).
    pub(super) fn walls_card(&self) -> Option<Card> {
        let m = &self.model;
        if m.line.enabled || m.walls.is_none() {
            return None;
        }
        Some(Card::new("vessel-walls", "Minimum wall by requirement", Relevance::Primary, vec![self.requirements_table(true)]).for_steps(&["Results", "Report"]))
    }

    /// The requirement rows' own fields, the class and the allowables.
    pub(super) fn requirement_cards(&self) -> Vec<Card> {
        let m = &self.model;
        let mut rows = vec![self.switch_row("Material class", m.kind().label(), VesselAct::ToggleKind)];
        for (i, r) in m.requirements.iter().enumerate() {
            rows.push(DockItem::text(format!("{}", r.label), Tone::Title));
            rows.push(self.edit_row("Label", Edit::Req(i, ReqField::Label), r.label.clone()));
            rows.push(self.switch_row("Criterion", r.criterion.label(), VesselAct::CycleRequirement(i)));
            rows.push(self.edit_row("Factor", Edit::Req(i, ReqField::Factor), trim(r.factor)));
            rows.push(self.edit_row("Required MS", Edit::Req(i, ReqField::RequiredMs), trim(r.required_ms)));
            rows.push(DockItem::row("Remove requirement", Tone::Dim, Some(act(VesselAct::RemoveRequirement(i))), false));
        }
        rows.push(DockItem::row("+ Add requirement", Tone::Normal, Some(act(VesselAct::AddRequirement)), false));
        rows.push(DockItem::text(NEUTRAL_HINT, Tone::Dim));
        let e = m.effective_material();
        let allow = vec![
            self.edit_row("Yield strength (ksi)", Edit::Allowable(Allowable::Yield), trim(e.sy_ksi)),
            self.edit_row("Ultimate strength (ksi)", Edit::Allowable(Allowable::Ultimate), trim(e.ftu_ksi)),
            self.edit_row("Ultimate compression (ksi)", Edit::Allowable(Allowable::UltimateCompression), trim(m.compression_ultimate_ksi())),
            DockItem::text("Catalog values unless you type others; typing the catalog value again clears the override.", Tone::Dim),
        ];
        vec![
            Card::new("vessel-requirements", "Requirements", Relevance::Primary, rows).for_steps(&["Inputs"]),
            Card::new("vessel-allowables", "Allowables", Relevance::Context, allow).for_steps(&["Inputs"]).folded(),
        ]
    }

    /// The line run's switch and lists (Inputs), and its Tube, Governing and Reactions cards (Results).
    pub(super) fn line_cards(&self) -> Vec<Card> {
        let m = &self.model;
        let l = &m.line;
        let mut run = vec![self.switch_row("Supported line run", if l.enabled { "On" } else { "Off" }, VesselAct::ToggleLineRun)];
        if l.enabled {
            run.push(self.edit_row("Run length (in)", Edit::Line(LineField::Length), trim(l.length)));
            run.push(self.edit_row("Contents weight (lbf/in)", Edit::Line(LineField::Contents), trim(l.contents_weight)));
            run.push(self.edit_row("Tube weight density (lbf/in3)", Edit::Line(LineField::Density), trim(l.density)));
            if l.density == 0.0 {
                run.push(DockItem::text("Tube weight is zero until you enter a density; it is not assumed.", Tone::Dim));
            }
        } else {
            run.push(DockItem::text("Supports, point loads and load cases along the vessel, solved as a beam with the stress checks at every station.", Tone::Dim));
        }
        let mut cards = vec![Card::new("vessel-line-run", "Line run", Relevance::Primary, run).for_steps(&["Inputs"])];
        if !l.enabled {
            return cards;
        }
        let mut supports = Vec::new();
        for (i, s) in l.supports.iter().enumerate() {
            supports.push(self.edit_row(&format!("Support {} at x (in)", i + 1), Edit::Line(LineField::SupportX(i)), trim(s.x)));
            supports.push(self.switch_row("  Axial anchor", if s.axial_anchor { "Yes" } else { "No" }, VesselAct::ToggleSupport(i, false)));
            supports.push(self.switch_row("  Rotation fixed", if s.rotation_fixed { "Yes" } else { "No" }, VesselAct::ToggleSupport(i, true)));
            supports.push(DockItem::row("  Remove support", Tone::Dim, Some(act(VesselAct::RemoveSupport(i))), false));
        }
        supports.push(DockItem::row("+ Add support", Tone::Normal, Some(act(VesselAct::AddSupport)), false));
        cards.push(Card::new("vessel-supports", "Supports", Relevance::Primary, supports).for_steps(&["Inputs"]));
        let mut loads = Vec::new();
        for (i, ld) in l.loads.iter().enumerate() {
            loads.push(self.edit_row(&format!("Load {} label", i + 1), Edit::LoadLabel(i), ld.label.clone()));
            loads.push(self.edit_row("  At x (in)", Edit::Line(LineField::LoadX(i)), trim(ld.x)));
            for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                loads.push(self.edit_row(&format!("  Force {axis} (lbf)"), Edit::Line(LineField::LoadForce(i, k)), trim(ld.force[k])));
            }
            loads.push(self.edit_row("  Weight (lbf)", Edit::Line(LineField::LoadWeight(i)), trim(ld.weight)));
            loads.push(DockItem::row("  Remove load", Tone::Dim, Some(act(VesselAct::RemoveLoad(i))), false));
        }
        loads.push(DockItem::row("+ Add point load (a valve, a fitting)", Tone::Normal, Some(act(VesselAct::AddLoad)), false));
        cards.push(Card::new("vessel-loads", "Point loads", Relevance::Context, loads).for_steps(&["Inputs"]));
        let mut cs = Vec::new();
        for (i, c) in l.cases.iter().enumerate() {
            cs.push(self.edit_row(&format!("Case {} name", i + 1), Edit::CaseName(i), c.name.clone()));
            cs.push(self.edit_row("  Internal pressure (psi)", Edit::Line(LineField::CaseInternal(i)), trim(c.internal_pressure)));
            cs.push(self.edit_row("  External pressure (psi)", Edit::Line(LineField::CaseExternal(i)), trim(c.external_pressure)));
            cs.push(self.edit_row("  Temperature differential (F)", Edit::Line(LineField::CaseDeltaT(i)), trim(c.delta_t)));
            for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                cs.push(self.edit_row(&format!("  Load factor {axis}"), Edit::Line(LineField::CaseFactor(i, k)), trim(c.load_factors[k])));
            }
            cs.push(self.switch_row("  Ends", if c.closed_ends { "Closed" } else { "Open" }, VesselAct::ToggleCaseEnds(i)));
            if l.cases.len() > 1 {
                cs.push(DockItem::row("  Remove case", Tone::Dim, Some(act(VesselAct::RemoveCase(i))), false));
            }
        }
        cs.push(DockItem::row("+ Add case", Tone::Normal, Some(act(VesselAct::AddCase)), false));
        cs.push(DockItem::text("Load factors scale gravity, applied loads and contents along x, y, z (z down is -1).", Tone::Dim));
        cards.push(Card::new("vessel-cases", "Load cases", Relevance::Context, cs).for_steps(&["Inputs"]));

        // Results.
        if let Some(Ok(g)) = Some(&m.geometry) {
            let tube = vec![
                DockItem::field("Outer diameter", format!("{:.4} in", 2.0 * g.outer_radius), Tone::Normal, None, false),
                DockItem::field("Remaining wall", format!("{:.4} in", g.wall_thickness()), Tone::Normal, None, false),
                DockItem::field("Material", format!("{} ({})", m.material().name, m.kind().label()), Tone::Normal, None, false),
                DockItem::field("Run length", format!("{} in", trim(l.length)), Tone::Normal, None, false),
            ];
            cards.push(Card::new("vessel-tube", "Tube", Relevance::Primary, tube).for_steps(&["Results", "Report"]));
        }
        if let Some(Ok(w)) = &m.walls {
            if let Some(g) = w.governing() {
                let r = &m.requirements[g];
                let mut items = vec![DockItem::field("Requirement", r.label.clone(), Tone::Title, None, false)];
                match w.rows[g].outcome {
                    WallOutcome::Wall { wall, .. } => items.push(DockItem::field("Minimum wall", format!("{wall:.4} in"), if w.current_wall >= wall { Tone::On } else { Tone::Warn }, None, false)),
                    WallOutcome::Infeasible { best_margin } => items.push(DockItem::text(format!("No wall meets it (best margin {}).", fmt_margin(best_margin)), Tone::Warn)),
                    WallOutcome::NotEvaluable(_) => {}
                }
                if let Some(ms) = w.rows[g].margin_at_wall {
                    items.push(DockItem::field("Margin at your wall", fmt_margin(ms), if ms >= r.required_ms { Tone::On } else { Tone::Warn }, None, false));
                }
                if let Some(x) = m.worst_point() {
                    items.push(DockItem::field("Worst station", format!("x {:.2} in", x[0]), Tone::Normal, None, false));
                }
                cards.push(Card::new("vessel-governing", "Governing", Relevance::Primary, items).for_steps(&["Results", "Report"]));
            }
        }
        let mut react: Vec<DockItem> = m
            .reactions()
            .iter()
            .enumerate()
            .map(|(i, r)| DockItem::field(format!("Support {} at x {}", i + 1, trim(r.x)), format!("{:.1} / {:.1} / {:.1} lbf", r.force[0], r.force[1], r.force[2]), Tone::Normal, None, false))
            .collect();
        if let Some(e) = m.equilibrium() {
            react.push(DockItem::field("Sum of reactions", format!("{:.1} lbf", norm(e.reacted)), Tone::Normal, None, false));
            react.push(DockItem::field("Applied load", format!("{:.1} lbf", norm(e.applied)), if e.balanced() { Tone::On } else { Tone::Warn }, None, false));
            react.push(DockItem::text("Reactions are the force x / y / z the supports put on the run.", Tone::Dim));
        }
        if !react.is_empty() {
            cards.push(Card::new("vessel-reactions", "Reactions", Relevance::Primary, react).for_steps(&["Results", "Report"]));
        }
        cards
    }
}
