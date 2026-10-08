use super::model::{self, FieldRow, NumberTarget, Output, Task};
use super::*;
use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

fn bushing() -> BushingModel {
    BushingModel::default()
}

fn select(state: &mut EccentricState, row: FieldRow) {
    // Rows of the Advanced section exist only while it is open.
    state.ui.advanced_open = !model::field_rows(false).contains(&row);
    state.selected = model::field_rows(state.ui.advanced_open).iter().position(|r| *r == row).unwrap();
}

#[test]
fn the_default_bushing_workbench_model_gives_a_valid_solver_input() {
    let input = model::build_input(&bushing(), &EccentricUi::default()).unwrap();
    assert!(input.bore_dia > 0.0 && input.interference_dia > 0.0 && input.walls().0 > 0.0, "{input:?}");
    // The shared inputs are the Bushing Workbench's, not copies.
    assert!((input.bore_dia - bushing().output.bore_tol.nominal).abs() < 1e-12);
    assert!((input.load_lbf - bushing().load).abs() < 1e-12 && (input.friction - bushing().friction).abs() < 1e-12);
}

#[test]
fn an_offset_that_eats_the_wall_is_reported_not_run() {
    let mut s = EccentricState::default();
    s.ui.offset = 10.0;
    let effects = s.start(&bushing(), Task::Analyze);
    assert!(effects.is_empty() && s.job.is_none());
    assert!(s.error.as_deref().is_some_and(|e| e.contains("wall")), "{:?}", s.error);
}

#[test]
fn r_m_and_l_start_one_run_each_in_either_case_and_a_second_press_while_running_is_ignored() {
    for (c, task) in [('r', Task::Analyze), ('R', Task::Analyze), ('m', Task::MaxOffset), ('M', Task::MaxOffset), ('l', Task::MaxLoad), ('L', Task::MaxLoad)] {
        let mut s = EccentricState::default();
        let (consumed, effects) = handle_key(&mut s, &bushing(), key(KeyCode::Char(c)));
        assert!(consumed);
        let [Effect::RunEccentric { task: t, .. }] = effects.as_slice() else { panic!("{c}: {effects:?}") };
        assert_eq!(*t, task, "{c}");
        assert!(handle_key(&mut s, &bushing(), key(KeyCode::Char(c))).1.is_empty(), "{c}: a run is already going");
    }
}

#[test]
fn typing_a_number_edits_the_selected_row_and_the_value_is_clamped() {
    let mut s = EccentricState::default();
    select(&mut s, FieldRow::Number(NumberTarget::Offset));
    for c in "0.03".chars() {
        handle_key(&mut s, &bushing(), key(KeyCode::Char(c)));
    }
    assert!(s.editing);
    handle_key(&mut s, &bushing(), key(KeyCode::Enter));
    assert!(!s.editing && (s.ui.offset - 0.03).abs() < 1e-12);
    select(&mut s, FieldRow::Number(NumberTarget::BossFactor));
    for c in "0.2".chars() {
        handle_key(&mut s, &bushing(), key(KeyCode::Char(c)));
    }
    handle_key(&mut s, &bushing(), key(KeyCode::Enter));
    assert!((s.ui.boss_factor - 1.5).abs() < 1e-12, "the boss must stay wider than the bore");
    select(&mut s, FieldRow::Number(NumberTarget::LoadAngle));
    for c in "450".chars() {
        handle_key(&mut s, &bushing(), key(KeyCode::Char(c)));
    }
    handle_key(&mut s, &bushing(), key(KeyCode::Enter));
    assert!((s.ui.load_angle - 90.0).abs() < 1e-12, "angles wrap");
}

#[test]
fn up_down_skip_headers_and_wrap() {
    let mut s = EccentricState::default();
    let first = s.selected;
    assert!(!matches!(model::field_rows(false)[first], FieldRow::Header(_)));
    handle_key(&mut s, &bushing(), key(KeyCode::Up));
    assert_eq!(s.selected, model::field_rows(false).len() - 1);
    handle_key(&mut s, &bushing(), key(KeyCode::Down));
    assert_eq!(s.selected, first);
}

#[test]
fn a_finished_run_is_kept_and_marked_stale_when_the_inputs_change() {
    let b = bushing();
    let mut s = EccentricState::default();
    let effects = s.start(&b, Task::Analyze);
    let [Effect::RunEccentric { id, task, input, .. }] = effects.as_slice() else { panic!("{effects:?}") };
    let out = model::run(*task, input).unwrap();
    let Output::Analysis(a) = &out else { panic!() };
    assert!(a.design_capacity > 0.0 && a.torque_required > 0.0, "{a:?}");
    // Global equilibrium of the FE interface returns the pin load.
    assert!((a.net_force[1].abs() - b.load).abs() < 0.02 * b.load.max(1.0), "net force {:?} for {} lbf", a.net_force, b.load);
    s.finish(*id, Ok(out), &b);
    assert!(s.job.is_none() && s.analysis.is_some());
    assert!(s.report_text(&b).is_some());
    s.ui.offset += 0.001;
    assert!(s.report_text(&b).is_none(), "a report only describes the inputs it ran for");
}

#[test]
fn a_stale_job_result_is_dropped() {
    let mut s = EccentricState::default();
    s.start(&bushing(), Task::Analyze);
    s.finish(999, Err("x".into()), &bushing());
    assert!(s.job.is_some() && s.error.is_none());
}

#[test]
fn the_advanced_section_is_collapsed_by_default_and_toggles_with_space_or_enter() {
    let mut s = EccentricState::default();
    assert!(!model::field_rows(s.ui.advanced_open).contains(&FieldRow::ToggleDirectOnset), "collapsed: no advanced rows");
    select(&mut s, FieldRow::AdvancedSection);
    assert!(!s.ui.advanced_open);
    handle_key(&mut s, &bushing(), key(KeyCode::Char(' ')));
    assert!(s.ui.advanced_open && model::field_rows(true).contains(&FieldRow::ToggleDirectOnset));
    handle_key(&mut s, &bushing(), key(KeyCode::Enter));
    assert!(!s.ui.advanced_open, "Enter closes it again");
}

#[test]
fn c_cancels_the_running_job_clears_the_queue_and_a_cancelled_run_returns_at_once() {
    use std::sync::atomic::Ordering;
    let b = bushing();
    let mut s = EccentricState::default();
    let effects = s.start(&b, Task::Analyze);
    let [Effect::RunEccentric { task, input, control, .. }] = effects.as_slice() else { panic!("{effects:?}") };
    let flag = control.interrupt.cancel.clone().unwrap();
    assert!(!flag.load(Ordering::Relaxed));
    s.start(&b, Task::MaxLoad);
    assert_eq!(s.queue.len(), 1);
    for c in ['c', 'C'] {
        flag.store(false, Ordering::Relaxed);
        s.queue.push_back(Task::Sweep);
        let (consumed, fx) = handle_key(&mut s, &b, key(KeyCode::Char(c)));
        assert!(consumed && fx.is_empty() && flag.load(Ordering::Relaxed) && s.queue.is_empty(), "{c}");
    }
    let started = std::time::Instant::now();
    let err = model::run_controlled(*task, input, control).unwrap_err();
    assert_eq!(err, "cancelled");
    assert!(started.elapsed().as_secs_f64() < 5.0);
}

#[test]
fn runs_asked_for_while_one_is_going_wait_in_order_and_start_when_it_finishes() {
    let b = bushing();
    let mut s = EccentricState::default();
    let first = s.start(&b, Task::MaxOffset);
    let [Effect::RunEccentric { id, .. }] = first.as_slice() else { panic!("{first:?}") };
    for t in [Task::MaxLoad, Task::Sweep, Task::MaxLoad, Task::Analyze, Task::Fields, Task::MaxOffset] {
        assert!(s.start(&b, t).is_empty());
    }
    // The same task is not queued twice and the queue is bounded.
    assert_eq!(s.queue.iter().copied().collect::<Vec<_>>(), vec![Task::MaxLoad, Task::Sweep, Task::Analyze, Task::Fields]);
    let next = s.finish(*id, Err("x".into()), &b);
    let [Effect::RunEccentric { task, .. }] = next.as_slice() else { panic!("{next:?}") };
    assert_eq!(*task, Task::MaxLoad);
    assert_eq!(s.queue.len(), 3);
}

#[test]
fn s_v_and_x_start_the_sweep_and_the_field_export_and_write_the_csv_in_either_case() {
    for (c, task) in [('s', Task::Sweep), ('S', Task::Sweep), ('v', Task::Fields), ('V', Task::Fields)] {
        let mut s = EccentricState::default();
        let (consumed, effects) = handle_key(&mut s, &bushing(), key(KeyCode::Char(c)));
        let [Effect::RunEccentric { task: t, control, .. }] = effects.as_slice() else { panic!("{c}: {effects:?}") };
        assert!(consumed && *t == task && control.progress.is_some() && control.interrupt.cancel.is_some(), "{c}");
        // A sweep has a deadline, the single analysis behind a field export is bounded by its solver.
        assert_eq!(control.interrupt.deadline.is_some(), task == Task::Sweep);
    }
    let b = bushing();
    let mut s = EccentricState::default();
    assert!(handle_key(&mut s, &b, key(KeyCode::Char('x'))).1.is_empty(), "nothing to export yet");
    let input = model::build_input(&b, &s.ui).unwrap();
    let points = vec![
        eccentric_bushing::SweepPoint { offset: 0.0, margin: Some(f64::INFINITY), capacity: 150.0, required: 0.0, error: None },
        eccentric_bushing::SweepPoint { offset: 0.05, margin: Some(1.5), capacity: 150.0, required: 60.0, error: None },
        eccentric_bushing::SweepPoint { offset: 0.1, margin: None, capacity: 0.0, required: 0.0, error: Some("cancelled".into()) },
    ];
    let csv = model::csv_text(&input, None, Some(&points));
    assert!(csv.contains("offset_in,margin") && csv.contains("0.00000,inf,150.000,0.000,") && csv.contains("0.05000,1.50000,150.000,60.000,") && csv.contains("0.10000,,0.000,0.000,cancelled"), "{csv}");
    s.sweep = Some((input, points));
    let fx = handle_key(&mut s, &b, key(KeyCode::Char('X'))).1;
    assert!(matches!(fx.as_slice(), [Effect::WriteTextFile { path, contents }] if path.ends_with("eccentric-bushing.csv") && contents.contains("0.05000")), "{fx:?}");
}

#[test]
fn a_running_job_shows_its_progress_and_the_queue_in_the_results() {
    let b = bushing();
    let mut s = EccentricState::default();
    let effects = s.start(&b, Task::MaxLoad);
    let [Effect::RunEccentric { control, .. }] = effects.as_slice() else { panic!("{effects:?}") };
    s.start(&b, Task::Sweep);
    let p = control.progress.clone().unwrap();
    // The worker's side: a stage, a step event and a search bracket become the display lines.
    p.set(eccentric_bushing::ProgressState { stage: "pin load".into(), detail: "e 0.0200 in, 8856 lbf, pin load: step 3, pin carries 1180 lbf".into(), solves_done: 5, bracket: Some((0.0475, 0.0612)) });
    let theme = crate::theme::Theme::default();
    let text: String = super::view::readout_lines(&theme, &s, &b).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("searching the maximum load") && text.contains("c cancels") && text.contains("queued next: sweep"), "{text}");
    assert!(text.contains("pin carries 1180 lbf") && text.contains("5 solves done") && text.contains("0.0475 to 0.0612"), "{text}");
}

#[test]
fn a_stopped_search_reports_its_verified_bracket() {
    let l = eccentric_bushing::OffsetLimit { value: 0.0123, upper: Some(0.0456), halted: Some("time budget used up".into()), caveat: None, wall_limit: 0.1, bounded_by_wall: false, set_by_loaded_run: true, evaluations: 8 };
    let note = model::halted_note(&l, "in", 4).unwrap();
    assert!(note.contains("time budget used up") && note.contains("between 0.0123 and 0.0456 in"), "{note}");
    let open = model::halted_note(&eccentric_bushing::OffsetLimit { upper: None, ..l.clone() }, "in", 4).unwrap();
    assert!(open.contains("0.0123 in is verified to hold"), "{open}");
    assert!(model::halted_note(&eccentric_bushing::OffsetLimit { halted: None, ..l.clone() }, "in", 4).is_none());
    let text = model::report_text(&model::build_input(&bushing(), &EccentricUi::default()).unwrap(), None, Some(&l), None);
    assert!(text.contains("stopped early"), "{text}");
    let unsolved = eccentric_bushing::OffsetLimit { halted: None, caveat: Some("the solve at 0.1 failed (x)".into()), ..l.clone() };
    assert!(model::caveat_note(&unsolved).is_some_and(|n| n.starts_with("caution")) && model::caveat_note(&l).is_none());
}

#[test]
fn analyses_are_kept_for_comparison_and_an_identical_rerun_replaces_the_newest() {
    let b = bushing();
    let mut s = EccentricState::default();
    // A cheap real analysis (the fit alone on a coarse mesh) stands in for the solver's output.
    let base = {
        let mut i = model::build_input(&b, &s.ui).unwrap();
        i.load_lbf = 0.0;
        i.mesh_size = Some(0.1);
        eccentric_bushing::analyze(&i).unwrap()
    };
    // Offsets 0.001 .. 0.007 with one identical rerun of 0.002: seven distinct entries, the limit keeps the newest six.
    for (i, offset) in [0.001, 0.002, 0.002, 0.003, 0.004, 0.005, 0.006, 0.007].into_iter().enumerate() {
        s.ui.offset = offset;
        let eff = s.start(&b, Task::Analyze);
        let [Effect::RunEccentric { id, .. }] = eff.as_slice() else { panic!("{eff:?}") };
        s.finish(*id, Ok(Output::Analysis(Box::new(eccentric_bushing::Analysis { margin: 2.0 - 0.1 * i as f64, ..base.clone() }))), &b);
    }
    let offsets: Vec<f64> = s.history.iter().map(|h| h.offset).collect();
    assert_eq!(offsets, vec![0.002, 0.003, 0.004, 0.005, 0.006, 0.007]);
    // The rerun of 0.002 replaced its first run: the kept margin is the later one (index 2 of the loop).
    assert!((s.history[0].margin - 1.8).abs() < 1e-12, "{}", s.history[0].margin);
    let theme = crate::theme::Theme::default();
    let text: String = super::view::readout_lines(&theme, &s, &b).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("Earlier analyses") && text.contains("latest") && text.contains("points"), "{text}");
}
