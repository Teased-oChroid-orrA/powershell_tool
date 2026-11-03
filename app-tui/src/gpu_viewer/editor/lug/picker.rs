//! The lug and bushing material pickers as a value card: a search field over a short list of the catalog's best matches.
//! The catalog and its search are Material Lookup's own (`material_lookup::model::{catalog, filter_sort}`); the allowables
//! a pick brings are `LugUiModel::input`'s.
use super::{LugAct, LugSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    ui::{CardRow, ValueCard},
};
use crate::toolboxes::material_lookup::model::{catalog, filter_sort, Query};

/// Entries the picker lists at once; the search narrows the rest.
const SHOWN: usize = 12;

/// Which material the picker is choosing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Lug,
    Bushing,
}

pub struct Picker {
    pub target: Target,
    pub query: String,
}

fn act(a: LugAct) -> UiAction {
    UiAction::Editor(EditorAction::Lug(a))
}

impl LugSession {
    pub(super) fn open_picker(&mut self, target: Target) {
        self.prompt = None;
        self.picker = Some(Picker { target, query: String::new() });
    }

    fn picked(&self, target: Target) -> usize {
        match target {
            Target::Lug => self.model.material_index,
            Target::Bushing => self.model.bushing_material_index,
        }
    }

    pub(super) fn picker_card(&self) -> Option<ValueCard> {
        let p = self.picker.as_ref()?;
        let current = self.picked(p.target);
        let hits = filter_sort(&Query { text: p.query.clone(), ..Default::default() });
        let more = hits.len().saturating_sub(SHOWN);
        // The current material first so it is never lost behind the search.
        let mut shown: Vec<usize> = hits.iter().copied().take(SHOWN).collect();
        if p.query.trim().is_empty() && !shown.contains(&current) {
            shown.insert(0, current);
            shown.truncate(SHOWN);
        }
        let mut rows = vec![CardRow::Field { label: "Search".into(), text: p.query.clone(), index: 0, unit: String::new() }];
        for i in shown {
            let m = catalog()[i];
            // The handbook leaves some bearing allowables blank (stored as 0).
            let fbru = if m.fbru_ksi > 0.0 { format!("{:.0}", m.fbru_ksi) } else { "n/a".into() };
            let detail = format!("Ftu {:.0}, Fty {:.0}, Fbru {fbru} ksi", m.ftu_ksi, m.sy_ksi);
            rows.push(CardRow::Choice { label: m.name.to_string(), detail, action: act(LugAct::PickMaterial(i)), current: i == current });
        }
        if more > 0 {
            rows.push(CardRow::Text(format!("{more} more: type to narrow the list.")));
        }
        if hits.is_empty() {
            rows.push(CardRow::Text("No material matches the search.".into()));
        }
        let (title, note) = match p.target {
            Target::Lug => ("Lug material", "E, Fty, Ftu and the bearing allowables of the checks come from it."),
            Target::Bushing => ("Bushing material", "The bushing is elastic in every analysis: its modulus, Poisson ratio and expansion are used."),
        };
        rows.push(CardRow::Text(note.into()));
        Some(ValueCard { title: title.into(), width: 400.0, tag: "catalog".into(), rows, apply_label: "Close".into(), apply: act(LugAct::ClosePicker), cancel: act(LugAct::ClosePicker) })
    }

    pub(super) fn picker_field(&mut self, index: usize, text: String) {
        if let (Some(p), 0) = (&mut self.picker, index) {
            p.query = text;
        }
    }

    pub(super) fn pick_material(&mut self, i: usize) {
        let Some(target) = self.picker.take().map(|p| p.target) else { return };
        if i == self.picked(target) || i >= catalog().len() {
            return;
        }
        self.checkpoint();
        match target {
            Target::Lug => self.model.material_index = i,
            Target::Bushing => self.model.bushing_material_index = i,
        }
        self.changed();
    }
}
