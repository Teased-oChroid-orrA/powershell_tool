//! One-off calibration of the finite-strain failure strain of every library material against the
//! material's own published bearing allowable (MIL-HDBK-5J `Fbru`, e/D 2.0, checked against e/D 1.5).
//!
//! For each condition with `Ftu`, `Fty`, `E`, Poisson's ratio, elongation and `Fbru`, the finite-strain
//! collapse of the standard bearing specimen (pin 0.5 in, plate 0.25 x 2.0 in, edge distance e/D)
//! is run once to the load peak; the equivalent plastic strain at the hole rises along the load, so
//! the strain whose collapse load equals `Fbru D t` is read from that one curve. The result is the
//! *effective bearing failure strain* of the model for that condition.
//!
//!   cargo run --release -p lug-solver --example calibrate_failure_strain > mechanics-core/data/failure-strain-calibration.jsonl
//!
//! Output: one JSON object per library entry (`index` into `builtin_catalog()`), see `tools/gen_fracture.py`.

use lug_solver::*;
use mechanics_core::materials::{builtin_catalog, Material as Lib};

const D: f64 = 0.5;
const T: f64 = 0.25;

/// Typical elongation (%) of the curated built-ins, which carry no elongation of their own
/// (from the handbook conditions they stand for; ASM Metals Handbook typical values).
fn curated_elongation(id: &str) -> Option<f64> {
    Some(match id {
        "al7075" | "al7075t6" => 11.0,
        "al2024" | "al2024t3" => 18.0,
        "al7050" => 10.0,
        "steel" => 12.0,
        "ti6al4v" => 10.0,
        "steel4340" => 11.0,
        "ph157mo" => 6.0,
        "ph174" => 12.0,
        "inconel718" => 12.0,
        "inconel625" => 40.0,
        "washer_steel" => 25.0,
        "washer_al" => 15.0,
        "bronze" => 15.0,
        "beryllium" => 4.0,
        _ => return None,
    })
}

fn elongation(m: &Lib) -> Option<f64> {
    if let Some(x) = m.extra {
        let v: Vec<f64> = [x.elong.l, x.elong.lt].into_iter().filter(|e| *e > 0.0).collect();
        if let Some(e) = v.iter().cloned().reduce(f64::min) {
            return Some(e);
        }
        if x.elong.st > 0.0 {
            return Some(x.elong.st);
        }
        return None;
    }
    curated_elongation(m.id)
}

/// `(strain, reachable)`: the failure strain whose collapse load equals `target` on the curve.
fn strain_for(curve: &[FsPoint], target: f64) -> (f64, &'static str) {
    let mut prev: Option<&FsPoint> = None;
    for p in curve {
        if p.load_lbf >= target {
            return match prev {
                Some(q) if p.load_lbf > q.load_lbf => (q.max_ep + (p.max_ep - q.max_ep) * (target - q.load_lbf) / (p.load_lbf - q.load_lbf), "ok"),
                _ => (p.max_ep, "below"),
            };
        }
        prev = Some(p);
    }
    (curve.last().map_or(0.0, |p| p.max_ep), "above")
}

fn curve_for(m: &Lib, e_u: f64, ed: f64) -> Result<Vec<FsPoint>, String> {
    let law = Hardening::true_curve(m.sy_ksi * 1000.0, (m.ftu_ksi * 1000.0).max(m.sy_ksi * 1000.0), e_u, 1.5)?;
    let g = LugGeometry { hole_dia: D, width: 2.0, edge: ed * D, length: 5.0, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let fl = FiniteLug::build(&g, MeshSpec { elements_around: 32, ..MeshSpec::default() }, FsMaterial { e_psi: m.e_ksi * 1000.0, nu: m.nu, law }, true)?;
    let r = fl.collapse(LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, FsOptions { strain_limit: None, travel_cap_over_a: 1.5, ..FsOptions::new(D - 0.0004, 0.0) })?;
    Ok(r.curve)
}

fn main() {
    let catalog: Vec<&Lib> = builtin_catalog().collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new(Vec::<(usize, String)>::new());
    let threads = 4;
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= catalog.len() {
                    break;
                }
                let m = catalog[i];
                let el = elongation(m);
                let mut fields = format!("{{\"index\":{i},\"id\":\"{}\"", m.id);
                let ok_inputs = m.ftu_ksi > 0.0 && m.sy_ksi > 0.0 && m.e_ksi > 0.0 && m.nu > 0.0 && m.nu < 0.5 && m.fbru_ksi > 0.0;
                match (el, ok_inputs) {
                    (Some(e), true) if m.id != "cfrp_qi" => {
                        let e_u = (e / 100.0).max(0.01);
                        fields += &format!(",\"elong\":{e}");
                        for (label, ed, fbru) in [("2", 2.0, m.fbru_ksi), ("15", 1.5, m.fbru_e15_ksi)] {
                            if fbru <= 0.0 {
                                continue;
                            }
                            match curve_for(m, e_u, ed) {
                                Ok(c) => {
                                    let (eps, how) = strain_for(&c, fbru * 1000.0 * D * T);
                                    fields += &format!(",\"eps{label}\":{eps:.4},\"how{label}\":\"{how}\"");
                                }
                                Err(err) => fields += &format!(",\"err{label}\":{:?}", err),
                            }
                        }
                    }
                    (None, _) => fields += ",\"note\":\"no elongation\"",
                    _ => fields += ",\"note\":\"incomplete inputs\"",
                }
                fields.push('}');
                out.lock().unwrap().push((i, fields));
            });
        }
    });
    let mut rows = out.into_inner().unwrap();
    rows.sort_by_key(|r| r.0);
    for (_, r) in rows {
        println!("{r}");
    }
}
