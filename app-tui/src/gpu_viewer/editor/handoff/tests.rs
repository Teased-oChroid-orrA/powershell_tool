use super::*;
use crate::gpu_viewer::editor::bushing::file::BushingFile;

fn edited(mut p: Problem, f: impl FnOnce(&mut Problem)) -> Problem {
    f(&mut p);
    p
}
fn hole(p: &mut Problem) -> &mut Shape {
    let Geometry::Sketch { holes, .. } = &mut p.geometry else { panic!("sketch") };
    &mut holes[0]
}
fn outer(p: &mut Problem) -> &mut Shape {
    let Geometry::Sketch { outer, .. } = &mut p.geometry else { panic!("sketch") };
    outer
}

#[test]
fn files_are_named_beside_each_other() {
    assert_eq!(fea_path(Path::new("/w/wing.json")), Path::new("/w/wing.fea.json"));
    assert_eq!(default_case(Path::new("/w/wing.fea.json"), EditorKind::Bushing), Path::new("/w/wing.bushing.json"));
    assert_eq!(default_case(Path::new("/w/solid-model.json"), EditorKind::Eccentric), Path::new("/w/solid-model.eccentric.json"));
}

#[test]
fn the_link_names_the_cases_relative_to_the_fea_file_and_falls_back_to_the_default_names() {
    let fea = Path::new("/w/wing.fea.json");
    let mut link = Link::default();
    assert_eq!(link.case(fea, EditorKind::Bushing), Path::new("/w/wing.bushing.json"));
    link.set(fea, EditorKind::Bushing, Path::new("/w/cases/a.json"));
    link.set(fea, EditorKind::Eccentric, Path::new("/w/ecc.json"));
    assert_eq!(link.bushing.as_deref(), Some("cases/a.json"));
    assert_eq!(link.case(fea, EditorKind::Bushing), Path::new("/w/cases/a.json"));
    assert_eq!(link.case(fea, EditorKind::Eccentric), Path::new("/w/ecc.json"));
    let d = std::env::temp_dir().join(format!("handoff-link-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("x.fea.json");
    assert!(!Link::exists(&f) && Link::load(&f) == Link::default());
    link.save(&f).unwrap();
    assert!(Link::exists(&f));
    assert_eq!(Link::load(&f), link);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_heartbeat_says_a_window_has_the_file_until_it_is_cleared() {
    let d = std::env::temp_dir().join(format!("handoff-beat-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("case.json");
    assert!(!is_open(&f));
    beat(&f);
    assert!(is_open(&f));
    clear_beat(&f);
    assert!(!is_open(&f));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_bushing_case_that_goes_to_the_fea_workbench_and_straight_back_changes_nothing() {
    let mut m = BushingModel::default();
    m.commit_number(NumberTarget::BoreTolPlus, 0.0005);
    m.commit_number(NumberTarget::EdgeDist, 1.1);
    let before = BushingFile::capture(&m, "c");
    let p = bushing_fe::housing_problem(&m).unwrap();
    assert!(bushing_from_problem(&mut m, &p).unwrap().is_empty());
    assert_eq!(BushingFile::capture(&m, "c"), before);
}

#[test]
fn what_is_edited_in_the_fea_workbench_reaches_the_case_and_what_the_fea_model_cannot_carry_stays() {
    let mut m = BushingModel::default();
    m.commit_number(NumberTarget::BoreTolPlus, 0.0005);
    m.commit_number(NumberTarget::MinWallStraight, 0.04);
    let p = bushing_fe::housing_problem(&m).unwrap();
    let new = edited(p, |p| {
        if let Shape::Circle { r, cx, .. } = hole(p) {
            *r = 0.28125;
            *cx += 0.2;
        }
        p.bushings[0].inner_diameter = 0.4;
        p.bushings[0].friction = 0.2;
        p.thickness = 0.75;
        p.loads = vec![Load::Bearing { hole: 1, fx: -2000.0, fy: 0.0 }];
    });
    let did = bushing_from_problem(&mut m, &new).unwrap();
    assert_eq!((m.bore_dia, m.id_bushing, m.friction, m.housing_len, m.load), (0.5625, 0.4, 0.2, 0.75, 2000.0));
    assert!((m.edge_dist - 0.95).abs() < 1e-12, "the hole moved 0.2 in from the edge: {}", m.edge_dist);
    assert_eq!((m.bore_tol_plus, m.min_wall_straight), (0.0005, 0.04), "tolerances and the minimum wall are not in the FEA model");
    assert!(did.iter().any(|d| d.starts_with("Bore Diameter")) && did.iter().any(|d| d.starts_with("Edge Distance")), "{did:?}");
    assert!(m.output.pressure.is_finite());
}

#[test]
fn a_material_changed_by_name_follows_and_one_the_catalog_lacks_is_reported_not_guessed() {
    let mut m = BushingModel::default();
    let p = bushing_fe::housing_problem(&m).unwrap();
    let steel = m.material_catalog().iter().find(|c| c.name.contains("4340") || c.name.to_lowercase().contains("steel")).map(|c| c.name).expect("a steel in the catalog");
    let was = m.housing_material().name;
    let did = bushing_from_problem(&mut m, &edited(p.clone(), |p| p.material.name = steel.to_string())).unwrap();
    assert_eq!(m.housing_material().name, steel);
    assert!(did.iter().any(|d| d.starts_with("housing material")));
    let base = bushing_fe::housing_problem(&m).unwrap();
    let did = bushing_from_problem(&mut m, &edited(base, |p| p.bushings[0].material.name = "Unobtainium".into())).unwrap();
    assert_ne!(m.bushing_material().name, "Unobtainium");
    assert!(did.iter().any(|d| d.contains("not in the catalog")), "{did:?}");
    let _ = was;
}

#[test]
fn a_model_that_is_not_a_bushing_in_a_housing_is_refused_with_the_reason() {
    let mut m = BushingModel::default();
    let plain = fea_problem::templates::templates().remove(0).1;
    assert!(bushing_from_problem(&mut m, &plain).unwrap_err().contains("no bushing"));
    let p = bushing_fe::housing_problem(&m).unwrap();
    let round = edited(p, |p| *outer(p) = Shape::Circle { cx: 0.75, cy: 0.75, r: 0.7 });
    assert!(bushing_from_problem(&mut m, &round).unwrap_err().contains("rectangle"));
}

#[test]
fn the_eccentric_model_round_trips_and_takes_the_offset_the_load_direction_and_the_boss_from_the_fea_edit() {
    let mut m = BushingModel::default();
    let mut ui = EccentricUi { offset: 0.03, load_angle: 90.0, ..EccentricUi::default() };
    let p = eccentric_fe::preview_problem(&m, &ui).unwrap();
    let ui0 = ui.clone();
    assert!(eccentric_from_problem(&mut m, &mut ui, &p).unwrap().is_empty());
    assert_eq!(ui, ui0);
    // Offset 0.05 in, the pin load turned to 45 degrees from the offset line, a bigger boss, a heavier load.
    let new = edited(p, |p| {
        p.bushings[0].offset = [0.05, 0.0];
        let r = 2.0 * 0.25 * 3.0 / 2.0;
        *outer(p) = Shape::Circle { cx: 0.0, cy: 0.0, r };
        let a = 45f64.to_radians();
        p.loads = vec![Load::Bearing { hole: 1, fx: 1500.0 * a.cos(), fy: 1500.0 * a.sin() }];
    });
    let did = eccentric_from_problem(&mut m, &mut ui, &new).unwrap();
    assert!((ui.offset - 0.05).abs() < 1e-12 && (ui.load_angle - 45.0).abs() < 1e-9 && (ui.boss_factor - 3.0).abs() < 1e-9, "{ui:?} {did:?}");
    assert!((m.load - 1500.0).abs() < 1e-9, "the load is the bushing case's");
    // The pin data is not in the FEA model and stays.
    assert_eq!((ui.pin_clearance, ui.pin_friction), (ui0.pin_clearance, ui0.pin_friction));
}

#[test]
fn an_edge_limited_eccentric_housing_takes_the_edge_distance_into_the_bushing_case() {
    let mut m = BushingModel::default();
    let mut ui = EccentricUi { edge_limited: true, offset: 0.02, ..EccentricUi::default() };
    let p = eccentric_fe::preview_problem(&m, &ui).unwrap();
    let new = edited(p, |p| {
        if let Shape::Rect { x0, .. } = outer(p) {
            *x0 = -1.2;
        }
    });
    eccentric_from_problem(&mut m, &mut ui, &new).unwrap();
    assert!((m.edge_dist - 1.2).abs() < 1e-12, "{}", m.edge_dist);
    assert!(ui.edge_limited);
}

// ---------------------------------------------------------------------- the whole loop through the windows' sessions

mod flow {
    use super::*;
    use crate::gpu_viewer::editor::{
        authoring::Authoring,
        bushing::{BushingAct, BushingSession},
        eccentric::{EccentricAct, EccentricSession},
        session::EditorAction,
        solid::{SolidAction, SolidSession},
    };
    use std::time::{Duration, Instant};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("handoff-flow-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn launched() -> Vec<(EditorKind, PathBuf)> {
        LAUNCHED.with(|l| std::mem::take(&mut *l.borrow_mut()))
    }
    fn bushing_act(s: &mut BushingSession, a: BushingAct) {
        s.act(EditorAction::Bushing(a));
    }
    /// Another window wrote the case file: make this window's next look at it due.
    fn due(s: &mut BushingSession) {
        s.make_watch_due();
        s.poll();
    }
    fn later() {
        std::thread::sleep(Duration::from_millis(30));
    }

    #[test]
    fn a_bushing_case_opens_in_the_fea_workbench_once_and_an_open_fea_window_is_not_overwritten() {
        let d = dir("open");
        let case = d.join("wing.json");
        let mut s = BushingSession::open(&case).unwrap();
        launched();
        bushing_act(&mut s, BushingAct::OpenInFea);
        assert!(case.exists(), "the case is saved first: what comes back goes into it");
        let fea = fea_path(&case);
        let on_disk = fea_problem::Problem::from_json(&std::fs::read_to_string(&fea).unwrap()).unwrap();
        assert_eq!(on_disk, bushing_fe::housing_problem(s.model()).unwrap());
        assert_eq!(launched(), [(EditorKind::Solid, fea.clone())]);
        assert_eq!(Link::load(&fea).case(&fea, EditorKind::Bushing), case);
        // The FEA window is open: asking again opens nothing and leaves its file alone.
        beat(&fea);
        std::fs::write(&fea, "edited in the FEA window").unwrap();
        bushing_act(&mut s, BushingAct::OpenInFea);
        assert!(launched().is_empty() && s.status().contains("already open"), "{}", s.status());
        assert_eq!(std::fs::read_to_string(&fea).unwrap(), "edited in the FEA window");
        // Closed again: the FEA file that differs is kept before it is replaced.
        clear_beat(&fea);
        bushing_act(&mut s, BushingAct::OpenInFea);
        assert!(fea.with_extension("previous.json").exists());
        assert_eq!(launched().len(), 1);
    }

    #[test]
    fn the_fea_workbench_sends_its_edit_back_and_the_open_bushing_window_takes_it_in() {
        let d = dir("loop");
        let case = d.join("wing.json");
        let mut s = BushingSession::open(&case).unwrap();
        bushing_act(&mut s, BushingAct::OpenInFea);
        let fea = fea_path(&case);
        launched();
        // The FEA workbench: open the file it was given, move the hole, widen the bore, add load, send.
        let mut f = SolidSession::new(crate::gpu_viewer::editor::solid::read_problem(&fea).unwrap(), fea.clone());
        let before = s.model().edge_dist;
        if let Geometry::Sketch { holes, .. } = &mut f.problem.geometry {
            if let Shape::Circle { cx, r, .. } = &mut holes[0] {
                *cx += 0.25;
                *r = 0.28125;
            }
        }
        f.problem.loads = vec![Load::Bearing { hole: 1, fx: -1800.0, fy: 0.0 }];
        assert!(f.send_availability(EditorKind::Bushing).is_ok());
        // The bushing window is open on the case: no second one is started.
        beat(&case);
        f.do_action(SolidAction::SendTo(EditorKind::Bushing));
        assert!(f.status.contains("updates itself"), "{}", f.status);
        assert!(launched().is_empty());
        later();
        due(&mut s);
        assert!((s.model().edge_dist - (before + 0.25)).abs() < 1e-12, "edge distance {}", s.model().edge_dist);
        assert_eq!((s.model().bore_dia, s.model().load), (0.5625, 1800.0));
        assert!(s.status().contains("Updated from the FEA workbench"), "{}", s.status());
        assert!(!s.dirty(), "taking in a file is not an unsaved edit");
        // And it is one undo step.
        bushing_act(&mut s, BushingAct::Undo);
        assert_eq!((s.model().bore_dia, s.model().load), (0.5, 1000.0));
        clear_beat(&case);
    }

    #[test]
    fn with_unsaved_edits_here_a_changed_file_is_reported_and_not_applied() {
        let d = dir("dirty");
        let case = d.join("wing.json");
        let mut s = BushingSession::open(&case).unwrap();
        bushing_act(&mut s, BushingAct::Save);
        s.act(EditorAction::Bushing(BushingAct::Row(crate::toolboxes::bushing::model::FieldRow::ToggleFitType)));
        assert!(s.dirty());
        let mut m = BushingModel::default();
        m.commit_number(NumberTarget::Load, 3333.0);
        later();
        std::fs::write(&case, BushingFile::capture(&m, "x").to_json()).unwrap();
        due(&mut s);
        assert_eq!(s.model().load, 1000.0);
        assert!(s.status().contains("unsaved edits"), "{}", s.status());
        assert!(s.dirty());
    }

    #[test]
    fn sending_to_the_eccentric_workbench_writes_both_cases_and_the_open_window_follows() {
        let d = dir("ecc");
        let fea = d.join("wing.fea.json");
        let m = BushingModel::default();
        let ui = EccentricUi { offset: 0.03, ..EccentricUi::default() };
        let problem = eccentric_fe::preview_problem(&m, &ui).unwrap();
        std::fs::write(&fea, problem.to_json().unwrap()).unwrap();
        let mut f = SolidSession::new(problem, fea.clone());
        f.problem.bushings[0].offset = [0.045, 0.0];
        f.problem.bushings[0].friction = 0.22;
        launched();
        f.do_action(SolidAction::SendTo(EditorKind::Eccentric));
        assert!(f.status.contains("opened its workbench"), "{}", f.status);
        let ecc = d.join("wing.eccentric.json");
        assert_eq!(launched(), [(EditorKind::Eccentric, ecc.clone())]);
        let s = EccentricSession::open(&ecc).unwrap();
        assert!((s.ui().offset - 0.045).abs() < 1e-12);
        assert_eq!(s.bushing().friction, 0.22, "the shared inputs went to the Bushing case the eccentric file names");
        assert!(d.join("wing.bushing.json").exists());
        // Open windows: the eccentric one reloads the Bushing case when it changes on disk.
        let mut s = s;
        let mut again = BushingModel::default();
        again.commit_number(NumberTarget::Load, 2468.0);
        later();
        std::fs::write(d.join("wing.bushing.json"), BushingFile::capture(&again, "").to_json()).unwrap();
        s.make_watch_due();
        s.poll();
        assert_eq!(s.bushing().load, 2468.0);
        assert!(s.status().contains("Bushing case changed"), "{}", s.status());
    }

    #[test]
    fn the_fea_workbench_refuses_a_model_that_is_not_a_bushing_and_says_why() {
        let d = dir("refuse");
        let mut f = SolidSession::new(fea_problem::templates::templates().remove(0).1, d.join("plate.json"));
        let why = f.send_availability(EditorKind::Bushing).unwrap_err();
        assert!(why.contains("no bushing"), "{why}");
        launched();
        f.do_action(SolidAction::SendTo(EditorKind::Bushing));
        assert!(f.status.contains("Cannot send") && launched().is_empty(), "{}", f.status);
        assert!(!d.join("plate.bushing.json").exists());
        let _ = Instant::now();
        let _ = EccentricAct::Undo;
    }
}

#[test]
fn si_handoff_preserves_bushing_dimensions_and_forces_in_its_inch_system() {
    for units in [fea_problem::UnitSystem::MmMpa, fea_problem::UnitSystem::MeterPa] {
        let mut model = BushingModel::default();
        let mut problem = bushing_fe::housing_problem(&model).unwrap();
        if let Geometry::Sketch { holes, .. } = &mut problem.geometry {
            if let Shape::Circle { r, .. } = &mut holes[0] { *r = 0.3125; }
        }
        problem.loads = vec![Load::Bearing { hole: 1, fx: -1500.0, fy: 0.0 }];
        problem.convert_units(units).unwrap();
        bushing_from_problem(&mut model, &problem).unwrap();
        assert!((model.bore_dia-0.625).abs()<1e-12);
        assert!((model.load-1500.0).abs()<1e-9);
    }
}
