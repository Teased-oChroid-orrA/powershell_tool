//! Material catalog filtering for the native Bushing window.
use super::model::BushingModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialTarget {
    Housing,
    Bushing,
}

#[derive(Default)]
pub struct MaterialPickerState {
    pub filter_text: String,
}

impl MaterialPickerState {
    pub(crate) fn visible_indices(&self, model: &BushingModel) -> Vec<usize> {
        let needle = self.filter_text.trim().to_lowercase();
        let catalog = model.material_catalog();
        if needle.is_empty() {
            (0..catalog.len()).collect()
        } else {
            catalog
                .iter()
                .enumerate()
                .filter(|(_, material)| crate::widgets::material_detail::matches(material, &needle))
                .map(|(i, _)| i)
                .collect()
        }
    }
}
