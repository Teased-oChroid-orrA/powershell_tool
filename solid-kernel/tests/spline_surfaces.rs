//! Rational B-spline surface faces: the fillet surfaces OpenCASCADE writes where a rounded edge follows an intersection curve
//! (`tests/data/README.md`). The native import charts them by the parameter rectangle, follows a face's seam by continuity, and
//! must produce a closed shell whose volume matches OpenCASCADE's own tessellation of the same file.
//! `docs/issue-46-phase-105.md`.

use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cylindrical_step::ChartPolicy;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};

struct Case {
    name: &'static str,
    source: &'static str,
    /// OpenCASCADE tessellation volume at 0.001 mm deflection.
    reference: f64,
    /// Source faces of the file (every one is meshed and tagged).
    faces: usize,
    /// A fillet face on a B-spline surface whose triangles are checked after grading.
    spline_face: Option<u64>,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "cylinder with a filleted cross bore",
            source: include_str!("data/cross-bore-filleted.step"),
            reference: REF_CROSS_BORE,
            faces: 9,
            spline_face: Some(1680),
        },
        Case {
            name: "cylinder with a filleted perpendicular branch",
            source: include_str!("data/t-junction-filleted.step"),
            reference: REF_T_JUNCTION,
            faces: 14,
            spline_face: None,
        },
        Case {
            name: "block with a filleted tilted hole",
            source: include_str!("data/tilted-hole-filleted.step"),
            reference: REF_TILTED_HOLE,
            faces: 29,
            spline_face: None,
        },
        Case {
            name: "periodic spline shell (unclamped surface knots)",
            source: include_str!("data/periodic-shell.step"),
            reference: REF_PERIODIC,
            faces: 1,
            spline_face: None,
        },
    ]
}
const REF_PERIODIC: f64 = 943.199593;
const REF_CROSS_BORE: f64 = 5578.128824;
const REF_T_JUNCTION: f64 = 3810.042464;
const REF_TILTED_HOLE: f64 = 22835.243076;

fn sample(source: &str) -> Result<SampledSurface, String> {
    let data = inventory(source, Limits::default()).map_err(|e| e.to_string())?;
    AnalyticModel::read(
        &data,
        Options {
            chord_mm: 0.01,
            geometry_tolerance_mm: 1e-4,
            ..Options::default()
        },
    )
    .and_then(|m| {
        m.sampled_surface(
            SurfaceLimits {
                chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
                face_work: 20_000_000,
                total_work: 800_000_000,
                ..SurfaceLimits::default()
            },
            |_| true,
        )
    })
    .map_err(|e| e.to_string())
}

#[test]
fn filleted_intersections_import_as_closed_shells_with_the_reference_volume() {
    for c in cases() {
        let s = sample(c.source).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        eprintln!("{:<50} native {:.6}", c.name, s.signed_volume_mm3);
        assert_eq!(s.source_faces.len(), c.faces, "{}", c.name);
        // every face is meshed: each source face tags at least one triangle
        let mut tagged = std::collections::BTreeSet::new();
        for t in &s.mesh.face {
            tagged.insert(*t);
        }
        assert_eq!(tagged.len(), c.faces, "{}: faces without triangles", c.name);
        let rel = s.signed_volume_mm3 / c.reference - 1.;
        assert!(rel.abs() < 3e-3, "{}: {rel}", c.name);
    }
}

#[cfg(feature = "occt")]
#[test]
// OpenCASCADE's tessellation volume depends on the platform's floating point at about 5e-6 (the revolved vase: 1503.0205 on macOS arm64,
// 1503.0272 on the hosted Linux runner), so the recorded value is checked to 1e-4, far below the 3e-3 the native import is held to.
fn the_recorded_reference_volumes_are_opencascade_s_and_native_agrees_with_them() {
    for c in cases() {
        let live = solid_kernel::occt::Brep::from_step_text(c.source)
            .unwrap()
            .solid(0.001)
            .unwrap()
            .volume();
        eprintln!(
            "{:<50} OpenCASCADE {live:.6} (recorded {:.6})",
            c.name, c.reference
        );
        assert!(
            (live / c.reference - 1.).abs() < 1e-4,
            "{}: recorded {} but OpenCASCADE says {live}",
            c.name,
            c.reference
        );
    }
}

#[test]
fn grading_and_quality_refinement_lift_the_filleted_shells_without_changing_the_body() {
    use solid_kernel::chart_grade::GradeOptions;
    for c in cases() {
        let data = inventory(c.source, Limits::default()).unwrap();
        let model = AnalyticModel::read(
            &data,
            Options {
                geometry_tolerance_mm: 1e-4,
                ..Options::default()
            },
        )
        .unwrap();
        let limits = |quality| SurfaceLimits {
            chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
            chart_quality: quality,
            face_work: 20_000_000,
            total_work: 800_000_000,
            ..SurfaceLimits::default()
        };
        let plain = model.sampled_surface(limits(None), |_| true).unwrap();
        let before = solid_kernel::plc_conform::min_shell_angle_deg(
            &plain.mesh.vertices,
            &plain.mesh.triangles,
        );
        let (graded, _) = model
            .graded(
                GradeOptions {
                    policy: ChartPolicy::CanonicalizeWithinTolerance,
                    ..GradeOptions::default()
                },
                50_000_000,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let after = graded
            .sampled_surface(
                limits(Some(solid_kernel::chart_mesh::ChartQuality {
                    min_angle_deg: 25.,
                    max_points: 20_000,
                })),
                |_| true,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let angle = solid_kernel::plc_conform::min_shell_angle_deg(
            &after.mesh.vertices,
            &after.mesh.triangles,
        );
        eprintln!("{:<50} {before:>7.3} -> {angle:>7.3} degrees", c.name);
        // the worst corners of these shells are on the tori and planes of the end fillets and on the acute ellipse edges of the
        // tilted hole (Phase 100/101 limits), not on a spline face; grading must not make the shell worse
        // (the plain shell can now be better than the graded one: with decimated spline edges its needles are gone, and the
        // graded corner is the intrinsic one of the end fillets on the filleted cross bore). Phase 157 re-baselined this bound
        // from 0.19 (owner decision): that corner was 0.198 degrees only while grading kept both faces' copies of each split of
        // the small-angle cascade at the fillet end vertex, 2.9e-6 to 2e-8 mm apart under a 1e-4 mm geometry tolerance. With
        // samples kept a geometry tolerance apart the same corner is 0.156 degrees.
        assert!(angle >= before.min(0.15), "{}: {before} -> {angle}", c.name);
        if let Some(face) = c.spline_face {
            let worst = face_min_angle(&after, face);
            eprintln!("  spline face #{face}: worst corner {worst:.2} degrees");
            assert!(
                worst > 10.,
                "{}: spline face #{face} has a {worst} degree corner",
                c.name
            );
        }
        // the same body: extra samples lie on the surface, so refinement moves the inscribed volume by well under 0.5 percent
        let rel = after.signed_volume_mm3 / plain.signed_volume_mm3 - 1.;
        assert!(rel.abs() < 4e-3, "{}: {rel}", c.name);
    }
}

/// The smallest triangle angle (degrees) among the triangles of one source face.
fn face_min_angle(s: &SampledSurface, source_face: u64) -> f64 {
    let tag = s
        .source_faces
        .iter()
        .position(|f| *f == source_face)
        .expect("face in the mesh");
    let mut worst = 180f64;
    for (t, tri) in s.mesh.triangles.iter().enumerate() {
        if s.mesh.face[t] != tag {
            continue;
        }
        let p = tri.map(|i| s.mesh.vertices[i]);
        for k in 0..3 {
            let (a, b, c) = (p[k], p[(k + 1) % 3], p[(k + 2) % 3]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let dot = u[0] * w[0] + u[1] * w[1] + u[2] * w[2];
            let nu = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
            let nw = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
            worst = worst.min((dot / (nu * nw)).clamp(-1., 1.).acos().to_degrees());
        }
    }
    worst
}
