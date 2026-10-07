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
    state.selected = model::field_rows().iter().position(|r| *r == row).unwrap();
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
    assert!(!matches!(model::field_rows()[first], FieldRow::Header(_)));
    handle_key(&mut s, &bushing(), key(KeyCode::Up));
    assert_eq!(s.selected, model::field_rows().len() - 1);
    handle_key(&mut s, &bushing(), key(KeyCode::Down));
    assert_eq!(s.selected, first);
}

#[test]
fn a_finished_run_is_kept_and_marked_stale_when_the_inputs_change() {
    let b = bushing();
    let mut s = EccentricState::default();
    let effects = s.start(&b, Task::Analyze);
    let [Effect::RunEccentric { id, task, input }] = effects.as_slice() else { panic!("{effects:?}") };
    let out = model::run(*task, input).unwrap();
    let Output::Analysis(a) = &out else { panic!() };
    assert!(a.design_capacity > 0.0 && a.torque_required > 0.0, "{a:?}");
    // Global equilibrium of the FE interface returns the pin load.
    assert!((a.net_force[1].abs() - b.load).abs() < 0.02 * b.load.max(1.0), "net force {:?} for {} lbf", a.net_force, b.load);
    s.finish(*id, Ok(out));
    assert!(s.job.is_none() && s.analysis.is_some());
    assert!(s.report_text(&b).is_some());
    s.ui.offset += 0.001;
    assert!(s.report_text(&b).is_none(), "a report only describes the inputs it ran for");
}

#[test]
fn a_stale_job_result_is_dropped() {
    let mut s = EccentricState::default();
    s.start(&bushing(), Task::Analyze);
    s.finish(999, Err("x".into()));
    assert!(s.job.is_some() && s.error.is_none());
}
