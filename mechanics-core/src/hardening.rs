//! Isotropic hardening laws for J2 plasticity: von Mises yield stress against equivalent plastic
//! strain, as a fixed-size piecewise-linear table (so material options stay `Copy`), with the
//! exact radial-return plastic increment. Shared by the lug solvers and `fea-core`.

/// Isotropic hardening law: von Mises yield stress against equivalent plastic strain, a
/// piecewise-linear table (flat beyond its last point). Fixed-size so options stay `Copy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hardening {
    n: usize,
    pts: [(f64, f64); Hardening::MAX],
}

impl Hardening {
    pub const MAX: usize = 24;

    /// Table of `(equivalent plastic strain, yield stress)`; strains strictly increasing from 0,
    /// stresses non-decreasing, at most [`Hardening::MAX`] points.
    pub fn table(points: &[(f64, f64)]) -> Result<Self, String> {
        if points.len() < 2 || points.len() > Self::MAX {
            return Err(format!("a hardening table needs 2 to {} points", Self::MAX));
        }
        if points[0].0 != 0.0 || !points[0].1.is_finite() || points[0].1 <= 0.0 {
            return Err("the hardening table must start at zero plastic strain with a positive stress".into());
        }
        for w in points.windows(2) {
            if !(w[1].0 > w[0].0 && w[1].1 >= w[0].1 && w[1].1.is_finite()) {
                return Err("hardening strains must increase and stresses must not fall".into());
            }
        }
        let mut pts = [(0.0, 0.0); Self::MAX];
        pts[..points.len()].copy_from_slice(points);
        Ok(Self { n: points.len(), pts })
    }

    /// Linear hardening `sigma_y + h * eps` up to `eps_max` (then flat).
    pub fn linear(sigma_y: f64, h: f64, eps_max: f64) -> Result<Self, String> {
        Self::table(&[(0.0, sigma_y), (eps_max, sigma_y + h * eps_max)])
    }

    /// Ramberg-Osgood curve `eps = s/E + 0.002 (s/Fty)^n` through the 0.2 % offset yield `fty`
    /// and the ultimate `ftu` at the strain `e_u` (engineering stress treated as true: this is a
    /// strength-level idealisation, not a necking model), tabulated up to plastic strain `e_u`.
    pub fn ramberg_osgood(fty: f64, ftu: f64, e_u: f64) -> Result<Self, String> {
        if !(fty > 0.0 && ftu >= fty && e_u > 0.002) {
            return Err("Ramberg-Osgood needs 0 < Fty <= Ftu and an ultimate strain above 0.2 %".into());
        }
        let n = if ftu > fty * (1.0 + 1e-9) { ((e_u / 0.002).ln() / (ftu / fty).ln()).clamp(3.0, 80.0) } else { 80.0 };
        let sigma = |ep: f64| (fty * (ep / 0.002).powf(1.0 / n)).min(ftu.max(fty));
        let e0 = 1e-5;
        let mut pts = vec![(0.0, sigma(e0))];
        let k = 20;
        for i in 0..k {
            let ep = e0 * (e_u / e0).powf(i as f64 / (k - 1) as f64);
            let sy = sigma(ep).max(pts.last().unwrap().1);
            pts.push((ep, sy));
        }
        pts.dedup_by(|b, a| b.0 <= a.0);
        Self::table(&pts)
    }

    /// True (Cauchy) yield stress against true equivalent plastic strain from the engineering
    /// data of a tensile test: Ramberg-Osgood through the 0.2 % yield `fty` and the ultimate `ftu`
    /// at the uniform elongation `e_u`, converted to true stress and logarithmic strain, then
    /// continued past the necking (Considere) point by the Swift law `sigma_u (eps / n)^n` with
    /// `n = ln (1 + e_u)` (value and slope continuous there) up to `eps_max`. This is the curve a
    /// finite-strain analysis needs: the local stress keeps rising toward `Ftu (1 + e_u)` and beyond.
    pub fn true_curve(fty: f64, ftu: f64, e_u: f64, eps_max: f64) -> Result<Self, String> {
        if !(fty > 0.0 && ftu >= fty && e_u > 0.002 && eps_max > 0.0) {
            return Err("the true curve needs 0 < Fty <= Ftu, an elongation above 0.2 % and a positive strain range".into());
        }
        let n_swift = (1.0 + e_u).ln();
        let sigma_u = ftu * (1.0 + e_u);
        let n_ro = if ftu > fty * (1.0 + 1e-9) { ((e_u / 0.002).ln() / (ftu / fty).ln()).clamp(3.0, 80.0) } else { 80.0 };
        // Engineering stress at engineering plastic strain, then true stress `s (1 + e)`.
        let ro = |ep_eng: f64| (fty * (ep_eng / 0.002).powf(1.0 / n_ro)).min(ftu) * (1.0 + ep_eng);
        let e0 = 1e-5;
        let mut pts: Vec<(f64, f64)> = vec![(0.0, ro(e0))];
        // Ramberg-Osgood part, 9 points to the uniform elongation (true plastic strain ln(1 + e)).
        for i in 0..9 {
            let e = e0 * (e_u / e0).powf(i as f64 / 8.0);
            let (eps, sig) = ((1.0 + e).ln(), ro(e));
            if eps > pts.last().unwrap().0 {
                pts.push((eps, sig.max(pts.last().unwrap().1)));
            }
        }
        // Swift part: geometric spacing from the uniform strain to eps_max.
        let start = pts.last().unwrap().0.max(n_swift);
        let k = Self::MAX - pts.len();
        if eps_max > start && k >= 2 {
            for i in 1..=k {
                let eps = start * (eps_max / start).powf(i as f64 / k as f64);
                let sig = sigma_u * (eps / n_swift).powf(n_swift);
                if eps > pts.last().unwrap().0 {
                    pts.push((eps, sig.max(pts.last().unwrap().1)));
                }
            }
        }
        Self::table(&pts)
    }

    /// Yield stress at equivalent plastic strain `ep`.
    pub fn stress(&self, ep: f64) -> f64 {
        let p = &self.pts[..self.n];
        if ep <= 0.0 {
            return p[0].1;
        }
        for w in p.windows(2) {
            if ep <= w[1].0 {
                return w[0].1 + (w[1].1 - w[0].1) * (ep - w[0].0) / (w[1].0 - w[0].0);
            }
        }
        p[self.n - 1].1
    }

    /// Hardening slope `d sigma_y / d ep` on the segment containing `ep` (the left derivative at a
    /// breakpoint; zero beyond the last table point). This is the `H'` of the consistent tangent.
    pub fn slope(&self, ep: f64) -> f64 {
        let p = &self.pts[..self.n];
        for w in p.windows(2) {
            if ep <= w[1].0 {
                return (w[1].1 - w[0].1) / (w[1].0 - w[0].0);
            }
        }
        0.0
    }

    /// The plastic increment `d >= 0` solving `q - 3 g d = sigma_y(ep + d)` for a trial von Mises
    /// stress `q` above `sigma_y(ep)`. The law is piecewise linear, so the root is exact and found
    /// segment by segment (no iteration).
    pub fn plastic_increment(&self, ep: f64, q: f64, g: f64) -> f64 {
        let p = &self.pts[..self.n];
        let mut e = ep;
        let mut y = self.stress(ep);
        // Segment index containing `ep`.
        let mut k = p.windows(2).position(|w| ep < w[1].0).unwrap_or(self.n - 1);
        loop {
            let (slope, end) = if k + 1 < self.n { ((p[k + 1].1 - p[k].1) / (p[k + 1].0 - p[k].0), p[k + 1].0) } else { (0.0, f64::INFINITY) };
            // On this segment sigma_y(e + t) = y + slope t; solve q - 3 g (e + t - ep) = y + slope t.
            let t = (q - y - 3.0 * g * (e - ep)) / (3.0 * g + slope);
            if e + t <= end || k + 1 >= self.n {
                return (e + t - ep).max(0.0);
            }
            y += slope * (end - e);
            e = end;
            k += 1;
        }
    }

    /// Initial yield stress.
    pub fn initial(&self) -> f64 {
        self.pts[0].1
    }

    /// Largest tabulated plastic strain.
    pub fn last_strain(&self) -> f64 {
        self.pts[self.n - 1].0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_hardening_increment_has_the_closed_form_root() {
        // q - 3 g d = sigma_y + h (ep + d)  =>  d = (q - sigma_y - h ep) / (3 g + h).
        let law = Hardening::linear(40_000.0, 2.0e5, 1.0).unwrap();
        let (g, ep, q) = (4.0e6, 0.003, 90_000.0);
        let d = law.plastic_increment(ep, q, g);
        let want = (q - 40_000.0 - 2.0e5 * ep) / (3.0 * g + 2.0e5);
        assert!((d - want).abs() < 1e-15 * want.abs().max(1.0), "{d} vs {want}");
        assert!((q - 3.0 * g * d - law.stress(ep + d)).abs() < 1e-8, "the root lies on the yield curve");
    }

    #[test]
    fn increment_crosses_segments_exactly_and_the_slope_is_the_segment_slope() {
        let law = Hardening::table(&[(0.0, 100.0), (0.01, 200.0), (0.02, 200.0), (0.05, 260.0)]).unwrap();
        assert_eq!(law.slope(0.005), 1.0e4);
        assert_eq!(law.slope(0.015), 0.0);
        assert!((law.slope(0.03) - 2.0e3).abs() < 1e-9);
        assert_eq!(law.slope(1.0), 0.0, "flat beyond the table");
        let g = 5.0e3;
        for q in [150.0, 250.0, 600.0, 2000.0] {
            let d = law.plastic_increment(0.0, q, g);
            assert!((q - 3.0 * g * d - law.stress(d)).abs() < 1e-9 * q, "q = {q}: residual {}", q - 3.0 * g * d - law.stress(d));
        }
    }

    #[test]
    fn invalid_tables_are_rejected() {
        assert!(Hardening::table(&[(0.0, 100.0)]).is_err());
        assert!(Hardening::table(&[(0.1, 100.0), (0.2, 120.0)]).is_err(), "must start at zero strain");
        assert!(Hardening::table(&[(0.0, 100.0), (0.1, 90.0)]).is_err(), "stress must not fall");
        assert!(Hardening::table(&[(0.0, 100.0), (0.0, 120.0)]).is_err(), "strain must increase");
    }
}
