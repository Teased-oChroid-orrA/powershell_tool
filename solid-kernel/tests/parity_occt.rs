//! Parity of the native STEP pipeline with OpenCASCADE on shapes OpenCASCADE builds (#63 groundwork, `occt` feature): for each
//! shape, the exact volume, OpenCASCADE's tessellated volume, and what the native pipeline (analytic import, sampled closed
//! surface) does with the exported STEP: its volume, or the refusal. The matrix is pinned, so a change in what is supported is
//! a visible decision. `docs/issue-46-phase-96.md`.
#![cfg(feature = "occt")]

use solid_kernel::native_surface::Limits as SurfaceLimits;
use solid_kernel::occt::{Bool, Brep, Loop, Profile};
use solid_kernel::step::{inventory, Limits};
use solid_kernel::analytic_step::{AnalyticModel, Options};
use std::f64::consts::PI;

struct Case {
    name: &'static str,
    shape: Brep,
    exact: f64,
    /// Whether the native pipeline is expected to import and sample it.
    native: bool,
}

/// Volume of a box with every edge and corner rounded by `r` (the Minkowski sum of the shrunk box and a ball).
fn rounded_box(a: f64, b: f64, c: f64, r: f64) -> f64 {
    let (x, y, z) = (a - 2. * r, b - 2. * r, c - 2. * r);
    x * y * z + 2. * r * (x * y + x * z + y * z) + PI * r * r * (x + y + z) + 4. / 3. * PI * r * r * r
}

fn cases() -> Vec<Case> {
    let plate = Brep::box_from_corners([0., 0., 0.], [12., 4., 0.25]);
    let hole = |x: f64| Brep::cylinder([x, 2., -1.], 0.5, [0., 0., 1.], 2.);
    let l_profile = Profile { outer: Loop::Polygon(vec![[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]]), holes: vec![] };
    vec![
        Case { name: "box", shape: Brep::box_from_corners([0., 0., 0.], [2., 3., 4.]), exact: 24., native: true },
        Case { name: "cylinder", shape: Brep::cylinder([0., 0., 0.], 2., [0., 0., 1.], 3.), exact: PI * 4. * 3., native: true },
        Case { name: "extruded L profile", shape: Brep::extrude(&l_profile, 0., 2.).unwrap(), exact: (4. + 2.) * 2., native: true },
        Case { name: "plate with a through hole", shape: plate.boolean(Bool::Cut, &hole(6.)).unwrap(), exact: (48. - PI * 0.25) * 0.25, native: true },
        Case { name: "plate with two holes", shape: plate.boolean(Bool::Cut, &hole(3.)).unwrap().boolean(Bool::Cut, &hole(9.)).unwrap(), exact: (48. - 2. * PI * 0.25) * 0.25, native: true },
        Case { name: "plate with a boss", shape: plate.boolean(Bool::Union, &Brep::cylinder([3., 2., 0.25], 0.8, [0., 0., 1.], 1.75)).unwrap(), exact: 12. * 4. * 0.25 + PI * 0.64 * 1.75, native: true },
        Case { name: "plate with a pocket", shape: plate.boolean(Bool::Cut, &Brep::box_from_corners([4., 1., 0.1], [6., 3., 1.])).unwrap(), exact: 12. * 4. * 0.25 - 2. * 2. * 0.15, native: true },
        Case { name: "union of two boxes (L)", shape: Brep::box_from_corners([0., 0., 0.], [4., 1., 1.]).boolean(Bool::Union, &Brep::box_from_corners([0., 0., 0.], [1., 3., 1.])).unwrap(), exact: 4. + 3. - 1., native: true },
        Case { name: "box with every edge filleted", shape: Brep::box_from_corners([0., 0., 0.], [2., 3., 4.]).fillet_all(0.2).unwrap(), exact: rounded_box(2., 3., 4., 0.2), native: false },
    ]
}

#[test]
fn the_native_pipeline_against_opencascade_on_opencascade_built_shapes() {
    for c in cases() {
        let occt = c.shape.solid(0.005).unwrap();
        let text = c.shape.to_step_text().unwrap();
        let import = |chord_mm: f64| {
            inventory(&text, Limits::default())
                .map_err(|e| e.to_string())
                .and_then(|data| AnalyticModel::read(&data, Options { chord_mm, ..Options::default() }).map_err(|e| e.to_string()))
                .and_then(|m| m.sampled_surface(SurfaceLimits::default(), |_| true).map_err(|e| e.to_string()))
        };
        let default_chord = import(Options::default().chord_mm);
        // The sampling chord is the only accuracy knob: 0.005 mm brings the curved shapes to OpenCASCADE's own tessellation.
        let native = import(0.005);
        if let (Ok(coarse), true) = (&default_chord, c.name.contains("cylinder")) {
            eprintln!("  default chord {} mm: native {:.4} ({:+.3} % of exact)", Options::default().chord_mm, coarse.signed_volume_mm3, 100. * (coarse.signed_volume_mm3 / c.exact - 1.));
        }
        match &native {
            Ok(s) => eprintln!("{:<30} exact {:>9.4} | OpenCASCADE tessellation {:>9.4} | native {:>9.4} ({:+.3} % of exact)", c.name, c.exact, occt.volume(), s.signed_volume_mm3, 100. * (s.signed_volume_mm3 / c.exact - 1.)),
            Err(e) => eprintln!("{:<30} exact {:>9.4} | OpenCASCADE tessellation {:>9.4} | native refused: {e}", c.name, c.exact, occt.volume()),
        }
        assert_eq!(native.is_ok(), c.native, "{}: {:?}", c.name, native.as_ref().err());
        if let Ok(s) = native {
            // Planar shapes are exact; curved ones agree to the chord tolerance (a sampled circle is inscribed: volume low).
            let curved = c.name.contains("cylinder") || c.name.contains("hole") || c.name.contains("boss");
            let tol = if curved { 5e-3 } else { 1e-9 };
            assert!((s.signed_volume_mm3 / c.exact - 1.).abs() < tol, "{}: native {} vs exact {}", c.name, s.signed_volume_mm3, c.exact);
            // An inscribed sampling of a solid cylinder or boss is a little small, of a hole a little too large.
            if curved {
                let hole = c.name.contains("hole");
                assert!(if hole { s.signed_volume_mm3 >= c.exact * (1. - 1e-9) } else { s.signed_volume_mm3 <= c.exact * (1. + 1e-9) }, "{}: {} against {}", c.name, s.signed_volume_mm3, c.exact);
            }
        }
        assert!((occt.volume() / c.exact - 1.).abs() < if c.name.contains("filleted") { 2e-3 } else { 2e-3 }, "{}: OpenCASCADE tessellation {} vs exact {}", c.name, occt.volume(), c.exact);
    }
}
