//! Faces on a B-spline curve turned about an axis (`SURFACE_OF_REVOLUTION`) or moved along a direction
//! (`SURFACE_OF_LINEAR_EXTRUSION`), written by OpenCASCADE (`tests/data/README.md`). The native import charts them by their
//! parameter rectangle like the B-spline surfaces of Phase 105 and must produce a closed shell whose volume matches
//! OpenCASCADE's tessellation of the same file. `docs/issue-46-phase-106.md`.

use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cylindrical_step::ChartPolicy;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};

struct Case {
    name: &'static str,
    source: &'static str,
    /// OpenCASCADE tessellation volume at 0.001 mm deflection.
    reference: f64,
    faces: usize,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "vase: a spline profile turned about the z axis",
            source: include_str!("data/vase.step"),
            reference: REF_VASE,
            faces: 3,
        },
        Case {
            name: "prism: a spline outline extruded 8 mm",
            source: include_str!("data/wavy-prism.step"),
            reference: REF_PRISM,
            faces: 4,
        },
    ]
}
const REF_VASE: f64 = 1503.020472;
const REF_PRISM: f64 = 440.809884;

fn sample(source: &str, chord_mm: f64) -> Result<SampledSurface, String> {
    let data = inventory(source, Limits::default()).map_err(|e| e.to_string())?;
    AnalyticModel::read(
        &data,
        Options {
            chord_mm,
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
fn revolved_and_extruded_spline_faces_import_with_the_reference_volume() {
    for c in cases() {
        let s = sample(c.source, 0.01).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        eprintln!("{:<52} native {:.6}", c.name, s.signed_volume_mm3);
        assert_eq!(s.source_faces.len(), c.faces, "{}", c.name);
        let tagged: std::collections::BTreeSet<_> = s.mesh.face.iter().collect();
        assert_eq!(tagged.len(), c.faces, "{}: faces without triangles", c.name);
        let rel = s.signed_volume_mm3 / c.reference - 1.;
        assert!(rel.abs() < 3e-3, "{}: {rel}", c.name);
    }
}

#[test]
fn the_revolved_face_follows_its_seam_and_the_shell_is_sampled_finer_on_request() {
    // a finer chord shrinks the inscribed-volume deficit: the native volume converges to the reference from below
    let c = &cases()[0];
    let coarse = sample(c.source, 0.05).unwrap().signed_volume_mm3;
    let fine = sample(c.source, 0.005).unwrap().signed_volume_mm3;
    assert!(
        (fine - c.reference).abs() < (coarse - c.reference).abs(),
        "coarse {coarse}, fine {fine}, reference {}",
        c.reference
    );
}

#[cfg(feature = "occt")]
#[test]
fn the_recorded_reference_volumes_are_opencascade_s() {
    for c in cases() {
        let live = solid_kernel::occt::Brep::from_step_text(c.source)
            .unwrap()
            .solid(0.001)
            .unwrap()
            .volume();
        eprintln!(
            "{:<52} OpenCASCADE {live:.6} (recorded {:.6})",
            c.name, c.reference
        );
        assert!(
            (live / c.reference - 1.).abs() < 1e-6,
            "{}: recorded {} but OpenCASCADE says {live}",
            c.name,
            c.reference
        );
    }
}
