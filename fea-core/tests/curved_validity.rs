//! Curved quadratic tetrahedra from the signed-distance mesher: every element of a sphere mesh is proved
//! valid over its whole volume (not just at the corners), the curved boundary integrates the volume better than the
//! straight-sided mesh does, and a deliberately damaged element is found.
use fea_core::meshing::curved::{check_mesh, simplex_validity, Verdict};
use fea_core::tetmesh::{mesh_solid, Solid, TetOptions};
use fea_core::{Elastic, ElementKind, Mesh};

/// Volume by the mass-matrix rule (exact for the cubic Jacobian of a Tet10).
fn integrated_volume(mesh: &Mesh) -> f64 {
    let mut volume = 0.0;
    for block in &mesh.blocks {
        let table = block.kind.mass_table();
        let nn = block.kind.n_nodes();
        for conn in block.conn.chunks(nn) {
            for g in 0..table.ngp {
                let mut cols = [[0.0; 3]; 3];
                for a in 0..nn {
                    for k in 0..3 {
                        for r in 0..3 {
                            cols[k][r] += table.dn[(g * nn + a) * 3 + k] * mesh.nodes[conn[a]][r];
                        }
                    }
                }
                let det = cols[0][0] * (cols[1][1] * cols[2][2] - cols[1][2] * cols[2][1])
                    - cols[1][0] * (cols[0][1] * cols[2][2] - cols[0][2] * cols[2][1])
                    + cols[2][0] * (cols[0][1] * cols[1][2] - cols[0][2] * cols[1][1]);
                volume += table.w[g] * det;
            }
        }
    }
    volume
}

fn sphere(h: f64, quadratic: bool) -> Mesh {
    let sdf = |x: [f64; 3]| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() - 1.0;
    let (size, face) = (|_: [f64; 3]| 10.0, |_: [f64; 3]| 0usize);
    let solid = Solid {
        lo: [-1.2; 3],
        hi: [1.2; 3],
        sdf: &sdf,
        size: &size,
        face_of: &face,
        face_dists: None,
    };
    mesh_solid(
        &solid,
        Elastic::new(1.0, 0.3),
        TetOptions {
            base: 0.6,
            boundary_size: h,
            quadratic,
            ..Default::default()
        },
    )
    .unwrap()
    .0
}

#[test]
fn every_element_of_a_curved_sphere_mesh_is_proved_valid_and_the_curved_boundary_is_more_accurate()
{
    let exact = 4.0 / 3.0 * std::f64::consts::PI;
    for h in [0.5, 0.3] {
        let curved = sphere(h, true);
        let report = check_mesh(&curved, 6).unwrap();
        assert!(
            report.checked > 50 && report.all_valid(),
            "h {h}: {report:?}"
        );
        assert!(report.min_ratio_bound > 0.0, "{report:?}");
        let straight = sphere(h, false);
        assert!(
            check_mesh(&straight, 6).unwrap().checked == 0,
            "Tet4 is not a curved element"
        );
        let (ec, es) = (
            (integrated_volume(&curved) - exact).abs() / exact,
            (integrated_volume(&straight) - exact).abs() / exact,
        );
        eprintln!("h {h}: curved volume error {ec:.3e}, straight {es:.3e}, {} elements, min Jacobian bound {:.3}", report.checked, report.min_ratio_bound);
        assert!(
            ec < es,
            "the curved boundary is closer to the sphere: {ec} vs {es}"
        );
    }
}

#[test]
fn a_damaged_element_is_located_by_the_mesh_check() {
    let mut mesh = sphere(0.5, true);
    let n = mesh.blocks[0].kind.n_nodes();
    assert_eq!(mesh.blocks[0].kind, ElementKind::Tet10);
    // Push one midside node far across its element.
    let victim = 17;
    let conn: Vec<usize> = mesh.blocks[0].conn[victim * n..(victim + 1) * n].to_vec();
    let corner = mesh.nodes[conn[0]];
    mesh.nodes[conn[9]] = [corner[0] + 3.0, corner[1] - 3.0, corner[2] + 3.0];
    let report = check_mesh(&mesh, 6).unwrap();
    assert!(report.invalid >= 1 && !report.all_valid(), "{report:?}");
    let (block, element) = report.first_invalid.unwrap();
    assert_eq!(block, 0);
    // The shared midside node damages every element that uses it; the victim is among them.
    let damaged: Vec<usize> = mesh.blocks[0]
        .conn
        .chunks(n)
        .enumerate()
        .filter(|(_, c)| c.contains(&conn[9]))
        .map(|(e, _)| e)
        .collect();
    assert!(damaged.contains(&element) && damaged.contains(&victim));
    let xyz: Vec<[f64; 3]> = conn.iter().map(|&i| mesh.nodes[i]).collect();
    assert_eq!(
        simplex_validity(ElementKind::Tet10, &xyz, 6)
            .unwrap()
            .verdict,
        Verdict::Invalid
    );
}
