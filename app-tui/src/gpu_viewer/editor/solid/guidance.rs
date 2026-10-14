//! Failure guidance in the solid editor (ADR-020): the findings of `fea_problem::diagnose` as an inspector card and a
//! "Fix" button in the status bar. After a failed solve the card is the primary one; while the model is being built it is
//! a quiet "Check" card with the same fixes. Applying a fix is one undoable edit. The rules decide what is offered; the
//! guidance ranker (`rank.rs`, the same measured gate as every other ranker) may only reorder the remedies, and the card
//! says which order it is showing: rules, simulated prior, or measured on the user's own sessions.
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        rank::Evidence,
        session::{EditorAction, Tone},
    },
    text::wrap,
    ui::DockItem,
};
use fea_problem::diagnose::{diagnose, Diagnosis, Edit, RemedyKind};

/// The findings for one state of the model, kept so the inspector does not recompute them every frame.
pub(super) struct Findings {
    problem: Problem,
    error: Option<String>,
    learned: bool,
    /// Findings from a result are part of the list: a new result is a new state.
    solved_id: Option<usize>,
    /// Which mesh the before-solve checks looked at.
    mesh_id: Option<usize>,
    list: Vec<Diagnosis>,
}

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

/// What the user is offered for a finding, as the guidance ranker sees it: the remedies' kinds, and a pseudo-kind for
/// answering its question when it has one.
fn offered_kinds(d: &Diagnosis) -> Vec<RemedyKind> {
    let mut kinds: Vec<RemedyKind> = d.remedies.iter().map(|r| r.kind).collect();
    if !d.questions.is_empty() {
        kinds.push(RemedyKind::AnswerQuestion);
    }
    kinds
}

/// Does the order put answering the question ahead of the first fix? (No question, or no fix: nothing to compare.)
pub(super) fn ask_first_in(order: &[RemedyKind]) -> bool {
    let ask = order.iter().position(|k| *k == RemedyKind::AnswerQuestion);
    let fix = order.iter().position(|k| *k != RemedyKind::AnswerQuestion);
    matches!((ask, fix), (Some(a), Some(f)) if a < f)
}

impl SolidSession {
    /// Findings for the model now, remedies in the rules' (or the learned) order. Empty while a sketch is open or for an
    /// empty model: there is nothing to check yet.
    pub(super) fn findings(&self) -> Vec<Diagnosis> {
        if self.sketching.is_some() || self.problem.validate_geometry().is_err_and(|e| !e.contains("tessellated") && !e.contains("out of date")) && self.failure.is_none() {
            return Vec::new();
        }
        let learned = self.advisors.guidance.settings.use_learned && self.advisors.guidance.learned_available().is_ok();
        let solved_id = self.current_solution().map(|s| std::sync::Arc::as_ptr(s) as usize);
        let mesh = self.shown_mesh();
        let mesh_id = mesh.map(|m| std::sync::Arc::as_ptr(m) as usize);
        if let Some(c) = self.findings_cache.borrow().as_ref() {
            if c.problem == self.problem && c.error == self.failure && c.learned == learned && c.solved_id == solved_id && c.mesh_id == mesh_id {
                return c.list.clone();
            }
        }
        let mut list = diagnose(&self.problem, self.failure.as_deref());
        // What the mesh shows without a solve: supports that leave the part free to float or spin.
        if let Some(m) = mesh {
            list.extend(fea_problem::diagnose::diagnose_mesh(&self.problem, m));
        }
        // What only a result can show (a displacement far too large for a linear model).
        if let Some(sol) = self.current_solution() {
            list.extend(fea_problem::diagnose::diagnose_result(sol));
        }
        for d in &mut list {
            let (order, _) = self.advisors.guidance.order(&d.class, &offered_kinds(d));
            d.remedies.sort_by_key(|r| order.iter().position(|k| *k == r.kind).unwrap_or(usize::MAX));
        }
        *self.findings_cache.borrow_mut() = Some(Findings { problem: self.problem.clone(), error: self.failure.clone(), learned, solved_id, mesh_id, list: list.clone() });
        list
    }

    /// A certain failure the checks found before the solve: the supports leave a rigid-body motion free.
    pub(super) fn predicted_blocker(&self) -> Option<String> {
        self.findings().into_iter().find(|d| d.class == fea_problem::diagnose::FailureClass::UnderConstrained && d.summary.starts_with("Before solving")).map(|d| d.summary)
    }

    /// What order the remedies are in because of: "rules", "simulated prior" or "measured" (the gate was passed on the user's
    /// own held-out sessions). A learned order is never shown as more than it is.
    fn ranking_label(&self) -> &'static str {
        let g = &self.advisors.guidance;
        if !(g.settings.use_learned && g.learned_available().is_ok()) {
            return "rules";
        }
        match g.bundle.as_ref().map(|b| b.report.evidence) {
            Some(Evidence::Real) => "measured",
            Some(Evidence::Simulated) => "simulated prior",
            None => "rules",
        }
    }

    /// The card: what is wrong (or worth checking), the fixes best first, and the questions whose answer settles it.
    pub(super) fn guidance_card(&self) -> Option<Card> {
        let findings = self.findings();
        if findings.is_empty() {
            return None;
        }
        let failed = self.failure.is_some();
        let mut items = Vec::new();
        for (fi, d) in findings.iter().enumerate() {
            if fi > 0 {
                items.push(DockItem::Gap);
            }
            items.push(DockItem::text(d.class.label(), Tone::Title));
            items.extend(wrap(&d.summary, 40).into_iter().map(|l| DockItem::text(l, if failed { Tone::Warn } else { Tone::Normal })));
            for e in &d.evidence {
                items.extend(wrap(e, 38).into_iter().map(|l| DockItem::text(format!("  {l}"), Tone::Dim)));
            }
            // Which comes first, the fixes or the question, is the guidance ranker's call (rules: the fixes).
            let ask_first = ask_first_in(&self.advisors.guidance.order(&d.class, &offered_kinds(d)).0);
            let mut fixes = Vec::new();
            if !d.remedies.is_empty() {
                fixes.push(DockItem::text(format!("Fixes (order: {})", self.ranking_label()), Tone::Dim));
                for (ri, r) in d.remedies.iter().enumerate() {
                    fixes.push(DockItem::row_with_preview(format!("{}. {}", ri + 1, r.label), Tone::On, ui(SolidAction::Remedy(fi, ri)), ui(SolidAction::PreviewFix(Some(FixRef::Remedy(fi, ri)))), ui(SolidAction::PreviewFix(None))));
                    fixes.extend(wrap(&r.why, 36).into_iter().map(|l| DockItem::text(format!("     {l}"), Tone::Dim)));
                }
                fixes.push(DockItem::text("click to apply (Ctrl/Cmd+Z undoes)", Tone::Dim));
            }
            let mut asks = Vec::new();
            for (qi, q) in d.questions.iter().enumerate() {
                asks.extend(wrap(&q.text, 40).into_iter().map(|l| DockItem::text(l, Tone::Normal)));
                for (oi, (label, _)) in q.options.iter().enumerate() {
                    asks.push(DockItem::row_with_preview(format!("  > {label}"), Tone::On, ui(SolidAction::Answer(fi, qi, oi)), ui(SolidAction::PreviewFix(Some(FixRef::Answer(fi, qi, oi)))), ui(SolidAction::PreviewFix(None))));
                }
            }
            if ask_first {
                items.extend(asks);
                items.extend(fixes);
            } else {
                items.extend(fixes);
                items.extend(asks);
            }
        }
        // A problem found before the solve (a forecast of absurd deformation, a free rigid-body motion) is as important as a
        // failure: it is the run that would be wasted.
        let predicted = findings.iter().any(|d| d.summary.starts_with("Before solving"));
        Some(Card::new("guidance", if failed { "Solve failed" } else if predicted { "Check before solving" } else { "Check" }, if failed || predicted { Relevance::Primary } else { Relevance::Context }, items))
    }

    /// The one-click fix the status bar offers after a failed solve: the best remedy of the first finding.
    pub(super) fn status_fix_action(&self) -> Option<(String, UiAction)> {
        self.failure.as_ref()?;
        let d = self.findings().into_iter().next()?;
        let r = d.remedies.first()?;
        Some((r.label.clone(), ui(SolidAction::Remedy(0, 0))))
    }

    /// Surfaces the fix under the pointer would touch (empty when none is previewed or it no longer exists).
    pub(super) fn fix_preview_surfaces(&self) -> Vec<String> {
        let Some(fix) = self.fix_preview else { return Vec::new() };
        let findings = self.findings();
        let edit = match fix {
            FixRef::Remedy(f, r) => findings.get(f).and_then(|d| d.remedies.get(r)).map(|r| &r.edit),
            FixRef::Answer(f, q, o) => findings.get(f).and_then(|d| d.questions.get(q)).and_then(|q| q.options.get(o)).map(|(_, e)| e),
        };
        edit.map(|e| e.surfaces(&self.problem)).unwrap_or_default()
    }

    fn apply_edit(&mut self, edit: &Edit, label: &str) -> bool {
        let mut next = self.problem.clone();
        match edit.apply(&mut next) {
            Ok(()) => {
                self.edit(|p| *p = next);
                self.fix_preview = None;
                self.status = format!("{label} (undoable). Solve again.");
                true
            }
            Err(e) => {
                self.status = e;
                false
            }
        }
    }

    /// Record which of the offered things (the fixes, answering the question) the user took, when recording is on.
    fn record_guidance_choice(&mut self, d: &Diagnosis, taken: RemedyKind) {
        if let Ok(Some(fit)) = self.advisors.guidance.record(d.class, offered_kinds(d), Some(taken)) {
            self.advisors.last_fit = Some(fit);
        }
    }

    pub(super) fn do_remedy(&mut self, finding: usize, remedy: usize) {
        let findings = self.findings();
        let Some((d, r)) = findings.get(finding).and_then(|d| d.remedies.get(remedy).map(|r| (d.clone(), r.clone()))) else {
            self.status = "That fix no longer applies.".into();
            return;
        };
        if self.apply_edit(&r.edit, &r.label) {
            self.record_guidance_choice(&d, r.kind);
        }
    }

    pub(super) fn do_answer(&mut self, finding: usize, question: usize, option: usize) {
        let findings = self.findings();
        let Some((d, label, edit)) = findings.get(finding).and_then(|d| d.questions.get(question).and_then(|q| q.options.get(option)).map(|(l, e)| (d.clone(), l.clone(), e.clone()))) else {
            self.status = "That question no longer applies.".into();
            return;
        };
        if self.apply_edit(&edit, &format!("Answered '{label}'")) {
            self.record_guidance_choice(&d, RemedyKind::AnswerQuestion);
        }
    }
}
