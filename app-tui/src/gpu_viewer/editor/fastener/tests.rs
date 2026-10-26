use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
use crate::toolboxes::fastener_hole::domain::CountersinkSolveFor;
use crate::toolboxes::fastener_hole::model::{HoleType, NumberPart, NumberTarget, ToleranceInputMode};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fastener-session-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn session(tag: &str) -> FastenerSession {
    FastenerSession::new(&FastenerFile::capture(&FastenerHoleModel::default(), "wing skin"), dir(tag).join("case.json"))
}
fn act(s: &mut FastenerSession, a: FastenerAct) {
    s.act(EditorAction::Fastener(a));
}
fn type_text(s: &mut FastenerSession, text: &str) {
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
}
fn edit(s: &mut FastenerSession, row: FieldRow, value: &str) {
    act(s, FastenerAct::Row(row));
    assert!(s.prompting(), "{row:?} opens a typed edit");
    type_text(s, value);
    s.key(Key::Enter, Mods::default());
    assert!(!s.prompting());
}
fn texts(cards: &[Card]) -> String {
    cards.iter().flat_map(|c| std::iter::once(c.title.clone()).chain(c.items.iter().filter_map(|i| i.plain_text()))).collect::<Vec<_>>().join("\n")
}
fn ids(cards: &[Card]) -> Vec<String> {
    cards.iter().map(|c| c.id.clone()).collect()
}
const HOLE2: FieldRow = FieldRow::Number(NumberTarget::RegularHole2, NumberPart::Nominal, "Hole 2 Diameter");

#[test]
fn it_opens_on_the_terminal_forms_inputs_with_the_fit_and_a_drawing() {
    let mut s = session("open");
    assert_eq!(s.title(), "Fastener Hole | wing skin");
    assert!(s.benches().is_empty());
    assert_eq!(s.ribbon().into_iter().map(|t| t.title).collect::<Vec<_>>(), ["Inputs", "Report"]);
    let cards = s.inspector();
    assert_eq!(ids(&cards), ["fastener-hole-setup", "fastener-regular-hole-dimensions", "fastener-fit", "fastener-companion", "fastener-drawing"]);
    let t = texts(&cards);
    // The rows are the terminal form's labels and values.
    assert!(t.contains("Hole Type  Regular") && t.contains("Hole 1 Diameter Nominal  0.2500 in"), "{t}");
    assert!(t.contains("Fit  0.0000 -0.0000/+0.0000 in") && t.contains("TRANSITION"), "{t}");
    assert!(s.status().starts_with("TRANSITION"), "{}", s.status());
    assert_eq!(s.key_labels(), ["Parts", "Hole 1 wall", "Hole 2 wall", "Cut face"]);
    assert!(s.needs_scene());
    assert!(s.scene().unwrap().positions.len() > 100);
    assert!(!s.needs_scene());
}

#[test]
fn a_typed_value_reaches_the_solver_redraws_the_hole_and_undo_returns() {
    let mut s = session("edit");
    s.scene().unwrap();
    edit(&mut s, HOLE2, "0.26");
    assert!((s.model().regular_fit.nominal - 0.010).abs() < 1e-12);
    assert!(s.status().starts_with("CLEARANCE"), "{}", s.status());
    assert!(s.dirty() && s.title().ends_with('*') && s.needs_scene());
    // Typing the same value again is not an edit; a non-number is refused with a reason.
    let undo_depth = s.undo.len();
    edit(&mut s, HOLE2, "0.26");
    assert_eq!(s.undo.len(), undo_depth);
    edit(&mut s, HOLE2, "-");
    assert!(s.status().contains("is not a number"), "{}", s.status());
    // Esc leaves the value alone.
    act(&mut s, FastenerAct::Row(HOLE2));
    type_text(&mut s, "9");
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.model().regular.hole2.nominal, 0.26);
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.model().regular.hole2.nominal, 0.25);
    s.key(Key::Char('z'), Mods { command: true, shift: true });
    assert_eq!(s.model().regular.hole2.nominal, 0.26);
}

#[test]
fn the_choices_cycle_and_the_countersink_cards_show_the_solved_dimension_only() {
    let mut s = session("cs");
    act(&mut s, FastenerAct::Row(FieldRow::ToggleHoleType));
    assert_eq!(s.model().hole_type, HoleType::Countersunk);
    assert_eq!(s.key_labels()[2], "Countersink");
    let t = texts(&s.inspector());
    assert!(t.contains("Angle (calculated)  90.000 -0.000/+0.000 deg"), "{t}");
    assert!(!t.contains("Angle Nominal"), "the solved dimension is never an input: {t}");
    assert!(t.contains("Secondary countersink (Preserve Depth)"), "{t}");
    act(&mut s, FastenerAct::Row(FieldRow::ToggleSolveFor));
    assert_eq!(s.model().countersink.solve_for, CountersinkSolveFor::OuterDiameter);
    assert!(texts(&s.inspector()).contains("Outer diameter (calculated)"));
    act(&mut s, FastenerAct::Row(FieldRow::ToggleToleranceMode));
    assert_eq!(s.model().tolerance_mode, ToleranceInputMode::MinMax);
    assert!(texts(&s.inspector()).contains("Depth Min  0.1250 in"));
    // A countersink that cannot exist says why, and the picture is empty rather than stale: solving for the hole, a
    // depth this large leaves none.
    act(&mut s, FastenerAct::Row(FieldRow::ToggleSolveFor));
    assert_eq!(s.model().countersink.solve_for, CountersinkSolveFor::HoleDiameter);
    edit(&mut s, FieldRow::Number(NumberTarget::CsDepth, NumberPart::Max, "Depth"), "5");
    edit(&mut s, FieldRow::Number(NumberTarget::CsDepth, NumberPart::Min, "Depth"), "5");
    let t = texts(&s.inspector());
    assert!(t.contains("INVALID") && t.contains("Nothing to draw"), "{t}");
    assert_eq!(s.scene().unwrap().positions.len(), 3);
}

#[test]
fn save_and_reopen_round_trips_every_input_and_export_writes_the_terminal_report() {
    let mut s = session("save");
    act(&mut s, FastenerAct::Row(FieldRow::ToggleHoleType));
    act(&mut s, FastenerAct::Row(FieldRow::ToggleSecondaryMethod));
    edit(&mut s, FieldRow::Number(NumberTarget::CsDepth, NumberPart::TolPlus, "Depth"), "0.005");
    act(&mut s, FastenerAct::Save);
    assert!(!s.dirty(), "{}", s.status());
    let back = FastenerSession::open(&s.path).unwrap();
    assert_eq!(FastenerFile::capture(back.model(), "wing skin"), FastenerFile::capture(s.model(), "wing skin"));
    assert_eq!(back.model().countersink.depth.max, 0.13);
    act(&mut s, FastenerAct::Export);
    let report = std::fs::read_to_string(s.path.with_extension("report.txt")).unwrap();
    assert_eq!(report, view::build_report_text(s.model()));
    // A file another program changed is not overwritten.
    crate::persistence::write(&s.path, "{}").unwrap();
    edit(&mut s, FieldRow::Number(NumberTarget::CsDepth, NumberPart::TolPlus, "Depth"), "0.004");
    act(&mut s, FastenerAct::Save);
    assert!(s.status().starts_with("Save conflict"), "{}", s.status());
}

#[test]
fn a_file_with_an_unknown_choice_or_an_impossible_band_applies_the_rest_and_says_what_it_skipped() {
    let mut f = FastenerFile::capture(&FastenerHoleModel::default(), "x");
    f.choices.insert("hole type".into(), "Countersunk".into());
    f.choices.insert("solve for".into(), "Volume".into());
    f.dimensions.insert("countersink depth".into(), [0.2, 0.1, 0.3]);
    f.dimensions.insert("countersink outer diameter".into(), [0.6, 0.62, 0.64]);
    let s = FastenerSession::new(&f, dir("skip").join("case.json"));
    assert_eq!(s.model().hole_type, HoleType::Countersunk);
    assert_eq!(s.model().countersink.outer.nominal, 0.62);
    assert_eq!(s.model().countersink.depth.nominal, 0.125, "the impossible band is not applied");
    assert!(s.status().contains("solve for: Volume") && s.status().contains("countersink depth"), "{}", s.status());
    assert!(FastenerFile::from_json("{\"version\": 99}").unwrap_err().contains("newer version"));
}
