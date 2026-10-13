//! Welding independently generated meshes into one: nodes closer than a tolerance become one node, blocks are concatenated.
//! This is how a structured block meets an unstructured region: both sides place their interface nodes at the same
//! positions and the weld makes them the same node, which is what makes the union conforming. The weld never moves a node and
//! never merges two nodes of one element (such an element would be degenerate and is reported).

use super::MeshError;
use crate::mesh::Mesh;
use std::collections::HashMap;

/// Find-or-insert nodes by position within `tol`.
pub struct NodeWelder {
    tol: f64,
    cell: f64,
    bins: HashMap<[i64; 3], Vec<usize>>,
}

impl NodeWelder {
    pub fn new(tol: f64) -> Self {
        Self { tol, cell: (2.0 * tol).max(1e-300), bins: HashMap::new() }
    }

    fn key(&self, x: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|c| (x[c] / self.cell).floor() as i64)
    }

    /// Index of an existing node within `tol` of `x` in `nodes`, else append `x` and return its index.
    pub fn node(&mut self, nodes: &mut Vec<[f64; 3]>, x: [f64; 3]) -> usize {
        let k = self.key(x);
        for di in -1..=1 {
            for dj in -1..=1 {
                for dk in -1..=1 {
                    if let Some(v) = self.bins.get(&[k[0] + di, k[1] + dj, k[2] + dk]) {
                        for &n in v {
                            if (0..3).map(|c| (nodes[n][c] - x[c]).powi(2)).sum::<f64>().sqrt() <= self.tol {
                                return n;
                            }
                        }
                    }
                }
            }
        }
        nodes.push(x);
        let n = nodes.len() - 1;
        self.bins.entry(k).or_default().push(n);
        n
    }
}

/// Union of `parts` (same physics) with nodes welded within `tol`. Named node sets and surfaces are not carried over: the
/// caller names the boundary of the union from its own geometry (`hybrid::assign_names`).
pub fn merge(parts: &[Mesh], tol: f64) -> Result<Mesh, MeshError> {
    let Some(first) = parts.first() else {
        return Err(MeshError::Failed("nothing to merge".into()));
    };
    let mut out = Mesh::new(first.physics);
    let mut welder = NodeWelder::new(tol);
    for part in parts {
        if part.physics != first.physics {
            return Err(MeshError::Failed("the parts have different physics".into()));
        }
        let map: Vec<usize> = part.nodes.iter().map(|&x| welder.node(&mut out.nodes, x)).collect();
        for blk in &part.blocks {
            let conn: Vec<usize> = blk.conn.iter().map(|&n| map[n]).collect();
            let nn = blk.kind.n_nodes();
            for (e, el) in conn.chunks_exact(nn).enumerate() {
                let mut s = el.to_vec();
                s.sort_unstable();
                if s.windows(2).any(|w| w[0] == w[1]) {
                    return Err(MeshError::Failed(format!("welding collapsed element {e} of block '{}' (tolerance {tol:e} too large for this mesh)", blk.name)));
                }
            }
            let mut b = blk.clone();
            b.conn = conn;
            out.blocks.push(b);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn square(x0: f64, kind: ElementKind) -> Mesh {
        grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| [x0 + p[0], p[1], 0.0]).unwrap()
    }

    #[test]
    fn two_squares_weld_along_their_shared_edge() {
        for kind in [ElementKind::Quad4, ElementKind::Quad8, ElementKind::Quad9] {
            let (a, b) = (square(0.0, kind), square(1.0, kind));
            let m = merge(&[a.clone(), b.clone()], 1e-9).unwrap();
            let shared = if kind == ElementKind::Quad4 { 3 } else { 5 };
            assert_eq!(m.nodes.len(), a.nodes.len() + b.nodes.len() - shared, "{kind:?}");
            let r = m.conformity();
            assert!(r.is_conforming() && r.components == 1, "{kind:?}: {}", r.summary());
        }
    }

    #[test]
    fn a_tolerance_that_collapses_an_element_is_refused() {
        let a = square(0.0, ElementKind::Quad4);
        assert!(merge(&[a], 10.0).is_err());
    }

    #[test]
    fn near_coincident_nodes_across_a_bin_boundary_still_weld() {
        let mut nodes = Vec::new();
        let mut w = NodeWelder::new(1e-6);
        let a = w.node(&mut nodes, [1.999_999_9e-6, 0.0, 0.0]);
        let b = w.node(&mut nodes, [2.000_000_1e-6, 0.0, 0.0]);
        assert_eq!(a, b);
        assert_eq!(w.node(&mut nodes, [5e-6, 0.0, 0.0]), 1);
    }
}
