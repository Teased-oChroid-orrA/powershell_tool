//! Quality-guarded Laplacian smoothing of the simplex part of a mesh.
//!
//! A node moves only if it is a corner node of simplex elements only, lies on no boundary face and belongs to elements of one
//! block (so boundaries, interfaces between element families, and structured blocks keep their exact positions and the
//! mesh stays conforming with identical topology). A move to a relaxed neighbour average is kept only when the worst scaled
//! Jacobian of the elements around the node does not decrease; midside nodes of the edges at a moved node are re-placed at the
//! edge midpoints (the elements stay straight-sided where they were).
//!
//! Topological operations (edge flips, collapses, splits) are not implemented: the constrained Delaunay mesher already
//! returns Delaunay topology, whose flips cannot improve the angle criterion, and size changes belong to the adaptation
//! step.

use super::adjacency::Adjacency;
use super::quality::element_quality;
use crate::element::ElementKind;
use crate::mesh::Mesh;

#[derive(Debug, Clone)]
pub struct SmoothOptions {
    pub passes: usize,
    /// Fraction of the way to the neighbour average (`1` = full Laplacian step).
    pub relax: f64,
    /// Nodes that must not move.
    pub protect: Vec<usize>,
}

impl Default for SmoothOptions {
    fn default() -> Self {
        Self { passes: 10, relax: 0.7, protect: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmoothReport {
    pub movable_nodes: usize,
    pub moves: usize,
    pub passes_run: usize,
    pub min_scaled_jacobian_before: f64,
    pub min_scaled_jacobian_after: f64,
    pub mean_scaled_jacobian_before: f64,
    pub mean_scaled_jacobian_after: f64,
}

/// `(midside local index, corner a, corner b)` of every midside node of a simplex element kind.
fn midside_pairs(kind: ElementKind) -> Vec<(usize, usize, usize)> {
    if !kind.is_simplex() || kind.n_nodes() == kind.n_corners() {
        return Vec::new();
    }
    let nc = kind.node_coords();
    (kind.n_corners()..kind.n_nodes())
        .map(|m| {
            let mut found = None;
            for a in 0..kind.n_corners() {
                for b in a + 1..kind.n_corners() {
                    if (0..3).all(|c| (nc[m][c] - 0.5 * (nc[a][c] + nc[b][c])).abs() < 1e-12) {
                        found = Some((m, a, b));
                    }
                }
            }
            found.expect("a simplex midside node lies at an edge midpoint")
        })
        .collect()
}

fn element_min_jacobian(mesh: &Mesh, block: usize, e: usize) -> f64 {
    let blk = &mesh.blocks[block];
    let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| mesh.nodes[n]).collect();
    element_quality(blk.kind, &xyz).scaled_jacobian
}

/// Smooth the movable nodes of `mesh` in place.
pub fn smooth(mesh: &mut Mesh, opt: &SmoothOptions) -> SmoothReport {
    let adj = Adjacency::new(mesh);
    let n = mesh.nodes.len();
    let mut fixed = vec![false; n];
    for f in adj.boundary_faces() {
        let (g, lf) = f.owners[0];
        let (b, e) = adj.local(g as usize);
        let conn = mesh.blocks[b].elem(e);
        for &l in &crate::topology::local_faces(mesh.blocks[b].kind)[lf as usize] {
            fixed[conn[l]] = true;
        }
    }
    for &p in &opt.protect {
        fixed[p] = true;
    }
    // per node: corner role only, simplex elements only, one block only
    let mut corner_only = vec![true; n];
    let mut block_of = vec![usize::MAX; n];
    for (b, blk) in mesh.blocks.iter().enumerate() {
        let nc = blk.kind.n_corners();
        for e in 0..blk.n_elems() {
            for (a, &nd) in blk.elem(e).iter().enumerate() {
                if a >= nc || !blk.kind.is_simplex() {
                    corner_only[nd] = false;
                }
                if block_of[nd] == usize::MAX {
                    block_of[nd] = b;
                } else if block_of[nd] != b {
                    corner_only[nd] = false;
                }
            }
        }
    }
    let movable: Vec<usize> = (0..n).filter(|&v| !fixed[v] && corner_only[v] && block_of[v] != usize::MAX).collect();

    // corner neighbours through element edges, and midside bookkeeping
    let pairs: Vec<Vec<(usize, usize, usize)>> = mesh.blocks.iter().map(|b| midside_pairs(b.kind)).collect();
    let before = mesh.quality();
    let mut moves = 0;
    let mut passes_run = 0;
    for _ in 0..opt.passes {
        let mut moved_this_pass = 0;
        for &v in &movable {
            let elems: Vec<(usize, usize)> = adj.node_elements(v).iter().map(|&g| adj.local(g as usize)).collect();
            let mut nbrs: Vec<usize> = Vec::new();
            for &(b, e) in &elems {
                let blk = &mesh.blocks[b];
                for &c in &blk.elem(e)[..blk.kind.n_corners()] {
                    if c != v && !nbrs.contains(&c) {
                        nbrs.push(c);
                    }
                }
            }
            if nbrs.is_empty() {
                continue;
            }
            let mut target = [0.0; 3];
            for &c in &nbrs {
                for k in 0..3 {
                    target[k] += mesh.nodes[c][k] / nbrs.len() as f64;
                }
            }
            let old = mesh.nodes[v];
            let worst_before = elems.iter().map(|&(b, e)| element_min_jacobian(mesh, b, e)).fold(f64::INFINITY, f64::min);
            // midside nodes of the edges at v, with their old positions
            let mut mids: Vec<(usize, [f64; 3])> = Vec::new();
            for &(b, e) in &elems {
                let conn = mesh.blocks[b].elem(e);
                for &(m, ca, cb) in &pairs[b] {
                    if conn[ca] == v || conn[cb] == v {
                        mids.push((conn[m], mesh.nodes[conn[m]]));
                    }
                }
            }
            let dim = mesh.dim();
            for k in 0..dim {
                mesh.nodes[v][k] = old[k] + opt.relax * (target[k] - old[k]);
            }
            for &(b, e) in &elems {
                let conn: Vec<usize> = mesh.blocks[b].elem(e).to_vec();
                for &(m, ca, cb) in &pairs[b] {
                    if conn[ca] == v || conn[cb] == v {
                        for k in 0..3 {
                            mesh.nodes[conn[m]][k] = 0.5 * (mesh.nodes[conn[ca]][k] + mesh.nodes[conn[cb]][k]);
                        }
                    }
                }
            }
            let worst_after = elems.iter().map(|&(b, e)| element_min_jacobian(mesh, b, e)).fold(f64::INFINITY, f64::min);
            if worst_after > worst_before + 1e-12 {
                moved_this_pass += 1;
            } else {
                mesh.nodes[v] = old;
                for (m, pos) in mids {
                    mesh.nodes[m] = pos;
                }
            }
        }
        passes_run += 1;
        moves += moved_this_pass;
        if moved_this_pass == 0 {
            break;
        }
    }
    let after = mesh.quality();
    SmoothReport {
        movable_nodes: movable.len(),
        moves,
        passes_run,
        min_scaled_jacobian_before: before.min_scaled_jacobian,
        min_scaled_jacobian_after: after.min_scaled_jacobian,
        mean_scaled_jacobian_before: before.mean_scaled_jacobian,
        mean_scaled_jacobian_after: after.mean_scaled_jacobian,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    #[test]
    fn a_distorted_interior_node_returns_toward_the_centre_and_the_boundary_stays_put() {
        let mut m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, Elastic::new(1.0, 0.3), [6, 6, 1], &|p| p).unwrap();
        let centre = m.nodes.iter().position(|x| (x[0] - 0.5).abs() < 1e-12 && (x[1] - 0.5).abs() < 1e-12).unwrap();
        // distort: pull the centre node and its midside nodes consistently towards a corner
        m.nodes[centre] = [0.62, 0.58, 0.0];
        let boundary_before: Vec<[f64; 3]> = m.nodes.iter().copied().filter(|x| x[0] == 0.0 || x[0] == 1.0 || x[1] == 0.0 || x[1] == 1.0).collect();
        let q0 = m.quality();
        let rep = smooth(&mut m, &SmoothOptions::default());
        assert!(rep.moves > 0 && rep.movable_nodes > 0);
        assert!(rep.min_scaled_jacobian_after >= rep.min_scaled_jacobian_before - 1e-12);
        assert!(rep.mean_scaled_jacobian_after >= q0.mean_scaled_jacobian - 1e-12);
        let boundary_after: Vec<[f64; 3]> = m.nodes.iter().copied().filter(|x| x[0] == 0.0 || x[0] == 1.0 || x[1] == 0.0 || x[1] == 1.0).collect();
        assert_eq!(boundary_before, boundary_after);
        assert!(m.conformity().is_conforming());
        assert!((m.nodes[centre][0] - 0.5).abs() < 0.05 && (m.nodes[centre][1] - 0.5).abs() < 0.05, "{:?}", m.nodes[centre]);
        // midside nodes of the edges at the centre sit at the new edge midpoints
        let blk = &m.blocks[0];
        for e in 0..blk.n_elems() {
            let c = blk.elem(e);
            if c.contains(&centre) {
                for (k, (a, b)) in [(0usize, 1usize), (1, 2), (2, 0)].into_iter().enumerate() {
                    if c[a] == centre || c[b] == centre {
                        for ax in 0..2 {
                            assert!((m.nodes[c[3 + k]][ax] - 0.5 * (m.nodes[c[a]][ax] + m.nodes[c[b]][ax])).abs() < 1e-12);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn structured_quads_and_block_interfaces_are_untouched() {
        let mut m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(1.0, 0.3), [4, 4, 1], &|p| p).unwrap();
        let before = m.nodes.clone();
        let rep = smooth(&mut m, &SmoothOptions::default());
        assert_eq!(rep.movable_nodes, 0);
        assert_eq!(before, m.nodes);
    }

    #[test]
    fn a_tetrahedral_interior_node_is_smoothed_without_breaking_the_mesh() {
        let mut m = grid(Physics::Solid, ElementKind::Tet10, Elastic::new(1.0, 0.3), [3, 3, 3], &|p| p).unwrap();
        let inner: Vec<usize> = m.nodes.iter().enumerate().filter(|(_, x)| x.iter().all(|&c| c > 1e-9 && c < 1.0 - 1e-9) && (x[0] * 3.0).fract().abs() < 1e-9 && (x[1] * 3.0).fract().abs() < 1e-9 && (x[2] * 3.0).fract().abs() < 1e-9).map(|(i, _)| i).collect();
        assert!(!inner.is_empty());
        let v = inner[0];
        m.nodes[v][0] += 0.08;
        let rep = smooth(&mut m, &SmoothOptions::default());
        assert!(rep.moves > 0);
        assert!(rep.min_scaled_jacobian_after >= rep.min_scaled_jacobian_before - 1e-12);
        assert!(m.conformity().is_conforming());
    }
}
