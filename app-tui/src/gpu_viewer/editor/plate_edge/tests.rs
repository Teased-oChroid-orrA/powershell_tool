//! The Plate Edge window: inputs reach the calculations, edits undo, the case file round-trips, a result is fresh only for its inputs.
use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use std::time::{Duration, Instant};

fn session() -> PlateEdgeSession {
    PlateEdgeSession::new(PlateEdgeInputs::default(), "case".into(), std::env::temp_dir().join("plate-edge-never-written.json"))
}

fn wait(s: &mut PlateEdgeSession) {
    let t = Instant::now();
    loop {
        s.poll();
        if s.job.is_none() && !s.quick_due {
            return;
        }
        assert!(t.elapsed() < Duration::from_secs(120), "the run did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn texts(s: &PlateEdgeSession) -> Vec<String> {
    s.properties().into_iter().filter_map(|i| i.plain_text()).collect()
}

#[test]
fn it_opens_with_a_case_and_computes_the_allowables() {
    let mut s = session();
    wait(&mut s);
    let o = s.outcome.as_ref().expect("the quick allowables ran on open");
    let m = o.modes.as_ref().expect("a valid default case");
    assert!(m.governing.is_some() && m.rows.len() == 9);
    let t = texts(&s).join("\n");
    assert!(t.contains("Allowable loads") || t.contains("Governing"), "{t}");
    assert!(s.status().starts_with("governing"), "{}", s.status());
}

#[test]
fn a_typed_value_reaches_the_calculation_and_undo_returns() {
    let mut s = session();
    wait(&mut s);
    let before = s.inputs.edge;
    let gov = |s: &PlateEdgeSession| s.outcome.as_ref().unwrap().modes.as_ref().unwrap().rows.iter().find(|r| r.id == "shear_out").unwrap().handbook.unwrap();
    let g0 = gov(&s);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Field(Field::Edge)));
    assert!(s.prompting());
    for c in "0.75".chars() {
        s.key(Key::Char(c), Mods::default());
    }
    s.key(Key::Enter, Mods::default());
    assert_eq!(s.inputs.edge, 0.75);
    wait(&mut s);
    assert!(gov(&s) > g0, "more edge distance carries more shear-out load");
    assert!(s.dirty());
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.inputs.edge, before);
    wait(&mut s);
    assert!((gov(&s) - g0).abs() < 1e-9);
}

#[test]
fn a_bad_value_is_refused_with_the_reason_and_changes_nothing() {
    let mut s = session();
    s.commit_field(Field::Edge, "-1");
    assert!(s.status.contains("must be positive") && s.inputs.edge == 0.5, "{}", s.status);
    s.commit_field(Field::Friction, "2");
    assert!(s.status.contains("between 0 and 1"));
    s.commit_field(Field::Thickness, "abc");
    assert!(s.status.contains("not a number"));
    assert!(s.undo.is_empty(), "a refused value is not an undo step");
}

#[test]
fn a_library_material_is_found_by_name_and_the_next_match_follows() {
    let mut s = session();
    s.commit_search(Pick::Plate, "2024");
    let first = s.inputs.plate_material.clone();
    assert!(first.to_lowercase().contains("2024") || s.inputs.plate_library().name.contains("2024") || s.inputs.plate_library().extra.is_some_and(|x| x.alloy.contains("2024")), "{first}");
    s.commit_search(Pick::Plate, "2024");
    assert_ne!(s.inputs.plate_material, first, "the same search again steps to the next match");
    s.commit_search(Pick::Plate, "no such alloy xyz");
    assert!(s.status.contains("No library material"));
}

#[test]
fn the_fit_type_sets_the_hole_and_the_interference_changes_the_allowables() {
    let mut i = PlateEdgeInputs::default();
    assert_eq!(i.fit, inputs::FitKind::PressFit);
    let pin = i.pin_diameter();
    assert!((i.hole_diameter() - (pin - 0.0008)).abs() < 1e-12, "a press fit makes the hole smaller than the pin");
    i.fit = inputs::FitKind::Clearance;
    i.fit_value = 0.002;
    assert!((i.hole_diameter() - (pin + 0.002)).abs() < 1e-12, "a clearance makes it larger");
    assert!(i.bracket_refusal().is_none() && i.bracket_case().is_ok());
    i.cycle_fit();
    assert_eq!(i.fit, inputs::FitKind::PressFit);
    assert!(i.bracket_refusal().is_some() && i.bracket_case().is_err(), "the FE bracket refuses an interference fit and says why");
    i.fit = inputs::FitKind::Custom;
    i.fit_value = 5000.0;
    assert_eq!(i.delta_and_pressure(), (0.0, Some(5000.0)));
}

#[test]
fn a_library_pin_class_gives_ratings_only_and_a_pin_material_gives_yield_and_bearing() {
    let mut i = PlateEdgeInputs::default();
    let s = i.pin_spec();
    assert!(s.fty.is_none() && s.fbru.is_none() && s.fsu == 95_000.0);
    i.pin_source = inputs::PinSource::Material;
    let s = i.pin_spec();
    assert!(s.fty.is_some() && s.fbru.is_some(), "a library material has both");
    i.pin_source = inputs::PinSource::Custom;
    i.pin_dia = 0.3;
    assert_eq!(i.pin_diameter(), 0.3);
}

#[test]
fn the_case_file_round_trips_every_input_and_refuses_a_newer_format() {
    let mut i = PlateEdgeInputs::default();
    i.plate_source = inputs::Source::Custom;
    i.plate_custom.fbry_ksi = 99.0;
    i.fit = inputs::FitKind::Interference;
    i.fit_value = 0.0021;
    i.mesh = inputs::MeshMode::Converge;
    i.planes = 1;
    let text = i.to_json("my case");
    let (back, name) = PlateEdgeInputs::from_json(&text).unwrap();
    assert_eq!((back, name.as_str()), (i, "my case"));
    assert!(PlateEdgeInputs::from_json(&text.replace("\"version\": 1", "\"version\": 99")).unwrap_err().contains("newer"));
    assert!(PlateEdgeInputs::from_json("[1]").is_err());
}

#[test]
fn a_saved_case_reopens_with_its_inputs_and_the_report_is_written_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plate-edge-case.json");
    let mut s = PlateEdgeSession::open(&path).unwrap();
    wait(&mut s);
    s.commit_field(Field::Thickness, "0.2");
    wait(&mut s);
    s.key(Key::Char('s'), Mods { command: true, ..Mods::default() });
    assert!(path.exists() && !s.dirty());
    let again = PlateEdgeSession::open(&path).unwrap();
    assert_eq!(again.inputs.thickness, 0.2);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Export));
    let report = std::fs::read_to_string(path.with_extension("report.txt")).unwrap();
    assert!(report.contains("Allowable loads") && report.contains("Governing") && report.contains("legacy handbook check"), "{report}");
}

#[test]
fn a_two_d_run_adds_the_fe_and_goes_stale_when_an_input_changes() {
    let mut s = session();
    wait(&mut s);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit)));
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit)));
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit)));
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit)));
    assert_eq!(s.inputs.fit, inputs::FitKind::PressFit, "four cycles return to the start");
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit)));
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit))); // Interference, then Custom
    s.inputs.fit = inputs::FitKind::Clearance;
    s.inputs.fit_value = 0.002;
    s.changed();
    wait(&mut s);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::TwoD)));
    assert!(s.solving());
    wait(&mut s);
    let o = s.outcome.as_ref().unwrap();
    assert_eq!(o.kind, run::Kind::TwoD);
    assert_eq!(o.lines.len(), 2, "plane strain and plane stress: {:?}", o.fe_note);
    let strain = &o.lines[0];
    assert!(strain.last().collapse > strain.last().onset.unwrap_or(0.0) * 0.99);
    assert!(o.lines[1].last().collapse < strain.last().collapse, "plane stress is below plane strain");
    assert!(s.shows_result(), "the solved model is the picture");
    assert!(s.scene().is_ok());
    // An edit makes the FE result stale: it is no longer drawn.
    s.commit_field(Field::Edge, "0.6");
    assert!(!s.current() && !s.shows_result());
}

#[test]
fn an_interference_case_runs_the_allowables_and_says_why_the_bracket_did_not() {
    let mut s = session();
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::TwoD)));
    wait(&mut s);
    let o = s.outcome.as_ref().unwrap();
    assert!(o.lines.is_empty() && o.fe_note.as_deref().is_some_and(|n| n.contains("interference")), "{:?}", o.fe_note);
    let m = o.modes.as_ref().unwrap();
    assert!(m.rows.iter().any(|r| r.id == "collapse" && r.contact.is_some()), "the contact FE ran for the pin with interference");
}

#[test]
fn escape_cancels_a_running_fe_run() {
    let mut s = session();
    s.inputs.fit = inputs::FitKind::Clearance;
    s.inputs.fit_value = 0.002;
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::ThreeD)));
    assert!(s.solving());
    s.key(Key::Escape, Mods::default());
    wait(&mut s);
    let o = s.outcome.as_ref().unwrap();
    assert!(o.lines.len() < 3, "the 3D line did not finish after a cancel");
}

#[test]
fn ribbon_tree_menus_and_cards_exist() {
    let mut s = session();
    wait(&mut s);
    assert_eq!(s.ribbon().len(), 2);
    assert!(s.tree().len() > 5);
    assert_eq!(s.menus().iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "Edit", "Run"]);
    let ids: Vec<String> = s.inspector().into_iter().map(|c| c.id).collect();
    for want in ["plate-edge-plate", "plate-edge-pin", "plate-edge-fit", "plate-edge-geometry", "plate-edge-mesh", "plate-edge-run", "plate-edge-allowables"] {
        assert!(ids.iter().any(|i| i == want), "{want} in {ids:?}");
    }
    assert!(s.scene().is_ok(), "the plate is drawn before any run");
}
