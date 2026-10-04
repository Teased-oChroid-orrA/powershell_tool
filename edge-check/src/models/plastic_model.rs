//! Elastic-perfectly-plastic limit load (J2, plane strain, fit pressure as a
//! dead load). The expensive model: one collapse solve is ~0.5 s, so the
//! collapse load is solved once on an edge-distance grid (in parallel, one
//! thread per solve) plus once at the actual edge distance, and every other
//! edge distance - the minimum-edge-distance search - is read off that
//! profile for free. See `plastic.rs` for the solver.

use crate::fem::{FemSolution, MeshSpec};
use crate::model::{EdgeModel, Response};
use crate::plastic::Collapse;
use crate::types::{Geometry, Mode, ModeMargin, Strengths, margin_of, Loads};
use std::sync::{Arc, Mutex};

/// Edge distances (in bore diameters) at which the collapse load is solved.
const GRID_E_OVER_D: [f64; 7] = [0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

/// Same plate sizing rule as the bushing adapter: `max(3 e, 10 a)`.
fn plate_for(a: f64, e: f64, template: &Geometry) -> Geometry {
    let reach = (3.0 * e).max(10.0 * a);
    Geometry { edge: e, plate_far: reach, plate_half_height: reach, ..*template }
}

pub struct PlasticModel {
    pub mesh: MeshSpec,
    cache: Mutex<Option<(Vec<u64>, Arc<Profile>)>>,
}

impl Default for PlasticModel {
    fn default() -> Self {
        Self { mesh: MeshSpec { n_radial: 12, n_arc: [3, 4, 12], grade: 2.0 }, cache: Mutex::new(None) }
    }
}

/// Collapse load (lbf) versus edge distance at the nominal fit pressure,
/// plus how much the fit pressure costs.
struct Profile {
    /// `(edge in, collapse load lbf)`, ascending in edge.
    points: Vec<(f64, f64)>,
    /// Collapse load without the fit divided by with it at the actual edge
    /// distance (>= 1): the relative cost of the interference.
    fit_ratio: f64,
    fit_nominal: f64,
}

impl Profile {
    fn at(&self, e: f64) -> f64 {
        let p = &self.points;
        if e <= p[0].0 {
            return p[0].1 * e / p[0].0; // edge-limited: collapse load ~ e
        }
        for w in p.windows(2) {
            if e <= w[1].0 {
                let t = (e - w[0].0) / (w[1].0 - w[0].0);
                return w[0].1 + t * (w[1].1 - w[0].1);
            }
        }
        p[p.len() - 1].1 // beyond the grid: hold (conservative)
    }

    /// Fit-pressure factor on the collapse load: linear in `p` between the
    /// solved no-fit and nominal-fit points.
    fn fit_factor(&self, p: f64) -> f64 {
        if self.fit_nominal <= 0.0 {
            1.0
        } else {
            (1.0 + (self.fit_ratio - 1.0) * (1.0 - p / self.fit_nominal)).max(0.0)
        }
    }
}

struct PlasticResponse {
    profile: Arc<Profile>,
    edge: f64,
}

impl Response for PlasticResponse {
    fn margins(&self, loads: &Loads, strength_scale: f64) -> Vec<ModeMargin> {
        let pc = self.profile.at(self.edge) * self.profile.fit_factor(loads.fit_pressure) * strength_scale;
        vec![ModeMargin { mode: Mode::Collapse, margin: margin_of(pc, loads.pin_load) }]
    }

    fn notes(&self) -> Vec<String> {
        vec![
            format!(
                "collapse load {:.0} lbf at this edge distance (J2 perfect plasticity, plane strain, flow stress min(Ftu, sqrt(3) Fsu), fit pressure as a dead load, load on the loaded half-arc)",
                self.profile.at(self.edge)
            ),
            "plane strain stands in for the triaxial confinement of a bushing bore; plane stress would under-predict bearing, so this brackets from the safe side only for the edge, not the bearing".to_string(),
        ]
    }
}

fn key(geom: &Geometry, mat: &Strengths, fit: f64) -> Vec<u64> {
    [geom.bore_radius, geom.thickness, mat.e, mat.nu, mat.ftu, mat.fsu, fit].iter().map(|v| v.to_bits()).collect()
}

impl EdgeModel for PlasticModel {
    fn id(&self) -> &'static str {
        "plastic"
    }
    fn label(&self) -> &'static str {
        "Elastic-plastic FE limit load"
    }
    fn is_field_model(&self) -> bool {
        false
    }

    fn respond(&self, geom: &Geometry, mat: &Strengths, fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        let k = key(geom, mat, fit_pressure);
        {
            let guard = self.cache.lock().unwrap();
            if let Some((ck, profile)) = guard.as_ref() {
                if *ck == k {
                    return Ok(Box::new(PlasticResponse { profile: profile.clone(), edge: geom.edge }));
                }
            }
        }
        let profile = Arc::new(self.build_profile(geom, mat, fit_pressure)?);
        *self.cache.lock().unwrap() = Some((k, profile.clone()));
        Ok(Box::new(PlasticResponse { profile, edge: geom.edge }))
    }
}

impl PlasticModel {
    fn build_profile(&self, geom: &Geometry, mat: &Strengths, fit_pressure: f64) -> Result<Profile, String> {
        let a = geom.bore_radius;
        let d = 2.0 * a;
        let sigma0 = mat.ftu.min(3f64.sqrt() * mat.fsu);
        // Jobs: (edge, fit pressure). Grid at the nominal fit, the actual
        // edge at the nominal fit, and the actual edge without the fit.
        let mut jobs: Vec<(f64, f64)> = GRID_E_OVER_D.iter().map(|f| (f * d, fit_pressure)).collect();
        if !jobs.iter().any(|(e, _)| (e - geom.edge).abs() < 1e-9 * geom.edge) {
            jobs.push((geom.edge, fit_pressure));
        }
        if fit_pressure > 0.0 {
            jobs.push((geom.edge, 0.0));
        }
        let mesh = self.mesh;
        let template = *geom;
        let results: Vec<Result<f64, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|&(e, p)| {
                    scope.spawn(move || -> Result<f64, String> {
                        let g = plate_for(a, e, &template);
                        let sol = FemSolution::solve_with(&g, mat.e, mat.nu, mesh, true)?;
                        match sol.collapse_load(p, sigma0) {
                            Collapse::Limit(l) => Ok(l),
                            Collapse::Unbounded(l) => Err(format!("no collapse found up to {l:.0} lbf at edge {e:.3} in")),
                            Collapse::FitOnlyFails => Err("the fit pressure alone exceeds the plate's capacity".to_string()),
                        }
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("collapse solver panicked".to_string()))).collect()
        });
        let mut vals = Vec::with_capacity(results.len());
        for r in results {
            vals.push(r?);
        }
        let mut points: Vec<(f64, f64)> = jobs.iter().zip(&vals).filter(|((_, p), _)| *p == fit_pressure).map(|(&(e, _), &v)| (e, v)).collect();
        points.sort_by(|x, y| x.0.total_cmp(&y.0));
        points.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-12);
        let with_fit = points.iter().find(|(e, _)| (e - geom.edge).abs() < 1e-9 * geom.edge).map(|p| p.1).unwrap_or(0.0);
        let no_fit = if fit_pressure > 0.0 {
            jobs.iter().zip(&vals).find(|((e, p), _)| *p == 0.0 && (e - geom.edge).abs() < 1e-9 * geom.edge).map(|(_, v)| *v).unwrap_or(with_fit)
        } else {
            with_fit
        };
        let fit_ratio = if with_fit > 0.0 { (no_fit / with_fit).max(1.0) } else { 1.0 };
        Ok(Profile { points, fit_ratio, fit_nominal: fit_pressure })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_core::materials::get_material;

    fn geom(ed: f64) -> Geometry {
        let a = 0.25;
        let e = ed * 2.0 * a;
        plate_for(a, e, &Geometry { bore_radius: a, edge: e, thickness: 0.5, plate_far: 1.0, plate_half_height: 1.0, plane_angle_deg: 40.0 })
    }

    #[test]
    fn the_profile_is_solved_once_then_every_edge_distance_is_free() {
        let mat = Strengths::from_material(get_material("al7075"));
        let model = PlasticModel::default();
        let t0 = std::time::Instant::now();
        let r1 = model.respond(&geom(1.5), &mat, 5000.0).unwrap();
        let first = t0.elapsed();
        let t1 = std::time::Instant::now();
        let r2 = model.respond(&geom(1.8), &mat, 5000.0).unwrap(); // different edge, same plate family -> cache hit
        assert!(t1.elapsed() < first / 20, "cache hit took {:?} vs {:?}", t1.elapsed(), first);
        // Margin at 1.8 D exceeds margin at 1.5 D for the same load.
        let loads = Loads { fit_pressure: 5000.0, pin_load: 30_000.0 };
        assert!(r2.margins(&loads, 1.0)[0].margin > r1.margins(&loads, 1.0)[0].margin);
    }

    #[test]
    fn the_profile_matches_a_direct_solve_at_an_off_grid_edge_distance() {
        let mat = Strengths::from_material(get_material("al7075"));
        let model = PlasticModel::default();
        let resp = model.respond(&geom(1.5), &mat, 0.0).unwrap();
        let pc_profile = resp.margins(&Loads { fit_pressure: 0.0, pin_load: 1.0 }, 1.0)[0].margin + 1.0; // = Pc / 1 lbf
        // Direct solve at 1.7 D, interpolated value from the profile at 1.7 D.
        let g = geom(1.7);
        let sol = FemSolution::solve_with(&g, mat.e, mat.nu, model.mesh, true).unwrap();
        let direct = match sol.collapse_load(0.0, mat.ftu.min(3f64.sqrt() * mat.fsu)) {
            Collapse::Limit(l) => l,
            o => panic!("{o:?}"),
        };
        let interp = model.respond(&g, &mat, 0.0).unwrap().margins(&Loads { fit_pressure: 0.0, pin_load: 1.0 }, 1.0)[0].margin + 1.0;
        assert!((interp / direct - 1.0).abs() < 0.04, "profile {interp} vs direct {direct} (at 1.5 D: {pc_profile})");
    }

    #[test]
    fn the_fit_pressure_lowers_the_collapse_load() {
        let mat = Strengths::from_material(get_material("al7075"));
        let model = PlasticModel::default();
        let resp = model.respond(&geom(1.5), &mat, 8000.0).unwrap();
        let with = resp.margins(&Loads { fit_pressure: 8000.0, pin_load: 20_000.0 }, 1.0)[0].margin;
        let without = resp.margins(&Loads { fit_pressure: 0.0, pin_load: 20_000.0 }, 1.0)[0].margin;
        assert!(without > with, "{without} vs {with}");
    }
}
