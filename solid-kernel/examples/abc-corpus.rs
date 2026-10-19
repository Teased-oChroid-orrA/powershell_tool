//! Native-pipeline survey over a directory of STEP files (the ABC dataset, kept outside the repository).
//!
//! Driver: `cargo run --release -p solid-kernel --no-default-features --example abc-corpus -- DIR OUT.csv [--limit N] [--timeout SECONDS] [--conform-timeout SECONDS]`
//! runs every `*.step`/`*.stp` under DIR in its own child process (so a panic, abort or hang costs one row, not the survey) and
//! writes one CSV row per file: `file,bytes,stage,detail,faces,vertices,triangles,volume_mm3,min_angle_deg,seconds`.
//! `stage` is the last stage reached: `inventory` -> `model` -> `shell` -> `conform` (the best, a conforming tetrahedralization
//! whose divergence-theorem volume matched), or `timeout` / `crash` when the child did not finish. `detail` is the refusal
//! reason for the stage that failed (commas and newlines removed), empty on success.
//! `--jobs N` runs N files at once (default 1). `--foreign` reads with `Options::foreign_cad` (edge/surface agreement to a
//! fiftieth of the chord) instead of the strict default.
//! `--graded` retries a failed body from the graded model (`AnalyticModel::graded`, 25 degree chart quality): slow, and on
//! the first 300 files it rescued 2.
//! Child mode (internal): `abc-corpus --one FILE [--conform-timeout S]` prints one row to stdout.
//! Models are never copied into the repository; commit aggregate statistics only (their licence stays with their creators).
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::step::{inventory, Limits};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c == ',' || c == '\n' || c == '\r' { ' ' } else { c })
        .take(160)
        .collect()
}

/// One file, all stages; returns `stage,detail,faces,vertices,triangles,volume,min_angle`.
fn survey(path: &Path, conform_timeout: Duration, foreign: bool, graded_fallback: bool) -> String {
    let row = |stage: &str, detail: &str, f: usize, v: usize, t: usize, vol: f64, ang: f64| {
        format!("{stage},{},{f},{v},{t},{vol},{ang}", clean(detail))
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return row("read", &e.to_string(), 0, 0, 0, f64::NAN, f64::NAN),
    };
    let data = match inventory(&text, Limits::default()) {
        Ok(d) => d,
        Err(e) => return row("inventory", &e.to_string(), 0, 0, 0, f64::NAN, f64::NAN),
    };
    let models = match AnalyticModel::read_all(
        &data,
        if foreign { Options::foreign_cad(0.05) } else { Options::default() },
    ) {
        Ok(m) => m,
        Err(e) => return row("model", &e.to_string(), 0, 0, 0, f64::NAN, f64::NAN),
    };
    // A multi-body part succeeds only when every body does; a failure names its body. Each body is meshed from plain
    // per-face sampling first; when that stage fails, from the graded model (`AnalyticModel::graded`), the production path.
    let (mut faces, mut nv, mut nt, mut vol, mut angle) = (0, 0, 0, 0., f64::INFINITY);
    let (mut steiner, mut cells, mut worst_err, mut graded_bodies) = (0usize, 0usize, 0f64, 0usize);
    let limits = solid_kernel::native_surface::Limits {
        face_work: 20_000_000,
        // an admission bound (faces x face_work), not accounting: sized to the part
        total_work: models.iter().map(|m| m.body().faces.len()).max().unwrap_or(1).saturating_mul(20_000_000),
        // real files carry rounded coordinates: the strict policy refuses every seam that closes to 1e-12 only
        chart_policy: solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
        ..Default::default()
    };
    for (k, model) in models.iter().enumerate() {
        let tag = |d: &str| format!("body {}/{}: {d}", k + 1, models.len());
        faces += model.body().faces.len();
        let attempt = |model: &AnalyticModel, limits: solid_kernel::native_surface::Limits| {
            let sampled = model
                .sampled_surface(limits, |_| true)
                .map_err(|e| ("sample", e.to_string(), None))?;
            let m = &sampled.mesh;
            let a = solid_kernel::plc_conform::min_shell_angle_deg(&m.vertices, &m.triangles);
            let facts = (m.vertices.len(), m.triangles.len(), sampled.signed_volume_mm3, a);
            let start = Instant::now();
            solid_kernel::plc_conform::conform(
                &m.vertices,
                &m.triangles,
                solid_kernel::plc_conform::Options::default(),
                |_| start.elapsed() < conform_timeout,
            )
            .map(|c| (facts, c.report))
            .map_err(|e| ("shell", e.to_string(), Some(facts)))
        };
        let plain = attempt(model, limits);
        let (facts, report, was_graded) = match plain {
            Ok((f, r)) => (f, r, false),
            Err(first) if !graded_fallback => {
                let (stage, detail, f) = first;
                let f = f.unwrap_or((nv, nt, vol, angle));
                return row(stage, &tag(&detail), faces, f.0, f.1, f.2, f.3);
            }
            Err(first) => {
                let graded = model
                    .graded(
                        solid_kernel::chart_grade::GradeOptions {
                            policy: limits.chart_policy,
                            ..Default::default()
                        },
                        50_000_000,
                    )
                    .map_err(|e| ("grade", e.to_string(), None))
                    .and_then(|(g, _)| {
                        attempt(
                            &g,
                            solid_kernel::native_surface::Limits {
                                chart_quality: Some(solid_kernel::chart_mesh::ChartQuality {
                                    min_angle_deg: 25.,
                                    max_points: 5000,
                                }),
                                ..limits
                            },
                        )
                    });
                match graded {
                    Ok((f, r)) => (f, r, true),
                    // report the plain failure: it is the baseline stage
                    Err(_) => {
                        let (stage, detail, f) = first;
                        let f = f.unwrap_or((nv, nt, vol, angle));
                        return row(stage, &tag(&detail), faces, f.0, f.1, f.2, f.3);
                    }
                }
            }
        };
        graded_bodies += was_graded as usize;
        nv += facts.0;
        nt += facts.1;
        vol += facts.2;
        angle = angle.min(facts.3);
        steiner += report.segment_steiner + report.facet_steiner;
        cells += report.interior_cells;
        worst_err = worst_err.max(report.volume_error_relative);
    }
    row(
        "conform",
        &format!(
            "bodies {} graded {graded_bodies} steiner {steiner} cells {cells} volume_error {worst_err:.3e}",
            models.len()
        ),
        faces,
        nv,
        nt,
        vol,
        angle,
    )
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
            Some("step" | "stp")
        ) {
            out.push(p);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let flag = |args: &mut Vec<String>, name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        args.remove(i);
        Some(args.remove(i))
    };
    let conform_s: f64 = flag(&mut args, "--conform-timeout").map_or(60., |v| v.parse().unwrap());
    let graded_fallback = args.iter().position(|a| a == "--graded").map(|i| args.remove(i)).is_some();
    let foreign = args.iter().position(|a| a == "--foreign").map(|i| args.remove(i)).is_some();
    if let Some(file) = flag(&mut args, "--one") {
        println!("{}", survey(Path::new(&file), Duration::from_secs_f64(conform_s), foreign, graded_fallback));
        return;
    }
    let jobs: usize = flag(&mut args, "--jobs").map_or(1, |v| v.parse().unwrap());
    let limit: usize = flag(&mut args, "--limit").map_or(usize::MAX, |v| v.parse().unwrap());
    let timeout: f64 = flag(&mut args, "--timeout").map_or(180., |v| v.parse().unwrap());
    let (dir, out) = match args.as_slice() {
        [d, o] => (PathBuf::from(d), PathBuf::from(o)),
        _ => panic!("usage: abc-corpus DIR OUT.csv [--limit N] [--timeout S] [--conform-timeout S] [--jobs N] [--foreign] [--graded]"),
    };
    let mut files = Vec::new();
    collect(&dir, &mut files);
    files.truncate(limit);
    let exe = std::env::current_exe().unwrap();
    let csv = std::sync::Mutex::new(std::fs::File::create(&out).unwrap());
    writeln!(
        csv.lock().unwrap(),
        "file,bytes,stage,detail,faces,vertices,triangles,volume_mm3,min_angle_deg,seconds"
    )
    .unwrap();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(f) = files.get(i) else { return };
                let started = Instant::now();
                let mut command = Command::new(&exe);
                command
                    .arg("--one")
                    .arg(f)
                    .args(["--conform-timeout", &conform_s.to_string()])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                if foreign {
                    command.arg("--foreign");
                }
                if graded_fallback {
                    command.arg("--graded");
                }
                let mut child = command.spawn().unwrap();
                let mut stdout = child.stdout.take().unwrap();
                let reader = std::thread::spawn(move || {
                    let mut s = String::new();
                    let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
                    s
                });
                let status = loop {
                    if let Some(s) = child.try_wait().unwrap() {
                        break Some(s);
                    }
                    if started.elapsed().as_secs_f64() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                let body = reader.join().unwrap();
                let line = match status {
                    None => "timeout,,0,0,0,NaN,NaN".to_string(),
                    Some(s) if s.success() && !body.trim().is_empty() => body.trim().to_string(),
                    Some(s) => format!("crash,{},0,0,0,NaN,NaN", clean(&s.to_string())),
                };
                let name = f.file_name().unwrap().to_string_lossy();
                let bytes = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                let mut out = csv.lock().unwrap();
                writeln!(out, "{name},{bytes},{line},{:.2}", started.elapsed().as_secs_f64()).unwrap();
                out.flush().unwrap();
                eprintln!("[{}/{}] {name}: {}", i + 1, files.len(), line.split(',').next().unwrap());
            });
        }
    });
}
