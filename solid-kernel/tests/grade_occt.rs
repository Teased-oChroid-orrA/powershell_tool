//! Graded sampling on shapes OpenCASCADE builds (feature `occt`): the plain native shell has a small input angle (the
//! registry gate is 18 degrees), the graded one clears it, and only the graded one lets the conforming mesher reach a
//! radius-edge ratio of 2. `docs/issue-46-phase-100.md`.
#![cfg(feature = "occt")]

use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::chart_grade::GradeOptions;
use solid_kernel::chart_mesh::ChartQuality;
use solid_kernel::native_surface::Limits as SurfaceLimits;
use solid_kernel::occt::{Bool, Brep};
use solid_kernel::plc::{Limits as PlcLimits, ValidatedPlc};
use solid_kernel::plc_conform::{conform, min_shell_angle_deg, Options as ConformOptions};
use solid_kernel::step::{inventory, Limits};

const GATE: f64 = fea_core::meshing::compatibility::NATIVE_MIN_SHELL_ANGLE_DEG;

fn shapes() -> Vec<(&'static str, Brep)> {
    let block = Brep::box_from_corners([0., 0., 0.], [40., 30., 20.]);
    let plate = Brep::box_from_corners([0., 0., 0.], [12., 4., 0.25]);
    let pin = |x: f64| Brep::cylinder([x, 2., -1.], 0.5, [0., 0., 1.], 2.);
    vec![
        (
            "block with a through hole",
            block
                .boolean(
                    Bool::Cut,
                    &Brep::cylinder([20., 15., -1.], 6., [0., 0., 1.], 22.),
                )
                .unwrap(),
        ),
        (
            "block with a pocket",
            block
                .boolean(
                    Bool::Cut,
                    &Brep::box_from_corners([10., 5., 12.], [30., 25., 25.]),
                )
                .unwrap(),
        ),
        (
            "thin plate with two holes",
            plate
                .boolean(Bool::Cut, &pin(3.))
                .unwrap()
                .boolean(Bool::Cut, &pin(9.))
                .unwrap(),
        ),
    ]
}

fn sample(text: &str, graded: bool) -> solid_kernel::native_surface::SampledSurface {
    let data = inventory(text, Limits::default()).unwrap();
    let mut model = AnalyticModel::read(&data, Options::default()).unwrap();
    let mut limits = SurfaceLimits::default();
    if graded {
        model = model.graded(GradeOptions::default(), 50_000_000).unwrap().0;
        limits.chart_quality = Some(ChartQuality {
            min_angle_deg: 25.,
            max_points: 20_000,
        });
        limits.face_work = 20_000_000;
        limits.total_work = 800_000_000;
    }
    model.sampled_surface(limits, |_| true).unwrap()
}

#[test]
fn grading_lifts_opencascade_built_shells_over_the_registry_gate() {
    for (name, shape) in shapes() {
        let text = shape.to_step_text().unwrap();
        let (plain, graded) = (sample(&text, false), sample(&text, true));
        let angle = |s: &solid_kernel::native_surface::SampledSurface| {
            min_shell_angle_deg(&s.mesh.vertices, &s.mesh.triangles)
        };
        let (a0, a1) = (angle(&plain), angle(&graded));
        eprintln!("{name:<28} plain {a0:>7.3} degrees ({} triangles) | graded {a1:>7.3} degrees ({} triangles)", plain.mesh.triangles.len(), graded.mesh.triangles.len());
        assert!(
            a0 < GATE,
            "{name}: the plain shell is expected below the gate, got {a0}"
        );
        assert!(a1 >= GATE + 6., "{name}: graded {a1}");
        // The same body to the chord tolerance (an inscribed curved face is a little small, a hole a little large).
        let rel = (graded.signed_volume_mm3 / plain.signed_volume_mm3 - 1.).abs();
        assert!(
            rel < 2e-3,
            "{name}: volume {} -> {}",
            plain.signed_volume_mm3,
            graded.signed_volume_mm3
        );
    }
}

#[test]
fn only_the_graded_shell_reaches_radius_edge_two_in_the_conforming_mesher() {
    let (name, shape) = shapes().remove(1);
    let text = shape.to_step_text().unwrap();
    let mesh = |graded: bool| {
        let checked = sample(&text, graded)
            .check_intersections(50_000_000, |_| true)
            .unwrap();
        let plc = ValidatedPlc::new(checked, PlcLimits::default(), |_, _| true).unwrap();
        conform(
            plc.vertices(),
            plc.facets(),
            ConformOptions {
                max_steiner: 100_000,
                max_radius_edge: Some(2.),
                ..Default::default()
            },
            |_| true,
        )
    };
    let graded = mesh(true).unwrap_or_else(|e| panic!("{name}: graded shell refused: {e}"));
    assert!(
        graded.report.max_radius_edge_ratio <= 2. + 1e-9,
        "{:?}",
        graded.report
    );
    assert_eq!(graded.report.unresolved_bad_cells, 0);
    assert!(graded.report.volume_error_relative < 1e-9);
    // The plain shell of the same part: refused, or not brought down to 2.
    match mesh(false) {
        Err(e) => eprintln!("{name}: plain shell refused: {e}"),
        Ok(m) => assert!(
            m.report.max_radius_edge_ratio > 2. || m.report.unresolved_bad_cells > 0,
            "{:?}",
            m.report
        ),
    }
}
