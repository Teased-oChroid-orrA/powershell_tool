//! Heap allocations per stage of the estimators (a counting global allocator), to decide with numbers whether allocation is a
//! bottleneck: `cargo test -p fea-core --release --test alloc_profile -- --ignored --nocapture`.

use fea_core::adaptivity::dwr::DwrEstimator;
use fea_core::adaptivity::equilibrated::reconstruct;
use fea_core::adaptivity::*;
use fea_core::geometry::{Loop, Region};
use fea_core::meshing::hybrid::{mesh_unstructured_2d, ElementFamily, HybridOptions2d};
use fea_core::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;
static COUNT: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        COUNT.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size(), Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn measure<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let (c0, b0, t0) = (COUNT.load(Relaxed), BYTES.load(Relaxed), std::time::Instant::now());
    let r = f();
    let (dc, db) = (COUNT.load(Relaxed) - c0, BYTES.load(Relaxed) - b0);
    eprintln!("{name:<28} {:>9} allocations {:>8.1} MB {:>8.1} ms ({:.0} ns per allocation)", dc, db as f64 / 1e6, 1e3 * t0.elapsed().as_secs_f64(), 1e9 * t0.elapsed().as_secs_f64() / dc.max(1) as f64);
    r
}

#[test]
#[ignore = "profile: run with --release --ignored --nocapture"]
fn allocations_per_stage() {
    let region = Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()], Elastic::new(1.0e7, 0.3)).unwrap();
    let size = |p: [f64; 2]| (0.04 + 0.15 * ((p[0] - 6.0).hypot(p[1] - 2.0) - 0.6).max(0.0)).min(0.5);
    let opt = HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Quadratic);
    let mesh = measure("mesh (unstructured)", || mesh_unstructured_2d(&region, &size, &opt, "alloc").unwrap().mesh);
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    bc.fix(model.mesh.node_set("left").unwrap()[0], 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    let sol = measure("solve", || model.solve_static(&loads, &bc).unwrap());
    let adj = model.mesh.adjacency();
    let ctx = EstimatorContext::new(&model, &loads, &bc, &sol.u, &adj).unwrap();
    measure("ZZ", || model.zz_error(&sol.u, 0.0).unwrap());
    measure("equilibrated", || reconstruct(&ctx).unwrap());
    let w = fea_core::adaptivity::goal::LinearFunctional::compliance(&model, &loads).unwrap();
    measure("recovered-stress DWR", || DwrEstimator.estimate_goals(&ctx, &[&w]).unwrap());
}
