//! The simulated corpus: deterministic, content-free, broad, and never able to pass the gate on its own authority.
use super::corpus::{build, SEED, SESSIONS};
use crate::gpu_viewer::editor::rank::{train, Evidence, CORPUS_BIT};
use fea_problem::{advice::BcKind, sketch::infer::Rule};
use std::collections::BTreeSet;

#[test]
fn building_the_corpus_is_deterministic_and_seed_dependent() {
    let (a, b) = (build(40, SEED), build(40, SEED));
    assert_eq!((a.constraints.clone(), a.boundary.clone()), (b.constraints, b.boundary));
    let c = build(40, SEED + 1);
    assert_ne!(a.constraints, c.constraints);
}

#[test]
fn the_corpus_is_content_free_marked_simulated_and_internally_consistent() {
    let c = build(SESSIONS, SEED);
    for r in &c.constraints {
        assert!(r.session & CORPUS_BIT != 0 && !r.offered.is_empty());
        assert!(r.taken.is_none_or(|t| r.offered.contains(&t)), "{r:?}");
    }
    for r in &c.boundary {
        assert!(r.session & CORPUS_BIT != 0 && !r.offered.is_empty());
        assert!(r.taken.is_none_or(|t| r.offered.contains(&t)), "{r:?}");
    }
    let json = serde_json::to_string(&c.constraints[0]).unwrap() + &serde_json::to_string(&c.boundary[0]).unwrap();
    for forbidden in ["pos", "name", "path", "value", "x\":", "hole1"] {
        assert!(!json.contains(forbidden), "{forbidden} in {json}");
    }
}

#[test]
fn the_corpus_covers_the_situations_tools_rules_and_boundary_kinds_the_editor_can_offer() {
    let c = build(SESSIONS, SEED);
    let sit: BTreeSet<_> = c.constraints.iter().map(|r| r.situation).collect();
    let tools: BTreeSet<_> = sit.iter().map(|s| s.tool).collect();
    assert!(tools.len() >= 4, "line, polyline, circle and arc offers: {tools:?}");
    assert!(sit.iter().any(|s| s.constrained) && sit.iter().any(|s| !s.constrained));
    assert!(sit.len() >= 8, "{} constraint situations", sit.len());
    let taken: BTreeSet<Rule> = c.constraints.iter().filter_map(|r| r.taken).collect();
    for rule in [Rule::Horizontal, Rule::Vertical, Rule::Tangent, Rule::Equal, Rule::Parallel] {
        assert!(taken.contains(&rule), "{rule:?} never taken: {taken:?}");
    }
    let bsit: BTreeSet<_> = c.boundary.iter().map(|r| r.situation).collect();
    assert!(bsit.len() >= 30, "{} boundary situations", bsit.len());
    let kinds: BTreeSet<BcKind> = c.boundary.iter().filter_map(|r| r.taken).collect();
    assert_eq!(kinds.len(), 4, "{kinds:?}");
    // Each situation that matters has enough offers for the ranker to act on (MIN_OFFERS, weighted).
    let thick = |counts: Vec<usize>| counts.into_iter().filter(|n| *n >= 50).count();
    let mut m: std::collections::BTreeMap<_, usize> = Default::default();
    for r in &c.boundary {
        *m.entry(r.situation).or_default() += 1;
    }
    assert!(thick(m.values().copied().collect()) >= 15);
}

#[test]
fn trained_only_on_the_corpus_the_gate_is_open_but_labelled_simulated() {
    let c = build(SESSIONS, SEED);
    for b in [train(&[], &c.constraints).unwrap().report, train(&[], &c.boundary).unwrap().report] {
        assert_eq!(b.evidence, Evidence::Simulated);
        assert_eq!(b.real_sessions, 0);
        assert!(b.summary("x").contains("SIMULATED"));
    }
}
