//! What the Preload Analysis workbench shows: the inspector's cards, the tree, the ribbon and the menus. Plain data built
//! from the model; the window draws it. Row labels, values, margins, the verdict and their tones are the terminal
//! toolbox's (`model::row_label`, `NumberTarget::label`, `report::{display_value, status_label, margins, passes}`,
//! `theme::{fmt_margin, tone_for_margin}`); every number comes from `fastened-joint-solver` or the member FE.
use super::{drawing, PreloadAct, PreloadSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Authoring, Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use crate::theme::{fmt_margin, tone_for_margin, StatusTone};
use crate::toolboxes::preload_analysis::{
    joint_templates::stack_diagram,
    model::{self, FieldRow, MAX_MEMBERS},
    report::{display_value, margins, passes, status_label},
};
use fastened_joint_solver::solve::{JointSolution, SolverStatus, StressSection};

fn act(a: PreloadAct) -> UiAction {
    UiAction::Editor(EditorAction::Preload(a))
}
fn entry(label: &str, shortcut: &str, a: PreloadAct) -> Entry {
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
/// The window's tone for a solver or margin tone: a pass reads on, a failure or a thin margin warns.
fn tone(t: StatusTone) -> Tone {
    match t {
        StatusTone::Success => Tone::On,
        StatusTone::Warning | StatusTone::Danger => Tone::Warn,
        StatusTone::Neutral | StatusTone::Info => Tone::Normal,
    }
}
fn margin(label: impl Into<String>, m: f64) -> DockItem {
    DockItem::field(label, fmt_margin(m), tone(tone_for_margin(m)), None, false)
}
fn ok_tone(ok: bool) -> Tone {
    if ok {
        Tone::On
    } else {
        Tone::Warn
    }
}

/// An input row's label in the window: the form's (`row_label`), or the number's own name.
fn row_label(row: FieldRow) -> String {
    match row {
        FieldRow::Number(t) => t.label(),
        r => model::row_label(r).to_string(),
    }
}

impl PreloadSession {
    /// The joint solves, converges and every margin holds (`report::passes`).
    pub(super) fn all_pass(&self) -> bool {
        self.model.output.as_ref().is_ok_and(passes)
    }

    /// One line for the status bar: the verdict, the preload and the smallest margin, or why there is none.
    pub(super) fn verdict(&self) -> String {
        match &self.model.output {
            Err(e) => format!("Invalid input: {e:?}."),
            Ok(s) => {
                let least = margins(s).into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).map(|(n, m)| format!("; {n} governs, MS {}", fmt_margin(m))).unwrap_or_default();
                format!("{}: preload {:.0} lbf from {:.1} in-lbf, {}{least}.", if self.all_pass() { "PASS" } else { "REVIEW" }, s.preload, s.torque.applied, status_label(s.status).0.to_lowercase())
            }
        }
    }

    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let mut cards = self.input_cards();
        cards.push(self.stack_card());
        cards.push(self.result_card());
        if let Ok(s) = &self.model.output {
            cards.extend(self.solution_cards(s));
        }
        cards.push(self.fe_card());
        cards.push(self.drawing_card());
        cards
    }

    /// The window's text for an input row: the model's, with its key hints worded for a click.
    fn row_value(&self, row: FieldRow) -> String {
        let n = self.model.members.len();
        match row {
            FieldRow::OpenTemplatePicker => format!("{n} member{}", if n == 1 { "" } else { "s" }),
            FieldRow::ToggleAddMember => format!("Add ({n}/{MAX_MEMBERS})"),
            FieldRow::ToggleRemoveMember => "Remove".into(),
            FieldRow::AdvancedSection => if self.model.advanced_open { "Hide" } else { "Show" }.into(),
            r => display_value(&self.model, r),
        }
    }

    /// One card per section of the input rows (`field_rows`), in the form's own order; the Advanced row starts its
    /// own card and its sections follow it.
    fn input_cards(&self) -> Vec<Card> {
        let mut sections: Vec<(String, Vec<DockItem>)> = Vec::new();
        for row in model::field_rows(&self.model) {
            match row {
                FieldRow::Header(h) => {
                    sections.push((h.to_string(), Vec::new()));
                    continue;
                }
                FieldRow::AdvancedSection => sections.push((model::row_label(row).to_string(), Vec::new())),
                _ => {}
            }
            let editing = self.prompt.as_ref().filter(|p| p.row == row);
            let text = editing.map_or_else(|| self.row_value(row), |p| format!("{}_", p.text));
            // The window draws a Warn value as an edit box: only the row being typed is one.
            let tone = if editing.is_some() { Tone::Warn } else { Tone::Normal };
            let mut items = vec![DockItem::field(row_label(row), text, tone, Some(act(PreloadAct::Row(row))), editing.is_some())];
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
                Card::new(format!("preload-{}", slug(&title)), title, Relevance::Primary, items).for_steps(&["Inputs"])
            })
            .collect()
    }

    /// The clamped stack from head to nut (as captions).
    fn stack_card(&self) -> Card {
        let rows = stack_diagram(&self.model);
        let last = rows.len().saturating_sub(1);
        let items = rows.into_iter().enumerate().map(|(k, r)| line(r.caption, if k == 0 || k == last { Tone::Dim } else { Tone::Normal })).collect();
        Card::new("preload-joint", "Joint, head to nut", Relevance::Context, items).for_steps(&["Inputs"])
    }

    /// The preload, the torque that makes it, the solver's status and the verdict: shown at every step.
    fn result_card(&self) -> Card {
        let m = &self.model;
        let items = match &m.output {
            Err(e) => vec![line(format!("Invalid input: {e:?}."), Tone::Warn), line("Every result needs a joint the solver accepts: check the members, bearing radii and thread.", Tone::Dim)],
            Ok(s) => {
                let (status, status_tone) = status_label(s.status);
                let all = margins(s);
                let held = all.iter().filter(|(_, m)| *m >= 0.0).count();
                let mut items = vec![
                    line(
                        match all.len() {
                            // No strength limit, service load or slip check is set: the verdict is the solve alone.
                            0 => format!("{}  no margins checked", if self.all_pass() { "PASS" } else { "REVIEW" }),
                            n => format!("{}  {held} / {n} margins hold", if self.all_pass() { "PASS" } else { "REVIEW" }),
                        },
                        ok_tone(self.all_pass()),
                    ),
                    DockItem::field("Preload", format!("{:.0} lbf", s.preload), Tone::Title, None, false),
                    value("Applied torque", format!("{:.1} in-lbf", s.torque.applied)),
                    DockItem::field("Solver", status, tone(status_tone), None, false),
                ];
                if s.status != SolverStatus::Converged {
                    items.push(value("Torque residual", format!("{:.4} in-lbf", s.torque.residual)));
                }
                items.push(value("Load fraction C", format!("{:.4}", s.compliance.load_fraction)));
                items.push(value("Member cone", format!("{:.1} deg", m.solver_cone_angle_deg())));
                if m.member_stiffness_fe {
                    items.push(line(if m.uses_fe_stiffness() { "Member stiffness from the finite-element model." } else { "Finite-element stiffness pending: the cone is used meanwhile." }, Tone::Dim));
                }
                items
            }
        };
        Card::new("preload-result", "Result", Relevance::Primary, items)
    }

    /// Every result section the terminal shows, one card each, only those the inputs turn on.
    fn solution_cards(&self, s: &JointSolution) -> Vec<Card> {
        let results = |c: Card| c.for_steps(&["Results", "Report"]);
        let mut cards = Vec::new();
        let t = &s.torque;
        cards.push(results(Card::new(
            "preload-torque",
            "Torque breakdown",
            Relevance::Primary,
            vec![
                value("Thread", format!("{:.2} in-lbf ({:.1}%)", t.thread, t.thread_fraction() * 100.0)),
                value("Bearing", format!("{:.2} in-lbf ({:.1}%)", t.bearing, t.bearing_fraction() * 100.0)),
                value("Prevailing", format!("{:.2} in-lbf ({:.1}%)", t.prevailing, t.prevailing_fraction() * 100.0)),
                value("Applied", format!("{:.2} in-lbf", t.applied)),
            ],
        )));
        let mut stress = vec![line("Axial / torsional shear / von Mises, then the yield and ultimate margins.", Tone::Dim)];
        for (label, sec) in [("Shank", &s.stress.shank), ("Tensile area", &s.stress.tensile_stress_area), ("Thread root", &s.stress.thread_root)] {
            stress.extend(section_rows(label, sec));
        }
        stress.push(value("Bearing pressure", psi(s.bearing_contact_pressure)));
        stress.push(value("Member compression", psi(s.member_average_compressive_stress)));
        if let Some(m) = s.thread_shear_margin {
            stress.push(margin("Thread shear MS", m));
        }
        cards.push(results(Card::new("preload-stress", "Fastener stress", Relevance::Primary, stress)));
        if let Some(service) = &s.service_load {
            let mut items = vec![
                value("External load", format!("{:.1} lbf", service.external_load)),
                value("Bolt load", format!("{:.1} lbf", service.bolt_load)),
                value("Remaining clamp", format!("{:.1} lbf", service.member_load)),
                value("Separation load", format!("{:.1} lbf", service.separation_load)),
                margin("Separation MS", service.separation_margin),
            ];
            if service.separated {
                items.push(line("The joint separates under this load.", Tone::Warn));
            }
            cards.push(results(Card::new("preload-service", "Service load", Relevance::Primary, items)));
        }
        if let (Some(capacity), Some(m)) = (s.slip_capacity, s.slip_margin) {
            cards.push(results(Card::new("preload-slip", "Slip resistance", Relevance::Primary, vec![value("Slip capacity", format!("{capacity:.1} lbf")), margin("Slip MS", m)])));
        }
        let d = &s.deformation;
        cards.push(
            Card::new(
                "preload-deformation",
                "Deformation",
                Relevance::Context,
                vec![
                    value("Fastener stretch", format!("{:.6} in", d.fastener_elongation)),
                    value("Member compression", format!("{:.6} in", d.member_compression)),
                    value("Total closure", format!("{:.6} in", d.total_closure)),
                    value("Compliance C_b", format!("{:.3e} in/lbf", s.compliance.c_b)),
                    value("Compliance C_m", format!("{:.3e} in/lbf", s.compliance.c_m)),
                ],
            )
            .for_steps(&["Results"]),
        );
        let r = &s.rotation;
        cards.push(
            Card::new(
                "preload-rotation",
                "Nut rotation",
                Relevance::Context,
                vec![
                    value("Relative rotation", format!("{:.4} rev", r.revolutions)),
                    value("In degrees", format!("{:.2} deg", r.degrees)),
                    value("Fastener twist", format!("{:.3} deg", r.torsional_twist_degrees)),
                    value("Est. tool rotation", format!("{:.2} deg", r.estimated_tool_rotation_degrees)),
                ],
            )
            .for_steps(&["Results"]),
        );
        let mut uncertainty = Vec::new();
        if let Some(u) = s.uncertainty {
            uncertainty.push(line("Worst case over every tolerance corner:", Tone::Dim));
            uncertainty.push(value("Preload min / max", format!("{:.0} / {:.0} lbf", u.preload_min, u.preload_max)));
            uncertainty.push(value("Nominal", format!("{:.0} lbf", u.preload_nominal)));
        }
        if let Some(mc) = s.monte_carlo {
            uncertainty.push(line(format!("Monte Carlo, {} samples, seed {}:", mc.samples, mc.seed), Tone::Dim));
            uncertainty.push(value("Mean +/- std dev", format!("{:.0} +/- {:.0} lbf", mc.preload_mean, mc.preload_std_dev)));
            uncertainty.push(value("Min / max", format!("{:.0} / {:.0} lbf", mc.preload_min, mc.preload_max)));
        }
        if !uncertainty.is_empty() {
            cards.push(results(Card::new("preload-uncertainty", "Uncertainty", Relevance::Context, uncertainty)));
        }
        if let Some(tld) = &s.thread_load_distribution {
            let shown = tld.per_thread_loads.len().min(20);
            let mut items = vec![value("First thread", format!("{:.1} lbf", tld.first_thread_load)), value("Most loaded", format!("{:.1} lbf", tld.max_thread_load))];
            items.extend(tld.per_thread_loads.iter().take(shown).enumerate().map(|(i, l)| value(format!("Thread {}", i + 1), format!("{l:.1} lbf"))));
            if tld.per_thread_loads.len() > shown {
                items.push(line(format!("{} more threads not shown.", tld.per_thread_loads.len() - shown), Tone::Dim));
            }
            cards.push(Card::new("preload-threads", "Thread load distribution", Relevance::Context, items).for_steps(&["Results"]));
        }
        if !s.warnings.is_empty() {
            cards.push(Card::new("preload-warnings", "Warnings", Relevance::Primary, s.warnings.iter().map(|w| line(w.to_string(), Tone::Warn)).collect()));
        }
        cards
    }

    /// The member stack on the general kernel: its compliance beside the cone's, the load fraction each gives and the
    /// cone angle that reproduces it.
    fn fe_card(&self) -> Card {
        let items = match self.fe_current() {
            Some(Ok((r, _))) => {
                let diff = r.difference();
                let mut items = vec![
                    value("Cone C_m", format!("{:.4e} in/lbf", r.cone)),
                    value("FE C_m", format!("{:.4e} in/lbf", r.fe.compliance)),
                    DockItem::field("FE vs cone", format!("{:+.1}%", diff * 100.0), if diff.abs() > 0.25 { Tone::Warn } else { Tone::Normal }, None, false),
                    value("Load fraction", format!("cone {:.3}, FE {:.3}", r.fraction_cone, r.fraction_fe)),
                    value("Mesh", format!("{} elements, {:.0} ms", r.fe.elements, r.fe.ms)),
                ];
                if let Some(a) = r.equivalent_angle_deg {
                    items.push(value("Equivalent cone", format!("{a:.1} deg")));
                }
                items.push(line("Axisymmetric; each member uses its own Poisson's ratio. Member Stiffness: Finite Element solves the joint with them.", Tone::Dim));
                items
            }
            Some(Err(e)) => vec![line(format!("The member finite-element model failed: {e}."), Tone::Warn)],
            None if self.model.output.is_err() => vec![line("Runs once the joint solves.", Tone::Dim)],
            None if self.fe_running() => vec![line("Running the member finite-element model...", Tone::Dim)],
            None => vec![line("Starts a moment after the joint stops changing.", Tone::Dim)],
        };
        Card::new("preload-fe", "Member FE cross-check", Relevance::Context, items).for_steps(&["Results", "Report"])
    }

    fn drawing_card(&self) -> Card {
        let text = if self.model.output.is_err() {
            return Card::new("preload-drawing", "Drawing", Relevance::Info, vec![line("Nothing to draw: the inputs make no joint.", Tone::Warn)]);
        } else if self.shows_result() {
            let angle = self.fe_current().and_then(|r| r.as_ref().ok()).and_then(|(r, _)| r.equivalent_angle_deg);
            format!(
                "The members as the finite-element model solved them under this preload, a quarter cut away, with the shank in the bore. The outline is the double cone at {} that matches the model's stiffness. Click the legend's title for the next stress.",
                angle.map_or_else(|| format!("the joint's {:.1} deg", self.model.cone_half_angle_deg), |a| format!("{a:.1} deg"))
            )
        } else if matches!(self.fe_current(), Some(Err(_))) {
            format!("The parts at the joint's {:.1} deg cone, coloured by part: the member model failed, so no stress to show.", self.model.cone_half_angle_deg)
        } else {
            format!("The parts at the joint's {:.1} deg cone, coloured by part, while the member finite-element model runs; its stress follows.", self.model.cone_half_angle_deg)
        };
        let mut items = vec![line(text, Tone::Dim)];
        if self.shows_result() {
            items.push(value("Fields", drawing::FIELDS.join(", ").to_lowercase()));
        }
        Card::new("preload-drawing", "Drawing", Relevance::Info, items)
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let step = |s: &'static str| act(PreloadAct::Step(s));
        let mut out = vec![TreeNode::new(1, 0, 'P', format!("Preload: {}", self.name)).on_click(step("Inputs"))];
        out.push(TreeNode::new(2, 1, '+', "Inputs").folder().selected(self.step == "Inputs").on_click(step("Inputs")));
        for (k, r) in model::field_rows(&self.model).into_iter().enumerate() {
            if let FieldRow::Header(h) = r {
                out.push(TreeNode::new(10 + k as u32, 2, '#', h).on_click(step("Inputs")));
            }
        }
        for (k, m) in self.model.members.iter().enumerate() {
            out.push(TreeNode::new(100 + k as u32, 2, '=', format!("{} {:.3} in", m.name, m.thickness)).on_click(step("Inputs")));
        }
        out.push(TreeNode::new(3, 1, '+', format!("Results ({})", if self.all_pass() { "pass" } else { "review" })).folder().tone(ok_tone(self.all_pass())).selected(self.step == "Results").on_click(step("Results")));
        if let Ok(s) = &self.model.output {
            for (k, (name, m)) in margins(s).into_iter().enumerate() {
                let ok = m >= 0.0;
                out.push(TreeNode::new(200 + k as u32, 2, if ok { '^' } else { '!' }, format!("{name} MS {}", fmt_margin(m))).tone(ok_tone(ok)).on_click(step("Results")));
            }
        }
        out.push(TreeNode::new(4, 1, 'R', "Report").selected(self.step == "Report").on_click(step("Report")));
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let never = |what: &str| Err(format!("nothing to {what}"));
        let edit = vec![
            entry("Undo", "Ctrl/Cmd+Z", PreloadAct::Undo).when(if self.undo.is_empty() { never("undo") } else { Ok(()) }),
            entry("Redo", "Ctrl/Cmd+Shift+Z", PreloadAct::Redo).when(if self.redo.is_empty() { never("redo") } else { Ok(()) }),
            entry("Save", "Ctrl/Cmd+S", PreloadAct::Save),
        ];
        let n = self.model.members.len();
        let joint = vec![
            entry("Bolt", "", PreloadAct::Row(FieldRow::OpenBoltPicker)),
            entry("Joint template", "", PreloadAct::Row(FieldRow::OpenTemplatePicker)),
            entry("Add member", "", PreloadAct::Row(FieldRow::ToggleAddMember)).when(if n < MAX_MEMBERS { Ok(()) } else { Err(format!("at most {MAX_MEMBERS} members")) }),
            entry("Remove member", "", PreloadAct::Row(FieldRow::ToggleRemoveMember)).when(if n > model::MIN_MEMBERS { Ok(()) } else { Err("one member must remain".into()) }),
        ];
        let analysis = vec![entry("Analysis mode", "", PreloadAct::Row(FieldRow::ToggleMode)), entry("Member stiffness", "", PreloadAct::Row(FieldRow::ToggleMemberStiffness))];
        vec![
            RibbonTab::new("Inputs").group("Joint", joint).group("Analysis", analysis).group("Edit", edit),
            RibbonTab::new("Results").group("Results", vec![entry("Export report", "", PreloadAct::Export)]),
            RibbonTab::new("Report").group("Report", vec![entry("Export report", "", PreloadAct::Export), entry("Save", "Ctrl/Cmd+S", PreloadAct::Save)]),
        ]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |tab: &str| tabs.iter().find(|t| t.title == tab).map(|t| t.groups.iter().flat_map(|g| g.buttons.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let pick = |tab: &str, labels: &[&str]| all(tab).into_iter().filter(|e| labels.contains(&e.label.as_str())).map(Item::Entry).collect::<Vec<_>>();
        vec![
            Menu { title: "File".into(), items: pick("Report", &["Save", "Export report"]) },
            Menu { title: "Edit".into(), items: pick("Inputs", &["Undo", "Redo"]) },
            Menu { title: "Tools".into(), items: pick("Inputs", &["Bolt", "Joint template", "Add member", "Remove member", "Analysis mode", "Member stiffness"]) },
        ]
    }
}

/// A stress section's rows: its three stresses on one row, its two margins on the next (one row would overflow the card).
fn section_rows(label: &str, s: &StressSection) -> Vec<DockItem> {
    vec![
        value(label, format!("{:.0} / {:.0} / {:.0} psi", s.axial, s.torsional_shear, s.von_mises)),
        DockItem::field(format!("{label} MS"), format!("yield {}, ult {}", fmt_margin(s.yield_margin), fmt_margin(s.ultimate_margin)), tone(tone_for_margin(s.yield_margin.min(s.ultimate_margin))), None, false),
    ]
}
