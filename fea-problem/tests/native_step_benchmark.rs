//! The built-in native rows of the same-STEP benchmark (`occt_vs_native.rs`) on the feature-off path: the STEP text of every model
//! that needs no OpenCascade to build (the `NativeBrep` shapes and the committed STEP files) imported by `Problem::from_step`, which
//! without the `occt` feature reads, sizes and tessellates it natively (`native_solid::import_step_native`), then meshed through
//! `solid_mesh` and solved on the same benchmark (bottom face fixed, 10 MPa on the top face). This is the application's path when
//! the feature is off, so it runs on any host, Windows included (`docs/issue-46-windows-runtime-plan.md`, `docs/issue-46-phase-167.md`).
//! The chord and element size come from the native import, not OpenCascade's, so the numbers are not the `occt_vs_native` table's.
//! Run: `cargo test -p fea-problem --no-default-features --release --test native_step_benchmark -- --ignored --nocapture`.
#![cfg(not(feature = "occt"))]

use fea_core::meshing::adjacency::Adjacency;
use fea_core::meshing::quality::QualityThresholds;
use fea_problem::solid::{flat_face, solid_mesh, SolidCache};
use fea_problem::{solve, Geometry, Load, Problem, Support};
use solid_kernel::native_brep::NativeBrep;
use std::time::Instant;

/// The models: the native-built rows and the STEP-file rows of `occt_vs_native::cases`, in its order.
fn cases() -> Vec<(&'static str, String)> {
    let nbox = NativeBrep::box_from_corners([0.; 3], [10., 10., 4.]).unwrap();
    vec![
        ("box (native)", NativeBrep::box_from_corners([0.; 3], [20., 30., 40.]).unwrap().to_step_text()),
        ("cylinder (native)", NativeBrep::cylinder_z([0.; 3], 10., 30.).unwrap().to_step_text()),
        ("box + hole (native)", nbox.drill(5, [3., 6., 4.], 1., None).unwrap().to_step_text()),
        ("box + boss (native)", nbox.boss(5, [5., 5., 4.], 2., 3.).unwrap().to_step_text()),
        ("box + boss + hole (native)", nbox.boss(5, [5., 5., 4.], 2., 3.).unwrap().drill(7, [5., 5., 7.], 0.5, None).unwrap().to_step_text()),
        ("box, all edges filleted (native)", NativeBrep::box_from_corners([0.; 3], [12., 14., 16.]).unwrap().fillet_all_convex(2.).unwrap().to_step_text()),
        ("cylinder, filleted top", include_str!("../../solid-kernel/tests/data/cylinder-filleted-top.step").into()),
        ("edit plate (40x30x10, hole)", include_str!("../../solid-kernel/tests/data/edit-plate.step").into()),
        ("cross-bore (R8, R3)", include_str!("../../solid-kernel/tests/data/cross-bore.step").into()),
        ("cross-bore, filleted", include_str!("../../solid-kernel/tests/data/cross-bore-filleted.step").into()),
        ("t-junction", include_str!("../../solid-kernel/tests/data/t-junction.step").into()),
        ("tilted hole", include_str!("../../solid-kernel/tests/data/tilted-hole.step").into()),
        ("vase (spline revolution)", include_str!("../../solid-kernel/tests/data/vase.step").into()),
        ("NEMA17 motor bracket", include_str!("../../nema17-motor-bracket.step").into()),
    ]
}

/// The flat face with outward normal `+-z` that lies furthest along it (the top for `+1`, the bottom for `-1`).
fn z_face(c: &SolidCache, sign: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for f in 1..=c.face_names().len() {
        if let Ok((centre, n)) = flat_face(c, f) {
            if n[2] * sign > 0.999 && best.is_none_or(|(_, z)| centre[2] * sign > z * sign) {
                best = Some((f, centre[2]));
            }
        }
    }
    best.map(|(f, _)| f)
}

/// Import, mesh and solve one model; `Err` names the stage that refused. The row is the benchmark table's.
fn measure(name: &str, step: &str) -> Result<(String, bool), String> {
    let t = Instant::now();
    let mut p = Problem::from_step(name, step).map_err(|e| format!("native import: {e}"))?;
    let import_s = t.elapsed().as_secs_f64();
    let Geometry::Solid { cache: Some(c), .. } = &p.geometry else { return Err("the import is not a cached solid".into()) };
    let (Some(bottom), Some(top)) = (z_face(c, -1.0), z_face(c, 1.0)) else { return Err("no flat +z / -z face pair for the benchmark".into()) };
    p.supports = vec![Support::fixed(&format!("face{bottom}"))];
    p.loads = vec![Load::Traction { edge: format!("face{top}"), tx: 0.0, ty: 0.0, tz: 10.0 }];
    let t = Instant::now();
    let (mesh, rep) = solid_mesh(&p).map_err(|e| format!("mesh: {e}"))?;
    let mesh_s = t.elapsed().as_secs_f64();
    let q = mesh.quality().with_progression(&Adjacency::new(&mesh));
    let gate = QualityThresholds::default();
    let gates = q.acceptable(&gate);
    let below = q.elements.iter().filter(|e| e.scaled_jacobian < gate.min_scaled_jacobian).count();
    let min_rr = q.elements.iter().map(|e| e.radius_ratio).fold(f64::INFINITY, f64::min);
    let s = solve(&p, None).map_err(|e| format!("solve: {e}"))?;
    let sum = &s.summary;
    if !(sum.equilibrium_error < 1e-6 && q.min_scaled_jacobian > 0.0) {
        return Err(format!("solve not usable: equilibrium {:.1e}, min scaled Jacobian {}", sum.equilibrium_error, q.min_scaled_jacobian));
    }
    let row = format!(
        "{name:<34} h {:.4} chord {:.5} tets {:>6} dofs {:>7} sJ {:.3} rr {:.3} er {:>5.1} jump {:>4.1} gates {} (<0.1: {below}) import {import_s:>5.2}s mesh {mesh_s:>6.2}s solve {:>7.1}ms | umax {:.5e} U {:.5e} Rz {:>9.3} eq {:.1e}",
        p.mesh.size, c.deflection, rep.tets, mesh.n_dofs(), q.min_scaled_jacobian, min_rr, q.max_edge_ratio, q.max_size_jump,
        if gates { "ok" } else { "NO" }, sum.solve_ms, sum.max_displacement.value, sum.strain_energy, sum.reaction[2], sum.equilibrium_error
    );
    Ok((row, gates))
}

/// Every built-in native row imports, meshes (volume within 5 %, named faces resolved: `solid_mesh`'s own refusals) and solves
/// in force balance. The default solver-quality gate is printed per row, not asserted: rows that fail it are evidence for the
/// phase report (NEMA17 is below it on every host so far, `docs/issue-46-phase-155.md`).
#[test]
#[ignore = "imports, meshes and solves 14 models (minutes in release); run with --nocapture"]
fn native_rows() {
    let only = std::env::var("CASE").unwrap_or_default();
    let (mut models, mut gates, mut refused) = (0, 0, Vec::new());
    for (name, text) in cases().into_iter().filter(|(n, _)| n.contains(&only)) {
        models += 1;
        match measure(name, &text) {
            Ok((row, ok)) => {
                gates += usize::from(ok);
                eprintln!("{row}");
            }
            Err(e) => {
                eprintln!("{name:<34} REFUSED: {e}");
                refused.push(format!("{name}: {e}"));
            }
        }
    }
    eprintln!("summary: {models} models; native solved {}, default quality gate {gates}", models - refused.len());
    assert!(models > 0, "CASE={only} selects no model");
    assert!(refused.is_empty(), "native rows refused: {refused:#?}");
}
