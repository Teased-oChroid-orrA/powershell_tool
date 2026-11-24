//! The Plate Edge window: inputs reach the calculations, edits undo, the case file round-trips, a result is fresh only for its inputs.
use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use std::time::{Duration, Instant};

fn session() -> PlateEdgeSession {
    PlateEdgeSession::new(PlateEdgeInputs::default(), "case".into(), std::env::temp_dir().join("plate-edge-never-written.json"))
}

fn wait(s: &mut PlateEdgeSession) {
    wait_up_to(s, 120);
}

/// `wait` with a longer limit (a 3D run with friction takes about two minutes by itself).
fn wait_up_to(s: &mut PlateEdgeSession, secs: u64) {
    let t = Instant::now();
    loop {
        s.poll();
        if s.job.is_none() && !s.quick_due {
            return;
        }
        assert!(t.elapsed() < Duration::from_secs(secs), "the run did not finish");
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
    // Four presses of the fit choice come back to the start.
    for _ in 0..4 {
        s.act(EditorAction::PlateEdge(PlateEdgeAct::Cycle(Choice::Fit)));
    }
    assert_eq!(s.inputs.fit, inputs::FitKind::PressFit);
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

#[test]
fn the_picture_is_a_filled_plate_spanning_the_geometry_and_the_hole() {
    let mut s = session();
    let scene = s.scene().unwrap();
    let ext = |a: usize| scene.positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max) - scene.positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
    eprintln!("tris {} positions {} extents {} {} {} samples range {:?}", scene.triangles.len() / 3, scene.positions.len(), ext(0), ext(1), ext(2), scene.range);
    assert!(scene.triangles.len() >= 3 * 100, "a graded plate has hundreds of faces, got {}", scene.triangles.len() / 3);
    assert!(ext(2) > 0.0, "the plate has thickness");
}

/// 2D plus the 3D solid end to end (about three minutes): the 3D line exists, is below plane strain (the recorded bracket),
/// and its deformed model is what the window draws.
#[test]
#[ignore = "minutes: the 3D solid"]
fn a_three_d_run_adds_the_solid_below_plane_strain_and_draws_it() {
    let mut s = session();
    s.inputs.fit = inputs::FitKind::Clearance;
    s.inputs.fit_value = 0.002;
    s.changed();
    wait(&mut s);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::ThreeD)));
    wait_up_to(&mut s, 600);
    let o = s.outcome.as_ref().unwrap();
    assert!(o.fe_note.is_none(), "{:?}", o.fe_note);
    let get = |label: &str| o.lines.iter().find(|l| l.label == label).unwrap_or_else(|| panic!("no {label} line")).last().collapse;
    let (strain, stress, solid) = (get("2D plane strain"), get("2D plane stress"), get("3D solid"));
    eprintln!("plane strain {strain:.0}, plane stress {stress:.0}, 3D {solid:.0}");
    assert!(stress < solid && solid < strain, "the 2D idealisations bracket the 3D answer");
    // The results step has the load-travel plot with a curve per line, and the lines side by side.
    let fe = s.inspector().into_iter().find(|c| c.id == "plate-edge-fe").unwrap();
    let plot = fe.items.iter().find_map(|i| if let DockItem::Plot(p) = i { Some(p) } else { None }).expect("a load-travel plot");
    assert_eq!(plot.series.len(), 3);
    assert_eq!(plot.series.iter().map(|s| s.color).collect::<std::collections::BTreeSet<_>>().len(), 3, "three colours for three lines");
    let table = fe.items.iter().find_map(|i| if let DockItem::Table(t) = i { Some(t) } else { None }).expect("the lines side by side");
    assert_eq!(table.columns.len(), 4, "a label column and one column per FE line");
    assert_eq!(super::scene::pick_fields(o).map(|l| l.label), Some("3D solid"));
    assert!(s.scene().is_ok() && s.shows_result());
}

/// Exploration of the window's default case (clearance fit) in the bracket, 2D plane strain and 3D, without and with pin-hole friction
/// (`BRACKET_TRACE=1` prints the solver's progress). Minutes: `-- --ignored --nocapture`.
#[test]
#[ignore = "exploration: minutes"]
fn explore_default_case_friction() {
    use lug_solver::bracket::{try_run, Idealisation};
    let mut inputs = PlateEdgeInputs::default();
    inputs.fit = inputs::FitKind::Clearance;
    inputs.fit_value = 0.002;
    let case = inputs.bracket_case().unwrap();
    for (name, ideal) in [("2D strain", Idealisation::Strain), ("2D stress", Idealisation::Stress), ("3D", inputs.solid())] {
        for mu in [0.0, inputs.friction] {
            let t = Instant::now();
            let r = try_run(lug_solver::bracket::Case { pin_mu: mu, ..case }, ideal, None);
            match r {
                Ok(o) => println!("{name} mu {mu}: collapse {:.0} lbf, {} dof, {:.1} s", o.at_limit(), o.dofs, t.elapsed().as_secs_f64()),
                Err(e) => println!("{name} mu {mu}: FAILED after {:.1} s: {e}", t.elapsed().as_secs_f64()),
            }
        }
    }
}

/// A 2D run on the clearance fit (seconds), used by the result-step tests: the scene before the run, and the session after it.
fn after_a_2d_run() -> (PlateEdgeSession, Scene) {
    let mut s = session();
    s.inputs.fit = inputs::FitKind::Clearance;
    s.inputs.fit_value = 0.002;
    s.changed();
    wait(&mut s);
    let before = s.scene().unwrap();
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::TwoD)));
    wait(&mut s);
    assert!(s.shows_result());
    (s, before)
}

/// A scene vertex within `tol` scene units of `at`.
fn has_point(scene: &Scene, at: [f32; 2], tol: f32) -> bool {
    scene.positions.iter().any(|p| (p[0] - at[0]).hypot(p[1] - at[1]) < tol)
}

#[test]
fn the_picture_and_the_result_share_one_framing_on_the_hole_ligament_and_edge() {
    let (mut s, before) = after_a_2d_run();
    let after = s.scene().unwrap();
    let f = scene::Frame::of(&s.inputs);
    // The free edge on the load line and the top of the hole wall sit where the one frame puts them, before and after.
    // (Within a mesh spacing: the hole wall has a node every 10 degrees, so not exactly at its top.)
    for (x, y, tol) in [(-f.e, 0.0, 1e-4), (0.0, 0.5 * f.d, 0.02), (0.5 * f.d, 0.0, 0.02)] {
        let at = f.xy(x, y);
        assert!(has_point(&before, at, tol), "the picture has no point at model ({x}, {y}) = scene {at:?}");
        assert!(has_point(&after, at, tol), "the result has no point at model ({x}, {y}) = scene {at:?}");
    }
    // Same scale: the result's deformation is drawn to scale (the automatic gain made the hole look larger than the picture's),
    // and under an ordinary landscape camera the free edge, the whole hole and the ligament are in view, before and after.
    assert_eq!((before.gain, after.gain), (1.0, 1.0));
    let cam = crate::gpu_viewer::editor::camera::Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.5, pan: [0.0, 0.0] };
    for scene in [&before, &after] {
        for (x, y) in [(-f.e, 0.0), (-f.e, 2.0 * f.d), (0.5 * f.d, 0.0), (2.0 * f.d, 0.0)] {
            let (ndc, _) = cam.project(f.point(x, y));
            assert!(ndc[0].abs() < 0.95 && ndc[1].abs() < 0.95, "model ({x}, {y}) is out of view: {ndc:?} ({})", scene.label);
        }
    }
    // The framed region fits the unit span: the hole, the ligament and the edge are in view.
    for (x, y) in [(-f.e, 0.0), (2.0 * f.d, 2.0 * f.d)] {
        let [a, b] = f.xy(x, y);
        assert!(a.abs() <= 0.5 && b.abs() <= 0.5, "({x}, {y}) maps outside the view: {a}, {b}");
    }
}

#[test]
fn the_picture_draws_the_pin_the_free_edge_and_the_planes_in_front_of_the_plate() {
    let mut s = session();
    let scene = s.scene().unwrap();
    assert!(scene.validate().is_ok());
    let code = |c: f32| scene.samples.iter().filter(|v| v[3] == c).count();
    assert!(code(2.0) >= 3 * 40, "the pin half-disc: {} vertices", code(2.0));
    assert!(code(3.0) >= 3 * 10, "the free edge, the planes and the edge dimension: {} vertices", code(3.0));
    // Annotations sit in front of the plate (smaller scene z) and are the only vertices there.
    let front = scene.positions.iter().filter(|p| p[2] < 0.0).count();
    assert_eq!(front, code(2.0) + code(3.0));
    assert_eq!(scene::KEY_LABELS, ["Plate", "Hole wall", "Pin", "Free edge, planes"]);
}

#[test]
fn the_load_arrow_is_always_there_and_the_dimensions_only_before_a_result() {
    use crate::gpu_viewer::editor::authoring::OverlayKind;
    use crate::gpu_viewer::editor::camera::Camera;
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let size = [800.0, 800.0];
    let (done, _) = after_a_2d_run();
    let mut open = session();
    open.inputs.fit = inputs::FitKind::Clearance;
    open.inputs.fit_value = 0.002;
    let before = Authoring::overlay(&open, &cam, size);
    let after = Authoring::overlay(&done, &cam, size);
    let arrow = |v: &[crate::gpu_viewer::editor::authoring::Overlay]| v.iter().filter(|o| o.kind == OverlayKind::Load).count();
    assert_eq!((arrow(&before), arrow(&after)), (1, 1));
    let named = |v: &[crate::gpu_viewer::editor::authoring::Overlay], start: &str| v.iter().any(|o| o.text.starts_with(start));
    for want in ["e = ", "D ", "shear-out", "free edge"] {
        assert!(named(&before, want) && !named(&after, want), "{want}: before {}, after {}", named(&before, want), named(&after, want));
    }
    // The half-model caption is there before and after a run.
    for v in [&before, &after] {
        assert!(v.iter().any(|o| o.text == HALF_MODEL && o.kind == OverlayKind::Dimension), "caption missing");
    }
    assert!(HALF_MODEL.contains("symmetric about the load line"));
    // The arrow runs from the pin toward the free edge and stays on screen.
    let a = before.iter().find(|o| o.kind == OverlayKind::Load).unwrap();
    let tip = a.tip.unwrap();
    assert!(tip[0] < a.px[0], "the load points toward the free edge, at the left");
    for v in [a.px, tip] {
        assert!(v[0] >= 0.0 && v[0] <= size[0] && v[1] >= 0.0 && v[1] <= size[1], "{v:?} off screen");
    }
}

#[test]
fn the_results_step_names_each_collapse_number_by_its_model_and_shows_plot_and_tables() {
    let (s, _) = after_a_2d_run();
    let t = texts(&s).join("\n");
    assert!(t.contains("Collapse (contact FE, plane strain)") || t.contains("Collapse, edge-check contact FE (plane strain)"), "{t}");
    assert!(t.contains("2D plane strain (FE bracket)") && t.contains("2D plane stress (FE bracket)"), "{t}");
    assert!(!t.contains("ultimate, 2D FE"));
    let cards = s.inspector();
    let fe = cards.iter().find(|c| c.id == "plate-edge-fe").unwrap();
    let plot = fe.items.iter().find_map(|i| if let DockItem::Plot(p) = i { Some(p) } else { None }).expect("a load-travel plot");
    assert_eq!(plot.series.len(), 2);
    let last = |l: &run::FeLine| *l.last().curve.last().unwrap();
    let o = s.outcome.as_ref().unwrap();
    for (series, line) in plot.series.iter().zip(&o.lines) {
        assert!(series.points.len() >= 6, "{} points", series.points.len());
        let end = series.points.last().unwrap();
        assert!((end[1] - line.last().collapse).abs() < 1e-6 * line.last().collapse && (end[0] - last(line).0).abs() < 1e-12, "the curve ends at the collapse load");
    }
    assert!(plot.marks.iter().any(|m| m.text.contains("edge-check")), "the contact FE collapse is marked on the curve");
    let colours: std::collections::BTreeSet<u8> = plot.series.iter().map(|s| s.color).collect();
    assert_eq!(colours.len(), plot.series.len(), "every line has its own colour");
    assert!(plot.marks.iter().filter(|m| m.text.is_empty()).count() == 2 && plot.marks.iter().all(|m| m.text.is_empty() || m.text.contains("edge-check")), "onset dots carry no overlapping labels");
    // The lines side by side: a label column and one column per line, the collapse row and the spread against plane strain.
    let table = fe.items.iter().find_map(|i| if let DockItem::Table(t) = i { Some(t) } else { None }).expect("a 2D | 3D table");
    assert_eq!(table.columns.len(), 3);
    let rows = DockItem::Table(table.clone()).plain_text().unwrap();
    assert!(rows.contains("Collapse (0.1 D)") && rows.contains("vs plane strain") && rows.contains("Dof"), "{rows}");
    // The convergence table exists for one mesh too, and says there is no convergence check.
    let conv = cards.iter().find(|c| c.id == "plate-edge-convergence").expect("a convergence card after any FE run");
    let DockItem::Table(ct) = &conv.items[0] else { panic!("a table") };
    assert_eq!(ct.rows.len(), 2);
    assert!(ct.footer.contains("no convergence check"), "{}", ct.footer);
}
