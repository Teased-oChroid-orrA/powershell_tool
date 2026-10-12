//! Menu bar, ribbon and context-menu contents shared by every window mode. Model-specific menus and
//! ribbon tabs come from the `Authoring` session (whose rules own availability); this module adds the
//! viewer's own View / Window / Help menus and View ribbon tab and composes them in CAD order
//! (File, Edit, View, Insert, Tools, Simulation, Window, Help). Pure: reads state, returns data.
use super::{
    bench::Bench,
    controller::{Command, LayoutOp, Panel, StandardView, UiAction, ViewState, Visible},
    editor::authoring::Authoring,
    menu::{Entry, Item, Menu},
    ui::RibbonTab,
    Scene,
};

/// Everything the menus depend on.
pub struct MenuContext<'a> {
    pub view: &'a ViewState,
    pub scene: &'a Scene,
    /// Number of loaded results (viewer mode); 1 in an editor.
    pub results: usize,
    pub editor: Option<&'a dyn Authoring>,
    pub visible: Visible,
    /// The adaptive layout is learning from use.
    pub adaptive: bool,
}

fn cmd(label: &str, shortcut: &str, c: Command) -> Entry {
    Entry::new(label, shortcut, UiAction::Command(c))
}
fn ok_if(e: Entry, ok: bool, why: &str) -> Entry {
    e.when(if ok { Ok(()) } else { Err(why.to_string()) })
}

/// Display, orientation, animation and result-navigation entries, grouped as the ribbon shows them.
struct ViewEntries {
    display: Vec<Entry>,
    orientation: Vec<Entry>,
    animation: Vec<Entry>,
    results: Vec<Entry>,
}
fn view_entries(c: &MenuContext) -> ViewEntries {
    let (v, scene) = (c.view, c.scene);
    let animated = scene.duration() > 0.0;
    let nfields = 1 + scene.fields.len();
    let still = "this result is not animated";
    let gain_keys = if c.editor.is_some() { (".", ",") } else { ("Up", "Down") };
    let mut results = Vec::new();
    if c.results > 1 {
        results.push(Entry::new("Previous result", "[", UiAction::CycleResult { forward: false }));
        results.push(Entry::new("Next result", "]", UiAction::CycleResult { forward: true }));
    }
    results.push(ok_if(cmd("Next field", "F", Command::CycleField { forward: true }), nfields > 1, "this result has a single field"));
    results.push(ok_if(cmd("Previous field", "Shift+F", Command::CycleField { forward: false }), nfields > 1, "this result has a single field"));
    ViewEntries {
        display: vec![
            cmd("Mesh edges", "M", Command::ToggleMesh).checked(v.show_mesh),
            cmd("Contour", "C", Command::ToggleContour).checked(v.show_contour),
            cmd("Deformation", "D", Command::ToggleDeformation).checked(v.deformed),
            cmd("Larger deformation", gain_keys.0, Command::GainUp),
            cmd("Smaller deformation", gain_keys.1, Command::GainDown),
        ],
        orientation: {
            let mut o: Vec<Entry> = StandardView::ALL.iter().map(|s| cmd(s.label(), "", Command::StandardView(*s))).collect();
            o.push(cmd("Reset camera", "R", Command::ResetCamera));
            o
        },
        animation: vec![
            ok_if(cmd(if v.paused { "Play" } else { "Pause" }, "Space", Command::TogglePause), animated, still),
            ok_if(cmd("Rewind", "Home", Command::Rewind), animated, still),
            ok_if(cmd("Step forward", "Right", Command::StepFrame { forward: true }), animated, still),
            ok_if(cmd("Step back", "Left", Command::StepFrame { forward: false }), animated, still),
            cmd("Faster", "+", Command::SpeedUp),
            cmd("Slower", "-", Command::SpeedDown),
        ],
        results,
    }
}

fn items(entries: Vec<Entry>) -> impl Iterator<Item = Item> {
    entries.into_iter().map(Item::Entry)
}

fn view_menu(c: &MenuContext) -> Menu {
    let e = view_entries(c);
    let mut out: Vec<Item> = Vec::new();
    out.extend(items(e.results));
    out.push(Item::Separator);
    out.extend(items(e.display));
    out.push(Item::Separator);
    out.extend(items(e.orientation));
    out.push(Item::Separator);
    out.extend(items(e.animation));
    Menu { title: "View".into(), items: out }
}

fn window_menu(c: &MenuContext) -> Menu {
    let v = c.visible;
    let panel = |label: &str, p: Panel, on: bool| Item::Entry(Entry::new(label, "", UiAction::TogglePanel(p)).checked(on));
    let mut items = Vec::new();
    if let Some(ed) = c.editor {
        // Benches keep the other studies' menus out of the way; the model is kept across solid benches.
        for b in Bench::ALL {
            items.push(Item::Entry(Entry::new(format!("Bench: {}", b.label()), "", b.action()).checked(ed.bench() == b)));
        }
        items.push(Item::Separator);
    }
    items.extend([
        panel("Tool bar", Panel::Ribbon, v.ribbon),
        panel("Feature tree", Panel::Tree, v.tree),
        panel("Inspector", Panel::Properties, v.properties),
        panel("Status bar", Panel::Status, v.status),
    ]);
    items.push(Item::Separator);
    items.push(Item::Entry(Entry::new("Adaptive layout", "", UiAction::Layout(LayoutOp::ToggleAdaptive)).checked(c.adaptive)));
    items.push(Item::Entry(Entry::new("What has the layout learned?", "", UiAction::Layout(LayoutOp::Explain))));
    items.push(Item::Entry(Entry::new("Reset learned layout", "", UiAction::Layout(LayoutOp::Reset))));
    items.push(Item::Entry(Entry::new("Open or fold the feature tree", "", UiAction::Layout(LayoutOp::ToggleTree))));
    Menu { title: "Window".into(), items }
}

fn help_menu(c: &MenuContext) -> Menu {
    Menu {
        title: "Help".into(),
        items: vec![
            Item::Entry(Entry::new("Command palette", "Ctrl/Cmd+K", UiAction::OpenPalette)),
            Item::Entry(Entry::new("Keyboard shortcuts", "H", UiAction::Command(Command::ToggleHelp)).checked(c.view.show_help)),
        ],
    }
}

/// The ribbon's View tab (always last): display, orientation, animation, results.
pub fn view_tab(c: &MenuContext) -> RibbonTab {
    let e = view_entries(c);
    let mut tab = RibbonTab::new("View").group("Display", e.display).group("Orientation", e.orientation);
    if c.scene.duration() > 0.0 {
        tab = tab.group("Animation", e.animation);
    }
    tab.group("Results", e.results)
}

pub fn ribbon(c: &MenuContext) -> Vec<RibbonTab> {
    let mut tabs = c.editor.map(|e| e.ribbon()).unwrap_or_default();
    tabs.push(view_tab(c));
    tabs
}

const ORDER: [&str; 9] = ["File", "Edit", "View", "Insert", "Tools", "Simulation", "Advisor", "Window", "Help"];

/// The menu bar in CAD order. Unknown authoring titles sort with Tools/Simulation, keeping their order.
pub fn bar(c: &MenuContext) -> Vec<Menu> {
    let mut menus = c.editor.map(|e| e.menus()).unwrap_or_default();
    if menus.is_empty() {
        menus.push(Menu { title: "File".into(), items: vec![Item::Entry(Entry::new("Quit", "Esc", UiAction::Quit))] });
    }
    menus.push(view_menu(c));
    menus.push(window_menu(c));
    menus.push(help_menu(c));
    let rank = |m: &Menu| ORDER.iter().position(|t| *t == m.title).unwrap_or(5);
    menus.sort_by_key(rank);
    menus
}

/// Right-click menu: the model's operations for the current selection, then the display toggles.
pub fn context(c: &MenuContext) -> Vec<Item> {
    let mut items = c.editor.map(|e| e.context_menu()).unwrap_or_default();
    if !items.is_empty() {
        items.push(Item::Separator);
    }
    let v = c.view;
    items.push(Item::Entry(cmd("Mesh edges", "M", Command::ToggleMesh).checked(v.show_mesh)));
    items.push(Item::Entry(cmd("Contour", "C", Command::ToggleContour).checked(v.show_contour)));
    items.push(Item::Entry(cmd("Deformation", "D", Command::ToggleDeformation).checked(v.deformed)));
    items.push(Item::Entry(cmd("Reset camera", "R", Command::ResetCamera)));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::editor::{
        caps::{self, Operation},
        session::{EditorAction, Session},
        state::Tool,
    };

    fn scene() -> Scene {
        Scene {
            label: "t".into(),
            positions: vec![],
            triangles: vec![],
            samples: vec![],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        }
    }
    fn titles(m: &[Menu]) -> Vec<&str> {
        m.iter().map(|m| m.title.as_str()).collect()
    }
    fn find<'a>(items: &'a [Item], label: &str) -> &'a Entry {
        items
            .iter()
            .find_map(|i| match i {
                Item::Entry(e) if e.label == label => Some(e),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no entry {label}"))
    }
    fn session() -> Session {
        Session::new(fea_problem::members::blank_planar(), std::env::temp_dir().join("menus-test.json"))
    }
    fn ctx<'a>(v: &'a ViewState, s: &'a Scene, ed: Option<&'a dyn Authoring>) -> MenuContext<'a> {
        MenuContext { view: v, scene: s, results: 1, editor: ed, visible: Visible::default(), adaptive: true }
    }

    #[test]
    fn viewer_mode_has_file_view_window_help_in_cad_order() {
        let (v, s) = (ViewState::new(&scene()), scene());
        let m = bar(&ctx(&v, &s, None));
        assert_eq!(titles(&m), ["File", "View", "Window", "Help"]);
        let view = &m[1].items;
        assert!(!find(view, "Rewind").enabled, "a still result cannot rewind");
        assert_eq!(find(view, "Rewind").reason.as_deref(), Some("this result is not animated"));
        for sv in StandardView::ALL {
            assert!(find(view, sv.label()).enabled, "{sv:?}");
        }
        let window = &m[2].items;
        assert!(["Tool bar", "Feature tree", "Inspector", "Status bar"].iter().all(|l| find(window, l).checked == Some(true)));
    }

    #[test]
    fn editor_menus_follow_solidworks_order() {
        let (v, s, ed) = (ViewState::new(&scene()), scene(), session());
        let m = bar(&ctx(&v, &s, Some(&ed)));
        assert_eq!(titles(&m), ["File", "Edit", "View", "Insert", "Tools", "Simulation", "Advisor", "Window", "Help"]);
    }

    #[test]
    fn editor_availability_comes_from_the_rule_engine() {
        let (v, s, ed) = (ViewState::new(&scene()), scene(), session());
        let m = bar(&ctx(&v, &s, Some(&ed)));
        for menu in &m {
            for item in &menu.items {
                let Item::Entry(e) = item else { continue };
                if let UiAction::Editor(EditorAction::Run(op)) = e.action {
                    let rule = caps::availability(&ed.state, ed.solving(), op);
                    assert_eq!(e.enabled, rule.is_ok(), "{op:?}");
                    assert_eq!(e.reason, rule.err(), "{op:?}");
                }
            }
        }
        let sim = m.iter().find(|m| m.title == "Simulation").unwrap();
        let solve = find(&sim.items, "Solve");
        assert!(!solve.enabled, "an empty model cannot be solved");
        assert_eq!(solve.shortcut, "Enter");
    }

    #[test]
    fn every_operation_and_tool_is_reachable_from_menus_and_from_the_ribbon() {
        let (v, s, ed) = (ViewState::new(&scene()), scene(), session());
        let c = ctx(&v, &s, Some(&ed));
        let menu_actions: Vec<UiAction> = bar(&c).into_iter().flat_map(|m| m.items).filter_map(|i| if let Item::Entry(e) = i { Some(e.action) } else { None }).collect();
        let ribbon_actions: Vec<UiAction> = ribbon(&c).into_iter().flat_map(|t| t.groups).flat_map(|g| g.buttons).map(|b| b.action).collect();
        for op in Operation::ALL {
            let a = UiAction::Editor(EditorAction::Run(op));
            assert!(menu_actions.contains(&a), "{op:?} missing from the menus");
            assert!(ribbon_actions.contains(&a), "{op:?} missing from the ribbon");
        }
        for t in Tool::ALL {
            let a = UiAction::Editor(EditorAction::SetTool(t));
            assert!(menu_actions.contains(&a) && ribbon_actions.contains(&a), "{t:?}");
        }
    }

    #[test]
    fn ribbon_ends_with_the_view_tab_and_hides_animation_for_still_results() {
        let (v, s, ed) = (ViewState::new(&scene()), scene(), session());
        let tabs = ribbon(&ctx(&v, &s, Some(&ed)));
        assert_eq!(tabs.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Model", "Simulation", "View"]);
        assert!(!tabs.last().unwrap().groups.iter().any(|g| g.title == "Animation"));
        let mut animated = scene();
        animated.times = vec![0.0, 1.0];
        let tab = view_tab(&ctx(&v, &animated, None));
        assert!(tab.groups.iter().any(|g| g.title == "Animation"));
    }

    #[test]
    fn tools_are_checked_in_the_insert_menu() {
        let (v, s, mut ed) = (ViewState::new(&scene()), scene(), session());
        ed.set_tool(Tool::Truss);
        let m = bar(&ctx(&v, &s, Some(&ed)));
        let insert = m.iter().find(|m| m.title == "Insert").unwrap();
        assert_eq!(find(&insert.items, Tool::Truss.label()).checked, Some(true));
        assert_eq!(find(&insert.items, Tool::Node.label()).checked, Some(false));
    }

    #[test]
    fn advisor_entries_explain_why_the_learned_ranking_is_unavailable() {
        let (v, s, mut ed) = (ViewState::new(&scene()), scene(), session());
        let tools = |ed: &Session| bar(&ctx(&v, &s, Some(ed))).into_iter().find(|m| m.title == "Advisor").unwrap();
        // Without a store (no app-data folder) nothing is available and every entry says why.
        let m = tools(&ed);
        assert!(!find(&m.items, "Record my sessions (local)").enabled);
        let dir = std::env::temp_dir().join(format!("menus-advisor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ed.attach_learning(crate::gpu_viewer::editor::learning::Store::new(dir.clone()), 1);
        let m = tools(&ed);
        assert_eq!(find(&m.items, "Show suggestions").checked, Some(true));
        assert_eq!(find(&m.items, "Record my sessions (local)").checked, Some(false));
        assert!(find(&m.items, "Record my sessions (local)").enabled);
        assert!(!find(&m.items, "Train from recordings").enabled, "nothing recorded yet");
        let learned = find(&m.items, "Use learned ranking");
        assert!(!learned.enabled && learned.reason.as_deref().unwrap().contains("no trained model"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn context_menu_lists_selection_properties_and_gated_operations() {
        let (v, s, mut ed) = (ViewState::new(&scene()), scene(), session());
        let empty = context(&ctx(&v, &s, Some(&ed)));
        assert!(!find(&empty, "Delete selection").enabled);
        assert_eq!(find(&empty, "Delete selection").reason.as_deref(), Some("select a node or member first"));
        assert!(!empty.iter().any(|i| matches!(i, Item::Entry(e) if e.label.starts_with("Edit ") && !e.label.starts_with("Edit plane"))), "nothing selected: only the working plane is editable");
        ed.state.commit(fea_problem::members::Edit::AddNode { pos: [1.0, 2.0, 0.0] }).unwrap();
        let id = ed.state.problem.nodes[0].id;
        ed.state.select(Some(crate::gpu_viewer::editor::camera::Hit::Node(id)));
        let items = context(&ctx(&v, &s, Some(&ed)));
        assert!(find(&items, "Delete selection").enabled);
        assert!(items.iter().any(|i| matches!(i, Item::Entry(e) if e.label.starts_with("Edit x "))));
    }

    #[test]
    fn viewer_context_menu_is_the_display_toggles() {
        let (v, s) = (ViewState::new(&scene()), scene());
        let items = context(&ctx(&v, &s, None));
        assert_eq!(find(&items, "Contour").checked, Some(true));
        assert_eq!(find(&items, "Mesh edges").checked, Some(false));
    }
}
