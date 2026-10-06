//! Mesh topology: boundary faces (edges in 2D) of any element mix, in the orientation the load
//! routines expect (counter-clockwise seen from outside; 2D: body on the left).
//!
//! Corner-face lists are fixed per element family; midside and face-centre nodes are found by
//! natural-coordinate geometry (the node sitting at the average of the corner coordinates), so the
//! same code serves linear, serendipity and Lagrange elements.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::mesh::Mesh;
use std::collections::HashMap;

/// A boundary face: node list ready for `Loads::faces` / `Mesh::surfaces`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryFace {
    pub nodes: Vec<usize>,
    /// Index into `Mesh::blocks` and element index within the block.
    pub block: usize,
    pub elem: usize,
}

const HEX_FACES: [[usize; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [3, 7, 6, 2], [0, 4, 7, 3], [1, 2, 6, 5]];
const TET_FACES: [[usize; 3]; 4] = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]];

/// Corner-node faces of an element, oriented outward (counter-clockwise from outside).
fn corner_faces(kind: ElementKind) -> Vec<Vec<usize>> {
    use ElementKind::*;
    match kind {
        Tri3 | Tri6 => vec![vec![0, 1], vec![1, 2], vec![2, 0]],
        Quad4 | Quad8 | Quad9 => vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![3, 0]],
        Tet4 | Tet10 => TET_FACES.iter().map(|f| f.to_vec()).collect(),
        Hex8 | Hex20 | Hex27 => HEX_FACES.iter().map(|f| f.to_vec()).collect(),
    }
}

/// Local index of the node of `kind` located at natural coordinates `x` (within tolerance).
fn node_at(kind: ElementKind, x: [f64; 3]) -> Option<usize> {
    kind.node_coords().iter().position(|c| (0..3).all(|i| (c[i] - x[i]).abs() < 1e-12))
}

/// Local node lists of every face of `kind` (corners, then midsides of consecutive corner pairs,
/// then the face centre if the element has one).
fn local_faces(kind: ElementKind) -> Vec<Vec<usize>> {
    let nc = kind.node_coords();
    corner_faces(kind)
        .into_iter()
        .map(|corners| {
            let mut nodes = corners.clone();
            if kind.n_nodes() > kind.n_corners() {
                let k = corners.len();
                for i in 0..k {
                    let (a, b) = (nc[corners[i]], nc[corners[(i + 1) % k]]);
                    // A two-corner "face" (2D edge) closes on itself: only one midside.
                    if k == 2 && i == 1 {
                        break;
                    }
                    let mid = std::array::from_fn(|c| 0.5 * (a[c] + b[c]));
                    nodes.push(node_at(kind, mid).expect("midside node"));
                }
                if kind == ElementKind::Hex27 {
                    let mid = std::array::from_fn(|c| corners.iter().map(|&q| nc[q][c]).sum::<f64>() / 4.0);
                    nodes.push(node_at(kind, mid).expect("face centre node"));
                }
            }
            nodes
        })
        .collect()
}

impl Mesh {
    /// Faces (edges in 2D) that belong to exactly one element, in load orientation. Faces shared by
    /// two blocks (material interfaces) are interior and not reported.
    pub fn boundary_faces(&self) -> Vec<BoundaryFace> {
        struct Entry {
            count: u32,
            face: BoundaryFace,
        }
        let mut map: HashMap<Vec<usize>, Entry> = HashMap::new();
        let mut order: Vec<Vec<usize>> = Vec::new();
        for (bi, blk) in self.blocks.iter().enumerate() {
            let faces = local_faces(blk.kind);
            let ncorner = if blk.kind.dim() == 2 { 2 } else if blk.kind.is_simplex() { 3 } else { 4 };
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                for lf in &faces {
                    let nodes: Vec<usize> = lf.iter().map(|&l| conn[l]).collect();
                    let mut key: Vec<usize> = nodes[..ncorner].to_vec();
                    key.sort_unstable();
                    match map.get_mut(&key) {
                        Some(en) => en.count += 1,
                        None => {
                            order.push(key.clone());
                            map.insert(key, Entry { count: 1, face: BoundaryFace { nodes, block: bi, elem: e } });
                        }
                    }
                }
            }
        }
        order.into_iter().filter_map(|k| map.remove(&k)).filter(|e| e.count == 1).map(|e| e.face).collect()
    }

    /// Boundary faces whose centroid satisfies `pred`, registered as the named surface (node lists,
    /// ready for `Loads::faces`) and returned.
    pub fn select_faces(&mut self, name: &str, pred: impl Fn(&[f64; 3]) -> bool) -> &[Vec<usize>] {
        let faces: Vec<Vec<usize>> = self
            .boundary_faces()
            .into_iter()
            .filter(|f| {
                let k = f.nodes.len().min(if self.dim() == 2 { 2 } else { 4 });
                let k = if self.dim() == 3 && matches!(f.nodes.len(), 3 | 6) { 3 } else { k };
                let mut c = [0.0; 3];
                for &n in &f.nodes[..k] {
                    for i in 0..3 {
                        c[i] += self.nodes[n][i] / k as f64;
                    }
                }
                pred(&c)
            })
            .map(|f| f.nodes)
            .collect();
        self.surfaces.insert(name.to_string(), faces);
        &self.surfaces[name]
    }
}

impl Mesh {
    /// Register as surface `name` the boundary faces whose corner nodes all belong to the node set
    /// `set` (a way to build loadable surfaces from imported node sets).
    pub fn select_faces_by_nodes(&mut self, name: &str, set: &str) -> Result<&[Vec<usize>], String> {
        let members: std::collections::HashSet<usize> = self.node_set(set)?.iter().copied().collect();
        let dim = self.dim();
        let faces: Vec<Vec<usize>> = self
            .boundary_faces()
            .into_iter()
            .filter(|f| {
                let k = if dim == 2 { 2 } else if matches!(f.nodes.len(), 3 | 6) { 3 } else { 4 };
                f.nodes[..k].iter().all(|n| members.contains(n))
            })
            .map(|f| f.nodes)
            .collect();
        self.surfaces.insert(name.to_string(), faces);
        Ok(&self.surfaces[name])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ALL_KINDS;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn unit(kind: ElementKind, div: usize) -> Mesh {
        let phys = if kind.dim() == 3 { Physics::Solid } else { Physics::PlaneStrain { thickness: 1.0 } };
        grid(phys, kind, Elastic::new(1.0, 0.3), [div, div, div], &|p| p).unwrap()
    }

    /// Outward normal times measure of one face from its corner nodes.
    fn area_normal(m: &Mesh, nodes: &[usize]) -> [f64; 3] {
        let p = |i: usize| m.nodes[nodes[i]];
        let k = if matches!(nodes.len(), 3 | 6) { 3 } else { 4 };
        if m.dim() == 2 {
            let (a, b) = (p(0), p(1));
            return [b[1] - a[1], -(b[0] - a[0]), 0.0];
        }
        // Newell: sum of cross products around the polygon.
        let mut n = [0.0; 3];
        for i in 0..k {
            let (a, b) = (p(i), p((i + 1) % k));
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        [0.5 * n[0], 0.5 * n[1], 0.5 * n[2]]
    }

    #[test]
    fn boundary_of_a_unit_block_has_the_right_count_area_and_outward_normals() {
        for kind in ALL_KINDS {
            let m = unit(kind, 2);
            let faces = m.boundary_faces();
            let d = kind.dim();
            // Total boundary measure (perimeter 4 / surface 6) and closure: sum of area vectors = 0.
            let mut total = 0.0;
            let mut sum = [0.0; 3];
            let centre = [0.5; 3];
            for f in &faces {
                let an = area_normal(&m, &f.nodes);
                total += an.iter().map(|x| x * x).sum::<f64>().sqrt();
                for i in 0..3 {
                    sum[i] += an[i];
                }
                // Outward: area vector points away from the cube centre.
                let mut c = [0.0; 3];
                for &n in &f.nodes {
                    for i in 0..3 {
                        c[i] += m.nodes[n][i] / f.nodes.len() as f64;
                    }
                }
                let out: f64 = (0..d).map(|i| an[i] * (c[i] - centre[i])).sum();
                assert!(out > 0.0, "{kind:?}: face {:?} points inward", f.nodes);
            }
            let want = if d == 2 { 4.0 } else { 6.0 };
            assert!((total - want).abs() < 1e-12, "{kind:?}: boundary measure {total}");
            assert!(sum.iter().all(|x| x.abs() < 1e-12), "{kind:?}: faces do not close {sum:?}");
        }
    }

    #[test]
    fn face_node_counts_follow_the_element_order() {
        for (kind, want) in [
            (ElementKind::Quad4, 2),
            (ElementKind::Quad8, 3),
            (ElementKind::Tri6, 3),
            (ElementKind::Hex8, 4),
            (ElementKind::Hex20, 8),
            (ElementKind::Hex27, 9),
            (ElementKind::Tet4, 3),
            (ElementKind::Tet10, 6),
        ] {
            let m = unit(kind, 1);
            assert!(m.boundary_faces().iter().all(|f| f.nodes.len() == want), "{kind:?}");
        }
    }

    #[test]
    fn a_selected_pressure_face_integrates_to_pressure_times_area_on_every_3d_kind() {
        use crate::loads::{assemble, Loads, SurfaceLoad};
        for kind in [ElementKind::Tet4, ElementKind::Tet10, ElementKind::Hex8, ElementKind::Hex20, ElementKind::Hex27] {
            let mut m = unit(kind, 2);
            let faces = m.select_faces("top", |c| (c[2] - 1.0).abs() < 1e-9).to_vec();
            assert!(!faces.is_empty(), "{kind:?}");
            let loads = Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Pressure(3.0))).collect(), ..Default::default() };
            let f = assemble(&m, &loads).unwrap();
            let fz: f64 = (0..m.nodes.len()).map(|n| f[n * 3 + 2]).sum();
            // Pressure pushes into the body: top face (normal +z) gets force -p * area.
            assert!((fz + 3.0).abs() < 1e-12, "{kind:?}: total force {fz}");
        }
    }
}
