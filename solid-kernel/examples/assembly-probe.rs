//! Corpus run for the assembly-feature epic (`docs/epic-assembly-feature-advice.md`, Phase A): per STEP file, how many bodies
//! `AnalyticModel::read_all` yields and what `features::detect` finds (holes by closure, fastener sites, interfaces).
//! `cargo run --release -p solid-kernel --example assembly-probe -- FILE.stp [FILE.stp ...]`: one tab-separated row per file.
//! The read order is the production one (strict, then `Options::foreign_cad(0.05)`). Models are never copied into the repository;
//! only the aggregate counts are recorded in the docs.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::features::{detect, Closure, FaceGeometry, Side};
use solid_kernel::step::{inventory, Limits};
use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().into_owned();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                println!("{name}\tERR\tread: {e}");
                continue;
            }
        };
        let data = match inventory(&text, Limits::default()) {
            Ok(d) => d,
            Err(e) => {
                println!("{name}\tERR\tinventory: {}", e.detail);
                continue;
            }
        };
        let models = match AnalyticModel::read_all(&data, Options::default()).or_else(|_| AnalyticModel::read_all(&data, Options::foreign_cad(0.05))) {
            Ok(m) => m,
            Err(e) => {
                println!("{name}\tERR\tread_all: {}", e.detail);
                continue;
            }
        };
        let started = Instant::now();
        let f = match detect(&models) {
            Ok(f) => f,
            Err(e) => {
                println!("{name}\tERR\tdetect: {}", e.detail);
                continue;
            }
        };
        let seconds = started.elapsed().as_secs_f64();
        let concave_cylinders = f.faces.iter().filter(|x| matches!(x.geometry, FaceGeometry::Cylinder { side: Side::Concave, .. })).count();
        let grouped: usize = f.holes.iter().map(|h| h.faces.len()).sum();
        let count = |c: Closure| f.holes.iter().filter(|h| h.closure() == c).count();
        println!(
            "{name}\tbodies={}\tfaces={}\tconcave_cyl={concave_cylinders}\tholes={}\tgrouped={grouped}\tthrough={}\tblind={}\tenclosed={}\tsites={}\tinterfaces={}\tunresolved={}\tdetect_s={seconds:.3}",
            models.len(),
            f.faces.len(),
            f.holes.len(),
            count(Closure::Through),
            count(Closure::Blind),
            count(Closure::Enclosed),
            f.sites.len(),
            f.interfaces.len(),
            f.unresolved.len(),
        );
    }
}
