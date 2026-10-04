//! Material library, ported verbatim from engineering.toolbox's
//! `src/lib/core/bushing/materials.ts` - same values, same ids, same
//! "intentionally mirrors the legacy web-tool baseline" provenance note.
//! Units: `e_ksi` in ksi, strengths in ksi, `alpha_u_f` in
//! microstrain/°F.

/// A property that depends on grain direction. `0.0` = not given.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Dir {
    pub l: f64,
    pub lt: f64,
    pub st: f64,
}

/// Everything else a MIL-HDBK-5J table says about a condition, kept verbatim
/// so nothing is lost when it is reduced to the solver `Material` fields.
/// Strengths in ksi, `elong` in %, densities in lb/in^3; `0.0` = not given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialExtras {
    pub alloy: &'static str,
    pub group: &'static str,
    pub form: &'static str,
    pub temper: &'static str,
    pub spec: &'static str,
    /// Thickness range as printed, in inches (empty = not applicable).
    pub thickness: &'static str,
    /// Statistical basis: "A", "B", "S" (specification minimum) or "".
    pub basis: &'static str,
    pub clad: bool,
    pub table: &'static str,
    pub caption: &'static str,
    pub ftu: Dir,
    pub fty: Dir,
    pub fcy: Dir,
    pub fsu: Dir,
    pub elong: Dir,
    pub fbru_e20_ksi: f64,
    pub fbru_e15_ksi: f64,
    pub fbry_e20_ksi: f64,
    pub fbry_e15_ksi: f64,
    pub ec_ksi: f64,
    pub g_ksi: f64,
    pub density_lb_in3: f64,
    /// Which solver fields the handbook did not give and were filled with
    /// group-typical values (always includes `alpha`).
    pub estimated: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Material {
    pub id: &'static str,
    pub name: &'static str,
    pub e_ksi: f64,
    pub sy_ksi: f64,
    pub fbru_ksi: f64,
    /// MMPDS (or other substantiated) bearing ultimate at e/D = 1.5, ksi;
    /// `0.0` = not tabulated. `fbru_ksi` is taken as the e/D = 2.0 value.
    pub fbru_e15_ksi: f64,
    pub fsu_ksi: f64,
    pub ftu_ksi: f64,
    pub nu: f64,
    pub alpha_u_f: f64,
    /// Full handbook record when this material came from `handbook::HANDBOOK`.
    pub extra: Option<&'static MaterialExtras>,
}

impl Default for Material {
    /// `MATERIALS[0]` (al7075) - matches [`get_material`]'s own fallback on
    /// an unknown id, so a caller building a `Material`-typed input via
    /// `..Default::default()` gets the exact same material a bad/missing id
    /// string used to silently fall back to.
    fn default() -> Self {
        MATERIALS[0]
    }
}

pub static MATERIALS: &[Material] = &[
    Material { id: "al7075", name: "Al 7075-T6 (typical)", e_ksi: 10300.0, sy_ksi: 70.0, fbru_ksi: 121.0, fbru_e15_ksi: 0.0, fsu_ksi: 48.0, ftu_ksi: 77.0, nu: 0.33, alpha_u_f: 12.8, extra: None },
    Material { id: "al2024", name: "Al 2024-T3 (typical)", e_ksi: 10500.0, sy_ksi: 47.0, fbru_ksi: 98.0, fbru_e15_ksi: 0.0, fsu_ksi: 41.0, ftu_ksi: 64.0, nu: 0.33, alpha_u_f: 12.5, extra: None },
    Material { id: "steel", name: "Steel (typical)", e_ksi: 29000.0, sy_ksi: 120.0, fbru_ksi: 160.0, fbru_e15_ksi: 0.0, fsu_ksi: 70.0, ftu_ksi: 150.0, nu: 0.30, alpha_u_f: 6.5, extra: None },
    Material { id: "al2024t3", name: "Al 2024-T3 Bare", e_ksi: 10500.0, sy_ksi: 47.0, fbru_ksi: 98.0, fbru_e15_ksi: 0.0, fsu_ksi: 41.0, ftu_ksi: 64.0, nu: 0.33, alpha_u_f: 12.5, extra: None },
    Material { id: "al7075t6", name: "Al 7075-T6 Clad", e_ksi: 10300.0, sy_ksi: 70.0, fbru_ksi: 121.0, fbru_e15_ksi: 0.0, fsu_ksi: 48.0, ftu_ksi: 77.0, nu: 0.33, alpha_u_f: 12.8, extra: None },
    Material { id: "al7050", name: "Al 7050-T7451", e_ksi: 10300.0, sy_ksi: 68.0, fbru_ksi: 118.0, fbru_e15_ksi: 0.0, fsu_ksi: 46.0, ftu_ksi: 76.0, nu: 0.33, alpha_u_f: 12.8, extra: None },
    Material { id: "ti6al4v", name: "Ti-6Al-4V Gr.5", e_ksi: 16000.0, sy_ksi: 126.0, fbru_ksi: 215.0, fbru_e15_ksi: 0.0, fsu_ksi: 76.0, ftu_ksi: 130.0, nu: 0.34, alpha_u_f: 4.9, extra: None },
    Material { id: "steel4340", name: "Steel 4340", e_ksi: 29000.0, sy_ksi: 217.0, fbru_ksi: 360.0, fbru_e15_ksi: 0.0, fsu_ksi: 130.0, ftu_ksi: 260.0, nu: 0.29, alpha_u_f: 6.6, extra: None },
    Material { id: "ph157mo", name: "15-7 Mo PH", e_ksi: 29000.0, sy_ksi: 185.0, fbru_ksi: 300.0, fbru_e15_ksi: 0.0, fsu_ksi: 115.0, ftu_ksi: 200.0, nu: 0.29, alpha_u_f: 6.3, extra: None },
    Material { id: "ph174", name: "17-4 PH H1025", e_ksi: 28500.0, sy_ksi: 145.0, fbru_ksi: 240.0, fbru_e15_ksi: 0.0, fsu_ksi: 105.0, ftu_ksi: 160.0, nu: 0.29, alpha_u_f: 6.0, extra: None },
    Material { id: "inconel718", name: "Inconel 718", e_ksi: 29700.0, sy_ksi: 150.0, fbru_ksi: 260.0, fbru_e15_ksi: 0.0, fsu_ksi: 100.0, ftu_ksi: 180.0, nu: 0.29, alpha_u_f: 7.1, extra: None },
    Material { id: "inconel625", name: "Inconel 625", e_ksi: 30100.0, sy_ksi: 60.0, fbru_ksi: 180.0, fbru_e15_ksi: 0.0, fsu_ksi: 75.0, ftu_ksi: 120.0, nu: 0.29, alpha_u_f: 7.3, extra: None },
    Material { id: "cfrp_qi", name: "Carbon/Epoxy (QI)", e_ksi: 8500.0, sy_ksi: 80.0, fbru_ksi: 80.0, fbru_e15_ksi: 0.0, fsu_ksi: 45.0, ftu_ksi: 90.0, nu: 0.30, alpha_u_f: 1.5, extra: None },
    Material { id: "washer_steel", name: "Washer (Steel)", e_ksi: 29000.0, sy_ksi: 30.0, fbru_ksi: 90.0, fbru_e15_ksi: 0.0, fsu_ksi: 30.0, ftu_ksi: 45.0, nu: 0.29, alpha_u_f: 6.5, extra: None },
    Material { id: "washer_al", name: "Washer (Al)", e_ksi: 10300.0, sy_ksi: 30.0, fbru_ksi: 60.0, fbru_e15_ksi: 0.0, fsu_ksi: 20.0, ftu_ksi: 40.0, nu: 0.33, alpha_u_f: 12.8, extra: None },
    Material { id: "bronze", name: "Al-Bronze (C630)", e_ksi: 17000.0, sy_ksi: 50.0, fbru_ksi: 130.0, fbru_e15_ksi: 0.0, fsu_ksi: 45.0, ftu_ksi: 90.0, nu: 0.34, alpha_u_f: 9.0, extra: None },
    Material { id: "beryllium", name: "Be-Copper", e_ksi: 18000.0, sy_ksi: 95.0, fbru_ksi: 140.0, fbru_e15_ksi: 0.0, fsu_ksi: 70.0, ftu_ksi: 100.0, nu: 0.30, alpha_u_f: 9.4, extra: None },
];

/// Every built-in material: the small curated table first (stable indices),
/// then all MIL-HDBK-5J conditions from [`crate::handbook`].
pub fn builtin_catalog() -> impl Iterator<Item = &'static Material> {
    MATERIALS.iter().chain(crate::handbook::HANDBOOK.iter())
}

/// Length of [`builtin_catalog`].
pub fn builtin_len() -> usize {
    MATERIALS.len() + crate::handbook::HANDBOOK.len()
}

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

    #[test]
    fn the_handbook_has_every_published_condition_with_unique_ids_and_names() {
        use crate::handbook::HANDBOOK;
        // 1,379 published conditions; 4 have no tension strength in the source and are omitted.
        assert_eq!(HANDBOOK.len(), 1375);
        let ids: std::collections::HashSet<_> = HANDBOOK.iter().map(|m| m.id).collect();
        let names: std::collections::HashSet<_> = HANDBOOK.iter().map(|m| m.name).collect();
        assert_eq!((ids.len(), names.len()), (1375, 1375));
        assert_eq!(builtin_len(), MATERIALS.len() + 1375);
        assert_eq!(builtin_catalog().count(), builtin_len());
    }

    #[test]
    fn every_handbook_material_is_usable_by_the_solvers() {
        for m in crate::handbook::HANDBOOK.iter() {
            assert!(m.e_ksi > 1000.0 && m.sy_ksi > 0.0 && m.ftu_ksi > 0.0 && m.alpha_u_f > 0.0, "{}", m.name);
            assert!((0.2..0.5).contains(&m.nu), "{}: nu {}", m.name, m.nu);
            let x = m.extra.expect("handbook entries carry their full record");
            assert!(x.estimated.split(", ").any(|e| e == "alpha"), "alpha is never in the tables and must be flagged: {}", m.name);
            assert!(m.fbru_e15_ksi == 0.0 || m.fbru_ksi == 0.0 || m.fbru_e15_ksi <= m.fbru_ksi + 1e-9, "{}: Fbru(1.5) above Fbru(2.0)", m.name);
        }
    }

    #[test]
    fn a_known_handbook_condition_matches_the_published_table() {
        // AISI 1025 annealed sheet (MIL-HDBK-5J Table 2.2.1.0(b)): Ftu 55, Fty 36, Fsu 35,
        // Fbru(e/D 2.0) 90 ksi, E 29 Msi, nu 0.32, density 0.284 lb/in^3.
        let m = crate::handbook::HANDBOOK.iter().find(|m| m.id == "mh5:2.2.1.0b#0").unwrap();
        assert_eq!((m.ftu_ksi, m.sy_ksi, m.fsu_ksi, m.fbru_ksi, m.e_ksi, m.nu), (55.0, 36.0, 35.0, 90.0, 29000.0, 0.32));
        assert_eq!(m.extra.unwrap().density_lb_in3, 0.284);
        assert_eq!(m.fbru_e15_ksi, 0.0, "table gives no e/D 1.5 value");
    }

    #[test]
    fn most_aluminum_conditions_carry_the_e_over_d_one_and_a_half_bearing_value() {
        let with: usize = crate::handbook::HANDBOOK.iter().filter(|m| m.fbru_e15_ksi > 0.0).count();
        assert!(with >= 900, "{with}");
    }
}
