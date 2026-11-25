//! The Plate Edge window: inputs reach the calculations, edits undo, the case file round-trips, a result is fresh only for its inputs.
use super::*;
use crate::gpu_viewer::editor::{authoring::Authoring, session::Tone};
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
    assert!(t.contains("Shear-out") && t.contains("Bearing ultimate"), "{t}");
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
    // An interference fit is modelled: the pin is an elastic body (a ring of the pin's modulus around the rigid core of the
    // contact FE) pressed into the hole by the diametral interference, with the window's friction.
    assert!(i.bracket_refusal().is_none());
    let case = i.bracket_case().expect("the FE bracket models an interference fit");
    let b = case.bush.expect("the pin is installed by interference contact");
    let spec = i.pin_spec();
    assert!((b.interference_dia - 0.0008).abs() < 1e-12 && (b.e - spec.e).abs() < 1.0 && (b.nu - spec.nu).abs() < 1e-12 && (b.mu - i.friction).abs() < 1e-12);
    assert!((b.inner_dia - edge_check::estimate::PIN_CORE * i.hole_diameter()).abs() < 1e-12 && (case.dia - i.hole_diameter()).abs() < 1e-12);
    i.fit = inputs::FitKind::Custom;
    i.fit_value = 5000.0;
    assert_eq!(i.delta_and_pressure(), (0.0, Some(5000.0)));
    assert!(i.bracket_refusal().is_some() && i.bracket_case().is_err(), "a custom pressure has no interference to impose, and the refusal says so");
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
    // Choosing a fit sets its typical value and is one undo step; choosing the one already chosen is not an edit.
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Set(Setting::Fit(inputs::FitKind::PressFit))));
    assert!(s.undo.is_empty());
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Set(Setting::Fit(inputs::FitKind::Clearance))));
    assert_eq!((s.inputs.fit, s.inputs.fit_value, s.undo.len()), (inputs::FitKind::Clearance, 0.002, 1));
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

/// The default press fit runs in the 2D bracket as an elastic pin pressed into the hole (it used to be refused): both
/// idealisations return, the fit pressure is the FE's own, and a pin that is only line to line is within a few percent of it.
#[test]
fn an_interference_case_runs_in_the_bracket_as_an_elastic_pin() {
    let mut s = session();
    assert_eq!(s.inputs.fit, inputs::FitKind::PressFit);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::TwoD)));
    wait(&mut s);
    let o = s.outcome.as_ref().unwrap();
    assert!(o.fe_note.is_none(), "{:?}", o.fe_note);
    assert_eq!(o.lines.len(), 2, "plane strain and plane stress");
    let m = o.modes.as_ref().unwrap();
    assert!(m.rows.iter().any(|r| r.id == "collapse" && r.contact.is_some()), "the contact FE ran for the pin with interference");
    // The bracket's fit pressure is the Lame pressure of a solid pin to within the ring idealisation and the housing's yield.
    let fit = o.lines[0].last().collapse;
    assert!(fit > 0.0);
}

#[test]
fn a_custom_pressure_is_not_modelled_by_the_bracket_and_the_run_says_why() {
    let mut s = session();
    s.inputs.fit = inputs::FitKind::Custom;
    s.inputs.fit_value = 5000.0;
    s.changed();
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Run(run::Kind::TwoD)));
    wait(&mut s);
    let o = s.outcome.as_ref().unwrap();
    assert!(o.lines.is_empty() && o.fe_note.as_deref().is_some_and(|n| n.contains("contact pressure")), "{:?}", o.fe_note);
    assert!(o.modes.as_ref().unwrap().rows.iter().any(|r| r.id == "collapse" && r.contact.is_some()));
}

/// The planes are drawn where they were derived: the elastic scan's best plane from the allowables alone, the FE's ridge once a
/// result is shown (when the solve has one), never the old tangents once anything is derived.
#[test]
fn the_planes_come_from_the_elastic_field_then_from_the_fe_ridge() {
    use scene::PlaneSource;
    let (s, _) = after_a_2d_run();
    let o = s.outcome.as_ref().unwrap();
    let m = o.modes.as_ref().unwrap();
    let best = m.shear_scan.as_ref().expect("an elastic scan").best;
    let r = 0.5 * s.inputs.hole_diameter();
    let elastic = scene::planes(&s.inputs, Some(o), Dim::Two, false);
    assert_eq!(elastic.source, PlaneSource::Elastic);
    let phi = best.start_deg.to_radians();
    assert!((elastic.wall[0] + r * phi.cos()).abs() < 1e-12 && (elastic.wall[1] - r * phi.sin()).abs() < 1e-12);
    assert!((elastic.edge[0] + s.inputs.edge).abs() < 1e-12 && (elastic.edge[1] - best.edge_y).abs() < 1e-12);
    // The wall point is on the hole wall; the planes are not the tangents (whose wall point is at 90 degrees).
    assert!((elastic.wall[0].hypot(elastic.wall[1]) - r).abs() < 1e-9 && best.start_deg < 89.0);
    let shown = scene::planes(&s.inputs, Some(o), Dim::Two, true);
    match &o.lines[0].last().shear {
        Ok(p) => {
            assert_eq!(shown.source, PlaneSource::Fe("2D plane strain"));
            assert_eq!((shown.wall, shown.edge), (p.wall, p.edge_pt));
            assert!((shown.wall[0].hypot(shown.wall[1]) - r).abs() < 1e-3 * r, "the ridge ends on the wall");
        }
        Err(_) => assert_eq!(shown.source, PlaneSource::Elastic),
    }
    assert_eq!(scene::planes(&s.inputs, None, Dim::Two, false).source, PlaneSource::Tangent);
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
fn the_window_is_a_boxed_workbench_with_inputs_results_and_menus() {
    let mut s = session();
    wait(&mut s);
    assert!(s.has_workbench() && s.tree().is_empty() && s.inspector().is_empty() && s.ribbon().is_empty());
    assert_eq!(s.menus().iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["File", "Edit", "Run"]);
    let wb = s.workbench().unwrap();
    assert_eq!(wb.look, crate::gpu_viewer::ui::Look::Boxed);
    let ids = |cards: &[crate::gpu_viewer::editor::authoring::Card]| cards.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&wb.inputs), ["plate-edge-plate", "plate-edge-pin", "plate-edge-fit", "plate-edge-geometry", "plate-edge-mesh"]);
    assert_eq!(ids(&wb.results), ["plate-edge-allowables", "plate-edge-check", "plate-edge-notes"]);
    // Each switched card opens with its segmented control; the chosen segment is the input's value.
    for c in wb.inputs.iter().filter(|c| c.id != "plate-edge-geometry") {
        let DockItem::Segmented(seg) = &c.items[0] else { panic!("{} starts with a switch", c.id) };
        assert_eq!(seg.iter().filter(|e| e.checked == Some(true)).count(), 1, "{}", c.id);
    }
    assert_eq!(wb.actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["Save case", "Report", "Run 2D", "Run 2D + 3D"]);
    assert!(wb.chips.iter().any(|(t, tone)| t.starts_with("Governing: ") && *tone == Tone::On), "{:?}", wb.chips);
    // Before a run: the plate, no plot, the 3D view and the strain field not offered, a hint line in the status.
    assert!(wb.under.is_empty() && wb.view_status.as_ref().is_some_and(|(t, _)| t.starts_with("Allowables only")));
    assert!(wb.views.iter().any(|v| v.label == "3D solid" && !v.enabled) && wb.fields.iter().any(|f| f.label == "Plastic strain" && !f.enabled));
    assert!(s.scene().is_ok(), "the plate is drawn before any run");
}

#[test]
fn the_allowables_card_compares_the_modes_by_method_and_marks_the_governing_one() {
    let mut s = session();
    wait(&mut s);
    let cards = s.result_cards();
    let DockItem::Compare(c) = &cards[0].items[0] else { panic!("a comparison") };
    assert_eq!(c.heads, ["Mode", "Allowable", "2D FE", "3D FE"]);
    assert_eq!(c.rows.len(), 9);
    let gov: Vec<_> = c.rows.iter().filter(|r| r.governing).collect();
    assert_eq!(gov.len(), 1);
    assert!(gov[0].bar.is_some_and(|b| b > 0.0 && b <= 1.0) && cards[0].tag == format!("governing: {}", gov[0].title) && cards[0].tag_tone == Tone::On);
    // No FE has run: no FE figure. The informational rows are dimmed and never govern.
    assert!(c.rows.iter().all(|r| r.values[1] == "-" && r.values[2] == "-"));
    assert!(c.rows.iter().filter(|r| r.title == "First yield" || r.title.starts_with("Collapse")).all(|r| r.dim && !r.governing));
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
    // The plot under the view has a curve per line; the FE card has the lines side by side.
    let wb = s.workbench().unwrap();
    let plot = wb.under[0].items.iter().find_map(|i| if let DockItem::Plot(p) = i { Some(p) } else { None }).expect("a load-travel plot");
    let fe = s.result_cards().into_iter().find(|c| c.id == "plate-edge-fe").unwrap();
    assert_eq!(plot.series.len(), 3);
    assert_eq!(plot.series.iter().map(|s| s.color).collect::<std::collections::BTreeSet<_>>().len(), 3, "three colours for three lines");
    let table = fe.items.iter().find_map(|i| if let DockItem::Table(t) = i { Some(t) } else { None }).expect("the lines side by side");
    assert_eq!(table.columns.len(), 4, "a label column and one column per FE line");
    assert_eq!(super::scene::pick_fields(o, Dim::Three).map(|l| l.label), Some("3D solid"));
    assert_eq!(super::scene::pick_fields(o, Dim::Two).map(|l| l.label), Some("2D plane strain"));
    // The finished run shows the solid; the allowables card carries its collapse and its bearing yield next to the 2D figures.
    assert!(s.dim == Dim::Three && s.shows_result() && s.scene().is_ok());
    let DockItem::Compare(c) = &s.result_cards()[0].items[0] else { panic!("a comparison") };
    let collapse = c.rows.iter().find(|r| r.title.starts_with("Collapse")).unwrap();
    assert!(collapse.values[1] != "-" && collapse.values[2] != "-", "{:?}", collapse.values);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::View(Dim::Two)));
    assert!(s.scene().unwrap().label.contains("2D plane strain"));
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
fn the_load_arrow_is_always_there_and_the_dimensions_before_and_after_a_result() {
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
    // The plate's dimensions stay with the contour (the mockup shows e/D, the free edge and the planes on it); the bore's own
    // D label is only before a run, where the hole is undeformed.
    for want in ["e = ", "shear-out", "free edge"] {
        assert!(named(&before, want) && named(&after, want), "{want}: before {}, after {}", named(&before, want), named(&after, want));
    }
    assert!(named(&before, "D ") && !named(&after, "D "));
    // One caption line, before and after a run; the result's names the pin travel too.
    for (v, want) in [(&before, HALF_MODEL), (&after, PIN_TRAVEL)] {
        let captions: Vec<_> = v.iter().filter(|o| o.text.starts_with("half model")).collect();
        assert!(captions.len() == 1 && captions[0].text == want && captions[0].kind == OverlayKind::Dimension, "one caption {want:?}");
    }
    assert!(HALF_MODEL.contains("mirrored about the load line") && PIN_TRAVEL.starts_with("half model, mirrored"));
    // The arrow runs from the pin toward the free edge and stays on screen.
    let a = before.iter().find(|o| o.kind == OverlayKind::Load).unwrap();
    let tip = a.tip.unwrap();
    assert!(tip[0] < a.px[0], "the load points toward the free edge, at the left");
    for v in [a.px, tip] {
        assert!(v[0] >= 0.0 && v[0] <= size[0] && v[1] >= 0.0 && v[1] <= size[1], "{v:?} off screen");
    }
}

#[test]
fn the_results_name_each_collapse_number_by_its_model_and_show_plot_and_tables() {
    let (s, _) = after_a_2d_run();
    let t = texts(&s).join("\n");
    assert!(t.contains("Collapse (contact FE, plane strain)") || t.contains("Collapse, edge-check contact FE (plane strain)"), "{t}");
    assert!(t.contains("2D plane strain (FE bracket)") && t.contains("2D plane stress (FE bracket)"), "{t}");
    assert!(!t.contains("ultimate, 2D FE"));
    let wb = s.workbench().unwrap();
    assert_eq!(wb.under.len(), 1, "the plot sits under the view");
    assert_eq!(wb.under[0].title, "Load vs. pin travel");
    let plot = wb.under[0].items.iter().find_map(|i| if let DockItem::Plot(p) = i { Some(p) } else { None }).expect("a load-travel plot");
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
    let cards = s.result_cards();
    let fe = cards.iter().find(|c| c.id == "plate-edge-fe").unwrap();
    let table = fe.items.iter().find_map(|i| if let DockItem::Table(t) = i { Some(t) } else { None }).expect("a 2D | 3D table");
    assert_eq!(table.columns.len(), 3);
    let rows = DockItem::Table(table.clone()).plain_text().unwrap();
    assert!(rows.contains("Collapse (0.1 D)") && rows.contains("vs plane strain") && rows.contains("Dof"), "{rows}");
    // The allowables table carries the FE: plane strain in the 2D column, nothing in the 3D column (not run), and only for
    // the two rows the FE bracket reaches.
    let DockItem::Compare(c) = &cards[0].items[0] else { panic!("a comparison") };
    let strain = o.lines[0].last();
    for r in &c.rows {
        let want = match r.title.as_str() {
            t if t.starts_with("Collapse") => super::cards::grp(strain.collapse),
            "Bearing onset (yield)" => strain.onset.map_or_else(|| "-".into(), super::cards::grp),
            _ => "-".into(),
        };
        assert_eq!((r.values[1].as_str(), r.values[2].as_str()), (want.as_str(), "-"), "{}", r.title);
    }
    // The handbook check card: the legacy bearing figure and no invented margin.
    let DockItem::Compare(check) = &cards[1].items[0] else { panic!("a comparison") };
    assert!(check.rows[0].title == "Handbook bearing ultimate" && check.rows[0].detail.contains("legacy Fbru + 0.8 p check stays governing"));
    assert!(check.footer.contains("Advisory"));
    // The convergence card exists for one mesh too, and says there is no convergence check.
    let conv = cards.iter().find(|c| c.id == "plate-edge-convergence").expect("a convergence card after any FE run");
    let DockItem::Compare(cc) = &conv.items[0] else { panic!("a comparison") };
    assert_eq!(cc.rows.len(), 2);
    assert!(cc.footer.contains("no convergence check") && conv.tag == "single mesh", "{}", cc.footer);
    // The status line under the view describes the run; an edit makes it stale.
    assert!(wb.view_status.as_ref().is_some_and(|(t, _)| t.contains("one mesh") && t.contains("dof")), "{:?}", wb.view_status);
}

#[test]
fn the_centre_view_follows_the_run_and_the_switches() {
    let (mut s, _) = after_a_2d_run();
    assert_eq!((s.dim, s.layer), (Dim::Two, Layer::Strain), "a finished run shows its solved model");
    assert!(s.scene().unwrap().label.contains("2D plane strain"));
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Layer(Layer::Geometry)));
    assert!(!s.shows_result() && s.geometry_view() && s.needs_scene());
    let wb = s.workbench().unwrap();
    assert_eq!(wb.fields.iter().map(|f| f.checked).collect::<Vec<_>>(), [Some(true), Some(false)]);
    assert!(wb.model_title.starts_with("Geometry"), "{}", wb.model_title);
    s.act(EditorAction::PlateEdge(PlateEdgeAct::Layer(Layer::Strain)));
    assert!(s.shows_result() && !s.geometry_view());
    assert!(s.workbench().unwrap().model_title.starts_with("2D plane strain"));
    // An edit makes the result stale: the plate is drawn again, the strain field is not offered, the status line says why.
    s.commit_field(Field::Edge, "0.6");
    assert!(!s.shows_result());
    assert!(s.workbench().unwrap().fields.iter().any(|f| f.label == "Plastic strain" && !f.enabled));
}

/// The drawing is the whole plate: the solved half is mirrored about the load line, so the box is symmetric about it and the
/// hole outline closes into a circle of the hole's radius, before and after a run.
#[test]
fn the_drawing_mirrors_the_half_model_into_a_whole_plate_with_a_circular_hole() {
    let (mut s, mut before) = after_a_2d_run();
    let after = s.scene().unwrap();
    let f = scene::Frame::of(&s.inputs);
    let [cx, y0] = f.xy(0.0, 0.0);
    let r = f.xy(0.5 * f.d, 0.0)[0] - cx;
    for (name, scene) in [("picture", &mut before), ("result", &mut s.scene().unwrap())] {
        // The plate's own vertices (annotations sit in front of it, z < 0).
        let plate: Vec<[f32; 3]> = scene.positions.iter().copied().filter(|p| p[2] >= 0.0).collect();
        let (lo, hi) = plate.iter().fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p[1]), h.max(p[1])));
        assert!((lo + hi - 2.0 * y0).abs() < 1e-5, "{name}: the box [{lo}, {hi}] is not symmetric about the load line {y0}");
        let tris = scene.triangles.chunks_exact(3).filter(|t| t.iter().all(|&i| scene.positions[i as usize][2] >= 0.0)).count();
        assert!(tris > 0 && tris % 2 == 0, "{name}: {tris} plate triangles are not a mirrored pair of halves");
        // The hole wall: nodes at the hole's radius from its centre, all the way round.
        let mut angles: Vec<f32> = plate.iter().filter(|p| ((p[0] - cx).hypot(p[1] - y0) - r).abs() < 1e-4).map(|p| (p[1] - y0).atan2(p[0] - cx)).collect();
        angles.sort_by(f32::total_cmp);
        assert!(angles.len() >= 36, "{name}: only {} hole-wall nodes", angles.len());
        let gap = angles.windows(2).map(|w| w[1] - w[0]).fold(angles[0] + 2.0 * std::f32::consts::PI - angles[angles.len() - 1], f32::max);
        assert!(gap < 0.35, "{name}: the hole outline has a {gap} rad gap, it does not close into a circle");
        assert!(angles.iter().any(|a| *a > 1.4 && *a < 1.7) && angles.iter().any(|a| *a < -1.4 && *a > -1.7), "{name}: the hole is missing its top or bottom");
    }
    assert!(after.validate().is_ok());
}

/// The result shows the pin: a filled disc of the pin's radius at its travelled position (the hole's centre moved 0.1 D toward
/// the free edge), so the deformed hole reads as pin plus open crescent, not as a second hole.
#[test]
fn the_result_draws_the_pin_at_its_travelled_position() {
    use crate::gpu_viewer::editor::authoring::OverlayKind;
    use crate::gpu_viewer::editor::camera::Camera;
    let (mut s, _) = after_a_2d_run();
    let _ = s.scene().unwrap();
    let f = scene::Frame::of(&s.inputs);
    let pin_dia = s.inputs.pin_diameter();
    let (centre, radius) = scene::pin_disc(&f, pin_dia);
    let (r_start, c_start) = f.pin(pin_dia);
    assert!((radius - 0.5 * pin_dia).abs() < 1e-12 && (radius - r_start).abs() < 1e-12, "the radius is the pin's");
    assert!(((c_start - centre) - 0.1 * f.d).abs() < 1e-12, "the pin moved 0.1 D from its start, toward the edge");
    // The drawn disc: centred on the travelled position and as large as the pin, in the window's pixels.
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.5, pan: [0.0, 0.0] };
    let size = [900.0, 600.0];
    let px = |x: f64| {
        let (ndc, _) = cam.project(f.point(x, 0.0));
        Camera::pixel(ndc, size[0], size[1])
    };
    let discs: Vec<_> = Authoring::overlay(&s, &cam, size).into_iter().filter(|o| o.kind == OverlayKind::Disc).collect();
    assert_eq!(discs.len(), 1, "one pin");
    let (d, tip) = (&discs[0], discs[0].tip.unwrap());
    assert!((d.px[0] - px(centre)[0]).abs() < 1e-3 && (d.px[1] - px(centre)[1]).abs() < 1e-3, "centred on the travelled pin: {:?}", d.px);
    let r_px = (tip[0] - d.px[0]).hypot(tip[1] - d.px[1]);
    assert!((r_px - (px(centre + radius)[0] - px(centre)[0]).abs()).abs() < 1e-3, "the radius is the pin's: {r_px}");
    // Against the original hole centre it is offset by the clearance and the travel: (r - rp) + 0.1 D toward the edge.
    let offset = px(0.0)[0] - d.px[0];
    let want = px(0.0)[0] - px(-(0.5 * f.d - radius) - 0.1 * f.d)[0];
    assert!((offset - want).abs() < 1e-3 && offset > 0.0, "offset {offset} px, expected {want}");
    // The picture before a run draws no travelled pin (its pin is in the scene), and the captions follow the state.
    let open = session();
    assert!(Authoring::overlay(&open, &cam, size).iter().all(|o| o.kind != OverlayKind::Disc));
    let texts = |v: &[crate::gpu_viewer::editor::authoring::Overlay]| v.iter().filter(|o| o.kind == OverlayKind::Dimension).map(|o| o.text.clone()).collect::<Vec<_>>();
    assert!(texts(&Authoring::overlay(&s, &cam, size)).iter().any(|t| t == PIN_TRAVEL));
    assert!(!texts(&Authoring::overlay(&open, &cam, size)).iter().any(|t| t == PIN_TRAVEL));
}

/// Every text and circle the window painted in one frame at 1500 x 1000 with the editor's overlays on: `(text, right edge, y)`
/// of each text, `(centre, radius, colour)` of each circle.
#[cfg(feature = "gpu-viewer")]
type Painted = (Vec<(String, f32, f32)>, Vec<(egui::Pos2, f32, egui::Color32)>);
#[cfg(feature = "gpu-viewer")]
fn paint_window(s: &mut PlateEdgeSession) -> Painted {
    use crate::gpu_viewer::{controller::ViewState, editor::camera::Camera, gui::Gui, results::GuiInput};
    use egui::epaint::Shape;
    let scene = s.scene().unwrap();
    let view = ViewState::new(&scene);
    let scenes = [scene];
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1500.0, 1000.0))), ..Default::default() };
    let mut gui = Gui::new();
    gui.ctx().set_theme(egui::Theme::Dark);
    let input = |overlay: &'static [crate::gpu_viewer::editor::authoring::Overlay]| GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&*s as &dyn Authoring), error: None, box_drag: None, overlay };
    for _ in 0..3 {
        gui.run(raw(), &input(&[])).0.textures_delta.clear();
    }
    let vp = gui.viewport_px((1500, 1000));
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.5, aspect: vp.w as f32 / vp.h as f32, pan: [0.0, 0.0] };
    let overlay: &'static [_] = Box::leak(Authoring::overlay(s, &cam, [vp.w as f32, vp.h as f32]).into_boxed_slice());
    let (mut full, _) = gui.run(raw(), &input(overlay));
    full.textures_delta.clear();
    fn walk(shape: &Shape, out: &mut Painted) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            Shape::Text(t) => out.0.push((t.galley.text().to_string(), t.pos.x + t.galley.rect.right(), t.pos.y)),
            Shape::Circle(c) => out.1.push((c.center, c.radius, c.fill)),
            _ => {}
        }
    }
    let mut out = (Vec::new(), Vec::new());
    full.shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    out
}

#[cfg(feature = "gpu-viewer")]
#[test]
fn the_workbench_window_paints_the_pin_disc_and_the_plate_captions_over_the_view() {
    let (mut s, _) = after_a_2d_run();
    assert!(s.shows_result());
    let (texts, circles) = paint_window(&mut s);
    for want in ["free edge", PIN_TRAVEL] {
        assert!(texts.iter().any(|(t, _, _)| t == want), "{want} is not painted: the workbench layout dropped the overlays");
    }
    assert!(texts.iter().any(|(t, _, _)| t.starts_with("e = ")), "the edge distance dimension is missing");
    assert!(texts.iter().any(|(t, _, _)| t.starts_with("shear-out planes (derived from")), "the planes' caption names where they come from");
    // The pin as a filled disc of a real size (the overlay's `Disc`), not only the load arrow's tip.
    assert!(circles.iter().any(|(_, r, _)| *r > 8.0), "no pin disc painted: {circles:?}");
}

#[cfg(feature = "gpu-viewer")]
#[test]
fn the_allowables_heads_sit_over_the_right_edge_of_their_figures() {
    let (mut s, _) = after_a_2d_run();
    let (texts, _) = paint_window(&mut s);
    let is_figure = |t: &str| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.');
    for head in ["ALLOWABLE", "2D FE"] {
        let (_, right, y) = texts.iter().find(|(t, _, _)| t == head).unwrap_or_else(|| panic!("no head {head}"));
        let under: Vec<_> = texts.iter().filter(|(t, r, ty)| is_figure(t) && (r - right).abs() < 0.6 && ty > y).collect();
        assert!(!under.is_empty(), "no figure ends where {head} ends ({right}): {:?}", texts.iter().filter(|(t, _, _)| is_figure(t)).collect::<Vec<_>>());
    }
}

#[test]
fn each_result_label_sits_on_the_geometry_it_names() {
    use crate::gpu_viewer::editor::authoring::OverlayKind;
    use crate::gpu_viewer::editor::camera::Camera;
    let (s, _) = after_a_2d_run();
    let f = scene::Frame::of(&s.inputs);
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.5, pan: [0.0, 0.0] };
    let size = [900.0, 600.0];
    let px = |x: f64, y: f64| {
        let (ndc, _) = cam.project(f.point(x, y));
        Camera::pixel(ndc, size[0], size[1])
    };
    let ov = Authoring::overlay(&s, &cam, size);
    let label = |text: &str| ov.iter().find(|o| o.kind == OverlayKind::Dimension && o.text.starts_with(text)).unwrap_or_else(|| panic!("no label {text}"));
    let tol = 3.0;
    let of_kind = |k: OverlayKind| ov.iter().filter(move |o| o.kind == k).collect::<Vec<_>>();
    // The free edge label is on the edge line.
    assert!((label("free edge").px[0] - px(-f.e, 0.0)[0]).abs() < tol, "free edge label off its line");
    // The e dimension: a line from the edge to the hole centre, a tick at each end, the label centred on the line.
    let y = scene::dimension_y(f.d);
    let lines = of_kind(OverlayKind::Line);
    let main = lines.iter().find(|o| (o.px[1] - o.tip.unwrap()[1]).abs() < 1e-3 && (o.px[0] - o.tip.unwrap()[0]).abs() > 10.0).expect("dimension line");
    assert!((main.px[0] - px(-f.e, y)[0]).abs() < tol && (main.tip.unwrap()[0] - px(0.0, y)[0]).abs() < tol && (main.px[1] - px(0.0, y)[1]).abs() < tol);
    let ticks: Vec<_> = lines.iter().filter(|o| (o.px[0] - o.tip.unwrap()[0]).abs() < 1e-3).collect();
    assert_eq!(ticks.len(), 2, "an end tick each side");
    for (t, x) in ticks.iter().zip([-f.e, 0.0]) {
        let mid = 0.5 * (t.px[1] + t.tip.unwrap()[1]);
        assert!((t.px[0] - px(x, y)[0]).abs() < tol && (mid - px(x, y)[1]).abs() < tol, "tick at x = {x}");
    }
    let e = label("e = ");
    assert!((e.px[0] - 0.5 * (main.px[0] + main.tip.unwrap()[0])).abs() < tol && (e.px[1] - main.px[1]).abs() < tol, "e label off its dimension line");
    // Both shear-out planes (the view is mirrored) are dashed lines from the hole wall to the free edge at the derived geometry,
    // one each side of the load line, and their label names the source and is on one of them.
    let derived = s.planes();
    let planes = of_kind(OverlayKind::Dashes);
    assert_eq!(planes.len(), 2);
    for (p, side) in planes.iter().zip([1.0, -1.0]) {
        let (edge, wall) = (px(derived.edge[0], side * derived.edge[1]), px(derived.wall[0], side * derived.wall[1]));
        assert!((p.px[0] - edge[0]).abs() < tol && (p.px[1] - edge[1]).abs() < tol && (p.tip.unwrap()[0] - wall[0]).abs() < tol && (p.tip.unwrap()[1] - wall[1]).abs() < tol, "plane at side {side}");
    }
    let l = label("shear-out");
    assert!(l.text.contains("derived from"), "{}", l.text);
    assert!(planes.iter().any(|p| {
        let (a, b) = (p.px, p.tip.unwrap());
        let t = (l.px[0] - a[0]) / (b[0] - a[0]);
        (0.0..=1.0).contains(&t) && (l.px[1] - (a[1] + t * (b[1] - a[1]))).abs() < tol
    }), "plane label off its plane");
    // The one caption is inside the view, along its bottom.
    let c = label(PIN_TRAVEL);
    assert!(c.px[0] > 0.0 && c.px[0] < size[0] && c.px[1] > 0.75 * size[1] && c.px[1] < size[1]);
}
