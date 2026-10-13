//! Moving a solution from one mesh to the next.
//!
//! [`NodalInterpolation`] evaluates the old displacement field at every new node (shape functions of the old element that
//! contains it). It is exact for any field the old elements reproduce (affine for linear elements, quadratic for quadratic
//! ones) and is the warm start and the comparison field of a remeshing step; a *constrained* value (a Dirichlet dof) must be
//! re-imposed by the caller. History variables (plastic strain, contact slip) are not interpolable and are refused until a
//! physically admissible transfer exists (re-equilibration after transfer); callers must treat the refusal as "start the
//! nonlinear solve from the virgin state", never as success.

use super::SolverSnapshot;
use crate::analysis::Model;
use crate::mesh::Mesh;
use crate::meshing::MeshError;
use crate::nonlinear::NlState;

/// Result of a transfer.
#[derive(Debug, Clone)]
pub struct Transferred {
    pub snapshot: SolverSnapshot,
    /// New nodes no old element contains (their value is zero); the transfer fails when any exists unless explicitly allowed.
    pub unlocated: Vec<usize>,
}

pub trait StateTransferOperator {
    /// Displacements of `old` (solved as `from`) on the nodes of `to`.
    fn transfer(&self, old: &Model, from: &SolverSnapshot, to: &Mesh) -> Result<Transferred, MeshError>;

    /// Nonlinear history from `old` to `to`. Refused by default.
    fn transfer_history(&self, _old: &Model, _state: &NlState, _to: &Mesh) -> Result<NlState, MeshError> {
        Err(MeshError::Unsupported("history-variable transfer between meshes (restart the nonlinear solve from the virgin state)".into()))
    }
}

/// Shape-function interpolation of the old field at the new nodes.
#[derive(Debug, Clone, Copy)]
pub struct NodalInterpolation {
    /// Fraction of the new nodes that may lie outside the old mesh (a shrunk boundary, a smoother arc) before the transfer
    /// is refused. `0` demands every node be located.
    pub max_unlocated_fraction: f64,
}

impl Default for NodalInterpolation {
    fn default() -> Self {
        Self { max_unlocated_fraction: 0.0 }
    }
}

impl StateTransferOperator for NodalInterpolation {
    fn transfer(&self, old: &Model, from: &SolverSnapshot, to: &Mesh) -> Result<Transferred, MeshError> {
        if !from.matches(&old.mesh) {
            return Err(MeshError::Failed("the snapshot was not computed on this mesh".into()));
        }
        if to.dim() != old.mesh.dim() {
            return Err(MeshError::Failed("the meshes have different dimensions".into()));
        }
        let d = to.dim();
        let loc = old.locator();
        let mut u = vec![0.0; to.n_dofs()];
        let mut unlocated = Vec::new();
        for (i, x) in to.nodes.iter().enumerate() {
            let hits = loc.locate(old, *x);
            // the hit with the smallest excess (most interior) is the best element for a node on a shared edge
            let Some(h) = hits.iter().min_by(|a, b| a.excess.total_cmp(&b.excess)) else {
                unlocated.push(i);
                continue;
            };
            let blk = &old.mesh.blocks[h.block];
            let (shape, _) = blk.kind.shape(h.xi);
            for (a, &n) in blk.elem(h.elem).iter().enumerate() {
                for c in 0..d {
                    u[i * d + c] += shape[a] * from.u[n * d + c];
                }
            }
        }
        if unlocated.len() as f64 > self.max_unlocated_fraction * to.nodes.len() as f64 {
            return Err(MeshError::Failed(format!("{} of {} new nodes lie outside the old mesh", unlocated.len(), to.nodes.len())));
        }
        let snapshot = SolverSnapshot::new(to, u, from.delta_t, &format!("{} transferred", from.label)).map_err(MeshError::Failed)?;
        Ok(Transferred { snapshot, unlocated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn model(kind: ElementKind, n: usize) -> Model {
        Model::new(grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0, 0.3), [n, n, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap()).unwrap()
    }

    fn field(mesh: &Mesh, f: impl Fn(&[f64; 3]) -> [f64; 2]) -> Vec<f64> {
        mesh.nodes.iter().flat_map(f).collect()
    }

    #[test]
    fn affine_and_quadratic_fields_transfer_exactly() {
        let affine = |x: &[f64; 3]| [0.3 + 1.5 * x[0] - 0.7 * x[1], -2.0 + 0.1 * x[0] + 4.0 * x[1]];
        let quad = |x: &[f64; 3]| [x[0] * x[0] + 3.0 * x[0] * x[1], x[1] * x[1] - x[0]];
        for (kind, f, exact) in [(ElementKind::Quad4, &affine as &dyn Fn(&[f64; 3]) -> [f64; 2], "affine"), (ElementKind::Quad8, &quad, "quadratic"), (ElementKind::Tri6, &quad, "quadratic")] {
            let old = model(kind, 3);
            let snap = SolverSnapshot::new(&old.mesh, field(&old.mesh, f), 0.0, "old").unwrap();
            let new = model(ElementKind::Quad4, 7).mesh;
            let t = NodalInterpolation::default().transfer(&old, &snap, &new).unwrap();
            assert!(t.unlocated.is_empty());
            let want = field(&new, f);
            let err = t.snapshot.u.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(err < 1e-12, "{kind:?} {exact}: {err:e}");
            assert!(t.snapshot.matches(&new));
        }
    }

    #[test]
    fn a_node_outside_the_old_mesh_fails_the_transfer() {
        let old = model(ElementKind::Quad4, 2);
        let snap = SolverSnapshot::new(&old.mesh, vec![0.0; old.mesh.n_dofs()], 0.0, "old").unwrap();
        let mut new = model(ElementKind::Quad4, 2).mesh;
        new.nodes[0][0] = -3.0;
        assert!(NodalInterpolation::default().transfer(&old, &snap, &new).is_err());
        let lenient = NodalInterpolation { max_unlocated_fraction: 0.2 }.transfer(&old, &snap, &new).unwrap();
        assert_eq!(lenient.unlocated, vec![0]);
    }

    #[test]
    fn a_snapshot_refuses_another_mesh_and_bad_vectors() {
        let m = model(ElementKind::Quad4, 2);
        assert!(SolverSnapshot::new(&m.mesh, vec![0.0; 3], 0.0, "x").is_err());
        let mut bad = vec![0.0; m.mesh.n_dofs()];
        bad[1] = f64::NAN;
        assert!(SolverSnapshot::new(&m.mesh, bad, 0.0, "x").is_err());
        let snap = SolverSnapshot::new(&m.mesh, vec![0.0; m.mesh.n_dofs()], 0.0, "x").unwrap();
        let other = model(ElementKind::Quad4, 3);
        assert!(NodalInterpolation::default().transfer(&other, &snap, &m.mesh).is_err());
    }

    #[test]
    fn history_transfer_is_refused_not_faked() {
        let m = model(ElementKind::Quad4, 2);
        let st = NlState::new(&m.mesh);
        assert!(matches!(NodalInterpolation::default().transfer_history(&m, &st, &m.mesh), Err(MeshError::Unsupported(_))));
    }
}
