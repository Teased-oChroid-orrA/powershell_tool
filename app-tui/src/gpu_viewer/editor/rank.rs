//! Generic, opt-in learned *ranking of already-allowed choices* (sketch constraint suggestions, boundary-condition
//! kinds), with the same discipline as the member advisor (`advisory.rs` / `learning.rs`):
//!
//! * the deterministic rules decide what is *offered*; a learned model may only reorder it, never add to it;
//! * recordings are local, off by default and content-free (a coarse situation of enums and the candidate taken;
//!   no coordinates, names, values or paths);
//! * training is explicit; the model is split deterministically by session into training and held-out sets, and it may
//!   be switched on only when the saved report says it is measurably better than the rules on held-out sessions
//!   (enough sessions, enough held-out events, a paired sign test `z >= 2`, no top-3 regression).
//!
//! The model is a smoothed per-situation acceptance rate: `(taken + 1) / (offered + 2)` for each candidate, learned
//! from how often a candidate offered in a situation was the one the user took. Unseen situations defer to the rules.
//!
//! Two sources of sessions feed it. The user's own recordings (`<name>.jsonl`) are the only real evidence. A
//! *simulated corpus* (`<name>-corpus.jsonl`, `solid/corpus.rs`: scripted sessions whose choices follow stated
//! design-intent policies) gives the model a broad prior over situations nobody has recorded yet, at a fraction of the
//! weight of a real session. The corpus can never replace the gate: the verdict is measured on the user's held-out
//! sessions as soon as there are enough of them (`Evidence::Real`); until then it is measured on held-out corpus
//! sessions and says so (`Evidence::Simulated`). Learning continues while recording is on: the model is refitted
//! every time the recordings have grown by a tenth (at least `AUTO_TRAIN_EVERY` offers).
use super::learning::{is_held_out, Verdict, MIN_HELD_OUT_EVENTS, MIN_SESSIONS, MIN_Z};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

pub const RANK_VERSION: u32 = 1;
/// A learned ranking needs this many offers in the exact situation, otherwise it defers to the rules.
pub const MIN_OFFERS: f64 = 5.0;
/// A simulated session counts for this fraction of a recorded one when fitting.
pub const CORPUS_WEIGHT: f64 = 0.25;
/// Session ids with this bit set belong to the simulated corpus (recorded ids are millisecond timestamps).
pub const CORPUS_BIT: u64 = 1 << 62;
/// Refit after at least this many new offers (and after every tenth of growth).
pub const AUTO_TRAIN_EVERY: usize = 50;

/// What a situation or candidate type must be to be learned from.
pub trait Label: Copy + Ord + Serialize + DeserializeOwned + std::fmt::Debug {}
impl<T: Copy + Ord + Serialize + DeserializeOwned + std::fmt::Debug> Label for T {}

/// One offer: the candidates the rules proposed (best first) and the one taken, if any.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recorded<S, C> {
    pub session: u64,
    pub situation: S,
    pub offered: Vec<C>,
    /// `None`: the offer was ignored.
    pub taken: Option<C>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Row<S, C> {
    situation: S,
    candidate: C,
    /// Weighted counts (a simulated offer counts `CORPUS_WEIGHT`; stored files from before weights are whole numbers).
    offered: f64,
    taken: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model<S, C> {
    pub version: u32,
    rows: Vec<Row<S, C>>,
}

impl<S: Label, C: Label> Model<S, C> {
    pub fn train(events: &[Recorded<S, C>]) -> Result<Self, String> {
        Self::train_weighted(events.iter().map(|e| (e, 1.0)))
    }

    pub fn train_weighted<'a>(events: impl IntoIterator<Item = (&'a Recorded<S, C>, f64)>) -> Result<Self, String>
    where
        S: 'a,
        C: 'a,
    {
        let mut m: BTreeMap<(S, C), (f64, f64)> = BTreeMap::new();
        let mut any = false;
        for (e, w) in events {
            any = true;
            for c in &e.offered {
                let slot = m.entry((e.situation, *c)).or_default();
                slot.0 += w;
                slot.1 += if e.taken == Some(*c) { w } else { 0.0 };
            }
        }
        if !any {
            return Err("no recorded training events".into());
        }
        Ok(Self { version: RANK_VERSION, rows: m.into_iter().map(|((situation, candidate), (offered, taken))| Row { situation, candidate, offered, taken }).collect() })
    }

    /// Weighted offers of the situation seen in training (of its most-offered candidate).
    pub fn observations(&self, s: &S) -> f64 {
        self.rows.iter().filter(|r| r.situation == *s).map(|r| r.offered).fold(0.0, f64::max)
    }

    fn rate(&self, s: &S, c: &C) -> f64 {
        let (o, t) = self.rows.iter().find(|r| r.situation == *s && r.candidate == *c).map_or((0.0, 0.0), |r| (r.offered, r.taken));
        (t + 1.0) / (o + 2.0)
    }

    /// `offered` reordered by learned acceptance (ties keep the rules' order); `None` defers to the rules.
    pub fn rank(&self, s: &S, offered: &[C]) -> Option<Vec<C>> {
        if self.observations(s) < MIN_OFFERS {
            return None;
        }
        let mut v: Vec<(usize, C, f64)> = offered.iter().copied().enumerate().map(|(i, c)| (i, c, self.rate(s, &c))).collect();
        v.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        Some(v.into_iter().map(|x| x.1).collect())
    }
}

/// What the verdict was measured on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Evidence {
    /// The user's own held-out sessions.
    #[default]
    Real,
    /// Held-out sessions of the simulated corpus (the user has too few of their own yet).
    Simulated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub evidence: Evidence,
    /// The user's recorded sessions and the simulated corpus sessions the model was fitted on / checked against.
    #[serde(default)]
    pub real_sessions: u32,
    #[serde(default)]
    pub corpus_sessions: u32,
    pub sessions: u32,
    pub train_sessions: u32,
    pub held_sessions: u32,
    pub held_events: usize,
    pub deferred_events: usize,
    pub rules_top1: f64,
    pub learned_top1: f64,
    pub rules_top3: f64,
    pub learned_top3: f64,
    pub wins: u32,
    pub losses: u32,
    pub z: f64,
    pub verdict: Verdict,
}

impl Report {
    pub fn summary(&self, what: &str) -> String {
        let pct = |v: f64| format!("{:.0}%", 100.0 * v);
        let verdict = match &self.verdict {
            Verdict::Eligible => "eligible to use".to_string(),
            Verdict::NotBetter(w) | Verdict::Insufficient(w) => format!("not used: {w}"),
        };
        let evidence = match self.evidence {
            Evidence::Real => format!("measured on your own sessions ({} recorded, {} simulated for the prior)", self.real_sessions, self.corpus_sessions),
            Evidence::Simulated => format!("measured on SIMULATED sessions only ({} recorded so far; your own take over at {MIN_SESSIONS})", self.real_sessions),
        };
        format!(
            "{what}: {evidence}: {} sessions ({} train / {} held out, {} held-out events, {} deferred); top-1 rules {} vs learned {} (wins {}, losses {}, z {:.1}); {verdict}.",
            self.sessions, self.train_sessions, self.held_sessions, self.held_events, self.deferred_events, pct(self.rules_top1), pct(self.learned_top1), self.wins, self.losses, self.z
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bundle<S, C> {
    pub version: u32,
    pub model: Model<S, C>,
    pub report: Report,
}
impl<S: Label, C: Label> Bundle<S, C> {
    pub fn usable(&self) -> Result<(), String> {
        if self.version != RANK_VERSION || self.model.version != RANK_VERSION {
            return Err("the trained model is from an incompatible version; train again".into());
        }
        match &self.report.verdict {
            Verdict::Eligible => Ok(()),
            Verdict::NotBetter(w) | Verdict::Insufficient(w) => Err(format!("not enabled: {w}")),
        }
    }
}

struct Split<'a, S, C> {
    train: Vec<&'a Recorded<S, C>>,
    held: Vec<&'a Recorded<S, C>>,
    sessions: u32,
    train_sessions: u32,
    held_sessions: u32,
}

/// Deterministic split by session (so recording order cannot bias which sessions are held out).
fn split<S, C>(recorded: &[Recorded<S, C>]) -> Split<'_, S, C> {
    let sessions: BTreeSet<u64> = recorded.iter().map(|r| r.session).collect();
    let held_ids: BTreeSet<u64> = sessions.iter().copied().filter(|s| is_held_out(*s)).collect();
    Split {
        train: recorded.iter().filter(|r| !held_ids.contains(&r.session)).collect(),
        held: recorded.iter().filter(|r| held_ids.contains(&r.session)).collect(),
        sessions: sessions.len() as u32,
        train_sessions: (sessions.len() - held_ids.len()) as u32,
        held_sessions: held_ids.len() as u32,
    }
}

struct Stats {
    scored: usize,
    deferred: usize,
    rules1: usize,
    rules3: usize,
    learned1: usize,
    learned3: usize,
    wins: u32,
    losses: u32,
    z: f64,
}

/// Rules (the offered order) against the model on held-out offers; only offers where the user took one of the offered
/// candidates say anything about ranking.
fn evaluate<S: Label, C: Label>(model: &Model<S, C>, held: &[&Recorded<S, C>]) -> Stats {
    let scored: Vec<&&Recorded<S, C>> = held.iter().filter(|r| r.taken.is_some_and(|t| r.offered.contains(&t))).collect();
    let mut st = Stats { scored: scored.len(), deferred: 0, rules1: 0, rules3: 0, learned1: 0, learned3: 0, wins: 0, losses: 0, z: 0.0 };
    for r in scored {
        let taken = r.taken.expect("filtered");
        let ranked = model.rank(&r.situation, &r.offered);
        st.deferred += usize::from(ranked.is_none());
        let learned = ranked.unwrap_or_else(|| r.offered.clone());
        let pos = |v: &[C]| v.iter().position(|c| *c == taken).unwrap_or(usize::MAX);
        let (rp, lp) = (pos(&r.offered), pos(&learned));
        st.rules1 += usize::from(rp == 0);
        st.learned1 += usize::from(lp == 0);
        st.rules3 += usize::from(rp < 3);
        st.learned3 += usize::from(lp < 3);
        st.wins += u32::from(lp == 0 && rp != 0);
        st.losses += u32::from(rp == 0 && lp != 0);
    }
    let nn = f64::from(st.wins + st.losses);
    st.z = if nn > 0.0 { (f64::from(st.wins) - f64::from(st.losses)) / nn.sqrt() } else { 0.0 };
    st
}

fn verdict_of(sessions: u32, st: &Stats) -> Verdict {
    if sessions < MIN_SESSIONS {
        Verdict::Insufficient(format!("needs {MIN_SESSIONS} recorded sessions, have {sessions}"))
    } else if st.scored < MIN_HELD_OUT_EVENTS {
        Verdict::Insufficient(format!("needs {MIN_HELD_OUT_EVENTS} held-out events, have {}", st.scored))
    } else if st.deferred == st.scored {
        Verdict::NotBetter("the model recognises none of the held-out situations".into())
    } else if st.wins <= st.losses || st.z < MIN_Z {
        Verdict::NotBetter(format!("no significant top-1 gain over the rules (wins {}, losses {}, z {:.1} < {MIN_Z})", st.wins, st.losses, st.z))
    } else if st.learned3 < st.rules3 {
        Verdict::NotBetter("top-3 ranking is worse than the rules".into())
    } else {
        Verdict::Eligible
    }
}

/// Fit on the training sessions (the user's at full weight, the simulated corpus's at `CORPUS_WEIGHT`) and measure
/// against the rules on held-out sessions: the user's own once there are enough of them, else the corpus's.
pub fn train<S: Label, C: Label>(recorded: &[Recorded<S, C>], corpus: &[Recorded<S, C>]) -> Result<Bundle<S, C>, String> {
    let (real, sim) = (split(recorded), split(corpus));
    let weighted = real.train.iter().map(|r| (*r, 1.0)).chain(sim.train.iter().map(|r| (*r, CORPUS_WEIGHT)));
    let model = Model::train_weighted(weighted).map_err(|_| "no recorded training sessions yet; turn on recording and use the editor".to_string())?;
    let real_stats = evaluate(&model, &real.held);
    let real_counts = real.sessions >= MIN_SESSIONS && real_stats.scored >= MIN_HELD_OUT_EVENTS;
    let (evidence, used, st) = if real_counts || sim.sessions == 0 {
        (Evidence::Real, &real, real_stats)
    } else {
        (Evidence::Simulated, &sim, evaluate(&model, &sim.held))
    };
    let verdict = verdict_of(used.sessions, &st);
    let n = st.scored.max(1) as f64;
    let report = Report {
        evidence,
        real_sessions: real.sessions,
        corpus_sessions: sim.sessions,
        sessions: used.sessions,
        train_sessions: used.train_sessions,
        held_sessions: used.held_sessions,
        held_events: st.scored,
        deferred_events: st.deferred,
        rules_top1: st.rules1 as f64 / n,
        learned_top1: st.learned1 as f64 / n,
        rules_top3: st.rules3 as f64 / n,
        learned_top3: st.learned3 as f64 / n,
        wins: st.wins,
        losses: st.losses,
        z: st.z,
        verdict,
    };
    Ok(Bundle { version: RANK_VERSION, model, report })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    pub record: bool,
    pub use_learned: bool,
    /// Refit as the recordings grow (only while recording is on).
    #[serde(default = "yes")]
    pub auto_train: bool,
}
fn yes() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self { version: RANK_VERSION, record: false, use_learned: false, auto_train: true }
    }
}

/// Local files of one ranker: `<name>.jsonl` (events), `<name>-settings.json`, `<name>-model.json`.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
    name: &'static str,
}
const MAX_EVENTS: usize = 200_000;
impl Store {
    pub fn new(dir: PathBuf, name: &'static str) -> Self {
        Self { dir, name }
    }
    /// `<app data>/fea-editor/advisor/<bench>/`, shared by the rankers of a bench under different names.
    pub fn for_bench(bench: crate::gpu_viewer::bench::Bench, name: &'static str) -> Option<Self> {
        super::learning::Store::for_bench(bench).map(|s| Self::new(s.dir().to_path_buf(), name))
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    fn file(&self, suffix: &str) -> PathBuf {
        self.dir.join(format!("{}{suffix}", self.name))
    }
    pub fn append<S: Label, C: Label>(&self, r: &Recorded<S, C>) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.file(".jsonl")).map_err(|e| e.to_string())?;
        writeln!(f, "{}", serde_json::to_string(r).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    pub fn read<S: Label, C: Label>(&self) -> Result<Vec<Recorded<S, C>>, String> {
        let text = match std::fs::read_to_string(self.file(".jsonl")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.to_string()),
        };
        let mut all: Vec<Recorded<S, C>> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        if all.len() > MAX_EVENTS {
            all.drain(..all.len() - MAX_EVENTS);
        }
        Ok(all)
    }
    /// Replace the simulated corpus file.
    pub fn write_corpus<S: Label, C: Label>(&self, events: &[Recorded<S, C>]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut text = String::new();
        for e in events {
            text.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
            text.push('\n');
        }
        std::fs::write(self.file("-corpus.jsonl"), text).map_err(|e| e.to_string())
    }
    pub fn read_corpus<S: Label, C: Label>(&self) -> Vec<Recorded<S, C>> {
        std::fs::read_to_string(self.file("-corpus.jsonl")).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).filter(|r: &Recorded<S, C>| r.session & CORPUS_BIT != 0).collect()).unwrap_or_default()
    }
    pub fn has_corpus(&self) -> bool {
        std::fs::metadata(self.file("-corpus.jsonl")).is_ok_and(|m| m.len() > 0)
    }
    pub fn has_recordings(&self) -> bool {
        std::fs::metadata(self.file(".jsonl")).is_ok_and(|m| m.len() > 0)
    }
    pub fn delete_recordings(&self) -> Result<(), String> {
        match std::fs::remove_file(self.file(".jsonl")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
    pub fn load_settings(&self) -> Settings {
        std::fs::read_to_string(self.file("-settings.json")).ok().and_then(|t| serde_json::from_str::<Settings>(&t).ok()).filter(|s| s.version == RANK_VERSION).unwrap_or_default()
    }
    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.file("-settings.json"), serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
    pub fn load_bundle<S: Label, C: Label>(&self) -> Option<Bundle<S, C>> {
        std::fs::read_to_string(self.file("-model.json")).ok().and_then(|t| serde_json::from_str(&t).ok())
    }
    pub fn save_bundle<S: Label, C: Label>(&self, b: &Bundle<S, C>) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.file("-model.json"), serde_json::to_string_pretty(b).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

/// Session-side state of one ranker: settings, the gated model and a recorder.
pub struct Ranker<S, C> {
    pub store: Option<Store>,
    pub settings: Settings,
    pub bundle: Option<Bundle<S, C>>,
    session: u64,
    pub what: &'static str,
    /// Offers recorded since the last fit and the number of recorded offers it used.
    pending: usize,
    fitted_on: usize,
}

impl<S: Label, C: Label> Ranker<S, C> {
    pub fn new(store: Option<Store>, session: u64, what: &'static str) -> Self {
        let (settings, bundle) = store.as_ref().map_or((Settings::default(), None), |s| (s.load_settings(), s.load_bundle()));
        Self { store, settings, bundle, session, what, pending: 0, fitted_on: 0 }
    }
    pub fn learned_available(&self) -> Result<(), String> {
        match &self.bundle {
            None => Err("no trained model: record sessions, then Train".into()),
            Some(b) => b.usable(),
        }
    }
    /// Offered candidates in the order to show: the rules' order, or the learned one when enabled, gated and sure.
    pub fn order(&self, situation: &S, offered: &[C]) -> (Vec<C>, bool) {
        if self.settings.use_learned && self.learned_available().is_ok() {
            if let Some(b) = &self.bundle {
                if let Some(v) = b.model.rank(situation, offered) {
                    return (v, true);
                }
            }
        }
        (offered.to_vec(), false)
    }
    /// Record one offer when recording is on (errors are returned, never fatal). Returns the new report's summary when
    /// this offer made the recordings grow enough for the model to be refitted.
    pub fn record(&mut self, situation: S, offered: Vec<C>, taken: Option<C>) -> Result<Option<String>, String> {
        if !self.settings.record || offered.is_empty() {
            return Ok(None);
        }
        let Some(store) = &self.store else { return Ok(None) };
        store.append(&Recorded { session: self.session, situation, offered, taken })?;
        self.pending += 1;
        if self.settings.auto_train && self.pending >= AUTO_TRAIN_EVERY.max(self.fitted_on / 10) {
            return self.train().map(Some);
        }
        Ok(None)
    }
    fn persist(&self) -> Result<(), String> {
        match &self.store {
            Some(s) => s.save_settings(&self.settings),
            None => Err("no application data folder on this platform".into()),
        }
    }
    pub fn set_record(&mut self, on: bool) -> Result<(), String> {
        self.settings.record = on;
        self.persist()
    }
    pub fn set_auto_train(&mut self, on: bool) -> Result<(), String> {
        self.settings.auto_train = on;
        self.persist()
    }
    pub fn set_use_learned(&mut self, on: bool) -> Result<(), String> {
        if on {
            self.learned_available()?;
        }
        self.settings.use_learned = on;
        self.persist()
    }
    /// Train from the recordings and the simulated corpus and keep the bundle (switching the learned ranking off if it
    /// is no longer usable).
    pub fn train(&mut self) -> Result<String, String> {
        let store = self.store.as_ref().ok_or("no application data folder on this platform")?;
        let recorded = store.read::<S, C>()?;
        let bundle = train(&recorded, &store.read_corpus::<S, C>())?;
        store.save_bundle(&bundle)?;
        let text = bundle.report.summary(self.what);
        self.bundle = Some(bundle);
        self.pending = 0;
        self.fitted_on = recorded.len();
        if self.learned_available().is_err() && self.settings.use_learned {
            self.settings.use_learned = false;
            self.persist()?;
        }
        Ok(text)
    }
    /// Install the simulated corpus (replacing any earlier one).
    pub fn install_corpus(&self, events: &[Recorded<S, C>]) -> Result<(), String> {
        self.store.as_ref().ok_or("no application data folder on this platform")?.write_corpus(events)
    }
    pub fn delete_recordings(&mut self) -> Result<(), String> {
        self.store.as_ref().ok_or("no application data folder on this platform")?.delete_recordings()
    }
}

#[cfg(test)]
mod tests;
