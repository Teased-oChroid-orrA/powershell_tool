//! What the Assembly Joints window shows: the inspector's cards, the tree, the ribbon and the menus. Plain data built from the loaded
//! assembly; the window draws it. Advice rows are `AssemblyAdvice::label()` with the measured numbers (`detail()`) dimmed under it.
use super::{AssemblyAct, AssemblySession, Loaded, Sel};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::{EditorAction, Tone},
    },
    menu::{Entry, Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
};
use fea_problem::assembly_advice::AssemblyAdvice;

const STEP: &str = "Assembly";
/// Tree node ids: the fixed nodes are below 1000, then one block for the bodies and one for the advice items, so an assembly of
/// any size never gives two nodes one id.
const BODY_ID: u32 = 1_000;
const ADVICE_ID: u32 = 1_000_000;

fn act(a: AssemblyAct) -> UiAction {
    UiAction::Editor(EditorAction::Assembly(a))
}
pub(super) fn entry(label: &str, shortcut: &str, a: AssemblyAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The advice group an item belongs to in the inspector.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Sites,
    Interfaces,
    Unmated,
    Check,
}
fn group(a: &AssemblyAdvice) -> Group {
    match a {
        AssemblyAdvice::Fastener { .. } => Group::Sites,
        AssemblyAdvice::Tie { .. } => Group::Interfaces,
        AssemblyAdvice::Unmated { .. } => Group::Unmated,
        AssemblyAdvice::Unresolved { .. } => Group::Check,
    }
}

impl AssemblySession {
    /// One line for the status bar.
    pub(super) fn summary(&self) -> String {
        match &self.loaded {
            Some(l) => {
                let unmated = if l.unmated > 0 { format!(", {}", plural(l.unmated, "hole without a mate", "holes without a mate")) } else { String::new() };
                format!(
                    "{}, {}, {}{unmated}, detected in {:.2} s",
                    plural(l.mesh.bodies(), "body", "bodies"),
                    plural(l.sites, "fastener site", "fastener sites"),
                    plural(l.interfaces, "interface", "interfaces"),
                    l.seconds
                )
            }
            None => "Open a STEP assembly: File > Open".to_string(),
        }
    }

    // ------------------------------------------------------------------ inspector

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        let Some(l) = &self.loaded else {
            return vec![Card::new("assembly-empty", "Assembly advice", Relevance::Primary, vec![DockItem::text("No assembly is open. File > Open reads a STEP file with more than one body.", Tone::Dim)]).for_steps(&[STEP])];
        };
        let mut items = Vec::new();
        for (g, title) in [(Group::Sites, "Fastener sites"), (Group::Interfaces, "Interfaces"), (Group::Unmated, "Holes without a mate"), (Group::Check, "Check")] {
            let rows: Vec<(usize, &AssemblyAdvice)> = l.advice.iter().enumerate().filter(|(_, a)| group(a) == g).collect();
            if rows.is_empty() {
                continue;
            }
            items.push(DockItem::Heading(title.to_string()));
            for (i, a) in rows {
                let on = self.selected == Some(Sel::Item(i));
                let tone = if g == Group::Check { Tone::Warn } else { Tone::Normal };
                items.push(DockItem::row(a.label(), tone, Some(act(AssemblyAct::Select(Sel::Item(i)))), on));
                items.push(DockItem::row(format!("  {}", a.detail()), Tone::Dim, Some(act(AssemblyAct::Select(Sel::Item(i)))), false));
            }
        }
        if items.is_empty() {
            items.push(DockItem::text("No fastener site, contact interface or bolt-sized hole without a mate was found.", Tone::Dim));
        }
        let mut cards = vec![Card::new("assembly-advice", "Assembly advice", Relevance::Primary, items).for_steps(&[STEP])];
        cards.push(self.why_card(l));
        cards.push(self.summary_card(l));
        cards
    }

    /// The reason of the selected item, one sentence per line; for a body, what was found in it.
    fn why_card(&self, l: &Loaded) -> Card {
        let items = match self.selected {
            Some(Sel::Item(i)) => match l.advice.get(i) {
                Some(a) => a.reason().split(". ").map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| DockItem::text(if s.ends_with('.') { s.to_string() } else { format!("{s}.") }, Tone::Normal)).collect(),
                None => Vec::new(),
            },
            Some(Sel::Body(b)) => vec![DockItem::text(format!("Body {}: {}.", b + 1, plural(l.holes_per_body.get(b).copied().unwrap_or(0), "hole", "holes")), Tone::Normal)],
            None => vec![DockItem::text("Select an advice item or a body to see why and where it is on the model.", Tone::Dim)],
        };
        Card::new("assembly-why", "Why", Relevance::Context, items).for_steps(&[STEP])
    }

    fn summary_card(&self, l: &Loaded) -> Card {
        let mut items = vec![
            DockItem::field("Bodies", l.mesh.bodies().to_string(), Tone::Normal, None, false),
            DockItem::field("Holes", l.holes.to_string(), Tone::Normal, None, false),
            DockItem::field("Fastener sites", l.sites.to_string(), Tone::Normal, None, false),
            DockItem::field("Interfaces", l.interfaces.to_string(), Tone::Normal, None, false),
        ];
        if l.unmated > 0 {
            items.push(DockItem::field("Holes without a mate", l.unmated.to_string(), Tone::Normal, None, false));
        }
        if l.unresolved > 0 {
            items.push(DockItem::field("To check", l.unresolved.to_string(), Tone::Warn, None, false));
        }
        items.push(DockItem::text("Advice only: nothing is applied to a problem, since a problem holds one body.", Tone::Dim));
        Card::new("assembly-summary", "Summary", Relevance::Info, items).for_steps(&[STEP])
    }

    // ------------------------------------------------------------------ tree

    pub(super) fn tree_nodes(&self) -> Vec<TreeNode> {
        let sel = |s: Sel| act(AssemblyAct::Select(s));
        let clear = act(AssemblyAct::ClearSelection);
        let Some(l) = &self.loaded else {
            return vec![TreeNode::new(1, 0, 'A', "Assembly: none open").on_click(act(AssemblyAct::Open))];
        };
        let mut out = vec![TreeNode::new(1, 0, 'A', format!("Assembly: {}", self.name)).on_click(clear.clone())];
        out.push(TreeNode::new(2, 1, '+', format!("Bodies ({})", l.mesh.bodies())).folder().on_click(clear.clone()));
        for b in 0..l.mesh.bodies() {
            out.push(TreeNode::new(BODY_ID + b as u32, 2, 'B', format!("Body {}", b + 1)).selected(self.selected == Some(Sel::Body(b))).on_click(sel(Sel::Body(b))));
        }
        out.push(TreeNode::new(3, 1, '+', "Detected").folder().on_click(clear.clone()));
        for (k, (glyph, text)) in [('o', format!("Holes ({})", l.holes)), ('H', format!("Fastener sites ({})", l.sites)), ('=', format!("Interfaces ({})", l.interfaces)), ('o', format!("Holes without a mate ({})", l.unmated))].into_iter().enumerate() {
            out.push(TreeNode::new(100 + k as u32, 2, glyph, text).on_click(clear.clone()));
        }
        out.push(TreeNode::new(4, 1, '+', format!("Advice ({})", l.advice.len())).folder().on_click(clear));
        for (i, a) in l.advice.iter().enumerate() {
            let (glyph, tone) = match group(a) {
                Group::Sites => ('H', Tone::Normal),
                Group::Interfaces => ('=', Tone::Normal),
                Group::Unmated => ('o', Tone::Normal),
                Group::Check => ('!', Tone::Warn),
            };
            out.push(TreeNode::new(ADVICE_ID + i as u32, 2, glyph, a.label()).tone(tone).selected(self.selected == Some(Sel::Item(i))).on_click(sel(Sel::Item(i))));
        }
        out
    }

    // ------------------------------------------------------------------ ribbon and menus

    pub(super) fn ribbon_tabs(&self) -> Vec<RibbonTab> {
        let none_selected = if self.selected.is_some() { Ok(()) } else { Err("nothing is selected".to_string()) };
        vec![RibbonTab::new(STEP)
            .group("File", vec![entry("Open STEP assembly", "Ctrl/Cmd+O", AssemblyAct::Open)])
            .group(
                "View",
                vec![entry("Exploded", "E", AssemblyAct::ToggleExploded).checked(self.exploded), entry("Clear selection", "Esc", AssemblyAct::ClearSelection).when(none_selected)],
            )]
    }

    pub(super) fn menu_bar(&self) -> Vec<Menu> {
        let tabs = self.ribbon_tabs();
        let all = |group: &str| tabs[0].groups.iter().filter(|g| g.title == group).flat_map(|g| g.buttons.clone()).map(Item::Entry).collect::<Vec<_>>();
        vec![Menu { title: "File".into(), items: all("File") }, Menu { title: "View".into(), items: all("View") }]
    }
}
