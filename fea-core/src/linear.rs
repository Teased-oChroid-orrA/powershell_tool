//! Linear solution of `K u = f` with Dirichlet constraints.
//!
//! Constrained dofs are eliminated (not penalised): the reduced matrix over the free dofs is
//! gathered from the assembled blocks through a precomputed index map, its symbolic Cholesky
//! (fill-reducing ordering, supernodal analysis) is done once per constraint set, and every later
//! numeric factorization only refills values. The right-hand side is corrected for prescribed
//! non-zero values by one symmetric product.

use crate::assembly::{BlockMatrix, Pattern};
use crate::ordering;
use dyn_stack::{MemBuffer, MemStack};
use faer::perm::PermRef;
use faer::linalg::cholesky::ldlt::factor::LdltRegularization;
use faer::sparse::linalg::cholesky::{factorize_symbolic_cholesky, CholeskySymbolicParams, LdltRef, LltRef, SymbolicCholesky, SymmetricOrdering};
use faer::sparse::{SparseColMatRef, SymbolicSparseColMatRef};
use faer::{get_global_parallelism, Conj, MatMut, Par, Side};
use std::sync::Arc;

const FIXED: u32 = u32::MAX;

/// Prescribed displacements.
#[derive(Debug, Clone)]
pub struct Dirichlet {
    pub d: usize,
    pub fixed: Vec<bool>,
    pub value: Vec<f64>,
}

impl Dirichlet {
    pub fn new(n_nodes: usize, d: usize) -> Self {
        Self { d, fixed: vec![false; n_nodes * d], value: vec![0.0; n_nodes * d] }
    }

    /// Prescribe displacement component `comp` of `node`.
    pub fn fix(&mut self, node: usize, comp: usize, value: f64) {
        self.fixed[node * self.d + comp] = true;
        self.value[node * self.d + comp] = value;
    }

    /// Prescribe zero displacement of every component of `node`.
    pub fn fix_node(&mut self, node: usize) {
        for c in 0..self.d {
            self.fix(node, c, 0.0);
        }
    }

    pub fn n_fixed(&self) -> usize {
        self.fixed.iter().filter(|f| **f).count()
    }
}

/// Fill-reducing ordering of the reduced system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    /// Approximate minimum degree (general graphs).
    Amd,
    /// Geometric nested dissection on the node graph (needs node coordinates; best on meshes).
    NestedDissection,
    /// Minimum degree in 2D; in 3D both are analysed and the sparser factor kept (nested
    /// dissection wins 2-3x on trilinear hexahedra, minimum degree on quadratic ones).
    Auto,
}

/// The reduced system structure for one constraint set.
pub struct Reduced {
    pat: Arc<Pattern>,
    /// New index of every dof (`FIXED` if constrained).
    free_dofs: Vec<u32>,
    col_ptr: Vec<u32>,
    row_idx: Vec<u32>,
    /// Source index in the block value array of every reduced entry.
    src: Vec<u32>,
    /// `None` for structure-only systems solved iteratively.
    symbolic: Option<SymbolicCholesky<u32>>,
}

/// Free dofs below which faer factorises and solves sequentially: at the sizes of a 2D lug (5-10k dofs) its
/// threads cost more than they save (measured: 3.4 s sequential against 6.0 s on 8 threads for the same
/// 165 factorisations of a 10k-dof lug), while the assembly keeps its rayon parallelism.
const SEQUENTIAL_BELOW: usize = 40_000;

fn parallelism_for(n: usize) -> Par {
    if n < SEQUENTIAL_BELOW {
        Par::Seq
    } else {
        get_global_parallelism()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolveError {
    /// Every dof constrained, or no dof at all.
    NothingToSolve,
    /// The reduced matrix is not positive definite: a mechanism (insufficient supports), a
    /// negative-stiffness state, or an inverted element.
    NotPositiveDefinite,
    Other(String),
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveError::NothingToSolve => write!(f, "no free degrees of freedom"),
            SolveError::NotPositiveDefinite => write!(f, "the stiffness matrix is not positive definite (rigid-body motion left unsupported, or an unstable state)"),
            SolveError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl Reduced {
    /// Reduced system with the default ordering (minimum degree).
    pub fn new(pat: &Arc<Pattern>, bc: &Dirichlet) -> Result<Self, SolveError> {
        Self::with_ordering(pat, bc, None, Ordering::Amd)
    }

    /// `coords` are the node coordinates (required for nested dissection).
    pub fn structure_only(pat: &Arc<Pattern>, bc: &Dirichlet) -> Result<Self, SolveError> {
        let (free_of, free_dofs, col_ptr, row_idx, src) = Self::gather(pat, bc)?;
        let _ = free_of;
        Ok(Self { pat: pat.clone(), free_dofs, col_ptr, row_idx, src, symbolic: None })
    }

    /// Free-dof numbering and the gathered reduced pattern.
    #[allow(clippy::type_complexity)]
    fn gather(pat: &Arc<Pattern>, bc: &Dirichlet) -> Result<(Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>), SolveError> {
        let d = pat.d;
        let n = pat.n_nodes * d;
        let mut free_of = vec![FIXED; n];
        let mut free_dofs = Vec::new();
        #[allow(clippy::needless_range_loop)] // dof indexes two arrays and is stored
        for dof in 0..n {
            if !bc.fixed[dof] {
                free_of[dof] = free_dofs.len() as u32;
                free_dofs.push(dof as u32);
            }
        }
        if free_dofs.is_empty() {
            return Err(SolveError::NothingToSolve);
        }
        let mut col_ptr = Vec::with_capacity(free_dofs.len() + 1);
        let mut row_idx = Vec::new();
        let mut src = Vec::new();
        col_ptr.push(0u32);
        let dd = d * d;
        for &cdof in &free_dofs {
            let (j, q) = (cdof as usize / d, cdof as usize % d);
            for k in pat.col_ptr[j]..pat.col_ptr[j + 1] {
                let i = pat.row_idx[k] as usize;
                for p in 0..d {
                    if i == j && p < q {
                        continue;
                    }
                    let r = free_of[i * d + p];
                    if r != FIXED {
                        row_idx.push(r);
                        src.push((k * dd + p * d + q) as u32);
                    }
                }
            }
            col_ptr.push(row_idx.len() as u32);
        }
        Ok((free_of, free_dofs, col_ptr, row_idx, src))
    }

    /// `coords` are the node coordinates (required for nested dissection).
    pub fn with_ordering(pat: &Arc<Pattern>, bc: &Dirichlet, coords: Option<&[[f64; 3]]>, ordering: Ordering) -> Result<Self, SolveError> {
        let d = pat.d;
        let (free_of, free_dofs, col_ptr, row_idx, src) = Self::gather(pat, bc)?;
        let m = free_dofs.len();
        let sym = SymbolicSparseColMatRef::new_checked(m, m, &col_ptr, None, &row_idx);
        let nd_perm = |c: &[[f64; 3]]| -> (Vec<u32>, Vec<u32>) {
            // Node order -> dof order restricted to the free dofs. `fwd[new] = old`.
            let node_order = ordering::nested_dissection(c, pat);
            let fwd: Vec<u32> = node_order.iter().flat_map(|&nd| (0..d).map(move |q| nd * d + q)).filter_map(|dof| (free_of[dof] != FIXED).then_some(free_of[dof])).collect();
            let mut inv = vec![0u32; m];
            for (new, &old) in fwd.iter().enumerate() {
                inv[old as usize] = new as u32;
            }
            (fwd, inv)
        };
        let analyse = |perm: &Option<(Vec<u32>, Vec<u32>)>| -> Result<SymbolicCholesky<u32>, SolveError> {
            let ord = match perm {
                Some((fwd, inv)) => SymmetricOrdering::Custom(PermRef::new_checked(fwd, inv, m)),
                None => SymmetricOrdering::Amd,
            };
            factorize_symbolic_cholesky(sym, Side::Lower, ord, CholeskySymbolicParams::default()).map_err(|e| SolveError::Other(format!("symbolic factorization failed: {e:?}")))
        };
        let symbolic = match (ordering, coords) {
            (Ordering::NestedDissection, Some(c)) => analyse(&Some(nd_perm(c)))?,
            (Ordering::NestedDissection, None) => return Err(SolveError::Other("nested dissection needs node coordinates".into())),
            (Ordering::Auto, Some(c)) if d == 3 => {
                let (amd, nd) = (analyse(&None)?, analyse(&Some(nd_perm(c)))?);
                if nd.len_val() < amd.len_val() {
                    nd
                } else {
                    amd
                }
            }
            _ => analyse(&None)?,
        };
        Ok(Self { pat: pat.clone(), free_dofs, col_ptr, row_idx, src, symbolic: Some(symbolic) })
    }

    /// Entries of the Cholesky factor (a proxy for memory and flops).
    pub fn factor_nnz(&self) -> usize {
        self.symbolic.as_ref().map_or(0, |s| s.len_val())
    }

    pub fn n_free(&self) -> usize {
        self.free_dofs.len()
    }

    /// Global dof of every reduced dof.
    pub fn free_dofs(&self) -> &[u32] {
        &self.free_dofs
    }

    /// The reduced matrix (lower triangle, compressed column) with values from `k`.
    pub fn csc(&self, k: &BlockMatrix) -> (&[u32], &[u32], Vec<f64>) {
        (&self.col_ptr, &self.row_idx, self.src.iter().map(|&s| k.vals[s as usize]).collect())
    }

    /// Numeric factorization of the reduced matrix.
    pub fn factor(&self, k: &BlockMatrix) -> Result<Factor<'_>, SolveError> {
        let symbolic = self.symbolic.as_ref().ok_or_else(|| SolveError::Other("structure-only system has no factorization".into()))?;
        let vals: Vec<f64> = self.src.iter().map(|&s| k.vals[s as usize]).collect();
        let m = self.n_free();
        let sym = SymbolicSparseColMatRef::new_checked(m, m, &self.col_ptr, None, &self.row_idx);
        let mat = SparseColMatRef::new(sym, &vals);
        let par = parallelism_for(m);
        let mut numeric = vec![0.0f64; symbolic.len_val()];
        let mut buf = MemBuffer::new(symbolic.factorize_numeric_llt_scratch::<f64>(par, Default::default()));
        symbolic.factorize_numeric_llt::<f64>(&mut numeric, mat, Side::Lower, Default::default(), par, MemStack::new(&mut buf), Default::default()).map_err(|_| SolveError::NotPositiveDefinite)?;
        Ok(Factor { red: self, numeric })
    }
}

/// Symmetric indefinite factorization `K = L D L^T` (no pivoting: the fill-reducing order is kept and
/// only an exactly zero pivot fails). For tangents past a limit point, which Cholesky rejects.
pub struct LdltFactor<'a> {
    red: &'a Reduced,
    numeric: Vec<f64>,
}

impl LdltFactor<'_> {
    pub fn solve_reduced(&self, rhs: &mut [f64]) {
        let m = rhs.len();
        let par = parallelism_for(m);
        let symbolic = self.red.symbolic.as_ref().expect("a factorization");
        let mut buf = MemBuffer::new(symbolic.solve_in_place_scratch::<f64>(1, par));
        LdltRef::<u32, f64>::new(symbolic, &self.numeric).solve_in_place_with_conj(Conj::No, MatMut::from_column_major_slice_mut(rhs, m, 1), par, MemStack::new(&mut buf));
    }
}

/// A factorization of either kind (see [`Reduced::factor_any`]).
pub enum AnyFactor<'a> {
    Llt(Factor<'a>),
    Ldlt(LdltFactor<'a>),
}

impl AnyFactor<'_> {
    pub fn solve_reduced(&self, rhs: &mut [f64]) {
        match self {
            AnyFactor::Llt(f) => f.solve_reduced(rhs),
            AnyFactor::Ldlt(f) => f.solve_reduced(rhs),
        }
    }

    /// `true` for the indefinite (LDL^T) factorization, i.e. the matrix was not positive definite.
    pub fn is_indefinite(&self) -> bool {
        matches!(self, AnyFactor::Ldlt(_))
    }
}

pub struct Factor<'a> {
    red: &'a Reduced,
    numeric: Vec<f64>,
}

impl Factor<'_> {
    /// Solve for the free dofs: `K_ff u_f = rhs_f` (both over the reduced numbering).
    pub fn solve_reduced(&self, rhs: &mut [f64]) {
        let m = rhs.len();
        let par = parallelism_for(m);
        let symbolic = self.red.symbolic.as_ref().expect("a Factor exists only for a factorized system");
        let mut buf = MemBuffer::new(symbolic.solve_in_place_scratch::<f64>(1, par));
        LltRef::<u32, f64>::new(symbolic, &self.numeric).solve_in_place_with_conj(Conj::No, MatMut::from_column_major_slice_mut(rhs, m, 1), par, MemStack::new(&mut buf));
    }

    /// Full displacement vector for load vector `f` and prescribed values `bc`.
    pub fn solve(&self, k: &BlockMatrix, f: &[f64], bc: &Dirichlet) -> Vec<f64> {
        let (mut rf, mut u) = self.red.reduced_rhs(k, f, bc);
        self.solve_reduced(&mut rf);
        self.red.scatter(&rf, &mut u);
        u
    }
}

impl Reduced {
    /// LDL^T factorization of the reduced matrix (for symmetric indefinite systems). The factorization has no
    /// pivoting, so an exactly zero pivot (a symmetrised frictional tangent can produce one) is retried with a
    /// tiny diagonal shift (`1e-10`, `1e-8`, `1e-6` of the largest diagonal): the Newton step it gives is
    /// inexact by that much, which the residual-based convergence test absorbs.
    pub fn factor_ldlt(&self, k: &BlockMatrix) -> Result<LdltFactor<'_>, SolveError> {
        let symbolic = self.symbolic.as_ref().ok_or_else(|| SolveError::Other("structure-only system has no factorization".into()))?;
        let mut vals: Vec<f64> = self.src.iter().map(|&s| k.vals[s as usize]).collect();
        let m = self.n_free();
        let sym = SymbolicSparseColMatRef::new_checked(m, m, &self.col_ptr, None, &self.row_idx);
        let par = parallelism_for(m);
        let mut numeric = vec![0.0f64; symbolic.len_val()];
        let mut buf = MemBuffer::new(symbolic.factorize_numeric_ldlt_scratch::<f64>(par, Default::default()));
        // Positions of the diagonal entries (the lower triangle stores each column's diagonal).
        let diag: Vec<usize> = (0..m).filter_map(|j| (self.col_ptr[j] as usize..self.col_ptr[j + 1] as usize).find(|&p| self.row_idx[p] as usize == j)).collect();
        if vals.iter().any(|v| !v.is_finite()) {
            return Err(SolveError::Other("the tangent has non-finite entries".into()));
        }
        let scale = diag.iter().map(|&p| vals[p].abs()).fold(0.0f64, f64::max).max(1e-300);
        let base = vals.clone();
        let mut last = String::new();
        for shift in [0.0, 1e-10, 1e-8, 1e-6] {
            if shift > 0.0 {
                vals.copy_from_slice(&base);
                for &p in &diag {
                    vals[p] += shift * scale;
                }
            }
            let mat = SparseColMatRef::new(sym, &vals);
            let outcome = symbolic.factorize_numeric_ldlt::<f64>(&mut numeric, mat, Side::Lower, LdltRegularization::default(), par, MemStack::new(&mut buf), Default::default()).map(|_| ()).map_err(|e| e.to_string());
            match outcome {
                Ok(()) if numeric.iter().all(|v| v.is_finite()) => return Ok(LdltFactor { red: self, numeric }),
                Ok(()) => last = "produced non-finite values".to_string(),
                Err(e) => last = e,
            }
        }
        Err(SolveError::Other(format!("indefinite factorization failed: {last}")))
    }

    /// Cholesky if the matrix is positive definite, otherwise LDL^T.
    pub fn factor_any(&self, k: &BlockMatrix) -> Result<AnyFactor<'_>, SolveError> {
        match self.factor(k) {
            Ok(f) => Ok(AnyFactor::Llt(f)),
            Err(SolveError::NotPositiveDefinite) => self.factor_ldlt(k).map(AnyFactor::Ldlt),
            Err(e) => Err(e),
        }
    }

    /// Right-hand side over the free dofs (corrected for prescribed non-zero displacements by one
    /// symmetric product) and the displacement vector holding the prescribed values.
    pub fn reduced_rhs(&self, k: &BlockMatrix, f: &[f64], bc: &Dirichlet) -> (Vec<f64>, Vec<f64>) {
        let n = f.len();
        let mut u = vec![0.0; n];
        let mut r = f.to_vec();
        if bc.value.iter().zip(&bc.fixed).any(|(v, fx)| *fx && *v != 0.0) {
            let up: Vec<f64> = (0..n).map(|i| if bc.fixed[i] { bc.value[i] } else { 0.0 }).collect();
            let mut ku = vec![0.0; n];
            k.matvec_add(&self.pat, &up, &mut ku);
            for i in 0..n {
                r[i] -= ku[i];
            }
            u = up;
        }
        (self.free_dofs.iter().map(|&dof| r[dof as usize]).collect(), u)
    }

    /// Write the free-dof solution into the full displacement vector.
    pub fn scatter(&self, free_solution: &[f64], u: &mut [f64]) {
        for (&dof, v) in self.free_dofs.iter().zip(free_solution) {
            u[dof as usize] = *v;
        }
    }
}

/// Reaction forces `K u - f` at every dof (zero at free dofs up to the solve residual).
pub fn residual(k: &BlockMatrix, pat: &Pattern, u: &[f64], f: &[f64]) -> Vec<f64> {
    let mut r = vec![0.0; u.len()];
    k.matvec_add(pat, u, &mut r);
    for i in 0..r.len() {
        r[i] -= f[i];
    }
    r
}

/// Sparse Cholesky of a symmetric positive definite matrix given as its lower triangle in
/// compressed-column form (AMD ordering), factored once and solved many times. Used for reduced
/// multi-point-constraint systems and the coarsest multigrid level; repeated factorizations with a
/// fixed pattern use [`Reduced`].
pub struct CscCholesky {
    m: usize,
    symbolic: SymbolicCholesky<u32>,
    numeric: Vec<f64>,
}

impl CscCholesky {
    pub fn new(m: usize, col_ptr: &[u32], row_idx: &[u32], vals: &[f64]) -> Result<Self, SolveError> {
        if m == 0 {
            return Err(SolveError::NothingToSolve);
        }
        let sym = SymbolicSparseColMatRef::new_checked(m, m, col_ptr, None, row_idx);
        let symbolic = factorize_symbolic_cholesky(sym, Side::Lower, SymmetricOrdering::Amd, CholeskySymbolicParams::default()).map_err(|e| SolveError::Other(format!("symbolic factorization failed: {e:?}")))?;
        let par = get_global_parallelism();
        let mut numeric = vec![0.0f64; symbolic.len_val()];
        let mut buf = MemBuffer::new(symbolic.factorize_numeric_llt_scratch::<f64>(par, Default::default()));
        symbolic.factorize_numeric_llt::<f64>(&mut numeric, SparseColMatRef::new(sym, vals), Side::Lower, Default::default(), par, MemStack::new(&mut buf), Default::default()).map_err(|_| SolveError::NotPositiveDefinite)?;
        Ok(Self { m, symbolic, numeric })
    }

    pub fn solve_in_place(&self, rhs: &mut [f64]) {
        let par = get_global_parallelism();
        let mut buf = MemBuffer::new(self.symbolic.solve_in_place_scratch::<f64>(1, par));
        LltRef::<u32, f64>::new(&self.symbolic, &self.numeric).solve_in_place_with_conj(Conj::No, MatMut::from_column_major_slice_mut(rhs, self.m, 1), par, MemStack::new(&mut buf));
    }
}

/// One-shot solve of `A x = rhs` (see [`CscCholesky`]).
pub fn solve_csc_lower(m: usize, col_ptr: &[u32], row_idx: &[u32], vals: &[f64], rhs: &mut [f64]) -> Result<(), SolveError> {
    CscCholesky::new(m, col_ptr, row_idx, vals)?.solve_in_place(rhs);
    Ok(())
}
