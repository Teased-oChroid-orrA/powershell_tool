//! What a menu, the command palette and a ribbon button are made of: plain data supplied by
//! `menus.rs` (and the authoring sessions), whose availability rules come from `editor::caps` so they stay
//! single-sourced. The egui front end (`gui`) draws them and routes the clicks; nothing here knows the window.
use super::controller::UiAction;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    /// Keyboard shortcut shown right-aligned (display only; the key handlers own the binding).
    pub shortcut: String,
    pub action: UiAction,
    pub enabled: bool,
    /// Why the entry cannot run now (shown when it is hovered or chosen).
    pub reason: Option<String>,
    /// `Some` renders a check box.
    pub checked: Option<bool>,
}
impl Entry {
    pub fn new(label: impl Into<String>, shortcut: impl Into<String>, action: UiAction) -> Self {
        Self { label: label.into(), shortcut: shortcut.into(), action, enabled: true, reason: None, checked: None }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
    /// Enable or disable from a rule result (`Err` carries the explanation).
    pub fn when(mut self, rule: Result<(), String>) -> Self {
        match rule {
            Ok(()) => {}
            Err(reason) => {
                self.enabled = false;
                self.reason = Some(reason);
            }
        }
        self
    }
    /// Hover text: the label, its shortcut, and why it is unavailable.
    pub fn hover_text(&self) -> String {
        let mut t = self.label.clone();
        if !self.shortcut.is_empty() {
            t.push_str(&format!(" ({})", self.shortcut));
        }
        if let Some(r) = self.reason.as_ref().filter(|_| !self.enabled) {
            t.push_str(&format!(": {r}"));
        }
        t
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Entry(Entry),
    Separator,
    /// A non-selectable line of text (a popup's title).
    Heading(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<Item>,
}

/// What a click on a piece of chrome asks the window to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// An enabled entry was chosen.
    Action(UiAction),
    /// A disabled entry was chosen; the reason is for the status line.
    Blocked(String),
}

/// Subsequence score of `query` in `text` (case-insensitive): `None` when it does not match; lower is better
/// (fewer skipped characters, earlier start).
fn fuzzy(query: &str, text: &str) -> Option<usize> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut at, mut score, mut first) = (0usize, 0usize, None);
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let pos = text[at..].iter().position(|c| *c == q)? + at;
        first.get_or_insert(pos);
        score += pos - at;
        at = pos + 1;
    }
    Some(score * 4 + first.unwrap_or(0))
}

/// Rows the palette shows at most.
pub const PALETTE_ROWS: usize = 14;

/// The palette's rows: the best matching entries of every menu, labelled "Menu: label", best first. Disabled entries
/// stay listed and explain themselves when chosen.
pub fn palette_entries(menus: &[Menu], query: &str) -> Vec<Entry> {
    let mut found: Vec<(usize, usize, Entry)> = Vec::new();
    for (order, menu) in menus.iter().enumerate() {
        for item in &menu.items {
            if let Item::Entry(e) = item {
                let full = format!("{}: {}", menu.title, e.label);
                if let Some(score) = fuzzy(query, &full) {
                    let mut e = e.clone();
                    e.label = full;
                    found.push((score, order * 1000 + found.len(), e));
                }
            }
        }
    }
    found.sort_by_key(|(score, order, _)| (*score, *order));
    found.into_iter().take(PALETTE_ROWS).map(|(_, _, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::controller::Command;

    fn act(c: Command) -> UiAction {
        UiAction::Command(c)
    }
    fn menus() -> Vec<Menu> {
        vec![
            Menu {
                title: "File".into(),
                items: vec![
                    Item::Entry(Entry::new("Mesh", "M", act(Command::ToggleMesh)).checked(true)),
                    Item::Separator,
                    Item::Entry(Entry::new("Rewind", "Home", act(Command::Rewind)).when(Err("no animation".into()))),
                    Item::Entry(Entry::new("Reset", "R", act(Command::ResetCamera))),
                ],
            },
            Menu { title: "View".into(), items: vec![Item::Entry(Entry::new("Help", "H", act(Command::ToggleHelp)))] },
        ]
    }

    #[test]
    fn the_palette_filters_every_menu_fuzzily_and_keeps_disabled_entries_findable() {
        let m = menus();
        assert_eq!(palette_entries(&m, "").len(), 4, "an empty query lists everything");
        // "rst" is a subsequence of "File: Reset" only.
        let rows: Vec<String> = palette_entries(&m, "rst").into_iter().map(|e| e.label).collect();
        assert_eq!(rows, ["File: Reset"]);
        let rewind = palette_entries(&m, "rew").remove(0);
        assert!(!rewind.enabled && rewind.reason.as_deref() == Some("no animation"));
        assert!(palette_entries(&m, "z").is_empty(), "no match, no rows");
        // Better matches come first: a prefix beats a scattered subsequence.
        let order: Vec<String> = palette_entries(&m, "file: re").into_iter().map(|e| e.label).collect();
        assert_eq!(order[0], "File: Rewind");
    }

    #[test]
    fn hover_text_names_the_shortcut_and_the_reason() {
        let m = menus();
        let Item::Entry(rewind) = &m[0].items[2] else { panic!() };
        assert_eq!(rewind.hover_text(), "Rewind (Home): no animation");
        let Item::Entry(reset) = &m[0].items[3] else { panic!() };
        assert_eq!(reset.hover_text(), "Reset (R)");
    }
}
