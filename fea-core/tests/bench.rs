//! Speed baseline of the kernel (ignored: timings, not assertions).
//!
//! `cargo test -p fea-core --release --test bench -- --ignored --nocapture`

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::kernel::{geometry, stiffness, stiffness_fast, Work};
use fea_core::linear::{Ordering, Reduced};
use fea_core::*;
use std::time::{Duration, Instant};

const MAT: Elastic = Elastic { e: 10.3e6, nu: 0.33, alpha: 0.0, aniso: None };

fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut out = None;
    let mut min = Duration::MAX;
    for _ in 0..n {
        let t = Instant::now();
        let v = f();
        min = min.min(t.elapsed());
        out = Some(v);
    }
    (min, out.unwrap())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[test]
#[ignore]
fn element_kernel_throughput() {
    let mut work = Work::new();
    for (kind, physics) in [
        (ElementKind::Quad4, Physics::PlaneStress { thickness: 1.0 }),
        (ElementKind::Quad9, Physics::PlaneStress { thickness: 1.0 }),
        (ElementKind::Tri6, Physics::PlaneStress { thickness: 1.0 }),
        (ElementKind::Hex8, Physics::Solid),
        (ElementKind::Hex20, Physics::Solid),
        (ElementKind::Hex27, Physics::Solid),
        (ElementKind::Tet10, Physics::Solid),
    ] {
        let d = kind.dim();
        let nn = kind.n_nodes();
        let xyz: Vec<[f64; 3]> = kind.node_coords().iter().map(|x| [x[0] + 0.05 * x[1], x[1] + 0.03 * x[0], if d == 3 { x[2] + 0.02 * x[0] } else { 0.0 }]).collect();
        let mut ke = vec![0.0; 27 * 27 * 10];
        let reps = if d == 2 { 20_000 } else { 2_000 };
        let (t_old, _) = best(5, || {
            let mut acc = 0.0;
            for _ in 0..reps {
                stiffness(kind, physics, &MAT, std::hint::black_box(&xyz), &mut work, &mut ke).unwrap();
                acc += ke[0];
            }
            acc
        });
        let (t_new, _) = best(5, || {
            let mut acc = 0.0;
            for _ in 0..reps {
                stiffness_fast(kind, physics, &MAT, std::hint::black_box(&xyz), &mut work, &mut ke).unwrap();
                acc += ke[0];
            }
            acc
        });
        let (t_geo, _) = best(5, || {
            for _ in 0..reps {
                geometry(kind, physics, std::hint::black_box(&xyz), &mut work).unwrap();
            }
        });
        let per = |t: Duration| t.as_secs_f64() * 1e6 / reps as f64;
        eprintln!("BENCH   geometry only {kind:?}: {:.2} us", per(t_geo));
        eprintln!("BENCH element stiffness {kind:?} ({nn} nodes): node-pair {:.2} us, component-major {:.2} us ({:.2}x)", per(t_old), per(t_new), per(t_old) / per(t_new));
    }
}

fn pipeline(label: &str, kind: ElementKind, physics: Physics, div: [usize; 3]) -> (Model, f64) {
    let t = Instant::now();
    let mesh = grid(physics, kind, MAT, div, &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let setup = ms(t.elapsed());
    let d = kind.dim();
    let mut bc = model.dirichlet();
    for (axis, name) in ["u0", "v0", "w0"].iter().enumerate().take(d) {
        for &n in model.mesh.node_set(name).unwrap() {
            bc.fix(n, axis, 0.0);
        }
    }
    let top = if d == 2 { "v1" } else { "w1" };
    let mut loads = Loads::default();
    for &n in model.mesh.node_set(top).unwrap() {
        loads.nodal.push((n, [0.0, 0.0, 0.0]));
        loads.nodal.last_mut().unwrap().1[d - 1] = 1.0;
    }
    let (t_asm, k) = best(3, || model.assemble().unwrap());
    let (t_sym, red) = best(3, || Reduced::new(&model.pattern, &bc).unwrap());
    let (t_fac, fac) = best(3, || red.factor(&k).unwrap());
    let f = fea_core::loads::assemble(&model.mesh, &loads).unwrap();
    let (t_sol, _) = best(3, || fac.solve(&k, &f, &bc));
    eprintln!(
        "BENCH {label:<22} nodes {:>7} dofs {:>7} nnzL {:>9} colors {:>3} | setup {setup:>8.1} assemble {:>8.2} symbolic {:>8.2} factor {:>8.2} solve {:>7.2} ms",
        model.mesh.nodes.len(),
        model.mesh.n_dofs(),
        red.factor_nnz(),
        model.pattern.n_colors(),
        ms(t_asm),
        ms(t_sym),
        ms(t_fac),
        ms(t_sol)
    );
    (model, ms(t_fac))
}

#[test]
#[ignore]
fn pipeline_2d_and_3d() {
    for n in [20, 50, 100] {
        pipeline(&format!("Quad9 {n}x{n}"), ElementKind::Quad9, Physics::PlaneStress { thickness: 1.0 }, [n, n, 1]);
    }
    for n in [40, 100] {
        pipeline(&format!("Quad4 {n}x{n}"), ElementKind::Quad4, Physics::PlaneStress { thickness: 1.0 }, [n, n, 1]);
    }
    for n in [6, 10, 14] {
        pipeline(&format!("Hex8 {n}^3"), ElementKind::Hex8, Physics::Solid, [n * 2, n * 2, n * 2]);
        pipeline(&format!("Hex20 {n}^3"), ElementKind::Hex20, Physics::Solid, [n, n, n]);
        pipeline(&format!("Hex27 {n}^3"), ElementKind::Hex27, Physics::Solid, [n, n, n]);
    }
    for n in [6, 10] {
        pipeline(&format!("Tet10 {n}^3"), ElementKind::Tet10, Physics::Solid, [n, n, n]);
    }
}

/// AMD against geometric nested dissection (same matrix, same constraints).
#[test]
#[ignore]
fn ordering_comparison() {
    let cases: Vec<(String, ElementKind, Physics, [usize; 3])> = vec![
        ("Quad9 50x50".into(), ElementKind::Quad9, Physics::PlaneStress { thickness: 1.0 }, [50, 50, 1]),
        ("Quad9 100x100".into(), ElementKind::Quad9, Physics::PlaneStress { thickness: 1.0 }, [100, 100, 1]),
        ("Hex8 20^3".into(), ElementKind::Hex8, Physics::Solid, [20, 20, 20]),
        ("Hex20 10^3".into(), ElementKind::Hex20, Physics::Solid, [10, 10, 10]),
        ("Hex27 10^3".into(), ElementKind::Hex27, Physics::Solid, [10, 10, 10]),
        ("Hex8 28^3".into(), ElementKind::Hex8, Physics::Solid, [28, 28, 28]),
        ("Tet10 10^3".into(), ElementKind::Tet10, Physics::Solid, [10, 10, 10]),
    ];
    for (label, kind, physics, div) in cases {
        let mesh = grid(physics, kind, MAT, div, &|p| p).unwrap();
        let model = Model::new(mesh).unwrap();
        let d = kind.dim();
        let mut bc = model.dirichlet();
        for (axis, name) in ["u0", "v0", "w0"].iter().enumerate().take(d) {
            for &n in model.mesh.node_set(name).unwrap() {
                bc.fix(n, axis, 0.0);
            }
        }
        let k = model.assemble().unwrap();
        let mut line = format!("BENCH ordering {label:<14} dofs {:>7}", model.mesh.n_dofs());
        for ord in [Ordering::Amd, Ordering::NestedDissection] {
            let t = Instant::now();
            let red = Reduced::with_ordering(&model.pattern, &bc, Some(&model.mesh.nodes), ord).unwrap();
            let sym = ms(t.elapsed());
            let (tf, _) = best(2, || red.factor(&k).unwrap());
            line += &format!(" | {ord:?}: symbolic {sym:>7.1} nnz(L) {:>10} factor {:>9.1} ms", red.factor_nnz(), ms(tf));
        }
        eprintln!("{line}");
    }
}

/// Direct (faer Cholesky) against conjugate gradients with smoothed-aggregation multigrid on a
/// clamped Hex8 block: the break-even size and the speed-up beyond it.
#[test]
#[ignore]
fn iterative_against_direct_on_large_hex_models() {
    for (kind, n) in [(ElementKind::Hex8, 10usize), (ElementKind::Hex8, 14), (ElementKind::Hex8, 18), (ElementKind::Hex8, 24), (ElementKind::Hex8, 32), (ElementKind::Hex8, 40), (ElementKind::Hex20, 6), (ElementKind::Hex20, 10), (ElementKind::Hex20, 14), (ElementKind::Hex20, 20), (ElementKind::Tet10, 8), (ElementKind::Tet10, 14)] {
        let mut mesh = fea_core::generate::grid(Physics::Solid, kind, Elastic::new(10.0e6, 0.3), [n, n, n], &|p| [p[0], 0.8 * p[1], 0.6 * p[2]]).unwrap();
        mesh.select_nodes("base", |x| x[2] < 1e-9);
        let top: Vec<Vec<usize>> = mesh.select_faces("top", |c| c[2] > 0.6 - 1e-9).to_vec();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &nd in model.mesh.node_set("base").unwrap() {
            bc.fix_node(nd);
        }
        let loads = Loads { faces: top.into_iter().map(|f| (f, SurfaceLoad::Traction([100.0, 50.0, -300.0]))).collect(), ..Loads::default() };
        let t = Instant::now();
        let it = model.solve_static_with(&loads, &bc, SolveMethod::iterative()).unwrap();
        let t_it = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let direct = model.solve_static_with(&loads, &bc, SolveMethod::Direct).unwrap();
        let t_d = t.elapsed().as_secs_f64() * 1e3;
        let scale = direct.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let diff = it.u.iter().zip(&direct.u).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / scale;
        eprintln!("BENCH iter-vs-direct {kind:?} {n}^3 ({} dofs, factor nnz {}): PCG+AMG {t_it:.0} ms ({} its, setup {:.0} + solve {:.0}); direct {t_d:.0} ms (symbolic {:.0} + factor {:.0}); max rel diff {diff:.1e}", it.n_free, direct.factor_nnz, it.iterations.unwrap(), it.timings.symbolic_ms, it.timings.solve_ms, direct.timings.symbolic_ms, direct.timings.factor_ms);
        assert!(diff < 1e-8);
    }
}

/// Unstructured mesher throughput: triangles per second for a plate with a hole at several sizes,
/// and the whole pipeline (mesh -> Tri6 -> assemble -> solve).
#[test]
#[ignore]
fn unstructured_mesher_throughput() {
    use fea_core::delaunay::{triangulate, MeshOptions};
    use fea_core::geometry::{Loop, Region};
    let region = Region::new(Loop::rectangle(0.0, 0.0, 10.0, 10.0).unwrap(), vec![Loop::circle([5.0, 5.0], 1.5, "hole").unwrap()], Elastic::new(10.0e6, 0.3)).unwrap();
    for h in [0.2, 0.1, 0.05, 0.025] {
        let t = Instant::now();
        let m = triangulate(&region, &|_| h, MeshOptions::default()).unwrap();
        let t_mesh = t.elapsed().as_secs_f64();
        eprintln!("h = {h}: {} triangles in {:.0} ms ({:.0} k tri/s), min angle {:.1} deg", m.tris.len(), t_mesh * 1e3, m.tris.len() as f64 / t_mesh / 1e3, m.quality().0);
    }
    let t = Instant::now();
    let mesh = fea_core::mesh2d::mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|_| 0.05, MeshOptions::default()).unwrap();
    let t_mesh = t.elapsed().as_secs_f64() * 1e3;
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix_node(n);
    }
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    eprintln!("pipeline at h = 0.05: mesh+Tri6 {t_mesh:.0} ms, {} dofs: assemble {:.0} / symbolic {:.0} / factor {:.0} / solve {:.0} ms", model.mesh.n_dofs(), sol.timings.assemble_ms, sol.timings.symbolic_ms, sol.timings.factor_ms, sol.timings.solve_ms);
}

/// Where a nonlinear Newton iteration spends its time: element assembly (internal force + tangent),
/// numeric factorization, solve. Decides whether a tiled/SIMD element kernel is worth building.
#[test]
#[ignore]
fn nonlinear_iteration_profile() {
    use fea_core::material::J2;
    use fea_core::nonlinear::NlState;
    use mechanics_core::hardening::Hardening;
    for (kind, n) in [(ElementKind::Hex8, 16usize), (ElementKind::Hex20, 8), (ElementKind::Hex27, 7), (ElementKind::Quad9, 120)] {
        let physics = if kind.dim() == 3 { Physics::Solid } else { Physics::PlaneStrain { thickness: 1.0 } };
        let mut mesh = grid(physics, kind, Elastic::new(10.0e6, 0.3), [n, n, n], &|p| p).unwrap();
        mesh.set_plasticity(0, J2::small(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for i in 0..model.mesh.nodes.len() {
            if model.mesh.nodes[i][0] < 1e-12 {
                bc.fix_node(i);
            }
        }
        // A displacement field with plasticity everywhere.
        let u: Vec<f64> = (0..model.mesh.n_dofs()).map(|i| 0.03 * ((i * 2654435761) % 1000) as f64 / 1000.0 * model.mesh.nodes[i / model.mesh.dim()][0]).collect();
        let old = NlState::new(&model.mesh);
        let reps = 3;
        let t = Instant::now();
        let mut new = old.clone();
        for _ in 0..reps {
            new = old.clone();
            let _ = fea_core::nonlinear::assemble_nl_public(&model.mesh, &model.pattern, &u, &old, &mut new, true).unwrap();
        }
        let t_asm = t.elapsed().as_secs_f64() * 1e3 / reps as f64;
        let k = model.assemble().unwrap();
        let red = fea_core::linear::Reduced::with_ordering(&model.pattern, &bc, Some(&model.mesh.nodes), Ordering::Auto).unwrap();
        let t = Instant::now();
        let fac = red.factor(&k).unwrap();
        let t_fac = t.elapsed().as_secs_f64() * 1e3;
        let mut rhs = vec![1.0; red.n_free()];
        let t = Instant::now();
        fac.solve_reduced(&mut rhs);
        let t_sol = t.elapsed().as_secs_f64() * 1e3;
        eprintln!("{kind:?} {n}^d ({} elements, {} dofs, ep max {:.3}): tangent assembly {t_asm:.0} ms, factor {t_fac:.0} ms, solve {t_sol:.0} ms", model.mesh.n_elems(), model.mesh.n_dofs(), new.max_ep());
    }
}

/// Full Newton against chord Newton on a 3D plastic block stretched into the plastic range.
#[test]
#[ignore]
fn chord_newton_on_a_3d_plastic_block() {
    use fea_core::material::J2;
    use fea_core::{Control, NlOptions};
    use mechanics_core::hardening::Hardening;
    for (kind, n) in [(ElementKind::Hex20, 7usize), (ElementKind::Hex8, 14)] {
        let mut mesh = grid(Physics::Solid, kind, Elastic::new(10.0e6, 0.3), [n, n, n], &|p| p).unwrap();
        mesh.set_plasticity(0, J2::finite(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for i in 0..model.mesh.nodes.len() {
            let x = model.mesh.nodes[i];
            if x[0] < 1e-12 {
                bc.fix(i, 0, 0.0);
            }
            if x[1] < 1e-12 {
                bc.fix(i, 1, 0.0);
            }
            if x[2] < 1e-12 {
                bc.fix(i, 2, 0.0);
            }
            if x[0] > 1.0 - 1e-12 {
                bc.fix(i, 0, 0.25); // 25 % stretch, nonuniform through the lateral constraint of the notch below
            }
        }
        // A weak spot: pin one lateral corner so the field is not uniform.
        for i in 0..model.mesh.nodes.len() {
            let x = model.mesh.nodes[i];
            if x[0] > 1.0 - 1e-12 && x[1] > 1.0 - 1e-12 {
                bc.fix(i, 1, -0.02);
            }
        }
        for chord in [0usize, 5] {
            let sol = model.solve_nonlinear(&Loads::default(), &bc, &NlOptions { steps: 5, chord_iters: chord, control: Control::Load, ..NlOptions::default() }).unwrap();
            let iters: usize = sol.steps.iter().map(|s| s.iterations).sum();
            eprintln!("{kind:?} {n}^3 ({} dofs), chord_iters {chord}: {:?}, {iters} iterations, {} factorisations, {:.0} ms, ep max {:.3}", model.mesh.n_dofs(), sol.stop, sol.factorisations, sol.elapsed_ms, sol.state.max_ep());
        }
    }
}

/// Modal analysis cost: shift-and-invert subspace iteration on cantilever bars (the factorisation, the iterations and the
/// time; `docs/adr/ADR-013-eigen-and-transient-methods.md`).
#[test]
#[ignore]
fn modal_analysis_cost() {
    use fea_core::{EigMethod, ModalOptions};
    for (kind, div) in [(ElementKind::Quad9, [200usize, 4, 1]), (ElementKind::Hex20, [20, 3, 3]), (ElementKind::Hex20, [40, 4, 4]), (ElementKind::Hex8, [30, 6, 6])] {
        let physics = if div[2] == 1 { Physics::PlaneStress { thickness: 1.0 } } else { Physics::Solid };
        let mut mesh = grid(physics, kind, Elastic::new(1.0e7, 0.3), div, &|p| [10.0 * p[0], p[1], p[2]]).unwrap();
        mesh.set_density_all(1.0).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for n in 0..model.mesh.nodes.len() {
            if model.mesh.nodes[n][0] < 1e-12 {
                bc.fix_node(n);
            }
        }
        for (nev, method) in [6usize, 20].into_iter().flat_map(|n| [(n, EigMethod::Subspace), (n, EigMethod::Lanczos)]) {
            let t = Instant::now();
            let r = model.modal(&bc, &ModalOptions { n_modes: nev, method, ..ModalOptions::default() }).unwrap();
            eprintln!("BENCH modal {method:?} {kind:?} {div:?} ({} dofs) nev {nev}: {} iterations, subspace {}, factor nnz {}, {:.0} ms, worst residual {:.1e}", model.mesh.n_dofs() - bc.n_fixed(), r.iterations, r.subspace, r.factor_nnz, ms(t.elapsed()), r.modes.iter().map(|m| m.residual).fold(0.0f64, f64::max));
        }
    }
}

/// Roadmap performance evidence, not a speed assertion. Benchmark-only timing-trained decision
/// stump; production selection remains deterministic. Includes held-out aspect/physics cases.
#[test]
#[ignore = "profile and advisory experiment: run explicitly with --ignored --nocapture"]
fn roadmap_profiles_and_advisory_selection() {
    use fea_core::fields::Field::{Rotation as R, Translation as U};
    use fea_core::structural::{FrameSection, Member, MemberModel};
    for div in [8, 32, 128] {
        let l = 4.0;
        let section = FrameSection {
            e: 1e7,
            g: 4e6,
            area: 0.3,
            iy: 0.02,
            iz: 0.03,
            torsion: 0.01,
            shear_areas: None,
        };
        let model = MemberModel::new(
            (0..=div)
                .map(|n| [l * n as f64 / div as f64, 0.0, 0.0])
                .collect(),
            (0..div)
                .map(|n| Member::Frame {
                    nodes: [n, n + 1],
                    section,
                    reference: [0.0, 1.0, 0.0],
                    uniform_load: [0.0; 3],
                })
                .collect(),
        )
        .unwrap();
        let mut bc = model.fields().constraints();
        for axis in 0..3 {
            bc.prescribe(model.fields(), 0, U(axis), 0.0).unwrap();
            bc.prescribe(model.fields(), 0, R(axis), 0.0).unwrap();
        }
        let exact = 100.0 * l.powi(3) / (3.0 * section.e * section.iz);
        let (elapsed, out) = best(3, || model.solve(&bc, &[(div, U(1), 100.0)], 1e-8).unwrap());
        let error = (out.fields.value(div, U(1)).unwrap() / exact - 1.0).abs();
        assert!(error < 1e-6, "frame div={div}: error {error:e}");
        let repeat = model.solve(&bc, &[(div, U(1), 100.0)], 1e-8).unwrap();
        assert_eq!(
            out.fields.values, repeat.fields.values,
            "frame results must reproduce bit for bit"
        );
        eprintln!("ROADMAP frame div={div} dofs={} min_ms={:.3} relative_error={error:.3e} residual={:.3e}",model.fields().n_dofs(),ms(elapsed),out.fields.rel_residual);
    }
    fn problem(n: usize, aspect: f64, solid: bool) -> (Model, Loads, Dirichlet, f64) {
        let physics = if solid {
            Physics::Solid
        } else {
            Physics::PlaneStress { thickness: 1.0 }
        };
        let kind = if solid {
            ElementKind::Hex8
        } else {
            ElementKind::Quad4
        };
        let mesh = grid(
            physics,
            kind,
            Elastic::new(1e7, 0.0),
            [n, n, if solid { n } else { 1 }],
            &|p| [aspect * p[0], p[1], p[2]],
        )
        .unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &node in model.mesh.node_set("u0").unwrap() {
            bc.fix_node(node);
        }
        let loads = Loads {
            faces: model.mesh.surfaces["u1"]
                .iter()
                .map(|f| (f.clone(), SurfaceLoad::Traction([100.0, 0.0, 0.0])))
                .collect(),
            ..Default::default()
        };
        (model, loads, bc, 100.0 * aspect / 1e7)
    }
    fn timed(
        model: &Model,
        loads: &Loads,
        bc: &Dirichlet,
        choose: &dyn Fn() -> bool,
        exact: f64,
    ) -> (f64, Adaptive) {
        let (elapsed, out) = best(3, || {
            let method = if choose() {
                SolveMethod::Iterative {
                    tol: 1e-10,
                    max_iter: 400,
                }
            } else {
                SolveMethod::Direct
            };
            model
                .solve_adaptive(
                    loads,
                    bc,
                    &Requirements {
                        force_method: Some(method),
                        ..Default::default()
                    },
                )
                .unwrap()
        });
        let dim = model.mesh.dim();
        let mut error = 0.0_f64;
        for &node in model.mesh.node_set("u1").unwrap() {
            error = error.max((out.solution.u[node * dim] / exact - 1.0).abs());
        }
        assert!(error < 1e-6, "independent axial response error {error:e}");
        assert!(out.verification.passed);
        (ms(elapsed), out)
    }
    let mut training = Vec::new();
    for n in [4, 8, 12] {
        let (model, loads, bc, exact) = problem(n, 1.0, true);
        let (direct, d) = timed(&model, &loads, &bc, &|| false, exact);
        let (iterative, i) = timed(&model, &loads, &bc, &|| true, exact);
        let dofs = model.mesh.n_dofs() - bc.n_fixed();
        eprintln!("ROADMAP train n={n} free={dofs} direct_ms={direct:.3} iterative_ms={iterative:.3} iterative_actual={:?} factor_nnz={} residual_direct={:.3e} residual_iterative={:.3e}",i.method,d.solution.factor_nnz,d.verification.rel_residual,i.verification.rel_residual);
        training.push((dofs, direct, iterative));
    }
    // Cost-minimizing one-feature decision stump, fit only to training timings.
    let thresholds = [0, training[0].0 + 1, training[1].0 + 1, usize::MAX];
    let threshold = *thresholds
        .iter()
        .min_by(|&&a, &&b| {
            let cost = |threshold| {
                training
                    .iter()
                    .map(|&(d, direct, iterative)| if d >= threshold { iterative } else { direct })
                    .sum::<f64>()
            };
            cost(a).total_cmp(&cost(b))
        })
        .unwrap();
    eprintln!("ROADMAP advisory trained_threshold={threshold}; OOD reverts to deterministic direct; benchmark only");
    for (n, aspect, solid) in [
        (6, 1.0, true),
        (10, 1.0, true),
        (6, 25.0, true),
        (6, 0.04, true),
        (12, 3.0, false),
    ] {
        let (model, loads, bc, exact) = problem(n, aspect, solid);
        let dofs = model.mesh.n_dofs() - bc.n_fixed();
        let in_domain = solid && aspect == 1.0 && dofs >= training[0].0 && dofs <= training[2].0;
        let choice = in_domain && dofs >= threshold;
        let (direct, d) = timed(&model, &loads, &bc, &|| false, exact);
        let (advised, a) = timed(
            &model,
            &loads,
            &bc,
            &|| {
                let free = model.mesh.n_dofs() - bc.n_fixed();
                std::hint::black_box(
                    solid
                        && aspect == 1.0
                        && free >= training[0].0
                        && free <= training[2].0
                        && free >= threshold,
                )
            },
            exact,
        );
        let error = d
            .solution
            .u
            .iter()
            .zip(&a.solution.u)
            .map(|(d, a)| (d - a).abs() / exact.abs())
            .fold(0.0_f64, f64::max);
        assert!(error < 1e-6);
        eprintln!("ROADMAP heldout n={n} aspect={aspect} solid={solid} free={dofs} in_domain={in_domain} advisory_iterative={choice} direct_ms={direct:.3} advised_ms={advised:.3} speed_ratio={:.3} displacement_difference={error:.3e} fallback_events={}",direct/advised,a.events.len());
    }
}
