//! Native import of sphere, cone and torus faces (STEP written by OpenCASCADE, see `tests/data/README.md`): the sampled shell is
//! closed, consistently oriented and free of facet intersections, its volume is the closed-form volume to the chord tolerance
//! (an inscribed sampling is a little small, never large), and graded sampling lifts it over the registry's angle gate.
//! `docs/issue-46-phase-101.md`.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::chart_grade::GradeOptions;
use solid_kernel::chart_mesh::ChartQuality;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::plc_conform::min_shell_angle_deg;
use solid_kernel::step::{inventory, Limits};
use std::f64::consts::{PI, TAU};

struct Case {
    name: &'static str,
    source: &'static str,
    exact: f64,
    faces: usize,
}

fn rounded_box(a: f64, b: f64, c: f64, r: f64) -> f64 {
    let (x, y, z) = (a - 2. * r, b - 2. * r, c - 2. * r);
    x * y * z
        + 2. * r * (x * y + x * z + y * z)
        + PI * r * r * (x + y + z)
        + 4. / 3. * PI * r * r * r
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "box with every edge rounded (8 spheres, 12 cylinders)",
            source: include_str!("data/filleted-box.step"),
            exact: rounded_box(2., 3., 4., 0.2),
            faces: 26,
        },
        Case {
            name: "cylinder with a rounded end (torus)",
            source: include_str!("data/cylinder-filleted-top.step"),
            exact: PI * 25. * 10.
                - TAU
                    * (1.5 * 1.5 * (5. - 0.75)
                        - PI * 1.5 * 1.5 / 4. * (5. - 1.5 + 4. * 1.5 / (3. * PI))),
            faces: 4,
        },
        Case {
            name: "cylinder with a chamfered end (cone)",
            source: include_str!("data/cylinder-chamfered-top.step"),
            exact: PI * 25. * 10. - TAU * (1.5 * 1.5 / 2.) * (5. - 0.5),
            faces: 4,
        },
        Case {
            name: "cone with its apex",
            source: include_str!("data/cone.step"),
            exact: PI * 25. * 8. / 3.,
            faces: 2,
        },
        Case {
            name: "hemisphere (seam and pole inside the face)",
            source: include_str!("data/hemisphere.step"),
            exact: 2. / 3. * PI * 64.,
            faces: 2,
        },
        Case {
            name: "capsule (two spherical caps and a cylinder)",
            source: include_str!("data/capsule.step"),
            exact: PI * 9. * 8. + 4. / 3. * PI * 27.,
            faces: 3,
        },
        Case {
            name: "full torus (periodic in both directions)",
            source: include_str!("data/torus.step"),
            exact: 2. * PI * PI * 6. * 4.,
            faces: 1,
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
    .and_then(|m| m.sampled_surface(SurfaceLimits::default(), |_| true))
    .map_err(|e| e.to_string())
}

#[test]
fn sphere_cone_and_torus_shells_have_the_closed_form_volume_to_the_chord_tolerance() {
    for c in cases() {
        let s = sample(c.source, 0.005).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        assert_eq!(s.source_faces.len(), c.faces, "{}", c.name);
        let rel = s.signed_volume_mm3 / c.exact - 1.;
        eprintln!(
            "{:<58} exact {:>9.4} native {:>9.4} ({:+.3} %)",
            c.name,
            c.exact,
            s.signed_volume_mm3,
            100. * rel
        );
        // Inscribed: a little small, never large; every curved patch is within the sagitta bound.
        assert!(rel <= 1e-9 && rel > -4e-3, "{}: {rel}", c.name);
        // Closed and consistently oriented (checked by the sampling); at the default chord also free of facet intersections.
        sample(c.source, 0.05)
            .unwrap_or_else(|e| panic!("{}: {e}", c.name))
            .check_intersections(5_000_000, |_| true)
            .unwrap_or_else(|e| panic!("{}: {e}", c.name));
    }
}

#[test]
fn graded_sampling_lifts_curved_shells_over_the_registry_gate() {
    let gate = fea_core::meshing::compatibility::NATIVE_MIN_SHELL_ANGLE_DEG;
    for c in cases() {
        // The torus is far from the default budget; the others converge.
        let data = inventory(c.source, Limits::default()).unwrap();
        let model = AnalyticModel::read(&data, Options::default()).unwrap();
        let plain = model
            .sampled_surface(SurfaceLimits::default(), |_| true)
            .unwrap();
        let before = min_shell_angle_deg(&plain.mesh.vertices, &plain.mesh.triangles);
        let (graded, report) = model.graded(GradeOptions::default(), 50_000_000).unwrap();
        // a grading that stops at its round limit has a platform-sensitive result (the torus used to: 24.2 against 19.1 degrees
        // on the hosted Windows runner at 12 rounds)
        assert!(report.converged, "{}: {report:?}", c.name);
        let limits = SurfaceLimits {
            chart_quality: Some(ChartQuality {
                min_angle_deg: 25.,
                max_points: 20_000,
            }),
            face_work: 20_000_000,
            total_work: 800_000_000,
            ..SurfaceLimits::default()
        };
        let after = graded.sampled_surface(limits, |_| true).unwrap();
        let angle = min_shell_angle_deg(&after.mesh.vertices, &after.mesh.triangles);
        eprintln!("{:<58} {before:>7.3} -> {angle:>7.3} degrees", c.name);
        assert!(before < gate, "{}: plain {before}", c.name);
        assert!(angle >= gate + 3., "{}: graded {angle}", c.name);
        let v = after.signed_volume_mm3 / plain.signed_volume_mm3 - 1.;
        assert!(v.abs() < 2e-2, "{}: volume changed by {v}", c.name);
    }
}

#[test]
fn a_full_sphere_is_refused_with_a_reason() {
    // A ball is one face bounded by a vertex loop (no edges, a pole): the topology reader refuses it, and so does the chart.
    let error = sample(include_str!("data/ball.step"), 0.05).unwrap_err();
    assert!(
        error.contains("EDGE_LOOP") || error.contains("sphere"),
        "{error}"
    );
}

#[test]
fn graded_curved_shells_reach_radius_edge_two_and_the_steiner_points_stay_on_the_shell() {
    use solid_kernel::plc::{Limits as PlcLimits, ValidatedPlc};
    use solid_kernel::plc_conform::{conform, Options as ConformOptions};
    // The cheap recipe: graded shell, recover only what is missing, then refine to radius-edge 2. The frustum has a 69 degree
    // wedge at its base rim (the protect-everything rule cascades there and stops at the feature floor, see below); the cone
    // has an apex.
    for (name, source) in [
        ("frustum", include_str!("data/frustum.step")),
        ("cone with its apex", include_str!("data/cone.step")),
    ] {
        let data = inventory(source, Limits::default()).unwrap();
        let model = AnalyticModel::read(&data, Options::default()).unwrap();
        let (graded, _) = model.graded(GradeOptions::default(), 50_000_000).unwrap();
        let limits = SurfaceLimits {
            chart_quality: Some(ChartQuality {
                min_angle_deg: 25.,
                max_points: 20_000,
            }),
            face_work: 20_000_000,
            total_work: 800_000_000,
            ..SurfaceLimits::default()
        };
        let checked = graded
            .sampled_surface(limits, |_| true)
            .unwrap()
            .check_intersections(50_000_000, |_| true)
            .unwrap();
        let plc = ValidatedPlc::new(checked, PlcLimits::default(), |_, _| true).unwrap();
        let (vertices, facets) = (plc.vertices().to_vec(), plc.facets().to_vec());
        let options = |protect_encroached| ConformOptions {
            max_steiner: 100_000,
            max_radius_edge: Some(2.),
            protect_encroached,
            // the protected-only refusal below is the point of the second call: no unprotected retry
            unprotected_fallback: false,
            ..Default::default()
        };
        let m = conform(&vertices, &facets, options(false), |_| true)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            m.report.max_radius_edge_ratio <= 2. + 1e-9,
            "{name}: {:?}",
            m.report
        );
        assert_eq!(m.report.unresolved_bad_cells, 0, "{name}");
        assert!(m.report.volume_error_relative < 1e-9, "{name}");
        // No pathological cells: the shortest edge is of the order of the shell's own.
        let shortest = m
            .cells
            .iter()
            .flat_map(|c| (0..4).flat_map(move |i| ((i + 1)..4).map(move |j| (c[i], c[j]))))
            .map(|(i, j)| {
                let (a, b) = (m.points[i], m.points[j]);
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
            })
            .fold(f64::MAX, f64::min);
        assert!(shortest > 1e-3, "{name}: shortest cell edge {shortest:e}");
        // Every vertex of a sub-facet lies on the plane of the input (flat) triangle it came from.
        let diag = {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for v in &vertices {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt()
        };
        let mut worst = 0f64;
        for (tri, parent) in &m.facets {
            let [a, b, c] = facets[*parent].map(|v| vertices[v]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            for v in tri {
                let p = m.points[*v];
                let d =
                    ((p[0] - a[0]) * n[0] + (p[1] - a[1]) * n[1] + (p[2] - a[2]) * n[2]).abs() / nl;
                worst = worst.max(d);
            }
        }
        assert!(worst < 1e-9 * diag, "{name}: a sub-facet vertex is {worst:e} mm off its input facet's plane (diagonal {diag})");
        // Protecting every encroached sub-segment cascades at the frustum's acute rim wedge: refused, not looped.
        if name == "frustum" {
            let e = conform(&vertices, &facets, options(true), |_| true).unwrap_err();
            assert!(e.detail.contains("feature-size floor"), "{e:?}");
        }
    }
}
