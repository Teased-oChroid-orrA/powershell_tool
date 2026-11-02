use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::toolboxes::material_lookup::model;

fn session() -> MaterialsSession {
    MaterialsSession::with_report(None)
}
fn doc(s: &MaterialsSession) -> Document {
    s.document().unwrap()
}
fn run(s: &mut MaterialsSession, a: MaterialsAct) {
    s.act(EditorAction::Materials(a));
}

#[test]
fn the_page_lists_every_material_with_the_toolbox_values() {
    let s = session();
    let d = doc(&s);
    assert_eq!(d.table.rows.len(), model::catalog().len());
    let first = model::catalog()[s.state().hits[0]];
    assert_eq!(d.table.rows[0].cells[0], first.name);
    assert!(d.table.rows[0].selected);
    assert_eq!(d.table.columns[0].sorted, Some(true), "name A-Z by default");
    let detail = d.detail.expect("the highlighted material's properties");
    assert_eq!(detail.title, first.name);
    assert!(detail.grid.unwrap().rows.iter().any(|r| r.cells[0] == "Fty / density"));
}

#[test]
fn typing_searches_from_the_best_match_and_esc_clears() {
    let mut s = session();
    s.act(EditorAction::Materials(MaterialsAct::Select(5)));
    s.document_field(INPUT_SEARCH, "7075 t6".into());
    let want = model::filter_sort(&model::Query { text: "7075 t6".into(), ..model::Query::default() });
    assert_eq!(s.state().hits, want, "the toolbox's own filter");
    assert_eq!(s.state().cursor, 0);
    assert!(doc(&s).inputs[0].note.as_deref().unwrap().starts_with(&format!("{} of", want.len())));
    assert_eq!(s.key(Key::Escape, Mods::default()), KeyResult::Handled);
    assert!(s.state().query.text.is_empty());
    assert_eq!(s.state().hits.len(), model::catalog().len());
    assert_eq!(s.key(Key::Escape, Mods::default()), KeyResult::Unhandled, "nothing left to clear");
}

#[test]
fn a_column_click_sorts_and_a_second_click_reverses() {
    let mut s = session();
    run(&mut s, MaterialsAct::SortBy(SortKey::Fty));
    let d = doc(&s);
    let fty = d.table.columns.iter().position(|c| c.title == "Fty ksi").unwrap();
    assert_eq!(d.table.columns[fty].sorted, Some(false), "a property starts largest first");
    assert_eq!(d.table.columns[0].sorted, None);
    let top = SortKey::Fty.value(model::catalog()[s.state().hits[0]]).unwrap();
    assert!(s.state().hits.iter().filter_map(|&i| SortKey::Fty.value(model::catalog()[i])).all(|v| v <= top));
    run(&mut s, MaterialsAct::SortBy(SortKey::Fty));
    assert!(!s.state().query.descending);
    assert_eq!(doc(&s).table.columns[fty].sorted, Some(true));
}

#[test]
fn basis_and_group_filter_like_the_terminal() {
    let mut s = session();
    run(&mut s, MaterialsAct::Basis(model::Basis::A));
    assert!(s.state().hits.iter().all(|&i| model::catalog()[i].extra.is_some_and(|x| x.basis == "A")));
    assert_eq!(doc(&s).inputs[0].choices.iter().find(|c| c.checked == Some(true)).unwrap().label, "A-basis");
    run(&mut s, MaterialsAct::Basis(model::Basis::Any));
    let curated = model::groups().iter().position(|g| g == model::CURATED_GROUP).unwrap();
    run(&mut s, MaterialsAct::Group(curated));
    assert!(s.state().hits.iter().all(|&i| model::catalog()[i].extra.is_none()));
    assert!(s.tree().iter().any(|n| n.selected && n.label == model::CURATED_GROUP));
    run(&mut s, MaterialsAct::Group(10_000));
    assert_eq!(s.state().query.group, curated, "an unknown group is ignored");
}

#[test]
fn double_click_marks_and_compare_shows_the_best_of_each_row() {
    let mut s = session();
    let al = model::catalog().iter().position(|m| m.extra.is_none()).unwrap();
    let ti = model::catalog().iter().position(|m| m.name.contains("Ti-6Al-4V")).unwrap();
    run(&mut s, MaterialsAct::ToggleMark(al));
    run(&mut s, MaterialsAct::ToggleMark(ti));
    assert_eq!(s.state().marked, vec![al, ti]);
    let d = doc(&s);
    let row = d.table.rows.iter().find(|r| r.cells[0] == model::catalog()[ti].name).unwrap();
    assert_eq!(row.badge.as_deref(), Some("#2"));
    run(&mut s, MaterialsAct::Compare);
    assert_eq!(s.focus_step(), Some("Compare"));
    let grid = doc(&s).detail.unwrap().grid.unwrap();
    assert_eq!(grid.columns, ["Property", "Unit", "#1", "#2"]);
    let fty = grid.rows.iter().find(|r| r.cells[0] == "Fty").unwrap();
    assert_eq!(fty.best, vec![3], "titanium (#2, cell 3) has the higher yield");
    // A row click shows that material again; marks stay.
    run(&mut s, MaterialsAct::Select(0));
    assert_eq!(s.focus_step(), Some("Browse"));
    assert_eq!(s.state().marked.len(), 2);
    // Unmarking the last one ends the comparison.
    run(&mut s, MaterialsAct::Compare);
    run(&mut s, MaterialsAct::ClearMarks);
    assert!(!s.state().compare && s.state().marked.is_empty());
    assert_eq!(doc(&s).chips[0].enabled, false, "Compare needs a mark");
}

#[test]
fn enter_marks_the_highlighted_row_and_arrows_move() {
    let mut s = session();
    s.key(Key::Down, Mods::default());
    s.key(Key::Down, Mods::default());
    assert_eq!(s.state().cursor, 2);
    s.key(Key::Enter, Mods::default());
    assert_eq!(s.state().marked, vec![s.state().hits[2]]);
    s.key(Key::Up, Mods::default());
    assert_eq!(s.state().cursor, 1);
}

#[test]
fn export_writes_the_toolbox_report() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reports").join("material-lookup.txt");
    let mut s = MaterialsSession::with_report(Some(path.clone()));
    s.document_field(INPUT_SEARCH, "4340".into());
    run(&mut s, MaterialsAct::MarkCurrent);
    assert_eq!(s.key(Key::Char('e'), Mods { command: true, shift: false }), KeyResult::Handled);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, s.state().report_text());
    assert!(text.contains("text \"4340\"") && text.contains("Comparison"));
    assert!(s.status().contains("Report written"));
    let mut none = session();
    run(&mut none, MaterialsAct::Export);
    assert!(none.status().contains("No folder"));
}

#[test]
fn every_material_has_a_property_grid() {
    for m in model::catalog().iter().step_by(23) {
        let g = view::property_grid(m);
        assert!(g.rows.iter().filter(|r| !r.section).count() >= 6, "{}", m.name);
        assert!(g.rows.iter().all(|r| r.section || r.cells.len() == 3), "{}", m.name);
    }
    let curated = model::catalog()[0];
    let g = view::property_grid(curated);
    assert!(g.rows.iter().any(|r| r.cells[0] == "Solver values"));
    assert!(g.note.contains("no density"));
}
