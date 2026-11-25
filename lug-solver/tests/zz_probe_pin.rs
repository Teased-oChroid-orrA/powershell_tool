use edge_check::models::contact_model::flow_stress;
use edge_check::types::Strengths;
use fea_core::nonlinear::Interrupt;
use lug_solver::bracket::*;
use std::time::{Duration, Instant};

fn case(delta: f64, edge: f64, around: usize) -> Case {
    let m = mechanics_core::materials::get_material("al7075");
    let s = Strengths::from_material(m);
    let pin = 0.25;
    let d = pin - delta;
    let reach = (3.0 * edge).max(5.0 * d);
    let bush = Some(Bush { inner_dia: 0.4 * d, e: 29.0e6, nu: 0.32, interference_dia: delta, mu: 0.12 });
    Case { dia: d, pin_clearance: 0.0, pin_mu: 0.12, ed: edge / d, td: 0.125 / d, width: 2.0 * reach, length: reach, around, housing: Housing { e: s.e, nu: s.nu, flow: flow_stress(&s), law: None }, bush }
}

#[test]
#[ignore]
fn probe() {
    let which = std::env::var("PROBE").unwrap_or_default();
    let secs: u64 = std::env::var("SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    for (name, ideal) in [("strain", Idealisation::Strain), ("stress", Idealisation::Stress)] {
        let mut c = case(0.0, 0.5, 36);
        c.bush = None;
        let o = try_run_with(c, ideal, Interrupt::default()).unwrap();
        println!("line-to-line rigid pin {name}: collapse {:.0}", o.at_limit());
    }
    for delta in [1e-5, 0.0008, 0.003] {
        for (name, ideal) in [("strain", Idealisation::Strain), ("stress", Idealisation::Stress)] {
            if !which.is_empty() && !which.contains(name) { continue; }
            let t = Instant::now();
            let r = try_run_with(case(delta, 0.5, 36), ideal, Interrupt { deadline: Some(Instant::now() + Duration::from_secs(secs)), cancel: None });
            match r {
                Ok(o) => println!("delta {delta} {name}: collapse {:.0} lbf fit {:?} psi dofs {} {:.1}s shear {:?}", o.at_limit(), o.fit_pressure, o.dofs, t.elapsed().as_secs_f64(), o.shear.as_ref().map(|p| (p.start_deg, p.incline_deg, p.mean_at_end())).map_err(|e| e.clone())),
                Err(e) => println!("delta {delta} {name}: FAILED after {:.1}s: {e}", t.elapsed().as_secs_f64()),
            }
        }
    }
}

#[test]
#[ignore]
fn probe3d() {
    let delta: f64 = std::env::var("DELTA").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0008);
    let t = Instant::now();
    let r = try_run_with(case(delta, 0.5, 36), Idealisation::Solid { layers: 2, lock_z: false }, Interrupt { deadline: Some(Instant::now() + Duration::from_secs(1500)), cancel: None });
    match r {
        Ok(o) => println!("3D delta {delta}: collapse {:.0} lbf fit {:?} psi dofs {} {:.1}s shear {:?}", o.at_limit(), o.fit_pressure, o.dofs, t.elapsed().as_secs_f64(), o.shear.as_ref().map(|p| p.mean_at_end()).map_err(|e| e.clone())),
        Err(e) => println!("3D delta {delta}: FAILED after {:.1}s: {e}", t.elapsed().as_secs_f64()),
    }
}
