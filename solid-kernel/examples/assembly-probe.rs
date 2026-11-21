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
        if std::env::var_os("PROBE_VERBOSE").is_some() {
            let mut radii: std::collections::BTreeMap<(String, i64), usize> = std::collections::BTreeMap::new();
            for x in &f.faces {
                if let FaceGeometry::Cylinder { side, radius, .. } = &x.geometry {
                    *radii.entry((format!("{side:?}"), (radius * 1000.0).round() as i64)).or_default() += 1;
                }
            }
            for ((side, r), n) in &radii {
                eprintln!("  cylinder faces {side} r={:.3} mm: {n}", *r as f64 / 1000.0);
            }
            if std::env::var_os("PROBE_FACES").is_some() {
                for x in &f.faces {
                    if let FaceGeometry::Cylinder { side, radius, range, axis_point, axis, arcs } = &x.geometry {
                        eprintln!("FACE\t{}\t{side:?}\tr={radius:.3}\twrap={:.3}\trange=[{:.3},{:.3}]\taxis={axis:.3?}\tpt={axis_point:.2?}\tarcs={arcs:.3?}", x.key.face, x.geometry.wrap().unwrap(), range[0], range[1]);
                    }
                }
            }
            for h in &f.holes {
                eprintln!("HOLE\t{name}\t{}\t{:.4}\t{:.4}\t{:?}", h.body, h.diameter(), h.depth(), h.closure());
            }
            for i in &f.interfaces {
                eprintln!("INTERFACE\t{name}\t{}\t{}\t{:.4}\t{:.4}", i.a.body, i.b.body, i.overlap_area, i.gap);
            }
            for s in &f.sites {
                let radii: Vec<String> = s.bores.iter().map(|b| format!("{:.2}", b.diameter / 2.0)).collect();
                eprintln!("  site bodies={:?} bore r=[{}] fastener={:?} mismatch={}", s.bodies, radii.join(","), s.fastener, s.diameter_mismatch);
            }
        }
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
