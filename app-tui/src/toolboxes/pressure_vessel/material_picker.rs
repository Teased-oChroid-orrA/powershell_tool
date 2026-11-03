//! The Pressure Vessel window's material picker logic (`gpu_viewer/editor/vessel/picker.rs` draws it): the catalog
//! filter (`visible_indices`) and the "add new material" form - Name/E/Sy/Ftu/Poisson's ratio/Thermal expansion plus
//! the checks that validate all six at once before `PressureVesselModel::add_custom_material`.
//!
//! Deliberately reads the model's live `material_catalog()` on every call rather than holding a snapshot, so a
//! material added mid-session appears in the very same list without a second synchronization step.

use super::model::PressureVesselModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMaterialField {
    Name,
    E,
    Sy,
    Ftu,
    Nu,
    Alpha,
}

pub const ADD_MATERIAL_FIELDS: [AddMaterialField; 6] =
    [AddMaterialField::Name, AddMaterialField::E, AddMaterialField::Sy, AddMaterialField::Ftu, AddMaterialField::Nu, AddMaterialField::Alpha];

impl AddMaterialField {
    pub fn label(self) -> &'static str {
        match self {
            AddMaterialField::Name => "Name",
            AddMaterialField::E => "E (ksi)",
            AddMaterialField::Sy => "Yield Strength Sy (ksi)",
            AddMaterialField::Ftu => "Ultimate Strength Ftu (ksi)",
            AddMaterialField::Nu => "Poisson's Ratio",
            AddMaterialField::Alpha => "Thermal Expansion (\u{b5}strain/\u{b0}F)",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AddMaterialForm {
    pub name: String,
    pub e_ksi: String,
    pub sy_ksi: String,
    pub ftu_ksi: String,
    pub nu: String,
    pub alpha_u_f: String,
    pub error: Option<String>,
}

impl AddMaterialForm {
    pub(crate) fn field_text(&self, field: AddMaterialField) -> &str {
        match field {
            AddMaterialField::Name => &self.name,
            AddMaterialField::E => &self.e_ksi,
            AddMaterialField::Sy => &self.sy_ksi,
            AddMaterialField::Ftu => &self.ftu_ksi,
            AddMaterialField::Nu => &self.nu,
            AddMaterialField::Alpha => &self.alpha_u_f,
        }
    }

    pub(crate) fn set_field_text(&mut self, field: AddMaterialField, text: String) {
        match field {
            AddMaterialField::Name => self.name = text,
            AddMaterialField::E => self.e_ksi = text,
            AddMaterialField::Sy => self.sy_ksi = text,
            AddMaterialField::Ftu => self.ftu_ksi = text,
            AddMaterialField::Nu => self.nu = text,
            AddMaterialField::Alpha => self.alpha_u_f = text,
        }
    }

    /// Parses every field and checks it against a physically sane range -
    /// `Err` leaves the form open with the message shown inline, `Ok`
    /// returns the six values ready for `PressureVesselModel::add_custom_material`.
    /// Deliberately permissive on exact bounds (this is a user-entered
    /// material, not a catalog entry curated against a spec sheet) but
    /// strict on "this text isn't even a usable number" and on signs/ranges
    /// that would silently produce nonsense elsewhere (e.g. Poisson's ratio
    /// outside its physically possible range would make
    /// `pressure-vessel-solver`'s own formulas divide by a near-zero or
    /// negative quantity).
    pub(crate) fn validate(&self) -> Result<(String, f64, f64, f64, f64, f64), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name must not be blank".to_string());
        }
        let e_ksi = parse_positive(&self.e_ksi, "E")?;
        let sy_ksi = parse_positive(&self.sy_ksi, "Yield strength")?;
        let ftu_ksi = parse_positive(&self.ftu_ksi, "Ultimate strength")?;
        let nu: f64 = self.nu.trim().parse().map_err(|_| "Poisson's ratio must be a number".to_string())?;
        if !(0.0..0.5).contains(&nu) {
            return Err("Poisson's ratio must be between 0 and 0.5".to_string());
        }
        let alpha_u_f: f64 = self.alpha_u_f.trim().parse().map_err(|_| "Thermal expansion must be a number".to_string())?;
        if !alpha_u_f.is_finite() || alpha_u_f < 0.0 {
            return Err("Thermal expansion must be \u{2265} 0".to_string());
        }
        Ok((name.to_string(), e_ksi, sy_ksi, ftu_ksi, nu, alpha_u_f))
    }
}

fn parse_positive(text: &str, label: &str) -> Result<f64, String> {
    let value: f64 = text.trim().parse().map_err(|_| format!("{label} must be a number"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{label} must be > 0"));
    }
    Ok(value)
}

/// Catalog entries (indices into `model.material_catalog()`) whose text matches `query` (case-insensitive, trimmed);
/// a blank query lists them all.
pub(crate) fn visible_indices(model: &PressureVesselModel, query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    let catalog = model.material_catalog();
    if needle.is_empty() {
        (0..catalog.len()).collect()
    } else {
        catalog.iter().enumerate().filter(|(_, m)| crate::widgets::material_detail::matches(m, &needle)).map(|(i, _)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(name: &str, nu: &str) -> AddMaterialForm {
        AddMaterialForm { name: name.into(), e_ksi: "12000".into(), sy_ksi: "55".into(), ftu_ksi: "65".into(), nu: nu.into(), alpha_u_f: "7.0".into(), ..Default::default() }
    }

    #[test]
    fn a_blank_name_fails_validation() {
        assert!(form("  ", "0.3").validate().is_err());
    }

    #[test]
    fn valid_fields_validate_to_their_values() {
        assert_eq!(form(" Test Alloy ", "0.31").validate(), Ok(("Test Alloy".to_string(), 12000.0, 55.0, 65.0, 0.31, 7.0)));
    }

    #[test]
    fn an_out_of_range_poissons_ratio_or_a_non_positive_modulus_is_rejected() {
        assert!(form("Bad", "0.9").validate().is_err());
        assert!(AddMaterialForm { e_ksi: "0".into(), ..form("Bad", "0.3") }.validate().is_err());
    }

    #[test]
    fn a_blank_query_lists_the_whole_catalog_and_text_narrows_it() {
        let model = PressureVesselModel::default();
        let all = visible_indices(&model, "  ");
        assert_eq!(all.len(), model.material_catalog().len());
        let name = model.material_catalog()[0].name.to_lowercase();
        let some = visible_indices(&model, &name);
        assert!(some.contains(&0) && some.len() <= all.len());
        assert!(visible_indices(&model, "no material is called this").is_empty());
    }
}
