//! Item 3: test-based allowables. No stress field - the tabulated bearing
//! ultimate `Fbru` already folds the edge-distance effect in (it is a test
//! value for a stated `e/D`), plus the classical inclined-plane shear-out
//! rule. Both are closed form.
//!
//! * Bearing: `P / (D t) <= Fbru(e/D)`. The material table carries one
//!   `Fbru` (taken to be the `e/D = 2.0` value, the usual tabulation point).
//!   Below `e/D = 2.0` the allowable is only used if the caller supplies the
//!   `e/D = 1.5` value ([`AllowableModel::fbru_e15`]) - then it is linearly
//!   interpolated; below `e/D = 1.5` no allowable exists. Nothing here is
//!   guessed: with no `e/D = 1.5` value the check simply requires `e/D >= 2`.
//! * Shear-out: two planes inclined `theta` (default 40 deg) to the load
//!   line, `P + p D t <= 2 Fsu t (e - (D/2) cos theta)`. The fit pressure `p`
//!   pushes the loaded half of the bore toward the edge with `p D t` (the
//!   same strip equilibrium the stress models verify), so the fit consumes
//!   shear capacity at 100%. Bearing is not reduced by the fit: the radial
//!   pressure confines the bore, so only the edge-failure rule is charged.

use crate::model::{EdgeModel, Response};
use crate::types::{margin_of, Geometry, Loads, Mode, ModeMargin, Strengths};

#[derive(Default)]
pub struct AllowableModel {
    /// `Fbru` at `e/D = 1.5`, psi, if known for the housing material.
    pub fbru_e15: Option<f64>,
}

struct AllowableResponse {
    geom: Geometry,
    mat: Strengths,
    fbru_e15: Option<f64>,
}

impl AllowableResponse {
    /// Tabulated bearing allowable at this `e/D`, or `None` if none applies.
    fn fbru_at(&self) -> Option<f64> {
        let ed = self.geom.edge / (2.0 * self.geom.bore_radius);
        if ed >= 2.0 {
            return Some(self.mat.fbru);
        }
        let f15 = self.fbru_e15?;
        if ed >= 1.5 {
            Some(f15 + (self.mat.fbru - f15) * (ed - 1.5) / 0.5)
        } else {
            None
        }
    }
}

impl Response for AllowableResponse {
    fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        let d = 2.0 * self.geom.bore_radius;
        let ed = self.geom.edge / d;
        if self.fbru_at().is_none() {
            notes.push(if ed < 1.5 {
                format!("no Fbru allowable for e/D {ed:.2}: MMPDS bearing values stop at e/D 1.5 (below that needs test substantiation)")
            } else {
                format!("no Fbru allowable for e/D {ed:.2}: the material's Fbru is the e/D = 2.0 value and it has no e/D = 1.5 value (add it in the material library), so bearing is not allowed below e/D 2.0")
            });
        }
        let td = self.geom.thickness / d;
        if !(0.25..=0.5).contains(&td) {
            notes.push(format!("t/D = {td:.2}: MMPDS bearing allowables are tabulated for 0.25 <= t/D <= 0.50"));
        }
        notes
    }

    fn margins(&self, loads: &Loads, strength_scale: f64) -> Vec<ModeMargin> {
        let g = &self.geom;
        let d = 2.0 * g.bore_radius;
        let bearing_stress = loads.pin_load / (d * g.thickness);
        let bearing = match self.fbru_at() {
            Some(f) => margin_of(f * strength_scale, bearing_stress),
            // Outside the tabulated range: not allowed, however small the load.
            None => -1.0,
        };
        let th = g.plane_angle_deg.to_radians();
        let shear_len = (g.edge - g.bore_radius * th.cos()).max(0.0);
        let shear_capacity = 2.0 * self.mat.fsu * strength_scale * g.thickness * shear_len;
        let shear = margin_of(shear_capacity, loads.pin_load + loads.fit_pressure * d * g.thickness);
        vec![ModeMargin { mode: Mode::Bearing, margin: bearing }, ModeMargin { mode: Mode::ShearOut, margin: shear }]
    }
}

impl EdgeModel for AllowableModel {
    fn id(&self) -> &'static str {
        "allowable"
    }
    fn label(&self) -> &'static str {
        "Tabulated allowables"
    }
    fn is_field_model(&self) -> bool {
        false
    }
    fn respond(&self, geom: &Geometry, mat: &Strengths, _fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        if !(geom.bore_radius > 0.0 && geom.thickness > 0.0 && geom.edge > 0.0) {
            return Err("degenerate geometry".to_string());
        }
        // A caller-supplied value overrides the material table's.
        let fbru_e15 = self.fbru_e15.or((mat.fbru_e15 > 0.0).then_some(mat.fbru_e15));
        Ok(Box::new(AllowableResponse { geom: *geom, mat: *mat, fbru_e15 }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_core::materials::get_material;

    fn geom(e_over_d: f64) -> Geometry {
        Geometry { bore_radius: 0.25, edge: e_over_d * 0.5, thickness: 0.5, plate_far: 5.0, plate_half_height: 5.0, plane_angle_deg: 40.0 }
    }

    fn margin(r: &dyn Response, mode: Mode, load: f64) -> f64 {
        r.margins(&Loads { fit_pressure: 0.0, pin_load: load }, 1.0).into_iter().find(|m| m.mode == mode).unwrap().margin
    }

    #[test]
    fn bearing_needs_e_over_d_two_without_an_e15_allowable() {
        let mat = Strengths::from_material(get_material("al7075"));
        let m = AllowableModel::default();
        assert!(margin(&*m.respond(&geom(2.0), &mat, 0.0).unwrap(), Mode::Bearing, 1000.0) > 0.0);
        assert_eq!(margin(&*m.respond(&geom(1.9), &mat, 0.0).unwrap(), Mode::Bearing, 1000.0), -1.0);
    }

    #[test]
    fn an_e15_allowable_is_interpolated_between_one_and_a_half_and_two() {
        let mat = Strengths::from_material(get_material("al7075"));
        let m = AllowableModel { fbru_e15: Some(100e3) };
        // e/D = 1.75: Fbru = 100 + (121-100)/2 = 110.5 ksi; P/(D t) = 4000 psi.
        let got = margin(&*m.respond(&geom(1.75), &mat, 0.0).unwrap(), Mode::Bearing, 1000.0);
        assert!((got - (110_500.0 / 4000.0 - 1.0)).abs() < 1e-9, "{got}");
        assert_eq!(margin(&*m.respond(&geom(1.4), &mat, 0.0).unwrap(), Mode::Bearing, 1000.0), -1.0);
    }

    #[test]
    fn a_missing_allowable_is_explained_in_the_notes() {
        let mat = Strengths::from_material(get_material("al7075"));
        let m = AllowableModel::default();
        let in_range = |ed: f64| Geometry { thickness: 0.2, ..geom(ed) }; // t/D = 0.4
        assert!(m.respond(&in_range(1.5), &mat, 0.0).unwrap().notes().iter().any(|n| n.contains("no Fbru allowable")));
        assert!(m.respond(&in_range(2.5), &mat, 0.0).unwrap().notes().is_empty());
        // t/D outside the MMPDS range is called out.
        let thin = Geometry { thickness: 0.05, ..geom(2.5) }; // t/D = 0.1
        assert!(m.respond(&thin, &mat, 0.0).unwrap().notes().iter().any(|n| n.contains("t/D")));
    }

    #[test]
    fn the_material_tables_e15_value_is_used_without_any_override() {
        let mut mat = Strengths::from_material(get_material("al7075"));
        mat.fbru_e15 = 100e3;
        let g = Geometry { thickness: 0.2, ..geom(1.75) }; // t/D = 0.4, inside the MMPDS range
        let r = AllowableModel::default().respond(&g, &mat, 0.0).unwrap();
        // e/D 1.75 midway between 1.5 (100 ksi) and 2.0 (Fbru): interpolated; P/(D t) = 10 ksi.
        let expected = (100_000.0 + (mat.fbru - 100_000.0) / 2.0) / 10_000.0 - 1.0;
        assert!((margin(&*r, Mode::Bearing, 1000.0) - expected).abs() < 1e-9);
        assert!(r.notes().is_empty());
    }

    #[test]
    fn the_fit_consumes_shear_out_capacity_at_full_pressure_but_not_bearing() {
        let mat = Strengths::from_material(get_material("al7075"));
        let g = geom(2.0);
        let r = AllowableModel::default().respond(&g, &mat, 0.0).unwrap();
        let cap = 2.0 * 48_000.0 * 0.5 * (1.0 - 0.25 * 40f64.to_radians().cos());
        let at = |fit: f64, mode| r.margins(&Loads { fit_pressure: fit, pin_load: 10_000.0 }, 1.0).into_iter().find(|m| m.mode == mode).unwrap().margin;
        // p D t = 8000 psi * 0.5 in * 0.5 in = 2000 lbf of extra shear load.
        assert!((at(8000.0, Mode::ShearOut) - (cap / 12_000.0 - 1.0)).abs() < 1e-9);
        assert_eq!(at(8000.0, Mode::Bearing), at(0.0, Mode::Bearing));
    }

    #[test]
    fn shear_out_matches_the_closed_form() {
        let mat = Strengths::from_material(get_material("al7075"));
        let g = geom(2.0);
        let r = AllowableModel::default().respond(&g, &mat, 0.0).unwrap();
        let cap = 2.0 * 48_000.0 * 0.5 * (1.0 - 0.25 * 40f64.to_radians().cos());
        assert!((margin(&*r, Mode::ShearOut, 10_000.0) - (cap / 10_000.0 - 1.0)).abs() < 1e-9);
    }
}
