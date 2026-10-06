//! The brief mesh-size test: the elastic case (and, with the plastic limit load on, the collapse) at several mesh
//! sizes on the general kernel (frictionless, in parallel), timed and compared, and a recommendation of the
//! coarsest size whose peak hoop stress, pressure, pin travel and collapse load have converged. It runs on a worker like the analysis (`Effect::RunLugMeshTest`).

use lug_solver::fea::FeaLug;
use lug_solver::{auto_refinement_for, LimitOptions, Material as FeMaterial, MeshSpec, PinBody, PinSpec, PlaneMode};

use super::model::LugInput;

/// Elements around the bore that are tried (the largest is the reference the others are compared with).
const SIZES: [usize; 5] = [24, 36, 54, 80, 120];
/// Peak hoop stress and pin travel must agree with the reference to this.
const HOOP_TOL: f64 = 0.02;
const TRAVEL_TOL: f64 = 0.01;
/// Peak pressure scatters at the Gauss points, so it only has to agree loosely.
const PRESSURE_TOL: f64 = 0.10;
/// The collapse load of a limit analysis converges from above and slowly for oblique loads.
const COLLAPSE_TOL: f64 = 0.03;
/// The collapse is run at these sizes only (it is much more expensive than the elastic solution).
const COLLAPSE_MAX: usize = 80;

/// One tested mesh size.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshTestRow {
    pub around: usize,
    pub peak_hoop: f64,
    pub peak_pressure: f64,
    pub travel: f64,
    /// Zienkiewicz-Zhu relative energy-norm error estimate.
    pub error_estimate: f64,
    /// Collapse load (lbf) when the plastic limit load is on and this size is within the collapse range.
    pub collapse: Option<f64>,
    pub ms: f64,
}

/// The result of the test for one set of inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshAdvice {
    /// The inputs the test was made for (the mesh fields of the input are ignored when matching).
    pub rows: Vec<MeshTestRow>,
    /// Recommended elements around the bore.
    pub recommended: usize,
    /// The recommendation is converged within the tolerances (else the finest tested size is returned).
    pub converged: bool,
    pub total_ms: f64,
}

impl MeshAdvice {
    /// One line for the field list.
    pub fn summary(&self) -> String {
        let r = self.rows.iter().find(|r| r.around == self.recommended);
        let t = r.map_or(String::new(), |r| format!(", {:.0} ms", r.ms));
        if self.converged {
            format!("Recommended {} around the bore{t} (Enter applies)", self.recommended)
        } else {
            format!("Not converged by {} around the bore{t}: use more (Enter applies)", self.recommended)
        }
    }
}

/// Everything the test depends on, so a stale result is recognised: the geometry, material, pin, load and
/// bushing - not the mesh fields, which the test varies itself.
pub fn signature(input: &LugInput) -> String {
    format!("{:?}|{:?}|{:?}|{:?}|{:?}|{}", input.geometry, input.material, input.pin, input.case, input.bushing, input.auto_refine)
}

fn rel(a: f64, b: f64) -> f64 {
    if b.abs() < 1e-300 {
        0.0
    } else {
        (a / b - 1.0).abs()
    }
}

/// Run the test. The pin is treated as rigid and frictionless (the converged hoop stress does not depend on
/// either, and it keeps the test brief); the load, angle, geometry, bushing and temperature are the inputs'.
pub fn run(input: &LugInput) -> Result<MeshAdvice, String> {
    let t0 = std::time::Instant::now();
    let fe_material = FeMaterial { e_psi: input.material.e_psi, nu: input.material.nu };
    let pin = PinSpec { friction: 0.0, body: PinBody::Rigid, ..input.pin };
    let bushing = input.bushing.as_ref().map(|b| b.spec());
    let contact_radius = input.bushing.as_ref().map_or(input.geometry.bore_radius(), |b| b.inner_dia / 2.0);
    let rows: Vec<Result<MeshTestRow, String>> = std::thread::scope(|sc| {
        let handles: Vec<_> = SIZES
            .iter()
            .map(|&around| {
                sc.spawn(move || {
                    let t = std::time::Instant::now();
                    let refine = if input.auto_refine { auto_refinement_for(&input.geometry, fe_material, pin, input.case, around, contact_radius) } else { None };
                    let spec = MeshSpec { elements_around: around, max_growth: input.mesh.max_growth, first_layer_aspect: input.mesh.first_layer_aspect, refine };
                    let lug = FeaLug::build_bushed(&input.geometry, fe_material, spec, input.case.is_axial(), PlaneMode::Stress, bushing)?;
                    let s = lug.analyze(pin, input.case)?;
                    let collapse = if input.plastic && around <= COLLAPSE_MAX {
                        let strain = FeaLug::build_bushed(&input.geometry, fe_material, spec, input.case.is_axial(), PlaneMode::Strain, bushing)?;
                        let flow = input.material.flow_for(input.flow_rule, input.elongation);
                        strain.limit_load(pin, input.case, flow, LimitOptions::default()).ok().map(|l| l.limit_load_lbf)
                    } else {
                        None
                    };
                    Ok(MeshTestRow { around, peak_hoop: s.peak_hoop, peak_pressure: s.peak_pressure, travel: s.bearing_deflection, error_estimate: s.verification.discretisation_error, collapse, ms: t.elapsed().as_secs_f64() * 1e3 })
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("the mesh test panicked".to_string()))).collect()
    });
    let rows = rows.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(advise(rows, t0.elapsed().as_secs_f64() * 1e3))
}

/// The recommendation from timed rows (ascending size): the coarsest size from which every finer one stays
/// within the tolerances of the finest.
pub fn advise(rows: Vec<MeshTestRow>, total_ms: f64) -> MeshAdvice {
    let finest = rows.last().cloned().expect("at least one row");
    // The collapse is compared with the finest size that has one.
    let collapse_ref = rows.iter().rev().find_map(|r| r.collapse);
    let within = |r: &MeshTestRow| {
        let collapse_ok = match (r.collapse, collapse_ref) {
            (Some(c), Some(f)) => rel(c, f) < COLLAPSE_TOL,
            _ => true,
        };
        rel(r.peak_hoop, finest.peak_hoop) < HOOP_TOL && rel(r.travel, finest.travel) < TRAVEL_TOL && rel(r.peak_pressure, finest.peak_pressure) < PRESSURE_TOL && collapse_ok
    };
    let first = (0..rows.len() - 1).find(|&i| rows[i..].iter().all(within));
    match first {
        Some(i) => MeshAdvice { recommended: rows[i].around, converged: true, rows, total_ms },
        None => MeshAdvice { recommended: finest.around, converged: false, rows, total_ms },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(around: usize, hoop: f64, p: f64, travel: f64) -> MeshTestRow {
        MeshTestRow { around, peak_hoop: hoop, peak_pressure: p, travel, error_estimate: 0.01, collapse: None, ms: 1.0 }
    }

    #[test]
    fn the_coarsest_size_that_stays_converged_is_recommended() {
        let rows = vec![row(24, 28.0e3, 16.0e3, 3.0e-3), row(36, 29.6e3, 16.9e3, 3.03e-3), row(54, 30.1e3, 17.2e3, 3.04e-3), row(80, 30.2e3, 17.3e3, 3.04e-3), row(120, 30.25e3, 17.3e3, 3.04e-3)];
        let a = advise(rows, 5.0);
        assert!(a.converged);
        assert_eq!(a.recommended, 54, "{a:?}");
        assert!(a.summary().contains("54"));
    }

    #[test]
    fn a_still_changing_hoop_stress_recommends_the_finest_and_says_so() {
        let rows = vec![row(24, 20.0e3, 10.0e3, 2.0e-3), row(36, 24.0e3, 12.0e3, 2.4e-3), row(54, 27.0e3, 14.0e3, 2.7e-3), row(80, 29.0e3, 15.0e3, 2.9e-3), row(120, 31.0e3, 16.0e3, 3.1e-3)];
        let a = advise(rows, 5.0);
        assert!(!a.converged);
        assert_eq!(a.recommended, 120);
        assert!(a.summary().contains("Not converged"));
    }

    #[test]
    fn a_size_that_matches_by_luck_but_has_finer_ones_that_do_not_is_not_recommended() {
        // 36 happens to land on the answer, 54 overshoots: only sizes from which all finer ones agree count.
        let rows = vec![row(24, 25.0e3, 15.0e3, 3.0e-3), row(36, 30.2e3, 17.3e3, 3.04e-3), row(54, 31.5e3, 17.3e3, 3.04e-3), row(80, 30.3e3, 17.3e3, 3.04e-3), row(120, 30.25e3, 17.3e3, 3.04e-3)];
        let a = advise(rows, 5.0);
        assert_eq!(a.recommended, 80, "{a:?}");
    }

    #[test]
    fn the_real_test_runs_on_the_default_lug_and_converges_to_a_sensible_size() {
        let input = super::super::model::LugUiModel::default().input().unwrap();
        let a = run(&input).unwrap();
        for r in &a.rows {
            eprintln!("mesh test {:>3}: hoop {:.0} p {:.0} travel {:.4e} zz {:.3} collapse {:?} ({:.0} ms)", r.around, r.peak_hoop, r.peak_pressure, r.travel, r.error_estimate, r.collapse, r.ms);
        }
        eprintln!("recommended {} converged {} in {:.1} s", a.recommended, a.converged, a.total_ms / 1e3);
        assert_eq!(a.rows.len(), SIZES.len());
        assert!(a.rows.iter().all(|r| r.peak_hoop > 10_000.0 && r.ms > 0.0), "{:?}", a.rows);
        assert!(SIZES.contains(&a.recommended));
        // A finer mesh is never the only converged one: the finest is compared with itself.
        assert!(rel(a.rows[3].peak_hoop, a.rows[4].peak_hoop) < 0.05, "{:?}", a.rows);
        assert_eq!(signature(&input), signature(&input));
    }
}
