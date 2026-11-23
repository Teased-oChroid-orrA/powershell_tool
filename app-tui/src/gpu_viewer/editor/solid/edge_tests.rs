//! The edge check of a selected hole in the FEA Workbench: read from the model, advisory, never changes the problem.
use super::*;
use crate::gpu_viewer::{editor::authoring::{Authoring, Card}, page::{Cell, DockTable}, ui::DockItem};
use fea_problem::Load;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solid-edge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
fn session(name: &str) -> SolidSession {
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.supports.clear();
    p.loads.clear();
    crate::toolboxes::fea_workbench::model::apply_material(&mut p, mechanics_core::materials::get_material("al7075"));
    let mut s = SolidSession::new(p, dir(name).join("model.json"));
    s.advisors = advising::Advisors::in_dir(dir(&format!("{name}-advisor")));
    s
}
fn card(s: &SolidSession) -> Option<Card> {
    s.inspector().into_iter().find(|c| c.id == "edge-check")
}
fn text(c: &Card) -> String {
    c.items.iter().filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
}
fn tables(c: &Card) -> Vec<DockTable> {
    c.items.iter().filter_map(|i| if let DockItem::Table(t) = i { Some((**t).clone()) } else { None }).collect()
}
fn first(r: &crate::gpu_viewer::page::TableRow) -> String {
    if let Cell::Text { text, .. } = &r.cells[0] { text.clone() } else { String::new() }
}
fn wait(s: &mut SolidSession) {
    let t = std::time::Instant::now();
    while s.edge_worker.is_running() {
        s.poll();
        assert!(t.elapsed().as_secs() < 300, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    s.poll();
}
fn with_bearing(s: &mut SolidSession, load: f64) {
    s.problem.loads.retain(|l| !matches!(l, Load::Bearing { .. }));
    s.problem.loads.push(Load::Bearing { hole: 1, fx: -load, fy: 0.0 });
}

#[test]
fn the_card_appears_for_a_selected_hole_and_says_what_is_missing() {
    let mut s = session("missing");
    assert!(card(&s).is_none(), "nothing selected");
    s.selected = Some("hole1".into());
    let c = card(&s).expect("a hole is selected");
    assert!(c.folded && c.tag == "not run");
    assert!(text(&c).contains("give hole 1 a bearing (pin) load first"), "{}", text(&c));
    s.selected = Some("left".into());
    assert!(card(&s).is_none(), "an edge is not a hole");
}

#[test]
fn a_quick_run_reads_the_model_and_is_advisory_with_nothing_governing() {
    let mut s = session("quick");
    s.selected = Some("hole1".into());
    with_bearing(&mut s, 800.0);
    let before = s.problem.clone();
    let c = card(&s).unwrap();
    assert!(text(&c).contains("pin load 800 lbf"), "{}", text(&c));
    s.start_edge_check(false);
    wait(&mut s);
    let c = card(&s).unwrap();
    assert_eq!(c.tag, "current");
    assert!(!c.folded);
    let ts = tables(&c);
    assert_eq!(ts.len(), 2);
    assert!(ts.iter().all(|t| t.rows.iter().all(|r| !r.governing) && t.rows.iter().any(|r| first(r) == "Superposition")));
    let t = text(&c);
    assert!(t.contains("Advisory: this workbench has no legacy edge check"), "{t}");
    assert_eq!(s.problem, before, "the check reads the model and changes nothing in it");
    // A changed load makes the run stale.
    with_bearing(&mut s, 900.0);
    assert_eq!(card(&s).unwrap().tag, "stale");
}

#[test]
fn the_contact_fe_run_is_the_best_estimate() {
    let mut s = session("deep");
    s.selected = Some("hole1".into());
    with_bearing(&mut s, 800.0);
    s.start_edge_check(true);
    wait(&mut s);
    let c = card(&s).unwrap();
    for t in tables(&c) {
        assert_eq!(first(&t.rows[0]), "Contact FE \u{b7} best estimate");
    }
}

#[test]
fn a_bushed_hole_and_other_units_are_refused_with_the_reason() {
    let mut s = session("refuse");
    s.selected = Some("hole1".into());
    with_bearing(&mut s, 800.0);
    s.problem.units = fea_problem::UnitSystem::MmMpa;
    assert!(text(&card(&s).unwrap()).contains("inches, pounds and psi"));
    s.problem.units = fea_problem::UnitSystem::InchPsi;
    s.problem.bushings.push(fea_problem::Bushing { hole: 1, inner_diameter: 0.1, offset: [0.0, 0.0], interference: 0.001, friction: 0.15, material: s.problem.material.clone() });
    assert!(text(&card(&s).unwrap()).contains("has a bushing"));
}

#[test]
fn a_material_without_library_strengths_falls_back_to_the_yield_strength_and_says_so() {
    let mut s = session("fallback");
    s.selected = Some("hole1".into());
    with_bearing(&mut s, 800.0);
    s.problem.material.ftu = None;
    s.problem.material.fsu = None;
    s.problem.material.fbru = None;
    let t = text(&card(&s).unwrap());
    assert!(t.contains("Ftu is not in the material: the yield strength stands in"), "{t}");
}
