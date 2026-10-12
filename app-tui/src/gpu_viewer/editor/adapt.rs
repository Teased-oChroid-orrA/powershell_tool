//! How the window adapts its layout to the person using it (ADR-018): which tools come first, which inspector cards
//! start collapsed, whether the feature tree starts open or as a rail, which step usually follows a finished one.
//!
//! This is deliberately a different tier from the engineering rankers (`rank.rs`, `advisory.rs`). Those propose
//! engineering choices (constraints, boundary conditions, refinement levels), so a learned ordering is gated behind a
//! measured held-out report. This only arranges the chrome: it never changes a model or a result, nothing it demotes is
//! unreachable (the palette and menus list everything), every decision starts from the fixed rules and only moves after
//! enough local observations, and it can be switched off or reset. So it learns from the first session.
//!
//! The model is the smoothed acceptance rate the rankers use, `(used + 1) / (offered + 2)`, per `(context, item)`, with a
//! per-session decay so recent habits outweigh old ones. What is stored is content-free: control labels and card ids
//! of the interface and counts, never a model value, a name or a path.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const VERSION: u32 = 1;
/// Offers of an item in a context before its learned score counts; until then the rules' order stands.
pub const MIN_OBS: f64 = 5.0;
/// Offers without a single use before a card starts collapsed.
pub const IDLE_OFFERS: f64 = 8.0;
/// Every stored count is multiplied by this at the start of a session.
pub const DECAY: f64 = 0.85;
/// A step is predicted to follow another when it took this share of at least `MIN_STEP_OBS` transitions.
pub const MIN_STEP_SHARE: f64 = 0.6;
pub const MIN_STEP_OBS: f64 = 4.0;
/// The tree starts as a rail when its smoothed use rate falls under this (after `MIN_TREE_SESSIONS` sessions).
pub const TREE_USE_FLOOR: f64 = 0.3;
pub const MIN_TREE_SESSIONS: f64 = 3.0;
const FILE: &str = "layout.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Rate {
    offered: f64,
    used: f64,
}
impl Rate {
    fn score(self) -> f64 {
        (self.used + 1.0) / (self.offered + 2.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    version: u32,
    sessions: u32,
    /// context -> item -> how often it was shown and how often it was used.
    items: BTreeMap<String, BTreeMap<String, Rate>>,
    /// Sessions in which the tree was available / in which a tree row was used.
    tree: Rate,
    /// step -> next step actually taken after it was finished.
    steps: BTreeMap<String, BTreeMap<String, f64>>,
}

pub struct Adapt {
    stats: Stats,
    path: Option<PathBuf>,
    /// Off: every decision is the fixed rule.
    pub enabled: bool,
    /// The last set offered per context, so an unchanged set is not counted again every frame.
    last_offer: BTreeMap<String, String>,
    /// Contexts offered in the frame being built; one that was not offered is offered anew when it returns.
    offered_now: std::collections::BTreeSet<String>,
    tree_used: bool,
    dirty: bool,
}

impl Adapt {
    /// Nothing is read or written (tests, `FEA_UI_ADAPT=off` keeps the file but ignores it).
    pub fn in_memory() -> Self {
        Self { stats: Stats { version: VERSION, ..Stats::default() }, path: None, enabled: true, last_offer: BTreeMap::new(), offered_now: Default::default(), tree_used: false, dirty: false }
    }

    /// `<app data>/fea-editor/layout/layout.json`, when the platform has an app-data folder.
    pub fn default_location() -> Option<PathBuf> {
        crate::paths::app_data_dir().map(|d| d.join("fea-editor").join("layout"))
    }

    /// Load the stored habits of `dir` (a missing or unreadable file starts empty), decay them and count this session.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let mut stats = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Stats>(&t).ok()).filter(|s| s.version == VERSION).unwrap_or(Stats { version: VERSION, ..Stats::default() });
        for rates in stats.items.values_mut() {
            for r in rates.values_mut() {
                r.offered *= DECAY;
                r.used *= DECAY;
            }
            rates.retain(|_, r| r.offered > 0.5);
        }
        stats.items.retain(|_, m| !m.is_empty());
        stats.tree = Rate { offered: stats.tree.offered * DECAY + 1.0, used: stats.tree.used * DECAY };
        for next in stats.steps.values_mut() {
            next.values_mut().for_each(|c| *c *= DECAY);
            next.retain(|_, c| *c > 0.3);
        }
        stats.sessions += 1;
        Self { stats, path: Some(path), enabled: !std::env::var("FEA_UI_ADAPT").is_ok_and(|v| v == "off"), last_offer: BTreeMap::new(), offered_now: Default::default(), tree_used: false, dirty: true }
    }

    pub fn sessions(&self) -> u32 {
        self.stats.sessions
    }

    /// The items of `context` are on screen. Counted once per distinct set, not per frame.
    pub fn offer(&mut self, context: &str, items: &[&str]) {
        self.offered_now.insert(context.to_string());
        let signature = items.join("\u{1f}");
        if self.last_offer.get(context).is_some_and(|s| *s == signature) {
            return;
        }
        self.last_offer.insert(context.to_string(), signature);
        let rates = self.stats.items.entry(context.to_string()).or_default();
        for item in items {
            rates.entry((*item).to_string()).or_default().offered += 1.0;
        }
        self.dirty = true;
    }

    /// End of a frame: a context that was not offered in it was off screen, so it counts again when it returns
    /// (a person going back to a step is a new look at its tools).
    pub fn frame_end(&mut self) {
        let now = std::mem::take(&mut self.offered_now);
        self.last_offer.retain(|k, _| now.contains(k));
    }

    /// The person used `item` in `context`.
    pub fn used(&mut self, context: &str, item: &str) {
        let r = self.stats.items.entry(context.to_string()).or_default().entry(item.to_string()).or_default();
        r.used += 1.0;
        r.offered = r.offered.max(r.used);
        self.dirty = true;
    }

    /// Learned score of `item` in `context`; `None` until it has been offered `MIN_OBS` times.
    pub fn score(&self, context: &str, item: &str) -> Option<f64> {
        let r = self.stats.items.get(context)?.get(item)?;
        (r.offered >= MIN_OBS).then(|| r.score())
    }

    /// `items` (the rules' order) with the learned ones sorted best first inside the places the learned ones occupy:
    /// an item without enough observations keeps its place, so a new tool never jumps around.
    pub fn order(&self, context: &str, items: &[String]) -> Vec<String> {
        if !self.enabled {
            return items.to_vec();
        }
        let mut learned: Vec<(usize, f64)> = items.iter().enumerate().filter_map(|(i, it)| self.score(context, it).map(|s| (i, s))).collect();
        let slots: Vec<usize> = learned.iter().map(|(i, _)| *i).collect();
        learned.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut out = items.to_vec();
        for (slot, (from, _)) in slots.iter().zip(&learned) {
            out[*slot] = items[*from].clone();
        }
        out
    }

    /// Shown often, never used: it starts collapsed (one click opens it).
    pub fn idle(&self, context: &str, item: &str) -> bool {
        self.enabled && self.stats.items.get(context).and_then(|m| m.get(item)).is_some_and(|r| r.offered >= IDLE_OFFERS && r.used < 0.5)
    }

    /// Whether the feature tree starts open. Open until enough sessions say it is hardly used.
    pub fn tree_open_by_default(&self) -> bool {
        !self.enabled || self.stats.tree.offered < MIN_TREE_SESSIONS || self.stats.tree.score() >= TREE_USE_FLOOR
    }
    /// A tree row was used this session.
    pub fn tree_used(&mut self) {
        if !std::mem::replace(&mut self.tree_used, true) {
            self.stats.tree.used += 1.0;
            self.dirty = true;
        }
    }

    /// The person went from the finished step `from` to `to`.
    pub fn step_taken(&mut self, from: &str, to: &str) {
        *self.stats.steps.entry(from.to_string()).or_default().entry(to.to_string()).or_default() += 1.0;
        self.dirty = true;
    }
    /// The step that usually follows `from`, with its share, once it is clear enough to act on.
    pub fn predicted_next(&self, from: &str) -> Option<(String, f64)> {
        if !self.enabled {
            return None;
        }
        let next = self.stats.steps.get(from)?;
        let total: f64 = next.values().sum();
        let (best, count) = next.iter().max_by(|a, b| a.1.total_cmp(b.1))?;
        (total >= MIN_STEP_OBS && count / total >= MIN_STEP_SHARE).then(|| (best.clone(), count / total))
    }

    /// Forget everything learned (the layout returns to the fixed rules).
    pub fn reset(&mut self) {
        self.stats = Stats { version: VERSION, sessions: 1, ..Stats::default() };
        self.last_offer.clear();
        self.dirty = true;
    }

    /// One line for the status bar: what has been learned so far.
    pub fn summary(&self) -> String {
        if !self.enabled {
            return "Adaptive layout is off: the fixed layout is used.".into();
        }
        let learned = self.stats.items.values().flat_map(|m| m.values()).filter(|r| r.offered >= MIN_OBS).count();
        format!(
            "Adaptive layout: {} session(s), {learned} learned item(s); the tree starts {}. Window > Reset learned layout forgets it.",
            self.stats.sessions,
            if self.tree_open_by_default() { "open" } else { "as a rail" }
        )
    }

    /// Write the habits when they changed (best effort: a layout never fails the window).
    pub fn flush(&mut self) {
        let (Some(path), true) = (&self.path, self.dirty) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(&self.stats) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_ok() {
                self.dirty = false;
            }
        }
    }
}

impl Drop for Adapt {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("adapt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn nothing_is_learned_until_an_item_has_been_offered_enough_and_the_rules_order_stands() {
        let mut a = Adapt::in_memory();
        let set = ["Line", "Arc", "Circle"];
        for _ in 0..4 {
            a.offer("Sketch", &set);
            a.offer("Sketch", &["x"]);
        }
        a.used("Sketch", "Circle");
        assert_eq!(a.score("Sketch", "Circle"), None);
        assert_eq!(a.order("Sketch", &items(&set)), set);
    }

    #[test]
    fn the_most_used_tools_move_up_and_unscored_tools_keep_their_place() {
        let mut a = Adapt::in_memory();
        for k in 0..8 {
            // The set changes between offers (as it does when the user changes step) so each one is counted.
            a.offer("Model", &["New part", "Add hole", "Add support", "Extrude"]);
            a.offer("Model", &["other"]);
            if k < 6 {
                a.used("Model", "Add support");
            }
        }
        a.offer("Model", &["Brand new tool"]);
        let rules = items(&["New part", "Add hole", "Add support", "Brand new tool", "Extrude"]);
        let ordered = a.order("Model", &rules);
        assert_eq!(ordered[0], "Add support", "{ordered:?}");
        assert_eq!(ordered.iter().position(|t| t == "Brand new tool"), Some(3), "an unscored tool keeps its slot: {ordered:?}");
        assert_eq!(ordered.len(), rules.len());
        a.enabled = false;
        assert_eq!(a.order("Model", &rules), rules, "switched off: the fixed order");
    }

    #[test]
    fn a_context_that_left_the_screen_is_counted_again_when_it_returns() {
        let mut a = Adapt::in_memory();
        for _ in 0..3 {
            a.offer("Mesh", &["Zones"]);
            a.frame_end();
        }
        assert_eq!(a.stats.items["Mesh"]["Zones"].offered, 1.0, "consecutive frames are one look");
        a.frame_end();
        a.offer("Mesh", &["Zones"]);
        assert_eq!(a.stats.items["Mesh"]["Zones"].offered, 2.0, "a return is a new look");
    }

    #[test]
    fn an_unchanged_offer_is_counted_once_not_every_frame() {
        let mut a = Adapt::in_memory();
        for _ in 0..50 {
            a.offer("Mesh", &["Generate mesh", "Zones"]);
        }
        assert_eq!(a.stats.items["Mesh"]["Zones"].offered, 1.0);
    }

    #[test]
    fn a_card_shown_often_and_never_used_starts_collapsed_and_one_use_keeps_it_open() {
        let mut b = Adapt::in_memory();
        for k in 0..10 {
            b.offer("Mesh", &["advice", &format!("{k}")]);
        }
        assert!(b.idle("Mesh", "advice"));
        b.used("Mesh", "advice");
        assert!(!b.idle("Mesh", "advice"));
        assert!(!b.idle("Mesh", "never offered"));
    }

    #[test]
    fn the_tree_starts_as_a_rail_only_after_several_sessions_without_using_it() {
        let mut a = Adapt::in_memory();
        assert!(a.tree_open_by_default(), "a new user sees the tree");
        a.stats.tree = Rate { offered: 6.0, used: 0.0 };
        assert!(!a.tree_open_by_default());
        a.stats.tree = Rate { offered: 6.0, used: 4.0 };
        assert!(a.tree_open_by_default());
        a.stats.tree = Rate { offered: 6.0, used: 0.0 };
        a.enabled = false;
        assert!(a.tree_open_by_default(), "off: the fixed layout");
    }

    #[test]
    fn the_step_that_usually_follows_is_predicted_only_when_it_is_clear() {
        let mut a = Adapt::in_memory();
        for _ in 0..3 {
            a.step_taken("Mesh", "Simulation");
        }
        assert_eq!(a.predicted_next("Mesh"), None, "too few transitions");
        a.step_taken("Mesh", "Simulation");
        assert_eq!(a.predicted_next("Mesh").map(|(s, _)| s), Some("Simulation".to_string()));
        for _ in 0..4 {
            a.step_taken("Mesh", "Model");
        }
        assert_eq!(a.predicted_next("Mesh"), None, "an even split is not a habit");
        assert_eq!(a.predicted_next("Simulation"), None);
    }

    #[test]
    fn habits_persist_decay_each_session_and_reset_forgets_them() {
        let d = dir("persist");
        {
            let mut a = Adapt::load(&d);
            assert_eq!(a.sessions(), 1);
            for k in 0..10 {
                a.offer("Mesh", &["Zones", &format!("p{k}")]);
                a.used("Mesh", "Zones");
            }
            a.step_taken("Mesh", "Simulation");
        }
        let a = Adapt::load(&d);
        assert_eq!(a.sessions(), 2);
        let r = a.stats.items["Mesh"]["Zones"];
        assert!((r.offered - 8.5).abs() < 1e-9 && (r.used - 8.5).abs() < 1e-9, "{r:?}: decayed once by {DECAY}");
        assert!(a.score("Mesh", "Zones").is_some());
        drop(a);
        let mut a = Adapt::load(&d);
        a.reset();
        assert_eq!(a.score("Mesh", "Zones"), None);
        assert!(a.stats.steps.is_empty());
        drop(a);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_corrupt_or_foreign_file_starts_empty_instead_of_failing() {
        let d = dir("corrupt");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE), "{ not json").unwrap();
        let a = Adapt::load(&d);
        assert_eq!(a.sessions(), 1);
        drop(a);
        std::fs::write(d.join(FILE), r#"{"version":99,"sessions":40,"items":{},"tree":{"offered":0,"used":0},"steps":{}}"#).unwrap();
        assert_eq!(Adapt::load(&d).sessions(), 1, "another version is not trusted");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_summary_says_what_was_learned_and_when_it_is_off() {
        let mut a = Adapt::in_memory();
        assert!(a.summary().contains("0 learned"));
        a.enabled = false;
        assert!(a.summary().contains("off"));
    }
}
