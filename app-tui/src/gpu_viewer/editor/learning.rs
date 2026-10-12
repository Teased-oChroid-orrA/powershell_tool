//! Opt-in learning loop for the advisory ranker: local content-free recording, a trainer, and the
//! gate that decides whether a trained model may be switched on. Nothing here is automatic:
//!
//! 1. *Record* (off by default): each tool/operation the user takes is appended, with its coarse
//!    [`Context`] and the set of actions the rules allowed, to a local JSON-lines file. No coordinates,
//!    names, values or paths are recorded, and nothing leaves the machine.
//! 2. *Train* (explicit): sessions are split deterministically into training and held-out sets; a
//!    [`CountModel`] is fitted on the former and compared with the rules `Baseline` on the latter.
//! 3. *Use* (explicit, gated): the model may be enabled only when the saved report says it is
//!    [`Verdict::Eligible`]: enough sessions, enough held-out events, a significant top-1 gain
//!    (paired sign test) and no top-3 regression. Contexts the model has not seen defer to the rules.
use super::advisory::{evaluate, suggest, Baseline, Context, CountModel, Event, Metrics, Suggestion, Trace, ADVISORY_VERSION, MIN_SUPPORT};
use super::advisory::Advisor;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
};

/// Distinct recorded sessions required before a model can be eligible.
pub const MIN_SESSIONS: u32 = 20;
/// Held-out events required before a comparison is considered meaningful.
pub const MIN_HELD_OUT_EVENTS: usize = 100;
/// Required paired sign-test statistic `(wins - losses) / sqrt(wins + losses)` for a top-1 gain.
pub const MIN_Z: f64 = 2.0;
/// Newest events read back for training (bounds memory on a very long recording).
const MAX_EVENTS: usize = 500_000;

/// What the user can ask the advisor to do from the Advisor menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvisorAction {
    ToggleSuggestions,
    ToggleRecording,
    ToggleLearned,
    Train,
    DeleteRecordings,
}

/// One recorded user action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recorded {
    pub session: u64,
    pub context: Context,
    /// The actions the capability rules allowed at that moment (needed to score a ranking).
    pub allowed: Vec<Suggestion>,
    pub taken: Suggestion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    /// Show ranked next-step suggestions in the editor panel.
    pub suggestions: bool,
    pub record: bool,
    pub use_learned: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self { version: ADVISORY_VERSION, suggestions: true, record: false, use_learned: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Verdict {
    Eligible,
    /// Enough data, but the model is not measurably better than the rules.
    NotBetter(String),
    Insufficient(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub sessions: u32,
    pub train_sessions: u32,
    pub held_sessions: u32,
    pub train_events: usize,
    pub held_events: usize,
    /// Held-out events whose context the model has too few observations of (it defers; equal to rules).
    pub deferred_events: usize,
    pub baseline: Metrics,
    pub learned: Metrics,
    pub wins: u32,
    pub losses: u32,
    pub z: f64,
    pub verdict: Verdict,
}
impl Report {
    pub fn summary(&self) -> String {
        let pct = |v: f64| format!("{:.0}%", 100.0 * v);
        let verdict = match &self.verdict {
            Verdict::Eligible => "eligible to use".to_string(),
            Verdict::NotBetter(w) => format!("not used: {w}"),
            Verdict::Insufficient(w) => format!("not used: {w}"),
        };
        format!(
            "Advisor: {} sessions ({} train / {} held out, {} held-out events, {} deferred); top-1 rules {} vs learned {} (wins {}, losses {}, z {:.1}); {verdict}.",
            self.sessions,
            self.train_sessions,
            self.held_sessions,
            self.held_events,
            self.deferred_events,
            pct(self.baseline.top1),
            pct(self.learned.top1),
            self.wins,
            self.losses,
            self.z
        )
    }
}

/// A trained model together with the evidence that gates it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub version: u32,
    pub model: CountModel,
    pub report: Report,
}
impl Bundle {
    /// `Ok` only when this bundle may be switched on.
    pub fn usable(&self) -> Result<(), String> {
        if self.version != ADVISORY_VERSION || self.model.version != ADVISORY_VERSION {
            return Err("the trained model is from an incompatible version; train again".into());
        }
        match &self.report.verdict {
            Verdict::Eligible => Ok(()),
            Verdict::NotBetter(w) | Verdict::Insufficient(w) => Err(format!("not enabled: {w}")),
        }
    }
}

/// Deterministic ~20% of sessions are held out (hash of the id, so recording order cannot bias it).
pub(crate) fn is_held_out(session: u64) -> bool {
    let mut z = session.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)).is_multiple_of(5)
}

/// Fit on the training sessions and measure against the rules on the held-out sessions.
pub fn train(recorded: &[Recorded]) -> Result<Bundle, String> {
    let sessions: BTreeSet<u64> = recorded.iter().map(|r| r.session).collect();
    let (held, train_ids): (BTreeSet<u64>, BTreeSet<u64>) = (
        sessions.iter().copied().filter(|s| is_held_out(*s)).collect(),
        sessions.iter().copied().filter(|s| !is_held_out(*s)).collect(),
    );
    let events: Vec<Event> = recorded.iter().filter(|r| train_ids.contains(&r.session)).map(|r| Event { context: r.context, taken: r.taken }).collect();
    if events.is_empty() {
        return Err("no recorded training sessions yet; turn on recording and use the editor".into());
    }
    let trace = Trace {
        version: ADVISORY_VERSION,
        provenance: format!("recorded locally, {} training sessions", train_ids.len()),
        sessions: train_ids.len() as u32,
        events,
    };
    let train_events = trace.events.len();
    let model = CountModel::train(&trace)?;
    let held_events: Vec<(Context, Vec<Suggestion>, Suggestion)> =
        recorded.iter().filter(|r| held.contains(&r.session)).map(|r| (r.context, r.allowed.clone(), r.taken)).collect();
    let (baseline, learned) = (evaluate(None, &held_events), evaluate(Some(&model), &held_events));
    let (mut wins, mut losses, mut deferred) = (0u32, 0u32, 0usize);
    for (context, allowed, taken) in held_events.iter().filter(|(_, a, t)| a.contains(t)) {
        let rules_hit = Baseline.rank(context, allowed).and_then(|r| r.first().copied()) == Some(*taken);
        let learned_hit = suggest(Some(&model), context, allowed).1.first() == Some(taken);
        wins += u32::from(learned_hit && !rules_hit);
        losses += u32::from(rules_hit && !learned_hit);
        deferred += usize::from(model.observations(context) < MIN_SUPPORT);
    }
    let n = f64::from(wins + losses);
    let z = if n > 0.0 { (f64::from(wins) - f64::from(losses)) / n.sqrt() } else { 0.0 };
    let total = sessions.len() as u32;
    let verdict = if total < MIN_SESSIONS {
        Verdict::Insufficient(format!("needs {MIN_SESSIONS} recorded sessions, have {total}"))
    } else if learned.events < MIN_HELD_OUT_EVENTS {
        Verdict::Insufficient(format!("needs {MIN_HELD_OUT_EVENTS} held-out events, have {}", learned.events))
    } else if learned.coverage == 0.0 {
        Verdict::NotBetter("the model recognises none of the held-out situations".into())
    } else if wins <= losses || z < MIN_Z {
        Verdict::NotBetter(format!("no significant top-1 gain over the rules (wins {wins}, losses {losses}, z {z:.1} < {MIN_Z})"))
    } else if learned.top3 < baseline.top3 {
        Verdict::NotBetter("top-3 ranking is worse than the rules".into())
    } else {
        Verdict::Eligible
    };
    let report = Report {
        sessions: total,
        train_sessions: train_ids.len() as u32,
        held_sessions: held.len() as u32,
        train_events,
        held_events: learned.events,
        deferred_events: deferred,
        baseline,
        learned,
        wins,
        losses,
        z,
        verdict,
    };
    Ok(Bundle { version: ADVISORY_VERSION, model, report })
}

/// Local files of the loop: `events.jsonl`, `settings.json`, `model.json`.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}
impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    /// `<app data>/fea-editor/advisor`, when the platform has an app-data folder.
    pub fn default_location() -> Option<Self> {
        crate::paths::app_data_dir().map(|d| Self::new(d.join("fea-editor").join("advisor")))
    }
    /// The recording / model folder of one bench: contexts, rules and recordings are never shared between
    /// benches. The member bench keeps the original folder so existing recordings stay valid; other
    /// benches get `<advisor>/<bench>/`.
    pub fn for_bench(bench: crate::gpu_viewer::bench::Bench) -> Option<Self> {
        let base = Self::default_location()?;
        Some(if bench == crate::gpu_viewer::bench::Bench::Members { base } else { Self::new(base.dir.join(bench.slug())) })
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    fn events(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }
    pub fn append(&self, r: &Recorded) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.events()).map_err(|e| e.to_string())?;
        let line = serde_json::to_string(r).map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())
    }
    /// Newest `MAX_EVENTS` events; unreadable lines (a torn last write) are skipped.
    pub fn read(&self) -> Result<Vec<Recorded>, String> {
        let text = match std::fs::read_to_string(self.events()) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.to_string()),
        };
        let mut all: Vec<Recorded> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        if all.len() > MAX_EVENTS {
            all.drain(..all.len() - MAX_EVENTS);
        }
        Ok(all)
    }
    pub fn has_recordings(&self) -> bool {
        std::fs::metadata(self.events()).is_ok_and(|m| m.len() > 0)
    }
    pub fn delete_recordings(&self) -> Result<(), String> {
        match std::fs::remove_file(self.events()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
    pub fn load_settings(&self) -> Settings {
        std::fs::read_to_string(self.dir.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
            .filter(|s| s.version == ADVISORY_VERSION)
            .unwrap_or_default()
    }
    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("settings.json"), serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    pub fn load_bundle(&self) -> Option<Bundle> {
        std::fs::read_to_string(self.dir.join("model.json")).ok().and_then(|t| serde_json::from_str(&t).ok())
    }
    pub fn save_bundle(&self, b: &Bundle) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("model.json"), serde_json::to_string_pretty(b).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

/// Session-side state of the loop.
pub struct Learning {
    pub store: Store,
    pub settings: Settings,
    pub bundle: Option<Bundle>,
    session: u64,
}
impl Learning {
    pub fn new(store: Store, session: u64) -> Self {
        let (settings, bundle) = (store.load_settings(), store.load_bundle());
        Self { store, settings, bundle, session }
    }
    pub fn record(&self, context: Context, allowed: Vec<Suggestion>, taken: Suggestion) -> Result<(), String> {
        self.store.append(&Recorded { session: self.session, context, allowed, taken })
    }
    /// Why the learned ranking cannot be switched on now (`Ok` when it can).
    pub fn learned_available(&self) -> Result<(), String> {
        match &self.bundle {
            None => Err("no trained model: record sessions, then Train".into()),
            Some(b) => b.usable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::editor::{
        advisory::{Selected, Stage},
        caps::Operation as O,
        state::Tool,
    };

    fn ctx(stage: Stage) -> Context {
        Context { stage, selected: Selected::Nothing, solving: false }
    }
    fn allowed() -> Vec<Suggestion> {
        vec![Suggestion::Op(O::Solve), Suggestion::Op(O::Save), Suggestion::Tool(Tool::Select)]
    }
    /// `n` sessions of `per` events each, all taking `taken` in the Loaded context.
    fn sessions(n: u64, per: usize, taken: Suggestion) -> Vec<Recorded> {
        (0..n).flat_map(|s| (0..per).map(move |_| Recorded { session: s, context: ctx(Stage::Loaded), allowed: allowed(), taken })).collect()
    }
    fn temp(name: &str) -> Store {
        let d = std::env::temp_dir().join(format!("fea-learning-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Store::new(d)
    }

    #[test]
    fn held_out_split_is_deterministic_and_about_a_fifth() {
        let held = (0..1000u64).filter(|s| is_held_out(*s)).count();
        assert!((150..250).contains(&held), "{held}");
        assert_eq!((0..50u64).map(is_held_out).collect::<Vec<_>>(), (0..50u64).map(is_held_out).collect::<Vec<_>>());
    }

    #[test]
    fn too_little_data_is_insufficient_whatever_the_numbers_look_like() {
        let b = train(&sessions(6, 40, Suggestion::Op(O::Save))).unwrap();
        assert!(matches!(b.report.verdict, Verdict::Insufficient(_)), "{:?}", b.report.verdict);
        assert!(b.usable().is_err());
    }

    #[test]
    fn a_population_the_rules_rank_wrongly_makes_the_model_eligible() {
        // Synthetic fixture (not user data): everybody saves before solving, the rules say Solve first.
        let b = train(&sessions(60, 12, Suggestion::Op(O::Save))).unwrap();
        let r = &b.report;
        assert_eq!(r.verdict, Verdict::Eligible, "{}", r.summary());
        assert!(r.learned.top1 > r.baseline.top1 && r.wins > r.losses && r.z >= MIN_Z);
        assert!(r.train_sessions + r.held_sessions == 60 && r.held_events >= MIN_HELD_OUT_EVENTS);
        assert!(b.usable().is_ok());
    }

    #[test]
    fn a_population_that_follows_the_rules_never_becomes_eligible() {
        // Users do exactly what the rules suggest: the model cannot beat them.
        let b = train(&sessions(60, 12, Suggestion::Op(O::Solve))).unwrap();
        assert!(matches!(b.report.verdict, Verdict::NotBetter(_)), "{}", b.report.summary());
        assert!(b.usable().is_err());
        assert_eq!(b.report.wins, 0);
    }

    #[test]
    fn mixed_noise_without_a_significant_gain_is_not_eligible() {
        // Half save, half solve, interleaved across sessions: no consistent edge over the rules.
        let mut r = sessions(30, 12, Suggestion::Op(O::Save));
        r.extend(sessions(60, 12, Suggestion::Op(O::Solve)).into_iter().map(|mut e| {
            e.session += 1000;
            e
        }));
        let b = train(&r).unwrap();
        assert_ne!(b.report.verdict, Verdict::Eligible, "{}", b.report.summary());
    }

    #[test]
    fn no_training_sessions_is_an_error() {
        assert!(train(&[]).is_err());
    }

    #[test]
    fn store_round_trips_events_settings_and_bundle_and_skips_torn_lines() {
        let store = temp("store");
        assert_eq!(store.read().unwrap(), vec![]);
        assert!(!store.has_recordings());
        assert_eq!(store.load_settings(), Settings::default());
        let rec = Recorded { session: 7, context: ctx(Stage::Empty), allowed: allowed(), taken: Suggestion::Tool(Tool::Node) };
        store.append(&rec).unwrap();
        std::fs::OpenOptions::new().append(true).open(store.events()).unwrap().write_all(b"{\"session\": 7, tru").unwrap();
        assert_eq!(store.read().unwrap(), vec![rec]);
        assert!(store.has_recordings());
        let s = Settings { record: true, ..Settings::default() };
        store.save_settings(&s).unwrap();
        assert_eq!(store.load_settings(), s);
        let b = train(&sessions(60, 12, Suggestion::Op(O::Save))).unwrap();
        store.save_bundle(&b).unwrap();
        assert_eq!(store.load_bundle().unwrap(), b);
        store.delete_recordings().unwrap();
        assert!(!store.has_recordings());
        store.delete_recordings().unwrap();
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn recorded_events_hold_only_coarse_enums() {
        let rec = Recorded { session: 1, context: ctx(Stage::Loaded), allowed: allowed(), taken: Suggestion::Op(O::Solve) };
        let json = serde_json::to_string(&rec).unwrap();
        for forbidden in ["pos", "name", "path", "value", "/"] {
            assert!(!json.contains(forbidden), "{forbidden} in {json}");
        }
    }

    #[test]
    fn learned_availability_explains_itself() {
        let store = temp("avail");
        let mut l = Learning::new(store.clone(), 1);
        assert!(l.learned_available().unwrap_err().contains("no trained model"));
        l.bundle = Some(train(&sessions(6, 40, Suggestion::Op(O::Save))).unwrap());
        assert!(l.learned_available().unwrap_err().starts_with("not enabled: needs 20 recorded sessions"));
        l.bundle = Some(train(&sessions(60, 12, Suggestion::Op(O::Save))).unwrap());
        assert!(l.learned_available().is_ok());
        let mut stale = l.bundle.clone().unwrap();
        stale.version = 99;
        assert!(stale.usable().unwrap_err().contains("incompatible"));
    }

    #[test]
    fn each_bench_has_its_own_advisor_folder_and_members_keep_the_original() {
        use crate::gpu_viewer::bench::Bench;
        let Some(base) = Store::default_location() else { return };
        assert_eq!(Store::for_bench(Bench::Members).unwrap().dir(), base.dir());
        let dirs: std::collections::BTreeSet<_> = Bench::ALL.iter().filter_map(|b| Store::for_bench(*b)).map(|s| s.dir().to_path_buf()).collect();
        assert_eq!(dirs.len(), Bench::ALL.len(), "no two benches share recordings");
    }
}
