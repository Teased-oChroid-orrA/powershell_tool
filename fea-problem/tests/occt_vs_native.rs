//! The owner's OCCT-retirement criterion, measured (`docs/issue-46-phase-124.md`): the same STEP text, the same element-size
//! target, meshed and solved through the OpenCascade tessellation and through the native one, then compared.
//!
//! Both paths end in the same volume mesher (`solid_kernel::mesh_volume`); they differ in the face-tagged surface mesh it is
//! given (`cad::Brep::tessellate` against `native_solid::tessellate_step_native`). The table prints with `--nocapture`.
//! Needs the `occt` feature. Run: `cargo test -p fea-problem --features occt --release --test occt_vs_native -- --nocapture`.
#![cfg(feature = "occt")]

use fea_core::meshing::adjacency::Adjacency;
use fea_core::meshing::quality::QualityThresholds;
use fea_problem::native_solid::tessellate_step_native;
use fea_problem::solid::{flat_face, solid_mesh, SolidCache};
use fea_problem::{solve, Geometry, Load, Problem, Support};
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::occt::{Bool, Brep};
use std::time::Instant;

/// One mesh and one benchmark solve.
#[derive(Debug, Clone, Default)]
struct Run {
    err: Option<String>,
    tets: usize,
    nodes: usize,
    dofs: usize,
    min_sj: f64,
    min_radius_ratio: f64,
    max_edge_ratio: f64,
    size_jump: f64,
    gates: bool,
    mesh_s: f64,
    solve_ms: f64,
    max_disp: f64,
    energy: f64,
    reaction_z: f64,
    equilibrium: f64,
    solved: bool,
    /// Elements below the solver gate's scaled Jacobian (0.1).
    below_gate: usize,
    /// ... of which touch the boundary (a corner node on a named surface).
    below_gate_on_boundary: usize,
}

struct Case {
    name: String,
    text: String,
    /// Element-size multiplier on the import's default (a few thin models need a coarser start to stay under the element cap).
    h_scale: f64,
}

fn from_occt(name: &str, b: &Brep) -> Case {
    Case { name: name.into(), text: b.to_step_text().unwrap(), h_scale: 1.0 }
}
fn from_native(name: &str, b: &NativeBrep) -> Case {
    Case { name: name.into(), text: b.to_step_text(), h_scale: 1.0 }
}
fn from_file(name: &str, text: &str, h_scale: f64) -> Case {
    Case { name: name.into(), text: text.into(), h_scale }
}

fn cases() -> Vec<Case> {
    let plate = Brep::box_from_corners([0., 0., 0.], [12., 4., 1.]);
    let hole = |x: f64| Brep::cylinder([x, 2., -1.], 0.8, [0., 0., 1.], 4.);
    let nbox = NativeBrep::box_from_corners([0.; 3], [10., 10., 4.]).unwrap();
    vec![
        from_occt("box (OCCT-built)", &Brep::box_from_corners([0., 0., 0.], [20., 30., 40.])),
        from_occt("cylinder (OCCT-built)", &Brep::cylinder([0., 0., 0.], 10., [0., 0., 1.], 30.)),
        from_occt("plate, two holes (OCCT)", &plate.boolean(Bool::Cut, &hole(3.)).unwrap().boolean(Bool::Cut, &hole(9.)).unwrap()),
        from_occt("plate with a boss (OCCT)", &plate.boolean(Bool::Union, &Brep::cylinder([6., 2., 1.], 1.2, [0., 0., 1.], 3.)).unwrap()),
        from_occt("box, all edges filleted (OCCT)", &Brep::box_from_corners([0., 0., 0.], [12., 14., 16.]).fillet_all(2.).unwrap()),
        from_native("box (native)", &NativeBrep::box_from_corners([0.; 3], [20., 30., 40.]).unwrap()),
        from_native("cylinder (native)", &NativeBrep::cylinder_z([0.; 3], 10., 30.).unwrap()),
        from_native("box + hole (native)", &nbox.drill(5, [3., 6., 4.], 1., None).unwrap()),
        from_native("box + boss (native)", &nbox.boss(5, [5., 5., 4.], 2., 3.).unwrap()),
        from_native("box + boss + hole (native)", &nbox.boss(5, [5., 5., 4.], 2., 3.).unwrap().drill(7, [5., 5., 7.], 0.5, None).unwrap()),
        from_native("box, all edges filleted (native)", &NativeBrep::box_from_corners([0.; 3], [12., 14., 16.]).unwrap().fillet_all_convex(2.).unwrap()),
        from_file("cylinder, filleted top", include_str!("../../solid-kernel/tests/data/cylinder-filleted-top.step"), 1.0),
        from_file("edit plate (40x30x10, hole)", include_str!("../../solid-kernel/tests/data/edit-plate.step"), 1.0),
        from_file("cross-bore (R8, R3)", include_str!("../../solid-kernel/tests/data/cross-bore.step"), 1.0),
        from_file("cross-bore, filleted", include_str!("../../solid-kernel/tests/data/cross-bore-filleted.step"), 1.0),
        from_file("t-junction", include_str!("../../solid-kernel/tests/data/t-junction.step"), 1.0),
        from_file("tilted hole", include_str!("../../solid-kernel/tests/data/tilted-hole.step"), 1.0),
        from_file("vase (spline revolution)", include_str!("../../solid-kernel/tests/data/vase.step"), 1.0),
        from_file("NEMA17 motor bracket", include_str!("../../nema17-motor-bracket.step"), 1.0),
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

/// Mesh (and, with `solve`, solve the benchmark on) the solid `step` through `cache` at element size `text_h`.
fn measure(base: &Problem, cache: SolidCache, step: &str, text_h: f64, solve_it: bool) -> Run {
    let mut r = Run::default();
    let mut p = base.clone();
    p.set_solid(step, cache);
    p.mesh.size = text_h;
    let Some((bottom, top)) = (match &p.geometry { Geometry::Solid { cache: Some(c), .. } => z_face(c, -1.0).zip(z_face(c, 1.0)), _ => None }) else {
        r.err = Some("no flat +z / -z face pair for the benchmark".into());
        return r;
    };
    p.supports = vec![Support::fixed(&format!("face{bottom}"))];
    p.loads = vec![Load::Traction { edge: format!("face{top}"), tx: 0.0, ty: 0.0, tz: 10.0 }];
    let t = Instant::now();
    let (mesh, rep) = match solid_mesh(&p) {
        Ok(v) => v,
        Err(e) => {
            r.err = Some(e);
            return r;
        }
    };
    r.mesh_s = t.elapsed().as_secs_f64();
    r.tets = rep.tets;
    r.nodes = mesh.nodes.len();
    r.dofs = mesh.n_dofs();
    let q = mesh.quality();
    let adj = Adjacency::new(&mesh);
    let q = q.with_progression(&adj);
    r.min_sj = q.min_scaled_jacobian;
    r.max_edge_ratio = q.max_edge_ratio;
    r.size_jump = q.max_size_jump;
    r.min_radius_ratio = q.elements.iter().map(|e| e.radius_ratio).fold(f64::INFINITY, f64::min);
    r.gates = q.acceptable(&QualityThresholds::default());
    r.below_gate = q.elements.iter().filter(|e| e.scaled_jacobian < QualityThresholds::default().min_scaled_jacobian).count();
    let on_surface: std::collections::HashSet<usize> = mesh.surfaces.values().flatten().flat_map(|f| f.iter().take(3).copied()).collect();
    let mut k = 0;
    for blk in &mesh.blocks {
        for e in 0..blk.n_elems() {
            if q.elements[k].scaled_jacobian < QualityThresholds::default().min_scaled_jacobian && blk.elem(e).iter().take(4).any(|n| on_surface.contains(n)) {
                r.below_gate_on_boundary += 1;
            }
            k += 1;
        }
    }
    if !solve_it {
        // meshing evidence only (the solve dominates the run time of the table)
        return r;
    }
    match solve(&p, None) {
        Ok(s) => {
            r.solved = true;
            r.solve_ms = s.summary.solve_ms;
            r.max_disp = s.summary.max_displacement.value;
            r.energy = s.summary.strain_energy;
            r.reaction_z = s.summary.reaction[2];
            r.equilibrium = s.summary.equilibrium_error;
        }
        Err(e) => r.err = Some(format!("solve: {e}")),
    }
    r
}

fn compare(c: &Case) -> (Run, Run) {
    let base = Problem::from_step(&c.name, &c.text).expect("OpenCascade import");
    let h = base.mesh.size * c.h_scale;
    let Geometry::Solid { cache: Some(occt_cache), .. } = &base.geometry else { panic!() };
    let occt_cache = occt_cache.clone();
    let native_cache = tessellate_step_native(&c.text, occt_cache.deflection);
    // NO_SOLVE=1 skips the solves
    let solve_it = std::env::var_os("NO_SOLVE").is_none();
    let o = measure(&base, occt_cache, &c.text, h, solve_it);
    let n = match native_cache {
        Ok(cache) => measure(&base, cache, &c.text, h, solve_it),
        Err(e) => Run { err: Some(format!("native tessellation: {e}")), ..Run::default() },
    };
    (o, n)
}

fn row(name: &str, which: &str, r: &Run) -> String {
    match &r.err {
        Some(e) if r.tets == 0 => format!("{name:<34} {which:<6} REFUSED: {e}"),
        _ => format!(
            "{name:<34} {which:<6} tets {:>6} dofs {:>7} sJ {:.3} rr {:.3} er {:>5.1} jump {:>4.1} gates {} (<0.1: {}, on boundary {}) mesh {:>6.2}s solve {:>7.1}ms | umax {:.5e} U {:.5e} Rz {:>9.3} eq {:.1e}{}",
            r.tets, r.dofs, r.min_sj, r.min_radius_ratio, r.max_edge_ratio, r.size_jump, if r.gates { "ok" } else { "NO" }, r.below_gate, r.below_gate_on_boundary, r.mesh_s, r.solve_ms, r.max_disp, r.energy, r.reaction_z, r.equilibrium,
            r.err.as_ref().map(|e| format!(" [{e}]")).unwrap_or_default()
        ),
    }
}

/// Print the table for every case (no assertions): the evidence of the document.
#[test]
#[ignore = "meshes and solves 19 models twice; run in release with --nocapture"]
fn table() {
    let only = std::env::var("CASE").unwrap_or_default();
    // SCALE multiplies every element size (1.6 is the size of the assertion test); NO_SOLVE=1 skips the solve.
    let scale: f64 = std::env::var("SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    for c in cases().into_iter().filter(|c| c.name.contains(&only)) {
        let c = Case { h_scale: c.h_scale * scale, ..c };
        let (o, n) = compare(&c);
        eprintln!("{}\n{}", row(&c.name, "OCCT", &o), row(&c.name, "native", &n));
    }
}

/// The comparison with assertions, at a size 1.6 times coarser than the table (about a quarter of the elements) so the solves stay
/// short: the native tessellation must mesh and solve whatever the OpenCascade one does, with quality, size and the benchmark solve
/// within stated factors of OpenCascade's.
#[test]
#[ignore = "meshes and solves a dozen models twice; run in release"]
fn the_native_path_matches_the_opencascade_path_on_quality_size_and_the_benchmark_solve() {
    let subset = ["box (OCCT", "cylinder (OCCT", "plate, two holes", "plate with a boss", "box, all edges filleted (OCCT", "box + hole", "box + boss + hole", "box, all edges filleted (native", "cylinder, filleted top", "edit plate", "cross-bore (R8", "vase"];
    for c in cases().into_iter().filter(|c| subset.iter().any(|s| c.name.contains(s))) {
        let c = Case { h_scale: c.h_scale * 1.6, ..c };
        let (o, n) = compare(&c);
        eprintln!("{}\n{}", row(&c.name, "OCCT", &o), row(&c.name, "native", &n));
        let who = &c.name;
        let rel = |a: f64, b: f64| (a / b - 1.0).abs();
        assert!(o.solved, "{who}: the OpenCascade path itself must mesh and solve this benchmark: {:?}", o.err);
        // Criterion 1: the native mesh is usable and the solve converges (force balance).
        assert!(n.solved, "{who}: native: {:?}", n.err);
        assert!(n.equilibrium < 1e-6 && n.min_sj > 0.0, "{who}: equilibrium {} sJ {}", n.equilibrium, n.min_sj);
        // The solver's gates: wherever the OpenCascade tessellation passes them the native one does.
        assert!(n.gates || !o.gates, "{who}: native fails the quality gates that OpenCascade passes");
        // Criterion 2: quality no worse than a stated fraction of OpenCascade's, and a benchmark that agrees to the discretisation error.
        assert!(n.min_sj >= 0.5 * o.min_sj, "{who}: min scaled Jacobian {} vs {}", n.min_sj, o.min_sj);
        assert!(n.min_radius_ratio >= 0.4 * o.min_radius_ratio, "{who}: radius ratio {} vs {}", n.min_radius_ratio, o.min_radius_ratio);
        // The worst edge ratio is a single-element extremum (cross-bore: 17 native, 5 OpenCascade at the coarse size, 27 / 30 at the
        // table's size): it must stay well inside the solver's gate of 50, and the size jump within 1.6 times.
        assert!(n.max_edge_ratio <= (2.0 * o.max_edge_ratio).max(25.0) && n.size_jump <= 1.6 * o.size_jump, "{who}: edge ratio {} vs {}, jump {} vs {}", n.max_edge_ratio, o.max_edge_ratio, n.size_jump, o.size_jump);
        // The share of elements under the gate's scaled Jacobian (all of them boundary-layer elements, see the document).
        let share = |r: &Run| r.below_gate as f64 / r.tets as f64;
        assert!(share(&n) <= 2.5 * share(&o) + 0.015, "{who}: sub-gate share {:.4} vs {:.4}", share(&n), share(&o));
        // Compliance, strain energy per squared load (U / Rz^2): independent of how much boundary area the mesh gives the loaded face.
        let compliance = |r: &Run| r.energy / (r.reaction_z * r.reaction_z);
        assert!(rel(compliance(&n), compliance(&o)) < 0.05, "{who}: compliance {} vs {}", compliance(&n), compliance(&o));
        // A load face that meets a tangent fillet owns a strip of boundary triangles along its rim, about sqrt(2 r 0.1 h) wide (a
        // vertex within a tenth of the size of the plane counts as on it) and different between tessellations: the loaded area
        // is 16 % (OpenCascade) / 26 % (native) above the exact 80 mm2 on the 12 x 14 x 16 box with 2 mm fillets at a 1 mm element.
        // The total load and the local maximum move with it, so those two are compared elsewhere only (Phase 124, limits).
        if !who.contains("all edges filleted") {
            assert!(rel(n.energy, o.energy) < 0.06, "{who}: strain energy {} vs {}", n.energy, o.energy);
            assert!(rel(n.reaction_z, o.reaction_z) < 0.04, "{who}: reaction {} vs {}", n.reaction_z, o.reaction_z);
            assert!(rel(n.max_disp, o.max_disp) < 0.15, "{who}: max displacement {} vs {}", n.max_disp, o.max_disp);
        } else {
            assert!(rel(n.reaction_z, o.reaction_z) < 0.15, "{who}: reaction {} vs {}", n.reaction_z, o.reaction_z);
        }
        // Criterion 3: element and degree-of-freedom counts within 15 %, times within a factor 3 (plus a second of slack for noise).
        assert!(rel(n.tets as f64, o.tets as f64) < 0.15 && rel(n.dofs as f64, o.dofs as f64) < 0.15, "{who}: tets {} vs {}", n.tets, o.tets);
        assert!(n.mesh_s <= 3.0 * o.mesh_s + 1.0, "{who}: meshing {:.2} s vs {:.2} s", n.mesh_s, o.mesh_s);
        assert!(n.solve_ms <= 3.0 * o.solve_ms + 1000.0, "{who}: solve {:.0} ms vs {:.0} ms", n.solve_ms, o.solve_ms);
    }
}

/// The boundary repair returns the best mesh it reached, not the last: on the t-junction through the OpenCascade tessellation its
/// rounds pass through 7 elements under the gate and used to end at 8 (`docs/issue-46-phase-142.md`).
#[test]
#[ignore = "meshes the t-junction (about 25 s in release)"]
fn the_t_junction_repair_keeps_its_best_round() {
    let c = cases().into_iter().find(|c| c.name == "t-junction").unwrap();
    let base = Problem::from_step(&c.name, &c.text).expect("OpenCascade import");
    let Geometry::Solid { cache: Some(oc), .. } = &base.geometry else { panic!() };
    let r = measure(&base, oc.clone(), &c.text, base.mesh.size * c.h_scale, false);
    assert!(r.err.is_none(), "{:?}", r.err);
    assert!(r.below_gate <= 7, "{} elements under the gate", r.below_gate);
}

/// Quality of the face-tagged surface triangulation each path hands to the volume mesher: triangle count, smallest angle, share of
/// triangles under 15 degrees, shortest and longest edge. Run with `--nocapture`; it locates the source of the mesh differences.
#[test]
#[ignore = "diagnostic table"]
fn surface_triangulation_quality() {
    let only = std::env::var("CASE").unwrap_or_default();
    for c in cases().into_iter().filter(|c| c.name.contains(&only)) {
        let base = Problem::from_step(&c.name, &c.text).expect("OpenCascade import");
        let Geometry::Solid { cache: Some(oc), .. } = &base.geometry else { panic!() };
        let nc = tessellate_step_native(&c.text, oc.deflection).expect("native tessellation");
        for (which, cache) in [("OCCT", oc), ("native", &nc)] {
            let (mut min_angle, mut bad, mut emin, mut emax) = (180f64, 0usize, f64::INFINITY, 0f64);
            for t in &cache.triangles {
                let v = t.map(|i| cache.vertices[i]);
                let len = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                let l = [len(v[0], v[1]), len(v[1], v[2]), len(v[2], v[0])];
                let angle = |a: f64, b: f64, c: f64| ((b * b + c * c - a * a) / (2.0 * b * c)).clamp(-1.0, 1.0).acos().to_degrees();
                let m = angle(l[0], l[1], l[2]).min(angle(l[1], l[0], l[2])).min(angle(l[2], l[0], l[1]));
                min_angle = min_angle.min(m);
                bad += (m < 15.0) as usize;
                emin = emin.min(l[0].min(l[1]).min(l[2]));
                emax = emax.max(l[0].max(l[1]).max(l[2]));
            }
            eprintln!("{:<34} {:<6} triangles {:>6} min angle {:>5.2} deg, under 15 deg {:>5.1} %, edges {:.4} .. {:.3}", c.name, which, cache.triangles.len(), min_angle, 100.0 * bad as f64 / cache.triangles.len() as f64, emin, emax);
        }
    }
}

/// Per-face breakdown of the native surface triangulation of one model (`CASE` picks it): triangle count, shortest edge and the
/// share of needle triangles (smallest angle under 5 degrees) of every face, next to the OpenCascade count.
#[test]
#[ignore = "diagnostic table"]
fn native_surface_triangulation_by_face() {
    let only = std::env::var("CASE").unwrap_or_default();
    for c in cases().into_iter().filter(|c| c.name.contains(&only)) {
        let base = Problem::from_step(&c.name, &c.text).expect("OpenCascade import");
        let Geometry::Solid { cache: Some(oc), .. } = &base.geometry else { panic!() };
        let nc = tessellate_step_native(&c.text, oc.deflection).expect("native tessellation");
        let by_face = |cache: &SolidCache| {
            let n = cache.face.iter().copied().max().map_or(0, |m| m + 1);
            let mut out = vec![(0usize, f64::INFINITY, 0usize); n];
            for (t, f) in cache.triangles.iter().zip(&cache.face) {
                let v = t.map(|i| cache.vertices[i]);
                let len = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                let l = [len(v[0], v[1]), len(v[1], v[2]), len(v[2], v[0])];
                let angle = |a: f64, b: f64, c: f64| ((b * b + c * c - a * a) / (2.0 * b * c)).clamp(-1.0, 1.0).acos().to_degrees();
                let m = angle(l[0], l[1], l[2]).min(angle(l[1], l[0], l[2])).min(angle(l[2], l[0], l[1]));
                let e = &mut out[*f];
                e.0 += 1;
                e.1 = e.1.min(l[0].min(l[1]).min(l[2]));
                e.2 += (m < 5.0) as usize;
            }
            out
        };
        let (o, n) = (by_face(oc), by_face(&nc));
        eprintln!("{}: {} faces OpenCascade, {} native", c.name, o.len(), n.len());
        for (i, (a, b)) in o.iter().zip(&n).enumerate() {
            eprintln!("  face{:<3} OCCT {:>6} tris (min edge {:>8.4}, {:>5} under 5 deg) | native {:>6} tris (min edge {:>8.4}, {:>5} under 5 deg)", i + 1, a.0, a.1, a.2, b.0, b.1, b.2);
        }
    }
}

/// Native edge sampling of one model at its import chord (`CASE` picks it): points per B-rep edge, their length and spacing.
#[test]
#[ignore = "diagnostic table"]
fn native_edge_sampling() {
    use solid_kernel::analytic_step::{AnalyticModel, Options};
    use solid_kernel::step::{inventory, Limits};
    let only = std::env::var("CASE").unwrap_or_default();
    for c in cases().into_iter().filter(|c| c.name.contains(&only)) {
        let base = Problem::from_step(&c.name, &c.text).expect("OpenCascade import");
        let Geometry::Solid { cache: Some(oc), .. } = &base.geometry else { panic!() };
        let data = inventory(&c.text, Limits::default()).unwrap();
        let model = AnalyticModel::read(&data, Options { chord_mm: oc.deflection, ..Options::default() }).unwrap();
        eprintln!("{} (chord {:.4} mm)", c.name, oc.deflection);
        for id in model.body().edges.keys() {
            let pts = model.edge_points(*id).unwrap_or(&[]);
            let d = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            let steps: Vec<f64> = pts.windows(2).map(|w| d(w[0], w[1])).collect();
            let length: f64 = steps.iter().sum();
            eprintln!("  edge {id}: {} points, length {:.3}, spacing {:.4} .. {:.4}", pts.len(), length, steps.iter().cloned().fold(f64::INFINITY, f64::min), steps.iter().cloned().fold(0.0, f64::max));
        }
    }
}
