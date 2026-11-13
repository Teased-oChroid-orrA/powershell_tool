//! What the Bushing workbench shows: the three panes (`ui::Workbench`: inputs, views, results), the Send dialog and the
//! menus. Plain data built from the model; the window draws it. The numbers and wording come from the workbench's own
//! model, checks and edge-check report; only the layout is new.
use super::{drawing::Field, fe, sketch, BushingAct, BushingSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        handoff,
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{Band, Bar, Dialog, DialogOption, DockItem, Figure, Mark, Plot, Series, Workbench, WorkbenchView},
};
use crate::toolboxes::bushing::{
    advice::{self, CheckKind, Severity},
    edge_check::{self, TableRow},
    model::{self, FieldRow, NumberTarget},
    report,
};
use bushing_solver::tolerance::ToleranceStatus;

fn act(a: BushingAct) -> UiAction {
    UiAction::Editor(EditorAction::Bushing(a))
}
fn entry(label: &str, shortcut: &str, a: BushingAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
/// Text of the model, the advice and the edge check, for a window font that lacks arrows and a few symbols.
pub(super) fn plain(s: &str) -> String {
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
fn stat(label: &str, value: impl Into<String>, note: impl Into<String>, note_tone: Tone, detail: impl Into<String>, action: Option<UiAction>) -> DockItem {
    DockItem::Stat { label: label.into(), value: value.into(), note: note.into(), note_tone, detail: detail.into(), action }
}
/// `v` rounded to `decimals`, with thousands grouped by commas (1036.04 -> "1,036.0").
pub(super) fn grouped(v: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, v.abs());
    let (int, frac) = s.split_once('.').map_or((s.as_str(), ""), |(i, f)| (i, f));
    let mut g = String::new();
    for (k, c) in int.chars().enumerate() {
        if k > 0 && (int.len() - k) % 3 == 0 {
            g.push(',');
        }
        g.push(c);
    }
    let sign = if v < 0.0 && s.chars().any(|c| c != '0' && c != '.') { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{g}")
    } else {
        format!("{sign}{g}.{frac}")
    }
}

/// The units of every input and result (the solver is US customary only).
pub(super) const UNITS_CHIP: &str = "in \u{b7} lbf \u{b7} ksi \u{b7} \u{b0}F";

impl BushingSession {
    // ------------------------------------------------------------------ the three panes

    pub(super) fn workbench_data(&self) -> Workbench {
        let m = &self.st.model;
        let failing = self.failing();
        let mut chips = vec![(UNITS_CHIP.to_string(), Tone::Normal)];
        chips.push(if failing == 0 { ("PASS".to_string(), Tone::On) } else { (format!("REVIEW \u{b7} {failing} check{} failing", if failing == 1 { "" } else { "s" }), Tone::Warn) });
        if handoff::Link::exists(&handoff::fea_path(&self.path)) {
            chips.push(("linked to the FEA Workbench".to_string(), Tone::Dim));
        }
        let view = |label: &str, v: WorkbenchView| entry(label, "", BushingAct::View(v)).checked(self.show == v);
        Workbench {
            chips,
            actions: vec![
                entry("Undo", "Ctrl/Cmd+Z", BushingAct::Undo).when(self.undo_availability()),
                entry("Redo", "Ctrl/Cmd+Shift+Z", BushingAct::Redo).when(self.redo_availability()),
                entry("Save", "Ctrl/Cmd+S", BushingAct::Save),
                entry("Export report", "", BushingAct::Export),
                entry("Send joint to FEM...", "", BushingAct::SendDialog).when(self.send_availability()),
            ],
            inputs: self.input_cards(),
            inputs_note: "US customary inputs only. ! marks an input a failing check points at.".into(),
            views: vec![view("Split", WorkbenchView::Split), view("3D", WorkbenchView::Model), view("Axial section", WorkbenchView::Sketch(0)), view("Cross-section", WorkbenchView::Sketch(1))],
            show: self.show,
            fields: Field::ALL.iter().map(|f| entry(f.label(), "", BushingAct::Field(*f)).checked(self.field == *f)).collect(),
            switches: vec![entry("Quarter cut", "", BushingAct::QuarterCut).checked(self.quarter_cut), entry("Reset view", "", BushingAct::ResetView)],
            view_note: "View only \u{b7} the inputs drive the picture".into(),
            model_title: if self.quarter_cut { "three-quarter section".into() } else { "whole joint".into() },
            model_hint: "Drag to orbit \u{b7} closed-form Lam\u{e9} field, no mesh".into(),
            sketches: sketch::sketches(m),
            results: self.result_cards(),
            dialog: self.dialog_open.then(|| self.send_dialog()),
        }
    }

    /// One card per section of the input form (`field_rows`), in the form's own order; the stackup's source first.
    pub(super) fn input_cards(&self) -> Vec<Card> {
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
                    let editing = self.prompt.as_ref().filter(|p| p.target == super::PromptTarget::Input(row));
                    let value = editing.map_or_else(|| report::display_value(m, row), |p| format!("{}_", p.text));
                    let flagged = matches!(row, FieldRow::Number(t) if related.contains(&t));
                    // The window draws an edit box on a Warn value, so a flagged input is marked in its label instead.
                    let label = report::row_text(m.tolerance_mode, row);
                    let label = if flagged && editing.is_none() { format!("! {label}") } else { label.to_string() };
                    DockItem::field(label, value, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(BushingAct::Row(row))), editing.is_some())
                }
            };
            match sections.last_mut() {
                Some(s) => s.1.push(item),
                None => sections.push((String::new(), vec![item])),
            }
        }
        let editing_hint = self.prompt.is_some();
        let mut cards: Vec<Card> = sections
            .into_iter()
            .enumerate()
            .map(|(k, (title, mut items))| {
                if editing_hint && items.iter().any(|i| matches!(i, DockItem::Field { focused: true, .. })) {
                    items.push(line("Type a value, Enter applies, Esc cancels.", Tone::Dim));
                }
                let title = if title.is_empty() { "Inputs".to_string() } else { title };
                Card::new(format!("bushing-{}", slug(&title)), title, if k < 2 { Relevance::Primary } else { Relevance::Context }, items)
            })
            .collect();
        if let Some(source) = &self.stackup {
            let member = source.inputs.members.iter().find(|m| m.id == source.member_id);
            let policy = member.map(|m| bushing_solver::stackup::wall_policy(&source.inputs, m));
            cards.insert(
                0,
                Card::new(
                    "stackup-machining-source",
                    "Calculated stackup hole",
                    Relevance::Primary,
                    vec![
                        line(format!("Source {} / member #{} / revision {}", source.source_file, source.member_id, source.source_revision), Tone::Dim),
                        line(format!("Current hole {:.5} in; angularity cleanup {:.5} in; adopted member hole {:.5} in", member.map_or(0.0, |m| m.hole.nominal), source.angularity_cleanup, source.adopted_hole.nominal), Tone::Normal),
                        line(source.enlargement_reason.as_ref().map_or_else(|| "Not enlarged for the bushing".into(), |reason| format!("ENLARGED: {reason}")), Tone::Title),
                        line(format!("Source wall policy {policy:?}. Provenance retained when target inputs are edited. Target load/fit are editable defaults; its plane FE uses an extended far boundary."), Tone::Dim),
                    ],
                ),
            );
        }
        cards
    }

    /// The results pane, top to bottom: the verdict, then what the fit does, then the cross-checks.
    pub(super) fn result_cards(&self) -> Vec<Card> {
        let mut cards = vec![self.verdict_card(), self.install_card(), self.fit_card(), self.stress_card(), self.edge_card(), self.margins_card(), self.through_wall_card()];
        if let Some(c) = self.fixes_card() {
            cards.push(c);
        }
        cards.push(self.cross_check_card());
        cards.push(self.quick_fe_card());
        cards
    }

    /// The failing check a "Fixes for" button opens: the governing one when it fails, else the first failing with a fix.
    fn fix_target(&self) -> Option<CheckKind> {
        let m = &self.st.model;
        let has_fix = |k: CheckKind| m.recommendations.iter().any(|r| r.fixes == k);
        let gov = CheckKind::from_candidate_name(m.output.governing.name).filter(|k| advice::severity_of(&m.checks, *k) == Severity::Fail && has_fix(*k));
        gov.or_else(|| m.checks.iter().find(|c| c.severity == Severity::Fail && has_fix(c.kind)).map(|c| c.kind))
    }

    /// PASS / REVIEW, what governs, and (behind "Why?") every check that needs attention.
    fn verdict_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let failing = self.failing();
        let mut actions = Vec::new();
        if let Some(k) = self.fix_target() {
            actions.push(entry(&format!("Fixes for {}", k.label()), "", BushingAct::ShowFixes(k)));
        }
        actions.push(entry(if self.why { "Hide checks" } else { "Why?" }, "", BushingAct::Why));
        let headline = if failing == 0 { "PASS".to_string() } else { format!("REVIEW \u{b7} {failing} failing") };
        let mut items = vec![DockItem::Verdict { headline, detail: format!("governing: {} {}", out.governing.name, margin(out.governing.margin)), tone: if failing == 0 { Tone::On } else { Tone::Warn }, actions }];
        if self.why {
            let attention: Vec<&advice::Check> = m.checks.iter().filter(|c| c.severity != Severity::Pass).collect();
            for c in &attention {
                let has_fix = m.recommendations.iter().any(|r| r.fixes == c.kind);
                let glyph = if c.severity == Severity::Fail { "x" } else { "!" };
                items.push(DockItem::row(plain(&format!("{glyph} {}: {}", c.kind.label(), c.detail)), Tone::Warn, has_fix.then(|| act(BushingAct::ShowFixes(c.kind))), false));
            }
            for a in edge_check::advisories(self.st.edge_check.as_ref(), !self.edge_current()).iter().take(1) {
                items.push(line(plain(&format!("! {}", a.text)), Tone::Warn));
            }
            if attention.is_empty() {
                items.push(line("Every check passes.", Tone::Dim));
            }
        }
        if let Some(why) = &self.st.input_error {
            items.push(line(plain(&format!("x {why}")), Tone::Warn));
        }
        if let Some(a) = &self.st.last_applied {
            items.push(line(plain(&format!("ok Applied: {a}")), Tone::On));
        }
        Card::new("bushing-verdict", "Verdict", Relevance::Primary, items)
    }

    /// Install force first (the figure the shop asks for), with the retained force and the formula behind it.
    fn install_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let mut retained = vec![if m.delta_t != 0.0 { format!("at service \u{394}T {:+.0} \u{b0}F", m.delta_t) } else { "in service".to_string() }];
        if m.assembly_thermal_enabled {
            retained.push("install uses the thermal assist".into());
        }
        let mut items = vec![DockItem::Figures(vec![
            Figure { label: "Install force".into(), value: grouped(out.install_force, 0), unit: "lbf".into(), detail: vec![format!("range {} to {}", grouped(out.install_force_range.min, 0), grouped(out.install_force_range.max, 0))] },
            Figure { label: "Retained".into(), value: grouped(out.retained_install_force, 0), unit: "lbf".into(), detail: retained },
        ])];
        // The formula, printed only while it reproduces the solver's number (it always should; a mismatch prints nothing
        // rather than a wrong equation).
        let f = m.friction * out.install_pressure * std::f64::consts::PI * m.bore_dia * m.housing_len;
        if out.install_force > 0.0 && ((f - out.install_force) / out.install_force).abs() < 0.005 {
            items.push(line(format!("F = \u{3bc} \u{b7} p \u{b7} \u{3c0} \u{b7} D \u{b7} L = {:.2} \u{b7} {} \u{b7} \u{3c0} \u{b7} {:.3} \u{b7} {:.3} = {} lbf", m.friction, grouped(out.install_pressure, 0), m.bore_dia, m.housing_len, grouped(out.install_force, 1)), Tone::Dim));
        }
        Card::new("bushing-install", "Install", Relevance::Primary, items)
    }

    fn candidate(&self, kind: CheckKind) -> Option<f64> {
        self.st.model.output.candidates.iter().find(|c| CheckKind::from_candidate_name(c.name) == Some(kind)).map(|c| c.margin)
    }
    /// A result tied to a check: its margin at the right (warning tone and a fixes link while the check fails).
    fn checked_stat(&self, label: &str, value: String, kind: CheckKind, ms: Option<f64>, detail: String) -> DockItem {
        let m = &self.st.model;
        let sev = advice::severity_of(&m.checks, kind);
        let has_fix = m.recommendations.iter().any(|r| r.fixes == kind);
        let ms = ms.or_else(|| self.candidate(kind));
        let note = match (sev, ms) {
            (Severity::Fail, Some(v)) => format!("MS {}  fixes", margin(v)),
            (Severity::Fail, None) => "FAIL  fixes".into(),
            (_, Some(v)) => format!("MS {}", margin(v)),
            (_, None) => "OK".into(),
        };
        let tone = if sev == Severity::Pass { ms.map_or(Tone::On, margin_tone) } else { Tone::Warn };
        stat(label, value, note, tone, detail, (sev == Severity::Fail && has_fix).then(|| act(BushingAct::ShowFixes(kind))))
    }

    fn fit_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let mut detail = format!("target {:.4} \u{b7} range {:.4} to {:.4}", m.interference, out.achieved_interference_tol.lower, out.achieved_interference_tol.upper);
        if m.delta_t != 0.0 {
            detail.push_str(&format!(" \u{b7} thermal \u{394} {:+.5}", out.delta_thermal));
        }
        if m.assembly_thermal_enabled {
            detail.push_str(&format!(" \u{b7} install assist {:+.5} (at install {:.5})", out.assembly_thermal_delta, out.install_delta));
        }
        let tol = match out.tolerance_status {
            ToleranceStatus::Ok => "OK",
            ToleranceStatus::Clamped => "clamped",
            ToleranceStatus::Infeasible => "INFEASIBLE",
        };
        let mut items = vec![
            stat("OD installed", format!("{:.4} in", out.od_installed), "", Tone::Normal, format!("range {:.4} to {:.4}", out.od_tol.lower, out.od_tol.upper), None),
            stat("Interference", format!("{:.4} in", out.delta_total), "", Tone::Normal, detail, None),
            self.checked_stat("Straight wall", format!("{:.4} in", out.wall_straight), CheckKind::StraightWall, None, format!("range {:.4} to {:.4}", out.wall_straight_range.min, out.wall_straight_range.max)),
            self.checked_stat("Neck wall", format!("{:.4} in", out.wall_neck), CheckKind::NeckWall, None, String::new()),
            self.checked_stat("Tolerance", tol.to_string(), CheckKind::Tolerance, None, format!("{} note(s)", out.tolerance_notes.len())),
        ];
        for n in &out.tolerance_notes {
            items.push(line(plain(&format!("! {n}")), Tone::Warn));
        }
        if matches!(m.fit_type, model::FitType::Clearance | model::FitType::Slip) && m.interference > 0.0 {
            items.push(line(format!("! Fit type is {} but the target interference is positive ({:.4} in): informational only.", model::label_fit_type(m.fit_type), m.interference), Tone::Warn));
        }
        Card::new("bushing-fit", "Fit", Relevance::Primary, items)
    }

    fn stress_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let mut items = vec![
            stat("Contact pressure", format!("{} psi", grouped(out.pressure, 0)), "", Tone::Normal, format!("range {} to {}", grouped(out.pressure_range.min, 0), grouped(out.pressure_range.max, 0)), None),
            self.checked_stat("Housing hoop", format!("{} psi", grouped(out.stress_hoop_housing, 0)), CheckKind::HousingStress, Some(out.housing_ms), String::new()),
            self.checked_stat("Bushing hoop", format!("{} psi", grouped(out.stress_hoop_bushing, 0)), CheckKind::BushingStress, Some(out.bushing_ms), String::new()),
        ];
        items.push(if out.axial_constraint_factor > 0.0 {
            stat("Axial, housing / bushing", format!("{} / {} psi", grouped(out.stress_axial_housing, 0), grouped(out.stress_axial_bushing, 0)), "", Tone::Normal, model::label_end_constraint(m.end_constraint), None)
        } else {
            line(format!("Axial, both 0 psi ({})", model::label_end_constraint(m.end_constraint)), Tone::Dim)
        });
        Card::new("bushing-stress", "Contact & stress", Relevance::Primary, items)
    }

    fn edge_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let items = vec![
            stat("Actual e/D", format!("{:.3}", out.ed_actual), "", Tone::Normal, format!("e {:.3} in \u{b7} D {:.4} in", m.edge_dist, m.bore_dia), None),
            self.checked_stat("Needs, sequencing", format!("{:.3}", out.ed_min_sequence), CheckKind::EdgeSequencing, Some(out.sequence_margin), String::new()),
            self.checked_stat("Needs, strength", format!("{:.3}", out.ed_min_strength), CheckKind::EdgeStrength, Some(out.strength_margin), String::new()),
        ];
        Card::new("bushing-edge", "Edge distance", Relevance::Primary, items)
    }

    /// Every margin the solver weighs, on one scale; the governing one tagged.
    fn margins_card(&self) -> Card {
        let m = &self.st.model;
        let out = &m.output;
        let rows = out
            .candidates
            .iter()
            .map(|c| {
                let kind = CheckKind::from_candidate_name(c.name);
                let fails = kind.is_some_and(|k| advice::severity_of(&m.checks, k) == Severity::Fail);
                Bar {
                    label: c.name.to_string(),
                    value: if c.margin.is_finite() { c.margin } else { 1.0 },
                    text: margin(c.margin),
                    tone: if fails { Tone::Warn } else { margin_tone(c.margin) },
                    tag: if c.name == out.governing.name { "governing".into() } else { String::new() },
                    action: kind.filter(|k| fails && m.recommendations.iter().any(|r| r.fixes == *k)).map(|k| act(BushingAct::ShowFixes(k))),
                }
            })
            .collect();
        Card::new("bushing-margins", "Margins of safety", Relevance::Context, vec![DockItem::Bars { rows, lo: -0.5, hi: 1.0 }])
    }

    /// Hoop and radial stress from the bore of the bushing out through the housing (the closed-form Lamé field).
    fn through_wall_card(&self) -> Card {
        let out = &self.st.model.output;
        let all: Vec<&mechanics_core::lame::LameSample> = out.bushing_stress_field.iter().chain(out.housing_stress_field.iter()).collect();
        if all.is_empty() {
            return Card::new("bushing-through-wall", "Through-wall stress", Relevance::Context, vec![line("No stress field for these inputs.", Tone::Dim)]);
        }
        let ri = out.bushing_stress_field.first().map_or(0.0, |s| s.r);
        let a = out.bushing_stress_field.last().map_or(0.0, |s| s.r);
        let ro = out.housing_stress_field.last().map_or(a, |s| s.r);
        let mut bands = vec![Band { x0: ri, x1: a, label: "Bushing".into() }];
        if ro > a {
            bands.push(Band { x0: a, x1: ro, label: "Housing to equivalent OD".into() });
        }
        let marks = out.housing_stress_field.first().map(|s| Mark { at: [s.r, s.sigma_theta], text: format!("{} psi", grouped(s.sigma_theta, 0)), color: 0 }).into_iter().collect();
        let plot = Plot {
            x_label: "r (in)".into(),
            y_label: "psi".into(),
            series: vec![
                Series { label: "\u{3c3}\u{3b8} hoop".into(), points: all.iter().map(|s| [s.r, s.sigma_theta]).collect(), color: 0, dashed: false },
                Series { label: "\u{3c3}r radial".into(), points: all.iter().map(|s| [s.r, s.sigma_r]).collect(), color: 1, dashed: true },
            ],
            bands,
            marks,
        };
        Card::new("bushing-through-wall", "Through-wall stress", Relevance::Context, vec![DockItem::Plot(plot)])
    }

    /// The verified fixes: pick one to see what it does to everything else, then apply it. Folded until a fixes link opens it.
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
        let title = self.fix_kind.map_or_else(|| "Fixes".to_string(), |k| format!("Fixes for {}", k.label()));
        let card = Card::new("bushing-fixes", title, Relevance::Primary, items).tag(format!("{}", m.recommendations.len()));
        Some(if self.show_fixes { card } else { card.folded() })
    }

    /// The independent edge-distance cross-check: the minimum e/D each model asks for, against what the housing has.
    fn cross_check_card(&self) -> Card {
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
        items.push(line("Uncertainty: CV = standard deviation / mean. Enter 0..100 percent.", Tone::Dim));
        for (id, label, cv) in self.st.edge_settings.rows(m) {
            let editing = self.prompt.as_ref().filter(|p| p.target == super::PromptTarget::EdgeCv(id));
            let value = editing.map_or_else(|| edge_check::cv_percent(cv), |p| format!("{}_", p.text));
            let label = if id.is_some() { format!("{label} model CV (%)") } else { label.to_string() };
            items.push(DockItem::field(label, value, if editing.is_some() { Tone::Warn } else { Tone::Normal }, Some(act(BushingAct::EdgeCv(id))), editing.is_some()));
        }
        items.push(line("Edits rerun after 400 ms. Settings last this session only.", Tone::Dim));
        if self.st.edge_job.is_some() {
            items.push(DockItem::row("Cancel", Tone::Warn, Some(act(BushingAct::CancelEdgeCheck)), false));
        } else {
            items.push(DockItem::row("Run quick check", Tone::Title, Some(act(BushingAct::EdgeCheck(false))), false));
            items.push(DockItem::row("Run with contact FE (about 3 s)", Tone::Title, Some(act(BushingAct::EdgeCheck(true))), false));
        }
        let tag = match (&self.st.edge_check, self.st.edge_job.is_some(), self.edge_current()) {
            (_, true, _) => "running",
            (None, false, _) => "not run",
            (Some(_), false, true) => "current",
            (Some(_), false, false) => "stale",
        };
        Card::new("bushing-edge-check", "Cross-checks", Relevance::Context, items).folded().tag(tag)
    }

    /// The FE solve of this joint (plane stress, fit contact), as a check on the closed-form fit pressure. Open while it
    /// runs or has a result.
    fn quick_fe_card(&self) -> Card {
        let mut items = vec![line("Solves the housing and bushing with fit contact (plane stress) to check the closed-form pressure. The closed-form results stay the verdict.", Tone::Dim)];
        let solution = self.fea.last_solution();
        let current = solution.filter(|s| self.fe_note.is_none() && s.problem == self.fea.problem);
        if let Some(t) = self.fea.solve_elapsed() {
            items.push(field("Solve", format!("running, {:.0} s", t.as_secs_f32()), Tone::Warn));
        }
        if let Some(s) = solution {
            if current.is_none() {
                items.push(line("! Inputs changed since this solve: run it again for these inputs.", Tone::Warn));
            }
            let sum = &s.summary;
            if let Some(i) = sum.interfaces.first() {
                items.push(field("FE mean / peak", format!("{} / {} psi", grouped(i.mean_pressure, 0), grouped(i.peak_pressure, 0)), Tone::Normal));
                let lame = self.st.model.output.pressure;
                if current.is_some() && lame > 0.0 {
                    let delta = i.mean_pressure - lame;
                    items.push(field("Lam\u{e9} closed form", format!("{} psi", grouped(lame, 0)), Tone::Normal));
                    items.push(field("FE minus Lam\u{e9}", format!("{:+.0} psi ({:+.1} %)", delta, 100.0 * delta / lame), if delta.abs() / lame > 0.15 { Tone::Warn } else { Tone::Dim }));
                }
                items.push(field("Open arc", format!("{:.0}\u{b0}", i.open_arc_deg), if i.open_arc_deg > 0.0 { Tone::Warn } else { Tone::Normal }));
            }
            items.push(field("Max von Mises", format!("{} psi", grouped(sum.max_von_mises.value, 0)), Tone::Normal));
            items.push(line("The FE uses a finite rectangular housing and discretized contact; Lam\u{e9} assumes ideal concentric cylinders. Treat the difference as a cross-check, not a correction.", Tone::Dim));
        }
        if self.fea.solving() {
            items.push(DockItem::row("Cancel", Tone::Warn, Some(act(BushingAct::CancelSolve)), false));
        } else {
            match self.quick_fe_availability() {
                Ok(()) => items.push(DockItem::row("Run quick FE check (about 8 s)", Tone::Title, Some(act(BushingAct::QuickFe)), false)),
                Err(why) => items.push(line(format!("Not available: {why}."), Tone::Dim)),
            }
        }
        let tag = if self.fea.solving() {
            "running"
        } else if current.is_some() {
            "current"
        } else if solution.is_some() {
            "stale"
        } else {
            "not run"
        };
        let card = Card::new("bushing-quick-fe", "Quick FE check", Relevance::Context, items).tag(tag);
        if self.fea.solving() || solution.is_some() {
            card
        } else {
            card.folded()
        }
    }

    // ------------------------------------------------------------------ the Send dialog

    fn send_dialog(&self) -> Dialog {
        let m = &self.st.model;
        let mut rows = Vec::new();
        if let Ok(l) = fe::layout(m) {
            rows.push(("Housing".to_string(), format!("{:.3} x {:.3} in, {:.3} in thick, {}", l.length, l.height, self.fea.problem.thickness, m.housing_material().name)));
        }
        rows.push(("Bushing".to_string(), format!("\u{d8}{:.4} x \u{d8}{:.4} in, {}, FE overlap {:.5} in", m.bore_dia, m.id_bushing, m.bushing_material().name, self.fea.problem.bushings[0].interference)));
        rows.push(("Load".to_string(), format!("P {} lbf toward the free edge", grouped(m.load, 0))));
        rows.push(("File".to_string(), handoff::fea_path(&self.path).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())));
        Dialog {
            title: "Send joint to the FEA Workbench".into(),
            intro: "Opens a full finite-element study of this joint in the FEA Workbench. The closed-form results here stay the verdict. The case is saved first.".into(),
            rows,
            options: vec![
                DialogOption { label: "Plane stress, 2D with fit contact".into(), detail: String::new(), on: true, enabled: false, action: None },
                DialogOption { label: "3D solid".into(), detail: "Not available: bushing contact runs in plane analyses only (fea-problem, issue #241)".into(), on: false, enabled: false, action: None },
                DialogOption { label: "Keep linked".into(), detail: "Always on: Send back from the FEA Workbench updates this case.".into(), on: true, enabled: false, action: None },
            ],
            note: "Solve time for this joint is about 8 s.".into(),
            cancel: entry("Cancel", "Esc", BushingAct::CloseDialog),
            confirm: entry("Send and open FEA Workbench", "", BushingAct::ConfirmSend).when(self.send_availability()),
        }
    }

    // ------------------------------------------------------------------ availability and menus

    pub(super) fn send_availability(&self) -> Result<(), String> {
        match &self.fe_note {
            Some(why) => Err(format!("no FE model for these inputs: {why}")),
            None => Ok(()),
        }
    }
    pub(super) fn quick_fe_availability(&self) -> Result<(), String> {
        self.send_availability()?;
        if self.fea.solving() {
            return Err("a solve is already running".into());
        }
        self.fea.solve_availability()
    }
    fn edge_availability(&self) -> Result<(), String> {
        if self.st.edge_job.is_some() {
            return Err("an edge check is already running".into());
        }
        edge_check::prepare(&self.st.model, false).map(|_| ())
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
    fn running(on: bool, what: &str) -> Result<(), String> {
        if on {
            Ok(())
        } else {
            Err(format!("no {what} is running"))
        }
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let view = |label: &str, v: WorkbenchView| Item::Entry(entry(label, "", BushingAct::View(v)).checked(self.show == v));
        let edge = self.edge_availability();
        vec![
            Menu {
                title: "File".into(),
                items: vec![
                    Item::Entry(entry("Save", "Ctrl/Cmd+S", BushingAct::Save)),
                    Item::Entry(entry("Export report", "", BushingAct::Export)),
                    Item::Entry(entry("Send joint to FEM...", "", BushingAct::SendDialog).when(self.send_availability())),
                ],
            },
            Menu { title: "Edit".into(), items: vec![Item::Entry(entry("Undo", "Ctrl/Cmd+Z", BushingAct::Undo).when(self.undo_availability())), Item::Entry(entry("Redo", "Ctrl/Cmd+Shift+Z", BushingAct::Redo).when(self.redo_availability()))] },
            Menu {
                title: "View".into(),
                items: [
                    vec![view("Split", WorkbenchView::Split), view("3D", WorkbenchView::Model), view("Axial section", WorkbenchView::Sketch(0)), view("Cross-section", WorkbenchView::Sketch(1))],
                    Field::ALL.iter().map(|f| Item::Entry(entry(f.label(), "", BushingAct::Field(*f)).checked(self.field == *f))).collect(),
                    vec![Item::Entry(entry("Quarter cut", "", BushingAct::QuarterCut).checked(self.quarter_cut)), Item::Entry(entry("Reset view", "", BushingAct::ResetView))],
                ]
                .concat(),
            },
            Menu {
                title: "Tools".into(),
                items: vec![
                    Item::Entry(entry("Edge check", "", BushingAct::EdgeCheck(false)).when(edge.clone())),
                    Item::Entry(entry("Edge check + contact FE", "", BushingAct::EdgeCheck(true)).when(edge)),
                    Item::Entry(entry("Cancel edge check", "Esc", BushingAct::CancelEdgeCheck).when(Self::running(self.st.edge_job.is_some(), "edge check"))),
                    Item::Entry(entry("Fixes", "", BushingAct::AllFixes).when(if self.st.model.recommendations.is_empty() { Err("no check is failing".to_string()) } else { Ok(()) })),
                    Item::Entry(entry("Quick FE check", "Ctrl/Cmd+Enter", BushingAct::QuickFe).when(self.quick_fe_availability())),
                    Item::Entry(entry("Cancel FE check", "", BushingAct::CancelSolve).when(Self::running(self.fea.solving(), "FE check"))),
                ],
            },
        ]
    }
}
