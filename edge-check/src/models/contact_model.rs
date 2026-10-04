//! Bushing-in-housing contact FE with elastic-plastic housing, as an
//! [`EdgeModel`]. Unlike the other models it needs no assumption about how
//! the fit and the pin load combine, how the pin load is distributed on the
//! bore, or whether contact is retained: all of that is solved
//! ([`crate::contact`]). It reports the elastic-plastic collapse load and
//! the first-yield load.
//!
//! Each solve is ~1.5 s and at most eight run at once (one per core on a
//! typical machine), so the collapse and first-yield loads are solved
//! once on an edge-distance grid (parallel threads) plus once at the actual
//! edge distance for three interference levels, and every other edge
//! distance or fit pressure (the minimum-edge-distance search, the Monte
//! Carlo pass) is interpolated from that table for free.

use crate::contact::ContactMesh;
use crate::fem::MeshSpec;
use crate::model::{EdgeModel, Response};
use crate::types::{BushingSpec, Geometry, Loads, Mode, ModeMargin, Strengths, margin_of};
use std::sync::{Arc, Mutex};

/// Edge distances (in bore diameters) at which the loads are solved.
const GRID_E_OVER_D: [f64; 5] = [1.0, 1.5, 2.0, 3.0, 4.0];
/// Interference multiples (of the nominal) at the actual edge distance.
const FIT_LEVELS: [f64; 3] = [0.0, 1.0, 1.5];

/// Same plate sizing rule as the other FE models: `max(3 e, 10 a)`.
fn plate_for(a: f64, e: f64, template: &Geometry) -> Geometry {
    let reach = (3.0 * e).max(10.0 * a);
    Geometry { edge: e, plate_far: reach, plate_half_height: reach, ..*template }
}

pub struct ContactModel {
    pub mesh: MeshSpec,
    pub spec: BushingSpec,
    cache: Mutex<Option<(Vec<u64>, Arc<Profile>)>>,
}

impl ContactModel {
    pub fn new(spec: BushingSpec) -> Self {
        Self { mesh: MeshSpec { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 }, spec, cache: Mutex::new(None) }
    }
}

struct Profile {
    /// `(edge in, collapse lbf, first yield lbf)` at the nominal interference.
    points: Vec<(f64, f64, f64)>,
    /// `(collapse, first yield)` at the actual edge for each of `FIT_LEVELS`.
    at_edge: [(f64, f64); 3],
    /// Mean contact pressure the FE computed for the nominal interference.
    fit_pressure: f64,
    /// Grid edge distances (in bore diameters) whose solve failed.
    skipped: Vec<f64>,
    /// Some solve stopped converging before its load plateaued: the loads are lower bounds.
    unplateaued: bool,
}

impl Profile {
    fn interp(&self, e: f64) -> (f64, f64) {
        let p = &self.points;
        if e <= p[0].0 {
            let k = e / p[0].0; // edge-limited: loads ~ e
            return (p[0].1 * k, p[0].2 * k);
        }
        for w in p.windows(2) {
            if e <= w[1].0 {
                let t = (e - w[0].0) / (w[1].0 - w[0].0);
                return (w[0].1 + t * (w[1].1 - w[0].1), w[0].2 + t * (w[1].2 - w[0].2));
            }
        }
        let last = p[p.len() - 1];
        (last.1, last.2) // beyond the grid: hold (conservative)
    }

    /// Multiplier on the nominal-interference loads at interference factor
    /// `f` (1 = nominal): piecewise linear through the solved levels, the
    /// last slope continued beyond them.
    fn fit_factor(&self, f: f64) -> (f64, f64) {
        let (c1, y1) = self.at_edge[1];
        let ratio = |a: f64, b: f64| if b > 0.0 { a / b } else { 1.0 };
        let (r, ry) = (
            [ratio(self.at_edge[0].0, c1), 1.0, ratio(self.at_edge[2].0, c1)],
            [ratio(self.at_edge[0].1, y1), 1.0, ratio(self.at_edge[2].1, y1)],
        );
        let f = f.max(0.0);
        let at = |v: [f64; 3]| {
            let (i, j) = if f <= FIT_LEVELS[1] { (0, 1) } else { (1, 2) };
            let t = (f - FIT_LEVELS[i]) / (FIT_LEVELS[j] - FIT_LEVELS[i]);
            (v[i] + t * (v[j] - v[i])).max(0.0)
        };
        (at(r), at(ry))
    }
}

struct ContactResponse {
    profile: Arc<Profile>,
    edge: f64,
    /// Nominal fit pressure the table was built for (psi), to turn the
    /// runner's pressure samples into interference factors.
    p_ref: f64,
}

impl Response for ContactResponse {
    fn margins(&self, loads: &Loads, strength_scale: f64) -> Vec<ModeMargin> {
        let (c, y) = self.profile.interp(self.edge);
        let f = if self.p_ref > 0.0 { loads.fit_pressure / self.p_ref } else { 1.0 };
        let (fc, fy) = self.profile.fit_factor(f);
        vec![
            ModeMargin { mode: Mode::Collapse, margin: margin_of(c * fc * strength_scale, loads.pin_load) },
            ModeMargin { mode: Mode::FirstYield, margin: margin_of(y * fy * strength_scale, loads.pin_load) },
        ]
    }

    fn notes(&self) -> Vec<String> {
        let (c, y) = self.profile.interp(self.edge);
        let mut notes = Vec::new();
        if !self.profile.skipped.is_empty() {
            let at: Vec<String> = self.profile.skipped.iter().map(|e| format!("{e:.2}")).collect();
            notes.push(format!("the contact solve did not converge at e/D {}: the minimum-edge-distance search interpolates over a coarser table", at.join(", ")));
        }
        if self.profile.unplateaued {
            notes.push("a contact solve stopped converging before the load plateaued: its collapse load is the highest converged load, a lower bound".to_string());
        }
        notes.extend([
            format!(
                "collapse {c:.0} lbf, first yield {y:.0} lbf at this edge distance; the FE fit pressure is {:.0} psi (bushing and housing both meshed, interference resolved by contact with friction, the pin a rigid cylinder pushed to collapse after the fit)",
                self.profile.fit_pressure
            ),
            "housing J2 perfect plasticity, plane strain, flow stress (Ftu + Fty)/2, capped at sqrt(3) Fsu; the bushing stays elastic (its own stress margin is a solver check)".to_string(),
        ]);
        notes
    }
}

/// Flow stress of the perfectly plastic housing: the mean of yield and
/// ultimate (the standard limit-analysis flow stress for a hardening
/// material), capped by `sqrt(3) Fsu` and never below yield. Validated
/// against NACA TN 1503 pin-bearing tests (`tests/validation_naca_tn1503.rs`):
/// with this rule all 12 test points are predicted within -18 % / +7 %; with
/// `Ftu` instead, 7075 is over-predicted by up to 16 %.
pub fn flow_stress(mat: &Strengths) -> f64 {
    (0.5 * (mat.ftu + mat.sy)).min(3f64.sqrt() * mat.fsu).max(mat.sy)
}

fn key(geom: &Geometry, mat: &Strengths, fit: f64) -> Vec<u64> {
    [geom.bore_radius, geom.thickness, mat.e, mat.nu, mat.ftu, mat.fsu, fit].iter().map(|v| v.to_bits()).collect()
}

impl EdgeModel for ContactModel {
    fn id(&self) -> &'static str {
        "contact"
    }
    fn label(&self) -> &'static str {
        "Contact FE (bushing + housing, elastic-plastic)"
    }
    fn is_field_model(&self) -> bool {
        false
    }

    /// Standard deviation of the prediction error over the 12 NACA TN 1503
    /// test points (-5.9 % mean, 8.5 % sd; `tests/validation_naca_tn1503.rs`).
    fn model_cv(&self) -> f64 {
        0.09
    }

    fn respond(&self, geom: &Geometry, mat: &Strengths, fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        let k = key(geom, mat, fit_pressure);
        if let Some((ck, profile)) = self.cache.lock().unwrap().as_ref() {
            if *ck == k {
                return Ok(Box::new(ContactResponse { profile: profile.clone(), edge: geom.edge, p_ref: fit_pressure }));
            }
        }
        let profile = Arc::new(self.build_profile(geom, mat)?);
        *self.cache.lock().unwrap() = Some((k, profile.clone()));
        Ok(Box::new(ContactResponse { profile, edge: geom.edge, p_ref: fit_pressure }))
    }
}

impl ContactModel {
    fn build_profile(&self, geom: &Geometry, mat: &Strengths) -> Result<Profile, String> {
        let a = geom.bore_radius;
        let d = 2.0 * a;
        let sigma0 = flow_stress(mat);
        let spec = self.spec;
        // Jobs `(edge, interference multiple)`: the grid at nominal, the
        // actual edge at nominal, and the actual edge at the other levels.
        let mut jobs: Vec<(f64, f64)> = GRID_E_OVER_D.iter().map(|f| (f * d, 1.0)).collect();
        if !jobs.iter().any(|(e, _)| (e - geom.edge).abs() < 1e-9 * geom.edge) {
            jobs.push((geom.edge, 1.0));
        }
        jobs.push((geom.edge, FIT_LEVELS[0]));
        jobs.push((geom.edge, FIT_LEVELS[2]));
        let (mesh, template, e_h, nu_h, t) = (self.mesh, *geom, mat.e, mat.nu, geom.thickness);
        let results: Vec<Result<(f64, f64, f64, bool), String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|&(e, level)| {
                    scope.spawn(move || -> Result<(f64, f64, f64, bool), String> {
                        let g = plate_for(a, e, &template);
                        let m = ContactMesh::build(&g, &spec, e_h, nu_h, mesh, 3, true)?;
                        let r = m.collapse(spec.interference * level, sigma0, t).map_err(|err| format!("edge {e:.3} in: {err}"))?;
                        Ok((r.collapse, r.first_yield, r.fit_pressure, r.plateau))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("contact solver panicked".to_string()))).collect()
        });
        let near = |e: f64| (e - geom.edge).abs() < 1e-9 * geom.edge;
        // The solves at the actual edge distance are essential; a failed
        // grid point elsewhere only coarsens the table (and is reported).
        let mut kept: Vec<((f64, f64), (f64, f64, f64, bool))> = Vec::new();
        let mut skipped = Vec::new();
        for (job, r) in jobs.iter().zip(results) {
            match r {
                Ok(v) => kept.push((*job, v)),
                Err(e) if near(job.0) => return Err(e),
                Err(_) => skipped.push(job.0 / d),
            }
        }
        let (jobs, vals): (Vec<(f64, f64)>, Vec<(f64, f64, f64, bool)>) = kept.into_iter().unzip();
        let unplateaued = vals.iter().any(|v| !v.3);
        let mut points: Vec<(f64, f64, f64)> = jobs.iter().zip(&vals).filter(|((_, l), _)| *l == 1.0).map(|(&(e, _), v)| (e, v.0, v.1)).collect();
        points.sort_by(|x, y| x.0.total_cmp(&y.0));
        points.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-12);
        let at_level = |level: f64| jobs.iter().zip(&vals).find(|((e, l), _)| near(*e) && *l == level).map(|(_, v)| (v.0, v.1)).ok_or("missing contact solve");
        let at_edge = [at_level(FIT_LEVELS[0])?, at_level(FIT_LEVELS[1])?, at_level(FIT_LEVELS[2])?];
        let fit_pressure = jobs.iter().zip(&vals).find(|((e, l), _)| near(*e) && *l == 1.0).map_or(0.0, |(_, v)| v.2);
        Ok(Profile { points, at_edge, fit_pressure, skipped, unplateaued })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_core::materials::get_material;

    fn model() -> ContactModel {
        ContactModel::new(BushingSpec { inner_radius: 0.1875, interference: 0.00075, e: 17.0e6, nu: 0.33, friction: 0.15 })
    }

    fn geom(ed: f64) -> Geometry {
        let a = 0.25;
        let e = ed * 2.0 * a;
        plate_for(a, e, &Geometry { bore_radius: a, edge: e, thickness: 0.5, plate_far: 1.0, plate_half_height: 1.0, plane_angle_deg: 40.0 })
    }

    fn cap(r: &dyn Response, fit: f64, mode: Mode) -> f64 {
        // Load at which the margin crosses zero = capacity (margin = cap/1 lbf - 1).
        r.margins(&Loads { fit_pressure: fit, pin_load: 1.0 }, 1.0).into_iter().find(|m| m.mode == mode).unwrap().margin + 1.0
    }

    #[test]
    fn the_table_is_solved_once_then_edge_distance_and_fit_are_free() {
        let mat = Strengths::from_material(get_material("al7075"));
        let m = model();
        let t0 = std::time::Instant::now();
        let r1 = m.respond(&geom(1.5), &mat, 9900.0).unwrap();
        let first = t0.elapsed();
        let t1 = std::time::Instant::now();
        let r2 = m.respond(&geom(2.0), &mat, 9900.0).unwrap(); // other edge, same plate family -> cache hit
        assert!(t1.elapsed() < first / 50, "cache hit took {:?} vs {:?}", t1.elapsed(), first);
        let (c15, c20) = (cap(&*r1, 9900.0, Mode::Collapse), cap(&*r2, 9900.0, Mode::Collapse));
        assert!(c20 > c15, "{c15} {c20}");
        assert!(cap(&*r1, 9900.0, Mode::FirstYield) < c15);
        // Zero fit and 1.5x the fit are interpolated/extrapolated from the solved levels.
        let (c0, c15x) = (cap(&*r1, 0.0, Mode::Collapse), cap(&*r1, 1.5 * 9900.0, Mode::Collapse));
        assert!(c0 > 0.0 && c15x > 0.0);
        assert!((cap(&*r1, 9900.0, Mode::Collapse) - c15).abs() < 1e-9);
    }

    #[test]
    fn a_bushing_larger_than_the_bore_is_an_error_not_a_panic() {
        let mat = Strengths::from_material(get_material("al7075"));
        let bad = ContactModel::new(BushingSpec { inner_radius: 0.3, interference: 0.00075, e: 17.0e6, nu: 0.33, friction: 0.15 });
        assert!(bad.respond(&geom(1.5), &mat, 9900.0).is_err());
    }
}
