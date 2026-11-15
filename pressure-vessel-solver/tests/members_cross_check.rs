//! Independent check of `line_run`'s member actions against the general 3-D frame solver of
//! `fea-problem::members` (Euler-Bernoulli sections, no shear deformation, so the two are the same
//! theory solved by different code): a three-span line with an overhang, distributed load in both
//! transverse planes, point loads at interior nodes and an axial anchor. Reactions must agree.
use fea_problem::members::{Edit, MemberKind, MemberProblem, SectionSpec};
use pressure_vessel_solver::line_run::*;

#[test]
fn line_run_reactions_match_the_frame_solver() {
    let tube = Tube { outer_diameter: 1.0, wall: 0.05, e: 10.0e6, alpha: 12.8e-6, weight_density: 0.0 };
    let xs = [0.0, 20.0, 60.0, 150.0, 190.0, 220.0, 250.0]; // supports at 0, 60, 150, 220; loads at 20, 190; free end 250
    let supports = [0usize, 2, 3, 5];
    let (qy, qz) = (0.7, -1.3);
    let (f20, f190) = ([0.0, 25.0, -40.0], [0.0, -15.0, 30.0]);

    // fea-problem frame model.
    let inertia = {
        let g = tube.geometry().unwrap();
        std::f64::consts::PI / 4.0 * (g.outer_radius.powi(4) - g.inner_radius.powi(4))
    };
    let area = tube.area();
    let section = SectionSpec { e: tube.e, g: tube.e / 2.66, area, iy: inertia, iz: inertia, torsion: 2.0 * inertia, shear_areas: None };
    let mut p = MemberProblem::new("line");
    let ids: Vec<u32> = xs.iter().map(|&x| p.apply(Edit::AddNode { pos: [x, 0.0, 0.0] }).unwrap().unwrap()).collect();
    for w in ids.windows(2) {
        p.apply(Edit::AddMember { a: w[0], b: w[1], kind: MemberKind::Frame { section, reference: [0.0, 1.0, 0.0], uniform_load: [0.0, qy, qz] } }).unwrap();
    }
    for &i in &supports {
        let fixed = if i == 0 { [true, true, true, true, false, false] } else { [false, true, true, false, false, false] };
        p.apply(Edit::SetSupport { node: ids[i], fixed }).unwrap();
    }
    p.apply(Edit::SetLoad { node: ids[1], force: f20, moment: [0.0; 3] }).unwrap();
    p.apply(Edit::SetLoad { node: ids[4], force: f190, moment: [0.0; 3] }).unwrap();
    let frame = p.solve().unwrap();

    // line_run: contents weight w with load factors (0, qy/w, qz/w) gives the same distributed load.
    let w = 2.0;
    let run = LineRun {
        tube,
        x_start: 0.0,
        x_end: 250.0,
        supports: supports.iter().map(|&i| Support { x: xs[i], axial_anchor: i == 0, rotation_fixed: false }).collect(),
        contents_weight_per_length: w,
        insulation_weight_per_length: 0.0,
        point_loads: vec![PointLoad { x: 20.0, force: f20, ..Default::default() }, PointLoad { x: 190.0, force: f190, ..Default::default() }],
        subdivisions: 2,
    };
    let class = MaterialClass::Ductile { yield_strength: 40000.0, ultimate_strength: 60000.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.5 };
    let case = LoadCase { name: "x".into(), internal_pressure: 0.0, external_pressure: 0.0, pressure_end_load: false, delta_t: 0.0, load_factors: [0.0, qy / w, qz / w], point_loads: vec![] };
    let res = analyze(&run, &class, &[case]).unwrap();
    for &i in &supports {
        let mine = res.cases[0].reactions.iter().find(|r| (r.x - xs[i]).abs() < 1e-9).unwrap();
        let theirs = frame.reaction[i];
        for k in 1..3 {
            let (a, b) = (mine.force[k], theirs[k]);
            assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "support x={} dir {k}: line_run {a} vs frame solver {b}", xs[i]);
        }
    }
}
