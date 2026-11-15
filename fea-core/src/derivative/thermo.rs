//! Sensitivities of a staggered thermal-structural analysis (Story J, coupled physics): a steady conduction solve `K_T T = q` feeds the
//! structural load through the thermal strain, `K u = f_mech + G T`, and a quantity `J = g . u` is differentiated through both solves.
//!
//! * `G` is the (linear) map from nodal temperatures to the thermal load vector; `G x` is the load assembly of the temperature field
//!   `x`, `G^T lambda` is built element by element from the load of each unit nodal temperature (exact: the load is linear in `T`).
//! * heat parameters `p` (conductivity scale of a block, scale of the heat sources / fluxes / point inputs) enter `T` only:
//!   `dT/dp = K_T^-1 (dq/dp - (dK_T/dp) T)`, then `du = K^-1 G dT`.
//! * structural parameters that do not move the conduction problem (modulus, Poisson ratio, expansion coefficient of a block) are
//!   handled by the exact thermal-load derivatives of [`super::linear_sens`] at the computed temperature field.
//!
//! Geometry and thickness change both the conduction and the structural problem and are not offered here (the coupled geometry
//! derivative is a separate, unimplemented extension: the result is an explicit error, not a partial derivative). Conductivity must
//! be constant (a temperature table makes the conduction nonlinear). Fixed temperatures are not parameters.
//! Direct and adjoint organisations give the same numbers; the adjoint costs two solves per quantity whatever the number of parameters.

use super::linalg::TangentSolver;
use super::linear_sens::{LinearParam, LinearSensitivityEngine};
use crate::analysis::Model;
use crate::assembly::Pattern;
use crate::kernel::{self, Work};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::thermal::{Conductivity, HeatLoads};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoupledParam {
    /// Multiplier of the (constant) conductivity of a block, at 1.
    ConductivityScale(usize),
    /// Multiplier of the volumetric sources, fluxes and point heat inputs (not of the convection ambient), at 1.
    HeatLoadScale,
    /// A structural parameter that leaves the conduction problem unchanged: modulus, Poisson ratio or expansion coefficient.
    Structural(LinearParam),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoupledReport {
    pub structural_solves: usize,
    pub thermal_solves: usize,
}

pub struct ThermoMechanicalSensitivity<'m> {
    model: &'m Model,
    mech_loads: &'m Loads,
    heat_loads: &'m HeatLoads,
    bc: &'m Dirichlet,
    thermal_bc: &'m Dirichlet,
}

/// The converged coupled state.
#[derive(Debug, Clone)]
pub struct CoupledState {
    pub temperature: Vec<f64>,
    pub u: Vec<f64>,
}

impl<'m> ThermoMechanicalSensitivity<'m> {
    /// `mech_loads` must not carry its own temperature field (the conduction solve provides it); a uniform `delta_t` is allowed.
    pub fn new(model: &'m Model, mech_loads: &'m Loads, heat_loads: &'m HeatLoads, bc: &'m Dirichlet, thermal_bc: &'m Dirichlet) -> Result<Self, String> {
        if mech_loads.temperature.is_some() {
            return Err("the structural loads must not carry a temperature field: the conduction solve provides it".into());
        }
        for (bi, b) in model.mesh.blocks.iter().enumerate() {
            match b.thermal.as_ref().map(|t| &t.conductivity) {
                Some(Conductivity::Table(_)) => return Err(format!("block {bi}: a temperature-dependent conductivity makes the conduction nonlinear (not supported)")),
                None => return Err(format!("block {bi} has no thermal properties")),
                _ => {}
            }
        }
        Ok(Self { model, mech_loads, heat_loads, bc, thermal_bc })
    }

    fn total_loads(&self, t: &[f64]) -> Loads {
        Loads { temperature: Some(t.to_vec()), ..self.mech_loads.clone() }
    }

    /// `G x`: the thermal load of the nodal temperature field `x` (no uniform part).
    fn g_apply(&self, x: &[f64]) -> Result<Vec<f64>, String> {
        loads::assemble(&self.model.mesh, &Loads { temperature: Some(x.to_vec()), ..Loads::default() })
    }

    /// `G^T lambda`, nodal.
    fn g_transpose(&self, lambda: &[f64]) -> Result<Vec<f64>, String> {
        let mesh = &self.model.mesh;
        let (n, d) = (mesh.nodes.len(), mesh.dim());
        let mut out = vec![0.0; n];
        let mut work = Work::new();
        let mut fe = vec![0.0; crate::element::MAX_NODES * crate::element::MAX_NODES];
        let mut xyz = [[0.0f64; 3]; crate::element::MAX_NODES];
        for blk in &mesh.blocks {
            let nn = blk.kind.n_nodes();
            for conn in blk.conn.chunks_exact(nn) {
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = mesh.nodes[nd];
                }
                for a in 0..nn {
                    let mut unit = vec![0.0; nn];
                    unit[a] = 1.0;
                    let gauss = kernel::gauss_temperatures(blk.kind, &unit);
                    kernel::thermal_load_with(blk.kind, mesh.physics, &blk.material, &|g| gauss[g], &xyz[..nn], &mut work, &mut fe).map_err(|e| e.to_string())?;
                    let mut s = 0.0;
                    for (b, &nd) in conn.iter().enumerate() {
                        for i in 0..d {
                            s += fe[b * d + i] * lambda[nd * d + i];
                        }
                    }
                    out[conn[a]] += s;
                }
            }
        }
        Ok(out)
    }

    fn scalar_pattern(&self) -> std::sync::Arc<Pattern> {
        std::sync::Arc::new(self.model.pattern.scalar())
    }

    /// `(dq/dp - (dK_T/dp) T)` over all nodes for a heat parameter.
    fn heat_rhs(&self, p: CoupledParam, t: &[f64], spat: &Pattern) -> Result<Vec<f64>, String> {
        let n = self.model.mesh.nodes.len();
        match p {
            CoupledParam::ConductivityScale(b) => {
                if b >= self.model.mesh.blocks.len() {
                    return Err(format!("conductivity scale of missing block {b}"));
                }
                let kb = self.model.conduction_matrix_block(spat, t, Some(b))?;
                let mut kt = vec![0.0; n];
                kb.matvec_add(spat, t, &mut kt);
                Ok(kt.iter().map(|v| -v).collect())
            }
            CoupledParam::HeatLoadScale => {
                let only = HeatLoads { convection: Vec::new(), ..self.heat_loads.clone() };
                let mut dummy = crate::assembly::BlockMatrix::zeros(spat);
                Ok(self.model.heat_system(spat, &only, &mut dummy)?.0)
            }
            CoupledParam::Structural(_) => Ok(vec![0.0; n]),
        }
    }

    pub fn solve(&self) -> Result<CoupledState, String> {
        let heat = self.model.solve_heat_steady(self.heat_loads, self.thermal_bc, 1e-12)?;
        let loads = self.total_loads(&heat.temperature);
        let sol = self.model.solve_static(&loads, self.bc)?;
        Ok(CoupledState { temperature: heat.temperature, u: sol.u })
    }

    /// Direct sensitivities `result[q][p]`.
    pub fn direct(&self, params: &[CoupledParam], qois: &[Vec<f64>]) -> Result<(Vec<Vec<f64>>, CoupledReport), String> {
        self.sensitivities(params, qois, false)
    }

    /// Adjoint sensitivities `result[q][p]`.
    pub fn adjoint(&self, params: &[CoupledParam], qois: &[Vec<f64>]) -> Result<(Vec<Vec<f64>>, CoupledReport), String> {
        self.sensitivities(params, qois, true)
    }

    fn sensitivities(&self, params: &[CoupledParam], qois: &[Vec<f64>], adjoint: bool) -> Result<(Vec<Vec<f64>>, CoupledReport), String> {
        let m = self.model;
        for p in params {
            if let CoupledParam::Structural(s) = p {
                if !matches!(s, LinearParam::ModulusScale(_) | LinearParam::PoissonRatio(_) | LinearParam::ThermalExpansion(_)) {
                    return Err(format!("{s:?}: geometry and thickness also change the conduction problem; the coupled derivative is not implemented"));
                }
            }
        }
        let state = self.solve()?;
        let loads = self.total_loads(&state.temperature);
        m.check_constrained(self.bc)?;
        let k = m.assemble()?;
        let red = Reduced::with_ordering(&m.pattern, self.bc, Some(&m.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let solver = TangentSolver::new(&red, &m.pattern, &k, m.mesh.n_dofs(), &free, &[]).map_err(|e| e.to_string())?;
        let spat = self.scalar_pattern();
        let kt = m.conduction_matrix_block(&spat, &state.temperature, None)?;
        let tred = Reduced::with_ordering(&spat, self.thermal_bc, Some(&m.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let tfree: Vec<usize> = tred.free_dofs().iter().map(|&d| d as usize).collect();
        let tsolver = TangentSolver::new(&tred, &spat, &kt, m.mesh.nodes.len(), &tfree, &[]).map_err(|e| e.to_string())?;
        let lin = LinearSensitivityEngine::new(m, &loads, self.bc);
        let restrict = |v: &[f64], idx: &[usize]| -> Vec<f64> { idx.iter().map(|&d| v[d]).collect() };
        let scatter = |x: &[f64], idx: &[usize], n: usize| -> Vec<f64> {
            let mut out = vec![0.0; n];
            for (&d, v) in idx.iter().zip(x) {
                out[d] = *v;
            }
            out
        };
        let (n_dof, n_node) = (m.mesh.n_dofs(), m.mesh.nodes.len());
        let heat_params: Vec<(usize, CoupledParam)> = params.iter().copied().enumerate().filter(|(_, p)| !matches!(p, CoupledParam::Structural(_))).collect();
        // structural parameters: partial derivative at the fixed temperature field (they do not move T)
        let structural: Vec<(usize, LinearParam)> = params.iter().enumerate().filter_map(|(i, p)| if let CoupledParam::Structural(s) = p { Some((i, *s)) } else { None }).collect();
        let mut out = vec![vec![0.0; params.len()]; qois.len()];
        if !structural.is_empty() {
            let sp: Vec<LinearParam> = structural.iter().map(|(_, s)| *s).collect();
            let (r, _) = lin.sensitivities(qois, &sp, Some(if adjoint { super::SensitivityMethod::DiscreteAdjoint } else { super::SensitivityMethod::Direct }))?;
            for (qi, row) in r.iter().enumerate() {
                for (k2, (pi, _)) in structural.iter().enumerate() {
                    out[qi][*pi] = row[k2];
                }
            }
        }
        let rhs: Vec<Vec<f64>> = heat_params.iter().map(|(_, p)| self.heat_rhs(*p, &state.temperature, &spat)).collect::<Result<_, _>>()?;
        if adjoint {
            for (qi, g) in qois.iter().enumerate() {
                let lambda = scatter(&solver.solve(&restrict(g, &free))?, &free, n_dof);
                let w = self.g_transpose(&lambda)?;
                let mu = scatter(&tsolver.solve(&restrict(&w, &tfree))?, &tfree, n_node);
                for ((pi, _), r) in heat_params.iter().zip(&rhs) {
                    out[qi][*pi] = tfree.iter().map(|&d| mu[d] * r[d]).sum();
                }
            }
        } else {
            for ((pi, _), r) in heat_params.iter().zip(&rhs) {
                let dt = scatter(&tsolver.solve(&restrict(r, &tfree))?, &tfree, n_node);
                let df = self.g_apply(&dt)?;
                let du = scatter(&solver.solve(&restrict(&df, &free))?, &free, n_dof);
                for (qi, g) in qois.iter().enumerate() {
                    out[qi][*pi] = g.iter().zip(&du).map(|(a, b)| a * b).sum();
                }
            }
        }
        Ok((out, CoupledReport { structural_solves: solver.n_solves(), thermal_solves: tsolver.n_solves() }))
    }
}
