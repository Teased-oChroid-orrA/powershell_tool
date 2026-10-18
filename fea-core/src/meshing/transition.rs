//! Verified prism-to-tetrahedron and hexahedron-to-tetrahedron transitions: convert selected `Wedge6` / `Hex8` elements into
//! `Tet4` elements so that a swept or structured region meets a tetrahedral one with a conforming interface. When the
//! hexahedra next to the tetrahedra are converted too, no pyramid element is needed (`docs/issue-46-phase-79.md`): a pyramid over
//! a quad is two tetrahedra, so the whole question is which diagonal the quad takes, and the rule below answers it from the
//! global node numbers. A hexahedron that is *kept* next to tetrahedra needs the `Pyramid5` element and a tetrahedral mesher
//! that preserves the pyramids' sides (`solid_kernel::hybrid`, `docs/issue-46-phase-102.md`; erratum in Phase 94).
//!
//! Each selected wedge is cut into three tetrahedra. The three vertical quad faces of a wedge are each split by a diagonal
//! chosen from the global node numbers alone: the diagonal that contains the smallest node number of the four. Both elements
//! sharing a quad face derive the same diagonal from the same four numbers, so adjacent converted wedges are cut identically,
//! and a converted wedge meets an unconverted one only through a triangular face, which tetrahedra and wedges share exactly.
//! The rule never makes the three diagonals of one wedge cycle (the two quads that contain the wedge's smallest node both take
//! their diagonal from it), so the three-tetrahedra split always exists: that node is the apex of a pyramid over the opposite
//! quad (two tetrahedra, split by the opposite quad's own diagonal) plus one tetrahedron over the remaining triangle.
//!
//! A hexahedron is cut into tetrahedra the same way: its six quad faces take the diagonal containing the smallest of their four
//! node numbers, and one of the (at most) 42 resulting diagonal patterns is tiled by tetrahedra without extra nodes (a table
//! found by exact search over the reference cube and proved by `every_numbering_of_a_hexahedron_splits_consistently`). A
//! tetrahedral region built elsewhere meets a converted hexahedron conformingly when its boundary triangles use
//! [`interface_diagonal`] on the shared quads: this is the contract for a boundary-constrained tetrahedral mesher.
//!
//! The selection must not separate two elements that share a *quad* face (one converted, one not): that interface would pair a
//! quad with two triangles. The conversion refuses such a selection with an error naming the face, in the epic's words "a
//! verified fallback rather than silently inserting invalid interfaces". Boundary surfaces keep their names; their quad faces
//! on converted wedges become the two triangles of the same diagonal, orientation preserved. Node sets, materials, density and
//! thermal data of the block carry over to the tetrahedra; only the first-order family is supported (`Wedge6` to `Tet4`).

#![allow(clippy::needless_range_loop)] // index-parallel loops over blocks and per-element flags read clearer than zips

use super::MeshError;
use crate::element::ElementKind;
use crate::mesh::{Block, Mesh};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionReport {
    pub wedges_converted: usize,
    pub wedges_kept: usize,
    pub hexes_converted: usize,
    pub hexes_kept: usize,
    pub tets_created: usize,
    pub surface_quads_split: usize,
    pub min_scaled_jacobian: f64,
}

fn vol(p: &[[f64; 3]; 4]) -> f64 {
    let (a, b, c) = (
        [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]],
        [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]],
        [p[3][0] - p[0][0], p[3][1] - p[0][1], p[3][2] - p[0][2]],
    );
    (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0]))
        / 6.0
}

/// For the quad `[n0, n1, n2, n3]` (cyclic): `true` when its diagonal is `n0-n2`, `false` for `n1-n3`; the diagonal contains
/// the smallest of the four node numbers.
fn diagonal_is_02(q: [usize; 4]) -> bool {
    let m = *q.iter().min().expect("four nodes");
    q[0] == m || q[2] == m
}

/// The diagonal (a pair of node numbers) the interface rule gives the quad `q` (nodes in cyclic order): the one containing the
/// smallest node number. A tetrahedral region meeting a converted `Hex8`/`Wedge6` on a quad must triangulate it with this diagonal.
pub fn interface_diagonal(q: [usize; 4]) -> [usize; 2] {
    if diagonal_is_02(q) {
        [q[0], q[2]]
    } else {
        [q[1], q[3]]
    }
}

/// Hex8 faces as cyclic local node lists; a face's diagonal is `(f0, f2)` (bit set) or `(f1, f3)`.
const HEX_FACES: [[usize; 4]; 6] = [
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];
const CUBE: [[i32; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];

/// Six times the signed volume of the reference-cube tetrahedron `t`.
fn cube_vol6(t: [usize; 4]) -> i32 {
    let d = |i: usize| {
        [
            CUBE[t[i]][0] - CUBE[t[0]][0],
            CUBE[t[i]][1] - CUBE[t[0]][1],
            CUBE[t[i]][2] - CUBE[t[0]][2],
        ]
    };
    let (a, b, c) = (d(1), d(2), d(3));
    a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0])
}

/// Whether the positively oriented tetrahedra tile the reference cube exactly: volumes add to the cube's, every interior face
/// is matched by an oppositely oriented one, and every unmatched face lies in a face of the cube.
fn tiles_cube(tets: &[[usize; 4]]) -> bool {
    if tets.iter().map(|&t| cube_vol6(t)).sum::<i32>() != 6 {
        return false;
    }
    let mut faces: BTreeMap<[usize; 3], Vec<[usize; 3]>> = BTreeMap::new();
    for t in tets {
        for f in [
            [t[0], t[2], t[1]],
            [t[0], t[1], t[3]],
            [t[1], t[2], t[3]],
            [t[2], t[0], t[3]],
        ] {
            let mut k = f;
            k.sort_unstable();
            faces.entry(k).or_default().push(f);
        }
    }
    faces.iter().all(|(k, v)| match v.len() {
        1 => (0..3).any(|a| {
            let c = CUBE[k[0]][a];
            k.iter().all(|&n| CUBE[n][a] == c)
        }),
        2 => {
            // Opposite orientation: the second is an odd permutation of the first.
            let (p, q) = (v[0], v[1]);
            let i = (0..3).find(|&i| p[i] == q[0]).expect("same nodes");
            q[1] == p[(i + 2) % 3] && q[2] == p[(i + 1) % 3]
        }
        _ => false,
    })
}

/// For each of the 64 diagonal patterns of a `Hex8`, a tiling by tetrahedra (local node indices, positively oriented) without
/// extra nodes, or an empty list if the pattern has none. Found once by exact search over the 70 candidate tetrahedra of the
/// reference cube; validated by [`tiles_cube`]. The numbering rule produces only 42 patterns, all tilable.
fn hex_tables() -> &'static Vec<Vec<[usize; 4]>> {
    static TABLES: std::sync::OnceLock<Vec<Vec<[usize; 4]>>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut all: Vec<[usize; 4]> = Vec::new();
        for a in 0..8 {
            for b in a + 1..8 {
                for c in b + 1..8 {
                    for d in c + 1..8 {
                        let mut t = [a, b, c, d];
                        match cube_vol6(t) {
                            0 => continue,
                            v if v < 0 => t.swap(2, 3),
                            _ => {}
                        }
                        all.push(t);
                    }
                }
            }
        }
        // Sample points, used only to prune overlapping candidates early (the final check is exact).
        let samples: Vec<[f64; 3]> = (1..=160)
            .map(|i| {
                let f = |a: f64, o: f64| (i as f64 * a + o).fract();
                [
                    f(0.618_033_988_7, 0.1),
                    f(0.754_877_666_2, 0.2),
                    f(0.569_840_291, 0.3),
                ]
            })
            .collect();
        let covers = |t: [usize; 4]| -> u128 {
            let p = |i: usize| CUBE[t[i]].map(f64::from);
            let det = |q0: [f64; 3], q1: [f64; 3], q2: [f64; 3], q3: [f64; 3]| {
                let (a, b, c) = (
                    [q1[0] - q0[0], q1[1] - q0[1], q1[2] - q0[2]],
                    [q2[0] - q0[0], q2[1] - q0[1], q2[2] - q0[2]],
                    [q3[0] - q0[0], q3[1] - q0[1], q3[2] - q0[2]],
                );
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
            };
            let mut bits = 0u128;
            for (k, s) in samples.iter().enumerate().take(128) {
                let d = [
                    det(*s, p(1), p(2), p(3)),
                    det(p(0), *s, p(2), p(3)),
                    det(p(0), p(1), *s, p(3)),
                    det(p(0), p(1), p(2), *s),
                ];
                if d.iter().all(|&x| x > 0.0) {
                    bits |= 1 << k;
                }
            }
            bits
        };
        let cover: Vec<u128> = all.iter().map(|&t| covers(t)).collect();
        (0..64)
            .map(|pattern: usize| {
                // The cube edges that are face diagonals, and which of them this pattern uses.
                let mut chosen: BTreeSet<[usize; 2]> = BTreeSet::new();
                let mut diagonals: BTreeSet<[usize; 2]> = BTreeSet::new();
                for (k, f) in HEX_FACES.iter().enumerate() {
                    for (a, b) in [(f[0], f[2]), (f[1], f[3])] {
                        diagonals.insert([a.min(b), a.max(b)]);
                    }
                    let (a, b) = if pattern >> k & 1 == 1 {
                        (f[0], f[2])
                    } else {
                        (f[1], f[3])
                    };
                    chosen.insert([a.min(b), a.max(b)]);
                }
                let usable: Vec<usize> = (0..all.len())
                    .filter(|&i| {
                        let t = all[i];
                        (0..4).all(|x| {
                            (x + 1..4).all(|y| {
                                let e = [t[x].min(t[y]), t[x].max(t[y])];
                                !diagonals.contains(&e) || chosen.contains(&e)
                            })
                        })
                    })
                    .collect();
                fn search(
                    usable: &[usize],
                    all: &[[usize; 4]],
                    cover: &[u128],
                    from: usize,
                    used: u128,
                    vol: i32,
                    picked: &mut Vec<[usize; 4]>,
                ) -> bool {
                    if vol == 6 {
                        return tiles_cube(picked);
                    }
                    for (n, &i) in usable.iter().enumerate().skip(from) {
                        let v = cube_vol6(all[i]);
                        if vol + v > 6 || cover[i] & used != 0 {
                            continue;
                        }
                        picked.push(all[i]);
                        if search(usable, all, cover, n + 1, used | cover[i], vol + v, picked) {
                            return true;
                        }
                        picked.pop();
                    }
                    false
                }
                let mut picked = Vec::new();
                if search(&usable, &all, &cover, 0, 0, 0, &mut picked) {
                    picked
                } else {
                    Vec::new()
                }
            })
            .collect()
    })
}

/// The tetrahedra of the hexahedron `h` (global node numbers in the usual `Hex8` order), cut by the interface rule, as node
/// tuples; `None` if the diagonal pattern has no tiling (never happens for the rule's 42 patterns).
fn split_hex(h: &[usize]) -> Option<Vec<[usize; 4]>> {
    let pattern = HEX_FACES.iter().enumerate().fold(0usize, |p, (k, f)| {
        p | usize::from(diagonal_is_02([h[f[0]], h[f[1]], h[f[2]], h[f[3]]])) << k
    });
    let t = &hex_tables()[pattern];
    (!t.is_empty()).then(|| t.iter().map(|l| l.map(|i| h[i])).collect())
}

/// The three tetrahedra of the wedge `w` (bottom `w[0..3]`, top `w[3..6]`, column `i` is `w[i]`-`w[i+3]`), as node tuples.
fn split_wedge(w: &[usize]) -> Vec<[usize; 4]> {
    let (g, h) = ([w[0], w[1], w[2]], [w[3], w[4], w[5]]);
    // The smallest node number of the wedge: its column `c` and whether it is on the bottom.
    let (c, bottom) = (0..3)
        .flat_map(|i| [(i, true), (i, false)])
        .min_by_key(|&(i, b)| if b { g[i] } else { h[i] })
        .expect("six nodes");
    let (j, k) = ((c + 1) % 3, (c + 2) % 3);
    let apex = if bottom { g[c] } else { h[c] };
    // The quad opposite column `c`: g_j g_k h_k h_j, split by its own diagonal.
    let q = [g[j], g[k], h[k], h[j]];
    let (t1, t2) = if diagonal_is_02(q) {
        ([q[0], q[1], q[2]], [q[0], q[2], q[3]])
    } else {
        ([q[1], q[2], q[3]], [q[1], q[3], q[0]])
    };
    let last = if bottom {
        [apex, h[c], h[j], h[k]]
    } else {
        [apex, g[c], g[j], g[k]]
    };
    vec![
        [apex, t1[0], t1[1], t1[2]],
        [apex, t2[0], t2[1], t2[2]],
        last,
    ]
}

/// Convert the `Wedge6` elements for which `select(block, element, nodes)` is true; hexahedra are left alone. See the module
/// documentation.
pub fn tetrahedralize_wedges(
    mesh: &Mesh,
    select: &dyn Fn(usize, usize, &[usize]) -> bool,
) -> Result<(Mesh, TransitionReport), MeshError> {
    convert(mesh, select, false)
}

/// Convert the selected `Wedge6` and `Hex8` elements into tetrahedra. See the module documentation.
pub fn tetrahedralize(
    mesh: &Mesh,
    select: &dyn Fn(usize, usize, &[usize]) -> bool,
) -> Result<(Mesh, TransitionReport), MeshError> {
    convert(mesh, select, true)
}

/// The quad faces of a `Wedge6` or `Hex8` element as sorted node tuples.
fn quad_keys(kind: ElementKind, e: &[usize]) -> Vec<[usize; 4]> {
    let faces: Vec<[usize; 4]> = match kind {
        ElementKind::Wedge6 => [(0, 1), (1, 2), (2, 0)]
            .map(|(i, j)| [e[i], e[j], e[j + 3], e[i + 3]])
            .to_vec(),
        ElementKind::Hex8 => HEX_FACES
            .map(|f| [e[f[0]], e[f[1]], e[f[2]], e[f[3]]])
            .to_vec(),
        _ => Vec::new(),
    };
    faces
        .into_iter()
        .map(|mut k| {
            k.sort_unstable();
            k
        })
        .collect()
}

fn convert(
    mesh: &Mesh,
    select: &dyn Fn(usize, usize, &[usize]) -> bool,
    hexes: bool,
) -> Result<(Mesh, TransitionReport), MeshError> {
    if mesh.dim() != 3 {
        return Err(MeshError::Failed(
            "the prism/hexahedron-tetrahedron transition needs a three-dimensional mesh".into(),
        ));
    }
    if mesh.blocks.iter().any(|b| {
        b.kind == ElementKind::Wedge15
            || (hexes && matches!(b.kind, ElementKind::Hex20 | ElementKind::Hex27))
    }) {
        return Err(MeshError::Failed("second-order wedges and hexahedra cannot be converted: orders are not mixed (they would need Tet10)".into()));
    }
    let convertible =
        |k: ElementKind| k == ElementKind::Wedge6 || (hexes && k == ElementKind::Hex8);
    // Which elements convert.
    let mut convert: Vec<Vec<bool>> = Vec::new();
    for (b, blk) in mesh.blocks.iter().enumerate() {
        convert.push(
            (0..blk.n_elems())
                .map(|e| convertible(blk.kind) && select(b, e, blk.elem(e)))
                .collect(),
        );
    }
    // Interface check: no quad face may be shared by a converted and an unconverted element (wedge or hexahedron).
    let mut quad_owner: BTreeMap<[usize; 4], Vec<bool>> = BTreeMap::new();
    for (b, blk) in mesh.blocks.iter().enumerate() {
        for e in 0..blk.n_elems() {
            for key in quad_keys(blk.kind, blk.elem(e)) {
                quad_owner.entry(key).or_default().push(convert[b][e]);
            }
        }
    }
    if let Some((face, _)) = quad_owner
        .iter()
        .find(|(_, o)| o.len() == 2 && o[0] != o[1])
    {
        return Err(MeshError::NonConforming(format!("the selection separates two elements sharing the quad face {face:?}: its interface would pair a quad with two triangles; select whole layers or regions bounded by triangular faces")));
    }
    let mut out = Mesh::new(mesh.physics);
    out.nodes = mesh.nodes.clone();
    out.node_sets = mesh.node_sets.clone();
    let (mut converted, mut kept, mut created) = ([0usize; 2], [0usize; 2], 0);
    let mut belongs: BTreeSet<[usize; 4]> = BTreeSet::new();
    for (b, blk) in mesh.blocks.iter().enumerate() {
        let nn = blk.kind.n_nodes();
        let class = usize::from(blk.kind == ElementKind::Hex8);
        let mut stay = blk.clone();
        stay.conn.clear();
        let mut tets: Vec<usize> = Vec::new();
        for e in 0..blk.n_elems() {
            if convert[b][e] {
                let el = blk.elem(e);
                let parts = if blk.kind == ElementKind::Wedge6 {
                    split_wedge(el)
                } else {
                    split_hex(el).ok_or_else(|| {
                        MeshError::Failed(format!(
                            "no tetrahedral tiling for the hexahedron {el:?}"
                        ))
                    })?
                };
                for mut t in parts {
                    let p = [
                        mesh.nodes[t[0]],
                        mesh.nodes[t[1]],
                        mesh.nodes[t[2]],
                        mesh.nodes[t[3]],
                    ];
                    if vol(&p) < 0.0 {
                        t.swap(2, 3);
                    }
                    tets.extend(t);
                    created += 1;
                }
                belongs.extend(quad_keys(blk.kind, el));
                converted[class] += 1;
            } else {
                stay.conn.extend_from_slice(&blk.conn[e * nn..(e + 1) * nn]);
                kept[class] += 1;
            }
        }
        if !stay.conn.is_empty() {
            out.blocks.push(stay);
        }
        if !tets.is_empty() {
            out.blocks.push(Block {
                kind: ElementKind::Tet4,
                conn: tets,
                material: blk.material,
                name: format!("{}-tet", blk.name),
                plasticity: blk.plasticity,
                density: blk.density,
                thermal: blk.thermal.clone(),
            });
        }
    }
    // Boundary surfaces: split the quad faces of converted elements by the same diagonal.
    let mut split = 0;
    for (name, list) in &mesh.surfaces {
        let mut faces: Vec<Vec<usize>> = Vec::with_capacity(list.len());
        for f in list {
            if f.len() == 4 {
                let mut key = [f[0], f[1], f[2], f[3]];
                key.sort_unstable();
                if belongs.contains(&key) {
                    let q = [f[0], f[1], f[2], f[3]];
                    if diagonal_is_02(q) {
                        faces.push(vec![q[0], q[1], q[2]]);
                        faces.push(vec![q[0], q[2], q[3]]);
                    } else {
                        faces.push(vec![q[1], q[2], q[3]]);
                        faces.push(vec![q[1], q[3], q[0]]);
                    }
                    split += 1;
                    continue;
                }
            }
            faces.push(f.clone());
        }
        out.surfaces.insert(name.clone(), faces);
    }
    let q = out.quality();
    q.require(&super::quality::QualityThresholds {
        min_scaled_jacobian: 1e-9,
        max_edge_ratio: f64::INFINITY,
        max_size_jump: f64::INFINITY,
    })?;
    let report = TransitionReport {
        wedges_converted: converted[0],
        wedges_kept: kept[0],
        hexes_converted: converted[1],
        hexes_kept: kept[1],
        tets_created: created,
        surface_quads_split: split,
        min_scaled_jacobian: q.min_scaled_jacobian,
    };
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn layered(nz: usize) -> Mesh {
        grid(
            Physics::Solid,
            ElementKind::Wedge6,
            Elastic::new(1000.0, 0.25),
            [3, 3, nz],
            &|p| p,
        )
        .expect("swept wedge mesh")
    }

    fn centroid_z(mesh: &Mesh, nodes: &[usize]) -> f64 {
        nodes.iter().map(|&n| mesh.nodes[n][2]).sum::<f64>() / nodes.len() as f64
    }

    fn total_volume(mesh: &Mesh) -> f64 {
        let q = mesh.quality();
        let _ = q;
        let mut v = 0.0;
        for b in &mesh.blocks {
            for e in 0..b.n_elems() {
                let el = b.elem(e);
                match b.kind {
                    ElementKind::Tet4 => {
                        v += vol(&[
                            mesh.nodes[el[0]],
                            mesh.nodes[el[1]],
                            mesh.nodes[el[2]],
                            mesh.nodes[el[3]],
                        ])
                    }
                    ElementKind::Wedge6 => {
                        let w = el;
                        // three-tet split of the wedge only for measuring: use the same rule
                        for t in split_wedge(w) {
                            v += vol(&[
                                mesh.nodes[t[0]],
                                mesh.nodes[t[1]],
                                mesh.nodes[t[2]],
                                mesh.nodes[t[3]],
                            ])
                            .abs();
                        }
                    }
                    _ => {}
                }
            }
        }
        v
    }

    #[test]
    fn the_top_layers_become_tetrahedra_and_the_mesh_stays_conforming_and_exact() {
        let mesh = layered(4);
        let before = total_volume(&mesh);
        let select = |_b: usize, _e: usize, nodes: &[usize]| centroid_z(&mesh, nodes) > 0.5;
        let (out, r) = tetrahedralize_wedges(&mesh, &select).unwrap();
        assert_eq!(r.tets_created, 3 * r.wedges_converted);
        assert!(r.wedges_converted > 0 && r.wedges_kept > 0, "{r:?}");
        assert!(r.surface_quads_split > 0);
        assert!(r.min_scaled_jacobian > 0.0);
        assert!(
            (total_volume(&out) - before).abs() < 1e-12 * before.max(1.0),
            "{} vs {before}",
            total_volume(&out)
        );
        assert!((before - 1.0).abs() < 1e-12);
        let adj = super::super::adjacency::Adjacency::new(&out);
        let c = super::super::adjacency::check_conformity(&out, &adj);
        c.require_conforming()
            .unwrap_or_else(|e| panic!("{e:?} {c:?}"));
        // Both element families are present with their materials and a tagged tetrahedron block.
        assert!(
            out.blocks.iter().any(|b| b.kind == ElementKind::Wedge6)
                && out.blocks.iter().any(|b| b.kind == ElementKind::Tet4)
        );
        // The boundary surfaces keep their area: quads on the converted layers became the two triangles of the same cut.
        let area = |m: &Mesh, name: &str| -> f64 {
            m.surfaces[name]
                .iter()
                .map(|f| {
                    let p: Vec<[f64; 3]> = f.iter().map(|&n| m.nodes[n]).collect();
                    let tri = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
                        let (u, v) = (
                            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                        );
                        0.5 * ((u[1] * v[2] - u[2] * v[1]).powi(2)
                            + (u[2] * v[0] - u[0] * v[2]).powi(2)
                            + (u[0] * v[1] - u[1] * v[0]).powi(2))
                        .sqrt()
                    };
                    if p.len() == 3 {
                        tri(p[0], p[1], p[2])
                    } else {
                        tri(p[0], p[1], p[2]) + tri(p[0], p[2], p[3])
                    }
                })
                .sum()
        };
        for name in mesh.surfaces.keys() {
            assert!(
                (area(&out, name) - area(&mesh, name)).abs() < 1e-12,
                "surface {name}"
            );
        }
    }

    /// The largest error of the solved linear field at interior nodes of the unit cube when the field is prescribed on its
    /// whole boundary: zero for a conforming mesh.
    fn patch_error(out: &Mesh) -> f64 {
        let field = |x: [f64; 3]| {
            [
                1e-3 * x[0] + 2e-4 * x[1] - 3e-4 * x[2],
                -1e-4 * x[0] + 5e-4 * x[1] + 2e-4 * x[2],
                3e-4 * x[0] - 2e-4 * x[1] + 4e-4 * x[2],
            ]
        };
        let model = crate::Model::new(out.clone()).unwrap();
        let mut bc = model.dirichlet();
        let mut interior = Vec::new();
        for (n, x) in out.nodes.iter().enumerate() {
            let on_boundary = (0..3).any(|k| x[k].abs() < 1e-12 || (x[k] - 1.0).abs() < 1e-12);
            if on_boundary {
                let u = field(*x);
                for c in 0..3 {
                    bc.fix(n, c, u[c]);
                }
            } else {
                interior.push(n);
            }
        }
        assert!(!interior.is_empty());
        let sol = model.solve_static(&crate::Loads::default(), &bc).unwrap();
        let worst = interior
            .iter()
            .map(|&n| {
                let u = field(out.nodes[n]);
                (0..3)
                    .map(|c| (sol.u[n * 3 + c] - u[c]).abs())
                    .fold(0.0, f64::max)
            })
            .fold(0.0, f64::max);
        worst
    }

    #[test]
    fn a_mixed_prism_tetrahedron_mesh_passes_the_linear_patch_test() {
        // A conforming interface reproduces any linear displacement field exactly: prescribe it on the whole boundary and every
        // interior node must come out on the field. A quad paired with two triangles, or an inconsistent cut, would fail.
        let mesh = layered(4);
        let select = |_b: usize, _e: usize, nodes: &[usize]| centroid_z(&mesh, nodes) > 0.5;
        let (out, _) = tetrahedralize_wedges(&mesh, &select).unwrap();
        let worst = patch_error(&out);
        assert!(worst < 1e-10, "patch test error {worst:.3e}");
    }

    #[test]
    fn a_selection_that_separates_quad_neighbours_and_unsupported_input_are_refused() {
        let mesh = layered(2);
        // Half of every layer: wedges at x < 0.5 convert, their neighbours at x > 0.5 do not, sharing quad faces.
        let split_in_plane = |_b: usize, _e: usize, nodes: &[usize]| {
            nodes.iter().map(|&n| mesh.nodes[n][0]).sum::<f64>() / 6.0 < 0.5
        };
        let err = tetrahedralize_wedges(&mesh, &split_in_plane).unwrap_err();
        assert!(format!("{err:?}").contains("quad face"), "{err:?}");
        // Nothing selected: the mesh is returned as it is.
        let (same, r) = tetrahedralize_wedges(&mesh, &|_, _, _| false).unwrap();
        assert_eq!((r.wedges_converted, r.tets_created), (0, 0));
        assert_eq!(same.blocks[0].conn, mesh.blocks[0].conn);
        // A two-dimensional mesh is refused.
        let plane = grid(
            Physics::PlaneStress { thickness: 1.0 },
            ElementKind::Quad4,
            Elastic::new(1.0, 0.3),
            [2, 2, 1],
            &|p| p,
        )
        .unwrap();
        assert!(tetrahedralize_wedges(&plane, &|_, _, _| true).is_err());
    }

    #[test]
    fn the_split_is_a_valid_three_tetrahedron_decomposition_for_every_node_numbering() {
        // All 720 numberings of the six nodes of a unit prism: the three tetrahedra must tile it (volumes 1/2 in total, none
        // flat) whichever vertex is the smallest.
        let pts = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [1., 0., 1.],
            [0., 1., 1.],
        ];
        let mut perm: Vec<usize> = (0..6).collect();
        let mut checked = 0;
        fn next_perm(p: &mut [usize]) -> bool {
            let n = p.len();
            let Some(i) = (0..n - 1).rev().find(|&i| p[i] < p[i + 1]) else {
                return false;
            };
            let j = (i + 1..n).rev().find(|&j| p[j] > p[i]).unwrap();
            p.swap(i, j);
            p[i + 1..].reverse();
            true
        }
        loop {
            // `perm[l]` is the global number of local node `l`; the geometry is by local index.
            let tets = split_wedge(&perm);
            let local = |g: usize| perm.iter().position(|&x| x == g).unwrap();
            let total: f64 = tets
                .iter()
                .map(|t| {
                    vol(&[
                        pts[local(t[0])],
                        pts[local(t[1])],
                        pts[local(t[2])],
                        pts[local(t[3])],
                    ])
                    .abs()
                })
                .sum();
            assert!((total - 0.5).abs() < 1e-12, "numbering {perm:?}: {total}");
            assert!(
                tets.iter().all(|t| vol(&[
                    pts[local(t[0])],
                    pts[local(t[1])],
                    pts[local(t[2])],
                    pts[local(t[3])]
                ])
                .abs()
                    > 1e-12),
                "{perm:?}"
            );
            checked += 1;
            if !next_perm(&mut perm) {
                break;
            }
        }
        assert_eq!(checked, 720);
    }

    fn hex_grid(div: [usize; 3]) -> Mesh {
        grid(
            Physics::Solid,
            ElementKind::Hex8,
            Elastic::new(1000.0, 0.25),
            div,
            &|p| p,
        )
        .expect("hex mesh")
    }

    #[test]
    fn every_numbering_of_a_hexahedron_splits_consistently() {
        // All 8! numberings of the eight nodes of a unit cube: the tetrahedra tile it exactly (positive volumes adding to 1),
        // and on every face the two triangles meet along the interface rule's diagonal, whichever vertex is the smallest.
        let pts: Vec<[f64; 3]> = CUBE.iter().map(|c| c.map(f64::from)).collect();
        let mut perm: Vec<usize> = (0..8).collect();
        let mut patterns = BTreeSet::new();
        let mut checked = 0;
        loop {
            // `perm[l]` is the global number of local node `l`.
            let local = |g: usize| perm.iter().position(|&x| x == g).unwrap();
            let tets = split_hex(&perm).unwrap_or_else(|| panic!("no tiling for {perm:?}"));
            let mut total = 0.0;
            for t in &tets {
                let v = vol(&[
                    pts[local(t[0])],
                    pts[local(t[1])],
                    pts[local(t[2])],
                    pts[local(t[3])],
                ]);
                assert!(v > 1e-12, "{perm:?}: tetrahedron {t:?} has volume {v}");
                total += v;
            }
            assert!((total - 1.0).abs() < 1e-12, "{perm:?}: {total}");
            for f in HEX_FACES {
                let q = [perm[f[0]], perm[f[1]], perm[f[2]], perm[f[3]]];
                let d = interface_diagonal(q);
                // The tetrahedron edges lying in this face: exactly one of its two diagonals appears, and it is the rule's.
                let in_face = |n: usize| q.contains(&n);
                let mut used = BTreeSet::new();
                for t in &tets {
                    for x in 0..4 {
                        for y in x + 1..4 {
                            let (a, b) = (t[x], t[y]);
                            if in_face(a)
                                && in_face(b)
                                && !(0..4).any(|i| {
                                    (q[i] == a && q[(i + 1) % 4] == b)
                                        || (q[i] == b && q[(i + 1) % 4] == a)
                                })
                            {
                                used.insert([a.min(b), a.max(b)]);
                            }
                        }
                    }
                }
                assert_eq!(
                    used,
                    BTreeSet::from([[d[0].min(d[1]), d[0].max(d[1])]]),
                    "{perm:?} face {f:?}"
                );
            }
            patterns.insert(
                HEX_FACES.map(|f| diagonal_is_02([perm[f[0]], perm[f[1]], perm[f[2]], perm[f[3]]])),
            );
            checked += 1;
            let n = perm.len();
            let Some(i) = (0..n - 1).rev().find(|&i| perm[i] < perm[i + 1]) else {
                break;
            };
            let j = (i + 1..n).rev().find(|&j| perm[j] > perm[i]).unwrap();
            perm.swap(i, j);
            perm[i + 1..].reverse();
        }
        assert_eq!(checked, 40320);
        assert_eq!(patterns.len(), 42, "the rule produces 42 diagonal patterns");
    }

    #[test]
    fn a_converted_hexahedral_block_is_exact_conforming_and_passes_the_patch_test() {
        let mesh = hex_grid([3, 3, 3]);
        let (out, r) = tetrahedralize(&mesh, &|_, _, _| true).unwrap();
        assert_eq!(
            (r.hexes_converted, r.hexes_kept, r.wedges_converted),
            (27, 0, 0)
        );
        assert_eq!(r.tets_created, out.blocks[0].n_elems());
        assert!(out.blocks.iter().all(|b| b.kind == ElementKind::Tet4));
        assert!(
            r.surface_quads_split > 0 && r.min_scaled_jacobian > 0.0,
            "{r:?}"
        );
        assert!((total_volume(&out) - 1.0).abs() < 1e-12);
        let adj = super::super::adjacency::Adjacency::new(&out);
        let c = super::super::adjacency::check_conformity(&out, &adj);
        c.require_conforming()
            .unwrap_or_else(|e| panic!("{e:?} {c:?}"));
        let worst = patch_error(&out);
        assert!(worst < 1e-10, "patch test error {worst:.3e}");
    }

    #[test]
    fn a_hexahedral_block_meets_an_independently_built_tetrahedral_block_through_the_rule_diagonal()
    {
        // Left cell: a Hex8, converted. Right cell: six tetrahedra built here around the main diagonal that starts at an
        // endpoint of the rule's diagonal on the shared quad, so its boundary triangles on that quad use that diagonal. The
        // interface is then conforming; the other diagonal is not.
        let grid2 = hex_grid([2, 1, 1]);
        let blk = &grid2.blocks[0];
        let left = (0..2)
            .find(|&e| centroid_x(&grid2, blk.elem(e)) < 0.5)
            .unwrap();
        let (hl, hr) = (blk.elem(left).to_vec(), blk.elem(1 - left).to_vec());
        // The shared quad: nodes of the left cell at x = 0.5, in cyclic order from the face table.
        let shared = HEX_FACES
            .iter()
            .map(|f| [hl[f[0]], hl[f[1]], hl[f[2]], hl[f[3]]])
            .find(|q| q.iter().all(|&n| (grid2.nodes[n][0] - 0.5).abs() < 1e-12))
            .expect("shared quad");
        let rule = interface_diagonal(shared);
        let off = *shared.iter().find(|n| !rule.contains(n)).unwrap();
        // The right cell as six tetrahedra around the main diagonal from the interface vertex `a`: bits of its nodes relative to
        // `a` flip x, y, z in each order.
        let build = |a: usize| -> Mesh {
            let origin = grid2.nodes[a];
            let axis_of =
                |n: usize, k: usize| usize::from((grid2.nodes[n][k] - origin[k]).abs() > 1e-12);
            let corner: BTreeMap<[usize; 3], usize> = hr
                .iter()
                .map(|&n| ([0, 1, 2].map(|k| axis_of(n, k)), n))
                .collect();
            assert_eq!(corner.len(), 8);
            let mut tets = Vec::new();
            for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
                let mut bits = [0usize; 3];
                let mut path = vec![corner[&bits]];
                for k in order {
                    bits[k] = 1;
                    path.push(corner[&bits]);
                }
                // Odd axis orders give negatively oriented tetrahedra: swap two nodes to orient them.
                let p: Vec<[f64; 3]> = path.iter().map(|&n| grid2.nodes[n]).collect();
                if vol(&[p[0], p[1], p[2], p[3]]) < 0.0 {
                    path.swap(2, 3);
                }
                tets.extend_from_slice(&path);
            }
            let mut mesh = grid2.clone();
            mesh.surfaces.clear();
            mesh.blocks[0].conn = hl.clone();
            mesh.blocks.push(Block {
                kind: ElementKind::Tet4,
                conn: tets,
                material: mesh.blocks[0].material,
                name: "tet-side".into(),
                plasticity: None,
                density: 0.0,
                thermal: mesh.blocks[0].thermal.clone(),
            });
            mesh
        };
        // A crack (two blocks triangulating the shared quad differently) is invisible to `check_conformity`: both triangulations
        // look like boundary. So count faces instead: in a conforming mesh of the unit cube a face used once lies on its surface.
        let conformity = |mesh: &Mesh| -> Result<(), String> {
            let (out, r) = tetrahedralize(mesh, &|b, _, _| b == 0).unwrap();
            assert_eq!((r.hexes_converted, r.hexes_kept), (1, 0));
            assert!((total_volume(&out) - 1.0).abs() < 1e-12);
            let mut count: BTreeMap<[usize; 3], usize> = BTreeMap::new();
            for blk in &out.blocks {
                assert_eq!(blk.kind, ElementKind::Tet4);
                for e in 0..blk.n_elems() {
                    let t = blk.elem(e);
                    for f in [[t[0], t[1], t[2]], [t[0], t[1], t[3]], [t[0], t[2], t[3]], [t[1], t[2], t[3]]] {
                        let mut k = f;
                        k.sort_unstable();
                        *count.entry(k).or_default() += 1;
                    }
                }
            }
            let on_surface = |f: &[usize; 3]| {
                (0..3).any(|a| {
                    [0.0, 1.0].iter().any(|&c| f.iter().all(|&n| (out.nodes[n][a] - c).abs() < 1e-12))
                })
            };
            match count.iter().find(|(f, &n)| n > 2 || (n == 1 && !on_surface(f))) {
                Some((f, n)) => Err(format!("face {f:?} used {n} times away from the surface")),
                None => Ok(()),
            }
        };
        // The tetrahedra's diagonal on the shared quad is the rule's: conforming.
        conformity(&build(rule[0])).unwrap();
        conformity(&build(rule[1])).unwrap();
        // The other diagonal is not: the same test sees the mismatch (the control that gives the assertions above their power).
        assert!(conformity(&build(off)).is_err());
    }

    fn centroid_x(mesh: &Mesh, nodes: &[usize]) -> f64 {
        nodes.iter().map(|&n| mesh.nodes[n][0]).sum::<f64>() / nodes.len() as f64
    }

    #[test]
    fn hexahedron_selections_that_would_pair_a_quad_with_triangles_and_second_order_input_are_refused(
    ) {
        let mesh = hex_grid([2, 2, 1]);
        // One cell of four: its neighbours share quad faces with it.
        let one = |_b: usize, e: usize, _n: &[usize]| e == 0;
        let err = tetrahedralize(&mesh, &one).unwrap_err();
        assert!(format!("{err:?}").contains("quad face"), "{err:?}");
        // The wedge-only entry point never touches hexahedra, even when everything is selected.
        let (same, r) = tetrahedralize_wedges(&mesh, &|_, _, _| true).unwrap();
        assert_eq!((r.hexes_converted, r.tets_created), (0, 0));
        assert_eq!(same.blocks[0].conn, mesh.blocks[0].conn);
        // Second-order hexahedra are refused (orders are not mixed).
        let h20 = grid(
            Physics::Solid,
            ElementKind::Hex20,
            Elastic::new(1000.0, 0.25),
            [1, 1, 1],
            &|p| p,
        )
        .unwrap();
        assert!(tetrahedralize(&h20, &|_, _, _| true).is_err());
        // ...but only when hexahedra are being converted: the wedge entry point ignores them.
        assert!(tetrahedralize_wedges(&h20, &|_, _, _| true).is_ok());
    }
}
