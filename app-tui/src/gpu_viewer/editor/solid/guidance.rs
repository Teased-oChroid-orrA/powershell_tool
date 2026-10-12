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
    list: Vec<Diagnosis>,
}

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

impl SolidSession {
    /// Findings for the model now, remedies in the rules' (or the learned) order. Empty while a sketch is open or for an
    /// empty model: there is nothing to check yet.
    pub(super) fn findings(&self) -> Vec<Diagnosis> {
        if self.sketching.is_some() || self.problem.validate_geometry().is_err_and(|e| !e.contains("tessellated") && !e.contains("out of date")) && self.failure.is_none() {
            return Vec::new();
        }
        let learned = self.advisors.guidance.settings.use_learned && self.advisors.guidance.learned_available().is_ok();
        if let Some(c) = self.findings_cache.borrow().as_ref() {
            if c.problem == self.problem && c.error == self.failure && c.learned == learned {
                return c.list.clone();
            }
        }
        let mut list = diagnose(&self.problem, self.failure.as_deref());
        for d in &mut list {
            let kinds: Vec<RemedyKind> = d.remedies.iter().map(|r| r.kind).collect();
            let (order, _) = self.advisors.guidance.order(&d.class, &kinds);
            d.remedies.sort_by_key(|r| order.iter().position(|k| *k == r.kind).unwrap_or(usize::MAX));
        }
        *self.findings_cache.borrow_mut() = Some(Findings { problem: self.problem.clone(), error: self.failure.clone(), learned, list: list.clone() });
        list
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
            if !d.remedies.is_empty() {
                items.push(DockItem::text(format!("Fixes (order: {})", self.ranking_label()), Tone::Dim));
                for (ri, r) in d.remedies.iter().enumerate() {
                    items.push(DockItem::row(format!("{}. {}", ri + 1, r.label), Tone::On, Some(ui(SolidAction::Remedy(fi, ri))), false));
                    items.extend(wrap(&r.why, 36).into_iter().map(|l| DockItem::text(format!("     {l}"), Tone::Dim)));
                }
                items.push(DockItem::text("click to apply (Ctrl/Cmd+Z undoes)", Tone::Dim));
            }
            for (qi, q) in d.questions.iter().enumerate() {
                items.extend(wrap(&q.text, 40).into_iter().map(|l| DockItem::text(l, Tone::Normal)));
                for (oi, (label, _)) in q.options.iter().enumerate() {
                    items.push(DockItem::row(format!("  > {label}"), Tone::On, Some(ui(SolidAction::Answer(fi, qi, oi))), false));
                }
            }
        }
        Some(Card::new("guidance", if failed { "Solve failed" } else { "Check" }, if failed { Relevance::Primary } else { Relevance::Context }, items))
    }

    /// The one-click fix the status bar offers after a failed solve: the best remedy of the first finding.
    pub(super) fn status_fix_action(&self) -> Option<(String, UiAction)> {
        self.failure.as_ref()?;
        let d = self.findings().into_iter().next()?;
        let r = d.remedies.first()?;
        Some((r.label.clone(), ui(SolidAction::Remedy(0, 0))))
    }

    fn apply_edit(&mut self, edit: &Edit, label: &str) -> bool {
        let mut next = self.problem.clone();
        match edit.apply(&mut next) {
            Ok(()) => {
                self.edit(|p| *p = next);
                self.status = format!("{label} (undoable). Solve again.");
                true
            }
            Err(e) => {
                self.status = e;
                false
            }
        }
    }

    pub(super) fn do_remedy(&mut self, finding: usize, remedy: usize) {
        let findings = self.findings();
        let Some((d, r)) = findings.get(finding).and_then(|d| d.remedies.get(remedy).map(|r| (d, r))) else {
            self.status = "That fix no longer applies.".into();
            return;
        };
        let (class, offered, taken, label, edit) = (d.class, d.remedies.iter().map(|r| r.kind).collect::<Vec<_>>(), r.kind, r.label.clone(), r.edit.clone());
        if self.apply_edit(&edit, &label) {
            if let Ok(Some(fit)) = self.advisors.guidance.record(class, offered, Some(taken)) {
                self.advisors.last_fit = Some(fit);
            }
        }
    }

    pub(super) fn do_answer(&mut self, finding: usize, question: usize, option: usize) {
        let findings = self.findings();
        let Some((label, edit)) = findings.get(finding).and_then(|d| d.questions.get(question)).and_then(|q| q.options.get(option)).cloned() else {
            self.status = "That question no longer applies.".into();
            return;
        };
        self.apply_edit(&edit, &format!("Answered '{label}'"));
    }
}
