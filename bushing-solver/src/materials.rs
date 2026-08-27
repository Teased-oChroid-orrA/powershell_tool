//! Material library, ported verbatim from engineering.toolbox's
//! `src/lib/core/bushing/materials.ts` - same values, same ids, same
//! "intentionally mirrors the legacy web-tool baseline" provenance note.
//! Units: `e_ksi` in ksi, strengths in ksi, `alpha_u_f` in
//! microstrain/°F.

#[derive(Debug, Clone, Copy)]
pub struct Material {
    pub id: &'static str,
    pub name: &'static str,
    pub e_ksi: f64,
    pub sy_ksi: f64,
    pub fbru_ksi: f64,
    pub fsu_ksi: f64,
    pub ftu_ksi: f64,
    pub nu: f64,
    pub alpha_u_f: f64,
}

pub static MATERIALS: &[Material] = &[
    Material { id: "al7075", name: "Al 7075-T6 (typical)", e_ksi: 10300.0, sy_ksi: 70.0, fbru_ksi: 121.0, fsu_ksi: 48.0, ftu_ksi: 77.0, nu: 0.33, alpha_u_f: 12.8 },
    Material { id: "al2024", name: "Al 2024-T3 (typical)", e_ksi: 10500.0, sy_ksi: 47.0, fbru_ksi: 98.0, fsu_ksi: 41.0, ftu_ksi: 64.0, nu: 0.33, alpha_u_f: 12.5 },
    Material { id: "steel", name: "Steel (typical)", e_ksi: 29000.0, sy_ksi: 120.0, fbru_ksi: 160.0, fsu_ksi: 70.0, ftu_ksi: 150.0, nu: 0.30, alpha_u_f: 6.5 },
    Material { id: "al2024t3", name: "Al 2024-T3 Bare", e_ksi: 10500.0, sy_ksi: 47.0, fbru_ksi: 98.0, fsu_ksi: 41.0, ftu_ksi: 64.0, nu: 0.33, alpha_u_f: 12.5 },
    Material { id: "al7075t6", name: "Al 7075-T6 Clad", e_ksi: 10300.0, sy_ksi: 70.0, fbru_ksi: 121.0, fsu_ksi: 48.0, ftu_ksi: 77.0, nu: 0.33, alpha_u_f: 12.8 },
    Material { id: "al7050", name: "Al 7050-T7451", e_ksi: 10300.0, sy_ksi: 68.0, fbru_ksi: 118.0, fsu_ksi: 46.0, ftu_ksi: 76.0, nu: 0.33, alpha_u_f: 12.8 },
    Material { id: "ti6al4v", name: "Ti-6Al-4V Gr.5", e_ksi: 16000.0, sy_ksi: 126.0, fbru_ksi: 215.0, fsu_ksi: 76.0, ftu_ksi: 130.0, nu: 0.34, alpha_u_f: 4.9 },
    Material { id: "steel4340", name: "Steel 4340", e_ksi: 29000.0, sy_ksi: 217.0, fbru_ksi: 360.0, fsu_ksi: 130.0, ftu_ksi: 260.0, nu: 0.29, alpha_u_f: 6.6 },
    Material { id: "ph157mo", name: "15-7 Mo PH", e_ksi: 29000.0, sy_ksi: 185.0, fbru_ksi: 300.0, fsu_ksi: 115.0, ftu_ksi: 200.0, nu: 0.29, alpha_u_f: 6.3 },
    Material { id: "ph174", name: "17-4 PH H1025", e_ksi: 28500.0, sy_ksi: 145.0, fbru_ksi: 240.0, fsu_ksi: 105.0, ftu_ksi: 160.0, nu: 0.29, alpha_u_f: 6.0 },
    Material { id: "inconel718", name: "Inconel 718", e_ksi: 29700.0, sy_ksi: 150.0, fbru_ksi: 260.0, fsu_ksi: 100.0, ftu_ksi: 180.0, nu: 0.29, alpha_u_f: 7.1 },
    Material { id: "inconel625", name: "Inconel 625", e_ksi: 30100.0, sy_ksi: 60.0, fbru_ksi: 180.0, fsu_ksi: 75.0, ftu_ksi: 120.0, nu: 0.29, alpha_u_f: 7.3 },
    Material { id: "cfrp_qi", name: "Carbon/Epoxy (QI)", e_ksi: 8500.0, sy_ksi: 80.0, fbru_ksi: 80.0, fsu_ksi: 45.0, ftu_ksi: 90.0, nu: 0.30, alpha_u_f: 1.5 },
    Material { id: "washer_steel", name: "Washer (Steel)", e_ksi: 29000.0, sy_ksi: 30.0, fbru_ksi: 90.0, fsu_ksi: 30.0, ftu_ksi: 45.0, nu: 0.29, alpha_u_f: 6.5 },
    Material { id: "washer_al", name: "Washer (Al)", e_ksi: 10300.0, sy_ksi: 30.0, fbru_ksi: 60.0, fsu_ksi: 20.0, ftu_ksi: 40.0, nu: 0.33, alpha_u_f: 12.8 },
    Material { id: "bronze", name: "Al-Bronze (C630)", e_ksi: 17000.0, sy_ksi: 50.0, fbru_ksi: 130.0, fsu_ksi: 45.0, ftu_ksi: 90.0, nu: 0.34, alpha_u_f: 9.0 },
    Material { id: "beryllium", name: "Be-Copper", e_ksi: 18000.0, sy_ksi: 95.0, fbru_ksi: 140.0, fsu_ksi: 70.0, ftu_ksi: 100.0, nu: 0.30, alpha_u_f: 9.4 },
];

/// Falls back to the first entry (`al7075`) on an unknown id, matching
/// the TS original's `getMaterial`'s own fallback behavior exactly -
/// never a hard error for a bad/missing material selection.
pub fn get_material(id: &str) -> &'static Material {
    MATERIALS.iter().find(|m| m.id == id).unwrap_or(&MATERIALS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seventeen_materials_present_matching_the_ts_source() {
        assert_eq!(MATERIALS.len(), 17);
    }

    #[test]
    fn get_material_falls_back_to_first_entry_on_unknown_id() {
        assert_eq!(get_material("does-not-exist").id, "al7075");
    }

    #[test]
    fn known_material_properties_match_the_ts_source_exactly() {
        let bronze = get_material("bronze");
        assert_eq!(bronze.e_ksi, 17000.0);
        assert_eq!(bronze.nu, 0.34);
        assert_eq!(bronze.alpha_u_f, 9.0);
    }
}
