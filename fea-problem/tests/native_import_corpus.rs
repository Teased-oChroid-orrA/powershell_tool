//! Evidence, not a gate: the production native import (`import_step_native`) over every STEP file of a local folder
//! (`STEP_DIR`, not committed: a licensed real-CAD corpus such as the NIST PMI files). Prints one line per file and a tally.
//! `cargo test -p fea-problem --release --test native_import_corpus -- --ignored --nocapture`
//! With `MESH=1` every imported file is also meshed through `solid_mesh` at the import's own size (the application's path),
//! printing tets, the volume against the tessellation's and the share of cells under the default quality gate.
use fea_core::meshing::adjacency::Adjacency;
use fea_core::meshing::quality::QualityThresholds;
use fea_problem::native_solid::import_step_native;
use fea_problem::solid::solid_mesh;
use fea_problem::Problem;

#[test]
#[ignore = "needs STEP_DIR, a local corpus"]
fn the_native_import_over_a_local_corpus() {
    let dir = std::env::var("STEP_DIR").expect("STEP_DIR");
    let mut paths = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("step") || x.eq_ignore_ascii_case("stp")) {
                paths.push(p);
            }
        }
    }
    paths.sort();
    let mesh = std::env::var_os("MESH").is_some();
    let (mut ok, mut n, mut meshed) = (0, 0, 0);
    for p in &paths {
        let text = String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned();
        let t = std::time::Instant::now();
        let r = import_step_native(&text);
        n += 1;
        ok += usize::from(r.is_ok());
        let what = match &r {
            Ok(i) => {
                // Signed volume of the closed triangle mesh (divergence theorem), model units cubed.
                let v = |k: usize| i.cache.vertices[k];
                let volume: f64 = i
                    .cache
                    .triangles
                    .iter()
                    .map(|t| {
                        let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
                        (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.
                    })
                    .sum();
                format!("ok: {} faces, {} triangles, size {:.3}, volume {volume:.3}", i.cache.face_names().len(), i.cache.triangles.len(), i.size)
            }
            Err(e) => format!("REFUSED: {}", e.chars().take(600).collect::<String>()),
        };
        eprintln!("{:<42} {:>6.1}s  {what}", p.file_name().unwrap().to_string_lossy(), t.elapsed().as_secs_f64());
        if mesh && r.is_ok() {
            let t = std::time::Instant::now();
            let line = match Problem::from_step("corpus", &text).and_then(|pr| solid_mesh(&pr)) {
                Ok((m, rep)) => {
                    meshed += 1;
                    let gate = QualityThresholds::default();
                    let q = m.quality().with_progression(&Adjacency::new(&m));
                    let below = q.elements.iter().filter(|e| e.scaled_jacobian < gate.min_scaled_jacobian).count();
                    format!("meshed: {} tets, volume {:.3}, min scaled Jacobian {:.3}, {below} under the gate, gate {}", rep.tets, rep.volume, q.min_scaled_jacobian, if q.acceptable(&gate) { "ok" } else { "NO" })
                }
                Err(e) => format!("MESH REFUSED: {}", e.chars().take(300).collect::<String>()),
            };
            eprintln!("{:<42} {:>6.1}s  {line}", "  mesh", t.elapsed().as_secs_f64());
        }
    }
    eprintln!("summary: {ok} of {n} files import natively");
    if mesh {
        eprintln!("summary: {meshed} of {ok} imported files mesh through solid_mesh");
    }
}
