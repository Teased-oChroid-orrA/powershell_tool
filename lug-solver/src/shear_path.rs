//! The shear-out path read from a solved elastic-plastic bracket instead of assumed.
//!
//! The bracket's displacement-driven run ends with a field of equivalent plastic strain. Where the plate shears out, that
//! strain concentrates on a band from the hole wall to the free edge: the ridge of the field. [`ridge`] finds it, a straight
//! line fitted through the strain maxima of strips across the ligament (weighted by the strain), and reports where it leaves
//! the wall and how it is inclined, with the two angles `edge_check::shear` uses for the elastic planes (`start_deg` from the
//! load line toward the edge at the hole centre, `incline_deg` to the load line, positive opening away from it).
//!
//! [`trace`] then measures the mean shear along that line (the traction component along it, Gauss quadrature over points
//! located in the mesh, through the thickness in 3D) at every step of the pin's travel: the solver reports each converged step
//! to an observer with its displacements and the material state it started from, and the stress of a plastic element is
//! read out by the same return map the solve used (`fea_core::locate::Locator::nl_stress_at`). The load at which that mean
//! reaches `Fsu` is then a read-off of the curve, not an extrapolation; if the mean never gets there by the end of the
//! travel, the path says so.
//!
//! Model frame: the hole centre is the origin, the free edge is `x = -edge`, the pin travels toward `-x`, the model is the
//! half `y >= 0` (and `z >= 0` in 3D).

use fea_core::locate::Locator;
use fea_core::nonlinear::NlState;
use fea_core::{Mesh, Model};

/// The ridge and the shear it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct ShearPath {
    /// Where the line leaves the hole wall, degrees from the load line toward the edge at the hole centre.
    pub start_deg: f64,
    /// Inclination to the load line, degrees, positive opening away from it toward the edge.
    pub incline_deg: f64,
    /// Length from the wall to the free edge, in.
    pub length: f64,
    /// The wall point and the edge point (model frame, upper half).
    pub wall: [f64; 2],
    pub edge_pt: [f64; 2],
    /// Largest equivalent plastic strain on the ridge.
    pub peak_plastic: f64,
    /// `(pin load lbf, mean shear psi)` after every pin step (the load as in `Out::pts`).
    pub curve: Vec<(f64, f64)>,
}

impl ShearPath {
    /// The mean shear at the end of the pin's travel.
    pub fn mean_at_end(&self) -> f64 {
        self.curve.last().map_or(0.0, |p| p.1)
    }

    /// The pin load at which the mean shear along the path first reaches `fsu`, by linear interpolation between the two steps
    /// that straddle it; `None` if the travel ends first.
    pub fn load_at(&self, fsu: f64) -> Option<f64> {
        let mut prev = (0.0, 0.0);
        for &(load, tau) in &self.curve {
            if tau >= fsu {
                let w = if tau > prev.1 { (fsu - prev.1) / (tau - prev.1) } else { 1.0 };
                return Some(prev.0 + w * (load - prev.0));
            }
            prev = (load, tau);
        }
        None
    }
}

/// A line through the plastic ridge, before its shear is measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ridge {
    pub start_deg: f64,
    pub incline_deg: f64,
    pub wall: [f64; 2],
    pub edge_pt: [f64; 2],
    pub peak: f64,
}

/// Strips across the ligament the ridge is searched in.
const STRIPS: usize = 14;
/// A strip counts toward the fit when its strain maximum is at least this fraction of the largest.
const KEEP: f64 = 0.15;
/// Nodes this close to the load line (degrees from it, seen from the hole centre) belong to the bearing crush ahead of the
/// pin, whose strain is the largest of the plate and says nothing about shear: they are not searched, and a ridge that
/// leaves the wall this close is refused.
const MIN_START_DEG: f64 = 15.0;
/// The strip maxima must span at least this share of the ligament, or the plastic zone has not reached the free edge and
/// there is no path to the edge to read.
const MIN_SPAN: f64 = 0.5;

/// The ridge of the equivalent plastic strain `plastic` (one value per node of `mesh`; in 3D the mid-plane nodes are read).
/// `a` is the hole radius, `edge` the distance from the hole centre to the free edge.
pub fn ridge(mesh: &Mesh, plastic: &[f64], a: f64, edge: f64) -> Result<Ridge, String> {
    // Per strip across the ligament (its distance `u` from the hole centre toward the edge), the node with the most strain.
    let mut best: Vec<Option<(f64, f64, f64)>> = vec![None; STRIPS];
    let (u0, u1) = (0.2 * a, edge);
    for (i, p) in mesh.nodes.iter().enumerate() {
        if p[1] < -1e-12 || p[2].abs() > 1e-12 || p[0] > 0.0 {
            continue;
        }
        let u = -p[0];
        if !(u0..=u1 + 1e-9).contains(&u) || p[0].hypot(p[1]) < a * (1.0 - 1e-6) || p[1].atan2(u).to_degrees() < MIN_START_DEG {
            continue;
        }
        let k = (((u - u0) / (u1 - u0)) * STRIPS as f64).floor().clamp(0.0, STRIPS as f64 - 1.0) as usize;
        let ep = plastic[i];
        if best[k].is_none_or(|b| ep > b.2) {
            best[k] = Some((u, p[1], ep));
        }
    }
    let peak = best.iter().flatten().fold(0.0f64, |m, b| m.max(b.2));
    if peak <= 0.0 {
        return Err("no plastic strain between the hole and the free edge".into());
    }
    let pts: Vec<(f64, f64, f64)> = best.iter().flatten().filter(|b| b.2 >= KEEP * peak).copied().collect();
    if pts.len() < 3 {
        return Err("the plastic strain is too localised to define a path".into());
    }
    let (lo, hi) = pts.iter().fold((f64::INFINITY, 0.0f64), |m, p| (m.0.min(p.0), m.1.max(p.0)));
    if hi - lo < MIN_SPAN * (edge - a) {
        return Err(format!("the plastic zone off the load line spans {:.0} % of the ligament: it has not reached the free edge, so there is no shear path to read", 100.0 * (hi - lo) / (edge - a)));
    }
    // Weighted least squares y = c0 + c1 u.
    let (sw, su, sy) = pts.iter().fold((0.0, 0.0, 0.0), |s, p| (s.0 + p.2, s.1 + p.2 * p.0, s.2 + p.2 * p.1));
    let (mu, my) = (su / sw, sy / sw);
    let (suu, suy) = pts.iter().fold((0.0, 0.0), |s, p| (s.0 + p.2 * (p.0 - mu).powi(2), s.1 + p.2 * (p.0 - mu) * (p.1 - my)));
    if suu <= 0.0 {
        return Err("the plastic strain maxima lie on one cross-section".into());
    }
    let c1 = suy / suu;
    let c0 = my - c1 * mu;
    // Where the line meets the wall: the root nearest the edge of (1 + c1^2) u^2 + 2 c0 c1 u + c0^2 - a^2 = 0.
    let (qa, qb, qc) = (1.0 + c1 * c1, 2.0 * c0 * c1, c0 * c0 - a * a);
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return Err("the plastic ridge does not reach the hole wall".into());
    }
    let uw = (-qb + disc.sqrt()) / (2.0 * qa);
    let yw = c0 + c1 * uw;
    if yw < 0.0 || uw <= 0.0 {
        return Err("the plastic ridge meets the wall on the far side of the hole".into());
    }
    let start_deg = yw.atan2(uw).to_degrees();
    if start_deg < MIN_START_DEG {
        return Err(format!("the plastic strain peaks on the load line (the ridge meets the wall at {start_deg:.0} degrees): bearing crush, not a shear path"));
    }
    Ok(Ridge { start_deg, incline_deg: c1.atan().to_degrees(), wall: [-uw, yw], edge_pt: [-edge, c0 + c1 * edge], peak })
}

/// One converged pin step: the displacements at its end and the material state at its start.
pub(crate) struct Step {
    pub u: Vec<f64>,
    pub before: NlState,
    /// The pin load after the step, lbf.
    pub load: f64,
}

const NG: usize = 16;

/// Mean shear along the ridge at every step. `half_t` is the half thickness of a 3D model (`None` for 2D): the path is
/// averaged through the thickness at the mid-plane, a quarter and nearly the surface (Simpson weights).
pub(crate) fn trace(model: &Model, steps: &[Step], r: &Ridge, half_t: Option<f64>) -> Result<ShearPath, String> {
    let loc: Locator = model.locator();
    let (dx, dy) = (r.edge_pt[0] - r.wall[0], r.edge_pt[1] - r.wall[1]);
    let length = dx.hypot(dy);
    let d = [dx / length, dy / length];
    let zs: Vec<(f64, f64)> = match half_t {
        None => vec![(0.0, 1.0)],
        Some(h) => vec![(0.0, 1.0 / 6.0), (0.5 * h, 4.0 / 6.0), (0.999 * h, 1.0 / 6.0)],
    };
    let gauss = gauss_unit(NG);
    let mut curve = Vec::with_capacity(steps.len());
    for s in steps {
        let mut acc = 0.0;
        for &(u, w) in &gauss {
            for &(z, wz) in &zs {
                let x = [r.wall[0] + u * dx, r.wall[1] + u * dy, z];
                let t = loc.nl_stress_at(model, &s.u, &s.before, x).ok_or("the shear path leaves the mesh")?;
                // Shear along the path: n . sigma . d with n = (-dy, dx).
                let shear = -d[1] * (t[0] * d[0] + t[3] * d[1]) + d[0] * (t[3] * d[0] + t[1] * d[1]);
                acc += w * wz * shear;
            }
        }
        curve.push((s.load, acc.abs()));
    }
    Ok(ShearPath { start_deg: r.start_deg, incline_deg: r.incline_deg, length, wall: r.wall, edge_pt: r.edge_pt, peak_plastic: r.peak, curve })
}

/// Gauss-Legendre nodes and weights on `[0, 1]`.
fn gauss_unit(n: usize) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let mut x = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
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
            (0.5 * (x + 1.0), 1.0 / ((1.0 - x * x) * dp * dp))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gauss_rule_integrates_a_cubic_exactly() {
        let s: f64 = gauss_unit(4).iter().map(|&(x, w)| w * (x * x * x + 2.0 * x)).sum();
        assert!((s - (0.25 + 1.0)).abs() < 1e-14);
    }

    #[test]
    fn a_path_reads_its_load_at_a_shear_by_interpolation_and_none_beyond_the_curve() {
        let p = ShearPath { start_deg: 50.0, incline_deg: 0.0, length: 1.0, wall: [0.0; 2], edge_pt: [0.0; 2], peak_plastic: 0.1, curve: vec![(100.0, 10.0), (200.0, 30.0), (300.0, 40.0)] };
        assert!((p.load_at(20.0).unwrap() - 150.0).abs() < 1e-12);
        assert!((p.load_at(5.0).unwrap() - 50.0).abs() < 1e-12);
        assert!(p.load_at(45.0).is_none());
        assert_eq!(p.mean_at_end(), 40.0);
    }
}
