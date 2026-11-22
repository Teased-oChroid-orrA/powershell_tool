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
    assert!(texts(&two.inspector()).contains("No fastener site, contact interface or bolt-sized hole without a mate"));
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

/// The plates with one unmated hole added to the advice (the fixture's two holes are mated, so none is detected in it).
fn plates_with_an_unmated_hole() -> AssemblySession {
    use fea_problem::assembly_advice::Closure;
    let mut s = plates();
    let key = s.loaded().unwrap().mesh.face[0];
    let l = s.loaded.as_mut().unwrap();
    l.advice.push(AssemblyAdvice::Unmated {
        body: 0,
        faces: vec![key],
        diameter: 5.0,
        closure: Closure::Through,
        size: "M5 or M6".into(),
        reason: "Body 1 has a \u{d8}5.000 mm through hole 10.00 mm deep with no coaxial bore on another body. It is a place for a support or a load.".into(),
    });
    l.unmated = 1;
    s
}

#[test]
fn holes_without_a_mate_are_a_third_group_in_the_inspector_the_tree_the_summary_and_the_status() {
    let mut s = plates_with_an_unmated_hole();
    let all = texts(&s.inspector());
    let group_titles: Vec<&str> = ["Fastener sites", "Interfaces", "Holes without a mate"].into_iter().filter(|t| all.contains(t)).collect();
    assert_eq!(group_titles.len(), 3, "{all}");
    let (fs, is, um) = (all.find("Fastener sites").unwrap(), all.find("Interfaces").unwrap(), all.find("Holes without a mate").unwrap());
    assert!(fs < is && is < um, "group order: {all}");
    let hole = s.loaded().unwrap().advice[2].clone();
    assert!(all.contains(hole.label().as_str()) && all.contains(hole.detail().as_str()), "{all}");
    assert!(all.contains("Holes without a mate") && s.status().contains("1 hole without a mate"), "{}", s.status());
    // the tree lists the item and the Detected count, and selecting it highlights only its faces
    let labels: Vec<String> = s.tree().iter().map(|n| n.label.clone()).collect();
    assert!(labels.iter().any(|l| l == "Holes without a mate (1)") && labels.iter().any(|l| *l == hole.label()), "{labels:?}");
    act(&mut s, AssemblyAct::Select(Sel::Item(2)));
    assert_eq!(s.selected_faces(), hole.faces());
    let why = s.inspector().into_iter().find(|c| c.id == "assembly-why").unwrap();
    assert_eq!(why.items.len(), 2, "one line per sentence");
    // the plates alone have none
    assert!(!texts(&plates().inspector()).contains("Holes without a mate"));
    assert!(!plates().status().contains("without a mate"));
}

/// The stacked plates with the upper one slid 15 mm along x: the holes no longer share an axis, so each is a hole without a mate.
#[test]
fn plates_slid_apart_report_both_holes_as_unmated_from_the_step_file() {
    let text = std::fs::read_to_string(data("assembly-stacked-plates.step")).unwrap();
    let shifted = text.replace("#944 = CARTESIAN_POINT('',(0.,0.,10.));", "#944 = CARTESIAN_POINT('',(15.,0.,10.));");
    assert_ne!(shifted, text, "the fixture's placement line changed");
    let file = std::env::temp_dir().join(format!("assembly-slid-plates-{}.step", std::process::id()));
    std::fs::write(&file, shifted).unwrap();
    let mut s = AssemblySession::open(&file).unwrap();
    let _ = std::fs::remove_file(&file);
    let l = s.loaded().expect("loaded");
    assert_eq!((l.sites, l.unmated, l.holes), (0, 2, 2), "{:?}", l.advice);
    let holes: Vec<&AssemblyAdvice> = l.advice.iter().filter(|a| matches!(a, AssemblyAdvice::Unmated { .. })).collect();
    let bodies: Vec<usize> = holes.iter().map(|a| match a { AssemblyAdvice::Unmated { body, .. } => *body, _ => unreachable!() }).collect();
    assert_eq!(bodies, vec![0, 1]);
    assert!(s.status().starts_with("2 bodies, 0 fastener sites, 1 interface, 2 holes without a mate, detected in "), "{}", s.status());
    // selecting one lights only that body's faces
    let first = l.advice.iter().position(|a| matches!(a, AssemblyAdvice::Unmated { .. })).unwrap();
    act(&mut s, AssemblyAct::Select(Sel::Item(first)));
    let faces = s.selected_faces();
    assert!(!faces.is_empty() && faces.iter().all(|f| f.body == 0), "{faces:?}");
    assert!(s.scene().is_ok());
}

#[test]
fn selecting_a_body_lists_each_of_its_faces_once_in_key_order() {
    let mut s = plates();
    act(&mut s, AssemblyAct::Select(Sel::Body(0)));
    let faces = s.selected_faces();
    let mut sorted = faces.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(faces, sorted);
    // every face of the body, not just the neighbours-distinct ones
    let all: std::collections::BTreeSet<FaceKey> = s.loaded().unwrap().mesh.face.iter().filter(|k| k.body == 0).copied().collect();
    assert_eq!(faces.len(), all.len());
}

#[test]
fn tree_node_ids_stay_unique_for_a_large_assembly() {
    let mut s = plates();
    let l = s.loaded.as_mut().unwrap();
    // 300 bodies and 300 advice items: ids `10 + body` and `200 + item` used to run into the fixed nodes and each other
    l.mesh.body_triangles.resize(300, 0);
    let more: Vec<AssemblyAdvice> = std::iter::repeat(l.advice[0].clone()).take(300).collect();
    l.advice.extend(more);
    let ids: Vec<u32> = s.tree().iter().map(|n| n.id).collect();
    let unique: std::collections::BTreeSet<u32> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "duplicate node ids");
}

#[test]
fn a_step_with_no_bodies_is_a_status_not_a_blank_window_or_a_panic() {
    let empty = std::env::temp_dir().join(format!("assembly-no-bodies-{}.step", std::process::id()));
    std::fs::write(&empty, "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('x'),'2;1');\nFILE_NAME('x','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n").unwrap();
    let mut s = AssemblySession::open(&empty).unwrap();
    let _ = std::fs::remove_file(&empty);
    assert!(s.loaded().is_none() && s.status().starts_with("Cannot open "), "{}", s.status());
    assert!(s.scene().is_ok(), "the empty window still draws");
}

/// The plates with one near-closed bore added to the advice (the fixture has none: its bores are full turns).
fn plates_with_a_near_closed_bore() -> AssemblySession {
    let mut s = plates();
    let key = s.loaded().unwrap().mesh.face[0];
    let l = s.loaded.as_mut().unwrap();
    l.advice.push(AssemblyAdvice::NearClosed {
        body: 0,
        faces: vec![key],
        diameter: 3.675,
        covered: 0.9689,
        gap_width: 0.359,
        gap_point: [1.0, 2.0, 3.0],
        reason: "Body 1 has a \u{d8}3.675 mm bore. It is not a hole. The missing 0.359 mm of wall is at (1.00, 2.00, 3.00) mm.".into(),
    });
    l.near_closed = 1;
    s
}

#[test]
fn a_near_closed_bore_is_listed_under_check_with_its_numbers_counted_and_its_faces_lit() {
    let mut s = plates_with_a_near_closed_bore();
    let all = texts(&s.inspector());
    let item = s.loaded().unwrap().advice[2].clone();
    // under the Check group (a warning), after the groups that are not warnings, with the label and the dim numbers
    assert!(all.contains(item.label().as_str()) && all.contains(item.detail().as_str()), "{all}");
    let advice_card = s.inspector().into_iter().find(|c| c.id == "assembly-advice").unwrap();
    let headings: Vec<String> = advice_card.items.iter().filter_map(|i| if let crate::gpu_viewer::ui::DockItem::Heading(t) = i { Some(t.clone()) } else { None }).collect();
    assert_eq!(headings, vec!["Fastener sites", "Interfaces", "Check"], "the item is under Check, the warning group, after the others");
    // the summary card and the status bar count it, and the tree has the Detected row and the item
    assert!(all.contains("Near-closed bores"), "{all}");
    assert!(s.status().contains("1 near-closed bore"), "{}", s.status());
    let labels: Vec<String> = s.tree().iter().map(|n| n.label.clone()).collect();
    assert!(labels.iter().any(|l| l == "Near-closed bores (1)") && labels.iter().any(|l| *l == item.label()), "{labels:?}");
    // selecting it lights only the bore's faces and the Why card says where the gap is
    act(&mut s, AssemblyAct::Select(Sel::Item(2)));
    assert_eq!(s.selected_faces(), item.faces());
    let why = s.inspector().into_iter().find(|c| c.id == "assembly-why").unwrap();
    assert_eq!(why.items.len(), 3, "one line per sentence");
    assert!(why.items.iter().filter_map(|i| i.plain_text()).any(|t| t.contains("(1.00, 2.00, 3.00)")));
    assert!(s.scene().is_ok());
    // the plates alone have none
    assert!(!texts(&plates().inspector()).contains("Near-closed bores"));
    assert!(!plates().status().contains("near-closed"));
}
