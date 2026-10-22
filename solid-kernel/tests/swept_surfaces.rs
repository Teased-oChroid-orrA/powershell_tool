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
        Case {
            name: "blob: a closed periodic spline outline extruded 6 mm",
            source: include_str!("data/blob-prism.step"),
            reference: REF_BLOB,
            faces: 3,
        },
        Case {
            name: "ring: a closed periodic spline section turned about z",
            source: include_str!("data/spline-ring.step"),
            reference: REF_RING,
            faces: 1,
        },
    ]
}
const REF_BLOB: f64 = 487.980501;
const REF_RING: f64 = 650.297534;
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

/// `source` with `dz` added to the z of every `CARTESIAN_POINT` (a pure translation: the placements and the spline controls
/// all hang on those points).
fn lifted(source: &str, dz: f64) -> String {
    source
        .lines()
        .map(|line| {
            let Some(at) = line.find("CARTESIAN_POINT('',(") else {
                return line.to_string();
            };
            let head = at + "CARTESIAN_POINT('',(".len();
            let close = head + line[head..].find(')').unwrap();
            let c: Vec<f64> = line[head..close].split(',').map(|v| v.trim().parse().unwrap()).collect();
            if c.len() != 3 {
                return line.to_string();
            }
            format!("{}{:?},{:?},{:?}{}", &line[..head], c[0], c[1], c[2] + dz, &line[close..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn an_extruded_face_away_from_the_origin_reads_like_the_one_at_the_origin() {
    // `S(t, h) = C(t) + h d`: the height range of the extrusion is relative to the curve, so a profile 25 mm off the origin
    // along the extrusion direction used to put every edge sample 25 mm off the surface (ABC corpus: 20 files refused)
    let c = &cases()[1];
    let at_origin = sample(c.source, 0.01).unwrap().signed_volume_mm3;
    let lifted = sample(&lifted(c.source, 25.), 0.01).unwrap_or_else(|e| panic!("{e}")).signed_volume_mm3;
    assert!((lifted / at_origin - 1.).abs() < 1e-4, "{lifted} vs {at_origin}");
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

#[test]
fn a_revolved_band_with_two_closed_circles_acquires_a_meridian_seam() {
    // Re-author the repo's vase as the same closed solid with two one-edge bounds on its revolution face.
    let source = include_str!("data/vase.step")
        .replace("#17 = ADVANCED_FACE('',(#18),#31,.T.);", "#17 = ADVANCED_FACE('',(#18,#130),#31,.T.);")
        .replace("#19 = EDGE_LOOP('',(#20,#61,#88,#115));", "#19 = EDGE_LOOP('',(#20));\n#130 = FACE_BOUND('',#131,.T.);\n#131 = EDGE_LOOP('',(#132));\n#132 = ORIENTED_EDGE('',*,*,#89,.F.);");
    let s = sample(&source, 0.01).unwrap();
    assert_eq!(s.source_faces.len(), 3);
    assert!((s.signed_volume_mm3 / REF_VASE - 1.).abs() < 3e-3);
    let reversed_bounds = source.replace("(#18,#130),#31", "(#130,#18),#31");
    let reversed = sample(&reversed_bounds, 0.01).unwrap();
    assert!((reversed.signed_volume_mm3 / REF_VASE - 1.).abs() < 3e-3);
}

#[test]
fn an_interior_trim_of_a_revolution_profile_acquires_only_its_bounded_meridian() {
    let mut source = include_str!("data/vase.step")
        .replace("#17 = ADVANCED_FACE('',(#18),#31,.T.);", "#17 = ADVANCED_FACE('',(#18,#130),#31,.T.);")
        .replace("#19 = EDGE_LOOP('',(#20,#61,#88,#115));", "#19 = EDGE_LOOP('',(#20));\n#130 = FACE_BOUND('',#131,.T.);\n#131 = EDGE_LOOP('',(#132));\n#132 = ORIENTED_EDGE('',*,*,#89,.F.);");
    let start = source.find("#32 = B_SPLINE_CURVE_WITH_KNOTS(").unwrap();
    let end = start + source[start..].find(';').unwrap() + 1;
    source.replace_range(start..end, "#32 = B_SPLINE_CURVE_WITH_KNOTS('',3,(#133,#134,#135,#33,#34,#35,#36,#37,#38,#39,#136,#137,#138),.UNSPECIFIED.,.F.,.F.,(4,3,1,1,1,3,4),(-3.,0.,5.385164807135,11.21611670198,16.601281509114,21.626219319675,24.626219319675),.UNSPECIFIED.);");
    source = source.replace("#129 = PRODUCT_RELATED_PRODUCT_CATEGORY", "#133 = CARTESIAN_POINT('',(4.,0.,-3.));\n#134 = CARTESIAN_POINT('',(4.,0.,-2.));\n#135 = CARTESIAN_POINT('',(4.,0.,-1.));\n#136 = CARTESIAN_POINT('',(4.5,0.,21.));\n#137 = CARTESIAN_POINT('',(4.5,0.,22.));\n#138 = CARTESIAN_POINT('',(4.5,0.,23.));\n#129 = PRODUCT_RELATED_PRODUCT_CATEGORY");
    let s = sample(&source, 0.01).unwrap();
    assert_eq!(s.source_faces.len(), 3);
    assert!((s.signed_volume_mm3 / REF_VASE - 1.).abs() < 3e-3);
}

#[cfg(feature = "occt")]
#[test]
// OpenCASCADE's tessellation volume depends on the platform's floating point at about 5e-6 (the revolved vase: 1503.0205 on macOS arm64,
// 1503.0272 on the hosted Linux runner), so the recorded value is checked to 1e-4, far below the 3e-3 the native import is held to.
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
            (live / c.reference - 1.).abs() < 1e-4,
            "{}: recorded {} but OpenCASCADE says {live}",
            c.name,
            c.reference
        );
    }
}
