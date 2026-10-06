//! Structured mesh generators: a parametric unit square / cube cut into cells, mapped to
//! physical space by any smooth map (so an annulus, a quarter plate with a hole or a curved block
//! is one closure away). Quadratic elements put their midside nodes on the mapped parametric
//! midpoints, so curved boundaries are represented to second order.
//!
//! Node sets `u0 u1 v0 v1 w0 w1` (parametric faces) and surfaces of the same names (boundary
//! edges, or faces of hexahedral meshes) are generated for applying constraints and loads.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::mesh::{Elastic, Mesh, Physics};
use std::collections::HashMap;

type Map<'a> = &'a dyn Fn([f64; 3]) -> [f64; 3];

const SIDE2: [&str; 4] = ["u0", "u1", "v0", "v1"];
const SIDE3: [&str; 6] = ["u0", "u1", "v0", "v1", "w0", "w1"];

/// Build a `div[0] x div[1] (x div[2])` grid of `kind` elements.
pub fn grid(physics: Physics, kind: ElementKind, mat: Elastic, div: [usize; 3], map: Map) -> Result<Mesh, String> {
    let d = kind.dim();
    if physics.dim() != d {
        return Err(format!("{kind:?} does not fit this analysis type"));
    }
    if div[..d].contains(&0) {
        return Err("every division count must be at least 1".into());
    }
    let mut mesh = Mesh::new(physics);
    if kind.is_simplex() {
        simplex_grid(&mut mesh, kind, mat, div, map)?;
    } else {
        tensor_grid(&mut mesh, kind, mat, div, map)?;
    }
    Ok(mesh)
}

fn tensor_grid(mesh: &mut Mesh, kind: ElementKind, mat: Elastic, div: [usize; 3], map: Map) -> Result<(), String> {
    let d = kind.dim();
    let order = if matches!(kind, ElementKind::Quad4 | ElementKind::Hex8) { 1 } else { 2 };
    let nl: [usize; 3] = std::array::from_fn(|a| if a < d { div[a] * order + 1 } else { 1 });
    let mut id = vec![usize::MAX; nl[0] * nl[1] * nl[2]];
    let mut node = |mesh: &mut Mesh, idx: [usize; 3]| -> usize {
        let slot = idx[0] + nl[0] * (idx[1] + nl[1] * idx[2]);
        if id[slot] == usize::MAX {
            let p: [f64; 3] = std::array::from_fn(|a| if a < d { idx[a] as f64 / (div[a] * order) as f64 } else { 0.0 });
            id[slot] = mesh.add_node(map(p));
        }
        id[slot]
    };
    let cells = [div[0], div[1], if d == 3 { div[2] } else { 1 }];
    let mut conn = Vec::new();
    for ck in 0..cells[2] {
        for cj in 0..cells[1] {
            for ci in 0..cells[0] {
                let c = [ci, cj, ck];
                for x in kind.node_coords() {
                    let idx: [usize; 3] = std::array::from_fn(|a| if a < d { order * c[a] + ((x[a] + 1.0) * 0.5 * order as f64).round() as usize } else { 0 });
                    conn.push(node(mesh, idx));
                }
            }
        }
    }
    mesh.add_block(kind, conn, mat, "grid")?;
    // Boundary surfaces.
    if d == 2 {
        for (s, name) in SIDE2.iter().enumerate() {
            let (axis, hi) = (s / 2, s % 2 == 1);
            let along = 1 - axis; // the axis the edge runs along
            let fixed = if hi { nl[axis] - 1 } else { 0 };
            // Walk with the body on the left: +u on v0, +v on u1, -u on v1, -v on u0.
            let forward = matches!((axis, hi), (1, false) | (0, true));
            let mut edges = Vec::new();
            for cell in 0..div[along] {
                let mut at = |t: usize| -> usize {
                    let mut idx = [0usize; 3];
                    idx[axis] = fixed;
                    idx[along] = order * cell + t;
                    node(mesh, idx)
                };
                let (a, b) = if forward { (0, order) } else { (order, 0) };
                let mut e = vec![at(a), at(b)];
                if order == 2 {
                    e.push(at(1));
                }
                edges.push(e);
            }
            if !forward {
                edges.reverse();
            }
            mesh.surfaces.insert((*name).to_string(), edges);
        }
    } else {
        let face_kind = match kind {
            ElementKind::Hex8 => ElementKind::Quad4,
            ElementKind::Hex20 => ElementKind::Quad8,
            _ => ElementKind::Quad9,
        };
        for (s, name) in SIDE3.iter().enumerate() {
            let (axis, hi) = (s / 2, s % 2 == 1);
            let fixed = if hi { nl[axis] - 1 } else { 0 };
            // (p, q) with e_p x e_q = +e_axis; swap on the minimum side so the normal points out.
            let (mut p, mut q) = ((axis + 1) % 3, (axis + 2) % 3);
            if !hi {
                std::mem::swap(&mut p, &mut q);
            }
            let mut faces = Vec::new();
            for cq in 0..div[q] {
                for cp in 0..div[p] {
                    let mut f = Vec::new();
                    for x in face_kind.node_coords() {
                        let mut idx = [0usize; 3];
                        idx[axis] = fixed;
                        idx[p] = order * cp + ((x[0] + 1.0) * 0.5 * order as f64).round() as usize;
                        idx[q] = order * cq + ((x[1] + 1.0) * 0.5 * order as f64).round() as usize;
                        f.push(node(mesh, idx));
                    }
                    faces.push(f);
                }
            }
            mesh.surfaces.insert((*name).to_string(), faces);
        }
    }
    // Node sets from the lattice (every node knows its lattice index through `id`).
    let mut lattice = vec![[0usize; 3]; mesh.nodes.len()];
    for (slot, &n) in id.iter().enumerate() {
        if n != usize::MAX {
            lattice[n] = [slot % nl[0], (slot / nl[0]) % nl[1], slot / (nl[0] * nl[1])];
        }
    }
    let names: &[&str] = if d == 2 { &SIDE2 } else { &SIDE3 };
    for (s, name) in names.iter().enumerate() {
        let (axis, hi) = (s / 2, s % 2 == 1);
        let target = if hi { nl[axis] - 1 } else { 0 };
        let ids: Vec<usize> = (0..lattice.len()).filter(|&n| lattice[n][axis] == target).collect();
        mesh.node_sets.insert((*name).to_string(), ids);
    }
    Ok(())
}

fn simplex_grid(mesh: &mut Mesh, kind: ElementKind, mat: Elastic, div: [usize; 3], map: Map) -> Result<(), String> {
    let d = kind.dim();
    let quadratic = matches!(kind, ElementKind::Tri6 | ElementKind::Tet10);
    let nl: [usize; 3] = std::array::from_fn(|a| if a < d { div[a] + 1 } else { 1 });
    let mut id = vec![usize::MAX; nl[0] * nl[1] * nl[2]];
    let mut param: Vec<[f64; 3]> = Vec::new();
    let mut lattice: Vec<[usize; 3]> = Vec::new();
    let mut vertex = |mesh: &mut Mesh, idx: [usize; 3]| -> usize {
        let slot = idx[0] + nl[0] * (idx[1] + nl[1] * idx[2]);
        if id[slot] == usize::MAX {
            let p: [f64; 3] = std::array::from_fn(|a| if a < d { idx[a] as f64 / div[a] as f64 } else { 0.0 });
            id[slot] = mesh.add_node(map(p));
            param.push(p);
            lattice.push(idx);
        }
        id[slot]
    };
    let mut simplices: Vec<Vec<usize>> = Vec::new();
    let cells = [div[0], div[1], if d == 3 { div[2] } else { 1 }];
    for ck in 0..cells[2] {
        for cj in 0..cells[1] {
            for ci in 0..cells[0] {
                let corner = |mesh: &mut Mesh, vertex: &mut dyn FnMut(&mut Mesh, [usize; 3]) -> usize, bits: [usize; 3]| vertex(mesh, [ci + bits[0], cj + bits[1], ck + bits[2]]);
                if d == 2 {
                    let c00 = corner(mesh, &mut vertex, [0, 0, 0]);
                    let c10 = corner(mesh, &mut vertex, [1, 0, 0]);
                    let c11 = corner(mesh, &mut vertex, [1, 1, 0]);
                    let c01 = corner(mesh, &mut vertex, [0, 1, 0]);
                    simplices.push(vec![c00, c10, c11]);
                    simplices.push(vec![c00, c11, c01]);
                } else {
                    // Kuhn subdivision: six tetrahedra along the main diagonal (conforming).
                    const PERMS: [([usize; 3], bool); 6] = [([0, 1, 2], true), ([0, 2, 1], false), ([1, 0, 2], false), ([1, 2, 0], true), ([2, 0, 1], true), ([2, 1, 0], false)];
                    for (perm, even) in PERMS {
                        let mut bits = [0usize; 3];
                        let v0 = corner(mesh, &mut vertex, bits);
                        bits[perm[0]] = 1;
                        let v1 = corner(mesh, &mut vertex, bits);
                        bits[perm[1]] = 1;
                        let v2 = corner(mesh, &mut vertex, bits);
                        bits[perm[2]] = 1;
                        let v3 = corner(mesh, &mut vertex, bits);
                        simplices.push(if even { vec![v0, v1, v2, v3] } else { vec![v0, v2, v1, v3] });
                    }
                }
            }
        }
    }
    let mut conn = Vec::new();
    if quadratic {
        let mut mids: HashMap<(usize, usize), usize> = HashMap::new();
        let edges: &[[usize; 2]] = if d == 2 { &[[0, 1], [1, 2], [2, 0]] } else { &[[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]] };
        for s in &simplices {
            conn.extend_from_slice(s);
            for &[p, q] in edges {
                let (a, b) = (s[p].min(s[q]), s[p].max(s[q]));
                let m = *mids.entry((a, b)).or_insert_with(|| {
                    let pm: [f64; 3] = std::array::from_fn(|k| 0.5 * (param[a][k] + param[b][k]));
                    mesh.add_node(map(pm))
                });
                conn.push(m);
            }
        }
        mesh.add_block(kind, conn, mat, "grid")?;
        let names: &[&str] = if d == 2 { &SIDE2 } else { &SIDE3 };
        // Side sets by mapped parameter of each node: recover the parameter from the vertex /
        // midpoint construction.
        let mut node_param = param.clone();
        let mut keys: Vec<((usize, usize), usize)> = mids.into_iter().collect();
        keys.sort_by_key(|(_, n)| *n);
        for ((a, b), n) in keys {
            debug_assert_eq!(n, node_param.len());
            node_param.push(std::array::from_fn(|k| 0.5 * (param[a][k] + param[b][k])));
        }
        for (s, name) in names.iter().enumerate() {
            let (axis, hi) = (s / 2, s % 2 == 1);
            let target = if hi { 1.0 } else { 0.0 };
            let ids: Vec<usize> = (0..node_param.len()).filter(|&n| (node_param[n][axis] - target).abs() < 1e-12).collect();
            mesh.node_sets.insert((*name).to_string(), ids);
        }
        if d == 2 {
            edges_2d(mesh, div, true, &|n| node_param[n]);
        }
    } else {
        for s in &simplices {
            conn.extend_from_slice(s);
        }
        mesh.add_block(kind, conn, mat, "grid")?;
        let names: &[&str] = if d == 2 { &SIDE2 } else { &SIDE3 };
        for (s, name) in names.iter().enumerate() {
            let (axis, hi) = (s / 2, s % 2 == 1);
            let target = if hi { nl[axis] - 1 } else { 0 };
            let ids: Vec<usize> = (0..lattice.len()).filter(|&n| lattice[n][axis] == target).collect();
            mesh.node_sets.insert((*name).to_string(), ids);
        }
        if d == 2 {
            let pr = param.clone();
            edges_2d(mesh, div, false, &|n| pr[n]);
        }
    }
    Ok(())
}

/// Boundary edges of a triangle mesh of the unit square: every element edge whose two corner
/// nodes lie on one side, oriented with the body on the left.
fn edges_2d(mesh: &mut Mesh, _div: [usize; 3], quadratic: bool, param: &dyn Fn(usize) -> [f64; 3]) {
    let blk = &mesh.blocks[0];
    let nn = blk.kind.n_nodes();
    let mut edges: HashMap<&str, Vec<Vec<usize>>> = HashMap::new();
    for conn in blk.conn.chunks_exact(nn) {
        for (k, &(p, q)) in [(0usize, 1usize), (1, 2), (2, 0)].iter().enumerate() {
            let (a, b) = (conn[p], conn[q]);
            let (pa, pb) = (param(a), param(b));
            for (s, name) in SIDE2.iter().enumerate() {
                let (axis, hi) = (s / 2, s % 2 == 1);
                let target = if hi { 1.0 } else { 0.0 };
                if (pa[axis] - target).abs() < 1e-12 && (pb[axis] - target).abs() < 1e-12 {
                    // Triangle corners are counter-clockwise, so the element edge a -> b already
                    // has the body on its left.
                    let mut e = vec![a, b];
                    if quadratic {
                        e.push(conn[3 + k]);
                    }
                    edges.entry(name).or_default().push(e);
                }
            }
        }
    }
    for (name, list) in edges {
        mesh.surfaces.insert(name.to_string(), list);
    }
}
