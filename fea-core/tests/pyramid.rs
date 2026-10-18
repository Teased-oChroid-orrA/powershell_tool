//! The five-node pyramid joins a hexahedron (bilinear quadrilateral face) to tetrahedra (triangular faces) face-conforming.
//! `docs/issue-46-phase-102.md`.

use fea_core::meshing::adjacency::{check_conformity, Adjacency};
use fea_core::*;

fn mat() -> Elastic {
    Elastic::new(1000.0, 0.3)
}

/// Shape functions of the pyramid restricted to its faces equal those of the neighbour's face: bilinear on the base,
/// the three linear barycentric functions on a side.
#[test]
fn the_pyramid_traces_are_the_hexahedron_and_tetrahedron_traces() {
    let k = ElementKind::Pyramid5;
    // base (zeta = 0): bilinear in xi, eta
    for (xi, eta) in [(0.3, -0.4), (-0.9, 0.2), (0.0, 0.0), (1.0, 1.0)] {
        let (n, _) = k.shape([xi, eta, 0.0]);
        let quad = [
            0.25 * (1.0 - xi) * (1.0 - eta),
            0.25 * (1.0 + xi) * (1.0 - eta),
            0.25 * (1.0 + xi) * (1.0 + eta),
            0.25 * (1.0 - xi) * (1.0 + eta),
        ];
        for a in 0..4 {
            assert!((n[a] - quad[a]).abs() < 1e-14);
        }
        assert!(n[4].abs() < 1e-14);
    }
    // each side: the node opposite the side vanishes and the other three are the barycentric coordinates of the triangle
    // (nodes a, b, apex) in the physical triangle, i.e. linear in the coordinates of the face
    for (a, b, sign, axis) in [
        (0usize, 1usize, -1.0f64, 1usize),
        (1, 2, 1.0, 0),
        (2, 3, 1.0, 1),
        (3, 0, -1.0, 0),
    ] {
        for (s, z) in [(0.2, 0.3), (-0.5, 0.1), (0.7, 0.6), (0.0, 0.9)] {
            // a point on the side: s (1 - z) along it, sign (1 - z) across
            let along = s * (1.0 - z);
            let (px, py) = if axis == 1 {
                (along, sign * (1.0 - z))
            } else {
                (sign * (1.0 - z), along)
            };
            let (n, _) = k.shape([px, py, z]);
            for i in (0..4).filter(|&i| i != a && i != b) {
                assert!(n[i].abs() < 1e-13, "node {i} on side ({a},{b}): {}", n[i]);
            }
            assert!((n[a] + n[b] + n[4] - 1.0).abs() < 1e-13);
            assert!((n[4] - z).abs() < 1e-14);
        }
    }
}

/// A block [0, 1]^3 cut at x = 1/2: a hexahedron on the left; on the right the interface quad carries a pyramid with apex A, the far
/// face carries a second pyramid with the same apex, and each lateral wall is a pyramid on A whose quadrilateral base is split into
/// two tetrahedra (so the right part has two pyramids and eight tetrahedra, joined face-conforming to the hexahedron).
fn block(map: &dyn Fn([f64; 3]) -> [f64; 3], apex: [f64; 3]) -> Mesh {
    let mut m = Mesh::new(Physics::Solid);
    let p = |x: f64, y: f64, z: f64| map([x, y, z]);
    // hexahedron nodes (VTK order): x = 0 face then x = 1/2 face
    let h: Vec<usize> = [
        [0., 0., 0.],
        [0., 1., 0.],
        [0., 1., 1.],
        [0., 0., 1.],
        [0.5, 0., 0.],
        [0.5, 1., 0.],
        [0.5, 1., 1.],
        [0.5, 0., 1.],
    ]
    .iter()
    .map(|c| m.add_node(p(c[0], c[1], c[2])))
    .collect();
    // The VTK hexahedron wants (bottom ccw, top ccw): bottom z = 0 is not used here; use the x direction as the "vertical" one:
    // bottom face x = 0 seen from +x must be counter-clockwise: (y, z) = (0,0), (1,0), (1,1), (0,1).
    let hex = [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]];
    m.add_block(ElementKind::Hex8, hex.to_vec(), mat(), "hex")
        .unwrap();
    // interface quad q (x = 1/2) and far quad f (x = 1), in the same (y, z) order
    let q = [h[4], h[5], h[6], h[7]];
    let f: Vec<usize> = [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]]
        .iter()
        .map(|c| m.add_node(p(c[0], c[1], c[2])))
        .collect();
    let a = m.add_node(map(apex));
    // P1: base q, ccw seen from the apex side (+x): (y, z) order is ccw seen from +x as above
    m.add_block(
        ElementKind::Pyramid5,
        vec![q[0], q[1], q[2], q[3], a],
        mat(),
        "p1",
    )
    .unwrap();
    // P2: base f, the apex is on its -x side: reverse the order
    m.add_block(
        ElementKind::Pyramid5,
        vec![f[0], f[3], f[2], f[1], a],
        mat(),
        "p2",
    )
    .unwrap();
    // lateral tetrahedra: (a, q_i, q_{i+1}, f_{i+1}) and (a, q_i, f_{i+1}, f_i)
    let mut tets = Vec::new();
    for i in 0..4 {
        let j = (i + 1) % 4;
        tets.extend([a, q[i], q[j], f[j]]);
        tets.extend([a, q[i], f[j], f[i]]);
    }
    // orient every tetrahedron positively
    let mut oriented = Vec::new();
    for t in tets.chunks(4) {
        let x: Vec<[f64; 3]> = t.iter().map(|&n| m.nodes[n]).collect();
        let d = |u: usize| [x[u][0] - x[0][0], x[u][1] - x[0][1], x[u][2] - x[0][2]];
        let (b, c, e) = (d(1), d(2), d(3));
        let vol = b[0] * (c[1] * e[2] - c[2] * e[1]) - b[1] * (c[0] * e[2] - c[2] * e[0])
            + b[2] * (c[0] * e[1] - c[1] * e[0]);
        if vol > 0.0 {
            oriented.extend_from_slice(t);
        } else {
            oriented.extend([t[0], t[2], t[1], t[3]]);
        }
    }
    m.add_block(ElementKind::Tet4, oriented, mat(), "tets")
        .unwrap();
    m
}

fn field(g: [[f64; 3]; 3], x: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| 1e-3 + g[r][0] * x[0] + g[r][1] * x[1] + g[r][2] * x[2])
}

#[test]
fn a_hexahedron_pyramid_tetrahedron_block_is_face_conforming_and_passes_the_linear_patch_test() {
    let g = [
        [1e-3, 2e-4, -3e-4],
        [-1e-4, 5e-4, 2e-4],
        [3e-4, -2e-4, 4e-4],
    ];
    // an affine image of the cube (shear, scale, rotation-like) and apexes inside the right half
    let affine = |x: [f64; 3]| {
        [
            2.0 * x[0] + 0.3 * x[1] - 0.2 * x[2] + 1.0,
            -0.1 * x[0] + 1.5 * x[1] + 0.2 * x[2],
            0.2 * x[0] - 0.4 * x[1] + 1.2 * x[2] - 2.0,
        ]
    };
    for (name, map, apex) in [
        (
            "cube",
            &(|x: [f64; 3]| x) as &dyn Fn([f64; 3]) -> [f64; 3],
            [0.75, 0.5, 0.5],
        ),
        (
            "cube, apex off the axis",
            &(|x: [f64; 3]| x),
            [0.8, 0.4, 0.6],
        ),
        ("affine image", &affine, [0.75, 0.5, 0.5]),
    ] {
        let mesh = block(map, apex);
        let adj = Adjacency::new(&mesh);
        let report = check_conformity(&mesh, &adj);
        assert_eq!(
            (
                report.non_manifold_faces,
                report.mismatched_faces,
                report.n_hanging,
                report.coincident_pairs,
                report.unused_nodes
            ),
            (0, 0, 0, 0, 0),
            "{name}: {report:?}"
        );
        let q = mesh.quality();
        assert_eq!(q.inverted, 0, "{name}: {q:?}");
        let model = Model::new(mesh.clone()).unwrap();
        let mut bc = model.dirichlet();
        let apex_node = mesh.nodes.len() - 1;
        for n in 0..mesh.nodes.len() {
            if n == apex_node {
                continue;
            }
            let u = field(g, mesh.nodes[n]);
            for c in 0..3 {
                bc.fix(n, c, u[c]);
            }
        }
        let sol = model.solve_static(&Loads::default(), &bc).unwrap();
        let want = field(g, mesh.nodes[apex_node]);
        let err = (0..3)
            .map(|c| (sol.u[apex_node * 3 + c] - want[c]).abs())
            .fold(0.0, f64::max);
        eprintln!("{name}: apex displacement error {err:.3e}");
        assert!(err < 1e-12, "{name}: {err}");
        // the volumes add up
        let vol: f64 = (0..mesh.blocks.len()).map(|b| block_volume(&mesh, b)).sum();
        let exact = {
            // determinant of the affine map (cube volume 1)
            let j = |a: [f64; 3], b: [f64; 3]| {
                [
                    map(a)[0] - map(b)[0],
                    map(a)[1] - map(b)[1],
                    map(a)[2] - map(b)[2],
                ]
            };
            let (e0, e1, e2) = (
                j([1., 0., 0.], [0., 0., 0.]),
                j([0., 1., 0.], [0., 0., 0.]),
                j([0., 0., 1.], [0., 0., 0.]),
            );
            (e0[0] * (e1[1] * e2[2] - e1[2] * e2[1]) - e0[1] * (e1[0] * e2[2] - e1[2] * e2[0])
                + e0[2] * (e1[0] * e2[1] - e1[1] * e2[0]))
                .abs()
        };
        assert!(
            (vol / exact - 1.0).abs() < 1e-12,
            "{name}: volume {vol} vs {exact}"
        );
    }
}

/// Volume of a block by its own quadrature.
fn block_volume(mesh: &Mesh, b: usize) -> f64 {
    let blk = &mesh.blocks[b];
    let t = blk.kind.table();
    let mut total = 0.0;
    for e in 0..blk.n_elems() {
        let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| mesh.nodes[n]).collect();
        for g in 0..t.ngp {
            let dn = t.dn_at(g);
            let mut j = [[0.0; 3]; 3];
            for a in 0..t.nn {
                for r in 0..3 {
                    for c in 0..3 {
                        j[r][c] += xyz[a][r] * dn[a * 3 + c];
                    }
                }
            }
            let det = j[0][0] * (j[1][1] * j[2][2] - j[1][2] * j[2][1])
                - j[0][1] * (j[1][0] * j[2][2] - j[1][2] * j[2][0])
                + j[0][2] * (j[1][0] * j[2][1] - j[1][1] * j[2][0]);
            total += t.w[g] * det;
        }
    }
    total
}
