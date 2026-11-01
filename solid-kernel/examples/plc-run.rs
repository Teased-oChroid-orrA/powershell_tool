//! Run `plc_conform::conform` on a shell dumped by the ABC harness (`ABC_DUMP_DIR=dir abc-corpus ...` writes `v x y z` /
//! `t a b c` lines per body): `plc-run FILE.plc`. `PLC_MERGE=1` sets `Options::merge_coplanar`; `PLC_REFINE=1` adds the sliver recipe (`PLC_QSEC` its clock, `PLC_PASSES` its flip passes). For offline debugging of
//! the conform stage of one failing shell.
use solid_kernel::plc_conform::{conform, Options};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut v, mut t) = (Vec::new(), Vec::new());
    for l in text.lines() {
        let x: Vec<&str> = l.split_whitespace().collect();
        match x[0] {
            "v" => v.push([x[1].parse::<f64>().unwrap(), x[2].parse().unwrap(), x[3].parse().unwrap()]),
            _ => t.push([x[1].parse::<usize>().unwrap(), x[2].parse().unwrap(), x[3].parse().unwrap()]),
        }
    }
    let o = Options { max_steiner: 200000, max_cells: 20_000_000, merge_coplanar: std::env::var_os("PLC_MERGE").is_some(), protect_encroached: std::env::var_os("PLC_NOPROTECT").is_none(), unprotected_fallback: std::env::var_os("PLC_NOFALLBACK").is_none(), ..Default::default() };
    // PLC_REFINE=1: the abc-corpus `--refine` sliver recipe (scaled Jacobian 0.2, six flip passes); PLC_QSEC sets its time
    let mut o = o;
    if std::env::var_os("PLC_REFINE").is_some() {
        o.min_scaled_jacobian = Some(0.2);
        o.max_quality_seconds = std::env::var("PLC_QSEC").ok().and_then(|s| s.parse().ok()).unwrap_or(20.);
        // PLC_PASSES: flip passes (0: no reconnection), as abc-corpus's ABC_PASSES
        let passes = std::env::var("PLC_PASSES").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
        o.reconnect = (passes > 0).then(|| fea_core::meshing::tet_flip::FlipOptions { max_passes: passes, ..Default::default() });
        // PLC_SMOOTH: smoothing rounds alternated with the flips, as abc-corpus's ABC_SMOOTH
        o.smooth_rounds = std::env::var("PLC_SMOOTH").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    }
    let secs: u64 = std::env::var("PLC_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(100);
    let start = std::time::Instant::now();
    let last = std::cell::Cell::new(0usize);
    let result = conform(&v, &t, o, |p| {
        last.set(p.steiner);
        // read once: the callback runs for every split, and reading the environment there took a quarter of the run
        start.elapsed().as_secs() < secs
    });
    println!("last progress steiner {}", last.get());
    match result {
        Ok(c) => {
            println!("ok {:?} secs {:.2}", c.report, start.elapsed().as_secs_f64());
            // cells under the solver gate's scaled Jacobian (0.1), by how many vertices lie on the boundary, and by how many
            // faces are sub-facets
            let mut on = vec![false; c.points.len()];
            let mut sub = std::collections::HashSet::new();
            for (f, _) in &c.facets {
                f.iter().for_each(|&v| on[v] = true);
                let mut k = *f;
                k.sort_unstable();
                sub.insert(k);
            }
            let (mut by_vertices, mut by_faces) = ([0usize; 5], [0usize; 5]);
            for t in &c.cells {
                let q = fea_core::meshing::quality::element_quality(fea_core::ElementKind::Tet4, &t.map(|v| c.points[v]));
                if q.scaled_jacobian < 0.1 {
                    by_vertices[t.iter().filter(|&&v| on[v]).count()] += 1;
                    let [a, b, d, e] = *t;
                    by_faces[[[a, b, d], [a, b, e], [a, d, e], [b, d, e]]
                        .iter()
                        .filter(|f| {
                            let mut k = **f;
                            k.sort_unstable();
                            sub.contains(&k)
                        })
                        .count()] += 1;
                }
            }
            println!("under 0.1 by boundary vertices {by_vertices:?} by sub-facet faces {by_faces:?}");
            // PLC_CELLS=1: each such cell, its vertices by kind (I input, S on a segment, F on a facet, Q interior), its edge
            // lengths and the height of each vertex over the opposite face, relative to the longest edge
            if std::env::var_os("PLC_CELLS").is_some() {
                let mut on_seg = vec![false; c.points.len()];
                c.segments.iter().flatten().for_each(|&v| on_seg[v] = true);
                let kind = |v: usize| if v < c.report.input_vertices { 'I' } else if on_seg[v] { 'S' } else if on[v] { 'F' } else { 'Q' };
                let d = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                for t in &c.cells {
                    let p = t.map(|v| c.points[v]);
                    let q = fea_core::meshing::quality::element_quality(fea_core::ElementKind::Tet4, &p);
                    if q.scaled_jacobian >= 0.1 {
                        continue;
                    }
                    let edges: Vec<f64> = (0..4).flat_map(|i| ((i + 1)..4).map(move |j| (i, j))).map(|(i, j)| d(p[i], p[j])).collect();
                    let longest = edges.iter().cloned().fold(0., f64::max);
                    // height of vertex i over the plane of the other three
                    let heights: Vec<f64> = (0..4)
                        .map(|i| {
                            let [a, b, e] = [0, 1, 2, 3].into_iter().filter(|&k| k != i).collect::<Vec<_>>()[..] else { unreachable!() };
                            let (u, w) = ([0, 1, 2].map(|k| p[b][k] - p[a][k]), [0, 1, 2].map(|k| p[e][k] - p[a][k]));
                            let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                            let nn = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                            ((0..3).map(|k| (p[i][k] - p[a][k]) * n[k]).sum::<f64>() / nn).abs() / longest
                        })
                        .collect();
                    println!(
                        "cell sj {:.4} kinds {} at {:.4?} longest {longest:.4e} edges/longest {:.3?} heights/longest {:.3?}",
                        q.scaled_jacobian,
                        t.map(kind).iter().collect::<String>(),
                        p[0],
                        edges.iter().map(|e| e / longest).collect::<Vec<_>>(),
                        heights
                    );
                }
            }
        }
        Err(e) => println!("err {e} secs {:.2}", start.elapsed().as_secs_f64()),
    }
}
