//! Advisory ranking of the next editor action (Phase 22). It can only *reorder actions the
//! deterministic capability rules already allow*; it never creates edits, never relaxes a rule and
//! never replaces numerical acceptance. Two rankers exist: the rule-based baseline (the default,
//! labelled "rules") and a count-based model that is trained from recorded sessions (labelled
//! "learned"). The learned one falls back to the baseline for contexts it has not seen, and is only
//! used when explicitly enabled; no trained model or session data ships with the application.
use super::{
    camera::Hit,
    caps::{self, Operation},
    session::Session,
    state::Tool,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of the trace / model file formats below.
pub const ADVISORY_VERSION: u32 = 1;
/// A learned ranking needs this many observations of the exact context, otherwise it defers.
pub const MIN_SUPPORT: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stage {
    Empty,
    HasNodes,
    HasMembers,
    Supported,
    Loaded,
    Solved,
    Stale,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Selected {
    Nothing,
    Node,
    Truss,
    Frame,
}
/// The only information a ranker sees: coarse, content-free state (no coordinates, names or values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Context {
    pub stage: Stage,
    pub selected: Selected,
    pub solving: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Suggestion {
    Tool(Tool),
    Op(Operation),
}

impl Context {
    pub fn of(session: &Session) -> Self {
        let p = &session.state.problem;
        let stage = if p.nodes.is_empty() {
            Stage::Empty
        } else if p.members.is_empty() {
            Stage::HasNodes
        } else if session.state.current_solution().is_some() {
            Stage::Solved
        } else if session.state.solution_is_stale() {
            Stage::Stale
        } else if p.supports.is_empty() {
            Stage::HasMembers
        } else if p.loads.is_empty() {
            Stage::Supported
        } else {
            Stage::Loaded
        };
        let selected = match session.state.selection {
            None => Selected::Nothing,
            Some(Hit::Node(_)) => Selected::Node,
            Some(Hit::Member(id)) => match p.member(id).map(|m| m.kind.is_frame()) {
                Some(true) => Selected::Frame,
                _ => Selected::Truss,
            },
        };
        Self { stage, selected, solving: session.solving() }
    }
}

/// Everything the capability rules allow right now (all tools; operations that are available).
pub fn allowed(session: &Session) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Tool::ALL.iter().map(|t| Suggestion::Tool(*t)).collect();
    out.extend(caps::all(&session.state, session.solving()).into_iter().filter(|(_, a)| a.is_ok()).map(|(op, _)| Suggestion::Op(op)));
    out
}

pub trait Advisor {
    fn label(&self) -> String;
    /// A permutation (or subset, in preference order) of `allowed`, or `None` to defer.
    fn rank(&self, context: &Context, allowed: &[Suggestion]) -> Option<Vec<Suggestion>>;
}

/// Deterministic workflow order per stage; the reference every learned ranker must beat.
pub struct Baseline;
impl Baseline {
    fn preferred(context: &Context) -> Vec<Suggestion> {
        use Operation as O;
        use Suggestion::{Op, Tool as T};
        if context.solving {
            return vec![Op(O::CancelSolve)];
        }
        match context.stage {
            Stage::Empty => vec![T(Tool::Node)],
            Stage::HasNodes => vec![T(Tool::Truss), T(Tool::Frame), T(Tool::Node)],
            Stage::HasMembers => vec![T(Tool::Support)],
            Stage::Supported => vec![T(Tool::Load)],
            Stage::Loaded | Stage::Stale => vec![Op(O::Solve)],
            Stage::Solved => vec![Op(O::ShowResults), Op(O::Save)],
        }
    }
}
impl Advisor for Baseline {
    fn label(&self) -> String {
        "rules".into()
    }
    fn rank(&self, context: &Context, allowed: &[Suggestion]) -> Option<Vec<Suggestion>> {
        let mut out: Vec<Suggestion> = Self::preferred(context).into_iter().filter(|s| allowed.contains(s)).collect();
        let rest: Vec<Suggestion> = allowed.iter().filter(|s| !out.contains(s)).copied().collect();
        out.extend(rest);
        Some(out)
    }
}

/// One recorded user action in its context.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub context: Context,
    pub taken: Suggestion,
}
/// Recorded sessions with provenance; the unit a model is trained and evaluated on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    pub version: u32,
    /// Where the data came from (who recorded it, how); never empty for a usable trace.
    pub provenance: String,
    pub sessions: u32,
    pub events: Vec<Event>,
}
impl Trace {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != ADVISORY_VERSION || self.provenance.trim().is_empty() || self.sessions == 0 || self.events.is_empty() {
            return Err("advisory trace needs the current version, provenance, sessions and events".into());
        }
        Ok(())
    }
}

/// Count-based model: how often each action followed each exact context. Deliberately tiny and
/// inspectable; it exists so ranking can be *measured* against [`Baseline`], not to be trusted by default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CountModel {
    pub version: u32,
    pub provenance: String,
    pub sessions: u32,
    counts: BTreeMap<String, BTreeMap<String, u32>>,
}
fn key<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("plain enums serialise")
}
impl CountModel {
    pub fn train(trace: &Trace) -> Result<Self, String> {
        trace.validate()?;
        let mut counts: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
        for e in &trace.events {
            *counts.entry(key(&e.context)).or_default().entry(key(&e.taken)).or_default() += 1;
        }
        Ok(Self { version: ADVISORY_VERSION, provenance: trace.provenance.clone(), sessions: trace.sessions, counts })
    }
    pub fn observations(&self, context: &Context) -> u32 {
        self.counts.get(&key(context)).map_or(0, |m| m.values().sum())
    }
}
impl Advisor for CountModel {
    fn label(&self) -> String {
        format!("learned v{} ({} sessions, {})", self.version, self.sessions, self.provenance)
    }
    fn rank(&self, context: &Context, allowed: &[Suggestion]) -> Option<Vec<Suggestion>> {
        if self.version != ADVISORY_VERSION || self.observations(context) < MIN_SUPPORT {
            return None; // out of distribution: defer to the rules
        }
        let seen = &self.counts[&key(context)];
        let base = Baseline.rank(context, allowed)?;
        let mut scored: Vec<(u32, usize, Suggestion)> = base.iter().enumerate().map(|(i, s)| (seen.get(&key(s)).copied().unwrap_or(0), i, *s)).collect();
        // Most frequent first; the baseline order breaks ties so the result is deterministic.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        Some(scored.into_iter().map(|(_, _, s)| s).collect())
    }
}

impl Suggestion {
    pub fn label(&self) -> String {
        match self {
            Suggestion::Tool(t) => format!("tool {} {}", t.key(), t.label()),
            Suggestion::Op(o) => o.label().to_string(),
        }
    }
}

/// Advisor chosen at start-up: `FEA_EDITOR_ADVISOR=off` hides suggestions (opt-out), otherwise a
/// `CountModel` JSON at `FEA_EDITOR_MODEL` is used when present and valid. Returns the advisor and a
/// note for the status line when a requested model could not be used (the rules remain active).
pub fn from_environment() -> (Option<Box<dyn Advisor>>, bool, Option<String>) {
    if std::env::var("FEA_EDITOR_ADVISOR").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        return (None, false, None);
    }
    let Ok(path) = std::env::var("FEA_EDITOR_MODEL") else { return (None, true, None) };
    let loaded = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str::<CountModel>(&t).map_err(|e| e.to_string()))
        .and_then(|m| if m.version == ADVISORY_VERSION { Ok(m) } else { Err("unsupported model version".to_string()) });
    match loaded {
        Ok(m) => (Some(Box::new(m)), true, None),
        Err(e) => (None, true, Some(format!("Advisory model {path} not used ({e}); showing rule-based suggestions."))),
    }
}

/// The ranking a user would see: the advisor's if it is valid, otherwise the baseline's. An advisor
/// can reorder `allowed` but never add to it or drop required safety (Cancel while solving).
pub fn suggest(advisor: Option<&dyn Advisor>, context: &Context, allowed: &[Suggestion]) -> (String, Vec<Suggestion>) {
    let fallback = || ("rules".to_string(), Baseline.rank(context, allowed).unwrap_or_default());
    let Some(a) = advisor else { return fallback() };
    match a.rank(context, allowed) {
        Some(r) if !r.is_empty() && r.iter().all(|s| allowed.contains(s)) && has_no_duplicates(&r) => (a.label(), r),
        _ => fallback(),
    }
}
fn has_no_duplicates(r: &[Suggestion]) -> bool {
    r.iter().enumerate().all(|(i, s)| !r[..i].contains(s))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub events: usize,
    /// Fraction of events where the taken action was ranked first / within the first three.
    pub top1: f64,
    pub top3: f64,
    /// Fraction of events the advisor answered itself (the rest fell back to the rules).
    pub coverage: f64,
}
/// Score `advisor` on held-out events. Events whose taken action is not currently allowed are skipped
/// (they say nothing about ranking among valid actions).
pub fn evaluate(advisor: Option<&dyn Advisor>, events: &[(Context, Vec<Suggestion>, Suggestion)]) -> Metrics {
    let mut m = Metrics::default();
    let (mut t1, mut t3, mut answered) = (0usize, 0usize, 0usize);
    for (context, allowed, taken) in events {
        if !allowed.contains(taken) {
            continue;
        }
        m.events += 1;
        let (label, ranking) = suggest(advisor, context, allowed);
        answered += usize::from(advisor.is_some_and(|a| label == a.label()));
        if let Some(pos) = ranking.iter().position(|s| s == taken) {
            t1 += usize::from(pos == 0);
            t3 += usize::from(pos < 3);
        }
    }
    if m.events > 0 {
        let n = m.events as f64;
        m.top1 = t1 as f64 / n;
        m.top3 = t3 as f64 / n;
        m.coverage = answered as f64 / n;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::editor::session::Mods;
    use fea_problem::members::{blank_planar, templates};

    fn ctx(stage: Stage) -> Context {
        Context { stage, selected: Selected::Nothing, solving: false }
    }
    fn session(p: fea_problem::members::MemberProblem) -> Session {
        Session::new(p, "x.json".into())
    }

    #[test]
    fn context_follows_the_authoring_stage_and_is_content_free() {
        let mut s = session(blank_planar());
        assert_eq!(Context::of(&s).stage, Stage::Empty);
        let t = templates();
        let mut s2 = session(t[0].clone());
        assert_eq!(Context::of(&s2).stage, Stage::Loaded);
        s2.run(Operation::Solve);
        while !s2.poll() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(Context::of(&s2).stage, Stage::Solved);
        s2.state.select(Some(Hit::Node(4)));
        s2.state.set_property(super::super::state::PropertyKind::Force(2), -1.0).unwrap();
        assert_eq!((Context::of(&s2).stage, Context::of(&s2).selected), (Stage::Stale, Selected::Node));
        s.state.select(None);
        let json = serde_json::to_string(&Context::of(&s)).unwrap();
        assert!(!json.contains("pos") && !json.contains("name"), "{json}");
        let _ = Mods::default();
    }
    #[test]
    fn baseline_orders_allowed_actions_by_workflow_stage() {
        let s = session(templates().remove(0));
        let a = allowed(&s);
        let (label, r) = suggest(None, &Context::of(&s), &a);
        assert_eq!(label, "rules");
        assert_eq!(r[0], Suggestion::Op(Operation::Solve), "loaded model: solve first");
        assert_eq!(r.len(), a.len(), "a permutation of the allowed set");
        let empty = session(blank_planar());
        let r = suggest(None, &Context::of(&empty), &allowed(&empty)).1;
        assert_eq!(r[0], Suggestion::Tool(Tool::Node));
        // While solving, only cancelling is preferred; nothing unavailable is ever suggested.
        let c = Context { solving: true, ..ctx(Stage::Loaded) };
        let a = vec![Suggestion::Op(Operation::CancelSolve), Suggestion::Tool(Tool::Select)];
        assert_eq!(suggest(None, &c, &a).1[0], Suggestion::Op(Operation::CancelSolve));
    }

    fn trace(events: Vec<Event>) -> Trace {
        Trace { version: ADVISORY_VERSION, provenance: "unit test fixture".into(), sessions: 3, events }
    }
    fn event(stage: Stage, taken: Suggestion) -> Event {
        Event { context: ctx(stage), taken }
    }

    #[test]
    fn learned_model_reorders_only_allowed_actions_and_defers_when_unsure() {
        let allowed = vec![Suggestion::Op(Operation::Solve), Suggestion::Op(Operation::Save), Suggestion::Tool(Tool::Load)];
        // Users in this fixture save before solving: the model learns that preference.
        let events: Vec<Event> = (0..8).map(|_| event(Stage::Loaded, Suggestion::Op(Operation::Save))).chain((0..2).map(|_| event(Stage::Loaded, Suggestion::Op(Operation::Solve)))).collect();
        let model = CountModel::train(&trace(events)).unwrap();
        let (label, r) = suggest(Some(&model), &ctx(Stage::Loaded), &allowed);
        assert!(label.starts_with("learned"), "{label}");
        assert_eq!(r[0], Suggestion::Op(Operation::Save));
        assert!(r.iter().all(|s| allowed.contains(s)) && r.len() == allowed.len());
        // A disallowed learned favourite is never surfaced.
        let only_solve = vec![Suggestion::Op(Operation::Solve)];
        assert_eq!(suggest(Some(&model), &ctx(Stage::Loaded), &only_solve).1, only_solve);
        // Unseen / thinly seen contexts fall back to the rules (out of distribution).
        for c in [ctx(Stage::Empty), Context { selected: Selected::Frame, ..ctx(Stage::Loaded) }] {
            let (label, _) = suggest(Some(&model), &c, &allowed);
            assert_eq!(label, "rules", "{c:?}");
        }
        assert!(model.observations(&ctx(Stage::Loaded)) >= MIN_SUPPORT);
    }
    #[test]
    fn invalid_advisor_output_is_rejected_in_favour_of_the_rules() {
        struct Rogue(Vec<Suggestion>);
        impl Advisor for Rogue {
            fn label(&self) -> String {
                "rogue".into()
            }
            fn rank(&self, _: &Context, _: &[Suggestion]) -> Option<Vec<Suggestion>> {
                Some(self.0.clone())
            }
        }
        let allowed = vec![Suggestion::Op(Operation::Solve), Suggestion::Op(Operation::Save)];
        let c = ctx(Stage::Loaded);
        for bad in [vec![Suggestion::Op(Operation::Delete)], vec![], vec![Suggestion::Op(Operation::Solve), Suggestion::Op(Operation::Solve)]] {
            assert_eq!(suggest(Some(&Rogue(bad)), &c, &allowed).0, "rules");
        }
        assert_eq!(suggest(Some(&Rogue(vec![Suggestion::Op(Operation::Save)])), &c, &allowed).0, "rogue", "a valid subset is accepted");
    }
    #[test]
    fn training_data_and_models_round_trip_and_are_validated() {
        let t = trace(vec![event(Stage::Empty, Suggestion::Tool(Tool::Node)); 6]);
        let model = CountModel::train(&t).unwrap();
        let back: CountModel = serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
        assert_eq!(back, model);
        assert!(CountModel::train(&Trace { provenance: " ".into(), ..t.clone() }).is_err());
        assert!(CountModel::train(&Trace { events: vec![], ..t.clone() }).is_err());
        assert!(CountModel::train(&Trace { version: 99, ..t }).is_err());
        let stale = CountModel { version: 99, ..model };
        assert!(stale.rank(&ctx(Stage::Empty), &[Suggestion::Tool(Tool::Node)]).is_none(), "unknown model version defers");
    }
    #[test]
    fn evaluation_measures_a_learned_ranker_against_the_baseline_on_held_out_and_ood_events() {
        use Operation as O;
        // Synthetic fixture (NOT user data): in this population solving is preceded by saving.
        let allowed = vec![Suggestion::Op(O::Solve), Suggestion::Op(O::Save), Suggestion::Tool(Tool::Select)];
        let train: Vec<Event> = (0..40).map(|_| event(Stage::Loaded, Suggestion::Op(O::Save))).collect();
        let model = CountModel::train(&trace(train)).unwrap();
        let held_out: Vec<_> = (0..10).map(|_| (ctx(Stage::Loaded), allowed.clone(), Suggestion::Op(O::Save))).collect();
        let b = evaluate(None, &held_out);
        let l = evaluate(Some(&model), &held_out);
        assert_eq!((b.events, l.events), (10, 10));
        assert_eq!(b.top1, 0.0, "the rules put Solve first");
        assert_eq!((l.top1, l.coverage), (1.0, 1.0), "on its own distribution the model wins");
        // Out of distribution (a context never seen in training): coverage is 0 and the result equals the baseline's.
        let ood: Vec<_> = (0..10).map(|_| (ctx(Stage::Stale), vec![Suggestion::Op(O::Solve), Suggestion::Op(O::Undo)], Suggestion::Op(O::Solve))).collect();
        let (b, l) = (evaluate(None, &ood), evaluate(Some(&model), &ood));
        assert_eq!((l.coverage, l.top1, l.top3), (0.0, b.top1, b.top3));
        // Events whose taken action was not allowed are skipped, not scored as misses.
        let skipped = vec![(ctx(Stage::Loaded), vec![Suggestion::Op(O::Solve)], Suggestion::Op(O::Delete))];
        assert_eq!(evaluate(Some(&model), &skipped).events, 0);
    }
}
