use fea_problem::{solve, templates};
#[test]
#[ignore]
fn scratch() {
    let base = templates::templates().into_iter().find(|(n, _)| n.contains("bushing")).unwrap().1;
    let mut cases = Vec::new();
    for e_bush in [29e6, 1e6, 1e4] {
        for interf in [0.002, 0.09] {
            for mu in [0.15, 1.9] {
                cases.push((format!("E={e_bush:e} int={interf} mu={mu}"), interf, mu, e_bush));
            }
        }
    }
    for (label, interf, mu, e_bush) in cases {
        let mut p = base.clone();
        p.mesh.size = 0.4;
        p.bushings[0].interference = interf;
        p.bushings[0].friction = mu;
        p.bushings[0].material.e = e_bush;
        let t = std::time::Instant::now();
        match solve(&p, None) {
            Ok(_) => eprintln!("{label}: ok in {:.1}s", t.elapsed().as_secs_f64()),
            Err(e) => eprintln!("{label}: ERR ({:.1}s) {}", t.elapsed().as_secs_f64(), &e[..e.len().min(160)]),
        }
    }
}
