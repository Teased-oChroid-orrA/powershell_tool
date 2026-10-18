//! Graded boundary sampling plus interior chart quality (`AnalyticModel::graded`, `native_surface::Limits::chart_quality`):
//! the sampled NEMA17 bracket shell. The plain per-face triangulation leaves needles on most faces (sparse boundary
//! samples, no interior points); grading removes them wherever the CAD itself has no near-degenerate feature.
//! `docs/issue-46-phase-100.md`.
mod common;

use common::SOURCE;
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::chart_grade::GradeOptions;
use solid_kernel::chart_mesh::ChartQuality;
use solid_kernel::native_surface::Limits as SurfaceLimits;
use solid_kernel::step::{inventory, Limits};

/// Fraction of the triangles with a corner below 18 degrees.
fn thin_fraction(vertices: &[[f64; 3]], triangles: &[[usize; 3]]) -> f64 {
    let thin = triangles
        .iter()
        .filter(|t| {
            let p = t.map(|i| vertices[i]);
            (0..3).any(|k| {
                let (a, b, c) = (p[k], p[(k + 1) % 3], p[(k + 2) % 3]);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let n = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                let cos = (u[0] * w[0] + u[1] * w[1] + u[2] * w[2]) / (n(u) * n(w));
                cos.clamp(-1., 1.).acos().to_degrees() < 18.
            })
        })
        .count();
    thin as f64 / triangles.len() as f64
}

#[test]
fn grading_removes_the_needles_of_the_bracket_shell_without_changing_its_shape() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let plain = model
        .sampled_surface(SurfaceLimits::default(), |_| true)
        .unwrap();
    let before = thin_fraction(&plain.mesh.vertices, &plain.mesh.triangles);
    assert!(
        before > 0.4,
        "the plain shell has needles on most faces: {before}"
    );

    let (graded, report) = model.graded(GradeOptions::default(), 50_000_000).unwrap();
    assert!(
        report.converged && report.samples_added > 1000,
        "{report:?}"
    );
    // A converged model asks for nothing more.
    let (_, again) = graded.graded(GradeOptions::default(), 50_000_000).unwrap();
    assert_eq!(
        (again.rounds, again.samples_added, again.converged),
        (0, 0, true),
        "{again:?}"
    );
    let refined = graded
        .sampled_surface(
            SurfaceLimits {
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
        .unwrap();
    let after = thin_fraction(&refined.mesh.vertices, &refined.mesh.triangles);
    assert!(after < 0.1, "needles remain on {after} of the triangles");
    // The same body: only points on the exact surface were added, and the curved faces get finer.
    let (v0, v1) = (plain.signed_volume_mm3, refined.signed_volume_mm3);
    assert!((v1 / v0 - 1.).abs() < 5e-4, "volume {v0} -> {v1}");
    assert_eq!(refined.source_faces, plain.source_faces);
}
