use super::*;
use crossterm::event::{KeyEventKind, KeyEventState};
use fea_problem::Analysis;
use model::FieldRow;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

/// The terminal Workbench is manual by default (ADR-016); these tests exercise its automatic mode.
fn auto_state() -> FeaWorkbenchState {
    FeaWorkbenchState { auto_solve: true, ..FeaWorkbenchState::default() }
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
            Effect::RunFeaDynamic { id, problem, import_text, kind, n_modes } => s.finish_dynamic(id, fea_problem::dynamics::run(kind, &problem, import_text.as_deref(), n_modes)),
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
    let mut s = auto_state();
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
    let mut s = auto_state();
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
    let mut s = auto_state();
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
    let mut s = auto_state();
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
fn a_polygon_hole_lists_its_points_and_edits_a_coordinate() {
    let mut s = FeaWorkbenchState::default();
    let Geometry::Sketch { holes, .. } = &mut s.problem.geometry else { panic!() };
    holes.push(fea_problem::Shape::Polygon { pts: vec![[4.0, 0.3], [4.2, 0.3], [4.1, 0.5]] });
    let h = holes.len() - 1;
    let row = FieldRow::HolePoint(h, 1, 0);
    assert!(s.rows().contains(&row));
    assert_eq!(model::row_value(&s.problem, &[], None, FieldRow::HoleKind(h)), "Polygon (3 editable points)");
    assert_eq!(model::row_label(&s.problem, row), "  Point 2 x");
    assert_eq!(model::number_value(&s.problem, row), Some(4.2));
    model::set_number(&mut s.problem, row, 4.25);
    assert_eq!(model::number_value(&s.problem, row), Some(4.25));
}

#[test]
fn the_geometry_source_toggles_to_an_imported_file_and_back_keeping_the_sketch() {
    let mut s = auto_state();
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
fn open_prompt_browses_and_keeps_typed_path() {
    let dir = std::env::temp_dir().join(format!("fea-browser-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("part.fea.json");
    std::fs::write(&file, FeaWorkbenchState::default().problem.to_json().unwrap()).unwrap();
    std::fs::write(dir.join("invalid.json"), "{}").unwrap();
    let mut s = FeaWorkbenchState::default();
    handle_key(&mut s, key(KeyCode::Char('o')));
    s.prompt.as_mut().unwrap().buffer.set(dir.join("typed.json").display().to_string());
    handle_key(&mut s, key(KeyCode::Tab));
    let browser = s.prompt.as_ref().unwrap().browser.as_ref().unwrap();
    assert!(browser.entries.contains(&file));
    handle_key(&mut s, key(KeyCode::Tab));
    assert!(s.prompt.as_ref().unwrap().browser.is_none());
    assert_eq!(s.prompt.as_ref().unwrap().buffer.trim(), dir.join("typed.json").to_string_lossy());
    handle_key(&mut s, key(KeyCode::Tab));
    handle_key(&mut s, key(KeyCode::Esc));
    assert!(s.prompt.is_none(), "Esc cancels browsing and the open flow");
    handle_key(&mut s, key(KeyCode::Char('o')));
    s.prompt.as_mut().unwrap().buffer.set(dir.join("typed.json").display().to_string());
    handle_key(&mut s, key(KeyCode::Tab));
    let browser = s.prompt.as_mut().unwrap().browser.as_mut().unwrap();
    browser.selected = browser.entries.iter().position(|p| p == &file).unwrap();
    let (_, effects) = handle_key(&mut s, key(KeyCode::Enter));
    assert!(matches!(effects.as_slice(), [Effect::ReadTextFile { purpose: FilePurpose::Problem, path }] if path == &file.to_string_lossy()));
    assert!(s.prompt.is_none());
    let msg = s.file_read(FilePurpose::Problem, file.display().to_string(), std::fs::read_to_string(&file).map_err(|e| e.to_string())).unwrap();
    assert_eq!(msg.1, crate::theme::StatusTone::Success);
    let bad = std::fs::read_to_string(dir.join("invalid.json")).unwrap();
    assert_eq!(s.file_read(FilePurpose::Problem, "invalid.json".into(), Ok(bad)).unwrap().1, crate::theme::StatusTone::Danger);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn browser_starts_at_existing_ancestor_of_unsaved_problem_path() {
    let dir = std::env::temp_dir();
    let missing = dir.join(format!("fea-not-created-{}", std::process::id())).join("case.json");
    let browser = FileBrowser::open(&missing.display().to_string());
    assert_eq!(browser.dir, dir);
    assert!(browser.error.is_none());
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
    let mut s = auto_state();
    assert!(handle_key(&mut s, key(KeyCode::Char('e'))).1.is_empty());
    assert!(handle_key(&mut s, key(KeyCode::Char('p'))).1.is_empty());
    let e = s.tick();
    run_effects(&mut s, e);
    settle(&mut s);
    let e = s.tick();
    run_effects(&mut s, e);
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('e')));
    assert!(matches!(e.as_slice(), [Effect::WriteTextFileAndOpen { path, contents }] if path.ends_with("plate-with-a-hole.txt") && contents.contains("Equilibrium error") && contents.contains("Benchmark (Heywood")), "{e:?}");
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('p')));
    assert!(matches!(e.as_slice(), [Effect::WriteTextFile { path, contents }] if path.ends_with("plate-with-a-hole.vtu") && contents.contains("von_mises")), "{e:?}");
    let (_, e) = handle_key(&mut s, key(KeyCode::Char('C')));
    let [Effect::WriteTextFile { path, contents }] = e.as_slice() else { panic!("{e:?}") };
    let mut lines = contents.lines();
    assert!(path.ends_with("plate-with-a-hole.csv") && lines.next().unwrap().starts_with("node,x,y,ux,uy,sxx,syy,szz,sxy,syz,szx,von_mises"), "{path}");
    // One row per node, every value finite, 14 columns (index, x, y, ux, uy, six stresses, three derived).
    let rows: Vec<&str> = lines.collect();
    assert!(rows.len() > 50 && rows.iter().all(|r| r.split(',').count() == 14 && r.split(',').all(|v| v.parse::<f64>().is_ok_and(f64::is_finite))));
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

#[test]
fn natural_frequencies_run_on_a_worker_and_a_mode_is_drawn() {
    let mut s = FeaWorkbenchState::default();
    let e = s.tick();
    run_effects(&mut s, e);
    // The default template is a plate with a hole; its steel material carries a density.
    let (consumed, effects) = handle_key(&mut s, key(KeyCode::Char('n')));
    assert!(consumed && matches!(effects.as_slice(), [Effect::RunFeaDynamic { .. }]), "{effects:?}");
    // A second request while one runs does nothing.
    assert!(handle_key(&mut s, key(KeyCode::Char('n'))).1.is_empty());
    assert!(s.analysing().is_some());
    run_effects(&mut s, effects);
    assert!(s.analysing().is_none());
    // Plate with a hole has a roller + point support: a mechanism-free plane model, so modes exist or an explained error does.
    match (s.dynamic.as_ref().map(|d| d.n_modes()), s.dyn_error.clone()) {
        (Some(n_modes), None) => {
            assert!(n_modes > 0 && s.dynamic_shown().is_some());
            let first = s.mode;
            handle_key(&mut s, key(KeyCode::Char(']')));
            assert_eq!(s.mode, (first + 1) % n_modes);
            handle_key(&mut s, key(KeyCode::Char('[')));
            assert_eq!(s.mode, first);
            // Editing an input makes the result stale (no longer shown) without discarding it.
            s.problem.material.e *= 1.1;
            assert!(s.dynamic_shown().is_none());
            // r returns to the static result display.
            s.show_dynamic = true;
            handle_key(&mut s, key(KeyCode::Char('r')));
            assert!(!s.show_dynamic);
        }
        (_, Some(e)) => assert!(!e.is_empty()),
        _ => panic!("neither a result nor an error"),
    }
}

#[test]
fn buckling_without_a_density_still_runs_and_frequencies_explain_the_missing_density() {
    let mut s = FeaWorkbenchState::default();
    s.problem.material.density = 0.0;
    let e = handle_key(&mut s, key(KeyCode::Char('n'))).1;
    run_effects(&mut s, e);
    assert!(s.dyn_error.as_deref().is_some_and(|m| m.contains("density")), "{:?}", s.dyn_error);
    assert!(s.dynamic.is_none());
}

#[test]
fn member_editor_key_opens_the_native_editor_on_the_member_file() {
    for c in ['f', 'F'] {
        let mut s = FeaWorkbenchState::default();
        let (handled, effects) = handle_key(&mut s, key(KeyCode::Char(c)));
        assert!(handled);
        let Some(Effect::OpenNativeEditor { kind, path }) = effects.first() else { panic!("{effects:?}") };
        assert_eq!(*kind, crate::gpu_viewer::EditorKind::Members);
        assert!(path.ends_with("member-model.json"), "{path}");
    }
}

#[test]
fn solid_editor_key_hands_the_current_problem_to_the_native_editor_and_reads_it_back() {
    let dir = std::env::temp_dir().join(format!("wb-solid-handoff-{}", std::process::id()));
    let mut s = FeaWorkbenchState::default();
    s.problem.name = "handed over".into();
    let effects = s.open_solid_editor_in(Some(dir.clone()));
    let Some(Effect::OpenNativeEditor { kind, path }) = effects.first() else { panic!("{effects:?}") };
    assert_eq!(*kind, crate::gpu_viewer::EditorKind::Solid);
    assert!(path.ends_with("native-solid.json"));
    let written = Problem::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(written, s.problem, "the native editor starts from the Workbench's problem");
    assert!(s.open_solid_editor_in(None).is_empty() && s.solve_error.as_ref().is_some_and(|e| e.contains("no app-data folder")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_thermal_only_study_shows_temperatures_not_stresses_and_cycles_only_its_field() {
    let mut s = auto_state();
    // The study row cycles Structural -> Thermal and swaps the supports / loads sections for the thermal ones.
    select(&mut s, FieldRow::StudyKind);
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!(s.problem.study, fea_problem::Study::Thermal);
    let rows = s.rows();
    assert!(rows.contains(&FieldRow::Conductivity) && !rows.contains(&FieldRow::SupportAdd));
    // A fixed temperature on the first edge and a flux on the last (the Add rows pick the first edge: edit the second).
    select(&mut s, FieldRow::HeatSupportAdd);
    handle_key(&mut s, key(KeyCode::Enter));
    select(&mut s, FieldRow::HeatLoadAdd);
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!((s.problem.thermal.supports.len(), s.problem.thermal.loads.len()), (1, 1));
    s.problem.thermal.loads[0].set_edge("right");
    let e = s.tick();
    run_effects(&mut s, e);
    settle(&mut s);
    let e = s.tick();
    run_effects(&mut s, e);
    let sol = s.solved.as_ref().expect("solved");
    assert!(sol.summary.heat.is_some() && sol.temperature.is_some());
    // 'v' cycles the displayed field through the fields the result has: just the temperature.
    s.field = fea_problem::Field::VonMises;
    s.problem.study = fea_problem::Study::Thermal;
    let all = s.solved.as_ref().unwrap().fields();
    assert_eq!(all, vec![fea_problem::Field::Temperature]);
    let text = view::readout_lines(&crate::theme::Theme::default(), &s).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("Temperature"), "{text}");
    assert!(!text.contains("Peak von Mises"), "{text}");
}

#[test]
fn nothing_meshes_or_solves_by_itself_until_asked() {
    let mut s = FeaWorkbenchState::default();
    assert!(!s.auto_solve);
    for _ in 0..3 {
        assert!(s.tick().is_empty(), "no automatic mesh");
        settle(&mut s);
    }
    assert!(s.preview.is_none() && s.solved.is_none());
    let text = view::readout_lines(&crate::theme::Theme::default(), &s).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("u generates the mesh"), "{text}");
    // `u` builds the mesh once ...
    handle_key(&mut s, key(KeyCode::Char('u')));
    let e = s.tick();
    assert!(matches!(e.as_slice(), [Effect::RunFeaPreview { .. }]), "{e:?}");
    run_effects(&mut s, e);
    assert!(s.preview.is_some() && s.solved.is_none());
    // ... and a changed input leaves it out of date instead of rebuilding it.
    select(&mut s, FieldRow::MeshSize);
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..8 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "0.7");
    handle_key(&mut s, key(KeyCode::Enter));
    settle(&mut s);
    assert!(s.tick().is_empty(), "an edit does not remesh by itself");
    // `r` still solves (it meshes inside the worker); `a` switches the automatic mode on.
    let e = handle_key(&mut s, key(KeyCode::Char('r'))).1;
    assert!(matches!(e.as_slice(), [Effect::RunFeaSolve { .. }]), "{e:?}");
    run_effects(&mut s, e);
    assert!(s.solved.is_some());
    handle_key(&mut s, key(KeyCode::Char('a')));
    assert!(s.auto_solve);
}

#[test]
fn the_mesh_strategy_and_adaptivity_driver_rows_cycle_and_the_driver_needs_adaptive_passes() {
    let mut s = FeaWorkbenchState::default();
    assert!(s.rows().contains(&FieldRow::MeshStrategyRow));
    select(&mut s, FieldRow::MeshStrategyRow);
    assert_eq!(s.problem.mesh.strategy, fea_problem::MeshStrategy::Hybrid, "a new problem meshes hybrid by default");
    handle_key(&mut s, key(KeyCode::Right));
    assert_eq!(s.problem.mesh.strategy, fea_problem::MeshStrategy::Unstructured);
    handle_key(&mut s, key(KeyCode::Left));
    assert_eq!(s.problem.mesh.strategy, fea_problem::MeshStrategy::Hybrid);
    s.problem.mesh.adapt_passes = 2;
    select(&mut s, FieldRow::AdaptModeRow);
    handle_key(&mut s, key(KeyCode::Right));
    assert_eq!(s.problem.mesh.adapt_mode, fea_problem::AdaptMode::Equilibrated);
    handle_key(&mut s, key(KeyCode::Right));
    assert_eq!(s.problem.mesh.adapt_mode, fea_problem::AdaptMode::GoalOriented);
    s.problem.mesh.adapt_passes = 0;
    assert!(!s.rows().contains(&FieldRow::AdaptModeRow), "no passes, no driver to choose");
}

#[test]
fn the_predictor_the_plastic_material_and_the_region_rows_edit_the_problem() {
    let mut s = FeaWorkbenchState::default();
    // size predictor
    select(&mut s, FieldRow::PredictRow);
    assert!(!s.problem.mesh.predict_sizes);
    handle_key(&mut s, key(KeyCode::Right));
    assert!(s.problem.mesh.predict_sizes);
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert!(!s.problem.mesh.predict_sizes);
    // elastic-plastic: the hardening row exists only while it is on, and a material without yield cannot be made plastic
    assert!(s.rows().contains(&FieldRow::PlasticRow) && !s.rows().contains(&FieldRow::HardeningRow));
    select(&mut s, FieldRow::PlasticRow);
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert!(s.problem.material.plastic && s.rows().contains(&FieldRow::HardeningRow));
    select(&mut s, FieldRow::HardeningRow);
    handle_key(&mut s, key(KeyCode::Enter));
    for _ in 0..8 {
        handle_key(&mut s, key(KeyCode::Backspace));
    }
    type_text(&mut s, "250000");
    handle_key(&mut s, key(KeyCode::Enter));
    assert_eq!(s.problem.material.hardening, 250_000.0);
    s.problem.material.yield_stress = None;
    s.problem.material.plastic = false;
    select(&mut s, FieldRow::PlasticRow);
    handle_key(&mut s, key(KeyCode::Char(' ')));
    assert!(!s.problem.material.plastic, "no yield stress, no plasticity");
    // a sketch with a second region offers the bond switch and a material per region
    assert!(!s.rows().contains(&FieldRow::BondRegionsRow));
    if let fea_problem::Geometry::Sketch { regions, .. } = &mut s.problem.geometry {
        regions.push(fea_problem::SketchRegion { outer: fea_problem::Shape::Rect { x0: 20.0, y0: 0.0, x1: 22.0, y1: 1.0 }, holes: Vec::new() });
    }
    assert!(s.rows().contains(&FieldRow::BondRegionsRow) && s.rows().contains(&FieldRow::RegionMaterial(0)));
    select(&mut s, FieldRow::BondRegionsRow);
    handle_key(&mut s, key(KeyCode::Right));
    assert!(s.problem.mesh.bond_regions);
    select(&mut s, FieldRow::RegionMaterial(0));
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(s.material_browser.is_some(), "Enter opens the material browser for the region");
    let want = crate::toolboxes::material_lookup::model::catalog()[s.material_browser.as_ref().unwrap().current().unwrap()];
    handle_key(&mut s, key(KeyCode::Enter));
    assert!(s.material_browser.is_none());
    let got = s.problem.region_materials[0].as_ref().expect("the region has a material of its own");
    assert_eq!(got.name, want.name);
    assert_ne!(s.problem.material.name, "", "the part's material is untouched");
    assert!((got.e - want.e_ksi * 1000.0).abs() < 1e-6);
}

#[test]
fn an_elastic_plastic_solve_shows_the_plastic_strain_field_and_the_summary() {
    let mut s = FeaWorkbenchState::default();
    s.problem = fea_problem::templates::templates().into_iter().nth(0).unwrap().1;
    s.problem.material.plastic = true;
    s.problem.material.yield_stress = Some(15_000.0); // the hole edge carries about 2.7 x the 10000 traction
    s.problem.material.hardening = 2.0e5;
    s.problem.mesh.size = 0.8;
    s.problem.mesh.adapt_passes = 0;
    let e = handle_key(&mut s, key(KeyCode::Char('r'))).1;
    run_effects(&mut s, e);
    let solved = s.solved.as_ref().expect("solved");
    assert!(solved.summary.plastic.as_ref().is_some_and(|q| q.max_strain > 0.0), "{:?}", solved.summary.plastic);
    assert!(solved.fields().contains(&fea_problem::Field::PlasticStrain));
    let text: String = super::view::readout_lines(&crate::theme::Theme::default(), &s).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("Plastic strain"), "{text}");
}
