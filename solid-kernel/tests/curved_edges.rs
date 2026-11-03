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

/// `cross-bore.step` with its spline edge #79 split in two at a point inside the curve: both pieces keep the whole curve
/// (`SURFACE_CURVE` #82), as exporters write an edge that uses part of a spline (OpenCASCADE's `screw.step` does). The pieces
/// read, end at the new vertex and give the unsplit volume; a vertex off the curve, or one the curve passes twice, refuses.
/// `docs/issue-46-phase-169.md`.
fn split_cross_bore(offset_mm: f64) -> String {
    let source = include_str!("data/cross-bore.step");
    let data = inventory(source, Limits::default()).unwrap();
    let context = solid_kernel::step_body::BodyContext::read(&data).unwrap();
    let edge = data.entity_reference(79).unwrap();
    let curve = context
        .read_spline_curve_active_domain(&data, edge, solid_kernel::nurbs::Limits::default())
        .unwrap()
        .curve;
    let d = curve.domain();
    let p = curve.evaluate(d[0] + 0.37 * (d[1] - d[0])).unwrap();
    let p = [p[0], p[1], p[2] + offset_mm];
    let mut text = source.replace("#79 = EDGE_CURVE('',#80,#22,#82,.T.);", "#79 = EDGE_CURVE('',#80,#2002,#82,.T.);");
    text = text.replace("#77,#78,", "#77,#2004,#78,").replace("#1075,#1076)", "#1075,#2005,#1076)");
    let added = format!(
        "#2001 = CARTESIAN_POINT('',({:.17E},{:.17E},{:.17E}));\n#2002 = VERTEX_POINT('',#2001);\n\
         #2003 = EDGE_CURVE('',#2002,#22,#82,.T.);\n#2004 = ORIENTED_EDGE('',*,*,#2003,.F.);\n\
         #2005 = ORIENTED_EDGE('',*,*,#2003,.F.);\nENDSEC;\nEND-ISO-10303-21;",
        p[0], p[1], p[2]
    );
    let split = text.replacen("ENDSEC;\nEND-ISO-10303-21;", &added, 1);
    assert_ne!(split, text, "the fixture changed shape");
    split
}

#[test]
fn a_spline_edge_that_uses_part_of_its_curve_reads_and_keeps_the_volume() {
    let whole = sample(include_str!("data/cross-bore.step"), 0.01).unwrap();
    let text = split_cross_bore(0.);
    let data = inventory(&text, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options { chord_mm: 0.01, ..Options::default() }).unwrap();
    let (low, high) = (model.edge_points(79).unwrap(), model.edge_points(2003).unwrap());
    // the pieces meet at the new vertex and keep the unsplit edge's ends
    let original = AnalyticModel::read(&inventory(include_str!("data/cross-bore.step"), Limits::default()).unwrap(), Options::default()).unwrap();
    let ends = original.edge_points(79).unwrap();
    assert_eq!(low.last(), high.first());
    assert_eq!((low.first(), high.last()), (ends.first(), ends.last()));
    let split = sample(&text, 0.01).unwrap();
    let rel = split.signed_volume_mm3 / whole.signed_volume_mm3 - 1.;
    eprintln!("whole {:.6} split {:.6} rel {rel:e}", whole.signed_volume_mm3, split.signed_volume_mm3);
    assert!(rel.abs() < 1e-4, "{rel}");
    // a vertex 0.01 mm off the curve is neither an end nor on it
    let off = split_cross_bore(0.01);
    let err = AnalyticModel::read(&inventory(&off, Limits::default()).unwrap(), Options::default()).unwrap_err();
    assert!(err.detail.contains("neither an end of its curve nor on it"), "{err}");
}
