//! A solved state tied to the mesh it was computed on.

use crate::meshing::fingerprint;
use crate::mesh::Mesh;

/// Displacements (and the uniform temperature change they were solved with) of one mesh. The fingerprint ties the vector to
/// that mesh: [`SolverSnapshot::matches`] is false for any other mesh, so a stale solution is never paired with a new mesh.
#[derive(Debug, Clone)]
pub struct SolverSnapshot {
    fingerprint: u64,
    pub u: Vec<f64>,
    pub delta_t: f64,
    /// Free-form provenance for the history ("pass 3", "after refine").
    pub label: String,
}

impl SolverSnapshot {
    /// Snapshot of `u` on `mesh`; refuses a vector of the wrong length or with non-finite entries.
    pub fn new(mesh: &Mesh, u: Vec<f64>, delta_t: f64, label: &str) -> Result<Self, String> {
        if u.len() != mesh.n_dofs() {
            return Err(format!("{} displacement values for {} dofs", u.len(), mesh.n_dofs()));
        }
        if let Some(i) = u.iter().position(|v| !v.is_finite()) {
            return Err(format!("displacement {i} is not finite"));
        }
        Ok(Self { fingerprint: fingerprint(mesh), u, delta_t, label: label.to_string() })
    }

    pub fn matches(&self, mesh: &Mesh) -> bool {
        self.fingerprint == fingerprint(mesh)
    }
}
