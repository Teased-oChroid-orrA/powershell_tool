//! Prints the faces of a STEP file with their surface, bounding box extent and edge loops (edge lengths, neighbouring faces):
//! where the small faces of a model are. `cargo run -p solid-kernel --example face-probe -- FILE.step [--below AREA] [--foreign]`
//! (`--foreign` reads with `Options::foreign_cad(0.05)`, as `abc-corpus --foreign` does; `PROBE_EDGES=id,id` prints those
//! edges' samples).
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::step::{inventory, Limits};
use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: face-probe FILE.step [--below AREA] [--foreign]");
    let mut below = f64::INFINITY;
    let mut options = Options::default();
    while let Some(a) = args.next() {
        if a == "--below" {
            below = args.next().unwrap().parse().unwrap();
        } else if a == "--foreign" {
            options = Options::foreign_cad(0.05);
        }
    }
    let text = std::fs::read_to_string(path).unwrap();
    let data = inventory(&text, Limits::default()).unwrap();
    let model = AnalyticModel::read_all(&data, options).unwrap().remove(0);
    let body = model.body();
    let mut owners: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for f in body.faces.values() {
        for b in &f.bounds {
            for u in &b.edges {
                owners.entry(u.edge).or_default().push(f.id);
            }
        }
    }
    for f in body.faces.values() {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut perimeter = 0.0;
        for b in &f.bounds {
            for u in &b.edges {
                let pts = model.edge_points(u.edge).unwrap();
                for w in pts.windows(2) {
                    perimeter += ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
                }
                for p in pts {
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
            }
        }
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        println!("face #{} {:?} extent {:.4} x {:.4} x {:.4}, perimeter {:.4}", f.id, std::mem::discriminant(model.surface(f.id).unwrap()), ext[0], ext[1], ext[2], perimeter);
        let _ = below;
        for b in &f.bounds {
            for u in &b.edges {
                let pts = model.edge_points(u.edge).unwrap();
                let len: f64 = pts.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt()).sum();
                let e = &body.edges[&u.edge];
                let others: Vec<u64> = owners[&u.edge].iter().copied().filter(|o| *o != f.id).collect();
                println!("    edge #{} {}->{} length {:.5} samples {} also on {:?}", u.edge, e.start, e.end, len, pts.len(), others);
                if std::env::var("PROBE_EDGES").map(|v| v.split(',').any(|x| x == u.edge.to_string())).unwrap_or(false) {
                    for p in pts.iter() {
                        println!("        {:.9} {:.9} {:.9}", p[0], p[1], p[2]);
                    }
                }
            }
        }
    }
}
