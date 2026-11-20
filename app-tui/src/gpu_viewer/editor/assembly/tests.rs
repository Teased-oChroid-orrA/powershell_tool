use super::scene::{explode_offsets, scene, CODE_SELECTED};
use super::*;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../solid-kernel/tests/data").join(name)
}
/// Two 40 x 30 x 10 plates, one on the other, each with a 4 mm hole at the same place: one interface and one fastener site.
fn plates() -> AssemblySession {
    AssemblySession::open(&data("assembly-stacked-plates.step")).unwrap()
}
fn texts(cards: &[Card]) -> String {
    cards.iter().flat_map(|c| std::iter::once(c.title.clone()).chain(c.items.iter().filter_map(|i| i.plain_text()))).collect::<Vec<_>>().join("\n")
}
fn act(s: &mut AssemblySession, a: AssemblyAct) {
    s.act(EditorAction::Assembly(a));
}
#[test]
fn stacked_plates_load_with_one_interface_and_one_fastener_site() {
    let s = plates();
    let l = s.loaded().expect("loaded");
    assert_eq!(l.mesh.bodies(), 2);
    assert_eq!((l.holes, l.sites, l.interfaces, l.unresolved), (2, 1, 1, 0));
    assert_eq!(l.advice.len(), 2, "{:?}", l.advice);
    assert_eq!(l.holes_per_body, vec![1, 1]);
    let status = s.status();
    assert!(status.starts_with("2 bodies, 1 fastener site, 1 interface, detected in "), "{status}");
    assert_eq!(s.title(), "Assembly Joints | assembly-stacked-plates");
}

#[test]
fn the_inspector_lists_every_advice_row_with_its_numbers_and_the_reason_of_the_selection() {
    let mut s = plates();
    let l = s.loaded().unwrap();
    let rows: Vec<(String, String)> = l.advice.iter().map(|a| (a.label(), a.detail())).collect();
    let all = texts(&s.inspector());
    for (label, detail) in &rows {
        assert!(all.contains(label.as_str()) && all.contains(detail.as_str()), "{label} / {detail} in\n{all}");
    }
    assert!(all.contains("Fastener sites") && all.contains("Interfaces") && !all.contains("\nCheck\n"), "{all}");
    assert!(all.contains("Select an advice item"), "{all}");
    // selecting an item fills the Why card with its reason, one sentence per line
    act(&mut s, AssemblyAct::Select(Sel::Item(1)));
    let why = s.inspector().into_iter().find(|c| c.id == "assembly-why").unwrap();
    let reason = s.loaded().unwrap().advice[1].reason().to_string();
    assert!(matches!(s.loaded().unwrap().advice[1], AssemblyAdvice::Fastener { .. }));
    assert!(why.items.len() >= 2, "{reason}");
    let first = why.items[0].plain_text().unwrap();
    assert!(reason.starts_with(first.trim_end_matches('.')), "{first} vs {reason}");
}

#[test]
fn the_tree_has_bodies_detected_and_advice_and_selecting_a_node_highlights_it() {
    let mut s = plates();
    let labels: Vec<String> = s.tree().iter().map(|n| n.label.clone()).collect();
    for want in ["Bodies (2)", "Body 1", "Body 2", "Detected", "Holes (2)", "Fastener sites (1)", "Interfaces (1)", "Advice (2)"] {
        assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
    }
    assert!(s.tree().iter().all(|n| !n.selected));
    act(&mut s, AssemblyAct::Select(Sel::Body(1)));
    let selected: Vec<String> = s.tree().iter().filter(|n| n.selected).map(|n| n.label.clone()).collect();
    assert_eq!(selected, vec!["Body 2".to_string()]);
    // a body's faces are all of that body's
    let faces = s.selected_faces();
    assert!(!faces.is_empty() && faces.iter().all(|f| f.body == 1));
}

#[test]
fn selection_changes_redraw_and_keys_walk_the_advice() {
    let mut s = plates();
    let _ = s.scene().unwrap();
    assert!(!s.needs_scene());
    assert_eq!(s.key(Key::Down, Mods::default()), KeyResult::Handled);
    assert_eq!(s.selection(), Some(Sel::Item(0)));
    assert!(s.needs_scene());
    let _ = s.scene().unwrap();
    s.key(Key::Down, Mods::default());
    assert_eq!(s.selection(), Some(Sel::Item(1)));
    s.key(Key::Down, Mods::default());
    assert_eq!(s.selection(), Some(Sel::Item(0)), "wraps");
    s.key(Key::Up, Mods::default());
    assert_eq!(s.selection(), Some(Sel::Item(1)));
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.selection(), None);
    // selecting what is already selected redraws nothing
    act(&mut s, AssemblyAct::Select(Sel::Body(0)));
    let _ = s.scene().unwrap();
    act(&mut s, AssemblyAct::Select(Sel::Body(0)));
    assert!(!s.needs_scene());
    // E toggles the explosion, case-insensitive (Windows reports Shift/Caps Lock letters in the other case)
    assert!(s.exploded());
    s.key(Key::Char('e'), Mods::default());
    assert!(!s.exploded() && s.needs_scene());
    s.key(Key::Char('E'), Mods::default());
    assert!(s.exploded());
    assert_eq!(s.key(Key::Char('x'), Mods::default()), KeyResult::Unhandled);
}

#[test]
fn the_selected_faces_take_the_selected_colour_and_bodies_alternate() {
    let mut s = plates();
    let codes = |sc: &Scene| sc.samples.iter().map(|v| v[3]).collect::<Vec<f32>>();
    let plain = s.scene().unwrap();
    assert!(codes(&plain).iter().all(|c| *c == 0.0 || *c == 1.0), "nothing selected, bodies 0 / 1");
    assert!(codes(&plain).contains(&0.0) && codes(&plain).contains(&1.0));
    act(&mut s, AssemblyAct::Select(Sel::Item(1)));
    let lit = s.scene().unwrap();
    let n = codes(&lit).iter().filter(|c| **c == CODE_SELECTED).count();
    assert!(n > 0 && n < lit.positions.len(), "{n} of {}", lit.positions.len());
    assert_eq!(lit.range, [0.0, 3.0]);
    // every triangle index is valid (Scene::validate also ran)
    assert!(lit.triangles.iter().all(|i| (*i as usize) < lit.positions.len()));
}

#[test]
fn exploding_separates_the_bodies_and_one_body_does_not_move() {
    let s = plates();
    let mesh = &s.loaded().unwrap().mesh;
    let off = explode_offsets(mesh);
    assert_eq!(off.len(), 2);
    assert!(off[0][2] < 0.0 && off[1][2] > 0.0 && off[0][0].abs() < 1e-9 && off[0][1].abs() < 1e-9, "{off:?}");
    // plates touch at z = 10; exploded, the bodies' z ranges no longer meet
    let z = |sc: &Scene, b: usize| {
        let first: usize = mesh.body_triangles[..b].iter().sum();
        let tris = &sc.triangles[3 * first..3 * (first + mesh.body_triangles[b])];
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for &i in tris {
            lo = lo.min(sc.positions[i as usize][2]);
            hi = hi.max(sc.positions[i as usize][2]);
        }
        (lo, hi)
    };
    let touching = scene(mesh, false, |_| false, "t".into()).unwrap();
    let apart = scene(mesh, true, |_| false, "a".into()).unwrap();
    assert!((z(&touching, 0).1 - z(&touching, 1).0).abs() < 1e-6);
    assert!(z(&apart, 1).0 - z(&apart, 0).1 > 0.05, "{:?} {:?}", z(&apart, 0), z(&apart, 1));
    // a single body is its own centre: no shift
    let one = load(&data("placed-single-part.step")).unwrap();
    assert_eq!(one.mesh.bodies(), 1);
    assert_eq!(explode_offsets(&one.mesh), vec![[0.0; 3]]);
}

#[test]
fn an_assembly_without_joints_says_so_and_the_empty_window_offers_open() {
    let two = AssemblySession::open(&data("assembly-two-instances.step")).unwrap();
    assert_eq!(two.loaded().unwrap().mesh.bodies(), 2);
    assert!(two.loaded().unwrap().advice.is_empty());
    assert!(texts(&two.inspector()).contains("No fastener site or contact interface"));
    assert!(two.start_options().is_empty());

    let mut empty = AssemblySession::open(Path::new("")).unwrap();
    assert!(empty.loaded().is_none());
    assert_eq!(empty.status(), "Open a STEP assembly: File > Open");
    assert_eq!(empty.start_options().len(), 1);
    assert!(empty.needs_scene());
    let sc = empty.scene().unwrap();
    assert!(sc.validate().is_ok());
    assert!(empty.tree().iter().all(|n| !n.selected));
    // nothing to step through
    assert_eq!(empty.key(Key::Down, Mods::default()), KeyResult::Handled);
    assert_eq!(empty.selection(), None);
}

#[test]
fn a_file_that_cannot_be_read_is_a_status_not_an_error() {
    let s = AssemblySession::open(&data("no-such-file.step")).unwrap();
    assert!(s.loaded().is_none());
    assert!(s.status().starts_with("Cannot open "), "{}", s.status());
    // a file that is not STEP at all
    let bad = std::env::temp_dir().join(format!("assembly-not-step-{}.step", std::process::id()));
    std::fs::write(&bad, "this is not a STEP file").unwrap();
    let s = AssemblySession::open(&bad).unwrap();
    let _ = std::fs::remove_file(&bad);
    assert!(s.loaded().is_none() && s.status().starts_with("Cannot open "), "{}", s.status());
}

#[test]
fn the_ribbon_and_menus_toggle_the_explosion_and_clear_the_selection() {
    let mut s = plates();
    let view = |s: &AssemblySession| s.ribbon().remove(0).groups.into_iter().find(|g| g.title == "View").unwrap().buttons;
    let b = view(&s);
    assert_eq!(b[0].label, "Exploded");
    assert!(b[0].checked == Some(true), "on by default");
    assert!(b[1].label == "Clear selection" && !b[1].enabled, "nothing to clear yet");
    act(&mut s, AssemblyAct::Select(Sel::Item(0)));
    assert!(view(&s)[1].enabled);
    act(&mut s, AssemblyAct::ClearSelection);
    act(&mut s, AssemblyAct::ToggleExploded);
    assert_eq!(view(&s)[0].checked, Some(false));
    let titles: Vec<String> = s.menus().into_iter().map(|m| m.title).collect();
    assert_eq!(titles, vec!["File", "View"]);
    assert_eq!(s.context_menu().len(), 2);
    assert_eq!(s.step_state("Assembly"), Some(StepState::Done));
    assert_eq!(s.focus_step(), Some("Assembly"));
    assert!(!s.dirty() && !s.solving() && !s.prompting());
}
