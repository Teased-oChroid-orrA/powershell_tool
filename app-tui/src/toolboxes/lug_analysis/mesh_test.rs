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
/// An axial collapse is run up to this size only (it is much more expensive than the elastic solution, and converged:
/// 9772 -> 9733 lbf from 24 to 120 around the bore). An oblique one runs at every size: on the default lug at 45 deg it
/// falls 6831 / 5899 / 5726 / 5639 / 5579 lbf at 24 / 54 / 80 / 120 / 180 around, so an 80-around reference called an
/// unconverged collapse converged.
const AXIAL_COLLAPSE_MAX: usize = 80;

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
    /// The collapse load the sizes are compared with (lbf): extrapolated from the three finest when they fall
    /// steadily, else the finest one's.
    pub collapse_reference: Option<f64>,
    pub total_ms: f64,
}

impl MeshAdvice {
    /// One line for the field list.
    pub fn summary(&self) -> String {
        let r = self.rows.iter().find(|r| r.around == self.recommended);
        let mut t = r.map_or(String::new(), |r| format!(", {:.0} ms", r.ms));
        if let Some(c) = self.collapse_reference {
            t.push_str(&format!(", collapse -> {c:.0} lbf"));
        }
        if self.converged {
            format!("Recommended {} around the bore{t} (click applies)", self.recommended)
        } else {
            format!("Not converged by {} around the bore{t}: use more (click applies)", self.recommended)
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
                    let collapse = if input.plastic && (around <= AXIAL_COLLAPSE_MAX || !input.case.is_axial()) {
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
    let collapse_ref = collapse_reference(&rows);
    let within = |r: &MeshTestRow| {
        let collapse_ok = match (r.collapse, collapse_ref) {
            (Some(c), Some(f)) => rel(c, f) < COLLAPSE_TOL,
            _ => true,
        };
        rel(r.peak_hoop, finest.peak_hoop) < HOOP_TOL && rel(r.travel, finest.travel) < TRAVEL_TOL && rel(r.peak_pressure, finest.peak_pressure) < PRESSURE_TOL && collapse_ok
    };
    let first = (0..rows.len() - 1).find(|&i| rows[i..].iter().all(within));
    match first {
        Some(i) => MeshAdvice { recommended: rows[i].around, converged: true, collapse_reference: collapse_ref, rows, total_ms },
        None => MeshAdvice { recommended: finest.around, converged: false, collapse_reference: collapse_ref, rows, total_ms },
    }
}

/// The collapse load the sizes are compared with. A limit load converges from above; when the three finest sizes
/// with a collapse fall steadily (each step smaller than the last) it is Richardson-extrapolated from them (element
/// size ~ 1 / elements around, observed order clamped to 0.5..2), so a slowly falling oblique collapse is not taken
/// as converged at the finest size. Otherwise (rising, scattered, fewer than three) the finest size's collapse.
pub fn collapse_reference(rows: &[MeshTestRow]) -> Option<f64> {
    let with: Vec<(usize, f64)> = rows.iter().filter_map(|r| r.collapse.map(|c| (r.around, c))).collect();
    let finest = with.last()?.1;
    let [.., (_, c1), (n2, c2), (n3, c3)] = with.as_slice() else { return Some(finest) };
    let (d1, d2) = (c1 - c2, c2 - c3);
    if !(d1 > 0.0 && d2 > 0.0 && d2 < d1) {
        return Some(finest);
    }
    let r = *n3 as f64 / *n2 as f64;
    let order = ((d1 / d2).ln() / r.ln()).clamp(0.5, 2.0);
    Some(c3 - d2 / (r.powf(order) - 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(around: usize, hoop: f64, p: f64, travel: f64) -> MeshTestRow {
        MeshTestRow { around, peak_hoop: hoop, peak_pressure: p, travel, error_estimate: 0.01, collapse: None, ms: 1.0 }
    }

    /// Converged elastic rows with the collapse measured on the default lug at 45 deg (Ftu, kernel).
    fn oblique_rows() -> Vec<MeshTestRow> {
        [(24, 6830.7), (36, 6495.7), (54, 5899.2), (80, 5725.9), (120, 5639.3)].into_iter().map(|(n, c)| MeshTestRow { collapse: Some(c), ..row(n, 30.0e3, 17.0e3, 3.0e-3) }).collect()
    }

    #[test]
    fn a_slowly_falling_oblique_collapse_is_compared_with_its_extrapolation_not_the_finest_size() {
        let a = advise(oblique_rows(), 5.0);
        // 54 / 80 / 120 extrapolate to ~5553 lbf (180 around gives 5579 and is still falling): 80 is 3.1 % above it.
        let c = a.collapse_reference.unwrap();
        assert!((5540.0..5570.0).contains(&c), "{c}");
        // 80 is not accepted; 120 is within 1.6 % but the finest size is never called converged (it is still falling).
        assert_eq!((a.recommended, a.converged), (120, false), "{a:?}");
        assert!(a.summary().contains("Not converged by 120") && a.summary().contains("collapse -> 55"), "{}", a.summary());
        // The measured 180-around collapse keeps falling: 80 / 120 / 180 extrapolate lower still (~5445 lbf) and 120 is
        // then 3.6 % above, so the reference follows the trend instead of settling on the finest size.
        let mut rows = oblique_rows();
        rows.push(MeshTestRow { collapse: Some(5579.4), ..row(180, 30.0e3, 17.0e3, 3.0e-3) });
        let a = advise(rows, 5.0);
        let c = a.collapse_reference.unwrap();
        assert!((5420.0..5470.0).contains(&c) && (a.recommended, a.converged) == (180, false), "{c} {a:?}");
        // A series that has flattened out is accepted below the finest size.
        let flat: Vec<_> = [(24, 6000.0), (36, 5700.0), (54, 5600.0), (80, 5570.0), (120, 5560.0)].into_iter().map(|(n, c)| MeshTestRow { collapse: Some(c), ..row(n, 30.0e3, 17.0e3, 3.0e-3) }).collect();
        let a = advise(flat, 5.0);
        assert_eq!((a.recommended, a.converged), (36, true), "{a:?}");
        // Compared with the finest size alone (the old rule), 80 would have passed at 1.5 %.
        assert!(rel(5725.9, 5639.3) < COLLAPSE_TOL);
    }

    #[test]
    fn a_rising_or_short_collapse_series_falls_back_to_the_finest_size() {
        let mut rows = oblique_rows();
        rows[4].collapse = Some(5800.0); // rises at the finest size
        assert_eq!(collapse_reference(&rows), Some(5800.0));
        let two: Vec<_> = oblique_rows().into_iter().map(|r| MeshTestRow { collapse: (r.around >= 80).then_some(r.collapse.unwrap()), ..r }).collect();
        assert_eq!(collapse_reference(&two), Some(5639.3));
        assert_eq!(collapse_reference(&[row(24, 1.0, 1.0, 1.0)]), None);
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

    #[test]
    #[ignore = "slow (about 90 s): the evidence for AXIAL_COLLAPSE_MAX; PROBE_ANGLE sets the load angle"]
    fn oblique_collapse_by_mesh_size() {
        let mut m = super::super::model::LugUiModel::default();
        m.load_angle = std::env::var("PROBE_ANGLE").ok().and_then(|s| s.parse().ok()).unwrap_or(45.0);
        let input = m.input().unwrap();
        let fe_material = FeMaterial { e_psi: input.material.e_psi, nu: input.material.nu };
        let pin = PinSpec { friction: 0.0, body: PinBody::Rigid, ..input.pin };
        let flow = input.material.flow_for(input.flow_rule, input.elongation);
        let sizes = [24usize, 36, 54, 72, 80, 120, 180];
        let out: Vec<_> = std::thread::scope(|sc| {
            let hs: Vec<_> = sizes.iter().map(|&around| sc.spawn(move || {
                let t = std::time::Instant::now();
                let cr = input.geometry.bore_radius();
                let refine = auto_refinement_for(&input.geometry, fe_material, pin, input.case, around, cr);
                let spec = MeshSpec { elements_around: around, max_growth: input.mesh.max_growth, first_layer_aspect: input.mesh.first_layer_aspect, refine };
                let s = FeaLug::build_bushed(&input.geometry, fe_material, spec, false, PlaneMode::Strain, None).unwrap();
                let l = s.limit_load(pin, input.case, flow, LimitOptions::default());
                (around, l.map(|l| (l.limit_load_lbf, l.plateau)), t.elapsed().as_secs_f64())
            })).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (a, l, t) in out { eprintln!("collapse at {} around {a:>3}: {l:?} {t:.1}s", m.load_angle); }
    }
}
