//! Patch topology shared by the recovery-based estimators: which nodes sit on the boundary and where the elements around a node
//! belong to different blocks or materials. Computed once per mesh (cache it with `meshing::Cached`).

use crate::mesh::Mesh;
use crate::meshing::adjacency::Adjacency;

#[derive(Debug, Clone)]
pub struct PatchTopology {
    /// Node lies on a boundary face (any node of the face, midside included).
    pub on_boundary: Vec<bool>,
    /// The elements around the node belong to more than one block.
    pub block_interface: Vec<bool>,
    /// The elements around the node have different materials: stress is discontinuous there and must not be smoothed across.
    pub material_interface: Vec<bool>,
}

impl PatchTopology {
    pub fn new(mesh: &Mesh, adj: &Adjacency) -> Self {
        let n = mesh.nodes.len();
        let mut on_boundary = vec![false; n];
        let local: Vec<Vec<Vec<usize>>> = mesh.blocks.iter().map(|b| crate::topology::local_faces(b.kind)).collect();
        for f in adj.boundary_faces() {
            let (g, lf) = f.owners[0];
            let (b, e) = adj.local(g as usize);
            let conn = mesh.blocks[b].elem(e);
            for &l in &local[b][lf as usize] {
                on_boundary[conn[l]] = true;
            }
        }
        let (mut block_interface, mut material_interface) = (vec![false; n], vec![false; n]);
        let mut first_block = vec![usize::MAX; n];
        for (b, blk) in mesh.blocks.iter().enumerate() {
            for &nd in &blk.conn {
                if first_block[nd] == usize::MAX {
                    first_block[nd] = b;
                } else if first_block[nd] != b {
                    block_interface[nd] = true;
                    if mesh.blocks[first_block[nd]].material != blk.material {
                        material_interface[nd] = true;
                    }
                }
            }
        }
        Self { on_boundary, block_interface, material_interface }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    #[test]
    fn boundary_and_material_interface_nodes_are_flagged() {
        let mut m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [4, 2, 1], &|p| [4.0 * p[0], 2.0 * p[1], 0.0]).unwrap();
        // two materials: split the single block by column
        let blk = m.blocks.remove(0);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for e in 0..blk.n_elems() {
            let c = blk.elem(e);
            if m.nodes[c[0]][0] < 2.0 - 1e-9 { left.extend_from_slice(c) } else { right.extend_from_slice(c) }
        }
        m.add_block(ElementKind::Quad4, left, Elastic::new(1.0, 0.3), "soft").unwrap();
        m.add_block(ElementKind::Quad4, right, Elastic::new(10.0, 0.3), "stiff").unwrap();
        let adj = Adjacency::new(&m);
        let p = PatchTopology::new(&m, &adj);
        for (i, x) in m.nodes.iter().enumerate() {
            let boundary = x[0] < 1e-9 || x[0] > 4.0 - 1e-9 || x[1] < 1e-9 || x[1] > 2.0 - 1e-9;
            assert_eq!(p.on_boundary[i], boundary, "node {i} {x:?}");
            assert_eq!(p.material_interface[i], (x[0] - 2.0).abs() < 1e-9, "node {i} {x:?}");
            assert_eq!(p.block_interface[i], p.material_interface[i]);
        }
    }
}
