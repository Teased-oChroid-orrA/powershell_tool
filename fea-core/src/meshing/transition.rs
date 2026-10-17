//! Verified prism-to-tetrahedron transition: convert selected `Wedge6` elements into `Tet4` elements so that a swept (prismatic)
//! region meets a tetrahedral one with a conforming interface.
//!
//! Each selected wedge is cut into three tetrahedra. The three vertical quad faces of a wedge are each split by a diagonal
//! chosen from the global node numbers alone: the diagonal that contains the smallest node number of the four. Both elements
//! sharing a quad face derive the same diagonal from the same four numbers, so adjacent converted wedges are cut identically,
//! and a converted wedge meets an unconverted one only through a triangular face, which tetrahedra and wedges share exactly.
//! The rule never makes the three diagonals of one wedge cycle (the two quads that contain the wedge's smallest node both take
//! their diagonal from it), so the three-tetrahedra split always exists: that node is the apex of a pyramid over the opposite
//! quad (two tetrahedra, split by the opposite quad's own diagonal) plus one tetrahedron over the remaining triangle.
//!
//! The selection must not separate two wedges that share a *quad* face (one converted, one not): that interface would pair a
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

/// Convert the wedges for which `select(block, element, nodes)` is true. See the module documentation.
pub fn tetrahedralize_wedges(
    mesh: &Mesh,
    select: &dyn Fn(usize, usize, &[usize]) -> bool,
) -> Result<(Mesh, TransitionReport), MeshError> {
    if mesh.dim() != 3 {
        return Err(MeshError::Failed(
            "the prism-tetrahedron transition needs a three-dimensional mesh".into(),
        ));
    }
    if mesh.blocks.iter().any(|b| b.kind == ElementKind::Wedge15) {
        return Err(MeshError::Failed("second-order wedges cannot be converted: orders are not mixed (Wedge15 would need Tet10)".into()));
    }
    // Which wedges convert.
    let mut convert: Vec<Vec<bool>> = Vec::new();
    for (b, blk) in mesh.blocks.iter().enumerate() {
        convert.push(
            (0..blk.n_elems())
                .map(|e| blk.kind == ElementKind::Wedge6 && select(b, e, blk.elem(e)))
                .collect(),
        );
    }
    // Interface check: no quad face may be shared by a converted and an unconverted wedge.
    let mut quad_owner: BTreeMap<[usize; 4], Vec<bool>> = BTreeMap::new();
    for (b, blk) in mesh.blocks.iter().enumerate() {
        if blk.kind != ElementKind::Wedge6 {
            continue;
        }
        for e in 0..blk.n_elems() {
            let w = blk.elem(e);
            for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                let mut key = [w[i], w[j], w[j + 3], w[i + 3]];
                key.sort_unstable();
                quad_owner.entry(key).or_default().push(convert[b][e]);
            }
        }
    }
    if let Some((face, _)) = quad_owner
        .iter()
        .find(|(_, o)| o.len() == 2 && o[0] != o[1])
    {
        return Err(MeshError::NonConforming(format!("the selection separates two wedges sharing the quad face {face:?}: its interface would pair a quad with two triangles; select whole layers bounded by triangular faces")));
    }
    let mut out = Mesh::new(mesh.physics);
    out.nodes = mesh.nodes.clone();
    out.node_sets = mesh.node_sets.clone();
    let (mut converted, mut kept, mut created) = (0, 0, 0);
    let mut converted_nodes: BTreeSet<[usize; 6]> = BTreeSet::new();
    for (b, blk) in mesh.blocks.iter().enumerate() {
        let nn = blk.kind.n_nodes();
        let mut stay = blk.clone();
        stay.conn.clear();
        let mut tets: Vec<usize> = Vec::new();
        for e in 0..blk.n_elems() {
            if convert[b][e] {
                let w = blk.elem(e);
                converted_nodes.insert([w[0], w[1], w[2], w[3], w[4], w[5]]);
                for mut t in split_wedge(w) {
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
                converted += 1;
            } else {
                stay.conn.extend_from_slice(&blk.conn[e * nn..(e + 1) * nn]);
                if blk.kind == ElementKind::Wedge6 {
                    kept += 1;
                }
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
    // Boundary surfaces: split the quad faces of converted wedges by the same diagonal.
    let belongs: BTreeSet<[usize; 4]> = converted_nodes
        .iter()
        .flat_map(|w| {
            [(0, 1), (1, 2), (2, 0)].map(|(i, j)| {
                let mut k = [w[i], w[j], w[j + 3], w[i + 3]];
                k.sort_unstable();
                k
            })
        })
        .collect();
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
        wedges_converted: converted,
        wedges_kept: kept,
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

    #[test]
    fn a_mixed_prism_tetrahedron_mesh_passes_the_linear_patch_test() {
        // A conforming interface reproduces any linear displacement field exactly: prescribe it on the whole boundary and every
        // interior node must come out on the field. A quad paired with two triangles, or an inconsistent cut, would fail.
        let mesh = layered(4);
        let select = |_b: usize, _e: usize, nodes: &[usize]| centroid_z(&mesh, nodes) > 0.5;
        let (out, _) = tetrahedralize_wedges(&mesh, &select).unwrap();
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
}
