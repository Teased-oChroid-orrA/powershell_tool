//! Natural frequencies and linear buckling of a problem, on the mesh, supports and loads the static solve uses.

use crate::build::{imported_mesh, sketch_mesh};
use crate::problem::{Geometry, Problem};
use crate::solve::{apply_supports, build_loads};
use fea_core::{BucklingOptions, Mesh, Model, ModalOptions};

/// One natural frequency.
#[derive(Debug, Clone)]
pub struct ModeInfo {
    pub frequency_hz: f64,
    pub omega2: f64,
    /// Share of the total mass moving along each global axis in this mode (`0..1`).
    pub effective_mass_share: [f64; 3],
    pub residual: f64,
    /// Mass-normalised shape, `dim` components per node.
    pub shape: Vec<f64>,
}

/// One mesh of an adaptive natural-frequency run.
#[derive(Debug, Clone, PartialEq)]
pub struct ModalCycle {
    pub elements: usize,
    pub dofs: usize,
    /// Largest estimated relative error of the followed eigenvalues on this mesh.
    pub max_error: f64,
}

#[derive(Clone)]
pub struct ModalSolved {
    pub problem: Problem,
    pub model: Model,
    pub modes: Vec<ModeInfo>,
    pub total_mass: f64,
    pub solve_ms: f64,
    /// The meshes of an adaptive run (`Adaptive Passes > 0` on a 2D sketch), first to last; empty for a single mesh.
    pub adapt: Vec<ModalCycle>,
    /// Why the adaptive run stopped.
    pub adapt_stop: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BucklingInfo {
    /// Multiplier of the problem's loads at which the structure buckles.
    pub load_factor: f64,
    pub residual: f64,
    /// Shape scaled to a unit peak, `dim` components per node.
    pub shape: Vec<f64>,
}

#[derive(Clone)]
pub struct BucklingSolved {
    pub problem: Problem,
    pub model: Model,
    pub modes: Vec<BucklingInfo>,
    pub solve_ms: f64,
}

fn prepare(p: &Problem, import_text: Option<&str>) -> Result<(Mesh, fea_core::Dirichlet, fea_core::Loads), String> {
    p.validate()?;
    if !p.bushings.is_empty() {
        return Err("natural frequencies and buckling are not available with interference-fit bushings (a contact problem)".into());
    }
    let mut mesh = match &p.geometry {
        Geometry::Sketch { .. } => sketch_mesh(p, None)?,
        Geometry::Imported { .. } => imported_mesh(p, import_text.ok_or("the mesh file has not been read")?)?,
        Geometry::Solid { .. } => crate::solid::solid_mesh(p)?.0,
    };
    let bc = apply_supports(&mesh, &p.supports)?;
    let loads = build_loads(&mut mesh, p)?;
    Ok((mesh, bc, loads))
}

fn friendly(e: String) -> String {
    let l = e.to_lowercase();
    if l.contains("rigid") || l.contains("positive definite") || l.contains("not fully constrained") || l.contains("unsupported") {
        format!("the model is not fully constrained (rigid-body motion remains): add supports. Solver said: {e}")
    } else {
        e
    }
}

/// Whether the natural frequencies are found on adaptively refined meshes: a 2D sketch with remeshing passes requested.
fn adaptive_modal(p: &Problem) -> bool {
    matches!(p.geometry, Geometry::Sketch { .. }) && p.analysis != crate::problem::Analysis::Solid && p.mesh.adapt_passes > 0 && p.bushings.is_empty()
}

/// The lowest `n_modes` natural frequencies of the supported structure (homogeneous supports; the loads play no part). With
/// remeshing passes on a 2D sketch the mesh is refined where the eigenvalue estimate says the lowest `n_modes` frequencies are
/// least resolved (`adaptivity::eigen::run_eigen_amr`, clusters of close frequencies refined together and every mode followed
/// by its shape), to the relative eigenvalue error `target_error^2` (an energy-norm error `e` enters an eigenvalue as `e^2`).
pub fn modal(p: &Problem, import_text: Option<&str>, n_modes: usize) -> Result<ModalSolved, String> {
    if p.material.density.is_nan() || p.material.density <= 0.0 {
        return Err("natural frequencies need a mass density (Material > Density, in consistent units: lbf s^2/in^4 with inch / psi, i.e. weight density / 386.09)".into());
    }
    let t0 = std::time::Instant::now();
    let (model, bc, adapt, adapt_stop) = if adaptive_modal(p) {
        p.validate()?;
        modal_adaptive(p, n_modes)?
    } else {
        let (mut mesh, bc, _) = prepare(p, import_text)?;
        mesh.set_density_all(p.material.density)?;
        (Model::new(mesh)?, bc, Vec::new(), None)
    };
    let r = model.modal(&bc, &ModalOptions { n_modes, ..ModalOptions::default() }).map_err(friendly)?;
    let modes = r
        .modes
        .iter()
        .map(|m| ModeInfo { frequency_hz: m.frequency_hz, omega2: m.omega2, effective_mass_share: m.effective_mass.map(|e| e / r.total_mass), residual: m.residual, shape: m.shape.clone() })
        .collect();
    Ok(ModalSolved { problem: p.clone(), model, modes, total_mass: r.total_mass, solve_ms: t0.elapsed().as_secs_f64() * 1e3, adapt, adapt_stop })
}

/// The adaptive run: the model, supports and history of the last accepted mesh.
fn modal_adaptive(p: &Problem, n_modes: usize) -> Result<(Model, fea_core::Dirichlet, Vec<ModalCycle>, Option<String>), String> {
    use fea_core::adaptivity::{run_eigen_amr, AdaptSize, EigenConfig, MeshedProblem};
    let meshed = |field: Option<&dyn Fn([f64; 2]) -> f64>| -> Result<MeshedProblem, String> {
        let mut mesh = crate::build::sketch_mesh_sized(p, field)?;
        mesh.set_density_all(p.material.density)?;
        let bc = apply_supports(&mesh, &p.supports)?;
        // the unstructured quadrilateral mesh has edges about half the requested size, the hybrid one keeps it
        let quad_scale = if p.mesh.element.is_quad() && p.mesh.strategy == crate::problem::MeshStrategy::Unstructured { 0.5 } else { 1.0 };
        Ok(MeshedProblem { model: Model::new(mesh)?, loads: fea_core::Loads::default(), bc, labels: None, quad_scale })
    };
    let mut cfg = EigenConfig { modes: (0..n_modes).collect(), rel_tol: p.mesh.target_error * p.mesh.target_error, max_cycles: p.mesh.adapt_passes as usize, ..EigenConfig::default() };
    cfg.adapt.order = if p.mesh.element.is_quadratic() { 2.0 } else { 1.0 };
    cfg.adapt.h_min = p.mesh.size / 16.0;
    cfg.adapt.h_max = p.mesh.size * 2.0;
    let mut mesher = |s: &AdaptSize| meshed(Some(&|x: [f64; 2]| s.at([x[0], x[1], 0.0])));
    let result = run_eigen_amr(meshed(None)?, &mut mesher, &cfg).map_err(|e| friendly(e.to_string()))?;
    let history = result.history.iter().map(|c| ModalCycle { elements: c.elements, dofs: c.dofs, max_error: c.relative_error.iter().cloned().filter(|e| e.is_finite()).fold(0.0, f64::max) }).collect();
    Ok((result.problem.model, result.problem.bc, history, Some(format!("{:?}", result.stop))))
}

/// The lowest `n_modes` linear buckling load factors of the problem's loads (the loads and prescribed displacements are the
/// reference state; a factor of 2 means the structure buckles at twice the applied load).
pub fn buckling(p: &Problem, import_text: Option<&str>, n_modes: usize) -> Result<BucklingSolved, String> {
    if p.analysis == crate::problem::Analysis::Axisymmetric {
        return Err("buckling is not available for axisymmetric analyses".into());
    }
    let (mesh, bc, loads) = prepare(p, import_text)?;
    let model = Model::new(mesh)?;
    let t0 = std::time::Instant::now();
    let r = model.buckling(&loads, &bc, &BucklingOptions { n_modes, ..BucklingOptions::default() }).map_err(friendly)?;
    let modes = r.modes.iter().map(|m| BucklingInfo { load_factor: m.load_factor, residual: m.residual, shape: m.shape.clone() }).collect();
    Ok(BucklingSolved { problem: p.clone(), model, modes, solve_ms: t0.elapsed().as_secs_f64() * 1e3 })
}

/// Either analysis, for a caller that runs one of them on a worker.
#[derive(Clone, Debug)]
pub enum DynSolved {
    Modal(ModalSolved),
    Buckling(BucklingSolved),
}

/// What to compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynKind {
    Modal,
    Buckling,
}

/// Run `kind` for `n_modes` modes.
pub fn run(kind: DynKind, p: &Problem, import_text: Option<&str>, n_modes: usize) -> Result<DynSolved, String> {
    // A load case studies its own supports and loads; the result stays tied to the full problem.
    let case = p.resolved();
    let mut solved = match kind {
        DynKind::Modal => modal(&case, import_text, n_modes).map(DynSolved::Modal),
        DynKind::Buckling => buckling(&case, import_text, n_modes).map(DynSolved::Buckling),
    }?;
    match &mut solved {
        DynSolved::Modal(m) => m.problem = p.clone(),
        DynSolved::Buckling(b) => b.problem = p.clone(),
    }
    Ok(solved)
}

impl DynSolved {
    pub fn problem(&self) -> &Problem {
        match self {
            DynSolved::Modal(m) => &m.problem,
            DynSolved::Buckling(b) => &b.problem,
        }
    }

    pub fn model(&self) -> &Model {
        match self {
            DynSolved::Modal(m) => &m.model,
            DynSolved::Buckling(b) => &b.model,
        }
    }

    pub fn n_modes(&self) -> usize {
        match self {
            DynSolved::Modal(m) => m.modes.len(),
            DynSolved::Buckling(b) => b.modes.len(),
        }
    }

    pub fn shape(&self, i: usize) -> &[f64] {
        match self {
            DynSolved::Modal(m) => &m.modes[i].shape,
            DynSolved::Buckling(b) => &b.modes[i].shape,
        }
    }

    pub fn magnitude(&self, i: usize) -> Vec<f64> {
        magnitude(self.model(), self.shape(i))
    }

    /// One line per mode for a readout.
    pub fn lines(&self) -> Vec<String> {
        match self {
            DynSolved::Modal(m) => {
                let mut v = vec![format!("Natural frequencies (total mass {:.4e}, {:.0} ms)", m.total_mass, m.solve_ms)];
                if let (Some(first), Some(last)) = (m.adapt.first(), m.adapt.last()) {
                    v.push(format!(
                        "  adaptive: {} meshes, {} -> {} elements, frequency error estimate {:.1e} -> {:.1e} ({})",
                        m.adapt.len(),
                        first.elements,
                        last.elements,
                        0.5 * first.max_error,
                        0.5 * last.max_error,
                        m.adapt_stop.as_deref().unwrap_or("-")
                    ));
                }
                for (i, mode) in m.modes.iter().enumerate() {
                    let s = mode.effective_mass_share;
                    v.push(format!("  {:>2}  {:>12.3} Hz   mass share x {:.0} % y {:.0} % z {:.0} %", i + 1, mode.frequency_hz, 100.0 * s[0], 100.0 * s[1], 100.0 * s[2]));
                }
                v
            }
            DynSolved::Buckling(b) => {
                let mut v = vec![format!("Buckling load factors (multiples of the applied loads, {:.0} ms)", b.solve_ms)];
                for (i, mode) in b.modes.iter().enumerate() {
                    v.push(format!("  {:>2}  {:>12.4}", i + 1, mode.load_factor));
                }
                v
            }
        }
    }

    /// What a mode shape shows besides its displacement magnitude: stresses and strains of the mode (relative to its arbitrary
    /// amplitude) and the displacement components. Empty when the stresses of the shape cannot be recovered.
    pub fn mode_fields(&self, i: usize) -> Vec<(String, Vec<f64>)> {
        let (problem, model) = match self {
            DynSolved::Modal(m) => (&m.problem, &m.model),
            DynSolved::Buckling(b) => (&b.problem, &b.model),
        };
        let Ok(nodal) = model.nodal_stresses(self.shape(i), 0.0) else { return Vec::new() };
        crate::solve::structural_fields(model.mesh.dim(), self.shape(i), &nodal, problem.material.e, problem.material.nu)
            .into_iter()
            .map(|(f, v)| (format!("{} (relative)", f.label()), v))
            .collect()
    }

    /// Title of the contour of mode `i`.
    pub fn title(&self, i: usize) -> String {
        match self {
            DynSolved::Modal(m) => format!("Mode {} - {:.3} Hz (shape magnitude)", i + 1, m.modes[i].frequency_hz),
            DynSolved::Buckling(b) => format!("Buckling mode {} - load factor {:.4} (shape magnitude)", i + 1, b.modes[i].load_factor),
        }
    }
}

impl ModalSolved {
    /// Displacement magnitude of mode `i` at every node (for a contour).
    pub fn magnitude(&self, i: usize) -> Vec<f64> {
        magnitude(&self.model, &self.modes[i].shape)
    }
}

impl BucklingSolved {
    pub fn magnitude(&self, i: usize) -> Vec<f64> {
        magnitude(&self.model, &self.modes[i].shape)
    }
}

fn magnitude(model: &Model, shape: &[f64]) -> Vec<f64> {
    let d = model.mesh.dim();
    (0..model.mesh.nodes.len()).map(|n| (0..d).map(|c| shape[n * d + c].powi(2)).sum::<f64>().sqrt()).collect()
}

impl std::fmt::Debug for ModalSolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ModalSolved({}: {} modes)", self.problem.name, self.modes.len())
    }
}

impl std::fmt::Debug for BucklingSolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BucklingSolved({}: {} modes)", self.problem.name, self.modes.len())
    }
}
