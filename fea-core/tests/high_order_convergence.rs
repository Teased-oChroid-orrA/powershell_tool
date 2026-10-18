//! Convergence and cost of the signed-distance mesher's first- and second-order tetrahedra on a curved body with an exact
//! solution (#55): a quarter of a thick cylinder under internal pressure, plane strain, the Lame solution. Tet10 midside nodes
//! are on the exact surface (`mesh_solid` snaps them); the study reports the observed rate against the mesh size and against
//! the unknowns, and the assembly / factor / solve cost. `docs/issue-46-phase-84.md`.

use fea_core::tetmesh::{mesh_solid, FaceHit, Solid, TetOptions};
use fea_core::*;

const E: f64 = 200e3;
const NU: f64 = 0.3;
const A: f64 = 1.0;
const B: f64 = 2.0;
const P: f64 = 100.0;
const LEN: f64 = 0.5;

struct Row {
    h: f64,
    dofs: usize,
    tets: usize,
    max_err: f64,
    energy_err: f64,
    assemble_ms: f64,
    factor_ms: f64,
    solve_ms: f64,
    factor_nnz: usize,
}

fn run(h: f64, quadratic: bool) -> Row {
    let sdf = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        (r - B).max(A - r).max(-x[0]).max(-x[1]).max((x[2] - LEN / 2.0).abs() - LEN / 2.0)
    };
    let face = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        let d = [(r - A).abs(), (r - B).abs(), x[0].abs(), x[1].abs(), x[2].abs(), (x[2] - LEN).abs()];
        (0..6).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap()
    };
    let faces = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]).max(1e-12);
        let radial = [x[0] / r, x[1] / r, 0.0];
        vec![
            FaceHit { face: 0, dist: A - r, normal: [-radial[0], -radial[1], 0.0] },
            FaceHit { face: 1, dist: r - B, normal: radial },
            FaceHit { face: 2, dist: -x[0], normal: [-1.0, 0.0, 0.0] },
            FaceHit { face: 3, dist: -x[1], normal: [0.0, -1.0, 0.0] },
            FaceHit { face: 4, dist: -x[2], normal: [0.0, 0.0, -1.0] },
            FaceHit { face: 5, dist: x[2] - LEN, normal: [0.0, 0.0, 1.0] },
        ]
    };
    let size = |_: [f64; 3]| 10.0;
    let solid = Solid { lo: [0.0, 0.0, 0.0], hi: [2.05, 2.05, LEN], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&faces) };
    let (mesh, rep) = mesh_solid(&solid, Elastic::new(E, NU), TetOptions { base: 0.5, boundary_size: h, quadratic, ..Default::default() }).unwrap();
    eprintln!("  snap report h {h}: snapped {} refused {} worst quality {:.3}", rep.snapped, rep.refused, rep.worst_quality);
    let inner = mesh.surfaces.get("face1").cloned().expect("inner wall");
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0].abs() < 1e-6 {
            bc.fix(n, 0, 0.0);
        }
        if x[1].abs() < 1e-6 {
            bc.fix(n, 1, 0.0);
        }
        if x[2].abs() < 1e-6 || (x[2] - LEN).abs() < 1e-6 {
            bc.fix(n, 2, 0.0);
        }
    }
    let mut loads = Loads::default();
    for f in inner {
        loads.faces.push((f, SurfaceLoad::Pressure(P)));
    }
    let mut best: Option<Solution> = None;
    for _ in 0..3 {
        let s = model.solve_static(&loads, &bc).unwrap();
        let t = |s: &Solution| s.timings.assemble_ms + s.timings.factor_ms + s.timings.solve_ms;
        if best.as_ref().is_none_or(|b| t(&s) < t(b)) {
            best = Some(s);
        }
    }
    let sol = best.unwrap();
    let outside = model.mesh.nodes.iter().filter(|x| { let r = x[0].hypot(x[1]); !(A - 1e-6..=B + 1e-6).contains(&r) }).count();
    let deepest = model.mesh.nodes.iter().map(|x| { let r = x[0].hypot(x[1]); (A - r).max(r - B) }).fold(0.0f64, f64::max);
    eprintln!("  {} h {h}: {outside} nodes outside the body, deepest {deepest:.3e}", if quadratic { "Tet10" } else { "Tet4" });
    let (aa, bb) = (P * A * A / (B * B - A * A), P * A * A * B * B / (B * B - A * A));
    let ur = |r: f64| (1.0 + NU) / E * ((1.0 - 2.0 * NU) * aa * r + bb / r);
    let (mut max_err, mut num, mut den) = (0.0f64, 0.0, 0.0);
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let r = x[0].hypot(x[1]);
        // The exact solution exists only inside the body: nodes the snapping left outside it are counted, not scored.
        if !(A - 1e-6..=B + 1e-6).contains(&r) {
            continue;
        }
        let u = (sol.u[n * 3] * x[0] + sol.u[n * 3 + 1] * x[1]) / r;
        max_err = max_err.max((u - ur(r)).abs() / ur(A));
        num += (u - ur(r)).powi(2);
        den += ur(r).powi(2);
    }
    Row {
        h,
        dofs: model.mesh.n_dofs(),
        tets: rep.tets,
        max_err,
        energy_err: (num / den).sqrt(),
        assemble_ms: sol.timings.assemble_ms,
        factor_ms: sol.timings.factor_ms,
        solve_ms: sol.timings.solve_ms,
        factor_nnz: sol.factor_nnz,
    }
}

/// Observed rate: slope of log(error) against log(x) between the first and last row.
fn rate(rows: &[Row], err: impl Fn(&Row) -> f64, x: impl Fn(&Row) -> f64) -> f64 {
    let (a, b) = (&rows[0], &rows[rows.len() - 1]);
    (err(b) / err(a)).ln() / (x(b) / x(a)).ln()
}

/// Both families on the same meshes; the second-order element has the lower rms radial error at every size.
fn compare(hs: &[f64]) {
    let t4: Vec<Row> = hs.iter().map(|&h| run(h, false)).collect();
    let t10: Vec<Row> = hs.iter().map(|&h| run(h, true)).collect();
    for (name, rows) in [("Tet4", &t4), ("Tet10", &t10)] {
        for r in rows {
            eprintln!(
                "{name:<5} h {:.2}: {:>6} tets {:>7} dofs  max radial err {:.3e}  rms err {:.3e}  assemble {:.1} factor {:.1} solve {:.1} ms  factor nnz {}",
                r.h, r.tets, r.dofs, r.max_err, r.energy_err, r.assemble_ms, r.factor_ms, r.solve_ms, r.factor_nnz
            );
        }
        eprintln!(
            "{name}: observed rate in h {:.2}, in dofs^(-1/3) (3D unknowns) {:.2}",
            rate(rows, |r| r.max_err, |r| r.h),
            rate(rows, |r| r.max_err, |r| (r.dofs as f64).powf(-1.0 / 3.0))
        );
    }
    // The second-order element has the lower rms radial error at every size (nodes inside the body). The maximum nodal error is
    // reported but not asserted: it is erratic for Tet10 (Phase 84).
    for (a, b) in t4.iter().zip(&t10) {
        assert!(b.energy_err < a.energy_err, "h {}: Tet10 {} vs Tet4 {}", a.h, b.energy_err, a.energy_err);
    }
}

#[test]
fn second_order_tetrahedra_have_the_lower_error_on_a_curved_body_at_two_sizes() {
    compare(&[0.4, 0.28]);
}

#[test]
#[ignore = "study: run with --release --ignored --nocapture"]
fn convergence_and_cost_over_four_sizes() {
    compare(&[0.4, 0.28, 0.2, 0.14]);
}

#[test]
#[ignore = "diagnostic"]
fn where_is_the_largest_tet10_error() {
    // Re-run the finest Tet10 case and report the worst node, its distance to the symmetry planes and whether it was constrained.
    let h = 0.2;
    let sdf = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        (r - B).max(A - r).max(-x[0]).max(-x[1]).max((x[2] - LEN / 2.0).abs() - LEN / 2.0)
    };
    let face = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        let d = [(r - A).abs(), (r - B).abs(), x[0].abs(), x[1].abs(), x[2].abs(), (x[2] - LEN).abs()];
        (0..6).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap()
    };
    let faces = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]).max(1e-12);
        let radial = [x[0] / r, x[1] / r, 0.0];
        vec![
            FaceHit { face: 0, dist: A - r, normal: [-radial[0], -radial[1], 0.0] },
            FaceHit { face: 1, dist: r - B, normal: radial },
            FaceHit { face: 2, dist: -x[0], normal: [-1.0, 0.0, 0.0] },
            FaceHit { face: 3, dist: -x[1], normal: [0.0, -1.0, 0.0] },
            FaceHit { face: 4, dist: -x[2], normal: [0.0, 0.0, -1.0] },
            FaceHit { face: 5, dist: x[2] - LEN, normal: [0.0, 0.0, 1.0] },
        ]
    };
    let size = |_: [f64; 3]| 10.0;
    let solid = Solid { lo: [0.0, 0.0, 0.0], hi: [2.05, 2.05, LEN], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&faces) };
    let (mesh, _) = mesh_solid(&solid, Elastic::new(E, NU), TetOptions { base: 0.5, boundary_size: h, quadratic: true, ..Default::default() }).unwrap();
    let mut off_plane = 0;
    for x in &mesh.nodes {
        for (c, lim) in [(0usize, 0.0), (1, 0.0), (2, 0.0), (2, LEN)] {
            let d = (x[c] - lim).abs();
            if d > 1e-12 && d < 1e-6 {
                off_plane += 1;
            }
        }
    }
    let near = |c: usize, lim: f64, tol: f64| mesh.nodes.iter().filter(|x| (x[c] - lim).abs() < tol).count();
    eprintln!("nodes {}; within 1e-6 of a symmetry plane but not on it: {off_plane}; within 1e-3: x=0 {}, y=0 {}, z=0 {}, z=L {}; within 1e-6: {} {} {} {}", mesh.nodes.len(), near(0, 0.0, 1e-3), near(1, 0.0, 1e-3), near(2, 0.0, 1e-3), near(2, LEN, 1e-3), near(0, 0.0, 1e-6), near(1, 0.0, 1e-6), near(2, 0.0, 1e-6), near(2, LEN, 1e-6));
    let model = Model::new(mesh.clone()).unwrap();
    let inner = mesh.surfaces.get("face1").cloned().unwrap();
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0].abs() < 1e-6 {
            bc.fix(n, 0, 0.0);
        }
        if x[1].abs() < 1e-6 {
            bc.fix(n, 1, 0.0);
        }
        if x[2].abs() < 1e-6 || (x[2] - LEN).abs() < 1e-6 {
            bc.fix(n, 2, 0.0);
        }
    }
    let mut loads = Loads::default();
    for f in inner {
        loads.faces.push((f, SurfaceLoad::Pressure(P)));
    }
    let sol = model.solve_static(&loads, &bc).unwrap();
    let (aa, bb) = (P * A * A / (B * B - A * A), P * A * A * B * B / (B * B - A * A));
    let ur = |r: f64| (1.0 + NU) / E * ((1.0 - 2.0 * NU) * aa * r + bb / r);
    let mut errs: Vec<(f64, usize)> = model.mesh.nodes.iter().enumerate().filter(|(_, x)| { let r = x[0].hypot(x[1]); (A - 1e-6..=B + 1e-6).contains(&r) }).map(|(n, x)| {
        let r = x[0].hypot(x[1]);
        let u = (sol.u[n * 3] * x[0] + sol.u[n * 3 + 1] * x[1]) / r;
        ((u - ur(r)).abs() / ur(A), n)
    }).collect();
    errs.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (e, n) in errs.iter().take(6) {
        let x = model.mesh.nodes[*n];
        eprintln!("err {e:.3e} node {n} at ({:.4}, {:.4}, {:.4}) r {:.4} u=({:.3e},{:.3e},{:.3e})", x[0], x[1], x[2], x[0].hypot(x[1]), sol.u[n * 3], sol.u[n * 3 + 1], sol.u[n * 3 + 2]);
    }
    let mut used = vec![false; mesh.nodes.len()];
    for b in &mesh.blocks {
        for &n in &b.conn {
            used[n] = true;
        }
    }
    eprintln!("unused (orphan) nodes: {} of {}", used.iter().filter(|u| !**u).count(), mesh.nodes.len());
    let mut off: Vec<(f64, [f64; 3])> = model.mesh.nodes.iter().map(|x| { let r = x[0].hypot(x[1]); ((A - r).max(r - B).max(-x[0]).max(-x[1]), *x) }).filter(|(d, _)| *d > 1e-6).collect();
    off.sort_by(|a, b| b.0.total_cmp(&a.0));
    eprintln!("{} nodes outside the body; the deepest:", off.len());
    for (d, x) in off.iter().take(12) {
        eprintln!("  depth {d:.3e} at ({:.4}, {:.4}, {:.4}) r {:.4} angle {:.1} deg", x[0], x[1], x[2], x[0].hypot(x[1]), x[1].atan2(x[0]).to_degrees());
    }
    let median = errs[errs.len() / 2].0;
    eprintln!("median node error {median:.3e}, 99th percentile {:.3e}", errs[errs.len() / 100].0);
}
