//! Geometric nested-dissection ordering of a node graph.
//!
//! A mesh graph has small separators, so eliminating the two halves of a recursive coordinate
//! bisection first and the separator last keeps the Cholesky factor far sparser than a general
//! minimum-degree ordering does on 3D meshes (fill ~ n^(4/3) against ~ n^(3/2)+, flops ~ n^2
//! against ~ n^(7/3)). Works in 2D and 3D; the graph is the lower-triangle block pattern.

use crate::assembly::Pattern;

const LEAF: usize = 24;

/// Node elimination order (`order[k]` = node eliminated `k`-th).
pub fn nested_dissection(coords: &[[f64; 3]], pat: &Pattern) -> Vec<usize> {
    let n = pat.n_nodes;
    // Full symmetric adjacency (CSR) without the diagonal.
    let mut deg = vec![0usize; n + 1];
    for j in 0..n {
        for k in pat.col_ptr[j]..pat.col_ptr[j + 1] {
            let i = pat.row_idx[k] as usize;
            if i != j {
                deg[i] += 1;
                deg[j] += 1;
            }
        }
    }
    let mut ptr = vec![0usize; n + 1];
    for i in 0..n {
        ptr[i + 1] = ptr[i] + deg[i];
    }
    let mut fill = ptr.clone();
    let mut adj = vec![0u32; ptr[n]];
    for j in 0..n {
        for k in pat.col_ptr[j]..pat.col_ptr[j + 1] {
            let i = pat.row_idx[k] as usize;
            if i != j {
                adj[fill[i]] = j as u32;
                fill[i] += 1;
                adj[fill[j]] = i as u32;
                fill[j] += 1;
            }
        }
    }
    let mut order = Vec::with_capacity(n);
    let mut side = vec![0u8; n];
    let all: Vec<u32> = (0..n as u32).collect();
    dissect(coords, &ptr, &adj, all, &mut side, &mut order);
    order
}

fn dissect(coords: &[[f64; 3]], ptr: &[usize], adj: &[u32], mut nodes: Vec<u32>, side: &mut [u8], order: &mut Vec<usize>) {
    if nodes.len() <= LEAF {
        order.extend(nodes.iter().map(|&v| v as usize));
        return;
    }
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for &v in &nodes {
        for c in 0..3 {
            lo[c] = lo[c].min(coords[v as usize][c]);
            hi[c] = hi[c].max(coords[v as usize][c]);
        }
    }
    let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
    nodes.sort_unstable_by(|&a, &b| coords[a as usize][axis].total_cmp(&coords[b as usize][axis]).then(a.cmp(&b)));
    let half = nodes.len() / 2;
    for (k, &v) in nodes.iter().enumerate() {
        side[v as usize] = if k < half { 1 } else { 2 };
    }
    // Vertex separator: the nodes of one half that touch the other half; take the smaller.
    let touches = |v: u32, other: u8| adj[ptr[v as usize]..ptr[v as usize + 1]].iter().any(|&w| side[w as usize] == other);
    let sep_a: Vec<u32> = nodes[..half].iter().copied().filter(|&v| touches(v, 2)).collect();
    let sep_b: Vec<u32> = nodes[half..].iter().copied().filter(|&v| touches(v, 1)).collect();
    let (sep, left, right): (Vec<u32>, Vec<u32>, Vec<u32>) = if sep_a.len() <= sep_b.len() {
        let in_sep: std::collections::HashSet<u32> = sep_a.iter().copied().collect();
        (sep_a, nodes[..half].iter().copied().filter(|v| !in_sep.contains(v)).collect(), nodes[half..].to_vec())
    } else {
        let in_sep: std::collections::HashSet<u32> = sep_b.iter().copied().collect();
        (sep_b, nodes[..half].to_vec(), nodes[half..].iter().copied().filter(|v| !in_sep.contains(v)).collect())
    };
    for &v in &nodes {
        side[v as usize] = 0;
    }
    if sep.is_empty() || left.is_empty() || right.is_empty() {
        // Disconnected pieces or no progress: order as is.
        order.extend(nodes.iter().map(|&v| v as usize));
        return;
    }
    dissect(coords, ptr, adj, left, side, order);
    dissect(coords, ptr, adj, right, side, order);
    order.extend(sep.iter().map(|&v| v as usize));
}
