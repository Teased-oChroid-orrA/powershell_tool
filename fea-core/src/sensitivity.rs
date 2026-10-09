//! Design sensitivities of linear static responses and natural frequencies by the adjoint method: one extra solve per
//! response, whatever the number of parameters.
//!
//! For a response `J(u)` with `K u = f` (constrained dofs eliminated): the adjoint `K lambda = dJ/du`, then
//! `dJ/dp = lambda^T (df/dp - (dK/dp) u)`. The stiffness derivative is assembled element by element only where the
//! parameter acts (a block modulus, the thickness, a node position) and applied to vectors, never formed globally. A frequency
//! `omega^2` of a simple mode `phi` (mass-normalised) changes as `phi^T (dK/dp - omega^2 dM/dp) phi`.
//!
//! Shape derivatives of element matrices and of the load vector are central differences of the element kernels (semi-analytic:
//! exact for the discretisation up to the difference step); every derivative is checked against a global finite difference
//! of the full solve in `tests/sensitivity.rs`.

use crate::analysis::Model;
use crate::dynamics::Mode;
use crate::kernel::{self, Work};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::Physics;

/// A design parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    /// Multiplier of the elastic modulus of one block, taken at its current value (the derivative is with respect to the
    /// multiplier at 1).
    ModulusScale(usize),
    /// Multiplier of the mass density of one block (frequencies only).
    DensityScale(usize),
    /// A node coordinate.
    Coord { node: usize, axis: usize },
    /// The plane thickness (plane stress / strain only).
    Thickness,
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Model {
    /// Elements `(block, element)` a parameter touches.
    fn affected(&self, p: Param) -> Vec<(usize, usize)> {
        match p {
            Param::ModulusScale(b) | Param::DensityScale(b) => (0..self.mesh.blocks.get(b).map_or(0, |bl| bl.n_elems())).map(|e| (b, e)).collect(),
            Param::Thickness => self.mesh.blocks.iter().enumerate().flat_map(|(b, bl)| (0..bl.n_elems()).map(move |e| (b, e))).collect(),
            Param::Coord { node, .. } => self.mesh.blocks.iter().enumerate().flat_map(|(b, bl)| (0..bl.n_elems()).filter(move |&e| bl.elem(e).contains(&node)).map(move |e| (b, e))).collect(),
        }
    }

    fn element_stiffness(&self, bi: usize, e: usize, xyz: &[[f64; 3]], work: &mut Work) -> Result<Vec<f64>, String> {
        let blk = &self.mesh.blocks[bi];
        let nn = blk.kind.n_nodes();
        let d = self.mesh.dim();
        let mut ke = vec![0.0; nn * nn * d * d];
        let _ = e;
        kernel::stiffness(blk.kind, self.mesh.physics, &blk.material, xyz, work, &mut ke).map_err(|er| er.to_string())?;
        Ok(ke)
    }

    fn element_mass(&self, bi: usize, xyz: &[[f64; 3]], work: &mut Work) -> Result<Vec<f64>, String> {
        let blk = &self.mesh.blocks[bi];
        let nn = blk.kind.n_nodes();
        let mut me = vec![0.0; nn * nn];
        kernel::mass(blk.kind, self.mesh.physics, blk.density, xyz, work, &mut me).map_err(|er| er.to_string())?;
        Ok(me)
    }

    /// Step for the central difference of a node coordinate: relative to the size of the smallest element around it.
    fn coord_step(&self, node: usize) -> f64 {
        let mut h = f64::INFINITY;
        for (bi, e) in self.affected(Param::Coord { node, axis: 0 }) {
            let blk = &self.mesh.blocks[bi];
            let c = blk.elem(e);
            for &a in c {
                let dx = (0..3).map(|k| (self.mesh.nodes[a][k] - self.mesh.nodes[node][k]).powi(2)).sum::<f64>().sqrt();
                if dx > 0.0 {
                    h = h.min(dx);
                }
            }
        }
        1e-6 * if h.is_finite() { h } else { 1.0 }
    }

    /// `x^T (dK/dp) y` over all dofs (`x`, `y` full vectors).
    fn dk_bilinear(&self, p: Param, x: &[f64], y: &[f64]) -> Result<f64, String> {
        let d = self.mesh.dim();
        let mut work = Work::new();
        let mut total = 0.0;
        match p {
            Param::DensityScale(_) => return Ok(0.0),
            Param::Thickness => {
                if !matches!(self.mesh.physics, Physics::PlaneStress { .. } | Physics::PlaneStrain { .. }) {
                    return Err("the thickness is a parameter of plane analyses only".into());
                }
                let k = self.assemble()?;
                let mut ky = vec![0.0; y.len()];
                k.matvec_add(&self.pattern, y, &mut ky);
                let t = match self.mesh.physics {
                    Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
                    _ => unreachable!(),
                };
                return Ok(dot(x, &ky) / t);
            }
            _ => {}
        }
        for (bi, e) in self.affected(p) {
            let blk = &self.mesh.blocks[bi];
            let nn = blk.kind.n_nodes();
            let conn = blk.elem(e);
            let mut xyz: Vec<[f64; 3]> = conn.iter().map(|&n| self.mesh.nodes[n]).collect();
            let dke: Vec<f64> = match p {
                Param::ModulusScale(_) => self.element_stiffness(bi, e, &xyz, &mut work)?,
                Param::Coord { node, axis } => {
                    let local = conn.iter().position(|&n| n == node).expect("affected element contains the node");
                    let h = self.coord_step(node);
                    let orig = xyz[local][axis];
                    xyz[local][axis] = orig + h;
                    let kp = self.element_stiffness(bi, e, &xyz, &mut work)?;
                    xyz[local][axis] = orig - h;
                    let km = self.element_stiffness(bi, e, &xyz, &mut work)?;
                    kp.iter().zip(&km).map(|(a, b)| (a - b) / (2.0 * h)).collect()
                }
                _ => unreachable!(),
            };
            for a in 0..nn {
                for b in 0..nn {
                    for i in 0..d {
                        for j in 0..d {
                            total += x[conn[a] * d + i] * dke[(a * nn + b) * d * d + i * d + j] * y[conn[b] * d + j];
                        }
                    }
                }
            }
        }
        Ok(total)
    }

    /// `x^T (dM/dp) y` over all dofs.
    fn dm_bilinear(&self, p: Param, x: &[f64], y: &[f64]) -> Result<f64, String> {
        let d = self.mesh.dim();
        let mut work = Work::new();
        let mut total = 0.0;
        match p {
            Param::ModulusScale(_) => return Ok(0.0),
            Param::Thickness => {
                let t = match self.mesh.physics {
                    Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
                    _ => return Err("the thickness is a parameter of plane analyses only".into()),
                };
                let m = self.assemble_mass(crate::dynamics::MassKind::Consistent)?;
                let mut my = vec![0.0; y.len()];
                m.matvec_add(&self.pattern, y, &mut my);
                return Ok(dot(x, &my) / t);
            }
            _ => {}
        }
        for (bi, e) in self.affected(p) {
            let blk = &self.mesh.blocks[bi];
            let nn = blk.kind.n_nodes();
            let conn = blk.elem(e);
            let mut xyz: Vec<[f64; 3]> = conn.iter().map(|&n| self.mesh.nodes[n]).collect();
            let dme: Vec<f64> = match p {
                Param::DensityScale(_) => self.element_mass(bi, &xyz, &mut work)?,
                Param::Coord { node, axis } => {
                    let local = conn.iter().position(|&n| n == node).expect("affected element contains the node");
                    let h = self.coord_step(node);
                    let orig = xyz[local][axis];
                    xyz[local][axis] = orig + h;
                    let mp = self.element_mass(bi, &xyz, &mut work)?;
                    xyz[local][axis] = orig - h;
                    let mm = self.element_mass(bi, &xyz, &mut work)?;
                    mp.iter().zip(&mm).map(|(a, b)| (a - b) / (2.0 * h)).collect()
                }
                _ => unreachable!(),
            };
            for a in 0..nn {
                for b in 0..nn {
                    for i in 0..d {
                        total += x[conn[a] * d + i] * dme[a * nn + b] * y[conn[b] * d + i];
                    }
                }
            }
        }
        Ok(total)
    }

    /// `df/dp` (full vector): the load vector's dependence on the parameter. Only the geometry (a coordinate, the thickness) changes
    /// it; central difference of the assembly.
    fn df_dp(&self, loads: &Loads, p: Param) -> Result<Vec<f64>, String> {
        let (h, perturbed): (f64, Box<dyn Fn(f64) -> Model>) = match p {
            Param::ModulusScale(_) | Param::DensityScale(_) => return Ok(vec![0.0; self.mesh.n_dofs()]),
            Param::Coord { node, axis } => {
                let h = self.coord_step(node);
                (h, Box::new(move |s| {
                    let mut m = self.mesh.clone();
                    m.nodes[node][axis] += s;
                    Model { mesh: m, pattern: self.pattern.clone() }
                }))
            }
            Param::Thickness => {
                let t = match self.mesh.physics {
                    Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
                    _ => return Err("the thickness is a parameter of plane analyses only".into()),
                };
                (1e-6 * t, Box::new(move |s| {
                    let mut m = self.mesh.clone();
                    m.physics = match m.physics {
                        Physics::PlaneStress { thickness } => Physics::PlaneStress { thickness: thickness + s },
                        Physics::PlaneStrain { thickness } => Physics::PlaneStrain { thickness: thickness + s },
                        other => other,
                    };
                    Model { mesh: m, pattern: self.pattern.clone() }
                }))
            }
        };
        let (fp, fm) = (loads::assemble(&perturbed(h).mesh, loads)?, loads::assemble(&perturbed(-h).mesh, loads)?);
        Ok(fp.iter().zip(&fm).map(|(a, b)| (a - b) / (2.0 * h)).collect())
    }

    /// Sensitivities `dJ/dp` of the linear static response `J(u)` to `params`. `grad_j` is `dJ/du` over all dofs (zero at
    /// constrained dofs). One factorisation and one adjoint solve whatever the number of parameters.
    pub fn sensitivities(&self, loads: &Loads, bc: &Dirichlet, grad_j: &[f64], params: &[Param]) -> Result<Vec<f64>, String> {
        self.check_constrained(bc)?;
        let k = self.assemble()?;
        let red = Reduced::with_ordering(&self.pattern, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let fac = red.factor(&k).map_err(|e| e.to_string())?;
        let f = loads::assemble(&self.mesh, loads)?;
        let u = fac.solve(&k, &f, bc);
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let mut rhs: Vec<f64> = free.iter().map(|&d| grad_j[d]).collect();
        fac.solve_reduced(&mut rhs);
        let mut lam = vec![0.0; u.len()];
        for (&d, v) in free.iter().zip(&rhs) {
            lam[d] = *v;
        }
        params
            .iter()
            .map(|&p| {
                let df = self.df_dp(loads, p)?;
                Ok(dot(&lam, &df) - self.dk_bilinear(p, &lam, &u)?)
            })
            .collect()
    }

    /// Sensitivities of the compliance `f^T u` (`= u^T K u`, twice the strain energy).
    pub fn compliance_sensitivities(&self, loads: &Loads, bc: &Dirichlet, params: &[Param]) -> Result<Vec<f64>, String> {
        // dC/du = 2 f (with the loads independent of u) restricted to the free dofs; the adjoint of a compliance is u itself, but the
        // generic path keeps one code for every response.
        let f = loads::assemble(&self.mesh, loads)?;
        let grad: Vec<f64> = f.iter().zip(&bc.fixed).map(|(v, fx)| if *fx { 0.0 } else { *v }).collect();
        // C = f^T u  =>  dJ/du = f, and df/dp enters explicitly as well (J depends on p through f): dC/dp = f^T du + df^T u.
        let base = self.sensitivities(loads, bc, &grad, params)?;
        let sol = self.solve_static(loads, bc)?;
        base.iter()
            .zip(params)
            .map(|(b, &p)| {
                let df = self.df_dp(loads, p)?;
                Ok(b + dot(&df, &sol.u.iter().zip(&bc.fixed).map(|(v, fx)| if *fx { 0.0 } else { *v }).collect::<Vec<_>>()))
            })
            .collect()
    }

    /// Sensitivity of the eigenvalue `omega^2` of a (simple) mode to `params`.
    pub fn eigenvalue_sensitivities(&self, mode: &Mode, params: &[Param]) -> Result<Vec<f64>, String> {
        params.iter().map(|&p| Ok(self.dk_bilinear(p, &mode.shape, &mode.shape)? - mode.omega2 * self.dm_bilinear(p, &mode.shape, &mode.shape)?)).collect()
    }
}
