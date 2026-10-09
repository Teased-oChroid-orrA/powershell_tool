//! Model validation before a solve: input completeness, physical consistency, well-posedness and capability. Findings are
//! data (`Issue`), not panics: an `Error` means the solve would fail or return a meaningless answer; a `Warning` means it
//! will run but the user should know (and the adaptive solver records it in its report).

use crate::analysis::Model;
use crate::kernel::{self, Work};
use crate::linear::Dirichlet;
use crate::loads::{self, Loads};
use crate::mesh::Physics;
use crate::{ElementKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    /// Stable machine-readable code (`rigid_body`, `inverted_element`, ...).
    pub code: &'static str,
    pub message: String,
}

fn issue(severity: Severity, code: &'static str, message: String) -> Issue {
    Issue { severity, code, message }
}

/// Longest over shortest edge of an element's corner polygon / polyhedron bound (a cheap aspect-ratio proxy: it uses the
/// bounding extents of the element's nodes along each axis).
fn extent_ratio(xyz: &[[f64; 3]], dim: usize) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in xyz {
        for k in 0..dim {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let ext: Vec<f64> = (0..dim).map(|k| hi[k] - lo[k]).collect();
    let max = ext.iter().cloned().fold(0.0f64, f64::max);
    let min = ext.iter().cloned().fold(f64::INFINITY, f64::min);
    if min > 0.0 {
        max / min
    } else {
        f64::INFINITY
    }
}

impl Model {
    /// Check a static problem (`loads`, `bc`) before solving it. Always returns every finding (never stops at the first).
    pub fn validate(&self, loads: &Loads, bc: &Dirichlet) -> Vec<Issue> {
        let mut out = Vec::new();
        let mesh = &self.mesh;
        let d = mesh.dim();
        if mesh.blocks.is_empty() || mesh.nodes.is_empty() {
            out.push(issue(Severity::Error, "empty_mesh", "the mesh has no elements".into()));
            return out;
        }
        if bc.fixed.len() != mesh.n_dofs() {
            out.push(issue(Severity::Error, "bc_size", format!("the constraint set is for {} dofs, the mesh has {}", bc.fixed.len(), mesh.n_dofs())));
            return out;
        }
        if bc.value.iter().zip(&bc.fixed).any(|(v, f)| *f && !v.is_finite()) {
            out.push(issue(Severity::Error, "bc_value", "a prescribed displacement is not finite".into()));
        }
        if mesh.nodes.iter().any(|p| p.iter().any(|v| !v.is_finite())) {
            out.push(issue(Severity::Error, "node_coordinates", "a node coordinate is not finite".into()));
            return out;
        }
        for (bi, blk) in mesh.blocks.iter().enumerate() {
            if let Err(e) = blk.material.validate() {
                out.push(issue(Severity::Error, "material", format!("block {bi} ({}): {e}", blk.name)));
            }
            if matches!(blk.kind, ElementKind::Tri3 | ElementKind::Tet4) {
                out.push(issue(Severity::Warning, "low_order_element", format!("block {bi} ({}) uses {:?}: constant-strain elements are poor on pressure and bending problems (a library element for meshers, not for results)", blk.name, blk.kind)));
            }
        }
        // Element geometry.
        let mut work = Work::new();
        let (mut inverted, mut slender, mut worst_ratio) = (Vec::new(), 0usize, 0.0f64);
        for (bi, blk) in mesh.blocks.iter().enumerate() {
            let nn = blk.kind.n_nodes();
            for (e, conn) in blk.conn.chunks_exact(nn).enumerate() {
                let xyz: Vec<[f64; 3]> = conn.iter().map(|&n| mesh.nodes[n]).collect();
                if kernel::geometry(blk.kind, mesh.physics, &xyz, &mut work).is_err() {
                    if inverted.len() < 5 {
                        inverted.push((bi, e));
                    }
                    continue;
                }
                let r = extent_ratio(&xyz, d);
                worst_ratio = worst_ratio.max(r);
                if r > 50.0 {
                    slender += 1;
                }
            }
        }
        if !inverted.is_empty() {
            out.push(issue(Severity::Error, "inverted_element", format!("inverted or degenerate elements (block, element): {inverted:?}{}", if inverted.len() == 5 { " ..." } else { "" })));
        }
        if slender > 0 {
            out.push(issue(Severity::Warning, "element_aspect", format!("{slender} elements have an extent ratio above 50 (worst {worst_ratio:.0}): the stiffness matrix is ill-conditioned and results may be inaccurate")));
        }
        // Free nodes no element touches have no stiffness.
        let mut used = vec![false; mesh.nodes.len()];
        for blk in &mesh.blocks {
            for &n in &blk.conn {
                used[n] = true;
            }
        }
        let orphans: Vec<usize> = (0..mesh.nodes.len()).filter(|&n| !used[n] && (0..d).any(|c| !bc.fixed[n * d + c])).take(5).collect();
        if !orphans.is_empty() {
            out.push(issue(Severity::Error, "orphan_node", format!("nodes {orphans:?} belong to no element and are not fully constrained (singular system)")));
        }
        if let Err(e) = self.check_constrained(bc) {
            out.push(issue(Severity::Error, "rigid_body", e));
        }
        // Loads.
        match loads::assemble(mesh, loads) {
            Err(e) => out.push(issue(Severity::Error, "loads", e)),
            Ok(f) => {
                if f.iter().any(|v| !v.is_finite()) {
                    out.push(issue(Severity::Error, "load_value", "the assembled load vector is not finite".into()));
                } else if f.iter().all(|v| *v == 0.0) && !bc.value.iter().zip(&bc.fixed).any(|(v, fx)| *fx && *v != 0.0) {
                    out.push(issue(Severity::Warning, "no_load", "no load and no prescribed displacement: the solution is zero".into()));
                }
            }
        }
        if matches!(mesh.physics, Physics::Axisymmetric) && mesh.nodes.iter().any(|p| p[0] < 0.0) {
            out.push(issue(Severity::Error, "axisymmetric_radius", "an axisymmetric model has nodes at negative radius".into()));
        }
        out
    }
}

/// `Err` with every error joined when `issues` contains one.
pub fn errors_of(issues: &[Issue]) -> Result<(), String> {
    let errs: Vec<String> = issues.iter().filter(|i| i.severity == Severity::Error).map(|i| format!("{}: {}", i.code, i.message)).collect();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}
