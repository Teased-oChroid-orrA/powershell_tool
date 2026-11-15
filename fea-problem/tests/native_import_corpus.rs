//! Evidence, not a gate: the production native import (`import_step_native`) over every STEP file of a local folder
//! (`STEP_DIR`, not committed: a licensed real-CAD corpus such as the NIST PMI files). Prints one line per file and a tally.
//! `cargo test -p fea-problem --release --test native_import_corpus -- --ignored --nocapture`
use fea_problem::native_solid::import_step_native;

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
    let (mut ok, mut n) = (0, 0);
    for p in &paths {
        let text = String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned();
        let t = std::time::Instant::now();
        let r = import_step_native(&text);
        n += 1;
        ok += usize::from(r.is_ok());
        let what = match &r {
            Ok(i) => format!("ok: {} faces, {} triangles, size {:.3}", i.cache.face_names().len(), i.cache.triangles.len(), i.size),
            Err(e) => format!("REFUSED: {}", e.chars().take(420).collect::<String>()),
        };
        eprintln!("{:<42} {:>6.1}s  {what}", p.file_name().unwrap().to_string_lossy(), t.elapsed().as_secs_f64());
    }
    eprintln!("summary: {ok} of {n} files import natively");
}
