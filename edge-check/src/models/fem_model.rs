//! Item 2: plane-stress FE check.

use crate::fem::{FemSolution, MeshSpec, UnitCase};
use crate::field::{FieldResponse, UnitFieldSource};
use crate::model::{EdgeModel, Response};
use crate::types::{Geometry, Strengths, Stress};

pub struct FemModel {
    pub mesh: MeshSpec,
    /// Coarser mesh used while searching for the minimum edge distance
    /// (many solves); the result at the actual geometry uses `mesh`.
    pub search_mesh: MeshSpec,
}

impl Default for FemModel {
    fn default() -> Self {
        Self { mesh: MeshSpec::default(), search_mesh: MeshSpec { n_radial: 12, n_arc: [3, 4, 12], grade: 2.0 } }
    }
}

impl UnitFieldSource for FemSolution {
    fn stress(&self, case: UnitCase, x: f64, y: f64) -> Option<Stress> {
        self.stress_at(case, x, y)
    }
}

impl EdgeModel for FemModel {
    fn id(&self) -> &'static str {
        "fem"
    }
    fn label(&self) -> &'static str {
        "Plane-stress FE"
    }
    fn respond(&self, geom: &Geometry, mat: &Strengths, _fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        let sol = FemSolution::solve(geom, mat.e, mat.nu, self.mesh)?;
        Ok(Box::new(FieldResponse::build(&sol, geom, mat)?))
    }
    fn respond_for_search(&self, geom: &Geometry, mat: &Strengths, _fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        let sol = FemSolution::solve(geom, mat.e, mat.nu, self.search_mesh)?;
        Ok(Box::new(FieldResponse::build(&sol, geom, mat)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Loads, Mode};
    use mechanics_core::materials::get_material;

    #[test]
    fn prints_margins_for_a_typical_case() {
        let mat = Strengths::from_material(get_material("al7075"));
        let a = 0.25;
        for ed in [1.0, 1.5, 2.0, 3.0] {
            let geom = Geometry { bore_radius: a, edge: ed * 2.0 * a, thickness: 0.5, plate_far: 6.0 * a * 2.0, plate_half_height: 6.0 * a * 2.0, plane_angle_deg: 40.0 };
            let t0 = std::time::Instant::now();
            let r = FemModel::default().respond(&geom, &mat, 0.0).unwrap();
            let dt = t0.elapsed();
            let m = r.margins(&Loads { fit_pressure: 5000.0, pin_load: 1000.0 }, 1.0);
            println!("e/D={ed} {dt:?} {:?}", m.iter().map(|x| (x.mode.label(), (x.margin * 1000.0).round() / 1000.0)).collect::<Vec<_>>());
            let _ = Mode::ShearOut;
        }
    }
}
