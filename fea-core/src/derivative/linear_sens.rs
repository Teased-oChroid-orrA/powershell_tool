//! Sensitivities of linear static responses with **exact** element derivatives (Story E, linear case, and the hybrid-mesh geometry
//! interface). The element stiffness is differentiated with dual numbers through the Jacobian, so shape (node coordinate), thickness,
//! modulus and Poisson-ratio derivatives are exact to rounding; `sensitivity.rs` keeps the semi-analytic central differences of the
//! element kernels for coordinates, which this engine is checked against (and which it replaces where exactness matters).
//!
//! `dJ/dp = dJ/dp|_explicit + lambda^T (df/dp - (dK/dp) u)` with `K lambda = dJ/du` (adjoint) or, per parameter,
//! `K du/dp = df/dp - (dK/dp) u` (direct). The method is chosen by [`super::select_sensitivity`] from the dimensions unless forced.
//! The load vector's dependence on geometry (surface tractions moving with the nodes) is the central difference of the load
//! assembly, as in `sensitivity.rs`.

use super::dual::Dual;
use super::element;
use super::linalg::TangentSolver;
use super::scalar::Scalar;
use super::policy::{select_sensitivity, SensitivityContext, SensitivityMethod};
use crate::analysis::Model;
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::Physics;
use crate::sensitivity::Param;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinearParam {
    /// Multiplier of the elastic modulus of a block, at 1.
    ModulusScale(usize),
    PoissonRatio(usize),
    Thickness,
    Coord { node: usize, axis: usize },
    /// Multiplier of the thermal expansion coefficient of a block, at 1.
    ThermalExpansion(usize),
    /// The uniform temperature change `Loads::delta_t`.
    UniformTemperature,
}

impl From<Param> for LinearParam {
    fn from(p: Param) -> Self {
        match p {
            Param::ModulusScale(b) => Self::ModulusScale(b),
            Param::Thickness => Self::Thickness,
            Param::Coord { node, axis } => Self::Coord { node, axis },
            Param::DensityScale(_) => panic!("a density does not enter a static response"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearReport {
    pub method: SensitivityMethod,
    pub reasons: Vec<String>,
    pub tangent_solves: usize,
}

pub struct LinearSensitivityEngine<'m> {
    model: &'m Model,
    loads: &'m Loads,
    bc: &'m Dirichlet,
}

type D1 = Dual<f64, 1>;

impl<'m> LinearSensitivityEngine<'m> {
    pub fn new(model: &'m Model, loads: &'m Loads, bc: &'m Dirichlet) -> Self {
        Self { model, loads, bc }
    }

    fn thickness(&self) -> f64 {
        match self.model.mesh.physics {
            Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
            _ => 1.0,
        }
    }

    /// `(dK/dp) u` over all dofs, element by element with exact derivatives.
    pub fn dk_u(&self, p: LinearParam, u: &[f64]) -> Result<Vec<f64>, String> {
        let m = self.model;
        let d = m.mesh.dim();
        let mut out = vec![0.0; m.mesh.n_dofs()];
        if matches!(p, LinearParam::ThermalExpansion(_) | LinearParam::UniformTemperature) {
            return Ok(out); // the stiffness does not depend on the expansion coefficient or the temperature
        }
        if matches!(p, LinearParam::Thickness) && !matches!(m.mesh.physics, Physics::PlaneStress { .. } | Physics::PlaneStrain { .. }) {
            return Err("the thickness is a parameter of plane analyses only".into());
        }
        for (bi, blk) in m.mesh.blocks.iter().enumerate() {
            let blk_acts = match p {
                LinearParam::ModulusScale(b) | LinearParam::PoissonRatio(b) => b == bi,
                _ => true,
            };
            if !blk_acts {
                continue;
            }
            if blk.material.aniso.is_some() {
                return Err(format!("block {bi}: anisotropic elasticity is not supported by the exact element derivatives"));
            }
            let nn = blk.kind.n_nodes();
            let nd = nn * d;
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let local_node = match p {
                    LinearParam::Coord { node, .. } => match conn.iter().position(|&n| n == node) {
                        Some(l) => Some(l),
                        None => continue,
                    },
                    _ => None,
                };
                let mut coords: Vec<[D1; 3]> = conn.iter().map(|&n| m.mesh.nodes[n].map(D1::cst)).collect();
                let (mut ev, mut nuv, mut tv) = (D1::cst(blk.material.e), D1::cst(blk.material.nu), D1::cst(self.thickness()));
                match p {
                    LinearParam::ModulusScale(_) => ev = D1::with(blk.material.e, [blk.material.e]),
                    LinearParam::PoissonRatio(_) => nuv = D1::var(blk.material.nu, 0),
                    LinearParam::Thickness => tv = D1::var(self.thickness(), 0),
                    LinearParam::Coord { axis, .. } => {
                        let l = local_node.expect("checked");
                        coords[l][axis] = D1::var(m.mesh.nodes[conn[l]][axis], 0);
                    }
                    LinearParam::ThermalExpansion(_) | LinearParam::UniformTemperature => unreachable!("returned above"),
                }
                let ke = element::stiffness(blk.kind, m.mesh.physics, &coords, ev, nuv, tv).map_err(|er| er.to_string())?;
                for a in 0..nn {
                    for i in 0..d {
                        let mut s = 0.0;
                        for b in 0..nn {
                            for j in 0..d {
                                s += ke[(a * d + i) * nd + b * d + j].d[0] * u[conn[b] * d + j];
                            }
                        }
                        out[conn[a] * d + i] += s;
                    }
                }
            }
        }
        Ok(out)
    }

    /// `df/dp` (the load vector's own dependence on the parameter). The thermal part of the load is differentiated exactly at the
    /// element level; the other geometry-dependent loads (tractions moving with the nodes) keep the central difference of the load
    /// assembly, applied to the loads without their thermal part so that nothing is counted twice.
    fn df_dp(&self, p: LinearParam) -> Result<Vec<f64>, String> {
        let n = self.model.mesh.n_dofs();
        let mechanical = || -> Loads { Loads { delta_t: 0.0, temperature: None, ..self.loads.clone() } };
        let mut out = match p {
            LinearParam::ModulusScale(_) | LinearParam::PoissonRatio(_) | LinearParam::ThermalExpansion(_) | LinearParam::UniformTemperature => vec![0.0; n],
            LinearParam::Thickness => self.model.df_dp(&mechanical(), Param::Thickness)?,
            LinearParam::Coord { node, axis } => self.model.df_dp(&mechanical(), Param::Coord { node, axis })?,
        };
        if self.loads.delta_t != 0.0 || self.loads.temperature.is_some() {
            for (o, v) in out.iter_mut().zip(self.thermal_df(p)?) {
                *o += v;
            }
        }
        Ok(out)
    }

    /// Exact derivative of the thermal load vector `f_th(E, nu, alpha, t, x, dT)` with respect to `p`.
    fn thermal_df(&self, p: LinearParam) -> Result<Vec<f64>, String> {
        let m = self.model;
        let d = m.mesh.dim();
        let mut out = vec![0.0; m.mesh.n_dofs()];
        for (bi, blk) in m.mesh.blocks.iter().enumerate() {
            let acts = match p {
                LinearParam::ModulusScale(b) | LinearParam::PoissonRatio(b) | LinearParam::ThermalExpansion(b) => b == bi,
                _ => true,
            };
            if !acts {
                continue;
            }
            if blk.material.aniso.is_some() {
                return Err(format!("block {bi}: anisotropic thermal loads are not supported by the exact derivatives"));
            }
            let nn = blk.kind.n_nodes();
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let local_node = match p {
                    LinearParam::Coord { node, .. } => match conn.iter().position(|&n| n == node) {
                        Some(l) => Some(l),
                        None => continue,
                    },
                    _ => None,
                };
                let gauss: Vec<f64> = match &self.loads.temperature {
                    Some(t) => crate::kernel::gauss_temperatures(blk.kind, &conn.iter().map(|&n| t[n]).collect::<Vec<_>>()),
                    None => vec![0.0; blk.kind.table().ngp],
                };
                let mut coords: Vec<[D1; 3]> = conn.iter().map(|&n| m.mesh.nodes[n].map(D1::cst)).collect();
                let (mut ev, mut nuv, mut av, mut tv) = (D1::cst(blk.material.e), D1::cst(blk.material.nu), D1::cst(blk.material.alpha), D1::cst(self.thickness()));
                let mut dt: Vec<D1> = gauss.iter().map(|g| D1::cst(self.loads.delta_t + g)).collect();
                match p {
                    LinearParam::ModulusScale(_) => ev = D1::with(blk.material.e, [blk.material.e]),
                    LinearParam::PoissonRatio(_) => nuv = D1::var(blk.material.nu, 0),
                    LinearParam::ThermalExpansion(_) => av = D1::with(blk.material.alpha, [blk.material.alpha]),
                    LinearParam::Thickness => tv = D1::var(self.thickness(), 0),
                    LinearParam::Coord { axis, .. } => {
                        let l = local_node.expect("checked");
                        coords[l][axis] = D1::var(m.mesh.nodes[conn[l]][axis], 0);
                    }
                    LinearParam::UniformTemperature => dt = gauss.iter().map(|g| D1::with(self.loads.delta_t + g, [1.0])).collect(),
                }
                let fe = element::thermal_load(blk.kind, m.mesh.physics, &coords, ev, nuv, av, tv, &dt).map_err(|er| er.to_string())?;
                for a in 0..nn {
                    for i in 0..d {
                        out[conn[a] * d + i] += fe[a * d + i].d[0];
                    }
                }
            }
        }
        Ok(out)
    }

    /// Sensitivities `result[q][p]` of the quantities `J_q = grad_q . u` to `params`. `method = None` lets the policy choose.
    pub fn sensitivities(&self, qois: &[Vec<f64>], params: &[LinearParam], method: Option<SensitivityMethod>) -> Result<(Vec<Vec<f64>>, LinearReport), String> {
        let m = self.model;
        m.check_constrained(self.bc)?;
        let k = m.assemble()?;
        let red = Reduced::with_ordering(&m.pattern, self.bc, Some(&m.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let f = loads::assemble(&m.mesh, self.loads)?;
        let solver = TangentSolver::new(&red, &m.pattern, &k, m.mesh.n_dofs(), &free, &[]).map_err(|e| e.to_string())?;
        // u
        let (rhs_u, mut u) = red.reduced_rhs(&k, &f, self.bc);
        let xr = solver.solve(&rhs_u)?;
        red.scatter(&xr, &mut u);
        // v_p = df/dp - (dK/dp) u
        let vectors: Vec<Vec<f64>> = params
            .iter()
            .map(|&p| {
                let df = self.df_dp(p)?;
                let dku = self.dk_u(p, &u)?;
                Ok(df.iter().zip(&dku).map(|(a, b)| a - b).collect())
            })
            .collect::<Result<_, String>>()?;
        let restrict = |v: &[f64]| -> Vec<f64> { free.iter().map(|&d| v[d]).collect() };
        let ctx = SensitivityContext { n_dofs: m.mesh.n_dofs(), n_parameters: params.len(), n_outputs: qois.len(), steps: 1, ..SensitivityContext::static_linear(m.mesh.n_dofs(), params.len(), qois.len()) };
        let plan = select_sensitivity(&ctx).map_err(|e| e.to_string())?;
        let chosen = method.unwrap_or(match plan.method {
            SensitivityMethod::MatrixFree => SensitivityMethod::DiscreteAdjoint,
            other => other,
        });
        let mut out = vec![vec![0.0; params.len()]; qois.len()];
        match chosen {
            SensitivityMethod::Direct => {
                for (pi, v) in vectors.iter().enumerate() {
                    let du = solver.solve(&restrict(v))?;
                    for (qi, g) in qois.iter().enumerate() {
                        out[qi][pi] = free.iter().zip(&du).map(|(&d, x)| g[d] * x).sum();
                    }
                }
            }
            _ => {
                for (qi, g) in qois.iter().enumerate() {
                    let lam = solver.solve_transpose(&restrict(g))?;
                    for (pi, v) in vectors.iter().enumerate() {
                        out[qi][pi] = free.iter().zip(&lam).map(|(&d, x)| v[d] * x).sum();
                    }
                }
            }
        }
        Ok((out, LinearReport { method: chosen, reasons: plan.reasons, tangent_solves: solver.n_solves() }))
    }
}
