//! Edges that are neither lines nor circles: ellipses (a tilted hole through a block) and B-spline intersection curves (two
//! cylinders meeting at right angles), on faces with holes. STEP written by OpenCASCADE (`tests/data/README.md`); the reference
//! volumes are OpenCASCADE's own tessellation at 0.001 mm (recorded below, recomputed live by the `occt` test).
//! `docs/issue-46-phase-104.md`.

use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::chart_grade::GradeOptions;
use solid_kernel::cylindrical_step::ChartPolicy;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};
use std::f64::consts::PI;

struct Case {
    name: &'static str,
    source: &'static str,
    /// OpenCASCADE tessellation volume at 0.001 mm deflection.
    reference: f64,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "block with a tilted hole (ellipse edges)",
            source: include_str!("data/tilted-hole.step"),
            reference: 22858.349437,
        },
        Case {
            name: "cylinder with a perpendicular branch (spline edges)",
            source: include_str!("data/t-junction.step"),
            reference: 3830.402787,
        },
        Case {
            name: "cylinder with a cross bore (spline edges, faces with holes)",
            source: include_str!("data/cross-bore.step"),
            reference: 5587.212225,
        },
    ]
}

fn sample(source: &str, chord_mm: f64) -> Result<SampledSurface, String> {
    let data = inventory(source, Limits::default()).map_err(|e| e.to_string())?;
    AnalyticModel::read(
        &data,
        Options {
            chord_mm,
            ..Options::default()
        },
    )
    .and_then(|m| {
        m.sampled_surface(
            SurfaceLimits {
                chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
                ..SurfaceLimits::default()
            },
            |_| true,
        )
    })
    .map_err(|e| e.to_string())
}

#[test]
fn ellipse_and_spline_edges_import_and_match_the_reference_volume() {
    for c in cases() {
        let s = sample(c.source, 0.01).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        eprintln!("{:<62} native {:.4}", c.name, s.signed_volume_mm3);
        if c.reference > 0. {
            let rel = s.signed_volume_mm3 / c.reference - 1.;
            assert!(rel.abs() < 3e-3, "{}: {rel}", c.name);
        }
    }
    // the tilted hole has a closed form: an oblique cylinder between two parallel planes has volume pi r^2 H / cos(axis, normal)
    let s = sample(include_str!("data/tilted-hole.step"), 0.005).unwrap();
    let axis_len = (0.5f64 * 0.5 + 0.2 * 0.2 + 1.0).sqrt();
    let hole = PI * 16. * 20. * axis_len;
    let exact = 24000. - hole;
    let rel = s.signed_volume_mm3 / exact - 1.;
    assert!(
        rel > 0. && rel < 2e-3,
        "an inscribed hole is a little small, so the block a little large: {rel}"
    );
}

#[test]
fn grading_lifts_the_shells_with_spline_edges_and_keeps_the_edge_on_its_curve() {
    for c in cases() {
        let data = inventory(c.source, Limits::default()).unwrap();
        let model = AnalyticModel::read(&data, Options::default()).unwrap();
        let plain = model
            .sampled_surface(
                SurfaceLimits {
                    chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
                    ..SurfaceLimits::default()
                },
                |_| true,
            )
            .unwrap();
        let before = solid_kernel::plc_conform::min_shell_angle_deg(
            &plain.mesh.vertices,
            &plain.mesh.triangles,
        );
        let options = GradeOptions {
            policy: ChartPolicy::CanonicalizeWithinTolerance,
            ..GradeOptions::default()
        };
        let (graded, report) = model
            .graded(options, 50_000_000)
            .unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let limits = SurfaceLimits {
            chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
            chart_quality: Some(solid_kernel::chart_mesh::ChartQuality {
                min_angle_deg: 25.,
                max_points: 20_000,
            }),
            face_work: 20_000_000,
            total_work: 800_000_000,
            ..SurfaceLimits::default()
        };
        let after = graded
            .sampled_surface(limits, |_| true)
            .unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let angle = solid_kernel::plc_conform::min_shell_angle_deg(
            &after.mesh.vertices,
            &after.mesh.triangles,
        );
        eprintln!(
            "{:<62} {before:>7.3} -> {angle:>7.3} degrees ({report:?})",
            c.name
        );
        assert!(angle > before * 5., "{}: {before} -> {angle}", c.name);
        // the extra edge samples lie on the curve: the volume is the same body
        let rel = after.signed_volume_mm3 / plain.signed_volume_mm3 - 1.;
        assert!(rel.abs() < 2e-3, "{}: {rel}", c.name);
    }
}

#[cfg(feature = "occt")]
#[test]
fn the_recorded_reference_volumes_are_opencascade_s_and_native_agrees_with_them() {
    for c in cases() {
        let live = solid_kernel::occt::Brep::from_step_text(c.source)
            .unwrap()
            .solid(0.001)
            .unwrap()
            .volume();
        eprintln!(
            "{:<62} OpenCASCADE {live:.6} (recorded {:.6})",
            c.name, c.reference
        );
        assert!(
            (live / c.reference - 1.).abs() < 1e-6,
            "{}: recorded {} but OpenCASCADE says {live}",
            c.name,
            c.reference
        );
        let native = sample(c.source, 0.01).unwrap();
        let rel = native.signed_volume_mm3 / live - 1.;
        assert!(
            rel.abs() < 2e-3,
            "{}: native {} vs OpenCASCADE {live}: {rel}",
            c.name,
            native.signed_volume_mm3
        );
    }
}
