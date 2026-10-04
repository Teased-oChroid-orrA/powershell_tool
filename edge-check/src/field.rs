//! Failure-mode evaluation shared by every model that can supply a stress
//! field (the FE model and the analytic model): sample the three unit
//! fields once at a fixed set of probe points, then any `(fit_pressure,
//! pin_load)` combination is a cheap linear combination of the stored
//! tensors - no re-solve, which is what makes Monte-Carlo sampling and the
//! edge-distance search fast.

use crate::fem::UnitCase;
use crate::model::Response;
use crate::types::{margin_of, Geometry, Loads, Mode, ModeMargin, Strengths, Stress};
use std::f64::consts::PI;

/// Anything that can return the plane-stress tensor of a unit load case at
/// a point `(x, y)` (the free edge is `x = 0`, the bore centre `(edge, 0)`).
pub trait UnitFieldSource {
    fn stress(&self, case: UnitCase, x: f64, y: f64) -> Option<Stress>;

    /// All three unit cases at once (a source whose cases share work may
    /// override this).
    fn stress_all(&self, x: f64, y: f64) -> Option<[Stress; 3]> {
        Some([self.stress(UnitCase::Fit, x, y)?, self.stress(UnitCase::PinHalf, x, y)?, self.stress(UnitCase::PinFull, x, y)?])
    }
}

type Triple = [Stress; 3];

struct Weighted {
    w: f64,
    s: Triple,
}

pub struct FieldResponse {
    geom: Geometry,
    mat: Strengths,
    /// Bore boundary and free-edge points: first-yield probes.
    yield_probes: Vec<Triple>,
    /// Ligament line `y = 0`, `x in [0, edge - a]`: Gauss weights sum to 1.
    ligament: Vec<Weighted>,
    /// Tangent shear-out plane `y = +a`, `x in [0, edge]` (the mirror plane
    /// `y = -a` carries the same shear): Gauss weights sum to 1.
    plane: Vec<Weighted>,
}

fn sample(src: &dyn UnitFieldSource, x: f64, y: f64) -> Option<Triple> {
    src.stress_all(x, y)
}

/// Gauss-Legendre nodes and weights on `[0, 1]` (weights sum to 1).
/// The shear and hoop integrals are smooth but steep near the bore, so a
/// Gauss rule is both cheaper and far more accurate than a trapezoid rule.
fn gauss_legendre_unit(n: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // Initial guess, then Newton on P_n.
        let mut x = (PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
        let mut dp = 1.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, x);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            dp = n as f64 * (x * p1 - p0) / (x * x - 1.0);
            let dx = p1 / dp;
            x -= dx;
            if dx.abs() < 1e-15 {
                break;
            }
        }
        out.push((0.5 * (x + 1.0), 1.0 / ((1.0 - x * x) * dp * dp)));
    }
    out
}

impl FieldResponse {
    pub fn build(src: &dyn UnitFieldSource, geom: &Geometry, mat: &Strengths) -> Result<Self, String> {
        let a = geom.bore_radius;
        let e = geom.edge;
        let miss = |what: &str, x: f64, y: f64| format!("stress field has no value at the {what} probe ({x:.4}, {y:.4})");

        let mut yield_probes = Vec::new();
        // Bore boundary, phi in [0, pi] (the half plate; the other half mirrors).
        const NB: usize = 73;
        for k in 0..NB {
            let phi = PI * k as f64 / (NB - 1) as f64;
            let (x, y) = (e + a * phi.cos(), a * phi.sin());
            yield_probes.push(sample(src, x, y).ok_or_else(|| miss("bore", x, y))?);
        }
        // Free edge x = 0, from the load line outward (geometric spacing).
        const NE: usize = 25;
        for k in 0..NE {
            let y = 4.0 * a * ((k as f64) / (NE - 1) as f64).powi(2);
            yield_probes.push(sample(src, 0.0, y).ok_or_else(|| miss("free-edge", 0.0, y))?);
        }

        // Ligament between bore and free edge on the load line.
        const NG: usize = 24;
        let gauss = gauss_legendre_unit(NG);
        let mut ligament = Vec::with_capacity(NG);
        for &(u, w) in &gauss {
            let x = (e - a) * u;
            ligament.push(Weighted { w, s: sample(src, x, 0.0).ok_or_else(|| miss("ligament", x, 0.0))? });
        }

        // Shear-out planes: the two lines tangent to the bore, parallel to the
        // load, running from the free edge to the bore's side points
        // (y = +-a, x in [0, edge]). The strip between them is in x-force
        // equilibrium with the bore arc it contains, so the mean shear on
        // them is exactly (arc force) / (2 t e) - integrating the field
        // checks the discretisation against that identity.
        let mut plane = Vec::with_capacity(NG);
        for &(u, w) in &gauss {
            let x = e * u;
            plane.push(Weighted { w, s: sample(src, x, a).ok_or_else(|| miss("shear plane", x, a))? });
        }
        Ok(Self { geom: *geom, mat: *mat, yield_probes, ligament, plane })
    }

    /// Pin-load case implied by the loads: with the fit retaining contact on
    /// the back side the pin load is shared over the whole bore, otherwise
    /// only the loaded half carries it.
    fn pin_case(&self, loads: &Loads) -> usize {
        let peak_back_tension = loads.pin_load / (PI * self.geom.bore_radius * self.geom.thickness);
        if loads.fit_pressure >= peak_back_tension {
            2
        } else {
            1
        }
    }

    fn combine(t: &Triple, pin_idx: usize, loads: &Loads) -> Stress {
        t[0].scaled(loads.fit_pressure).plus(t[pin_idx].scaled(loads.pin_load))
    }
}

impl Response for FieldResponse {
    fn margins(&self, loads: &Loads, strength_scale: f64) -> Vec<ModeMargin> {
        let pin = self.pin_case(loads);
        let sy = self.mat.sy * strength_scale;
        let fsu = self.mat.fsu * strength_scale;
        let ftu = self.mat.ftu * strength_scale;

        let vm = self.yield_probes.iter().map(|t| Self::combine(t, pin, loads).von_mises()).fold(0.0, f64::max);

        let sigma_yy: f64 = self.ligament.iter().map(|p| p.w * Self::combine(&p.s, pin, loads).yy).sum();
        let tau: f64 = self.plane.iter().map(|p| p.w * Self::combine(&p.s, pin, loads).xy).sum();

        vec![
            ModeMargin { mode: Mode::ShearOut, margin: margin_of(fsu, tau.abs()) },
            ModeMargin { mode: Mode::Splitting, margin: if sigma_yy > 0.0 { margin_of(ftu, sigma_yy) } else { f64::INFINITY } },
            ModeMargin { mode: Mode::FirstYield, margin: margin_of(sy, vm) },
        ]
    }

    fn contact_retained(&self, loads: &Loads) -> Option<bool> {
        Some(self.pin_case(loads) == 2)
    }
}
