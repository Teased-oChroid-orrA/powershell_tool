//! The bolt and joint-template pickers as value cards. The bolt catalog (`model::BOLT_SECTIONS`), what picking an entry
//! fills (`PreloadModel::select_bolt`), the presets and the random joints (`joint_templates::{presets, random,
//! random_seed}`) and what applying one sets (`PreloadModel::apply_template`) are the terminal toolbox's own.
use super::{PreloadAct, PreloadSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    ui::{CardRow, ValueCard},
};
use crate::toolboxes::preload_analysis::{
    joint_templates::{self, presets, random_seed},
    model::{is_hi_lok, BOLT_SECTIONS, HI_LOK_NOTE},
};
use fastened_joint_solver::thread_catalog::BoltCatalogEntry;

/// Bolts the picker lists at once; the search narrows the rest.
const SHOWN: usize = 12;

pub enum Picker {
    /// The fastener catalog, narrowed by a search over designation and section.
    Bolt { query: String },
    /// The presets and the random generator; `last` names the random joint applied last.
    Template { last: Option<String> },
}

fn act(a: PreloadAct) -> UiAction {
    UiAction::Editor(EditorAction::Preload(a))
}

/// Every catalog entry as `(section, entry, section title, entry)`, in the catalog's order.
fn catalog() -> impl Iterator<Item = (usize, usize, &'static str, &'static BoltCatalogEntry)> {
    BOLT_SECTIONS.into_iter().enumerate().flat_map(|(s, (title, entries))| entries.iter().enumerate().map(move |(e, entry)| (s, e, title, entry)))
}

/// Every word of `query` appears in the entry's designation or its section's title.
fn matches(query: &str, title: &str, entry: &BoltCatalogEntry) -> bool {
    let hay = format!("{} {}", entry.designation, title).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

impl PreloadSession {
    pub(super) fn open_picker(&mut self, p: Picker) {
        self.prompt = None;
        self.picker = Some(p);
    }

    pub(super) fn picker_card(&self) -> Option<ValueCard> {
        match self.picker.as_ref()? {
            Picker::Bolt { query } => Some(self.bolt_card(query)),
            Picker::Template { last } => Some(template_card(last.as_deref())),
        }
    }

    fn bolt_card(&self, query: &str) -> ValueCard {
        let current = self.model.matching_bolt().map(|b| b.designation);
        let hits: Vec<_> = catalog().filter(|(_, _, title, entry)| matches(query, title, entry)).collect();
        let more = hits.len().saturating_sub(SHOWN);
        let mut shown: Vec<_> = hits.iter().copied().take(SHOWN).collect();
        // The current bolt first so it is never lost behind the list.
        if query.trim().is_empty() {
            if let Some(cur) = catalog().find(|(_, _, _, e)| Some(e.designation) == current) {
                if !shown.iter().any(|h| (h.0, h.1) == (cur.0, cur.1)) {
                    shown.insert(0, cur);
                    shown.truncate(SHOWN);
                }
            }
        }
        let mut rows = vec![CardRow::Field { label: "Search".into(), text: query.to_string(), index: 0, unit: String::new() }];
        let mut section = None;
        for (s, e, title, entry) in shown {
            if section != Some(s) {
                rows.push(CardRow::Text(title.to_string()));
                section = Some(s);
            }
            let detail = format!("{:.4} in, {:.0} TPI{}", entry.major_diameter, entry.threads_per_inch, if is_hi_lok(entry) { ", thread only" } else { "" });
            rows.push(CardRow::Choice { label: entry.designation.to_string(), detail, action: act(PreloadAct::PickBolt(s, e)), current: Some(entry.designation) == current });
        }
        if more > 0 {
            rows.push(CardRow::Text(format!("{more} more: type to narrow the list.")));
        }
        if hits.is_empty() {
            rows.push(CardRow::Text("No fastener matches the search.".into()));
        }
        rows.push(CardRow::Text("Fills the thread and shank diameter; every value stays editable.".into()));
        ValueCard { title: "Bolt".into(), width: 400.0, tag: "catalog".into(), rows, apply_label: "Close".into(), apply: act(PreloadAct::ClosePicker), cancel: act(PreloadAct::ClosePicker) }
    }

    pub(super) fn picker_field(&mut self, index: usize, text: String) {
        if let (Some(Picker::Bolt { query }), 0) = (&mut self.picker, index) {
            *query = text;
        }
    }

    pub(super) fn pick_bolt(&mut self, section: usize, entry: usize) {
        let Some(e) = BOLT_SECTIONS.get(section).and_then(|(_, entries)| entries.get(entry)) else { return };
        self.picker = None;
        if self.model.matching_bolt().is_some_and(|b| b.designation == e.designation) {
            return;
        }
        self.checkpoint();
        self.model.select_bolt(e);
        self.changed();
        if is_hi_lok(e) {
            self.status = HI_LOK_NOTE.into();
        }
    }

    pub(super) fn apply_preset(&mut self, i: usize) {
        let Some(t) = presets().into_iter().nth(i) else { return };
        self.picker = None;
        self.checkpoint();
        self.model.apply_template(&t);
        self.changed();
        self.status = format!("Joint template applied: {}.", t.name);
    }

    /// Apply the next joint of this session's random sequence; the picker stays open for another.
    pub(super) fn random_joint(&mut self) {
        self.randoms += 1;
        let t = joint_templates::random(random_seed(self.random_base, self.randoms));
        self.checkpoint();
        self.model.apply_template(&t);
        self.changed();
        self.picker = Some(Picker::Template { last: Some(format!("#{}: {}", self.randoms, t.name)) });
    }
}

fn template_card(last: Option<&str>) -> ValueCard {
    let mut rows: Vec<CardRow> = presets()
        .into_iter()
        .enumerate()
        .map(|(i, t)| CardRow::Choice { label: t.name, detail: format!("{} layers", t.layers.len()), action: act(PreloadAct::ApplyTemplate(i)), current: false })
        .collect();
    rows.push(CardRow::Choice { label: "Random joint".into(), detail: "a new sensible stack-up each click".into(), action: act(PreloadAct::RandomJoint), current: false });
    rows.push(CardRow::Text(match last {
        Some(name) => format!("Applied random joint {name}. Undo returns to the one before."),
        None => "Applying fills the members, bearing faces, shank length, nut data and a typical torque; everything stays editable.".into(),
    }));
    ValueCard { title: "Joint template".into(), width: 400.0, tag: "stack-up".into(), rows, apply_label: "Close".into(), apply: act(PreloadAct::ClosePicker), cancel: act(PreloadAct::ClosePicker) }
}
