use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Sit {
    A,
    B,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Cand {
    X,
    Y,
    Z,
}
const OFFER: [Cand; 3] = [Cand::X, Cand::Y, Cand::Z];

/// `n` sessions of `per` offers in situation A where `taken` is chosen (synthetic fixture, not user data).
fn sessions(n: u64, per: usize, taken: Option<Cand>) -> Vec<Recorded<Sit, Cand>> {
    (0..n).flat_map(|s| (0..per).map(move |_| Recorded { session: s, situation: Sit::A, offered: OFFER.to_vec(), taken })).collect()
}
fn temp(name: &str) -> Store {
    let d = std::env::temp_dir().join(format!("fea-rank-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    Store::new(d, "test")
}

#[test]
fn model_reorders_by_acceptance_and_defers_when_unseen_or_thin() {
    let m = Model::train(&sessions(10, 3, Some(Cand::Z))).unwrap();
    assert_eq!(m.rank(&Sit::A, &OFFER).unwrap()[0], Cand::Z);
    assert!(m.rank(&Sit::B, &OFFER).is_none(), "unseen situation defers");
    let thin = Model::train(&sessions(1, 3, Some(Cand::Z))).unwrap();
    assert!(thin.rank(&Sit::A, &OFFER).is_none(), "fewer than MIN_OFFERS offers defers");
    // It reorders what was offered, never adds: a candidate that was not offered cannot appear.
    assert_eq!(m.rank(&Sit::A, &[Cand::X, Cand::Y]).unwrap().len(), 2);
    // Ties keep the rules' order.
    let none = Model::train(&sessions(10, 3, None)).unwrap();
    assert_eq!(none.rank(&Sit::A, &OFFER).unwrap(), OFFER);
}

#[test]
fn too_little_data_is_insufficient_whatever_the_numbers_look_like() {
    let b = train(&sessions(6, 40, Some(Cand::Z)), &[]).unwrap();
    assert!(matches!(b.report.verdict, Verdict::Insufficient(_)), "{:?}", b.report.verdict);
    assert!(b.usable().is_err());
}

#[test]
fn a_population_the_rules_rank_wrongly_makes_the_model_eligible() {
    let b = train(&sessions(60, 12, Some(Cand::Z)), &[]).unwrap();
    let r = &b.report;
    assert_eq!(r.verdict, Verdict::Eligible, "{}", r.summary("t"));
    assert!(r.learned_top1 > r.rules_top1 && r.wins > r.losses && r.z >= MIN_Z && r.held_events >= MIN_HELD_OUT_EVENTS);
    assert!(b.usable().is_ok());
}

#[test]
fn a_population_that_follows_the_rules_never_becomes_eligible() {
    let b = train(&sessions(60, 12, Some(Cand::X)), &[]).unwrap();
    assert!(matches!(b.report.verdict, Verdict::NotBetter(_)), "{}", b.report.summary("t"));
    assert_eq!(b.report.wins, 0);
    assert!(b.usable().is_err());
}

#[test]
fn ignored_offers_do_not_count_as_ranking_evidence() {
    let b = train(&sessions(60, 12, None), &[]).unwrap();
    assert_eq!(b.report.held_events, 0);
    assert!(matches!(b.report.verdict, Verdict::Insufficient(_)));
}

#[test]
fn mixed_populations_without_a_consistent_edge_are_not_eligible() {
    let mut r = sessions(30, 12, Some(Cand::Z));
    r.extend(sessions(60, 12, Some(Cand::X)).into_iter().map(|mut e| {
        e.session += 1000;
        e
    }));
    assert_ne!(train(&r, &[]).unwrap().report.verdict, Verdict::Eligible);
}

#[test]
fn no_training_sessions_is_an_error() {
    assert!(train::<Sit, Cand>(&[], &[]).is_err());
}

#[test]
fn store_round_trips_and_skips_torn_lines_and_events_are_content_free() {
    let store = temp("store");
    assert_eq!(store.read::<Sit, Cand>().unwrap(), vec![]);
    let rec = Recorded { session: 7, situation: Sit::B, offered: vec![Cand::Y, Cand::X], taken: Some(Cand::X) };
    store.append(&rec).unwrap();
    std::fs::OpenOptions::new().append(true).open(store.file(".jsonl")).unwrap().write_all(b"{\"session\": 7, tru").unwrap();
    assert_eq!(store.read::<Sit, Cand>().unwrap(), vec![rec.clone()]);
    let json = serde_json::to_string(&rec).unwrap();
    for forbidden in ["pos", "name", "path", "value", "/", "x\":"] {
        assert!(!json.contains(forbidden), "{forbidden} in {json}");
    }
    let s = Settings { record: true, ..Settings::default() };
    store.save_settings(&s).unwrap();
    assert_eq!(store.load_settings(), s);
    let b = train(&sessions(60, 12, Some(Cand::Z)), &[]).unwrap();
    store.save_bundle(&b).unwrap();
    assert_eq!(store.load_bundle::<Sit, Cand>().unwrap(), b);
    assert!(store.has_recordings());
    store.delete_recordings().unwrap();
    assert!(!store.has_recordings());
    store.delete_recordings().unwrap();
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn ranker_records_only_when_enabled_orders_only_when_gated_and_explains() {
    let store = temp("ranker");
    let mut r: Ranker<Sit, Cand> = Ranker::new(Some(store.clone()), 1, "Test");
    // Recording is off by default: nothing is written.
    r.record(Sit::A, OFFER.to_vec(), Some(Cand::X)).unwrap();
    assert!(!store.has_recordings());
    r.set_record(true).unwrap();
    r.record(Sit::A, OFFER.to_vec(), Some(Cand::X)).unwrap();
    r.record(Sit::A, vec![], None).unwrap();
    assert_eq!(store.read::<Sit, Cand>().unwrap().len(), 1, "empty offers are not recorded");
    // Cannot enable the learned order without a usable model.
    assert!(r.set_use_learned(true).unwrap_err().contains("no trained model"));
    assert_eq!(r.order(&Sit::A, &OFFER), (OFFER.to_vec(), false));
    // Train on a population that prefers Z: now it can be enabled and it changes the order.
    for e in sessions(60, 12, Some(Cand::Z)) {
        store.append(&e).unwrap();
    }
    let text = r.train().unwrap();
    assert!(text.contains("eligible to use"), "{text}");
    r.set_use_learned(true).unwrap();
    assert_eq!(r.order(&Sit::A, &OFFER), (vec![Cand::Z, Cand::X, Cand::Y], true));
    assert_eq!(r.order(&Sit::B, &OFFER), (OFFER.to_vec(), false), "unseen situation: the rules");
    // Deleting recordings does not break the model; a stale/incompatible bundle disables itself.
    let mut stale = r.bundle.clone().unwrap();
    stale.version = 99;
    r.bundle = Some(stale);
    assert!(r.learned_available().unwrap_err().contains("incompatible"));
    assert!(!r.order(&Sit::A, &OFFER).1);
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn each_bench_ranker_has_its_own_files() {
    use crate::gpu_viewer::bench::Bench;
    let Some(a) = Store::for_bench(Bench::Static, "constraints") else { return };
    let b = Store::for_bench(Bench::Static, "boundary").unwrap();
    assert_ne!(a.file(".jsonl"), b.file(".jsonl"));
    assert_ne!(Store::for_bench(Bench::Modal, "constraints").unwrap().file(".jsonl"), a.file(".jsonl"));
}

/// Simulated sessions (the corpus): ids carry `CORPUS_BIT`.
fn corpus(n: u64, per: usize, taken: Option<Cand>) -> Vec<Recorded<Sit, Cand>> {
    sessions(n, per, taken).into_iter().map(|mut r| {
        r.session |= CORPUS_BIT;
        r
    }).collect()
}

#[test]
fn a_corpus_alone_trains_a_model_but_says_its_evidence_is_simulated() {
    let b = train(&[], &corpus(60, 12, Some(Cand::Z))).unwrap();
    let r = &b.report;
    assert_eq!((r.evidence, r.real_sessions, r.corpus_sessions), (Evidence::Simulated, 0, 60));
    assert_eq!(r.verdict, Verdict::Eligible, "{}", r.summary("t"));
    assert!(r.summary("t").contains("SIMULATED"), "the report never passes simulated evidence off as the user's");
    assert_eq!(b.model.rank(&Sit::A, &OFFER).unwrap()[0], Cand::Z, "the prior covers situations nobody recorded");
}

#[test]
fn enough_real_sessions_overrule_a_corpus_that_looks_better() {
    // The corpus says Z; 60 real sessions of the user say X (what the rules already put first).
    let b = train(&sessions(60, 12, Some(Cand::X)), &corpus(60, 12, Some(Cand::Z))).unwrap();
    assert_eq!(b.report.evidence, Evidence::Real, "{}", b.report.summary("t"));
    assert!(b.usable().is_err(), "real held-out sessions find no gain over the rules: {}", b.report.summary("t"));
    // The user's evidence outweighs the prior in the fitted model too (full weight against CORPUS_WEIGHT).
    assert_eq!(b.model.rank(&Sit::A, &OFFER).unwrap()[0], Cand::X);
}

#[test]
fn too_few_real_sessions_do_not_hide_the_corpus_but_do_not_fake_real_evidence_either() {
    let b = train(&sessions(5, 12, Some(Cand::Z)), &corpus(60, 12, Some(Cand::Z))).unwrap();
    assert_eq!((b.report.evidence, b.report.real_sessions), (Evidence::Simulated, 5));
}

#[test]
fn corpus_offers_weigh_less_than_recorded_ones() {
    let real = sessions(10, 3, Some(Cand::X));
    let sim = corpus(10, 3, Some(Cand::Z));
    // 30 real offers for X against 30 simulated for Z: the real ones win because of the weight.
    let m = Model::train_weighted(real.iter().map(|r| (r, 1.0)).chain(sim.iter().map(|r| (r, CORPUS_WEIGHT)))).unwrap();
    assert_eq!(m.rank(&Sit::A, &OFFER).unwrap()[0], Cand::X);
    assert!((m.observations(&Sit::A) - (30.0 + 30.0 * CORPUS_WEIGHT)).abs() < 1e-9);
}

#[test]
fn the_corpus_file_is_separate_from_recordings_and_survives_deleting_them() {
    let store = temp("corpus");
    store.write_corpus(&corpus(3, 2, Some(Cand::Y))).unwrap();
    assert!(store.has_corpus() && !store.has_recordings());
    assert_eq!(store.read::<Sit, Cand>().unwrap(), vec![], "recordings are only the user's");
    assert_eq!(store.read_corpus::<Sit, Cand>().len(), 6);
    // A corpus file cannot smuggle in sessions that look like the user's.
    let mut mixed = corpus(1, 1, None);
    mixed.extend(sessions(1, 1, None));
    store.write_corpus(&mixed).unwrap();
    assert_eq!(store.read_corpus::<Sit, Cand>().len(), 1);
    store.append(&sessions(1, 1, Some(Cand::X))[0]).unwrap();
    store.delete_recordings().unwrap();
    assert!(store.has_corpus(), "deleting recordings keeps the prior");
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn recording_keeps_learning_refitting_as_the_recordings_grow() {
    let store = temp("online");
    store.write_corpus(&corpus(60, 12, Some(Cand::Z))).unwrap();
    let mut r: Ranker<Sit, Cand> = Ranker::new(Some(store.clone()), 1000, "Test");
    r.set_record(true).unwrap();
    assert!(r.bundle.is_none());
    // Fewer than AUTO_TRAIN_EVERY offers: nothing is refitted yet.
    for _ in 0..AUTO_TRAIN_EVERY - 1 {
        assert_eq!(r.record(Sit::A, OFFER.to_vec(), Some(Cand::X)).unwrap(), None);
    }
    assert!(r.bundle.is_none());
    // The next offer triggers a fit that includes the prior and the user's recordings so far.
    let fit = r.record(Sit::A, OFFER.to_vec(), Some(Cand::X)).unwrap().expect("refitted");
    assert!(fit.contains("SIMULATED"), "{fit}");
    let b = r.bundle.as_ref().expect("fitted");
    assert_eq!(b.report.real_sessions, 1);
    assert_eq!(store.load_bundle::<Sit, Cand>().unwrap(), *b, "the fit is saved");
    // With the setting off, nothing refits; the setting itself is persisted.
    r.set_auto_train(false).unwrap();
    for _ in 0..2 * AUTO_TRAIN_EVERY {
        assert_eq!(r.record(Sit::A, OFFER.to_vec(), Some(Cand::X)).unwrap(), None);
    }
    assert!(!store.load_settings().auto_train);
    // Recording off: nothing is stored and nothing is fitted.
    r.set_auto_train(true).unwrap();
    r.set_record(false).unwrap();
    let n = store.read::<Sit, Cand>().unwrap().len();
    assert_eq!(r.record(Sit::A, OFFER.to_vec(), Some(Cand::X)).unwrap(), None);
    assert_eq!(store.read::<Sit, Cand>().unwrap().len(), n);
    std::fs::remove_dir_all(store.dir()).unwrap();
}

#[test]
fn a_refit_that_stops_passing_the_gate_switches_the_learned_ranking_off() {
    let store = temp("online-off");
    let mut r: Ranker<Sit, Cand> = Ranker::new(Some(store.clone()), 1000, "Test");
    store.write_corpus(&corpus(60, 12, Some(Cand::Z))).unwrap();
    r.train().unwrap();
    r.set_use_learned(true).unwrap();
    assert!(r.order(&Sit::A, &OFFER).1);
    // The user's own (many) sessions disagree with the prior: after the refit it is no longer used.
    for e in sessions(60, 12, Some(Cand::X)) {
        store.append(&e).unwrap();
    }
    r.train().unwrap();
    assert!(!r.settings.use_learned && !r.order(&Sit::A, &OFFER).1);
    std::fs::remove_dir_all(store.dir()).unwrap();
}
