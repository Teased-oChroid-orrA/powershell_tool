//! Heat conduction on the mesh of a structural model: steady and transient, temperature-dependent conductivity, and the
//! temperature field as a thermal load of the structure (`Loads::temperature`).
//!
//! One temperature unknown per node, so the system has the scalar (`d = 1`) view of the structural pattern
//! ([`Pattern::scalar`]). Element matrices: conduction `integral grad N_a . k grad N_b` and capacity `integral rho c N_a N_b`
//! (the kernels of the stiffness and mass: `kernel::conduction`, `kernel::mass`); boundary terms by the same face
//! quadrature as the structural surface loads. Dirichlet conditions are eliminated, never penalised.

use crate::analysis::Model;
use crate::assembly::{assemble_scalar_identity, BlockMatrix, Pattern};
use crate::kernel::{self, Work};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::face_points;

/// Conductivity of a block: constant, or a table of `(temperature, k)` pairs interpolated linearly (held constant outside).
#[derive(Debug, Clone, PartialEq)]
pub enum Conductivity {
    Constant(f64),
    Table(Vec<(f64, f64)>),
}

impl Conductivity {
    pub fn at(&self, t: f64) -> f64 {
        match self {
            Conductivity::Constant(k) => *k,
            Conductivity::Table(p) => {
                if t <= p[0].0 {
                    return p[0].1;
                }
                for w in p.windows(2) {
                    if t <= w[1].0 {
                        let f = (t - w[0].0) / (w[1].0 - w[0].0);
                        return w[0].1 + f * (w[1].1 - w[0].1);
                    }
                }
                p[p.len() - 1].1
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let ok = match self {
            Conductivity::Constant(k) => k.is_finite() && *k > 0.0,
            Conductivity::Table(p) => !p.is_empty() && p.iter().all(|(t, k)| t.is_finite() && k.is_finite() && *k > 0.0) && p.windows(2).all(|w| w[1].0 > w[0].0),
        };
        if ok {
            Ok(())
        } else {
            Err("conductivity must be positive (a table: positive values at increasing temperatures)".into())
        }
    }
}

/// Thermal properties of one block.
#[derive(Debug, Clone, PartialEq)]
pub struct ThermalProps {
    pub conductivity: Conductivity,
    /// Volumetric heat capacity `rho c` (energy per volume per degree; the structural density is not used).
    pub capacity: f64,
}

/// Thermal loads and boundary conditions (the temperature constraints are a [`Dirichlet`] with one component per node).
#[derive(Debug, Clone, Default)]
pub struct HeatLoads {
    /// Uniform volumetric heat generation (energy per time per volume) in every block.
    pub source: f64,
    /// Heat generation in one block.
    pub block_source: Vec<(usize, f64)>,
    /// Heat entering through a boundary face, per unit area (positive heats the body).
    pub flux: Vec<(Vec<usize>, f64)>,
    /// Convection to an ambient: `(face, h, ambient temperature)`; the heat entering is `h (T_ambient - T)`.
    pub convection: Vec<(Vec<usize>, f64, f64)>,
    /// Point heat input at a node.
    pub nodal: Vec<(usize, f64)>,
}

#[derive(Debug, Clone)]
pub struct HeatSolution {
    pub temperature: Vec<f64>,
    /// `|K T - f| / |f|` over the free nodes, evaluated with the conductivity of the final temperature.
    pub rel_residual: f64,
    /// Picard iterations (`1` for a constant conductivity).
    pub iterations: usize,
    /// Heat entering through sources, fluxes and point inputs (independent of the solution).
    pub heat_in: f64,
    /// Heat leaving through the fixed-temperature nodes.
    pub heat_out_fixed: f64,
    /// Heat leaving through the convection faces, `integral h (T - T_ambient)`.
    pub heat_out_convection: f64,
}

impl HeatSolution {
    /// `|in - out|` over the sum of the magnitudes of every term: the global energy balance, which no solver tolerance enters.
    pub fn balance_error(&self) -> f64 {
        let out = self.heat_out_fixed + self.heat_out_convection;
        (self.heat_in - out).abs() / (self.heat_in.abs() + self.heat_out_fixed.abs() + self.heat_out_convection.abs()).max(1e-300)
    }
}

impl Pattern {
    /// The same node coupling with one unknown per node (the pattern of a scalar field on the structural mesh).
    pub fn scalar(&self) -> Pattern {
        Pattern { n_nodes: self.n_nodes, d: 1, col_ptr: self.col_ptr.clone(), row_idx: self.row_idx.clone(), scatter: self.scatter.clone(), colors: self.colors.clone() }
    }
}

impl Model {
    fn thermal_props(&self) -> Result<Vec<&ThermalProps>, String> {
        self.mesh
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| b.thermal.as_ref().ok_or_else(|| format!("block {i} ({}) has no thermal properties (Mesh::set_thermal)", b.name)))
            .collect()
    }

    /// Dirichlet set for the temperature field (one value per node).
    pub fn thermal_dirichlet(&self) -> Dirichlet {
        Dirichlet::new(self.mesh.nodes.len(), 1)
    }

    /// Conduction matrix at the temperature field `t` (only a conductivity table looks at it).
    fn conduction_matrix(&self, spat: &Pattern, t: &[f64]) -> Result<BlockMatrix, String> {
        let props = self.thermal_props()?;
        let mesh = &self.mesh;
        assemble_scalar_identity(mesh, spat, |bi, e, xyz, work, ke| {
            let blk = &mesh.blocks[bi];
            let nn = blk.kind.n_nodes();
            let conn = blk.elem(e);
            let tn: Vec<f64> = conn.iter().map(|&n| t[n]).collect();
            kernel::conduction(blk.kind, mesh.physics, &|tg| props[bi].conductivity.at(tg), &tn[..nn], xyz, work, ke)
        })
    }

    fn capacity_matrix(&self, spat: &Pattern) -> Result<BlockMatrix, String> {
        let props = self.thermal_props()?;
        let mesh = &self.mesh;
        assemble_scalar_identity(mesh, spat, |bi, _e, xyz, work, me| kernel::mass(mesh.blocks[bi].kind, mesh.physics, props[bi].capacity, xyz, work, me))
    }

    /// Add `h N_a N_b` of every convection face to `k`; returns the right-hand side of all heat loads and the heat the
    /// sources, fluxes and point inputs put in (the convection's `h T_ambient` is on the right-hand side but is not an input).
    fn heat_system(&self, spat: &Pattern, loads: &HeatLoads, k: &mut BlockMatrix) -> Result<(Vec<f64>, f64), String> {
        let mesh = &self.mesh;
        let n = mesh.nodes.len();
        let mut f = vec![0.0; n];
        for &(node, q) in &loads.nodal {
            if node >= n {
                return Err(format!("point heat input on missing node {node}"));
            }
            f[node] += q;
        }
        if loads.source != 0.0 || !loads.block_source.is_empty() {
            let mut work = Work::new();
            let mut fe = vec![0.0; crate::element::MAX_NODES * crate::element::MAX_NODES];
            let mut xyz = [[0.0f64; 3]; crate::element::MAX_NODES];
            for (bi, blk) in mesh.blocks.iter().enumerate() {
                let q: f64 = loads.source + loads.block_source.iter().filter(|(b, _)| *b == bi).map(|(_, v)| v).sum::<f64>();
                if q == 0.0 {
                    continue;
                }
                let nn = blk.kind.n_nodes();
                for conn in blk.conn.chunks_exact(nn) {
                    for (a, &nd) in conn.iter().enumerate() {
                        xyz[a] = mesh.nodes[nd];
                    }
                    // consistent source: q * integral N_a = row sums of the unit-density mass
                    kernel::mass(blk.kind, mesh.physics, q, &xyz[..nn], &mut work, &mut fe).map_err(|e| e.to_string())?;
                    for (a, &nd) in conn.iter().enumerate() {
                        f[nd] += (0..nn).map(|b| fe[a * nn + b]).sum::<f64>();
                    }
                }
            }
        }
        for (face, q) in &loads.flux {
            face_points(mesh, face, |nsh, _pos, _normal, w| {
                for (a, &nd) in face.iter().enumerate() {
                    f[nd] += w * nsh[a] * q;
                }
            })?;
        }
        let input: f64 = f.iter().sum();
        for (face, h, t_inf) in &loads.convection {
            if h.is_nan() || *h < 0.0 {
                return Err("a convection coefficient must be non-negative".into());
            }
            face_points(mesh, face, |nsh, _pos, _normal, w| {
                for (a, &na) in face.iter().enumerate() {
                    f[na] += w * nsh[a] * h * t_inf;
                    for (b, &nb) in face.iter().enumerate() {
                        let (hi, lo) = (na.max(nb), na.min(nb));
                        if let Some(p) = spat.find(hi, lo) {
                            // the lower triangle only; the diagonal block entry (hi == lo) takes every a == b term
                            if na >= nb {
                                k.vals[p] += w * nsh[a] * nsh[b] * h;
                            }
                        }
                    }
                }
            })?;
        }
        Ok((f, input))
    }

    /// Heat leaving through the convection faces at temperature field `t`.
    fn convection_out(&self, loads: &HeatLoads, t: &[f64]) -> Result<f64, String> {
        let mut out = 0.0;
        for (face, h, t_inf) in &loads.convection {
            face_points(&self.mesh, face, |nsh, _pos, _normal, w| {
                let tg: f64 = face.iter().enumerate().map(|(a, &nd)| nsh[a] * t[nd]).sum();
                out += w * h * (tg - t_inf);
            })?;
        }
        Ok(out)
    }

    /// Steady-state temperature field. A conductivity table makes it nonlinear: Picard iteration on `K(T) T = f` until the
    /// field changes by less than `tol` (relative to its range) AND the residual with the conductivity of the answer is
    /// below `tol`.
    pub fn solve_heat_steady(&self, loads: &HeatLoads, bc: &Dirichlet, tol: f64) -> Result<HeatSolution, String> {
        let spat = std::sync::Arc::new(self.pattern.scalar());
        let props = self.thermal_props()?;
        for p in &props {
            p.conductivity.validate()?;
        }
        if bc.d != 1 || bc.fixed.len() != self.mesh.nodes.len() {
            return Err("the temperature constraints must be a Dirichlet set with one component per node (Model::thermal_dirichlet)".into());
        }
        if bc.n_fixed() == 0 && loads.convection.is_empty() {
            return Err("a steady conduction problem needs a fixed temperature or a convection boundary (otherwise the level is undetermined)".into());
        }
        let nonlinear = props.iter().any(|p| matches!(p.conductivity, Conductivity::Table(_)));
        let n = self.mesh.nodes.len();
        let red = Reduced::with_ordering(&spat, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let mut t: Vec<f64> = (0..n).map(|i| if bc.fixed[i] { bc.value[i] } else { 0.0 }).collect();
        if nonlinear && bc.n_fixed() > 0 {
            let mean = bc.value.iter().zip(&bc.fixed).filter(|(_, f)| **f).map(|(v, _)| *v).sum::<f64>() / bc.n_fixed() as f64;
            for (i, v) in t.iter_mut().enumerate() {
                if !bc.fixed[i] {
                    *v = mean;
                }
            }
        }
        let mut iterations = 0usize;
        let last_change = loop {
            iterations += 1;
            let mut k = self.conduction_matrix(&spat, &t)?;
            let (f, _) = self.heat_system(&spat, loads, &mut k)?;
            let fac = red.factor(&k).map_err(|e| e.to_string())?;
            let tn = fac.solve(&k, &f, bc);
            let range = tn.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - tn.iter().cloned().fold(f64::INFINITY, f64::min);
            let change = tn.iter().zip(&t).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max) / range.max(1e-300);
            t = tn;
            if !nonlinear || change <= tol || iterations >= 100 {
                break change;
            }
        };
        if nonlinear && last_change > tol {
            return Err(format!("the temperature-dependent conduction iteration did not converge in {iterations} iterations (last change {last_change:.2e} of the range)"));
        }
        // Verification with the conductivity of the answer.
        let mut k = self.conduction_matrix(&spat, &t)?;
        let (f, heat_in) = self.heat_system(&spat, loads, &mut k)?;
        let mut kt = vec![0.0; n];
        k.matvec_add(&spat, &t, &mut kt);
        let (mut rn, mut fnorm, mut reaction, mut rfixed) = (0.0f64, 0.0f64, 0.0, 0.0f64);
        for i in 0..n {
            let r = kt[i] - f[i];
            if bc.fixed[i] {
                reaction += r;
                rfixed += r * r;
            } else {
                rn += r * r;
                fnorm += f[i] * f[i];
            }
        }
        // (scaled by the loads and by the heat the supports carry: a problem driven by temperatures alone has no load)
        let rel_residual = rn.sqrt() / (fnorm.sqrt() + rfixed.sqrt()).max(1e-300);
        let heat_out_convection = self.convection_out(loads, &t)?;
        Ok(HeatSolution { temperature: t, rel_residual, iterations, heat_in, heat_out_fixed: -reaction, heat_out_convection })
    }

    /// Transient temperature field by the theta method (`theta = 1` backward Euler, `0.5` Crank-Nicolson), constant
    /// conductivities, from the initial field `t0`; returns the fields at every step.
    pub fn solve_heat_transient(&self, loads: &HeatLoads, bc: &Dirichlet, t0: &[f64], dt: f64, steps: usize, theta: f64) -> Result<Vec<Vec<f64>>, String> {
        let spat = std::sync::Arc::new(self.pattern.scalar());
        let props = self.thermal_props()?;
        if props.iter().any(|p| matches!(p.conductivity, Conductivity::Table(_))) {
            return Err("temperature-dependent conductivity is available for the steady problem only".into());
        }
        for p in &props {
            p.conductivity.validate()?;
            if p.capacity.is_nan() || p.capacity <= 0.0 {
                return Err("a transient conduction problem needs a positive heat capacity in every block".into());
            }
        }
        if dt.is_nan() || dt <= 0.0 || !(0.5..=1.0).contains(&theta) {
            return Err("transient conduction: dt must be positive and theta within [0.5, 1]".into());
        }
        let n = self.mesh.nodes.len();
        if t0.len() != n {
            return Err("the initial temperature field has the wrong length".into());
        }
        let mut k = self.conduction_matrix(&spat, t0)?;
        let (f, _) = self.heat_system(&spat, loads, &mut k)?;
        let c = self.capacity_matrix(&spat)?;
        let lhs = BlockMatrix { d: 1, vals: k.vals.iter().zip(&c.vals).map(|(kk, cc)| cc / dt + theta * kk).collect() };
        let red = Reduced::with_ordering(&spat, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let fac = red.factor(&lhs).map_err(|e| e.to_string())?;
        let mut t: Vec<f64> = t0.to_vec();
        for (ti, (fx, v)) in t.iter_mut().zip(bc.fixed.iter().zip(&bc.value)) {
            if *fx {
                *ti = *v;
            }
        }
        let mut out = vec![t.clone()];
        for _ in 0..steps {
            let (mut ct, mut kt) = (vec![0.0; n], vec![0.0; n]);
            c.matvec_add(&spat, &t, &mut ct);
            k.matvec_add(&spat, &t, &mut kt);
            let rhs: Vec<f64> = (0..n).map(|i| ct[i] / dt - (1.0 - theta) * kt[i] + f[i]).collect();
            t = fac.solve(&lhs, &rhs, bc);
            out.push(t.clone());
        }
        Ok(out)
    }
}
