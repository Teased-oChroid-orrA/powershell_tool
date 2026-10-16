//! The displacement forecast reads whatever describes the geometry: an imported mesh and a STEP solid as well as a sketch.
use fea_problem::diagnose::{diagnose, diagnose_mesh, predicted_displacement, predicted_displacement_on, FailureClass};
use fea_problem::facts::Facts;
use fea_problem::solid::SolidCache;
use fea_problem::{solve, templates, Geometry, Load, MeshSpec, Problem, Support};
use solid_kernel::{SurfaceMesh, V3};

const STEP: &str = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";

fn box_mesh(w: f64, h: f64, d: f64) -> SurfaceMesh {
    let p = |x: f64, y: f64, z: f64| [x * w, y * h, z * d];
    let mut m = SurfaceMesh::default();
    let faces: [([V3; 4], usize); 6] = [
        ([p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0), p(0.0, 1.0, 0.0)], 0),
        ([p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 1.0, 1.0), p(1.0, 0.0, 1.0)], 1),
        ([p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 0.0, 1.0), p(0.0, 0.0, 1.0)], 2),
        ([p(0.0, 1.0, 0.0), p(0.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, 0.0)], 3),
        ([p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 0.0, 0.0)], 4),
        ([p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0), p(1.0, 1.0, 1.0), p(0.0, 1.0, 1.0)], 5),
    ];
    for (q, f) in faces {
        m.push_triangle(q[0], q[1], q[2], f);
        m.push_triangle(q[0], q[2], q[3], f);
    }
    m
}

fn bar() -> Problem {
    let mut p = templates::blank();
    p.name = "Solid bar".into();
    p.set_solid(STEP, SolidCache::new(STEP, box_mesh(1.0, 1.0, 3.0), 0.01));
    p.mesh = MeshSpec { size: 0.5, ..MeshSpec::default() };
    p.supports = vec![Support::fixed("face5")];
    p.loads = vec![Load::Traction { edge: "face6".into(), tx: 0.0, ty: 0.0, tz: 50.0 }];
    p
}

const INP: &str = "*NODE\n1,0,0\n2,1,0\n3,2,0\n4,0,1\n5,1,1\n6,2,1\n*ELEMENT, TYPE=CPS4\n1,1,2,5,4\n2,2,3,6,5\n*NSET, NSET=LEFT\n1,4\n*NSET, NSET=RIGHT\n3,6\n";

fn imported() -> Problem {
    let mut p = templates::templates().into_iter().find(|(n, _)| *n == "Cantilever beam").unwrap().1;
    p.geometry = Geometry::Imported { path: "bar.inp".into(), scale: Default::default() };
    p.thickness = 1.0;
    p.supports = vec![Support::fixed("LEFT")];
    p.loads = vec![Load::Force { edge: "RIGHT".into(), fx: 100.0, fy: 0.0, fz: 0.0 }];
    p
}

#[test]
fn a_step_solids_facts_come_from_its_tessellation_and_the_forecast_matches_the_solve() {
    let p = bar();
    let facts = Facts::of(&p).expect("facts from the cache");
    assert_eq!(facts.extent, [1.0, 1.0, 3.0]);
    let top = &facts.faces["face6"];
    assert!((top.area - 1.0).abs() < 1e-12 && top.normal == [0.0, 0.0, 1.0], "{top:?}");
    assert!((facts.faces["face1"].normal[0] + 1.0).abs() < 1e-12, "the x- face points out of the body");
    let (u, long) = predicted_displacement(&p).expect("a forecast");
    assert_eq!(long, 3.0);
    let real = solve(&p, None).unwrap().summary.max_displacement.value;
    assert!(u / real > 0.5 && u / real < 2.0, "forecast {u:.3e} against the solved {real:.3e}");
    assert!(!diagnose(&p, None).iter().any(|d| d.class == FailureClass::ScaleSuspect));
    // The classic slip: a modulus a million times too small.
    let mut slip = p.clone();
    slip.material.e = 29.0;
    let found: Vec<_> = diagnose(&slip, None).into_iter().filter(|d| d.class == FailureClass::ScaleSuspect && d.summary.starts_with("Before solving")).collect();
    assert!(!found.is_empty(), "the slip is caught before any meshing or solving");
}

#[test]
fn an_imported_meshs_facts_come_from_its_nodes_and_surfaces_and_the_forecast_runs_on_the_mesh() {
    let p = imported();
    let mesh = fea_problem::solve::preview(&p, Some(INP)).unwrap();
    let facts = Facts::from_mesh(&mesh, p.thickness).expect("facts from the mesh");
    assert_eq!(facts.extent, [2.0, 1.0, 1.0], "a plane mesh has the thickness for its third extent");
    let (u, long) = predicted_displacement_on(&p, &facts).expect("a forecast");
    assert_eq!(long, 2.0);
    let real = solve(&p, Some(INP)).unwrap().summary.max_displacement.value;
    assert!(u / real > 0.3 && u / real < 3.0, "forecast {u:.3e} against the solved {real:.3e}");
    // Without a mesh the problem alone has no facts (the file has not been read).
    assert!(predicted_displacement(&p).is_none());
    assert!(diagnose_mesh(&p, &mesh).iter().all(|d| d.class != FailureClass::ScaleSuspect));
    // A soft slip is caught from the mesh, before a solve.
    let mut slip = p.clone();
    slip.material.e = 290.0;
    let found: Vec<_> = diagnose_mesh(&slip, &mesh).into_iter().filter(|d| d.class == FailureClass::ScaleSuspect).collect();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].summary.starts_with("Before solving"));
}

#[test]
fn a_pressure_on_a_meshed_face_uses_the_faces_area_and_normal() {
    let p = bar();
    let mut q = p.clone();
    q.loads = vec![Load::Pressure { edge: "face6".into(), p: 50.0 }];
    // A pressure pushes into the face (-z); the forecast sees the same axial force as the traction of 50 psi on a unit face.
    let (a, _) = predicted_displacement(&p).unwrap();
    let (b, _) = predicted_displacement(&q).unwrap();
    assert!((a / b - 1.0).abs() < 1e-12, "{a} {b}");
    // A face with no known normal (a hole) is not forecast from.
    q.loads = vec![Load::Pressure { edge: "hole1".into(), p: 50.0 }];
    let (c, _) = predicted_displacement(&q).unwrap_or((0.0, 1.0));
    assert_eq!(c, 0.0);
}
