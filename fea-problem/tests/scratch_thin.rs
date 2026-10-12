use fea_problem::{solve, templates, Geometry, Shape};
#[test]
#[ignore]
fn scratch() {
    let base = templates::templates().remove(0).1;
    let cases: Vec<(&str, Shape)> = vec![
        ("tiny hole r=0.001", Shape::Circle { cx: 6.0, cy: 2.0, r: 0.001 }),
        ("tiny hole r=1e-6", Shape::Circle { cx: 6.0, cy: 2.0, r: 1e-6 }),
        ("hair slot", Shape::Slot { cx: 6.0, cy: 2.0, length: 2.0, width: 1e-3, angle: 0.0 }),
        ("hair slot 1e-5", Shape::Slot { cx: 6.0, cy: 2.0, length: 2.0, width: 1e-5, angle: 0.0 }),
    ];
    for (label, hole) in cases {
        let mut p = base.clone();
        if let Geometry::Sketch { holes, .. } = &mut p.geometry {
            *holes = vec![hole];
        }
        let t = std::time::Instant::now();
        match solve(&p, None) {
            Ok(s) => eprintln!("{label}: ok {} nodes in {:.1}s", s.summary.nodes, t.elapsed().as_secs_f64()),
            Err(e) => eprintln!("{label}: ERR {}", &e[..e.len().min(200)]),
        }
    }
    // A sheet thinner than the element size.
    let mut p = base.clone();
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 0.01 }, holes: vec![], depth: 1.0, layers: 1, regions: vec![] };
    p.supports = vec![fea_problem::Support::fixed("left")];
    p.loads = vec![fea_problem::Load::Traction { edge: "right".into(), tx: 1.0, ty: 0.0, tz: 0.0 }];
    match solve(&p, None) {
        Ok(s) => eprintln!("thin sheet: ok {} nodes", s.summary.nodes),
        Err(e) => eprintln!("thin sheet: ERR {}", &e[..e.len().min(200)]),
    }
}
