//! Explicit one-way steady thermal -> linear structural coupling on the same mesh.
//! No deformation feedback, transient coupling, monolithic tangent, or plasticity is claimed.
use crate::{Dirichlet, HeatLoads, HeatSolution, Loads, Model, Solution};
use crate::fields::DofMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CouplingDirection { ThermalToStructural }
#[derive(Debug, Clone)]
pub struct Thermomechanical {
    pub direction: CouplingDirection,
    pub thermal_fields: DofMap,
    pub structural_fields: DofMap,
    pub heat: HeatSolution,
    pub structure: Solution,
    pub reference_temperature: f64,
}
impl Model {
    /// Heat temperatures are absolute in the caller's units; structural thermal strains use
    /// their change from `reference_temperature`. An existing nodal temperature load is refused
    /// to avoid silently replacing it; `loads.delta_t` remains an additional uniform change.
    pub fn solve_thermomechanical(&self, heat_loads: &HeatLoads, heat_bc: &Dirichlet,
        loads: &Loads, structural_bc: &Dirichlet, reference_temperature: f64, tol: f64) -> Result<Thermomechanical, String> {
        if !reference_temperature.is_finite() || loads.temperature.is_some() {
            return Err("one-way coupling: finite reference temperature and no preexisting nodal temperature load required".into());
        }
        let heat = self.solve_heat_steady(heat_loads, heat_bc, tol)?;
        let mut structural_loads = loads.clone();
        structural_loads.temperature = Some(heat.temperature.iter().map(|t| t - reference_temperature).collect());
        let structure = self.solve_adaptive(&structural_loads, structural_bc,
            &crate::Requirements { residual_tol: tol, allow_iterative: false, ..Default::default() })?.solution;
        Ok(Thermomechanical { direction: CouplingDirection::ThermalToStructural,
            thermal_fields: self.temperature_fields(), structural_fields: self.displacement_fields(),
            heat, structure, reference_temperature })
    }
}
