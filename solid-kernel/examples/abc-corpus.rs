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
//! `--merge` conforms with `Options::merge_coplanar` (coplanar input triangles become one facet).
//! `--quality` also hands every conforming body to the solver gate (`Conforming::to_solver_mesh`: fea-core's
//! `QualityThresholds::default()` and conformity) and, for a passing body, runs a static solve (the end of the longest axis
//! fixed, a unit force across the other end); the detail then carries `gate=pass|fail`, `path=protected|fallback`, the
//! worst scaled Jacobian `sj`, the largest edge ratio `er`, `tets`, the gate refusal and the solve outcome.
//! `--refine` (with `--quality`) conforms with the sliver recipe: `Options::min_scaled_jacobian` 0.2, six passes of the flip pass (`ABC_PASSES`, 0 for none). The experiments' variables `ABC_RADIUS_EDGE`, `ABC_MIN_SJ`, `ABC_SMOOTH`, `ABC_RECONNECT`, `ABC_PLANAR`
//! override single options; `ABC_SHOW_BAD` lists the tetrahedra under the gate; `ABC_NOFALLBACK` reports the protected
//! recovery's refusal instead of falling back to unprotected recovery.
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
        .map(|c| {
            if c == ',' || c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .take(400)
        .collect()
}

/// One file, all stages; returns `stage,detail,faces,vertices,triangles,volume,min_angle`.
/// Quality of one conforming body against the solver's gate; quality statistics are of the mesh even when the gate refuses it.
struct BodyQuality {
    pass: bool,
    fallback: bool,
    min_sj: f64,
    max_er: f64,
    tets: usize,
    /// Tetrahedra under the gate's scaled Jacobian, and those among them with all four vertices on the boundary.
    bad: usize,
    bad_flat_boundary: usize,
    refusal: String,
    solve: String,
}

fn body_quality(c: &solid_kernel::plc_conform::Conforming) -> BodyQuality {
    use fea_core::{Elastic, Mesh, Physics};
    let material = Elastic {
        e: 200e3,
        nu: 0.3,
        alpha: 0.,
        aniso: None,
    };
    let mut mesh = Mesh::new(Physics::Solid);
    mesh.nodes = c.points.clone();
    let mut on_boundary = vec![false; c.points.len()];
    for (f, _) in &c.facets {
        for &v in f {
            on_boundary[v] = true;
        }
    }
    let (mut bad, mut bad_flat_boundary) = (0usize, 0usize);
    let stats = match mesh.add_block(
        fea_core::ElementKind::Tet4,
        c.cells.iter().flatten().copied().collect(),
        material,
        "t",
    ) {
        Ok(()) => {
            let q = mesh.quality();
            for (t, e) in c.cells.iter().zip(&q.elements) {
                if e.scaled_jacobian < 0.1 {
                    bad += 1;
                    if std::env::var_os("ABC_SHOW_BAD").is_some() {
                        eprintln!(
                            "bad tet sj {:.2e} er {:.1}: {:?}",
                            e.scaled_jacobian,
                            e.edge_ratio,
                            t.map(|v| (v, on_boundary[v], c.points[v]))
                        );
                    }
                    bad_flat_boundary += t.iter().all(|&v| on_boundary[v]) as usize;
                }
            }
            (q.min_scaled_jacobian, q.max_edge_ratio)
        }
        Err(_) => (f64::NAN, f64::NAN),
    };
    let mut out = BodyQuality {
        pass: false,
        fallback: c.report.unprotected_fallback_used,
        min_sj: stats.0,
        max_er: stats.1,
        tets: c.cells.len(),
        bad,
        bad_flat_boundary,
        refusal: String::new(),
        solve: "skip".into(),
    };
    let solver = match c.to_solver_mesh(material) {
        Ok(m) => m,
        Err(e) => {
            out.refusal = e.to_string();
            return out;
        }
    };
    out.pass = true;
    // static solve: fix the nodes within 2% of the extent of the minimum end of the longest axis, push the maximum end sideways
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &solver.nodes {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let axis = (0..3)
        .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
        .unwrap();
    let side = (axis + 1) % 3;
    let ext = hi[axis] - lo[axis];
    let mut bc = fea_core::Dirichlet::new(solver.nodes.len(), 3);
    let mut loads = fea_core::Loads::default();
    let tol = 0.02 * ext;
    let mut free_end = Vec::new();
    for (i, p) in solver.nodes.iter().enumerate() {
        if p[axis] <= lo[axis] + tol {
            bc.fix_node(i);
        } else if p[axis] >= hi[axis] - tol {
            free_end.push(i);
        }
    }
    for &i in &free_end {
        let mut f = [0.; 3];
        f[side] = 1. / free_end.len() as f64;
        loads.nodal.push((i, f));
    }
    let started = Instant::now();
    out.solve = match fea_core::Model::new(solver).and_then(|m| m.solve_static(&loads, &bc)) {
        Ok(s)
            if s.rel_residual <= 1e-6
                && s.u.iter().all(|x| x.is_finite())
                && s.u.iter().any(|x| *x != 0.) =>
        {
            format!(
                "ok res {:.1e} {:.2}s",
                s.rel_residual,
                started.elapsed().as_secs_f64()
            )
        }
        Ok(s) => format!("bad res {:.1e}", s.rel_residual),
        Err(e) => format!("err {e}"),
    };
    out
}

fn survey(
    path: &Path,
    conform_timeout: Duration,
    foreign: bool,
    graded_fallback: bool,
    steiner_budget: Option<usize>,
    merge: bool,
    quality: bool,
    refine: bool,
) -> String {
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
    let mut options = if foreign {
        Options::foreign_cad(0.05)
    } else {
        Options::default()
    };
    // ABC_TOL_MM: edge/surface tolerance override (mm), for measuring how much deviation a file needs; ABC_MODEL_ONLY: stop
    // after reading the model (stage `model-ok`)
    if let Some(t) = std::env::var("ABC_TOL_MM")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        options.geometry_tolerance_mm = t;
    }
    let models = match AnalyticModel::read_all(&data, options) {
        Ok(m) => m,
        Err(e) => return row("model", &e.to_string(), 0, 0, 0, f64::NAN, f64::NAN),
    };
    if std::env::var_os("ABC_MODEL_ONLY").is_some() {
        return row(
            "model-ok",
            &format!("bodies {}", models.len()),
            0,
            0,
            0,
            f64::NAN,
            f64::NAN,
        );
    }
    // A multi-body part succeeds only when every body does; a failure names its body. Each body is meshed from plain
    // per-face sampling first; when that stage fails, from the graded model (`AnalyticModel::graded`), the production path.
    let (mut faces, mut nv, mut nt, mut vol, mut angle) = (0, 0, 0, 0., f64::INFINITY);
    let (mut steiner, mut cells, mut worst_err, mut graded_bodies) = (0usize, 0usize, 0f64, 0usize);
    let (mut quality_steiner, mut unresolved) = (0usize, 0usize);
    let mut qualities = Vec::new();
    let limits = solid_kernel::native_surface::Limits {
        face_work: 20_000_000,
        // an admission bound (faces x face_work), not accounting: sized to the part
        total_work: models
            .iter()
            .map(|m| m.body().faces.len())
            .max()
            .unwrap_or(1)
            .saturating_mul(20_000_000),
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
            // ABC_DUMP_DIR: write each sampled shell as `v x y z` / `t a b c` lines for offline debugging of the conform stage
            if let Some(dir) = std::env::var_os("ABC_DUMP_DIR") {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("shell");
                let mut text = String::new();
                for v in &m.vertices {
                    text += &format!("v {:e} {:e} {:e}\n", v[0], v[1], v[2]);
                }
                for t in &m.triangles {
                    text += &format!("t {} {} {}\n", t[0], t[1], t[2]);
                }
                let _ = std::fs::write(Path::new(&dir).join(format!("{name}.body{k}.plc")), text);
            }
            let a = solid_kernel::plc_conform::min_shell_angle_deg(&m.vertices, &m.triangles);
            let facts = (
                m.vertices.len(),
                m.triangles.len(),
                sampled.signed_volume_mm3,
                a,
            );
            let start = Instant::now();
            solid_kernel::plc_conform::conform(
                &m.vertices,
                &m.triangles,
                {
                    let mut o = match steiner_budget {
                        Some(n) => solid_kernel::plc_conform::Options {
                            max_steiner: n,
                            max_cells: n.saturating_mul(100),
                            merge_coplanar: merge,
                            ..Default::default()
                        },
                        None => solid_kernel::plc_conform::Options {
                            merge_coplanar: merge,
                            ..Default::default()
                        },
                    };
                    // --refine: the sliver recipe (scaled Jacobian 0.2 by refinement, then flips and smoothing)
                    if refine {
                        o.min_scaled_jacobian = Some(0.2);
                        o.max_quality_seconds = std::env::var("ABC_QSEC")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(20.);
                        let passes = std::env::var("ABC_PASSES")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(6);
                        if passes > 0 {
                            o.reconnect = Some(fea_core::meshing::tet_flip::FlipOptions {
                                max_passes: passes,
                                ..Default::default()
                            });
                        }
                        o.smooth_rounds = 0;
                    }
                    // ABC_RADIUS_EDGE: Options::max_radius_edge; ABC_RECONNECT=1: flip/edge-removal pass; ABC_PLANAR=1: with it
                    // also reconnect flat boundary patches (experiments on the quality gate)
                    if let Some(r) = std::env::var("ABC_RADIUS_EDGE")
                        .ok()
                        .and_then(|v| v.parse().ok())
                    {
                        o.max_radius_edge = Some(r);
                    }
                    if let Some(r) = std::env::var("ABC_MIN_SJ")
                        .ok()
                        .and_then(|v| v.parse().ok())
                    {
                        o.min_scaled_jacobian = Some(r);
                    }
                    if let Some(r) = std::env::var("ABC_SMOOTH")
                        .ok()
                        .and_then(|v| v.parse().ok())
                    {
                        o.smooth_rounds = r;
                    }
                    // ABC_NOFALLBACK: report the protected recovery's own refusal instead of falling back to unprotected recovery
                    if std::env::var_os("ABC_NOFALLBACK").is_some() {
                        o.unprotected_fallback = false;
                    }
                    if std::env::var_os("ABC_RECONNECT").is_some() {
                        o.reconnect = Some(fea_core::meshing::tet_flip::FlipOptions {
                            planar_boundary: std::env::var_os("ABC_PLANAR").is_some(),
                            ..Default::default()
                        });
                    }
                    o
                },
                |_| start.elapsed() < conform_timeout,
            )
            .map(|c| {
                let q = quality.then(|| body_quality(&c));
                (facts, (c.report, q))
            })
            .map_err(|e| ("shell", e.to_string(), Some(facts)))
        };
        // ABC_GRADED_FIRST (with --graded): skip the plain per-face sampling and mesh from the graded model at once
        let plain = if std::env::var_os("ABC_GRADED_FIRST").is_some() {
            Err(("sample", "plain sampling skipped".to_string(), None))
        } else {
            attempt(model, limits)
        };
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
                    // In graded-first diagnostics there is no plain failure to report.
                    Err(graded_failure) => {
                        let (stage, detail, f) = if std::env::var_os("ABC_GRADED_FIRST").is_some() {
                            graded_failure
                        } else {
                            first
                        };
                        let f = f.unwrap_or((nv, nt, vol, angle));
                        return row(stage, &tag(&detail), faces, f.0, f.1, f.2, f.3);
                    }
                }
            }
        };
        let (report, body_q) = report;
        qualities.extend(body_q);
        graded_bodies += was_graded as usize;
        nv += facts.0;
        nt += facts.1;
        vol += facts.2;
        angle = angle.min(facts.3);
        steiner += report.segment_steiner + report.facet_steiner;
        cells += report.interior_cells;
        worst_err = worst_err.max(report.volume_error_relative);
        quality_steiner += report.quality_steiner;
        unresolved += report.unresolved_bad_cells;
    }
    let mut detail = format!(
        "bodies {} graded {graded_bodies} steiner {steiner} quality_steiner {quality_steiner} unresolved {unresolved} cells {cells} volume_error {worst_err:.3e}",
        models.len()
    );
    if quality {
        let pass = qualities.iter().all(|q| q.pass);
        let fallback = qualities.iter().any(|q| q.fallback);
        let sj = qualities
            .iter()
            .map(|q| q.min_sj)
            .fold(f64::INFINITY, f64::min);
        let er = qualities.iter().map(|q| q.max_er).fold(0., f64::max);
        let tets: usize = qualities.iter().map(|q| q.tets).sum();
        let refusal = qualities
            .iter()
            .map(|q| q.refusal.as_str())
            .find(|r| !r.is_empty())
            .unwrap_or("");
        let solve = qualities
            .iter()
            .map(|q| q.solve.as_str())
            .find(|r| !r.starts_with("ok"))
            .unwrap_or("ok");
        let bad: usize = qualities.iter().map(|q| q.bad).sum();
        let bad_b: usize = qualities.iter().map(|q| q.bad_flat_boundary).sum();
        detail += &format!(" bad={bad} badb={bad_b}");
        detail += &format!(
            " gate={} path={} sj={sj:.4} er={er:.1} tets={tets} solve={solve} refusal={refusal}",
            if pass { "pass" } else { "fail" },
            if fallback { "fallback" } else { "protected" }
        );
    }
    row("conform", &detail, faces, nv, nt, vol, angle)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
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
    let steiner: Option<usize> = flag(&mut args, "--steiner").map(|v| v.parse().unwrap());
    let graded_fallback = args
        .iter()
        .position(|a| a == "--graded")
        .map(|i| args.remove(i))
        .is_some();
    let foreign = args
        .iter()
        .position(|a| a == "--foreign")
        .map(|i| args.remove(i))
        .is_some();
    let merge = args
        .iter()
        .position(|a| a == "--merge")
        .map(|i| args.remove(i))
        .is_some();
    let quality = args
        .iter()
        .position(|a| a == "--quality")
        .map(|i| args.remove(i))
        .is_some();
    let refine = args
        .iter()
        .position(|a| a == "--refine")
        .map(|i| args.remove(i))
        .is_some();
    if let Some(file) = flag(&mut args, "--one") {
        println!(
            "{}",
            survey(
                Path::new(&file),
                Duration::from_secs_f64(conform_s),
                foreign,
                graded_fallback,
                steiner,
                merge,
                quality,
                refine
            )
        );
        return;
    }
    let jobs: usize = flag(&mut args, "--jobs").map_or(1, |v| v.parse().unwrap());
    let limit: usize = flag(&mut args, "--limit").map_or(usize::MAX, |v| v.parse().unwrap());
    let timeout: f64 = flag(&mut args, "--timeout").map_or(180., |v| v.parse().unwrap());
    let (dir, out) = match args.as_slice() {
        [d, o] => (PathBuf::from(d), PathBuf::from(o)),
        _ => panic!("usage: abc-corpus DIR OUT.csv [--limit N] [--timeout S] [--conform-timeout S] [--jobs N] [--foreign] [--graded] [--steiner N] [--merge] [--quality] [--refine]"),
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
                if merge {
                    command.arg("--merge");
                }
                if quality {
                    command.arg("--quality");
                }
                if refine {
                    command.arg("--refine");
                }
                if foreign {
                    command.arg("--foreign");
                }
                if graded_fallback {
                    command.arg("--graded");
                }
                if let Some(n) = steiner {
                    command.args(["--steiner", &n.to_string()]);
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
                writeln!(
                    out,
                    "{name},{bytes},{line},{:.2}",
                    started.elapsed().as_secs_f64()
                )
                .unwrap();
                out.flush().unwrap();
                eprintln!(
                    "[{}/{}] {name}: {}",
                    i + 1,
                    files.len(),
                    line.split(',').next().unwrap()
                );
            });
        }
    });
}
