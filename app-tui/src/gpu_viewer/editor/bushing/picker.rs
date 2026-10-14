//! The pickers behind a bore, a bushing ID, a friction coefficient and the two materials, as value cards: a search field
//! over a short list of the catalog's best matches. The lists, the filters and what choosing an entry sets are the
//! terminal toolbox's own (`reamer_picker`, `bushing_id_picker`, `friction_picker`, `material_picker`).
use super::{BushingAct, BushingSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    ui::{CardRow, ValueCard},
};
use crate::toolboxes::bushing::{
    bushing_id_picker::BushingIdPickerState,
    model::NumberTarget,
    reamer_picker::ReamerPickerState,
};
use bushing_solver::friction::FRICTION_TYPICALS;

/// Entries a picker lists at once; the search narrows the rest.
const SHOWN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Housing,
    Bushing,
    Bore,
    Id,
    Friction,
}

pub struct Picker {
    pub kind: Kind,
    pub query: String,
    /// The terminal pickers' own state: it loads the user's library and does the filtering.
    reamer: ReamerPickerState,
    ids: BushingIdPickerState,
}

fn act(a: BushingAct) -> UiAction {
    UiAction::Editor(EditorAction::Bushing(a))
}

impl BushingSession {
    pub(super) fn open_picker(&mut self, kind: Kind) {
        self.prompt = None;
        self.picker = Some(Picker { kind, query: String::new(), reamer: ReamerPickerState::open_near(&self.st.model), ids: BushingIdPickerState::open_near(&self.st.model) });
    }

    pub(super) fn picker_card(&self) -> Option<ValueCard> {
        let p = self.picker.as_ref()?;
        let m = &self.st.model;
        let mut rows = vec![CardRow::Field { label: "Search".into(), text: p.query.clone(), index: 0, unit: String::new() }];
        let (title, tag) = match p.kind {
            Kind::Housing => ("Housing material", "material"),
            Kind::Bushing => ("Bushing material", "material"),
            Kind::Bore => ("Bore diameter", "reamer"),
            Kind::Id => ("Bushing ID", "drill or saved"),
            Kind::Friction => ("Friction coefficient", "typical"),
        };
        let mut more = 0usize;
        match p.kind {
            Kind::Housing | Kind::Bushing => {
                let bushing = p.kind == Kind::Bushing;
                let current = if bushing { m.bushing_material_index } else { m.housing_material_index };
                let catalog = m.material_catalog();
                let mut filter = crate::toolboxes::bushing::material_picker::MaterialPickerState::default();
                filter.filter_text = p.query.clone();
                let hits = filter.visible_indices(m);
                more = hits.len().saturating_sub(SHOWN);
                // The current material first so it is never lost behind the search.
                let mut shown: Vec<usize> = hits.iter().copied().take(SHOWN).collect();
                if p.query.trim().is_empty() && !shown.contains(&current) {
                    shown.insert(0, current);
                    shown.truncate(SHOWN);
                }
                for i in shown {
                    let mat = catalog[i];
                    let detail = mat.extra.map_or(format!("E {:.0} ksi, Fy {:.0} ksi", mat.e_ksi, mat.sy_ksi), |x| format!("{} {} {}", x.alloy, x.temper, x.form).split_whitespace().collect::<Vec<_>>().join(" "));
                    rows.push(CardRow::Choice { label: mat.name.to_string(), detail, action: act(BushingAct::PickMaterial(bushing, i)), current: i == current });
                }
            }
            Kind::Bore => {
                let all = p.reamer.visible();
                more = all.len().saturating_sub(SHOWN);
                // Around the current bore when nothing is typed, else the first matches.
                let near = if p.query.trim().is_empty() { all.iter().position(|(e, _)| (e.nominal_in - m.bore_dia).abs() < 1e-9).or(Some(p.reamer.cursor)).map_or(0, |c| c.saturating_sub(SHOWN / 2)) } else { 0 };
                for (i, (e, labels)) in all.iter().enumerate().skip(near).take(SHOWN) {
                    let tol = format!("+{:.4}/-{:.4}", e.tool_tolerance_plus_in, e.tool_tolerance_minus_in);
                    let tags = if labels.is_empty() { String::new() } else { format!("  {}", labels.join(", ")) };
                    rows.push(CardRow::Choice { label: format!("{}  {:.4} in", e.size_label, e.nominal_in), detail: format!("{tol}{tags}"), action: act(BushingAct::PickBore(i)), current: (e.nominal_in - m.bore_dia).abs() < 1e-9 });
                }
                rows.push(CardRow::Choice { label: "Type a value".into(), detail: "a bore that is not a catalog size".into(), action: act(BushingAct::TypeValue(NumberTarget::BoreDia)), current: false });
            }
            Kind::Id => {
                let all = p.ids.visible();
                more = all.len().saturating_sub(SHOWN);
                let near = if p.query.trim().is_empty() { all.iter().position(|r| (r.id_in() - m.id_bushing).abs() < 1e-9).map_or(0, |c| c.saturating_sub(SHOWN / 2)) } else { 0 };
                for (i, r) in all.iter().enumerate().skip(near).take(SHOWN) {
                    rows.push(CardRow::Choice { label: format!("{}  {:.4} in", r.label(), r.id_in()), detail: r.detail(), action: act(BushingAct::PickId(i)), current: (r.id_in() - m.id_bushing).abs() < 1e-9 });
                }
                rows.push(CardRow::Choice { label: "Type a value".into(), detail: "an ID that is not a listed size".into(), action: act(BushingAct::TypeValue(NumberTarget::IdBushing)), current: false });
            }
            Kind::Friction => {
                let q = p.query.to_lowercase();
                for (i, f) in FRICTION_TYPICALS.iter().enumerate().filter(|(_, f)| q.trim().is_empty() || f.label.to_lowercase().contains(q.trim())) {
                    rows.push(CardRow::Choice { label: format!("{:.2}  {}", f.value, f.label), detail: f.note.chars().take(70).collect::<String>() + if f.note.chars().count() > 70 { "..." } else { "" }, action: act(BushingAct::PickFriction(i)), current: (f.value - m.friction).abs() < 1e-9 });
                }
                rows.push(CardRow::Choice { label: "Type a value".into(), detail: "a coefficient that is not listed".into(), action: act(BushingAct::TypeValue(NumberTarget::Friction)), current: false });
            }
        }
        if more > 0 {
            rows.push(CardRow::Text(format!("{more} more: type to narrow the list.")));
        }
        Some(ValueCard { title: title.into(), width: 400.0, tag: tag.into(), rows, apply_label: "Close".into(), apply: act(BushingAct::ClosePicker), cancel: act(BushingAct::ClosePicker) })
    }

    pub(super) fn picker_field(&mut self, index: usize, text: String) {
        if let (0, Some(p)) = (index, &mut self.picker) {
            p.reamer.filter_text = text.clone();
            p.ids.filter_text = text.clone();
            p.query = text;
        }
    }

    /// The `i`th entry of the bore picker's list for the current search.
    pub(super) fn pick_bore(&mut self, i: usize) {
        let Some(p) = &self.picker else { return };
        if let Some((entry, _)) = p.reamer.visible().get(i).cloned() {
            self.checkpoint();
            self.st.model.select_reamer(&entry);
            self.picker = None;
            self.changed();
        }
    }
    pub(super) fn pick_id(&mut self, i: usize) {
        let Some(p) = &self.picker else { return };
        let Some(value) = p.ids.visible().get(i).map(|r| r.id_in()) else { return };
        self.checkpoint();
        self.st.model.id_bushing = value;
        self.st.model.recompute();
        self.picker = None;
        self.changed();
    }
    pub(super) fn pick_friction(&mut self, i: usize) {
        let Some(f) = FRICTION_TYPICALS.get(i) else { return };
        self.checkpoint();
        self.st.model.friction = f.value;
        self.st.model.recompute();
        self.picker = None;
        self.changed();
    }
}
