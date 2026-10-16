//! Phase 8 benchmark and hardening suite for the derivative engine. It measures (and prints; run with `--nocapture`) the cost of
//! each differentiation mode against the plain kernel and the analytic and difference references, refits the policy's
//! [`CostCalibration`], counts allocations, and asserts the properties that must hold on any machine: exactness of AD against the
//! closed forms, bitwise determinism across thread counts, and deterministic storage/accuracy invariants. Wall-clock
//! numbers are observations: SIMD, compiler choices and shared-runner scheduling can change their ordering.

use fea_core::derivative::dual::Dual;
use fea_core::derivative::element;
use fea_core::derivative::matrix_free::*;
use fea_core::derivative::path::*;
use fea_core::derivative::policy::CostCalibration;
use fea_core::derivative::scalar::Scalar;
use fea_core::derivative::tape::Tape;
use fea_core::generate::grid;
use fea_core::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::time::Instant;

struct Counting;
thread_local! {
    // per thread, so that tests running in parallel do not count each other's allocations
    static ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}
#[global_allocator]
static A: Counting = Counting;

/// Best of `reps` repetitions of `f` run `inner` times, per call, in nanoseconds.
fn time(reps: usize, inner: usize, mut f: impl FnMut()) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..reps {
        let t = Instant::now();
        for _ in 0..inner {
            f();
        }
        best = best.min(t.elapsed().as_nanos() as f64 / inner as f64);
    }
    best
}

fn quad8_coords() -> Vec<[f64; 3]> {
    let mesh = grid(Physics::PlaneStress { thickness: 0.5 }, ElementKind::Quad8, Elastic::new(1.0e5, 0.3), [1, 1, 1], &|p| [1.7 * p[0] + 0.1 * p[1], p[1] + 0.07 * p[0] * p[0], 0.0]).unwrap();
    let blk = &mesh.blocks[0];
    blk.elem(0).iter().map(|&n| mesh.nodes[n]).collect()
}

fn stiffness<S: Scalar>(coords: &[[S; 3]], e: S) -> Vec<S> {
    element::stiffness(ElementKind::Quad8, Physics::PlaneStress { thickness: 0.5 }, coords, e, S::cst(0.3), S::cst(0.5)).unwrap()
}

#[test]
fn ad_is_exact_to_rounding_where_a_closed_form_exists_and_differences_are_not() {
    // K is linear in E, so dK/dE = K / E exactly: the reference needs no differentiation at all
    let xyz = quad8_coords();
    let plain: Vec<[f64; 3]> = xyz.clone();
    let k = stiffness(&plain, 1.0e5);
    let scale = k.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let dual_coords: Vec<[Dual<f64, 1>; 3]> = xyz.iter().map(|p| p.map(Dual::cst)).collect();
    let kd = stiffness(&dual_coords, Dual::<f64, 1>::with(1.0e5, [1.0]));
    let ad_err = k.iter().zip(&kd).map(|(a, d)| (d.d[0] - a / 1.0e5).abs()).fold(0.0f64, f64::max) * 1.0e5 / scale;
    let h = 1.0e-4 * 1.0e5;
    let (kp, km) = (stiffness(&plain, 1.0e5 + h), stiffness(&plain, 1.0e5 - h));
    let fd_err = k.iter().enumerate().map(|(i, a)| ((kp[i] - km[i]) / (2.0 * h) - a / 1.0e5).abs()).fold(0.0f64, f64::max) * 1.0e5 / scale;
    eprintln!("dK/dE: AD relative error {ad_err:.2e}, central difference (h = 1e-4 E) {fd_err:.2e}");
    assert!(ad_err < 1e-13, "AD of a linear dependence is exact to rounding: {ad_err:e}");
    assert!(fd_err > 10.0 * ad_err.max(1e-16), "the difference is a truncation / cancellation approximation, not exact");
}

#[test]
fn cost_of_each_mode_relative_to_the_plain_kernel_and_the_refit_calibration() {
    let xyz = quad8_coords();
    let t_plain = time(7, 400, || {
        std::hint::black_box(stiffness(&xyz, 1.0e5));
    });
    let d1: Vec<[Dual<f64, 1>; 3]> = xyz.iter().map(|p| p.map(Dual::cst)).collect();
    let d4: Vec<[Dual<f64, 4>; 3]> = xyz.iter().map(|p| p.map(Dual::cst)).collect();
    let d8: Vec<[Dual<f64, 8>; 3]> = xyz.iter().map(|p| p.map(Dual::cst)).collect();
    let t1 = time(7, 400, || {
        std::hint::black_box(stiffness(&d1, Dual::<f64, 1>::var(1.0e5, 0)));
    });
    let t4 = time(7, 400, || {
        std::hint::black_box(stiffness(&d4, Dual::<f64, 4>::var(1.0e5, 0)));
    });
    let t8 = time(7, 400, || {
        std::hint::black_box(stiffness(&d8, Dual::<f64, 8>::var(1.0e5, 0)));
    });
    // reverse mode: record once, sweep once for one output entry
    let tape = Tape::<f64>::with_limit(1 << 22);
    let mut nodes = 0;
    let t_rec = time(7, 200, || {
        tape.reset();
        let c: Vec<[_; 3]> = xyz.iter().map(|p| p.map(|x| tape.var(x))).collect();
        let k = stiffness(&c, tape.var(1.0e5));
        std::hint::black_box(&k);
        nodes = tape.len();
    });
    let t_sweep = {
        tape.reset();
        let c: Vec<[_; 3]> = xyz.iter().map(|p| p.map(|x| tape.var(x))).collect();
        let k = stiffness(&c, tape.var(1.0e5));
        time(7, 200, || {
            std::hint::black_box(tape.sweep(&k[0], 1.0));
        })
    };
    let cx: Vec<[fea_core::derivative::complex::Complex; 3]> = xyz.iter().map(|p| p.map(|x| fea_core::derivative::complex::Complex::new(x, 0.0))).collect();
    let t_cx = time(7, 400, || {
        std::hint::black_box(stiffness(&cx, fea_core::derivative::complex::Complex::new(1.0e5, 1e-30)));
    });
    let hd: Vec<[fea_core::derivative::hyperdual::HyperDual; 3]> = xyz.iter().map(|p| p.map(|x| fea_core::derivative::hyperdual::HyperDual::new(x, 0.0, 0.0, 0.0))).collect();
    let t_hd = time(7, 400, || {
        std::hint::black_box(stiffness(&hd, fea_core::derivative::hyperdual::HyperDual::seeded(1.0e5, true, true)));
    });
    let lane = |t: f64, w: f64| ((t / t_plain) - 1.0) / w;
    let cal = CostCalibration { dual_lane: (lane(t1, 1.0) + lane(t4, 4.0) + lane(t8, 8.0)) / 3.0, tape_record: t_rec / t_plain, tape_sweep: t_sweep / t_plain, complex: t_cx / t_plain, hyperdual: t_hd / t_plain, ..CostCalibration::default() };
    eprintln!("Quad8 stiffness: plain {t_plain:.0} ns; dual x1 {t1:.0}, x4 {t4:.0}, x8 {t8:.0}; tape record {t_rec:.0} ({nodes} nodes, {} B), sweep {t_sweep:.0}", tape.bytes());
    eprintln!("measured calibration on this machine: {cal:?}   (defaults {:?})", CostCalibration::default());
    // Timings calibrate a model; relative speed is not a portable correctness law.
    // On the Windows runner eight lanes were faster than four. Preserve the
    // observations while verifying the numerical work in every lane instead.
    for t in [t_plain,t1,t4,t8,t_rec,t_sweep,t_cx,t_hd] { assert!(t.is_finite() && t>0.0); }
    for ratio in [cal.dual_lane,cal.tape_record,cal.tape_sweep,cal.complex,cal.hyperdual] { assert!(ratio.is_finite()); }
    fn verify_lanes<const N:usize>(xyz:&[[f64;3]]) {
        let plain=stiffness(xyz,1.0e5);
        let coords:Vec<[Dual<f64,N>;3]>=xyz.iter().map(|p|p.map(Dual::cst)).collect();
        let seed=std::array::from_fn(|i|(i+1) as f64);
        let k=stiffness(&coords,Dual::<f64,N>::with(1.0e5,seed));
        let scale=plain.iter().map(|v|v.abs()).fold(0.0,f64::max);
        for (a,d) in plain.iter().zip(k) {
            assert!((d.v-a).abs()/scale<1e-13);
            for (j,&direction) in seed.iter().enumerate() {
                assert!((d.d[j]-a/1.0e5*direction).abs()*1.0e5/(scale*direction)<1e-13);
            }
        }
    }
    verify_lanes::<1>(&xyz);verify_lanes::<4>(&xyz);verify_lanes::<8>(&xyz);
    assert!(nodes > 100, "the tape recorded the element ({nodes} nodes)");
    assert_eq!(tape.bytes() / nodes.max(1), 24, "bytes per recorded node must equal the calibration's {} B", CostCalibration::default().tape_node_bytes);
}

#[test]
fn forward_mode_with_stack_lanes_does_not_allocate_beyond_the_result() {
    let xyz = quad8_coords();
    let d: Vec<[Dual<f64, 4>; 3]> = xyz.iter().map(|p| p.map(Dual::cst)).collect();
    let _ = stiffness(&d, Dual::<f64, 4>::var(1.0e5, 0)); // warm-up
    let before = ALLOCS.with(|c| c.get());
    let k = stiffness(&d, Dual::<f64, 4>::var(1.0e5, 0));
    let used = ALLOCS.with(|c| c.get()) - before;
    drop(k);
    let before = ALLOCS.with(|c| c.get());
    let k = stiffness(&xyz, 1.0e5);
    let plain = ALLOCS.with(|c| c.get()) - before;
    drop(k);
    eprintln!("allocations per Quad8 stiffness: plain {plain}, dual x4 {used}");
    assert!(used <= plain + 2, "dual numbers live on the stack: {used} allocations against {plain} for the plain kernel");
}

#[test]
fn path_sensitivities_are_bitwise_deterministic_and_report_measured_costs() {
    use fea_core::delaunay::MeshOptions;
    use fea_core::geometry::{Loop, Region};
    use fea_core::material::J2;
    use fea_core::mesh2d::mesh_region;
    use mechanics_core::hardening::Hardening;
    let build = |e_scale: f64| {
        let region = Region::new(Loop::rectangle(0.0, 0.0, 6.0, 2.0).unwrap(), vec![Loop::circle([3.0, 1.0], 0.45, "hole").unwrap()], Elastic::new(10.0e6 * e_scale, 0.3)).unwrap();
        let size = |p: [f64; 2]| (0.2 + 0.3 * ((p[0] - 3.0).hypot(p[1] - 1.0) - 0.45).max(0.0)).min(0.55);
        let mut mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
        mesh.set_plasticity(0, J2::small(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 1.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 1.0).abs())).unwrap();
        bc.fix(anchor, 1, 0.0);
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([21_000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
        (model, loads, bc)
    };
    let (model, loads, bc) = build(1.0);
    let opt = PathOptions { steps: 4, ..PathOptions::default() };
    let n = model.mesh.n_dofs();
    let mut g = vec![0.0; n];
    for &nd in model.mesh.node_set("right").unwrap() {
        g[nd * 2] = 1.0;
    }
    let q = [Qoi::displacement(g.clone())];
    let params = [NlParam::ModulusScale(0), NlParam::PoissonRatio(0), NlParam::YieldScale(0), NlParam::HardeningScale(0), NlParam::LoadScale];
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            let sol = solve_path(&model, &loads, &bc, &opt).unwrap();
            let eng = PathSensitivityEngine::new(&model, &loads, &bc).unwrap();
            let (adj, _) = eng.adjoint(&sol, &params, &q, JacobianStrategy::Recompute).unwrap();
            let (dir, _) = eng.direct(&sol, &params, &q).unwrap();
            (adj, dir)
        })
    };
    let (one, four) = (run(1), run(4));
    for (a, b) in one.0[0].iter().zip(&four.0[0]).chain(one.1[0].iter().zip(&four.1[0])) {
        assert_eq!(a.to_bits(), b.to_bits(), "the sensitivities must not depend on the thread count");
    }
    // cost: one forward solve plus a backward sweep, against two full solves per parameter for central differences
    let sol = solve_path(&model, &loads, &bc, &opt).unwrap();
    let eng = PathSensitivityEngine::new(&model, &loads, &bc).unwrap();
    let t_adj = time(3, 1, || {
        std::hint::black_box(eng.adjoint(&sol, &params, &q, JacobianStrategy::Recompute).unwrap());
    });
    let t_dir = time(3, 1, || {
        std::hint::black_box(eng.direct(&sol, &params, &q).unwrap());
    });
    let t_solve = time(3, 1, || {
        std::hint::black_box(solve_path(&model, &loads, &bc, &opt).unwrap());
    });
    let t_fd = 2.0 * params.len() as f64 * t_solve;
    eprintln!("{} dofs, {} steps, {} parameters: one solve {:.1} ms; adjoint sweep {:.1} ms (+ the solve), direct {:.1} ms, central differences {:.1} ms (2 solves per parameter)", n, opt.steps, params.len(), t_solve / 1e6, t_adj / 1e6, t_dir / 1e6, t_fd / 1e6);
    // Compare the independently computed sensitivities, not shared-runner speed.
    for (adjoint,direct) in one.0[0].iter().zip(&one.1[0]) {
        assert!((adjoint-direct).abs()/direct.abs().max(1e-30)<1e-10);
    }
}

#[test]
fn matrix_free_tangent_against_the_assembled_matvec_cost_and_memory() {
    use fea_core::delaunay::MeshOptions;
    use fea_core::geometry::{Loop, Region};
    use fea_core::material::J2;
    use fea_core::mesh2d::mesh_region;
    use fea_core::nonlinear::assemble_nl_public;
    use mechanics_core::hardening::Hardening;
    let region = Region::new(Loop::rectangle(0.0, 0.0, 6.0, 2.0).unwrap(), vec![Loop::circle([3.0, 1.0], 0.45, "hole").unwrap()], Elastic::new(10.0e6, 0.3)).unwrap();
    let size = |p: [f64; 2]| (0.08 + 0.2 * ((p[0] - 3.0).hypot(p[1] - 1.0) - 0.45).max(0.0)).min(0.4);
    let mut mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
    mesh.set_plasticity(0, J2::small(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    let n = model.mesh.n_dofs();
    let u = vec![1e-6; n];
    let old = NlState::new(&model.mesh);
    let mut new = old.clone();
    let (_, k) = assemble_nl_public(&model.mesh, &model.pattern, &u, &old, &mut new, true).unwrap();
    let k = k.unwrap();
    let v: Vec<f64> = (0..n).map(|i| ((i * 37 % 101) as f64) / 101.0 - 0.5).collect();
    let t_asm = time(5, 20, || {
        let mut y = vec![0.0; n];
        k.matvec_add(&model.pattern, &v, &mut y);
        std::hint::black_box(y);
    });
    let cached = MatrixFreeTangent::new(&model, &bc, &u, &old, MatrixFreeMode::Cached).unwrap();
    let recompute = MatrixFreeTangent::new(&model, &bc, &u, &old, MatrixFreeMode::Recompute).unwrap();
    let t_c = time(5, 20, || {
        std::hint::black_box(cached.apply_full(&v));
    });
    let t_r = time(5, 20, || {
        std::hint::black_box(recompute.apply_full(&v));
    });
    let asm_bytes = k.vals.len() * 8;
    eprintln!("{n} dofs: assembled matvec {:.0} us ({} KiB), matrix-free cached {:.0} us ({} KiB), recompute {:.0} us ({} KiB)", t_asm / 1e3, asm_bytes / 1024, t_c / 1e3, cached.stored_bytes() / 1024, t_r / 1e3, recompute.stored_bytes() / 1024);
    let mut expected=vec![0.0;n];k.matvec_add(&model.pattern,&v,&mut expected);
    let scale=expected.iter().map(|v|v.abs()).fold(0.0,f64::max);
    for actual in [cached.apply_full(&v),recompute.apply_full(&v)] {
        let error=actual.iter().zip(&expected).map(|(a,b)|(a-b).abs()).fold(0.0,f64::max);
        assert!(error/scale<1e-12,"matrix-free relative error {}",error/scale);
    }
    assert!(recompute.stored_bytes() < cached.stored_bytes());
}
