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
        debug_assert_colouring(mesh, &pat);
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

    /// Proves, for `mesh`, what the unsafe parallel scatter relies on (see [`Shared`]):
    /// * the pattern fits the mesh (node count, one scatter table and element count per block, nodes in range);
    /// * every element is in exactly one colour class and no two elements of a class share a node;
    /// * every scatter position is `NO_BLOCK` (strict upper triangle) or exactly the in-bounds block `find(ga, gb)` of
    ///   the element's own node pair, so a write can only land in a block owned by one of the element's nodes.
    ///
    /// Linear in the element connectivity (times a binary search per scatter entry).
    pub(crate) fn check_colouring(&self, mesh: &Mesh) -> Result<(), String> {
        let n = mesh.nodes.len();
        // `d` is only the block size (a scalar thermal pattern, `d = 1`, runs over a 2D / 3D mesh), so it is not compared.
        if self.n_nodes != n {
            return Err(format!("pattern is for {} nodes, the mesh has {n}", self.n_nodes));
        }
        if self.col_ptr.len() != n + 1 || self.col_ptr.last() != Some(&self.row_idx.len()) {
            return Err("pattern col_ptr does not span row_idx".into());
        }
        if self.scatter.len() != mesh.blocks.len() {
            return Err(format!("{} scatter tables for {} blocks", self.scatter.len(), mesh.blocks.len()));
        }
        let n_blocks = self.n_blocks();
        for (bi, blk) in mesh.blocks.iter().enumerate() {
            let nn = blk.kind.n_nodes();
            if self.scatter[bi].len() != blk.n_elems() * nn * nn {
                return Err(format!("block {bi}: scatter table has {} entries for {} elements", self.scatter[bi].len(), blk.n_elems()));
            }
            if let Some(&bad) = blk.conn.iter().find(|&&nd| nd >= n) {
                return Err(format!("block {bi} refers to node {bad} of {n}"));
            }
            for (e, conn) in blk.conn.chunks_exact(nn).enumerate() {
                let map = &self.scatter[bi][e * nn * nn..(e + 1) * nn * nn];
                for (b, &gb) in conn.iter().enumerate() {
                    for (a, &ga) in conn.iter().enumerate() {
                        let pos = map[a * nn + b];
                        if ga < gb {
                            if pos != NO_BLOCK {
                                return Err(format!("block {bi} element {e}: upper-triangle pair ({ga}, {gb}) has scatter position {pos}"));
                            }
                        } else if pos == NO_BLOCK || pos as usize >= n_blocks {
                            return Err(format!("block {bi} element {e}: pair ({ga}, {gb}) has scatter position {pos}, out of the {n_blocks} blocks"));
                        } else if self.find(ga, gb) != Some(pos as usize) {
                            return Err(format!("block {bi} element {e}: pair ({ga}, {gb}) scatters to block {pos}, not its own"));
                        }
                    }
                }
            }
        }
        let mut seen: Vec<Vec<bool>> = mesh.blocks.iter().map(|b| vec![false; b.n_elems()]).collect();
        // Which (colour, element ordinal in that colour) last claimed a node.
        let mut owner = vec![(u32::MAX, 0u32); n];
        for (c, class) in self.colors.iter().enumerate() {
            for (k, &(bi, e)) in class.iter().enumerate() {
                let (bi, e) = (bi as usize, e as usize);
                let Some(was) = seen.get_mut(bi).and_then(|s| s.get_mut(e)) else {
                    return Err(format!("colour {c} lists missing element {e} of block {bi}"));
                };
                if std::mem::replace(was, true) {
                    return Err(format!("element {e} of block {bi} is in more than one colour slot"));
                }
                for &nd in mesh.blocks[bi].elem(e) {
                    let prev = std::mem::replace(&mut owner[nd], (c as u32, k as u32));
                    if prev.0 == c as u32 && prev.1 != k as u32 {
                        return Err(format!("colour {c}: node {nd} is shared by two elements (block {bi} element {e} and slot {})", prev.1));
                    }
                }
            }
        }
        match seen.iter().enumerate().find_map(|(bi, s)| s.iter().position(|&x| !x).map(|e| (bi, e))) {
            Some((bi, e)) => Err(format!("element {e} of block {bi} is in no colour")),
            None => Ok(()),
        }
    }
}

/// Checks the colouring invariant under `debug_assert!`: panics (debug and test builds only) with the reason.
pub(crate) fn debug_assert_colouring(mesh: &Mesh, pat: &Pattern) {
    debug_assert!(pat.check_colouring(mesh).is_ok(), "broken element colouring: {}", pat.check_colouring(mesh).unwrap_err());
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

/// Per-thread element buffers of the colour-parallel assemblies (about 150 KB).
///
/// `try_for_each_init` builds its state once per rayon task, and a colour class of a small model splits into many
/// tasks, so zero-filling these buffers took 29 % of the samples of a 2D lug solve (`docs/performance-profile-2026-11.md`).
/// They live in a `thread_local!` instead: one allocation per thread for its lifetime. No kernel reads a buffer before
/// writing it (the per-task state was already reused across elements), so results do not depend on the history.
pub(crate) struct Scratch {
    pub work: Work,
    pub xyz: Vec<[f64; 3]>,
    /// Scalar element matrix of `assemble_scalar_identity`.
    pub me: Vec<f64>,
    /// Displacements and forces of one element (`assemble_nl`).
    pub ue: Vec<f64>,
    pub fe: Vec<f64>,
    /// Element tangent of `assemble_nl` (`9 MAX_NODES^2`).
    pub ke: Vec<f64>,
    /// Element stiffness of the fast kernel (`10 MAX_NODES^2`).
    pub kk: Vec<f64>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            work: Work::new(),
            xyz: vec![[0.0; 3]; MAX_NODES],
            me: vec![0.0; MAX_NODES * MAX_NODES],
            ue: vec![0.0; 3 * MAX_NODES],
            fe: vec![0.0; 3 * MAX_NODES],
            ke: vec![0.0; 9 * MAX_NODES * MAX_NODES],
            kk: vec![0.0; MAX_NODES * MAX_NODES * 10],
        }
    }
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Option<Scratch>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with this thread's [`Scratch`]. A re-entrant call (an element callback that itself assembles, or a rayon
/// thread that steals a task while blocked inside `f`) gets a fresh one instead of a second borrow.
pub(crate) fn with_scratch<R>(f: impl FnOnce(&mut Scratch) -> R) -> R {
    SCRATCH.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => f(slot.get_or_insert_with(Scratch::new)),
        Err(_) => f(&mut Scratch::new()),
    })
}

/// Raw pointer that may cross threads, for the colour-parallel scatter of the assemblies (this module and
/// `nonlinear.rs`).
///
/// **Contract** (what makes the `unsafe impl`s sound): the pointee outlives the parallel region, nothing else reads or
/// writes it meanwhile, and every thread writes only through offsets no other thread of the same region touches. The
/// assemblies get that from `Pattern::colors`: the elements of one colour class share no node, hence no node-pair
/// block, force entry or Gauss-point state, and the classes run one after the other. `Pattern::check_colouring` proves
/// it for a given mesh and runs under `debug_assert!` wherever a `Shared` is made. `T: Send` because the values behind
/// the pointer are written from other threads.
pub(crate) struct Shared<T>(pub(crate) *mut T);
// SAFETY: see the contract on `Shared`.
unsafe impl<T: Send> Send for Shared<T> {}
// SAFETY: see the contract on `Shared`.
unsafe impl<T: Send> Sync for Shared<T> {}

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
    debug_assert_colouring(mesh, pat);
    let shared = Shared(mat.vals.as_mut_ptr());
    let shared = &shared;
    for class in &pat.colors {
        class
            .par_iter()
            .try_for_each(|&(bi, e)| -> Result<(), ElementError> {
                with_scratch(|Scratch { work, xyz, kk: ke, .. }| -> Result<(), ElementError> {
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
                })
            })
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
    debug_assert_colouring(mesh, pat);
    let shared = Shared(mat.vals.as_mut_ptr());
    let shared = &shared;
    for class in &pat.colors {
        class
            .par_iter()
            .try_for_each(|&(bi, e)| -> Result<(), ElementError> {
                with_scratch(|Scratch { work, xyz, me, .. }| -> Result<(), ElementError> {
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
                })
            })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementKind, ALL_KINDS};
    use crate::mesh::{Elastic, Physics};

    const POOL: usize = 60;
    const ELEMS_PER_KIND: usize = 12;

    /// Elements of every kind in `kinds` on a shared pool of nodes, each element a window of consecutive nodes starting
    /// at a different place, so elements overlap heavily, within a block and across blocks. Pattern building never reads
    /// coordinates, so the geometry is irrelevant.
    fn overlapping_mesh(physics: Physics, kinds: &[ElementKind]) -> Mesh {
        let mut mesh = Mesh::new(physics);
        for i in 0..POOL {
            mesh.add_node([i as f64, 0.0, 0.0]);
        }
        for (k, &kind) in kinds.iter().enumerate() {
            let nn = kind.n_nodes();
            let conn: Vec<usize> = (0..ELEMS_PER_KIND).flat_map(|e| (0..nn).map(move |a| (e * 2 + k * 3 + a) % POOL)).collect();
            mesh.add_block(kind, conn, Elastic::new(1.0, 0.3), &format!("{kind:?}")).unwrap();
        }
        mesh
    }

    fn kinds_of_dim(dim: usize) -> Vec<ElementKind> {
        ALL_KINDS.iter().copied().filter(|k| k.dim() == dim).collect()
    }

    #[test]
    fn every_element_kind_has_a_valid_colouring() {
        for kind in ALL_KINDS {
            let physics = if kind.dim() == 3 { Physics::Solid } else { Physics::PlaneStress { thickness: 1.0 } };
            let mesh = overlapping_mesh(physics, &[kind]);
            let pat = Pattern::new(&mesh).unwrap();
            assert!(pat.n_colors() > 1, "{kind:?}: the test mesh must make elements share nodes");
            pat.check_colouring(&mesh).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        }
    }

    #[test]
    fn mixed_blocks_and_every_physics_have_a_valid_colouring() {
        let planar = kinds_of_dim(2);
        for physics in [Physics::PlaneStress { thickness: 1.0 }, Physics::PlaneStrain { thickness: 1.0 }, Physics::Axisymmetric] {
            let mesh = overlapping_mesh(physics, &planar);
            let pat = Pattern::new(&mesh).unwrap();
            pat.check_colouring(&mesh).unwrap_or_else(|e| panic!("{physics:?}: {e}"));
        }
        let mesh = overlapping_mesh(Physics::Solid, &kinds_of_dim(3));
        let pat = Pattern::new(&mesh).unwrap();
        assert!(kinds_of_dim(3).contains(&ElementKind::Wedge6) && kinds_of_dim(3).contains(&ElementKind::Pyramid5));
        pat.check_colouring(&mesh).unwrap();
        // Extra coupled pairs add pattern blocks but never scatter targets: the check still holds.
        let pat = Pattern::new_with_extra(&mesh, &[(0, 59), (3, 41)]).unwrap();
        pat.check_colouring(&mesh).unwrap();
    }

    fn quad_pattern() -> (Mesh, Pattern) {
        let mesh = overlapping_mesh(Physics::PlaneStress { thickness: 1.0 }, &[ElementKind::Quad4, ElementKind::Tri3]);
        let pat = Pattern::new(&mesh).unwrap();
        pat.check_colouring(&mesh).unwrap();
        assert!(pat.n_colors() > 1);
        (mesh, pat)
    }

    #[test]
    fn a_bad_colouring_is_rejected() {
        // Greedy colouring put every colour-1 element there because it shares a node with a colour-0 element:
        // moving one into colour 0 makes a class whose elements share a node.
        let (mesh, mut pat) = quad_pattern();
        let moved = pat.colors[1].remove(0);
        pat.colors[0].push(moved);
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("shared by two elements"), "{err}");

        // An element listed twice.
        let (mesh, mut pat) = quad_pattern();
        let dup = pat.colors[0][0];
        pat.colors[1].push(dup);
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("more than one colour slot"), "{err}");

        // An element left out of every colour.
        let (mesh, mut pat) = quad_pattern();
        pat.colors[0].pop();
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("in no colour"), "{err}");

        // An element that does not exist.
        let (mesh, mut pat) = quad_pattern();
        pat.colors[0].push((0, 10_000));
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("missing element"), "{err}");
    }

    #[test]
    fn a_bad_scatter_map_is_rejected() {
        // Position past the end of the value array.
        let (mesh, mut pat) = quad_pattern();
        let n = pat.n_blocks() as u32;
        let i = pat.scatter[0].iter().position(|&p| p != NO_BLOCK).unwrap();
        pat.scatter[0][i] = n;
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("out of the"), "{err}");

        // In bounds but another node pair's block.
        let (mesh, mut pat) = quad_pattern();
        let i = pat.scatter[0].iter().position(|&p| p != NO_BLOCK).unwrap();
        pat.scatter[0][i] = (pat.scatter[0][i] + 1) % n;
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("not its own"), "{err}");

        // A write for a pair of the strict upper triangle.
        let (mesh, mut pat) = quad_pattern();
        let i = pat.scatter[0].iter().position(|&p| p == NO_BLOCK).unwrap();
        pat.scatter[0][i] = 0;
        let err = pat.check_colouring(&mesh).unwrap_err();
        assert!(err.contains("upper-triangle"), "{err}");

        // A pattern for another mesh.
        let (mut mesh, pat) = quad_pattern();
        mesh.add_node([0.0; 3]);
        assert!(pat.check_colouring(&mesh).is_err());
    }

    /// The assemblies run the check under `debug_assert!`: a broken colouring stops a test build instead of racing.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "broken element colouring")]
    fn assembling_with_a_bad_colouring_panics_in_debug_builds() {
        let (mesh, mut pat) = quad_pattern();
        let moved = pat.colors[1].remove(0);
        pat.colors[0].push(moved);
        let _ = assemble_stiffness(&mesh, &pat);
    }
}
