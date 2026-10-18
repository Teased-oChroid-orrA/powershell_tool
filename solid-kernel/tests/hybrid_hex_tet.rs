//! A cantilever whose left part is hexahedra and whose right part is tetrahedra, joined by a layer of pyramids
//! (`hybrid::hex_pyramid_tet`): face-conforming, exact on a linear field, and bending close to the all-hexahedron answer.
//! `docs/issue-46-phase-102.md`.

use fea_core::generate::grid;
use fea_core::meshing::adjacency::{check_conformity, Adjacency};
use fea_core::meshing::transition::tetrahedralize;
use fea_core::meshing::weld::merge;
use fea_core::*;
use solid_kernel::hybrid::{hex_pyramid_tet, HybridOptions};
use solid_kernel::V3;

fn mat() -> Elastic {
    Elastic::new(1000.0, 0.3)
}

/// Outer surface of the block [x0, x1] x [0, 1]^2 without its x = x0 face: the x = x1 end and the four walls, gridded `n` cells
/// across and `nx` along, as outward triangles. The grid points on the walls at x = x0 are the interface ring.
fn region_shell(x0: f64, x1: f64, nx: usize, n: usize) -> (Vec<V3>, Vec<[usize; 3]>) {
    let mut v: Vec<V3> = Vec::new();
    let mut id = std::collections::BTreeMap::new();
    let mut node = |v: &mut Vec<V3>, p: V3| -> usize {
        let key = [
            (p[0] * 1e9).round() as i64,
            (p[1] * 1e9).round() as i64,
            (p[2] * 1e9).round() as i64,
        ];
        *id.entry(key).or_insert_with(|| {
            v.push(p);
            v.len() - 1
        })
    };
    let centre = [(x0 + x1) / 2., 0.5, 0.5];
    let mut facets = Vec::new();
    let tri = |v: &Vec<V3>, t: [usize; 3], facets: &mut Vec<[usize; 3]>| {
        let (a, b, c) = (v[t[0]], v[t[1]], v[t[2]]);
        let nrm = [
            (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
            (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
        ];
        let g = [
            (a[0] + b[0] + c[0]) / 3. - centre[0],
            (a[1] + b[1] + c[1]) / 3. - centre[1],
            (a[2] + b[2] + c[2]) / 3. - centre[2],
        ];
        let out = nrm[0] * g[0] + nrm[1] * g[1] + nrm[2] * g[2] > 0.;
        facets.push(if out { t } else { [t[0], t[2], t[1]] });
    };
    let h = 1.0 / n as f64;
    let dx = (x1 - x0) / nx as f64;
    for i in 0..n {
        for j in 0..n {
            let c = [
                [x1, i as f64 * h, j as f64 * h],
                [x1, (i + 1) as f64 * h, j as f64 * h],
                [x1, (i + 1) as f64 * h, (j + 1) as f64 * h],
                [x1, i as f64 * h, (j + 1) as f64 * h],
            ]
            .map(|p| node(&mut v, p));
            tri(&v, [c[0], c[1], c[2]], &mut facets);
            tri(&v, [c[0], c[2], c[3]], &mut facets);
        }
    }
    for k in 0..nx {
        let (xa, xb) = (x0 + k as f64 * dx, x0 + (k + 1) as f64 * dx);
        for i in 0..n {
            let (a0, a1) = (i as f64 * h, (i + 1) as f64 * h);
            for (axis, fixed) in [(1usize, 0.0), (1, 1.0), (2, 0.0), (2, 1.0)] {
                let mk = |x: f64, a: f64| -> V3 {
                    if axis == 1 {
                        [x, fixed, a]
                    } else {
                        [x, a, fixed]
                    }
                };
                let c = [mk(xa, a0), mk(xb, a0), mk(xb, a1), mk(xa, a1)].map(|p| node(&mut v, p));
                tri(&v, [c[0], c[1], c[2]], &mut facets);
                tri(&v, [c[0], c[2], c[3]], &mut facets);
            }
        }
    }
    (v, facets)
}

fn hybrid(
    nl: usize,
    nx: usize,
    n: usize,
    len: f64,
    options: HybridOptions,
) -> (Mesh, solid_kernel::hybrid::HybridReport) {
    let half = len / 2.;
    let hex = grid(
        Physics::Solid,
        ElementKind::Hex8,
        mat(),
        [nl, n, n],
        &move |p| [p[0] * half, p[1], p[2]],
    )
    .unwrap();
    // interface quadrilaterals: boundary faces on x = half, outward from the hexahedra
    let interface: Vec<[usize; 4]> = hex
        .boundary_faces()
        .iter()
        .filter(|f| {
            f.nodes
                .iter()
                .all(|&i| (hex.nodes[i][0] - half).abs() < 1e-12)
        })
        .map(|f| [f.nodes[0], f.nodes[1], f.nodes[2], f.nodes[3]])
        .collect();
    assert_eq!(interface.len(), n * n);
    let (sv, sf) = region_shell(half, len, nx, n);
    hex_pyramid_tet(&hex, &interface, &sv, &sf, options).unwrap()
}

fn tip_deflection(mesh: &Mesh, len: f64) -> f64 {
    let model = Model::new(mesh.clone()).unwrap();
    let mut bc = model.dirichlet();
    let mut tip = Vec::new();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0] < 1e-9 {
            bc.fix_node(n);
        }
        if x[0] > len - 1e-9 {
            tip.push(n);
        }
    }
    let loads = Loads {
        nodal: tip
            .iter()
            .map(|&n| (n, [0.0, -1.0 / tip.len() as f64, 0.0]))
            .collect(),
        ..Loads::default()
    };
    let sol = model.solve_static(&loads, &bc).unwrap();
    tip.iter().map(|&n| sol.u[n * 3 + 1]).sum::<f64>() / tip.len() as f64
}

#[test]
fn the_hybrid_mesh_conforms_passes_the_linear_patch_test_and_bends_like_hexahedra() {
    let len = 4.0;
    let (mesh, report) = hybrid(4, 4, 2, len, HybridOptions::default());
    eprintln!("hybrid mesh: {report:?}");
    assert_eq!((report.hexahedra, report.pyramids), (4 * 2 * 2, 2 * 2));
    let adj = Adjacency::new(&mesh);
    let c = check_conformity(&mesh, &adj);
    assert_eq!(
        (
            c.non_manifold_faces,
            c.mismatched_faces,
            c.n_hanging,
            c.coincident_pairs,
            c.unused_nodes,
            c.components
        ),
        (0, 0, 0, 0, 0, 1),
        "{c:?}"
    );
    let q = mesh.quality();
    assert_eq!(q.inverted, 0);
    // linear patch test: the whole boundary prescribed, every interior node (apexes, tetrahedron nodes, interface nodes) must follow
    let model = Model::new(mesh.clone()).unwrap();
    let field = |x: [f64; 3]| {
        [
            1e-3 * x[0] + 2e-4 * x[1] - 3e-4 * x[2],
            -1e-4 * x[0] + 5e-4 * x[1] + 2e-4 * x[2],
            3e-4 * x[0] - 2e-4 * x[1] + 4e-4 * x[2],
        ]
    };
    let mut bc = model.dirichlet();
    let mut interior = Vec::new();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let boundary = x[0] < 1e-9
            || x[0] > len - 1e-9
            || x[1] < 1e-9
            || x[1] > 1.0 - 1e-9
            || x[2] < 1e-9
            || x[2] > 1.0 - 1e-9;
        if boundary {
            let u = field(*x);
            for c in 0..3 {
                bc.fix(n, c, u[c]);
            }
        } else {
            interior.push(n);
        }
    }
    let sol = model.solve_static(&Loads::default(), &bc).unwrap();
    let on_pyramids: std::collections::BTreeSet<usize> = mesh
        .blocks
        .iter()
        .filter(|b| b.kind == ElementKind::Pyramid5)
        .flat_map(|b| b.conn.iter().copied())
        .collect();
    let interior_in_pyramids = interior.iter().filter(|n| on_pyramids.contains(n)).count();
    assert!(
        interior_in_pyramids >= 4,
        "the apexes are interior nodes: {interior_in_pyramids}"
    );
    let worst = interior
        .iter()
        .map(|&n| {
            (0..3)
                .map(|c| (sol.u[n * 3 + c] - field(model.mesh.nodes[n])[c]).abs())
                .fold(0.0, f64::max)
        })
        .fold(0.0, f64::max);
    eprintln!(
        "patch test over {} interior nodes ({} of the pyramids): worst error {worst:.3e}",
        interior.len(),
        interior_in_pyramids
    );
    assert!(worst < 1e-10, "{worst}");
}

#[test]
fn bending_through_the_pyramid_layer_is_between_the_all_hexahedron_and_the_all_tetrahedron_answers()
{
    let len = 4.0;
    for (n, nx) in [(2usize, 4usize), (4, 8)] {
        let nl = 2 * n; // hexahedra of the same size as the cross-section cells
        let (mixed, report) = hybrid(nl, nx, n, len, HybridOptions::default());
        let hex_only = grid(
            Physics::Solid,
            ElementKind::Hex8,
            mat(),
            [4 * n, n, n],
            &move |p| [p[0] * len, p[1], p[2]],
        )
        .unwrap();
        let tet_only = {
            let part = grid(
                Physics::Solid,
                ElementKind::Hex8,
                mat(),
                [4 * n, n, n],
                &move |p| [p[0] * len, p[1], p[2]],
            )
            .unwrap();
            tetrahedralize(&part, &|_, _, _| true).unwrap().0
        };
        // the node tie of Phase 94 at the same resolution
        let tie = {
            let half = len / 2.;
            let left = grid(
                Physics::Solid,
                ElementKind::Hex8,
                mat(),
                [2 * n, n, n],
                &move |p| [p[0] * half, p[1], p[2]],
            )
            .unwrap();
            let right = grid(
                Physics::Solid,
                ElementKind::Hex8,
                mat(),
                [2 * n, n, n],
                &move |p| [half + p[0] * half, p[1], p[2]],
            )
            .unwrap();
            merge(
                &[left, tetrahedralize(&right, &|_, _, _| true).unwrap().0],
                1e-9,
            )
            .unwrap()
        };
        let (h, m, t, w) = (
            tip_deflection(&hex_only, len),
            tip_deflection(&mixed, len),
            tip_deflection(&tet_only, len),
            tip_deflection(&tie, len),
        );
        eprintln!(
            "{n}x{n} cross-section: tip all-hex {h:.5e}, hex|pyramid|tet {m:.5e} ({:+.2} %), hex|tet tie {w:.5e} ({:+.2} %), all-tet {t:.5e} ({:+.2} %); {} pyramids, {} tetrahedra, {} Steiner, worst radius-edge {:.2}",
            100.0 * (m / h - 1.0),
            100.0 * (w / h - 1.0),
            100.0 * (t / h - 1.0),
            report.pyramids,
            report.tetrahedra,
            report.steiner_points,
            report.max_radius_edge_ratio
        );
        let (lo, hi) = (h.abs().min(t.abs()), h.abs().max(t.abs()));
        assert!(
            m.abs() >= lo * 0.97 && m.abs() <= hi * 1.03,
            "mixed {m} outside [{lo}, {hi}]"
        );
    }
}
