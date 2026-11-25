use edge_check::models::contact_model::flow_stress;
use edge_check::types::Strengths;
use lug_solver::bracket::*;

fn case(ed_edge: f64, around: usize) -> Case {
    let m = mechanics_core::materials::get_material("al7075");
    let s = Strengths::from_material(m);
    let d = 0.25;
    let reach = (3.0 * ed_edge).max(5.0 * d);
    Case { dia: d, pin_clearance: 0.002, pin_mu: 0.12, ed: ed_edge / d, td: 0.5, width: 2.0 * reach, length: reach, around, housing: Housing { e: s.e, nu: s.nu, flow: flow_stress(&s), law: None }, bush: None }
}

#[test]
#[ignore]
fn probe() {
    for (edge, ideal) in [(0.5, Idealisation::Strain), (0.375, Idealisation::Strain), (0.5, Idealisation::Stress)] {
        let out = try_run(case(edge, 36), ideal, None).unwrap();
        println!("edge {edge} collapse {:.0} dofs {} {:.1}s", out.at_limit(), out.dofs, out.secs);
        match &out.shear {
            Ok(p) => {
                let fsu = Strengths::from_material(mechanics_core::materials::get_material("al7075")).fsu;
                println!("  path start {:.1} incline {:.1} len {:.3} peak ep {:.3} wall {:?} edge {:?}", p.start_deg, p.incline_deg, p.length, p.peak_plastic, p.wall, p.edge_pt);
                println!("  mean shear at end {:.0} psi (Fsu {fsu:.0}); load at Fsu {:?}", p.mean_at_end(), p.load_at(fsu));
                for (k, (l, t)) in p.curve.iter().enumerate().filter(|(k, _)| k % 4 == 0) { println!("    step {k}: load {l:.0} mean {t:.0}"); }
            }
            Err(e) => println!("  no path: {e}"),
        }
    }
}

#[test]
#[ignore]
fn probe3d() {
    let out = try_run(case(0.5, 36), Idealisation::Solid { layers: 2, lock_z: false }, None).unwrap();
    println!("3D collapse {:.0} dofs {} {:.1}s", out.at_limit(), out.dofs, out.secs);
    match &out.shear {
        Ok(p) => {
            println!("  path start {:.1} incline {:.1} len {:.3} peak ep {:.3}", p.start_deg, p.incline_deg, p.length, p.peak_plastic);
            println!("  mean shear at end {:.0} psi; load at Fsu {:?}", p.mean_at_end(), p.load_at(48000.0));
        }
        Err(e) => println!("  no path: {e}"),
    }
}
