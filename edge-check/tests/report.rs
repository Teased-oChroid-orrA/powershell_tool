use edge_check::models::default_models;
use edge_check::runner::{run, EdgeConfig, EdgeInput, EdgeMin};
use edge_check::types::{Geometry, Strengths};
use mechanics_core::materials::get_material;

fn input(ed: f64, load: f64, fit: f64) -> EdgeInput {
    let a = 0.25;
    let e = ed * 2.0 * a;
    EdgeInput {
        geom: Geometry { bore_radius: a, edge: e, thickness: 0.5, plate_far: (3.0 * e).max(10.0 * a), plate_half_height: (3.0 * e).max(10.0 * a), plane_angle_deg: 40.0 },
        strengths: Strengths::from_material(get_material("al7075")),
        applied_load: load,
        fit_pressure: fit,
        fit_pressure_min: fit * 0.6,
        fit_pressure_max: fit * 1.4,
    }
}

#[test]
fn prints_a_full_report() {
    let cfg = EdgeConfig { include_plastic: true, ..EdgeConfig::default() };
    let models = default_models(&cfg);
    for (ed, load, fit) in [(1.5, 1000.0, 5000.0), (2.0, 15000.0, 5000.0)] {
        let t0 = std::time::Instant::now();
        let rep = run(&models, &input(ed, load, fit), &cfg);
        println!("=== e/D={ed} load={load} fit={fit} total {:?}", t0.elapsed());
        for m in &rep.models {
            println!("  [{}] {:?} err={:?} notes={:?}", m.label, m.elapsed, m.error, m.notes);
            for t in m.targets.iter().flatten() {
                let em = match t.e_min {
                    EdgeMin::Value(v) => format!("{:.3} (e/D {:.3})", v, v / rep.bore_diameter),
                    EdgeMin::AtMost(v) => format!("<= {:.3} (e/D {:.2})", v, v / rep.bore_diameter),
                    EdgeMin::Exceeds(v) => format!("> {:.3}", v),
                    EdgeMin::NotSearched => "-".into(),
                };
                println!("     {:<34} margin {:>8.3} ({}) nofit {:>8.3} e_min {} mc {:?}", t.target.label, t.margin, t.governing.label(), t.margin_no_fit, em, t.mc.as_ref().map(|s| (s.p_fail, (s.p05 * 100.0).round() / 100.0)));
            }
        }
    }
}
