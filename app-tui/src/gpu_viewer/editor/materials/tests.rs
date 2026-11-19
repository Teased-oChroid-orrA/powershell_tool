use super::*;

#[test]
fn cached_material_rows_follow_selection_marks_and_query() {
    let mut s = MaterialsSession::with_report(None);
    let original = view::document(&s).table.rows;
    assert_eq!(view::document(&s).table.rows, original);
    s.st.cursor = 1;
    s.st.toggle_mark();
    let rows = view::document(&s).table.rows;
    assert!(rows[1].selected && rows[1].badge.as_deref() == Some("#1"));
    s.st.query.text = "7075".into();
    s.st.refilter();
    assert!(view::document(&s).table.rows.len() < original.len());
}
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
fn basis_and_group_filter_the_results() {
    let mut s = session();
    run(&mut s, MaterialsAct::Basis(model::Basis::A));
    assert!(s.state().hits.iter().all(|&i| model::catalog()[i].extra.is_some_and(|x| x.basis == "A")));
    assert_eq!(doc(&s).inputs[0].choices.iter().find(|c| c.checked == Some(true)).unwrap().label, "A-basis");
    run(&mut s, MaterialsAct::Basis(model::Basis::Any));
    let curated = model::groups().iter().position(|g| g == model::CURATED_GROUP).unwrap();
    run(&mut s, MaterialsAct::Group(curated));
    assert!(s.state().hits.iter().all(|&i| model::catalog()[i].extra.is_none()));
    let rail = doc(&s).rail.unwrap();
    assert!(rail.sections[2].options.iter().any(|o| o.on && o.label == model::CURATED_GROUP));
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

fn rail_option(s: &MaterialsSession, section: &str, label: &str) -> crate::gpu_viewer::ui::DocNarrowOption {
    let rail = doc(s).rail.unwrap();
    let sec = rail.sections.into_iter().find(|x| x.title == section).unwrap();
    sec.options.into_iter().find(|o| o.label.starts_with(label)).unwrap_or_else(|| panic!("no {label} in {section}"))
}

#[test]
fn the_rail_lists_every_group_form_and_basis_with_counts_that_match_the_choice() {
    let mut s = session();
    let d = doc(&s);
    let rail = d.rail.as_ref().unwrap();
    assert_eq!(rail.sections[2].options.len(), model::groups().len() - 1, "every group but All");
    assert_eq!(rail.sections[1].options.len(), model::FORMS.len());
    assert!(rail.clear.is_none() && rail.shown == rail.total);
    // Choosing an option shows what its count promised, and takes the same click back.
    let plate = rail_option(&s, "FORM", "Plate");
    s.act(EditorAction::Materials(MaterialsAct::Form(model::FORMS.iter().position(|f| *f == "Plate").unwrap())));
    assert_eq!(s.state().hits.len(), plate.count);
    assert!(rail_option(&s, "FORM", "Plate").on);
    assert!(doc(&s).rail.unwrap().clear.is_some());
    assert_eq!(doc(&s).table.chips.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Plate"]);
    let b = rail_option(&s, "BASIS", "A-basis");
    assert_eq!(b.action, act(MaterialsAct::Basis(model::Basis::A)));
    run(&mut s, MaterialsAct::Basis(model::Basis::A));
    assert_eq!(rail_option(&s, "BASIS", "A-basis").action, act(MaterialsAct::Basis(model::Basis::Any)), "again goes back to any");
    run(&mut s, MaterialsAct::ClearFacets);
    assert_eq!(s.state().hits.len(), model::catalog().len());
    assert!(doc(&s).table.chips.is_empty());
}

#[test]
fn a_slider_sets_and_drops_a_property_limit_and_a_chip_takes_it_back() {
    let mut s = session();
    let all = s.state().hits.len();
    s.document_field(INPUT_FLOOR0, "100".into());
    assert_eq!(s.state().query.min_fty, Some(100.0));
    assert!(s.state().hits.len() < all && s.state().hits.iter().all(|&i| model::catalog()[i].sy_ksi >= 100.0));
    let d = doc(&s);
    let slider = &d.rail.as_ref().unwrap().sliders[0];
    assert!(slider.active && slider.value == 100.0 && slider.id == INPUT_FLOOR0);
    let chip = &d.table.chips[0];
    assert_eq!(chip.label, "Fty >= 100 ksi");
    assert_eq!(chip.remove, act(MaterialsAct::ClearFloor(model::Floor::Fty)));
    // Dragging back to the free end sends an empty text; the density slider is free at its largest value.
    s.document_field(INPUT_FLOOR0, String::new());
    assert_eq!(s.state().hits.len(), all);
    assert!(!doc(&s).rail.unwrap().sliders[0].active);
    s.document_field(INPUT_FLOOR0 + 1, "0.1".into());
    assert_eq!(s.state().query.max_density, Some(0.1));
    run(&mut s, MaterialsAct::ClearFloor(model::Floor::Density));
    assert_eq!(s.state().query.max_density, None);
    s.document_field(INPUT_FLOOR0 + 7, "5".into());
    assert_eq!(s.state().hits.len(), all, "an unknown slider id is ignored");
}

#[test]
fn the_basket_shows_the_marks_in_slots_and_a_row_box_toggles_one() {
    let mut s = session();
    let d = doc(&s);
    let b = d.basket.as_ref().unwrap();
    assert_eq!(b.slots.len(), model::MAX_COMPARE);
    assert!(b.slots.iter().all(Option::is_none));
    let first = s.state().hits[0];
    let mark = d.table.rows[0].mark.unwrap();
    assert!(!mark.on);
    assert_eq!(mark.action, act(MaterialsAct::ToggleMark(first)));
    s.act(EditorAction::Materials(MaterialsAct::ToggleMark(first)));
    let d = doc(&s);
    assert!(d.table.rows[0].mark.unwrap().on);
    let slot = d.basket.as_ref().unwrap().slots[0].as_ref().unwrap();
    assert_eq!(slot.name, model::catalog()[first].name);
    assert_eq!(slot.remove, act(MaterialsAct::ToggleMark(first)));
    // The slot's own remove button empties it again.
    s.act(EditorAction::Materials(MaterialsAct::ToggleMark(first)));
    assert!(doc(&s).basket.unwrap().slots[0].is_none());
}
