//! Recommendations and the opt-in learned ranking for the solid editor: sketch constraint suggestions and
//! boundary-condition kinds. The rules (`fea_problem::sketch::infer`, `fea_problem::advice`) decide what is offered;
//! the rankers of `editor::rank` may only reorder it, and only after the measured gate. Recording is off by default.
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        rank::{Ranker, Store},
        session::{EditorAction, Tone},
        sketcher::ConstraintSituation,
    },
    menu::{Entry, Item},
    text::wrap,
    ui::DockItem,
};
use fea_problem::{
    advice::{self, BcKind, SurfaceKind},
    sketch::infer::Rule,
    Analysis,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Count {
    None,
    One,
    Several,
}
impl Count {
    fn of(n: usize) -> Self {
        match n {
            0 => Self::None,
            1 => Self::One,
            _ => Self::Several,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AnalysisClass {
    Plane,
    Axisymmetric,
    Solid,
}

/// Coarse, content-free state the boundary-condition ranker learns from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BcSituation {
    pub analysis: AnalysisClass,
    pub supports: Count,
    pub loads: Count,
    pub surface: SurfaceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvisorOp {
    ToggleRecord,
    ToggleLearned,
    Train,
    DeleteRecordings,
    ToggleShow,
    /// Generate the simulated sessions (a prior) for both rankers and refit them.
    BuildCorpus,
    ToggleAutoTrain,
}

pub(super) struct Advisors {
    pub(super) constraints: Ranker<ConstraintSituation, Rule>,
    pub(super) boundary: Ranker<BcSituation, BcKind>,
    pub(super) refinement: Ranker<super::refining::RefineSituation, super::refining::RefineLevel>,
    /// Order of the fixes offered after a failed solve (situation: the failure class; candidate: the kind of fix).
    pub(super) guidance: Ranker<fea_problem::diagnose::FailureClass, fea_problem::diagnose::RemedyKind>,
    /// Show the recommendations panel.
    pub(super) show: bool,
    /// What the last boundary popup offered, for recording the choice.
    bc_offer: std::cell::RefCell<Option<(BcSituation, Vec<BcKind>)>>,
    /// The latest refit made while learning from the user's own sessions.
    pub(super) last_fit: Option<String>,
}

impl Advisors {
    /// Rankers whose files live in `dir` (tests; the application uses the bench's app-data folder).
    #[cfg(test)]
    pub(super) fn in_dir(dir: std::path::PathBuf) -> Self {
        let mut a = Self::new();
        a.constraints = Ranker::new(Some(Store::new(dir.clone(), "constraints")), 1, "Constraints");
        a.boundary = Ranker::new(Some(Store::new(dir.clone(), "boundary")), 1, "Boundary conditions");
        a.guidance = Ranker::new(Some(Store::new(dir.clone(), "guidance")), 1, "Failure fixes");
        a.refinement = Ranker::new(Some(Store::new(dir, "refinement")), 1, "Mesh density");
        a
    }

    pub(super) fn new() -> Self {
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        // Unit tests never touch the application-data folder.
        let store = |name| if cfg!(test) { None } else { Store::for_bench(Bench::Static, name) };
        Self { constraints: Ranker::new(store("constraints"), id, "Constraints"), boundary: Ranker::new(store("boundary"), id, "Boundary conditions"), refinement: Ranker::new(store("refinement"), id, "Mesh density"), guidance: Ranker::new(store("guidance"), id, "Failure fixes"), show: true, bc_offer: std::cell::RefCell::new(None), last_fit: None }
    }
}

/// The coarse situation of offering a boundary condition on `surface` of `p`.
pub(super) fn bc_situation_of(p: &Problem, surface: &str) -> BcSituation {
    BcSituation {
        analysis: match p.analysis {
            Analysis::PlaneStress | Analysis::PlaneStrain => AnalysisClass::Plane,
            Analysis::Axisymmetric => AnalysisClass::Axisymmetric,
            Analysis::Solid => AnalysisClass::Solid,
        },
        supports: Count::of(p.supports.len()),
        loads: Count::of(p.loads.len()),
        surface: advice::surface_kind(surface),
    }
}

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

impl SolidSession {
    pub(super) fn bc_situation(&self, surface: &str) -> BcSituation {
        bc_situation_of(&self.problem, surface)
    }

    /// Boundary-condition kinds to offer on `surface`, ranked (and remembered, to record the choice).
    pub(super) fn boundary_offer(&self, surface: &str) -> Vec<BcKind> {
        let options = advice::boundary_options(&self.problem, surface);
        let sit = self.bc_situation(surface);
        let (ordered, _) = self.advisors.boundary.order(&sit, &options);
        *self.advisors.bc_offer.borrow_mut() = Some((sit, options));
        ordered
    }

    /// Add the boundary condition `kind` on every selected surface and record the choice.
    pub(super) fn add_boundary_kind(&mut self, kind: BcKind) {
        let names = self.all_selected();
        if names.is_empty() {
            self.status = "Pick a surface in the viewport or the tree first.".into();
            return;
        }
        let mut errors = Vec::new();
        let mut next = self.problem.clone();
        for n in &names {
            if let Err(e) = advice::add_boundary(&mut next, n, kind) {
                errors.push(format!("{n}: {e}"));
            }
        }
        if !errors.is_empty() {
            self.status = errors.join("; ");
            return;
        }
        self.edit(|p| *p = next);
        self.record_boundary_choice(kind);
        self.status = format!("Added a {} on {}.", kind.label(), names.join(", "));
    }

    /// Remember (when recording is on) which of the offered kinds was taken.
    pub(super) fn record_boundary_choice(&mut self, kind: BcKind) {
        let offer = self.advisors.bc_offer.borrow_mut().take();
        if let Some((sit, offered)) = offer {
            if let Ok(Some(fit)) = self.advisors.boundary.record(sit, offered, Some(kind)) {
                self.advisors.last_fit = Some(fit);
            }
        }
    }

    /// What can be done to the selected surface(s), in one list: the constraints that fit them, the loads that fit them
    /// (each opens its value card), the thermal conditions of a thermal study, then what is already applied there (edit,
    /// remove). The right-click menu of a face and of a tree row. The advisor still orders what it offers: the section
    /// of the kind it ranks first comes first.
    pub(super) fn surface_menu(&self) -> Vec<Item> {
        use super::cards::CardForm;
        let names = self.all_selected();
        let label = if names.len() == 1 { format!("'{}'", names[0]) } else { format!("{} surfaces", names.len()) };
        let entry = |text: String, a: SolidAction| Item::Entry(Entry::new(text, "", ui(a)));
        let offer = self.boundary_offer(names.first().map_or("", String::as_str));
        let (mut constrain, mut load) = (Vec::new(), Vec::new());
        for k in &offer {
            match k {
                BcKind::Fixed => {
                    constrain.push(entry("Fixed".into(), SolidAction::BoundaryKind(BcKind::Fixed)));
                    constrain.push(entry("Hold directions...".into(), SolidAction::OpenCard(CardForm::Hold)));
                }
                BcKind::Roller => constrain.push(entry("Slides on the face".into(), SolidAction::BoundaryKind(BcKind::Roller))),
                BcKind::Pressure => {
                    load.push(entry("Pressure...".into(), SolidAction::OpenCard(CardForm::Pressure)));
                    load.push(entry("Force...".into(), SolidAction::OpenCard(CardForm::Force)));
                    load.push(entry("Traction...".into(), SolidAction::OpenCard(CardForm::Traction)));
                }
                BcKind::Bearing => load.push(entry("Pin load...".into(), SolidAction::OpenCard(CardForm::Bearing))),
                _ => {}
            }
        }
        let thermal: Vec<Item> = offer.iter().filter(|k| matches!(k, BcKind::FixedTemperature | BcKind::HeatFlux | BcKind::Convection)).map(|k| entry(format!("{}", capital(k.label())), SolidAction::BoundaryKind(*k))).collect();
        let first_load = offer.iter().position(|k| matches!(k, BcKind::Pressure | BcKind::Bearing));
        let first_constrain = offer.iter().position(|k| matches!(k, BcKind::Fixed | BcKind::Roller));
        let sections = if first_load < first_constrain { [("Load", load), ("Constrain", constrain)] } else { [("Constrain", constrain), ("Load", load)] };
        let mut items: Vec<Item> = Vec::new();
        for (title, entries) in sections {
            if !entries.is_empty() {
                if !items.is_empty() {
                    items.push(Item::Separator);
                }
                items.push(Item::Heading(format!("{title} {label}")));
                items.extend(entries);
            }
        }
        if !thermal.is_empty() {
            if !items.is_empty() {
                items.push(Item::Separator);
            }
            items.push(Item::Heading(format!("Thermal {label}")));
            items.extend(thermal);
        }
        let on = |edge: Option<&str>| edge.is_some_and(|e| names.iter().any(|n| n == e));
        let mut applied = Vec::new();
        for (i, sup) in self.problem.supports.iter().enumerate() {
            if on(sup.edge()) {
                applied.push(entry(format!("Change directions on '{}'...", sup.edge().unwrap_or_default()), SolidAction::OpenCard(CardForm::EditHold(i))));
                applied.push(entry(format!("Remove support on '{}'", sup.edge().unwrap_or_default()), SolidAction::Activate(FieldRow::SupportRemove(i))));
                applied.extend(self.convert_items(true, i));
            }
        }
        for (i, load) in self.problem.loads.iter().enumerate() {
            if on(load.edge()) {
                let edge = load.edge().unwrap_or_default();
                applied.push(entry(format!("Edit the load on '{edge}'..."), SolidAction::OpenCard(CardForm::EditLoad(i))));
                applied.push(entry(format!("Remove the load on '{edge}'"), SolidAction::Activate(FieldRow::LoadRemove(i))));
                applied.extend(self.convert_items(false, i));
            }
        }
        if !applied.is_empty() {
            items.push(Item::Separator);
            items.push(Item::Heading(format!("Already on {label}")));
            items.extend(applied);
        }
        items
    }

    /// Entries that change support `i` / load `i` to another kind (the current kind is not offered).
    pub(super) fn convert_items(&self, support: bool, i: usize) -> Vec<Item> {
        use fea_problem::conversion::{LoadKind, SupportKind};
        let entry = |text: String, a: SolidAction| Item::Entry(Entry::new(text, "", ui(a)));
        let mut out = Vec::new();
        if support {
            let Some(s) = self.problem.supports.get(i) else { return out };
            let name = s.edge().map_or("a point".to_string(), |e| format!("'{e}'"));
            out.push(Item::Heading(format!("Change the support on {name} to")));
            for k in SupportKind::ALL {
                let current = match (k, s) {
                    (SupportKind::Point, fea_problem::Support::Point { .. }) => true,
                    (SupportKind::Fixed, fea_problem::Support::Edge { .. }) => s.comps().iter().take(self.problem.analysis_dim()).all(|c| c.is_some()),
                    (SupportKind::Slides, fea_problem::Support::Edge { .. }) => s.comps().iter().take(self.problem.analysis_dim()).filter(|c| c.is_some()).count() == 1,
                    _ => false,
                };
                if !current {
                    out.push(entry(k.label().to_string(), SolidAction::ConvertSupport(i, k)));
                }
            }
        } else {
            let Some(l) = self.problem.loads.get(i) else { return out };
            let Some(now) = LoadKind::of(l) else { return out };
            let name = l.edge().map_or("a point".to_string(), |e| format!("'{e}'"));
            out.push(Item::Heading(format!("Change the load on {name} to")));
            let plane = matches!(self.problem.analysis, Analysis::PlaneStress | Analysis::PlaneStrain);
            for k in LoadKind::ALL {
                if k != now && (k != LoadKind::Bearing || plane) {
                    out.push(entry(k.label().to_string(), SolidAction::ConvertLoad(i, k)));
                }
            }
        }
        out
    }

    /// Items of the surface popup a click on the model asks for.
    pub(super) fn boundary_items(&mut self) -> Vec<Item> {
        let mut items = self.surface_menu();
        items.push(Item::Entry(Entry::new("Assign to focused row", "", ui(SolidAction::AssignSelection)).when(self.assign_availability())));
        items
    }

    pub(super) fn do_advice(&mut self, index: usize) {
        let list = advice::recommend(&self.problem);
        let Some(a) = list.get(index) else {
            self.status = "That recommendation no longer applies.".into();
            return;
        };
        let mut next = self.problem.clone();
        match a.apply(&mut next) {
            Ok(()) => {
                let label = a.label();
                self.edit(|p| *p = next);
                self.status = format!("{label} (undoable).");
            }
            Err(e) => self.status = e,
        }
    }

    /// The recommendations panel for the model as it is (not shown while sketching: the geometry is mid-edit).
    pub(super) fn advice_panel(&self) -> Vec<DockItem> {
        if !self.advisors.show || self.sketching.is_some() || matches!(self.problem.geometry, Geometry::Imported { .. }) {
            return Vec::new();
        }
        let list = advice::recommend(&self.problem);
        let learning = self.learning_lines();
        if list.is_empty() && learning.is_empty() {
            return Vec::new();
        }
        let mut out = vec![DockItem::Gap, DockItem::Heading("Recommendations".into())];
        for (i, a) in list.iter().enumerate() {
            out.push(DockItem::row(a.label(), Tone::On, Some(ui(SolidAction::Advice(i))), false));
            for l in wrap(a.reason(), 38) {
                out.push(DockItem::text(format!("  {l}"), Tone::Dim));
            }
        }
        if !list.is_empty() {
            out.push(DockItem::text("click to apply (Ctrl/Cmd+Z undoes)", Tone::Dim));
        }
        out.extend(learning);
        out
    }

    /// One line per trained ranker: what it was measured on and whether it is in use.
    fn learning_lines(&self) -> Vec<DockItem> {
        fn brief<S: crate::gpu_viewer::editor::rank::Label, C: crate::gpu_viewer::editor::rank::Label>(r: &Ranker<S, C>) -> Option<DockItem> {
            use crate::gpu_viewer::editor::rank::Evidence;
            let rep = &r.bundle.as_ref()?.report;
            let evidence = match rep.evidence {
                Evidence::Real => "your sessions",
                Evidence::Simulated => "simulated prior",
            };
            let state = if r.settings.use_learned && r.learned_available().is_ok() { "in use" } else if r.learned_available().is_ok() { "ready (Advisor > Use learned ranking)" } else { "not used yet" };
            Some(DockItem::text(format!("{}: {evidence}, {state}", r.what), Tone::Dim))
        }
        let a = &self.advisors;
        let mut v: Vec<DockItem> = [brief(&a.constraints), brief(&a.boundary), brief(&a.refinement), brief(&a.guidance)].into_iter().flatten().collect();
        if let Some(f) = &a.last_fit {
            v.extend(wrap(f, 38).into_iter().map(|l| DockItem::text(l, Tone::Dim)));
        }
        if !v.is_empty() {
            v.insert(0, DockItem::text("learning:", Tone::Dim));
        }
        v
    }

    // ------------------------------------------------------------------ sketch constraint ranking and recording

    /// After a sketch edit: show the suggestions in the learned order when enabled, and record the offers' fates.
    pub(super) fn settle_constraint_advice(&mut self) {
        let Some(st) = self.sketching.as_mut() else { return };
        if let Some(sit) = st.edit.situation() {
            let rules = st.edit.offered_rules();
            if !st.edit.learned_order && rules.len() > 1 {
                let (order, learned) = self.advisors.constraints.order(&sit, &rules);
                if learned {
                    st.edit.reorder(&order);
                }
            }
        }
        for ev in st.edit.take_events() {
            if let Ok(Some(fit)) = self.advisors.constraints.record(ev.situation, ev.offered, ev.taken) {
                self.advisors.last_fit = Some(fit);
            }
        }
    }

    // ------------------------------------------------------------------ Advisor menu

    pub(super) fn do_advisor_op(&mut self, op: AdvisorOp) {
        /// Run `$body` once for each ranker (`$r` is it) and collect the three results.
        macro_rules! each {
            (|$r:ident| $body:expr) => {{
                let $r = &mut self.advisors.constraints;
                let a = $body;
                let $r = &mut self.advisors.boundary;
                let b = $body;
                let $r = &mut self.advisors.refinement;
                let c = $body;
                let $r = &mut self.advisors.guidance;
                let d = $body;
                [a, b, c, d]
            }};
        }
        /// `Err` only when every ranker failed (a ranker with nothing to learn from yet is not an error for the others).
        fn joined(results: [Result<String, String>; 4]) -> Result<String, String> {
            if results.iter().all(Result::is_err) {
                return results.into_iter().next().expect("four").and(Ok(String::new()));
            }
            Ok(results.into_iter().map(|r| r.unwrap_or_else(|e| e)).collect::<Vec<_>>().join(" "))
        }
        fn all_ok(results: [Result<(), String>; 4]) -> Result<(), String> {
            results.into_iter().collect()
        }
        let result: Result<String, String> = match op {
            AdvisorOp::ToggleShow => {
                self.advisors.show = !self.advisors.show;
                Ok(if self.advisors.show { "Recommendations shown.".into() } else { "Recommendations hidden.".to_string() })
            }
            AdvisorOp::ToggleRecord => {
                let on = !self.advisors.constraints.settings.record;
                all_ok(each!(|r| r.set_record(on))).map(|()| if on { "Recording sketch, boundary and mesh-density choices locally (content-free: no coordinates, names or values).".into() } else { "Recording off.".to_string() })
            }
            AdvisorOp::ToggleLearned => {
                let a = &self.advisors;
                let on = !(a.constraints.settings.use_learned || a.boundary.settings.use_learned || a.refinement.settings.use_learned || a.guidance.settings.use_learned);
                let results = each!(|r| r.set_use_learned(on));
                match (on, results.iter().all(Result::is_err)) {
                    (true, true) => results.into_iter().next().expect("four").map(|()| String::new()),
                    (true, false) => Ok("Learned ranking on where a model passed the gate; the rules everywhere else.".to_string()),
                    (false, _) => Ok("Learned ranking off: the rules decide the order.".to_string()),
                }
            }
            AdvisorOp::Train => joined(each!(|r| r.train_background())),
            AdvisorOp::ToggleAutoTrain => {
                let on = !self.advisors.constraints.settings.auto_train;
                all_ok(each!(|r| r.set_auto_train(on))).map(|()| if on { "Learning continues as you work: the model is refitted as your recordings grow (needs recording on).".into() } else { "Automatic refitting off: use Train from recordings.".to_string() })
            }
            AdvisorOp::BuildCorpus => {
                let corpus = corpus::build(corpus::SESSIONS, corpus::SEED);
                let installed = self.advisors.constraints.install_corpus(&corpus.constraints).and(self.advisors.boundary.install_corpus(&corpus.boundary)).and(self.advisors.refinement.install_corpus(&corpus.refinement));
                installed.and_then(|()| {
                    let fits = joined(each!(|r| r.train_background()))?;
                    Ok(format!("Built {} simulated sketch, {} boundary and {} mesh-density offers (a prior, not your data). {fits}", corpus.constraints.len(), corpus.boundary.len(), corpus.refinement.len()))
                })
            }
            AdvisorOp::DeleteRecordings => {
                all_ok(each!(|r| r.delete_recordings())).map(|()| "Recordings deleted.".to_string())
            }
        };
        self.status = result.unwrap_or_else(|e| e);
        self.findings_cache.borrow_mut().take();
        self.touch();
    }

    pub(super) fn advisor_items(&self) -> Vec<Item> {
        let a = &self.advisors;
        let op = |label: &str, op: AdvisorOp| Entry::new(label, "", ui(SolidAction::Advisor(op)));
        let learned_reason = [a.constraints.learned_available(), a.boundary.learned_available(), a.refinement.learned_available(), a.guidance.learned_available()].into_iter().find(Result::is_ok).unwrap_or_else(|| a.constraints.learned_available());
        let no_store: Result<(), String> = if a.constraints.store.is_some() { Ok(()) } else { Err("no application data folder on this platform".into()) };
        let corpus_built = a.constraints.store.as_ref().is_some_and(|s| s.has_corpus());
        let recordings = a.constraints.store.as_ref().is_some_and(|s| s.has_recordings()) || a.boundary.store.as_ref().is_some_and(|s| s.has_recordings()) || a.refinement.store.as_ref().is_some_and(|s| s.has_recordings()) || a.guidance.store.as_ref().is_some_and(|s| s.has_recordings());
        vec![
            Item::Entry(op("Show recommendations", AdvisorOp::ToggleShow).checked(a.show)),
            Item::Entry(op("Record my sketch, boundary & mesh choices (local)", AdvisorOp::ToggleRecord).checked(a.constraints.settings.record).when(no_store.clone())),
            Item::Entry(op("Use learned ranking", AdvisorOp::ToggleLearned).checked(a.constraints.settings.use_learned || a.boundary.settings.use_learned || a.refinement.settings.use_learned || a.guidance.settings.use_learned).when(learned_reason)),
            Item::Entry(op("Learn as I work (refit as recordings grow)", AdvisorOp::ToggleAutoTrain).checked(a.constraints.settings.auto_train).when(no_store.clone())),
            Item::Entry(op("Build training corpus (simulated prior)", AdvisorOp::BuildCorpus).when(no_store.clone())),
            Item::Entry(op("Train from recordings", AdvisorOp::Train).when(if recordings || corpus_built { Ok(()) } else { Err("nothing recorded yet: turn on recording and sketch, or build the training corpus".into()) })),
            Item::Entry(op("Delete recordings", AdvisorOp::DeleteRecordings).when(if recordings { Ok(()) } else { Err("there are no recordings".into()) })),
        ]
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
