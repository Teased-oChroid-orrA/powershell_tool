//! Symmetric block-sparse assembly.
//!
//! The matrix is stored as the lower triangle of `d x d` node blocks in compressed-column form
//! (`col_ptr` over nodes, `row_idx` the nodes `i >= j` coupled to node `j`). The pattern and the
//! element-to-block scatter map are built once per mesh; every later assembly (a nonlinear
//! iteration, a new load case) only recomputes the element matrices and adds them at precomputed
//! positions. Elements are coloured so that no two elements of a colour share a node: they write
//! disjoint blocks and run in parallel without locks, and the sums are bit-for-bit reproducible
//! (the order of additions into a block is fixed by the colour order).

use crate::element::MAX_NODES;
use crate::kernel::{self, ElementError, Work};
use crate::mesh::Mesh;
use rayon::prelude::*;

pub(crate) const NO_BLOCK: u32 = u32::MAX;

/// Sparsity pattern (lower block triangle, compressed column) plus the scatter map.
#[derive(Debug)]
pub struct Pattern {
    pub n_nodes: usize,
    pub d: usize,
    pub col_ptr: Vec<usize>,
    pub row_idx: Vec<u32>,
    /// Per mesh block, per element: `nn * nn` block positions (`NO_BLOCK` for pairs in the strict
    /// upper triangle, which the symmetric partner supplies).
    pub(crate) scatter: Vec<Vec<u32>>,
    /// Elements grouped by colour as `(block, element)`.
    pub(crate) colors: Vec<Vec<(u32, u32)>>,
}

impl Pattern {
    pub fn new(mesh: &Mesh) -> Result<Self, String> {
        Self::new_with_extra(mesh, &[])
    }

    /// Pattern of the mesh plus extra coupled node pairs (contact between non-adjacent elements).
    pub fn new_with_extra(mesh: &Mesh, extra: &[(usize, usize)]) -> Result<Self, String> {
        let n = mesh.nodes.len();
        let mut cols: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(a, b) in extra {
            if a >= n || b >= n {
                return Err(format!("extra coupling ({a}, {b}) refers to a missing node"));
            }
            let (hi, lo) = (a.max(b), a.min(b));
            cols[lo].push(hi as u32);
        }
        for blk in &mesh.blocks {
            let nn = blk.kind.n_nodes();
            for conn in blk.conn.chunks_exact(nn) {
                for &gb in conn {
                    for &ga in conn {
                        if ga >= gb {
                            cols[gb].push(ga as u32);
                        }
                    }
                }
            }
        }
        // Every node needs its diagonal block even when no element touches it (kept so the
        // reduced matrix is well formed; such a node must be constrained by the caller).
        let mut col_ptr = Vec::with_capacity(n + 1);
        let mut row_idx = Vec::new();
        col_ptr.push(0);
        for (j, c) in cols.iter_mut().enumerate() {
            c.push(j as u32);
            c.sort_unstable();
            c.dedup();
            row_idx.extend_from_slice(c);
            col_ptr.push(row_idx.len());
        }
        let mut pat = Self { n_nodes: n, d: mesh.dim(), col_ptr, row_idx, scatter: Vec::new(), colors: Vec::new() };
        pat.scatter = mesh
            .blocks
            .iter()
            .map(|blk| {
                let nn = blk.kind.n_nodes();
                let mut map = vec![NO_BLOCK; blk.n_elems() * nn * nn];
                for (e, conn) in blk.conn.chunks_exact(nn).enumerate() {
                    for (b, &gb) in conn.iter().enumerate() {
                        for (a, &ga) in conn.iter().enumerate() {
                            if ga >= gb {
                                map[e * nn * nn + a * nn + b] = pat.find(ga, gb).expect("pattern covers every element pair") as u32;
                            }
                        }
                    }
                }
                map
            })
            .collect();
        pat.colors = colour_elements(mesh)?;
        Ok(pat)
    }

    /// Index of block `(row node, col node)` in the value array.
    pub fn find(&self, row: usize, col: usize) -> Option<usize> {
        let rows = &self.row_idx[self.col_ptr[col]..self.col_ptr[col + 1]];
        rows.binary_search(&(row as u32)).ok().map(|k| self.col_ptr[col] + k)
    }

    pub fn n_blocks(&self) -> usize {
        self.row_idx.len()
    }

    pub fn n_colors(&self) -> usize {
        self.colors.len()
    }
}

/// Greedy colouring: the smallest colour no element sharing a node already has.
fn colour_elements(mesh: &Mesh) -> Result<Vec<Vec<(u32, u32)>>, String> {
    const WORDS: usize = 8; // 512 colours: far beyond any valid mesh
    let mut used = vec![0u64; mesh.nodes.len() * WORDS];
    let mut colors: Vec<Vec<(u32, u32)>> = Vec::new();
    for (bi, blk) in mesh.blocks.iter().enumerate() {
        let nn = blk.kind.n_nodes();
        for (e, conn) in blk.conn.chunks_exact(nn).enumerate() {
            let mut busy = [0u64; WORDS];
            for &nd in conn {
                for w in 0..WORDS {
                    busy[w] |= used[nd * WORDS + w];
                }
            }
            let c = (0..WORDS * 64).find(|&c| busy[c / 64] >> (c % 64) & 1 == 0).ok_or("element colouring needs more than 512 colours (degenerate mesh)")?;
            for &nd in conn {
                used[nd * WORDS + c / 64] |= 1 << (c % 64);
            }
            if c >= colors.len() {
                colors.resize(c + 1, Vec::new());
            }
            colors[c].push((bi as u32, e as u32));
        }
    }
    Ok(colors)
}

/// The assembled symmetric matrix (lower block triangle).
#[derive(Debug, Clone)]
pub struct BlockMatrix {
    pub d: usize,
    pub vals: Vec<f64>,
}

/// Raw pointer that may cross threads: used only for colour-disjoint block writes.
struct Shared(*mut f64);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl BlockMatrix {
    pub fn zeros(pat: &Pattern) -> Self {
        Self { d: pat.d, vals: vec![0.0; pat.n_blocks() * pat.d * pat.d] }
    }

    /// `y += K x` (full symmetric product from the stored lower triangle).
    pub fn matvec_add(&self, pat: &Pattern, x: &[f64], y: &mut [f64]) {
        let d = self.d;
        for j in 0..pat.n_nodes {
            for k in pat.col_ptr[j]..pat.col_ptr[j + 1] {
                let i = pat.row_idx[k] as usize;
                let b = &self.vals[k * d * d..(k + 1) * d * d];
                for p in 0..d {
                    let mut s = 0.0;
                    for q in 0..d {
                        s += b[p * d + q] * x[j * d + q];
                    }
                    y[i * d + p] += s;
                }
                if i != j {
                    for q in 0..d {
                        let mut s = 0.0;
                        for p in 0..d {
                            s += b[p * d + q] * x[i * d + p];
                        }
                        y[j * d + q] += s;
                    }
                }
            }
        }
    }

    /// Diagonal of the matrix (dof order), for scaling and preconditioning.
    pub fn diagonal(&self, pat: &Pattern) -> Vec<f64> {
        let d = self.d;
        let mut out = vec![0.0; pat.n_nodes * d];
        for j in 0..pat.n_nodes {
            let k = pat.find(j, j).expect("diagonal block");
            for p in 0..d {
                out[j * d + p] = self.vals[k * d * d + p * d + p];
            }
        }
        out
    }
}

/// Assemble the global stiffness matrix. Parallel over colour classes.
pub fn assemble_stiffness(mesh: &Mesh, pat: &Pattern) -> Result<BlockMatrix, String> {
    let mut mat = BlockMatrix::zeros(pat);
    let d = pat.d;
    let dd = d * d;
    let shared = Shared(mat.vals.as_mut_ptr());
    let shared = &shared;
    for class in &pat.colors {
        class
            .par_iter()
            .try_for_each_init(
                || (Work::new(), vec![0.0f64; MAX_NODES * MAX_NODES * 10], vec![[0.0f64; 3]; MAX_NODES]),
                |(work, ke, xyz), &(bi, e)| -> Result<(), ElementError> {
                    let blk = &mesh.blocks[bi as usize];
                    let nn = blk.kind.n_nodes();
                    let conn = blk.elem(e as usize);
                    for (a, &nd) in conn.iter().enumerate() {
                        xyz[a] = mesh.nodes[nd];
                    }
                    let layout = kernel::stiffness_fast(blk.kind, mesh.physics, &blk.material, &xyz[..nn], work, ke)?;
                    let map = &pat.scatter[bi as usize][e as usize * nn * nn..(e as usize + 1) * nn * nn];
                    for a in 0..nn {
                        for b in 0..nn {
                            let pos = map[a * nn + b];
                            if pos == NO_BLOCK {
                                continue;
                            }
                            // SAFETY: elements of one colour share no node, so they touch disjoint
                            // node-pair blocks; `pos` is in bounds by construction of the pattern.
                            unsafe {
                                let dst = shared.0.add(pos as usize * dd);
                                for i in 0..d {
                                    for j in 0..d {
                                        *dst.add(i * d + j) += kernel::entry(layout, ke, nn, d, a, b, i, j);
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(mat)
}

/// Assemble a matrix whose node-pair blocks are a scalar times the identity (consistent mass, geometric stiffness):
/// `element(block, element, xyz, work, out)` fills the element's `nn x nn` scalars. Same colouring and scatter as the
/// stiffness.
pub fn assemble_scalar_identity<F>(mesh: &Mesh, pat: &Pattern, element: F) -> Result<BlockMatrix, String>
where
    F: Fn(usize, usize, &[[f64; 3]], &mut Work, &mut [f64]) -> Result<(), ElementError> + Sync,
{
    let mut mat = BlockMatrix::zeros(pat);
    let d = pat.d;
    let dd = d * d;
    let shared = Shared(mat.vals.as_mut_ptr());
    let shared = &shared;
    for class in &pat.colors {
        class
            .par_iter()
            .try_for_each_init(
                || (Work::new(), vec![0.0f64; MAX_NODES * MAX_NODES], vec![[0.0f64; 3]; MAX_NODES]),
                |(work, me, xyz), &(bi, e)| -> Result<(), ElementError> {
                    let blk = &mesh.blocks[bi as usize];
                    let nn = blk.kind.n_nodes();
                    for (a, &nd) in blk.elem(e as usize).iter().enumerate() {
                        xyz[a] = mesh.nodes[nd];
                    }
                    element(bi as usize, e as usize, &xyz[..nn], work, me)?;
                    let map = &pat.scatter[bi as usize][e as usize * nn * nn..(e as usize + 1) * nn * nn];
                    for a in 0..nn {
                        for b in 0..nn {
                            let pos = map[a * nn + b];
                            if pos == NO_BLOCK {
                                continue;
                            }
                            // SAFETY: as in `assemble_stiffness`: one colour's elements share no node.
                            unsafe {
                                let dst = shared.0.add(pos as usize * dd);
                                for i in 0..d {
                                    *dst.add(i * d + i) += me[a * nn + b];
                                }
                            }
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(mat)
}

impl BlockMatrix {
    /// `self + alpha * other` (same pattern).
    pub fn axpy(&self, alpha: f64, other: &BlockMatrix) -> BlockMatrix {
        BlockMatrix { d: self.d, vals: self.vals.iter().zip(&other.vals).map(|(a, b)| a + alpha * b).collect() }
    }
}
