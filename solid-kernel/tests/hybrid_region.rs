//! A hexahedral core inside a natively imported body, joined to tetrahedra by a pyramid layer (`hybrid_region::hex_core_hybrid`):
//! face-conforming, no inverted element, the element volumes add up to the body's, and the mesh reproduces a linear field when
//! the whole boundary is prescribed. `docs/issue-46-phase-111.md`.

use fea_core::meshing::adjacency::{check_conformity, Adjacency};
use fea_core::*;
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::chart_grade::GradeOptions;
use solid_kernel::chart_mesh::ChartQuality;
use solid_kernel::cylindrical_step::ChartPolicy;
use solid_kernel::hybrid_region::{hex_core_hybrid, RegionOptions};
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};
use solid_kernel::{SurfaceSolid, V3};

/// The shell the tetrahedral recovery needs: graded boundary sampling and chart quality (plain sampling leaves needles).
fn shell(source: &str, chord: f64) -> SampledSurface {
    let data = inventory(source, Limits::default()).unwrap();
    let model = AnalyticModel::read(
        &data,
        Options {
            chord_mm: chord,
            geometry_tolerance_mm: 1e-4,
            ..Options::default()
        },
    )
    .unwrap();
    let (graded, _) = model
        .graded(
            GradeOptions {
                policy: ChartPolicy::CanonicalizeWithinTolerance,
                ..GradeOptions::default()
            },
            50_000_000,
        )
        .unwrap();
    graded
        .sampled_surface(
            SurfaceLimits {
                chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
                chart_quality: Some(ChartQuality {
                    min_angle_deg: 25.,
                    max_points: 20_000,
                }),
                face_work: 20_000_000,
                total_work: 800_000_000,
                ..SurfaceLimits::default()
            },
            |_| true,
        )
        .unwrap()
}

fn tet_volume(p: [V3; 4]) -> f64 {
    let d = |a: V3, b: V3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let (a, b, c) = (d(p[1], p[0]), d(p[2], p[0]), d(p[3], p[0]));
    (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0]))
        / 6.0
}

/// Total volume of the mesh: hexahedra as six tetrahedra around the 0-6 diagonal, pyramids as two, tetrahedra as they are.
fn mesh_volume(mesh: &Mesh) -> f64 {
    let mut total = 0.0;
    for b in &mesh.blocks {
        for e in 0..b.n_elems() {
            let n: Vec<V3> = b.elem(e).iter().map(|&i| mesh.nodes[i]).collect();
            total += match b.kind {
                ElementKind::Hex8 => [
                    [0, 1, 2, 6],
                    [0, 2, 3, 6],
                    [0, 3, 7, 6],
                    [0, 7, 4, 6],
                    [0, 4, 5, 6],
                    [0, 5, 1, 6],
                ]
                .iter()
                .map(|t| tet_volume(t.map(|i| n[i])))
                .sum::<f64>(),
                ElementKind::Pyramid5 => {
                    tet_volume([n[0], n[1], n[2], n[4]]) + tet_volume([n[0], n[2], n[3], n[4]])
                }
                ElementKind::Tet4 => tet_volume([n[0], n[1], n[2], n[3]]),
                other => panic!("unexpected {other:?}"),
            };
        }
    }
    total
}

#[test]
fn a_hexahedral_core_inside_a_native_body_conforms_and_fills_it() {
    let sampled = shell(include_str!("data/tilted-hole.step"), 0.2);
    let solid = SurfaceSolid::new(sampled.mesh.clone(), 1e-9).unwrap();
    let options = RegionOptions {
        cell: 2.5,
        margin_cells: 2.0,
        ..RegionOptions::default()
    };
    let (mesh, report) =
        hex_core_hybrid(&sampled.mesh, Elastic::new(1000.0, 0.3), options).unwrap();
    eprintln!("tilted-hole core: {report:?}");
    assert!(
        report.hexahedra >= 50 && report.pyramids > 0 && report.tetrahedra > 0,
        "{report:?}"
    );
    assert_eq!(mesh.quality().inverted, 0);
    let c = check_conformity(&mesh, &Adjacency::new(&mesh));
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
    // the elements fill exactly the body
    let (v, body) = (mesh_volume(&mesh), solid.volume());
    eprintln!("element volume {v:.6}, body volume {body:.6}");
    assert!((v / body - 1.0).abs() < 1e-9, "{v} against {body}");
}

#[test]
fn the_cored_mesh_reproduces_a_linear_field_through_hexahedra_pyramids_and_tetrahedra() {
    let sampled = shell(include_str!("data/tilted-hole.step"), 0.2);
    let solid = SurfaceSolid::new(sampled.mesh.clone(), 1e-9).unwrap();
    let options = RegionOptions {
        cell: 2.5,
        margin_cells: 2.0,
        ..RegionOptions::default()
    };
    let (mesh, _) = hex_core_hybrid(&sampled.mesh, Elastic::new(1000.0, 0.3), options).unwrap();
    let field = |x: V3| {
        [
            1e-3 * x[0] + 2e-4 * x[1] - 3e-4 * x[2],
            -1e-4 * x[0] + 5e-4 * x[1] + 2e-4 * x[2],
            3e-4 * x[0] - 2e-4 * x[1] + 4e-4 * x[2],
        ]
    };
    let model = Model::new(mesh.clone()).unwrap();
    let mut bc = model.dirichlet();
    let mut interior = Vec::new();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if solid.signed_distance(*x).abs() < 1e-7 {
            let u = field(*x);
            for k in 0..3 {
                bc.fix(n, k, u[k]);
            }
        } else {
            interior.push(n);
        }
    }
    assert!(
        interior.len() > 100,
        "only {} interior nodes",
        interior.len()
    );
    let sol = model.solve_static(&Loads::default(), &bc).unwrap();
    let worst = interior
        .iter()
        .map(|&n| {
            (0..3)
                .map(|k| (sol.u[n * 3 + k] - field(model.mesh.nodes[n])[k]).abs())
                .fold(0.0, f64::max)
        })
        .fold(0.0, f64::max);
    eprintln!(
        "patch test over {} interior nodes: worst error {worst:.3e}",
        interior.len()
    );
    assert!(worst < 1e-9, "{worst}");
}

#[test]
fn a_body_too_small_for_the_margin_has_no_core() {
    let sampled = shell(include_str!("data/tilted-hole.step"), 0.2);
    let options = RegionOptions {
        cell: 12.0,
        margin_cells: 2.0,
        ..RegionOptions::default()
    };
    let e = hex_core_hybrid(&sampled.mesh, Elastic::new(1000.0, 0.3), options).unwrap_err();
    assert!(e.contains("no hexahedral core fits"), "{e}");
}

#[test]
fn a_filleted_cylinder_gets_a_core_too() {
    // a cylinder with a rounded top: curved faces on both sides of the core
    let sampled = shell(include_str!("data/cylinder-filleted-top.step"), 0.2);
    let solid = SurfaceSolid::new(sampled.mesh.clone(), 1e-9).unwrap();
    let options = RegionOptions {
        cell: 1.0,
        margin_cells: 2.0,
        ..RegionOptions::default()
    };
    let (mesh, report) =
        hex_core_hybrid(&sampled.mesh, Elastic::new(1000.0, 0.3), options).unwrap();
    eprintln!("filleted-cylinder core: {report:?}");
    assert!(report.hexahedra > 0 && report.tetrahedra > 0);
    let c = check_conformity(&mesh, &Adjacency::new(&mesh));
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
    assert_eq!(mesh.quality().inverted, 0);
    assert!((mesh_volume(&mesh) / solid.volume() - 1.0).abs() < 1e-9);
}
