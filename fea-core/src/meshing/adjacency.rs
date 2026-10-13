//! Element, face and node adjacency of any element mix, and the conformity check every mesh producer (hybrid mesher,
//! adaptation) must pass: each interior face is shared by exactly two elements that agree on all its nodes; no node sits on the
//! interior of another element's boundary face (a hanging node or crack); no two distinct nodes coincide (a failed weld).
//!
//! Elements are numbered globally, blocks in order, elements in order within a block (the order of `ZzEstimate::eta`).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::mesh::Mesh;
use crate::topology::{corner_faces, local_faces};
use std::collections::HashMap;

const NONE: u32 = u32::MAX;

/// One face (an edge in 2D) of the mesh by its corner nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceRec {
    /// Sorted corner nodes (a 2D edge has two; the rest is `usize::MAX`).
    pub key: [usize; 4],
    /// Up to two owning `(global element, local face index)`; a third is a non-manifold error and is counted, not stored.
    pub owners: [(u32, u8); 2],
    pub n_owners: u32,
}

impl FaceRec {
    pub fn is_boundary(&self) -> bool {
        self.n_owners == 1
    }
}

/// Adjacency built once per mesh (cache it with [`crate::meshing::Cached`]).
#[derive(Debug, Clone)]
pub struct Adjacency {
    block_start: Vec<usize>,
    n_elems: usize,
    node_start: Vec<u32>,
    node_elems: Vec<u32>,
    pub faces: Vec<FaceRec>,
    nbr_start: Vec<u32>,
    nbrs: Vec<u32>,
}

impl Adjacency {
    pub fn new(mesh: &Mesh) -> Self {
        let mut block_start = Vec::with_capacity(mesh.blocks.len() + 1);
        let mut total = 0;
        for b in &mesh.blocks {
            block_start.push(total);
            total += b.n_elems();
        }
        block_start.push(total);

        // node -> elements (counting sort, so the order is by global element id)
        let mut count = vec![0u32; mesh.nodes.len() + 1];
        for b in &mesh.blocks {
            for &n in &b.conn {
                count[n + 1] += 1;
            }
        }
        for i in 0..mesh.nodes.len() {
            count[i + 1] += count[i];
        }
        let node_start = count.clone();
        let mut fill = count;
        let mut node_elems = vec![0u32; *node_start.last().unwrap_or(&0) as usize];
        for (bi, b) in mesh.blocks.iter().enumerate() {
            for e in 0..b.n_elems() {
                for &n in b.elem(e) {
                    node_elems[fill[n] as usize] = (block_start[bi] + e) as u32;
                    fill[n] += 1;
                }
            }
        }

        // faces by sorted corner key
        let mut recs: Vec<([usize; 4], u32, u8)> = Vec::new();
        for (bi, b) in mesh.blocks.iter().enumerate() {
            let lf = corner_faces(b.kind);
            for e in 0..b.n_elems() {
                let conn = b.elem(e);
                for (k, f) in lf.iter().enumerate() {
                    let mut key = [usize::MAX; 4];
                    for (s, &l) in f.iter().enumerate() {
                        key[s] = conn[l];
                    }
                    key.sort_unstable();
                    recs.push((key, (block_start[bi] + e) as u32, k as u8));
                }
            }
        }
        recs.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut faces: Vec<FaceRec> = Vec::new();
        for (key, el, lf) in recs {
            match faces.last_mut() {
                Some(last) if last.key == key => {
                    if last.n_owners < 2 {
                        last.owners[last.n_owners as usize] = (el, lf);
                    }
                    last.n_owners += 1;
                }
                _ => faces.push(FaceRec { key, owners: [(el, lf), (NONE, 0)], n_owners: 1 }),
            }
        }

        // element -> neighbours through faces with exactly two owners
        let mut ncount = vec![0u32; total + 1];
        for f in &faces {
            if f.n_owners == 2 {
                ncount[f.owners[0].0 as usize + 1] += 1;
                ncount[f.owners[1].0 as usize + 1] += 1;
            }
        }
        for i in 0..total {
            ncount[i + 1] += ncount[i];
        }
        let nbr_start = ncount.clone();
        let mut nfill = ncount;
        let mut nbrs = vec![0u32; *nbr_start.last().unwrap_or(&0) as usize];
        for f in &faces {
            if f.n_owners == 2 {
                let (a, b) = (f.owners[0].0, f.owners[1].0);
                nbrs[nfill[a as usize] as usize] = b;
                nfill[a as usize] += 1;
                nbrs[nfill[b as usize] as usize] = a;
                nfill[b as usize] += 1;
            }
        }
        Self { block_start, n_elems: total, node_start, node_elems, faces, nbr_start, nbrs }
    }

    pub fn n_elems(&self) -> usize {
        self.n_elems
    }

    /// Global id of `(block, element)`.
    pub fn global(&self, block: usize, elem: usize) -> usize {
        self.block_start[block] + elem
    }

    /// `(block, element)` of a global id.
    pub fn local(&self, global: usize) -> (usize, usize) {
        let b = self.block_start.partition_point(|&s| s <= global) - 1;
        (b, global - self.block_start[b])
    }

    /// Elements that contain `node`, ascending.
    pub fn node_elements(&self, node: usize) -> &[u32] {
        &self.node_elems[self.node_start[node] as usize..self.node_start[node + 1] as usize]
    }

    /// Elements sharing a face with `elem`.
    pub fn neighbours(&self, elem: usize) -> &[u32] {
        &self.nbrs[self.nbr_start[elem] as usize..self.nbr_start[elem + 1] as usize]
    }

    /// Boundary faces (owned by one element).
    pub fn boundary_faces(&self) -> impl Iterator<Item = &FaceRec> {
        self.faces.iter().filter(|f| f.is_boundary())
    }

    /// Number of face-connected components of the element graph.
    pub fn components(&self) -> usize {
        self.component_ids().1
    }

    /// Component id of every element and the number of components.
    pub fn component_ids(&self) -> (Vec<u32>, usize) {
        let mut id = vec![NONE; self.n_elems];
        let (mut n, mut stack) = (0usize, Vec::new());
        for s in 0..self.n_elems {
            if id[s] != NONE {
                continue;
            }
            id[s] = n as u32;
            stack.push(s as u32);
            while let Some(e) = stack.pop() {
                for &m in self.neighbours(e as usize) {
                    if id[m as usize] == NONE {
                        id[m as usize] = n as u32;
                        stack.push(m);
                    }
                }
            }
            n += 1;
        }
        (id, n)
    }
}

/// What [`check_conformity`] found. A mesh is conforming when every field below is empty/zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConformityReport {
    /// Faces owned by more than two elements.
    pub non_manifold_faces: usize,
    /// Shared faces whose two owners disagree on a node of the face (midside or centre nodes differ, or order of the
    /// interpolation differs: Tri6 against Quad4, say).
    pub mismatched_faces: usize,
    /// Nodes lying on the interior of a boundary face of an element they do not belong to (hanging nodes or cracks), or that
    /// are a corner of one element and a midside node of another (an edge split on one side only). Ascending.
    pub hanging_nodes: Vec<usize>,
    pub n_hanging: usize,
    /// Pairs of distinct nodes at the same place (a weld that did not happen).
    pub coincident_pairs: usize,
    /// Nodes no element uses (they still need a constraint in the solver).
    pub unused_nodes: usize,
    pub components: usize,
}

impl ConformityReport {
    pub fn is_conforming(&self) -> bool {
        self.non_manifold_faces == 0 && self.mismatched_faces == 0 && self.n_hanging == 0 && self.coincident_pairs == 0
    }

    pub fn summary(&self) -> String {
        format!(
            "{} non-manifold faces, {} mismatched faces, {} hanging nodes, {} coincident node pairs, {} unused nodes, {} components",
            self.non_manifold_faces, self.mismatched_faces, self.n_hanging, self.coincident_pairs, self.unused_nodes, self.components
        )
    }

    /// `Ok` for a conforming mesh, else the first problem as a typed error.
    pub fn require_conforming(&self) -> Result<(), super::MeshError> {
        if self.is_conforming() {
            Ok(())
        } else {
            Err(super::MeshError::NonConforming(self.summary()))
        }
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist_point_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    let t = if l2 > 0.0 { (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
    dot(sub(p, q), sub(p, q)).sqrt()
}

/// Distance from `p` to triangle `abc` (closest-point method of Ericson, Real-Time Collision Detection).
fn dist_point_triangle(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let q = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else {
            let vc = d1 * d4 - d3 * d2;
            if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                let v = d1 / (d1 - d3);
                [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]]
            } else {
                let cp = sub(p, c);
                let (d5, d6) = (dot(ab, cp), dot(ac, cp));
                if d6 >= 0.0 && d5 <= d6 {
                    c
                } else {
                    let vb = d5 * d2 - d1 * d6;
                    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                        let w = d2 / (d2 - d6);
                        [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]]
                    } else {
                        let va = d3 * d6 - d5 * d4;
                        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
                            [b[0] + w * (c[0] - b[0]), b[1] + w * (c[1] - b[1]), b[2] + w * (c[2] - b[2])]
                        } else {
                            let denom = 1.0 / (va + vb + vc);
                            let (v, w) = (vb * denom, vc * denom);
                            [a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w]
                        }
                    }
                }
            }
        }
    };
    dot(sub(p, q), sub(p, q)).sqrt()
}

/// Uniform grid over node positions for fixed-radius queries.
struct NodeGrid {
    cell: f64,
    lo: [f64; 3],
    bins: HashMap<[i64; 3], Vec<u32>>,
}

impl NodeGrid {
    fn new(nodes: &[[f64; 3]], cell: f64) -> Self {
        let mut lo = [f64::INFINITY; 3];
        for x in nodes {
            for c in 0..3 {
                lo[c] = lo[c].min(x[c]);
            }
        }
        let mut g = Self { cell: cell.max(1e-300), lo, bins: HashMap::new() };
        for (i, x) in nodes.iter().enumerate() {
            g.bins.entry(g.idx(*x)).or_default().push(i as u32);
        }
        g
    }
    fn idx(&self, x: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|c| ((x[c] - self.lo[c]) / self.cell).floor() as i64)
    }
    fn query(&self, lo: [f64; 3], hi: [f64; 3], mut f: impl FnMut(u32)) {
        let (a, b) = (self.idx(lo), self.idx(hi));
        for i in a[0]..=b[0] {
            for j in a[1]..=b[1] {
                for k in a[2]..=b[2] {
                    if let Some(v) = self.bins.get(&[i, j, k]) {
                        v.iter().for_each(|&n| f(n));
                    }
                }
            }
        }
    }
}

/// Verify the conformity of `mesh` (see [`ConformityReport`]).
pub fn check_conformity(mesh: &Mesh, adj: &Adjacency) -> ConformityReport {
    let mut rep = ConformityReport { components: adj.components(), ..Default::default() };
    let d = mesh.dim();
    let local: Vec<Vec<Vec<usize>>> = mesh.blocks.iter().map(|b| local_faces(b.kind)).collect();
    let face_nodes = |g: u32, lf: u8| -> Vec<usize> {
        let (b, e) = adj.local(g as usize);
        let conn = mesh.blocks[b].elem(e);
        let mut v: Vec<usize> = local[b][lf as usize].iter().map(|&l| conn[l]).collect();
        v.sort_unstable();
        v
    };
    for f in &adj.faces {
        if f.n_owners > 2 {
            rep.non_manifold_faces += 1;
        } else if f.n_owners == 2 && face_nodes(f.owners[0].0, f.owners[0].1) != face_nodes(f.owners[1].0, f.owners[1].1) {
            rep.mismatched_faces += 1;
        }
    }
    let mut used = vec![false; mesh.nodes.len()];
    for b in &mesh.blocks {
        for &n in &b.conn {
            used[n] = true;
        }
    }
    rep.unused_nodes = used.iter().filter(|u| !**u).count();
    if mesh.nodes.is_empty() {
        return rep;
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for x in &mesh.nodes {
        for c in 0..3 {
            lo[c] = lo[c].min(x[c]);
            hi[c] = hi[c].max(x[c]);
        }
    }
    let diag = (0..3).map(|c| (hi[c] - lo[c]).powi(2)).sum::<f64>().sqrt().max(1e-300);
    // typical edge length -> grid cell
    let mut h = 0.0;
    let mut cnt = 0usize;
    for f in adj.faces.iter().take(2000) {
        if f.key[1] != usize::MAX {
            h += (0..3).map(|c| (mesh.nodes[f.key[0]][c] - mesh.nodes[f.key[1]][c]).powi(2)).sum::<f64>().sqrt();
            cnt += 1;
        }
    }
    let cell = if cnt > 0 && h > 0.0 { h / cnt as f64 } else { diag };
    let grid = NodeGrid::new(&mesh.nodes, cell);
    let tol_coincide = 1e-9 * diag;
    // coincident nodes
    for (i, x) in mesh.nodes.iter().enumerate() {
        let (a, b) = ([x[0] - tol_coincide, x[1] - tol_coincide, x[2] - tol_coincide], [x[0] + tol_coincide, x[1] + tol_coincide, x[2] + tol_coincide]);
        grid.query(a, b, |j| {
            if (j as usize) > i && (0..3).map(|c| (mesh.nodes[j as usize][c] - x[c]).powi(2)).sum::<f64>().sqrt() <= tol_coincide {
                rep.coincident_pairs += 1;
            }
        });
    }
    // hanging nodes: a node on the interior of a boundary face it does not belong to
    let mut hanging: Vec<usize> = Vec::new();
    // ... or one that is a vertex for one element and a midside node for another (a split edge on one side only)
    let (mut corner_role, mut mid_role) = (vec![false; mesh.nodes.len()], vec![false; mesh.nodes.len()]);
    for b in &mesh.blocks {
        let nc = b.kind.n_corners();
        for e in 0..b.n_elems() {
            for (a, &n) in b.elem(e).iter().enumerate() {
                if a < nc {
                    corner_role[n] = true;
                } else {
                    mid_role[n] = true;
                }
            }
        }
    }
    hanging.extend((0..mesh.nodes.len()).filter(|&n| corner_role[n] && mid_role[n]));
    for f in adj.boundary_faces() {
        let (g, lf) = f.owners[0];
        let (b, e) = adj.local(g as usize);
        let conn = mesh.blocks[b].elem(e);
        let corners: Vec<usize> = corner_faces(mesh.blocks[b].kind)[lf as usize].iter().map(|&l| conn[l]).collect();
        let p: Vec<[f64; 3]> = corners.iter().map(|&n| mesh.nodes[n]).collect();
        let size = (0..p.len()).map(|i| dot(sub(p[(i + 1) % p.len()], p[i]), sub(p[(i + 1) % p.len()], p[i])).sqrt()).fold(0.0, f64::max);
        let tol = 1e-7 * size.max(tol_coincide);
        let (mut blo, mut bhi) = (p[0], p[0]);
        for q in &p {
            for c in 0..3 {
                blo[c] = blo[c].min(q[c]);
                bhi[c] = bhi[c].max(q[c]);
            }
        }
        for c in 0..3 {
            blo[c] -= tol;
            bhi[c] += tol;
        }
        let own: Vec<usize> = local[b][lf as usize].iter().map(|&l| conn[l]).collect();
        grid.query(blo, bhi, |n| {
            let n = n as usize;
            if own.contains(&n) || !used[n] {
                return;
            }
            let x = mesh.nodes[n];
            let dist = match p.len() {
                2 => dist_point_segment(x, p[0], p[1]),
                3 => dist_point_triangle(x, p[0], p[1], p[2]),
                _ => dist_point_triangle(x, p[0], p[1], p[2]).min(dist_point_triangle(x, p[0], p[2], p[3])),
            };
            // a node of a neighbouring element lying exactly at a face corner is a coincident pair, reported above
            let at_corner = corners.iter().any(|&c| (0..3).map(|k| (mesh.nodes[c][k] - x[k]).powi(2)).sum::<f64>().sqrt() <= tol_coincide);
            if dist <= tol && !at_corner {
                // midside / centre nodes of the very face are in `own`; anything else here is hanging
                hanging.push(n);
            }
        });
    }
    hanging.sort_unstable();
    hanging.dedup();
    rep.n_hanging = hanging.len();
    rep.hanging_nodes = hanging;
    let _ = d;
    rep
}

impl Mesh {
    /// Adjacency of this mesh.
    pub fn adjacency(&self) -> Adjacency {
        Adjacency::new(self)
    }

    /// Conformity check (builds the adjacency).
    pub fn conformity(&self) -> ConformityReport {
        check_conformity(self, &Adjacency::new(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn unit_grid(kind: ElementKind, n: usize) -> Mesh {
        grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0, 0.3), [n, n, 1], &|p| p).unwrap()
    }

    #[test]
    fn a_structured_grid_is_conforming_and_connected() {
        for kind in [ElementKind::Quad4, ElementKind::Quad8, ElementKind::Quad9, ElementKind::Tri3, ElementKind::Tri6] {
            let m = unit_grid(kind, 4);
            let r = m.conformity();
            assert!(r.is_conforming(), "{kind:?}: {}", r.summary());
            assert_eq!(r.components, 1);
            assert_eq!(r.unused_nodes, 0);
        }
    }

    #[test]
    fn adjacency_counts_match_the_grid() {
        let m = unit_grid(ElementKind::Quad4, 3);
        let a = m.adjacency();
        assert_eq!(a.n_elems(), 9);
        // 3x3 quads: 2*3*2 = 12 interior edges, 12 boundary edges
        assert_eq!(a.faces.iter().filter(|f| f.n_owners == 2).count(), 12);
        assert_eq!(a.boundary_faces().count(), 12);
        assert_eq!(a.neighbours(4).len(), 4); // centre cell
        assert_eq!(a.neighbours(0).len(), 2); // corner cell
        assert_eq!(a.local(a.global(0, 7)), (0, 7));
        // the centre node (index 5 + 4 * ... ) belongs to four cells
        let centre = m.nodes.iter().position(|x| (x[0] - 1.0 / 3.0).abs() < 1e-12 && (x[1] - 1.0 / 3.0).abs() < 1e-12).unwrap();
        assert_eq!(a.node_elements(centre).len(), 4);
    }

    #[test]
    fn a_crack_and_a_hanging_node_are_detected() {
        // two quads sharing an edge by position but not by node: a crack (coincident pairs)
        let mut m = Mesh::new(Physics::PlaneStress { thickness: 1.0 });
        let n: Vec<usize> = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0]]
            .iter()
            .map(|p| m.add_node([p[0], p[1], 0.0]))
            .collect();
        m.add_block(ElementKind::Quad4, n.clone(), Elastic::new(1.0, 0.3), "a").unwrap();
        // second element uses the duplicated nodes 4..8
        let mut m2 = m.clone();
        m2.blocks.clear();
        m2.add_block(ElementKind::Quad4, vec![n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7]][..4].to_vec(), Elastic::new(1.0, 0.3), "a").unwrap();
        m2.add_block(ElementKind::Quad4, vec![n[4], n[5], n[6], n[7]], Elastic::new(1.0, 0.3), "b").unwrap();
        let r = m2.conformity();
        assert_eq!(r.coincident_pairs, 2);
        assert_eq!(r.components, 2);
        assert!(!r.is_conforming());

        // a quad next to two triangles: the edge midpoint node hangs on the quad's edge
        let mut h = Mesh::new(Physics::PlaneStress { thickness: 1.0 });
        let p: Vec<usize> = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [2.0, 0.0], [2.0, 1.0], [1.0, 0.5]].iter().map(|p| h.add_node([p[0], p[1], 0.0])).collect();
        h.add_block(ElementKind::Quad4, vec![p[0], p[1], p[2], p[3]], Elastic::new(1.0, 0.3), "q").unwrap();
        h.add_block(ElementKind::Tri3, vec![p[1], p[4], p[6], p[4], p[5], p[6], p[5], p[2], p[6]], Elastic::new(1.0, 0.3), "t").unwrap();
        let r = h.conformity();
        assert_eq!(r.hanging_nodes, vec![p[6]], "{}", r.summary());
        assert!(r.mismatched_faces == 0);
        assert!(r.require_conforming().is_err());
    }

    #[test]
    fn mismatched_interpolation_orders_on_a_shared_edge_are_reported() {
        // a Tri6 and a Quad4 sharing the edge (1,0)-(1,1): the Tri6 has a midside node there, the quad does not
        let mut m = Mesh::new(Physics::PlaneStress { thickness: 1.0 });
        let id = |m: &mut Mesh, x: f64, y: f64| m.add_node([x, y, 0.0]);
        let (a, b, c) = (id(&mut m, 1.0, 0.0), id(&mut m, 2.0, 0.0), id(&mut m, 1.0, 1.0));
        let (ab, bc, ca) = (id(&mut m, 1.5, 0.0), id(&mut m, 1.5, 0.5), id(&mut m, 1.0, 0.5));
        let (q0, q3) = (id(&mut m, 0.0, 0.0), id(&mut m, 0.0, 1.0));
        m.add_block(ElementKind::Tri6, vec![a, b, c, ab, bc, ca], Elastic::new(1.0, 0.3), "t").unwrap();
        m.add_block(ElementKind::Quad4, vec![q0, a, c, q3], Elastic::new(1.0, 0.3), "q").unwrap();
        let r = m.conformity();
        assert_eq!(r.mismatched_faces, 1, "{}", r.summary());
    }

    #[test]
    fn components_split_a_disconnected_mesh() {
        let mut m = unit_grid(ElementKind::Quad4, 2);
        let other = unit_grid(ElementKind::Quad4, 2);
        let off = m.nodes.len();
        m.nodes.extend(other.nodes.iter().map(|x| [x[0] + 5.0, x[1], x[2]]));
        let conn: Vec<usize> = other.blocks[0].conn.iter().map(|n| n + off).collect();
        m.add_block(ElementKind::Quad4, conn, Elastic::new(1.0, 0.3), "b").unwrap();
        let a = m.adjacency();
        let (ids, n) = a.component_ids();
        assert_eq!(n, 2);
        assert_eq!(ids.iter().filter(|&&i| i == 0).count(), 4);
        assert!(m.conformity().is_conforming());
    }

    #[test]
    fn a_hex_grid_and_a_tet_grid_are_conforming_in_3d() {
        for kind in [ElementKind::Hex8, ElementKind::Hex20, ElementKind::Tet4, ElementKind::Tet10] {
            let m = grid(Physics::Solid, kind, Elastic::new(1.0, 0.3), [2, 2, 2], &|p| p).unwrap();
            let r = m.conformity();
            assert!(r.is_conforming(), "{kind:?}: {}", r.summary());
            assert_eq!(r.components, 1);
        }
    }

    #[test]
    fn a_3d_hanging_node_on_a_quad_face_is_found() {
        // a unit cube with two half-width hexes on top: their shared mid-edge nodes lie on the cube's top-face edges
        let mut m = Mesh::new(Physics::Solid);
        let mut ids: std::collections::HashMap<[i64; 3], usize> = Default::default();
        let mut node = |m: &mut Mesh, x: f64, y: f64, z: f64| *ids.entry([(x * 4.0) as i64, (y * 4.0) as i64, (z * 4.0) as i64]).or_insert_with(|| m.add_node([x, y, z]));
        let mut hex = |m: &mut Mesh, x0: f64, x1: f64, z0: f64, z1: f64, name: &str| {
            let c = [(x0, 0.0, z0), (x1, 0.0, z0), (x1, 1.0, z0), (x0, 1.0, z0), (x0, 0.0, z1), (x1, 0.0, z1), (x1, 1.0, z1), (x0, 1.0, z1)];
            let conn: Vec<usize> = c.iter().map(|&(x, y, z)| node(m, x, y, z)).collect();
            m.add_block(ElementKind::Hex8, conn, Elastic::new(1.0, 0.3), name).unwrap();
        };
        hex(&mut m, 0.0, 1.0, 0.0, 1.0, "cube");
        hex(&mut m, 0.0, 0.5, 1.0, 2.0, "left");
        hex(&mut m, 0.5, 1.0, 1.0, 2.0, "right");
        let r = m.conformity();
        assert_eq!(r.n_hanging, 2, "{}", r.summary());
        assert_eq!(r.hanging_nodes.len(), 2);
        assert!(!r.is_conforming());
    }

    #[test]
    fn the_fingerprint_changes_with_geometry_and_connectivity_only() {
        let m = unit_grid(ElementKind::Quad4, 2);
        let f0 = crate::meshing::fingerprint(&m);
        let mut moved = m.clone();
        moved.nodes[4][0] += 1e-12;
        assert_ne!(f0, crate::meshing::fingerprint(&moved));
        let mut renamed = m.clone();
        renamed.blocks[0].name = "other".into();
        assert_eq!(f0, crate::meshing::fingerprint(&renamed));
        let cache = crate::meshing::Cached::new(&m, 7usize);
        assert_eq!(cache.get(&m), Some(&7));
        assert_eq!(cache.get(&moved), None);
    }
}
