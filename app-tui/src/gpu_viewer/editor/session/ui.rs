//! The member editor's contribution to the CAD-style chrome: feature tree, property manager, ribbon,
//! menus and context menu, plus the `Authoring` impl the window drives. Availability and reasons
//! always come from `caps::availability`.
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use super::super::{authoring::{Authoring, Card, Relevance}, caps, learning::AdvisorAction, state::SupportPreset};

/// `"Undo (Ctrl/Cmd+Z)"` -> `("Undo", "Ctrl/Cmd+Z")`.
pub fn split_label(label: &str) -> (&str, &str) {
    match label.split_once(" (") {
        Some((name, rest)) => (name, rest.trim_end_matches(')')),
        None => (label, ""),
    }
}

fn ui(a: EditorAction) -> UiAction {
    UiAction::Editor(a)
}
fn entry(label: &str, shortcut: &str, a: EditorAction) -> Entry {
    Entry::new(label, shortcut, ui(a))
}
fn item(label: &str, shortcut: &str, a: EditorAction) -> Item {
    Item::Entry(entry(label, shortcut, a))
}

fn group_move(label: &str, axis: usize, ed: &Session) -> Item {
    let why = if ed.state.group_nodes().is_empty() { Err("select nodes or members first (click, Shift-click or Shift-drag a box)".to_string()) } else { Ok(()) };
    Item::Entry(entry(label, "", EditorAction::MoveGroup(axis)).when(why))
}

/// An operation as a menu/ribbon entry, enabled by the rule engine.
pub fn operation(ed: &Session, op: Operation) -> Entry {
    let (name, shortcut) = split_label(op.label());
    Entry::new(name, shortcut, ui(EditorAction::Run(op))).when(caps::availability(&ed.state, ed.solving(), op))
}
fn op_item(ed: &Session, op: Operation) -> Item {
    Item::Entry(operation(ed, op))
}
fn tool_entry(ed: &Session, t: Tool) -> Entry {
    Entry::new(t.label(), t.key().to_string(), ui(EditorAction::SetTool(t))).checked(ed.state.tool == t)
}

/// A coordinate with at most three decimals and no trailing zeros.
fn short(v: f64) -> String {
    let t = format!("{v:.3}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-0" { "0".into() } else { t.to_string() }
}

/// Most entries listed per tree folder (the rest are summarised).
const TREE_LIMIT: usize = 200;
const NODE_BASE: u32 = 1_000;
const MEMBER_BASE: u32 = 1_000_000;
const SUPPORT_BASE: u32 = 2_000_000;
const LOAD_BASE: u32 = 3_000_000;

fn advisor_items(ed: &Session) -> Vec<Item> {
    let act = |a: AdvisorAction| ui(EditorAction::Advisor(a));
    match ed.advisor_menu() {
        Err(reason) => [
            ("Show suggestions", AdvisorAction::ToggleSuggestions),
            ("Record my sessions (local)", AdvisorAction::ToggleRecording),
            ("Use learned ranking", AdvisorAction::ToggleLearned),
            ("Train from recordings", AdvisorAction::Train),
            ("Delete recordings", AdvisorAction::DeleteRecordings),
        ]
        .into_iter()
        .map(|(label, a)| Item::Entry(Entry::new(label, "", act(a)).when(Err(reason.clone()))))
        .collect(),
        Ok(m) => vec![
            Item::Entry(Entry::new("Show suggestions", "", act(AdvisorAction::ToggleSuggestions)).checked(m.suggestions)),
            Item::Entry(Entry::new("Record my sessions (local)", "", act(AdvisorAction::ToggleRecording)).checked(m.recording)),
            Item::Entry(Entry::new("Train from recordings", "", act(AdvisorAction::Train)).when(m.can_train)),
            Item::Entry(Entry::new("Use learned ranking", "", act(AdvisorAction::ToggleLearned)).checked(m.learned).when(if m.learned { Ok(()) } else { m.learned_available })),
            Item::Entry(Entry::new("Delete recordings", "", act(AdvisorAction::DeleteRecordings)).when(if m.has_recordings { Ok(()) } else { Err("nothing is recorded".into()) })),
        ],
    }
}

impl Session {
    pub fn select_entity(&mut self, hit: Hit) {
        self.state.select(Some(hit));
        self.touch();
    }
}

impl Authoring for Session {
    fn bench(&self) -> crate::gpu_viewer::bench::Bench {
        crate::gpu_viewer::bench::Bench::Members
    }
    fn path(&self) -> &std::path::Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        Session::act(self, action)
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        Session::key(self, key, mods)
    }
    fn click(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        Session::click(self, px, size, cam)
    }
    fn select_at(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        Session::select_at(self, px, size, cam)
    }
    fn scene(&mut self) -> Result<Scene, String> {
        Session::scene(self)
    }
    fn needs_scene(&self) -> bool {
        Session::needs_scene(self)
    }
    fn poll(&mut self) -> bool {
        Session::poll(self)
    }
    fn prompting(&self) -> bool {
        Session::prompting(self)
    }
    fn solving(&self) -> bool {
        Session::solving(self)
    }
    fn status(&self) -> String {
        self.state.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.state.status = text;
    }
    fn title(&self) -> String {
        format!("FEA member editor | {}{}", self.state.problem.name, if self.state.dirty { " *" } else { "" })
    }
    fn dirty(&self) -> bool {
        self.state.dirty
    }

    fn tree(&self) -> Vec<TreeNode> {
        let s = &self.state;
        let p = &s.problem;
        let mut out = vec![TreeNode::new(0, 0, 'M', format!("{}{}", p.name, if s.dirty { " *" } else { "" })).tone(Tone::Title)];
        let selected = |hit: Hit| s.selection == Some(hit);
        out.push(TreeNode::new(1, 1, '+', format!("Nodes ({})", p.nodes.len())).folder());
        for n in p.nodes.iter().take(TREE_LIMIT) {
            let label = format!("Node {} ({}, {}, {})", n.id, short(n.pos[0]), short(n.pos[1]), short(n.pos[2]));
            out.push(TreeNode::new(NODE_BASE + n.id, 2, 'o', label).selected(selected(Hit::Node(n.id))).on_click(ui(EditorAction::SelectNode(n.id))));
        }
        out.push(TreeNode::new(2, 1, '+', format!("Members ({})", p.members.len())).folder());
        for m in p.members.iter().take(TREE_LIMIT) {
            let kind = if m.kind.is_frame() { "Frame" } else { "Truss" };
            let glyph = if m.kind.is_frame() { 'F' } else { 'T' };
            let label = format!("{kind} {} ({}-{})", m.id, m.nodes[0], m.nodes[1]);
            out.push(TreeNode::new(MEMBER_BASE + m.id, 2, glyph, label).selected(selected(Hit::Member(m.id))).on_click(ui(EditorAction::SelectMember(m.id))));
        }
        let eye = |on: bool| if on { "[x] shown in model" } else { "[ ] hidden in model" };
        out.push(TreeNode::new(3, 1, '+', format!("Supports ({})", p.supports.len())).folder());
        out.push(TreeNode::new(5, 2, ' ', eye(s.show_supports)).tone(Tone::Dim).on_click(ui(EditorAction::ToggleSupports)));
        for sup in p.supports.iter().take(TREE_LIMIT) {
            let held: Vec<&str> = ["ux", "uy", "uz", "rx", "ry", "rz"].iter().zip(sup.fixed).filter(|(_, f)| *f).map(|(n, _)| *n).collect();
            out.push(TreeNode::new(SUPPORT_BASE + sup.node, 2, '^', format!("Support @ node {} ({})", sup.node, held.join(" "))).selected(selected(Hit::Node(sup.node))).on_click(ui(EditorAction::SelectNode(sup.node))));
        }
        out.push(TreeNode::new(4, 1, '+', format!("Loads ({})", p.loads.len())).folder());
        out.push(TreeNode::new(6, 2, ' ', eye(s.show_loads)).tone(Tone::Dim).on_click(ui(EditorAction::ToggleLoads)));
        for l in p.loads.iter().take(TREE_LIMIT) {
            let mut text = format!("Load @ node {} F({}, {}, {})", l.node, short(l.force[0]), short(l.force[1]), short(l.force[2]));
            if l.moment.iter().any(|m| *m != 0.0) {
                text.push_str(&format!(" M({}, {}, {})", short(l.moment[0]), short(l.moment[1]), short(l.moment[2])));
            }
            out.push(TreeNode::new(LOAD_BASE + l.node, 2, '>', text).selected(selected(Hit::Node(l.node))).on_click(ui(EditorAction::SelectNode(l.node))));
        }
        out
    }

    fn properties(&self) -> Vec<DockItem> {
        crate::gpu_viewer::editor::authoring::flatten(self.inspector())
    }

    fn solving_for(&self) -> Option<std::time::Duration> {
        self.runner.elapsed()
    }

    fn inspector(&self) -> Vec<Card> {
        use crate::gpu_viewer::text::wrap;
        let s = &self.state;
        let mut cards = Vec::new();
        // What is selected: its values. Nothing selected, nothing to show here (the tree lists the entities).
        let title = match s.selection {
            Some(Hit::Node(id)) => Some(format!("Node {id}")),
            Some(Hit::Member(id)) => Some(format!("{} member {id}", s.problem.member(id).map_or("", |m| if m.kind.is_frame() { "Frame" } else { "Truss" }))),
            None => None,
        };
        let mut sel = Vec::new();
        if !s.group.is_empty() {
            sel.push(DockItem::text(format!("+{} more selected: Delete removes all, Edit > Move moves them", s.group.len()), Tone::Title));
        }
        let rows = s.properties();
        for (i, r) in rows.iter().enumerate() {
            let editing = self.prompt.as_ref().filter(|_| i == s.focus);
            let value = match editing {
                Some(p) => format!("[{}_]", p.text),
                None => format!("{:.6e}", r.value),
            };
            sel.push(DockItem::field(r.label.clone(), value, if editing.is_some() { Tone::Warn } else if i == s.focus { Tone::On } else { Tone::Normal }, Some(ui(EditorAction::Focus(i))), i == s.focus));
        }
        if !rows.is_empty() {
            sel.push(DockItem::text(if self.prompt.is_some() { "type a number, Enter applies, Esc cancels" } else { "E or click a row edits its value" }, Tone::Dim));
        }
        if let Some(title) = title {
            cards.push(Card::new("selection", title, Relevance::Primary, sel));
        }
        // The modelling mode: what the next click does (the tool is also checked in the tool pill).
        let mut mode = vec![DockItem::text(format!("tool {}", s.tool.label()), Tone::Normal), DockItem::text(format!("plane {}={} (P)  grid {} (G)", s.plane_name(), s.plane_value, s.grid), Tone::Dim)];
        if !s.problem.unit_note.is_empty() {
            mode.push(DockItem::text(format!("units {} (not converted)", s.problem.unit_note), Tone::Dim));
        }
        cards.push(Card::new("mode", "Modelling", Relevance::Context, mode).for_steps(&["Model"]));
        let mut result = Vec::new();
        if let Some(sol) = s.current_solution() {
            result.push(DockItem::text(format!("max displacement {:.4e}", sol.max_displacement()), Tone::Normal));
            let peak = sol.members.iter().fold(0.0_f64, |a, m| a.max(m.axial_stress.abs()));
            result.push(DockItem::text(format!("max |axial stress| {peak:.4e}"), Tone::Normal));
            result.push(DockItem::text(format!("equilibrium: force {:.1e} moment {:.1e}", sol.acceptance.force_balance, sol.acceptance.moment_balance), Tone::On));
        } else if s.solution_is_stale() {
            result.push(DockItem::text("Result is stale: model changed since last solve", Tone::Warn));
        }
        if !result.is_empty() {
            cards.push(Card::new("result", "Result", Relevance::Info, result).for_steps(&["Simulation"]));
        }
        let mut next: Vec<DockItem> = wrap(&caps::next_step(s, self.solving()), 44).into_iter().map(|l| DockItem::text(l, Tone::Dim)).collect();
        if self.advice {
            let context = super::super::advisory::Context::of(self);
            let (label, ranked) = super::super::advisory::suggest(self.advisor.as_deref(), &context, &super::super::advisory::allowed(self));
            if let Some(top) = ranked.first() {
                next.push(DockItem::text(format!("suggested [{}]", label.chars().take(30).collect::<String>()), Tone::Dim));
                next.extend(wrap(&top.label(), 40).into_iter().map(|l| DockItem::text(format!("  {l}"), Tone::On)));
            }
        }
        cards.push(Card::new("next-step", "Next step", Relevance::Context, next));
        cards
    }

    fn ribbon(&self) -> Vec<RibbonTab> {
        let tool = |t| tool_entry(self, t);
        vec![
            RibbonTab::new("Model")
                .group("Insert", vec![tool(Tool::Node), tool(Tool::Truss), tool(Tool::Frame)])
                .group("Boundary", vec![tool(Tool::Support), tool(Tool::Load), operation(self, Operation::ApplyMoment), operation(self, Operation::FixRotations)])
                .group("Select", vec![tool(Tool::Select), operation(self, Operation::Delete)])
                .group("File", vec![operation(self, Operation::Save)])
                .group("Edit", vec![operation(self, Operation::Undo), operation(self, Operation::Redo), operation(self, Operation::ConvertToFrame), operation(self, Operation::ConvertToTruss)]),
            RibbonTab::new("Simulation")
                .group("Study", vec![operation(self, Operation::Solve), operation(self, Operation::CancelSolve)])
                .group("Results", vec![operation(self, Operation::ShowResults), operation(self, Operation::TogglePlanar)]),
        ]
    }

    fn menus(&self) -> Vec<Menu> {
        let tools: Vec<Item> = Tool::ALL.iter().map(|t| Item::Entry(tool_entry(self, *t))).collect();
        let tool_menu = vec![
            item("Cycle working plane", "P", EditorAction::CyclePlane),
            item("Cycle grid size", "G", EditorAction::CycleGrid),
            item("Fit model to view", "Z", EditorAction::FitView),
            item("Edit focused property", "E", EditorAction::EditProperty),
            Item::Separator,
            Item::Entry(entry("Show supports", "", EditorAction::ToggleSupports).checked(self.state.show_supports)),
            Item::Entry(entry("Show loads", "", EditorAction::ToggleLoads).checked(self.state.show_loads)),
        ];
        vec![
            Menu {
                title: "File".into(),
                items: vec![
                    op_item(self, Operation::Save),
                    item("Reload from file", "Ctrl/Cmd+O", EditorAction::Load),
                    item("Next template", "Ctrl/Cmd+N", EditorAction::NextTemplate),
                    Item::Separator,
                    Item::Entry(Entry::new("Quit", "", UiAction::Quit)),
                ],
            },
            Menu {
                title: "Edit".into(),
                items: vec![
                    op_item(self, Operation::Undo),
                    op_item(self, Operation::Redo),
                    op_item(self, Operation::Delete),
                    Item::Separator,
                    op_item(self, Operation::ConvertToFrame),
                    op_item(self, Operation::ConvertToTruss),
                    op_item(self, Operation::ApplyMoment),
                    op_item(self, Operation::FixRotations),
                    Item::Separator,
                    group_move("Move selection in x...", 0, self),
                    group_move("Move selection in y...", 1, self),
                    group_move("Move selection in z...", 2, self),
                ],
            },
            Menu { title: "Insert".into(), items: tools },
            Menu { title: "Tools".into(), items: tool_menu },
            Menu { title: "Advisor".into(), items: advisor_items(self) },
            Menu {
                title: "Simulation".into(),
                items: [Operation::Solve, Operation::CancelSolve, Operation::ShowResults, Operation::TogglePlanar].into_iter().map(|op| op_item(self, op)).collect(),
            },
        ]
    }

    fn click_with(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) -> bool {
        if additive {
            Session::click_additive(self, px, size, cam)
        } else {
            Session::click(self, px, size, cam)
        }
    }
    fn box_select(&mut self, a: [f32; 2], b: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) {
        Session::box_select(self, a, b, size, cam, additive)
    }
    fn take_popup(&mut self) -> Option<Vec<Item>> {
        let (tool, node) = self.state.popup.take()?;
        let p = &self.state.problem;
        let mut items = Vec::new();
        if tool == Tool::Support {
            let current = p.support(node).map(|s| s.fixed);
            let frame = p.has_frame(node);
            for preset in SupportPreset::ALL {
                let e = entry(&preset.label(), "", EditorAction::Support(preset));
                let is_current = current.unwrap_or([false; 6]) == preset.fixed(frame);
                items.push(Item::Entry(e.checked(is_current).when(if preset == SupportPreset::Remove && current.is_none() { Err("this node has no support".into()) } else { Ok(()) })));
            }
        } else {
            let has = p.load(node).is_some();
            let frame = p.has_frame(node);
            items.push(item("Default load", "", EditorAction::DefaultLoad));
            for (k, name) in ["Fx", "Fy", "Fz"].iter().enumerate() {
                items.push(item(&format!("Edit {name}..."), "", EditorAction::Edit(PropertyKind::Force(k))));
            }
            if frame {
                for (k, name) in ["Mx", "My", "Mz"].iter().enumerate() {
                    items.push(item(&format!("Edit {name}..."), "", EditorAction::Edit(PropertyKind::Moment(k))));
                }
            }
            items.push(Item::Entry(entry("Remove load", "", EditorAction::RemoveLoad).when(if has { Ok(()) } else { Err("this node has no load".into()) })));
        }
        Some(items)
    }

    fn context_menu(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for (i, p) in self.state.properties().iter().enumerate() {
            items.push(Item::Entry(Entry::new(format!("Edit {} ({:.4e})", p.label, p.value), "", ui(EditorAction::Focus(i)))));
        }
        if !items.is_empty() {
            items.push(Item::Separator);
        }
        for op in [Operation::Delete, Operation::ConvertToFrame, Operation::ConvertToTruss, Operation::ApplyMoment, Operation::FixRotations] {
            items.push(op_item(self, op));
        }
        items.push(Item::Separator);
        for op in [Operation::Undo, Operation::Redo, Operation::Solve, Operation::CancelSolve, Operation::ShowResults] {
            items.push(op_item(self, op));
        }
        items.push(Item::Separator);
        items.push(item("Fit model to view", "Z", EditorAction::FitView));
        items
    }
}
