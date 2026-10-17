//! Shared mesh infrastructure of the hybrid meshing and goal-oriented adaptivity work (epic #25): adjacency, conformity
//! verification, element quality metrics, region labels and mesh fingerprints. Nothing here changes how a model solves; the
//! mesh generators (`delaunay`, `tetmesh`, `generate`, `sweep`) and the adaptivity code (`adaptivity`) both build on it and
//! do not depend on each other.

pub mod adjacency;
pub mod cost;
pub mod compatibility;
pub mod curved;
pub mod hybrid;
pub mod metric;
pub mod metric_refine;
pub mod multimaterial;
pub mod partition;
pub mod predictor;
pub mod quality;
pub mod smooth;
pub mod tet_collapse;
pub mod tet_flip;
pub mod tet_smooth;
pub mod tet_topology;
pub mod tet_delaunay;
pub mod weld;

use crate::mesh::Mesh;

/// Why a meshing or mesh-checking operation refused or failed. Typed so callers can react (fall back, report) without
/// matching strings.
#[derive(Debug, Clone, PartialEq)]
pub enum MeshError {
    /// The mesh has elements that are inverted or degenerate.
    InvalidElements { count: usize, first: (usize, usize) },
    /// Elements do not share their faces conformingly (cracks, hanging nodes, non-manifold faces, order mismatch).
    NonConforming(String),
    /// The requested configuration is not supported (reported, never silently replaced).
    Unsupported(String),
    /// A resource limit (elements, nodes) would be exceeded.
    LimitExceeded(String),
    /// An underlying mesher or kernel call failed.
    Failed(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidElements { count, first } => write!(f, "{count} inverted or degenerate elements (first: block {}, element {})", first.0, first.1),
            Self::NonConforming(s) => write!(f, "non-conforming mesh: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::LimitExceeded(s) => write!(f, "limit exceeded: {s}"),
            Self::Failed(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for MeshError {}

impl From<String> for MeshError {
    fn from(s: String) -> Self {
        Self::Failed(s)
    }
}

/// How an element was produced, for diagnostics and for the adaptation policy (which refinement operator may touch it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionKind {
    Structured,
    Unstructured,
    Transition,
}

/// A label per element in global element order (blocks in order, elements in order within a block).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ElementLabels(pub Vec<RegionKind>);

impl ElementLabels {
    pub fn uniform(mesh: &Mesh, kind: RegionKind) -> Self {
        Self(vec![kind; mesh.n_elems()])
    }

    pub fn count(&self, kind: RegionKind) -> usize {
        self.0.iter().filter(|k| **k == kind).count()
    }

    /// Fraction of the elements with the label (`0` for an empty mesh).
    pub fn fraction(&self, kind: RegionKind) -> f64 {
        if self.0.is_empty() {
            0.0
        } else {
            self.count(kind) as f64 / self.0.len() as f64
        }
    }
}

/// FNV-1a over everything that determines a matrix, a factorisation or an element-wise cache: dimension, node coordinates
/// (bit patterns), block kinds and connectivity. A cache keyed by this value is invalid exactly when the mesh changed;
/// materials and loads are not part of it (they invalidate numeric caches, not topological ones).
pub fn fingerprint(mesh: &Mesh) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
    };
    eat(mesh.dim() as u64);
    eat(mesh.nodes.len() as u64);
    for x in &mesh.nodes {
        for c in x {
            eat(c.to_bits());
        }
    }
    for blk in &mesh.blocks {
        eat(blk.kind.vtk_type() as u64);
        eat(blk.conn.len() as u64);
        for &n in &blk.conn {
            eat(n as u64);
        }
    }
    h
}

/// A value computed from a mesh, valid only for that mesh.
#[derive(Debug, Clone)]
pub struct Cached<T> {
    key: u64,
    value: T,
}

impl<T> Cached<T> {
    pub fn new(mesh: &Mesh, value: T) -> Self {
        Self { key: fingerprint(mesh), value }
    }

    /// The value if `mesh` is the mesh it was computed for.
    pub fn get(&self, mesh: &Mesh) -> Option<&T> {
        (self.key == fingerprint(mesh)).then_some(&self.value)
    }
}
