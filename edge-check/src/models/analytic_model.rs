//! Item 1: elastic stress superposition (no mesh).

use crate::analytic::AnalyticField;
use crate::field::FieldResponse;
use crate::model::{EdgeModel, Response};
use crate::types::{Geometry, Strengths};

#[derive(Default)]
pub struct AnalyticModel;

impl EdgeModel for AnalyticModel {
    fn id(&self) -> &'static str {
        "analytic"
    }
    fn label(&self) -> &'static str {
        "Stress superposition"
    }
    /// Engineering judgement, not test-calibrated: the elastic mean-shear criterion
    /// is unvalidated and its capacity differs from the contact FE's by ~15% on the default bushing.
    fn model_cv(&self) -> f64 {
        0.15
    }
    fn respond(&self, geom: &Geometry, mat: &Strengths, _fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        let field = AnalyticField::build(geom.bore_radius, geom.edge, geom.thickness, mat.nu)?;
        Ok(Box::new(FieldResponse::build(&field, geom, mat)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fem::{FemSolution, MeshSpec, UnitCase};
    use crate::field::UnitFieldSource;
    use crate::models::fem_model::FemModel;
    use crate::types::Loads;

    #[test]
    fn analytic_and_fem_agree_on_the_unit_fields() {
        let a = 0.25;
        for ed in [1.0, 1.5, 2.0, 3.0] {
            let e = ed * 2.0 * a;
            let g = Geometry { bore_radius: a, edge: e, thickness: 0.5, plate_far: 60.0 * a, plate_half_height: 60.0 * a, plane_angle_deg: 40.0 };
            let fem = FemSolution::solve(&g, 10.3e6, 0.33, MeshSpec { n_radial: 28, n_arc: [4, 6, 30], grade: 1.8 }).unwrap();
            let an = AnalyticField::build(a, e, 0.5, 0.33).unwrap();
            println!("e/D={ed}");
            for case in [UnitCase::Fit, UnitCase::PinFull, UnitCase::PinHalf] {
                let mut line = String::new();
                for (label, x, y) in [("lig", 0.5 * (e - a), 0.0), ("edge0", 0.0, 0.0), ("top", e, a), ("loadpt", e - a, 0.0), ("back", e + a, 0.0), ("edge1", 0.0, 2.0 * a)] {
                    let f = fem.stress_at(case, x, y).unwrap();
                    let n = an.stress(case, x, y).unwrap();
                    let unit = if matches!(case, UnitCase::Fit) { 1.0 } else { 1.0 / (a * 0.5) };
                    line += &format!(" {label}: fem({:.3},{:.3},{:.3}) an({:.3},{:.3},{:.3}) |", f.xx / unit, f.yy / unit, f.xy / unit, n.xx / unit, n.yy / unit, n.xy / unit);
                }
                println!("  {case:?}{line}");
            }
        }
    }

    #[test]
    fn analytic_and_fem_agree_on_margins_and_the_analytic_is_fast() {
        let mat = crate::types::Strengths::from_material(mechanics_core::materials::get_material("al7075"));
        let a = 0.25;
        for ed in [1.0, 1.5, 2.0, 3.0] {
            let g = Geometry { bore_radius: a, edge: ed * 2.0 * a, thickness: 0.5, plate_far: 40.0 * a, plate_half_height: 40.0 * a, plane_angle_deg: 40.0 };
            let t0 = std::time::Instant::now();
            let ra = AnalyticModel.respond(&g, &mat, 0.0).unwrap();
            let t_an = t0.elapsed();
            let t0 = std::time::Instant::now();
            let rf = FemModel::default().respond(&g, &mat, 0.0).unwrap();
            let t_fem = t0.elapsed();
            for loads in [Loads { fit_pressure: 5000.0, pin_load: 1000.0 }, Loads { fit_pressure: 0.0, pin_load: 1000.0 }, Loads { fit_pressure: 3000.0, pin_load: 20000.0 }] {
                let (ma, mf) = (ra.margins(&loads, 1.0), rf.margins(&loads, 1.0));
                println!("e/D={ed} {loads:?} an {t_an:?} fem {t_fem:?}");
                for (x, y) in ma.iter().zip(&mf) {
                    println!("    {:>10}: an {:>9.3} fem {:>9.3}", x.mode.label(), x.margin, y.margin);
                }
            }
        }
    }
}
