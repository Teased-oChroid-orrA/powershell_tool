//! The material picker as a value card: a search field over a short list of the catalog's best matches, and a form for
//! a material of one's own. The filter, the form's fields and their checks, and what adding sets are the toolbox
//! model's (`material_picker::{visible_indices, AddMaterialForm}`, `PressureVesselModel::add_custom_material`); an
//! added material is kept between runs (`persistence::save_at`).
use super::{VesselAct, VesselSession};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    ui::{CardRow, ValueCard},
};
use crate::toolboxes::pressure_vessel::{
    material_picker::{visible_indices, AddMaterialField, AddMaterialForm, ADD_MATERIAL_FIELDS},
    model::is_custom,
    persistence,
};

/// Entries the picker lists at once; the search narrows the rest.
const SHOWN: usize = 12;

#[derive(Default)]
pub struct Picker {
    pub query: String,
    /// The form for a material of one's own, when it is open instead of the list.
    pub add: Option<AddMaterialForm>,
}

fn act(a: VesselAct) -> UiAction {
    UiAction::Editor(EditorAction::Vessel(a))
}

/// The form's text fields, in order: field `i` of the card is `ADD_MATERIAL_FIELDS[i - 1]`.
fn form_fields() -> impl Iterator<Item = AddMaterialField> {
    ADD_MATERIAL_FIELDS.into_iter()
}

impl VesselSession {
    pub(super) fn open_picker(&mut self) {
        self.prompt = None;
        self.picker = Some(Picker::default());
    }

    pub(super) fn picker_card(&self) -> Option<ValueCard> {
        let p = self.picker.as_ref()?;
        if let Some(form) = &p.add {
            let mut rows: Vec<CardRow> = form_fields().enumerate().map(|(k, f)| CardRow::Field { label: f.label().into(), text: form.field_text(f).to_string(), index: k + 1, unit: String::new() }).collect();
            rows.push(CardRow::Text("Kept for the next run too.".into()));
            if let Some(e) = &form.error {
                rows.push(CardRow::Text(format!("Not added: {e}.")));
            }
            return Some(ValueCard { title: "New material".into(), width: 400.0, tag: "custom".into(), rows, apply_label: "Add material".into(), apply: act(VesselAct::AddMaterial), cancel: act(VesselAct::ListMaterials) });
        }
        let m = &self.model;
        let current = m.material_index;
        let catalog = m.material_catalog();
        let hits = visible_indices(m, &p.query);
        let more = hits.len().saturating_sub(SHOWN);
        // The current material first so it is never lost behind the search.
        let mut shown: Vec<usize> = hits.iter().copied().take(SHOWN).collect();
        if p.query.trim().is_empty() && !shown.contains(&current) {
            shown.insert(0, current);
            shown.truncate(SHOWN);
        }
        let mut rows = vec![CardRow::Field { label: "Search".into(), text: p.query.clone(), index: 0, unit: String::new() }];
        for i in shown {
            let mat = catalog[i];
            let detail = if is_custom(mat) {
                format!("custom: E {:.0} ksi, Fy {:.0} ksi", mat.e_ksi, mat.sy_ksi)
            } else {
                mat.extra.map_or(format!("E {:.0} ksi, Fy {:.0} ksi", mat.e_ksi, mat.sy_ksi), |x| format!("{} {} {}", x.alloy, x.temper, x.form).split_whitespace().collect::<Vec<_>>().join(" "))
            };
            rows.push(CardRow::Choice { label: mat.name.to_string(), detail, action: act(VesselAct::PickMaterial(i)), current: i == current });
        }
        if more > 0 {
            rows.push(CardRow::Text(format!("{more} more: type to narrow the list.")));
        }
        rows.push(CardRow::Choice { label: "Add a material".into(), detail: "one that is not in the catalog".into(), action: act(VesselAct::NewMaterial), current: false });
        Some(ValueCard { title: "Material".into(), width: 400.0, tag: "catalog".into(), rows, apply_label: "Close".into(), apply: act(VesselAct::ClosePicker), cancel: act(VesselAct::ClosePicker) })
    }

    pub(super) fn picker_field(&mut self, index: usize, text: String) {
        let Some(p) = &mut self.picker else { return };
        match (&mut p.add, index) {
            (None, 0) => p.query = text,
            (Some(form), i) if i > 0 => {
                if let Some(f) = form_fields().nth(i - 1) {
                    form.set_field_text(f, text);
                    form.error = None;
                }
            }
            _ => {}
        }
    }

    pub(super) fn pick_material(&mut self, i: usize) {
        if i == self.model.material_index || i >= self.model.material_catalog().len() {
            self.picker = None;
            return;
        }
        self.checkpoint();
        self.model.select_material(i);
        self.picker = None;
        self.changed();
    }

    /// Open (`true`) or leave (`false`) the form for a material of one's own.
    pub(super) fn new_material(&mut self, open: bool) {
        if let Some(p) = &mut self.picker {
            p.add = open.then(AddMaterialForm::default);
        }
    }

    pub(super) fn add_material(&mut self) {
        let Some(form) = self.picker.as_mut().and_then(|p| p.add.as_mut()) else { return };
        match form.validate() {
            Err(e) => form.error = Some(e),
            Ok((name, e_ksi, sy_ksi, ftu_ksi, nu, alpha)) => {
                self.checkpoint();
                self.model.add_custom_material(name.clone(), e_ksi, sy_ksi, ftu_ksi, nu, alpha);
                if let Some(library) = &self.library {
                    persistence::save_at(library, &self.model.persisted_custom_materials());
                }
                self.picker = None;
                self.changed();
                self.status = format!("{name} added to the catalog and selected.");
            }
        }
    }
}
