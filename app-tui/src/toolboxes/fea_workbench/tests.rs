use super::*;
use crossterm::event::{KeyEventKind, KeyEventState};
use fea_problem::Analysis;
use model::FieldRow;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

fn select(s: &mut FeaWorkbenchState, row: FieldRow) {
    s.selected = s.rows().iter().position(|r| *r == row).unwrap_or_else(|| panic!("no row {row:?}"));
}

fn type_text(s: &mut FeaWorkbenchState, text: &str) {
    for c in text.chars() {
        handle_key(s, key(KeyCode::Char(c)));
    }
}

/// Run the effects a tick asked for the way `main.rs` would, and feed the results back.
fn run_effects(s: &mut FeaWorkbenchState, effects: Vec<Effect>) {
    for e in effects {
        match e {
            Effect::RunFeaPreview { id, problem, import_text } => s.finish_preview(id, fea_problem::solve::preview(&problem, import_text.as_deref())),
            Effect::RunFeaSolve { id, problem, import_text } => s.finish_solve(id, fea_problem::solve(&problem, import_text.as_deref())),
            other => panic!("unexpected effect {other:?}"),
        }
    }
}

fn settle(s: &mut FeaWorkbenchState) {
    // The debounce only matters for the clock; back-date it.
    s.seen = s.seen.take().map(|(p, _)| (p, Instant::now() - std::time::Duration::from_secs(5)));
}

#[test]
fn the_first_tick_meshes_then_a_small_model_solves_by_itself() {
    let mut s = FeaWorkbenchState::default();
    let e = s.tick();
    assert!(matches!(e.as_slice(), [Effect::RunFeaPreview { .. }]), "{e:?}");
    run_effects(&mut s, e);
    assert!(s.preview.is_some() && s.solved.is_none());
    settle(&mut s);
    let e = s.tick();
    assert!(matches!(e.as_slice(), [Effect::RunFeaSolve { .. }]), "{e:?}");
    run_effects(&mut s, e);
    let sol = s.solved.as_ref().expect("solved");
    assert!(sol.summary.max_von_mises.value > 0.0);
    assert!(s.tick().is_empty(), "nothing more to do for unchanged inputs");
    assert!(!s.stale());
}

#[test]
fn editing_a_load_resolves_without_remeshing() {
    let mut s = FeaWorkbenchState::default();
    let e = s.tick();
    run_effects(&mut s, e);
    settle(&mut s);
    let e = s.tick();
    run_effects(&mut s, e);
    let before = s.solved.as_ref().unwrap().summary.max_von_mises.value;
    select(&mut s, FieldRow::LoadParam(0, 0));
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..8 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "20000");
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(s.stale());
    let e = s.tick();
    assert!(e.is_empty(), "debounce: nothing yet");
    settle(&mut s);
    let e = s.tick();
    assert!(matches!(e.as_slice(), [Effect::RunFeaSolve { .. }]), "the mesh is still valid, only the solve reruns: {e:?}");
    run_effects(&mut s, e);
    let after = s.solved.as_ref().unwrap().summary.max_von_mises.value;
    assert!((after / before - 2.0).abs() < 1e-6, "double the traction doubles the stress: {before} -> {after}");
}

#[test]
fn a_stale_job_result_is_dropped() {
    let mut s = FeaWorkbenchState::default();
    let e = s.tick();
    let Some(Effect::RunFeaPreview { id, problem, import_text }) = e.into_iter().next() else { panic!() };
    // The inputs change while the worker runs: a different mesh is now wanted.
    select(&mut s, FieldRow::MeshSize);
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..6 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "0.4");
    handle_key(&mut s, key(KeyCode::Enter));
    s.tick();
    s.finish_preview(id, fea_problem::solve::preview(&problem, import_text.as_deref()));
    assert!(s.preview.is_none(), "the result for the old mesh size must not be shown");
}

#[test]
fn invalid_inputs_report_why_instead_of_running() {
    let mut s = FeaWorkbenchState::default();
    s.problem.mesh.size = 0.0;
    assert!(s.tick().is_empty());
    assert!(s.preview_error.as_deref().unwrap().contains("mesh size"));
    s.problem.mesh.size = 0.5;
    s.tick();
    settle(&mut s);
    let e = s.tick();
    run_effects(&mut s, e);
    assert!(s.preview_error.is_none(), "the error clears once the inputs are valid");
    s.problem.supports.clear();
    s.start_solve();
    assert!(s.solve_error.as_deref().unwrap().contains("support"));
}

#[test]
fn rows_follow_the_problem_and_navigation_skips_headers() {
    let mut s = FeaWorkbenchState::default();
    assert!(!matches!(s.rows()[s.selected], FieldRow::Header(_)));
    handle_key(&mut s, key(KeyCode::Up));
    assert_eq!(s.selected, s.rows().len() - 1, "wraps to the last row");
    handle_key(&mut s, key(KeyCode::Down));
    assert!(!matches!(s.rows()[s.selected], FieldRow::Header(_)));
    // 3D adds the extrusion rows and the z displacement of every support.
    s.problem.analysis = Analysis::Solid;
    let rows = s.rows();
    assert!(rows.contains(&FieldRow::Depth) && rows.contains(&FieldRow::SupportComp(0, 2)));
    assert!(!rows.contains(&FieldRow::Thickness));
}

#[test]
fn the_analysis_row_cycles_and_a_3d_extrusion_gets_a_quad_element() {
    let mut s = FeaWorkbenchState::default();
    s.problem.mesh.element = fea_problem::ElementChoice::Tri6;
    select(&mut s, FieldRow::Analysis);
    for _ in 0..3 {
        handle_key(&mut s, key(KeyCode::Right));
    }
    assert_eq!(s.problem.analysis, Analysis::Solid);
    assert!(s.problem.mesh.element.is_quad());
    handle_key(&mut s, key(KeyCode::Left));
    assert_eq!(s.problem.analysis, Analysis::Axisymmetric);
}

#[test]
fn supports_and_loads_can_be_added_cycled_and_removed() {
    let mut s = FeaWorkbenchState::default();
    let (ns, nl) = (s.problem.supports.len(), s.problem.loads.len());
    select(&mut s, FieldRow::SupportAdd);
    handle_key(&mut s, key(KeyCode::Enter));
    select(&mut s, FieldRow::LoadAdd);
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!((s.problem.supports.len(), s.problem.loads.len()), (ns + 1, nl + 1));
    // Edge names cycle through the sketch's edges.
    select(&mut s, FieldRow::SupportEdge(ns));
    let first = s.problem.supports[ns].edge().unwrap().to_string();
    handle_key(&mut s, key(KeyCode::Right));
    assert_ne!(s.problem.supports[ns].edge().unwrap(), first);
    handle_key(&mut s, key(KeyCode::Left));
    assert_eq!(s.problem.supports[ns].edge().unwrap(), first);
    // A component is released and fixed again with Space, or typed.
    select(&mut s, FieldRow::SupportComp(ns, 0));
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert_eq!(s.problem.supports[ns].comps()[0], None);
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..8 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "0.01");
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!(s.problem.supports[ns].comps()[0], Some(0.01));
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..8 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "free");
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!(s.problem.supports[ns].comps()[0], None);
    // Load kinds cycle; removing restores the counts.
    select(&mut s, FieldRow::LoadKind(nl));
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!(s.problem.loads[nl].label(), "Traction");
    select(&mut s, FieldRow::LoadRemove(nl));
    handle_key(&mut s, key(KeyCode::Enter));
    select(&mut s, FieldRow::SupportRemove(ns));
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!((s.problem.supports.len(), s.problem.loads.len()), (ns, nl));
}

#[test]
fn holes_and_polygon_points_are_editable() {
    let mut s = FeaWorkbenchState::default();
    select(&mut s, FieldRow::HoleAdd);
    handle_key(&mut s, key(KeyCode::Enter));
    let Geometry::Sketch { holes, .. } = &s.problem.geometry else { panic!() };
    assert_eq!(holes.len(), 2);
    select(&mut s, FieldRow::HoleKind(1));
    handle_key(&mut s, key(KeyCode::Enter));
    let Geometry::Sketch { holes, .. } = &s.problem.geometry else { panic!() };
    assert_eq!(holes[1].label(), "Slot");
    select(&mut s, FieldRow::HoleRemove(1));
    handle_key(&mut s, key(KeyCode::Enter));
    // Polygon outline: kind cycles Rect -> Circle -> Slot -> Polygon, then points add and remove.
    select(&mut s, FieldRow::OuterKind);
    for _ in 0..3 {
        handle_key(&mut s, key(KeyCode::Enter));
    }
    assert!(s.rows().contains(&FieldRow::PolyPoint(3, 1)));
    select(&mut s, FieldRow::PolyAdd);
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(s.rows().contains(&FieldRow::PolyPoint(4, 0)));
    select(&mut s, FieldRow::PolyRemove);
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(!s.rows().contains(&FieldRow::PolyPoint(4, 0)));
    assert!(s.problem.edge_names().contains(&"edge4".to_string()));
}

#[test]
fn the_geometry_source_toggles_to_an_imported_file_and_back_keeping_the_sketch() {
    let mut s = FeaWorkbenchState::default();
    let sketch = s.problem.geometry.clone();
    select(&mut s, FieldRow::Source);
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(matches!(s.problem.geometry, Geometry::Imported { .. }));
    assert!(s.rows().contains(&FieldRow::ImportPath) && !s.rows().contains(&FieldRow::OuterKind));
    // Typing a path asks for the file to be read.
    select(&mut s, FieldRow::ImportPath);
    handle_key(&mut s, key(KeyCode::Enter));
    type_text(&mut s, "bar.inp");
    let (_, e) = handle_key(&mut s, key(KeyCode::Enter));
    assert!(matches!(e.as_slice(), [Effect::ReadTextFile { purpose: FilePurpose::Mesh, path }] if path == "bar.inp"), "{e:?}");
    let inp = "*NODE\n1,0,0\n2,1,0\n3,2,0\n4,0,1\n5,1,1\n6,2,1\n*ELEMENT, TYPE=CPS4\n1,1,2,5,4\n2,2,3,6,5\n*NSET, NSET=LEFT\n1,4\n*NSET, NSET=RIGHT\n3,6\n";
    s.file_read(FilePurpose::Mesh, "bar.inp".into(), Ok(inp.into()));
    let e = s.tick();
    run_effects(&mut s, e);
    assert!(s.names().contains(&"LEFT".to_string()), "imported names are offered: {:?}", s.names());
    select(&mut s, FieldRow::Source);
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!(s.problem.geometry, sketch);
}

#[test]
fn problems_open_from_json_and_a_bad_file_is_reported() {
    let mut s = FeaWorkbenchState::default();
    let (_, other) = templates().into_iter().nth(2).unwrap();
    let msg = s.file_read(FilePurpose::Problem, "p.json".into(), Ok(other.to_json().unwrap())).unwrap();
    assert!(msg.0.contains("Thick cylinder"), "{msg:?}");
    assert_eq!(s.problem, other);
    assert!(s.preview.is_none() && s.solved.is_none(), "results belong to the old problem");
    let msg = s.file_read(FilePurpose::Problem, "p.json".into(), Ok("{}".into())).unwrap();
    assert_eq!(msg.1, crate::theme::StatusTone::Danger);
    let msg = s.file_read(FilePurpose::Problem, "p.json".into(), Err("no such file".into())).unwrap();
    assert!(msg.0.contains("no such file"));
}

#[test]
fn letter_keys_work_in_both_cases_for_windows_caps_lock() {
    let mut s = FeaWorkbenchState::default();
    let f0 = s.field;
    handle_key(&mut s, key(KeyCode::Char('V')));
    assert_ne!(s.field, f0);
    let m = s.show_mesh;
    handle_key(&mut s, key(KeyCode::Char('M')));
    assert_ne!(s.show_mesh, m);
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('R')));
    assert!(matches!(e.as_slice(), [Effect::RunFeaSolve { .. }]), "{e:?}");
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('J')));
    assert!(matches!(e.as_slice(), [Effect::WriteTextFile { path, .. }] if path.ends_with(".json")), "{e:?}");
}

#[test]
fn exports_need_a_result_and_name_their_files_after_the_problem() {
    let mut s = FeaWorkbenchState::default();
    assert!(handle_key(&mut s, key(KeyCode::Char('e'))).1.is_empty());
    assert!(handle_key(&mut s, key(KeyCode::Char('p'))).1.is_empty());
    let e = s.tick();
    run_effects(&mut s, e);
    settle(&mut s);
    let e = s.tick();
    run_effects(&mut s, e);
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('e')));
    assert!(matches!(e.as_slice(), [Effect::WriteTextFileAndOpen { path, contents }] if path.ends_with("plate-with-a-hole.txt") && contents.contains("Equilibrium error")), "{e:?}");
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('p')));
    assert!(matches!(e.as_slice(), [Effect::WriteTextFile { path, contents }] if path.ends_with("plate-with-a-hole.vtu") && contents.contains("von_mises")), "{e:?}");
}

#[test]
fn the_material_browser_sets_the_material_properties() {
    let mut s = FeaWorkbenchState::default();
    select(&mut s, FieldRow::Material);
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(s.material_browser.is_some());
    type_text(&mut s, "steel 4340");
    let want = crate::toolboxes::material_lookup::model::catalog()[s.material_browser.as_ref().unwrap().current().unwrap()];
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(s.material_browser.is_none());
    let m = &s.problem.material;
    assert_eq!(m.name, want.name);
    assert!(want.name.contains("4340"), "{}", want.name);
    assert!((m.e - want.e_ksi * 1000.0).abs() < 1e-6 && (m.nu - want.nu).abs() < 1e-12);
    assert!((m.alpha - want.alpha_u_f * 1e-6).abs() < 1e-15);
    assert_eq!(m.yield_stress, Some(want.sy_ksi * 1000.0));
    assert!(m.e > 2.0e7, "psi, not ksi: {}", m.e);
}

#[test]
fn a_bushing_is_added_edited_and_dropped_with_its_hole_and_a_bushed_problem_is_not_solved_automatically() {
    let mut s = FeaWorkbenchState::default();
    // "Plate with a hole": one circular hole.
    assert!(!s.rows().iter().any(|r| matches!(r, FieldRow::BushingParam(..))));
    select(&mut s, FieldRow::BushingAdd);
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert_eq!(s.problem.bushings.len(), 1);
    assert!(s.rows().contains(&FieldRow::BushingParam(0, 4)));
    // Edit the interference (row 4: "Interference (dia)").
    select(&mut s, FieldRow::BushingParam(0, 4));
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..12 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "0.003");
    handle_key(&mut s, key(KeyCode::Enter));
    assert!((s.problem.bushings[0].interference - 0.003).abs() < 1e-12, "{}", s.problem.bushings[0].interference);
    // A second Add finds no hole without a bushing.
    select(&mut s, FieldRow::BushingAdd);
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert_eq!(s.problem.bushings.len(), 1);
    // The preview meshes, but the contact solve is not started by itself.
    let e = s.tick();
    run_effects(&mut s, e);
    settle(&mut s);
    assert!(s.tick().is_empty(), "a bushed problem solves on request only");
    // Removing the hole removes its bushing.
    select(&mut s, FieldRow::HoleRemove(0));
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert!(s.problem.bushings.is_empty());
}
