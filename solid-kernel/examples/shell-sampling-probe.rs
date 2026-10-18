//! Where the thin triangles of a natively sampled shell come from: per source face, the triangle count, the smallest corner
//! angle, the longest edge and the face's bounding extent. `cargo run --release -p solid-kernel --example shell-sampling-probe -- FILE.step`
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::step::{inventory, Limits};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: shell-sampling-probe FILE.step [--line MM] [--quality DEG] [--points N]");
    let (mut line, mut quality, mut points, mut grade, mut floor, mut canonical) =
        (None, None, 5000usize, None, 0.1, false);
    let mut rounds = 12usize;
    let mut tolerance = None;
    while let Some(a) = args.next() {
        let v: f64 = args.next().expect("value").parse().unwrap();
        match a.as_str() {
            "--line" => line = Some(v),
            "--quality" => quality = Some(v),
            "--points" => points = v as usize,
            "--grade" => grade = Some(v),
            "--floor" => floor = v,
            "--canonical" => canonical = v != 0.,
            "--rounds" => rounds = v as usize,
            "--tol" => tolerance = Some(v),
            _ => panic!("unknown flag"),
        }
    }
    let policy = if canonical {
        solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance
    } else {
        solid_kernel::cylindrical_step::ChartPolicy::Strict
    };
    let text = std::fs::read_to_string(path).unwrap();
    let data = inventory(&text, Limits::default()).unwrap();
    let model = AnalyticModel::read(
        &data,
        Options {
            max_line_mm: line,
            geometry_tolerance_mm: tolerance.unwrap_or(Options::default().geometry_tolerance_mm),
            ..Options::default()
        },
    )
    .unwrap();
    let model = match grade {
        Some(g) => {
            let (m, r) = model
                .graded(
                    solid_kernel::chart_grade::GradeOptions {
                        min_angle_deg: g,
                        floor_mm: floor,
                        max_rounds: rounds,
                        policy,
                        ..Default::default()
                    },
                    50_000_000,
                )
                .unwrap();
            println!("graded: {r:?}");
            m
        }
        None => model,
    };
    let s = model
        .sampled_surface(
            solid_kernel::native_surface::Limits {
                chart_policy: policy,
                face_work: 20_000_000,
                total_work: 800_000_000,
                chart_quality: quality.map(|q| solid_kernel::chart_mesh::ChartQuality {
                    min_angle_deg: q,
                    max_points: points,
                }),
                ..Default::default()
            },
            |_| true,
        )
        .unwrap();
    let m = &s.mesh;
    let mut rows = Vec::new();
    for tag in 0..s.source_faces.len() {
        let mut n = 0;
        let (mut min_angle, mut longest) = (180f64, 0f64);
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for (t, tri) in m.triangles.iter().enumerate() {
            if m.face[t] != tag {
                continue;
            }
            n += 1;
            let p = tri.map(|i| m.vertices[i]);
            for k in 0..3 {
                let (a, b, c) = (p[k], p[(k + 1) % 3], p[(k + 2) % 3]);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let (nu, nw) = (
                    (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt(),
                    (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt(),
                );
                longest = longest.max(nu);
                min_angle = min_angle.min(
                    ((u[0] * w[0] + u[1] * w[1] + u[2] * w[2]) / (nu * nw))
                        .clamp(-1., 1.)
                        .acos()
                        .to_degrees(),
                );
                for d in 0..3 {
                    lo[d] = lo[d].min(a[d]);
                    hi[d] = hi[d].max(a[d]);
                }
            }
        }
        rows.push((
            min_angle,
            tag,
            s.source_faces[tag],
            n,
            longest,
            [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        ));
    }
    let all = solid_kernel::plc_conform::min_shell_angle_deg(&m.vertices, &m.triangles);
    println!(
        "{} vertices, {} triangles, registry angle {all:.4} deg",
        m.vertices.len(),
        m.triangles.len()
    );
    let mut bad = std::collections::BTreeMap::<usize, (usize, f64)>::new();
    for (t, tri) in m.triangles.iter().enumerate() {
        let p = tri.map(|i| m.vertices[i]);
        let mut a_min = 180f64;
        for k in 0..3 {
            let (a, b, c) = (p[k], p[(k + 1) % 3], p[(k + 2) % 3]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let (nu, nw) = (
                (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt(),
                (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt(),
            );
            a_min = a_min.min(
                ((u[0] * w[0] + u[1] * w[1] + u[2] * w[2]) / (nu * nw))
                    .clamp(-1., 1.)
                    .acos()
                    .to_degrees(),
            );
        }
        if a_min < 18. {
            let e = bad.entry(m.face[t]).or_insert((0, 180.));
            e.0 += 1;
            e.1 = e.1.min(a_min);
            if std::env::var("SHOWBAD").is_ok() && e.0 <= 3 {
                println!("bad tri face {} angle {a_min:.3}: {p:?}", m.face[t]);
            }
        }
    }
    println!("triangles below 18 degrees per face tag (count, min): {bad:?}");
    if let Ok(w) = std::env::var("WINDOW") {
        let v: Vec<f64> = w.split(',').map(|x| x.parse().unwrap()).collect();
        for (t, tri) in m.triangles.iter().enumerate() {
            let p = tri.map(|i| m.vertices[i]);
            if p.iter()
                .all(|q| (0..3).all(|d| q[d] >= v[2 * d] && q[d] <= v[2 * d + 1]))
            {
                println!("tri face {} {:?}", m.face[t], p);
            }
        }
    }
    if let Ok(v) = std::env::var("DEFEATURE") {
        let v: Vec<f64> = v.split(',').map(|x| x.parse().unwrap()).collect();
        let mut sv = m.vertices.clone();
        let mut sf = m.triangles.clone();
        if v.len() > 3 && v[3] > 0. {
            let w =
                solid_kernel::shell_weld::weld_short_edges(&sv, &sf, Default::default()).unwrap();
            println!("weld: {:?}", w.report);
            sv = w.vertices;
            sf = w.facets;
        }
        let o = solid_kernel::shell_defeature::Options {
            max_edge_relative: v[0],
            bad_angle_deg: v[1],
            acceptable_angle_deg: v[2],
            max_rounds: 6,
        };
        match solid_kernel::shell_defeature::defeature_slivers(&sv, &sf, o) {
            Ok(d) => println!(
                "defeature: {:?}\nangle after {:.4}",
                d.report,
                solid_kernel::plc_conform::min_shell_angle_deg(&d.vertices, &d.facets)
            ),
            Err(e) => println!("defeature error: {e}"),
        }
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (a, tag, id, n, longest, ext) in rows.iter().take(14) {
        println!("min corner {a:8.4} deg  face tag {tag:2} (#{id}) {n:3} triangles, longest edge {longest:8.3}, extent {:.3} x {:.3} x {:.3}", ext[0], ext[1], ext[2]);
    }
}
