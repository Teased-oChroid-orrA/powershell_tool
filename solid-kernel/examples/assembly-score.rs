//! Scoring harness for the AutoMate check (`scripts/automate/README.md`, `docs/epic-assembly-feature-advice.md`): the STEP files
//! given are the placed parts of ONE assembly (each already in the root frame, one body per file, in occurrence order). Prints one
//! JSON object with what `features::detect` finds, in millimetres: every hole, every cylinder face (either side), the fastener
//! sites and the faces that are not analytic. Parts that cannot be read are listed under `unread` and left out of the detection
//! (`occ` maps each detected body index to the argument position, so the caller can match mates to bodies).
//! `cargo run --release -p solid-kernel --example assembly-score -- OCC0.stp OCC1.stp ...`
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::features::{detect, FaceGeometry, Side};
use solid_kernel::step::{inventory, Limits};

fn v(p: [f64; 3]) -> String {
    format!("[{:.6},{:.6},{:.6}]", p[0], p[1], p[2])
}

fn read(path: &str) -> Result<AnalyticModel, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let data = inventory(&text, Limits::default()).map_err(|e| format!("inventory: {}", e.detail))?;
    AnalyticModel::read(&data, Options::default())
        .or_else(|_| AnalyticModel::read(&data, Options::foreign_cad(0.05)))
        .map_err(|e| format!("read: {}", e.detail))
}

fn main() {
    let (mut models, mut occ, mut unread) = (Vec::new(), Vec::new(), Vec::new());
    for (i, path) in std::env::args().skip(1).enumerate() {
        match read(&path) {
            Ok(m) => {
                models.push(m);
                occ.push(i);
            }
            Err(e) => unread.push(format!("\"{i}: {}\"", e.replace('"', "'"))),
        }
    }
    let started = std::time::Instant::now();
    let f = match detect(&models) {
        Ok(f) => f,
        Err(e) => {
            println!("{{\"error\":\"detect: {}\",\"unread\":[{}]}}", e.detail.replace('"', "'"), unread.join(","));
            return;
        }
    };
    let seconds = started.elapsed().as_secs_f64();
    let mut cyl = Vec::new();
    let mut other = Vec::new();
    for face in &f.faces {
        let body = occ[face.key.body];
        match &face.geometry {
            FaceGeometry::Cylinder { axis_point, axis, radius, side, range, arcs } => cyl.push(format!(
                "{{\"body\":{body},\"p\":{},\"a\":{},\"r\":{radius:.6},\"side\":\"{}\",\"range\":[{:.6},{:.6}],\"wrap\":{:.6},\"arcs\":[{}]}}",
                v(*axis_point),
                v(*axis),
                if *side == Side::Concave { "concave" } else { "convex" },
                range[0],
                range[1],
                face.geometry.wrap().unwrap_or(0.0),
                arcs.iter().map(|a| format!("[{:.5},{:.5}]", a[0], a[1])).collect::<Vec<_>>().join(",")
            )),
            FaceGeometry::Other(_) | FaceGeometry::Cone { .. } => {
                let kind = if matches!(face.geometry, FaceGeometry::Cone { .. }) { "cone" } else { "other" };
                other.push(format!("{{\"body\":{body},\"kind\":\"{kind}\",\"lo\":{},\"hi\":{}}}", v(face.lo), v(face.hi)))
            }
            FaceGeometry::Plane { .. } => {}
        }
    }
    let holes: Vec<String> = f
        .holes
        .iter()
        .map(|h| {
            format!(
                "{{\"body\":{},\"p\":{},\"a\":{},\"d\":{:.6},\"range\":[{:.6},{:.6}],\"closure\":\"{:?}\"}}",
                occ[h.body],
                v(h.axis_point),
                v(h.axis),
                h.diameter(),
                h.range[0],
                h.range[1],
                h.closure()
            )
        })
        .collect();
    let sites: Vec<String> = f
        .sites
        .iter()
        .map(|s| {
            let bodies: Vec<String> = s.bodies.iter().map(|b| occ[*b].to_string()).collect();
            let diam: Vec<String> = s.bores.iter().map(|b| format!("{:.6}", b.diameter)).collect();
            format!(
                "{{\"bodies\":[{}],\"p\":{},\"a\":{},\"span\":[{:.6},{:.6}],\"diam\":[{}],\"fastener\":{},\"mismatch\":{},\"bridged_by\":{:?}}}",
                bodies.join(","),
                v(s.axis_point),
                v(s.axis),
                s.span[0],
                s.span[1],
                diam.join(","),
                s.fastener.map_or("null".to_string(), |n| format!("\"{n}\"")),
                s.diameter_mismatch,
                s.bridged_by.iter().map(|b| occ[*b]).collect::<Vec<_>>()
            )
        })
        .collect();
    println!(
        "{{\"occ\":{:?},\"unread\":[{}],\"seconds\":{seconds:.4},\"holes\":[{}],\"cyl\":[{}],\"sites\":[{}],\"interfaces\":{},\"unresolved\":{},\"other\":[{}]}}",
        occ,
        unread.join(","),
        holes.join(","),
        cyl.join(","),
        sites.join(","),
        f.interfaces.len(),
        f.unresolved.len(),
        other.join(",")
    );
}
