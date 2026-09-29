//! Thread geometry and the full (non-reduced) V-thread torque equation -
//! spec sections 3/4/7. Explicitly retains pitch diameter, lead angle, and
//! thread flank angle rather than collapsing to a generic torque
//! coefficient `T = K*F*d` (section 7's own instruction).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadGeometry {
    /// Nominal/major diameter, in (or consistent length unit throughout -
    /// this crate is unit-agnostic; the TUI layer fixes inches/lbf/in-lbf).
    pub d: f64,
    /// Pitch diameter.
    pub d2: f64,
    /// Root/minor diameter.
    pub d3: f64,
    /// Thread pitch (distance between adjacent threads for a single-start
    /// thread).
    pub pitch: f64,
    /// Number of thread starts - `lead = starts * pitch` (section 4: "do
    /// not assume single-start internally").
    pub starts: u32,
    /// Thread flank half-angle already expressed as the full included
    /// angle would be for a symmetric V-thread (e.g. 60 deg for UN/ISO
    /// metric) - halved internally wherever the flank-angle correction is
    /// applied.
    pub thread_angle_deg: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThreadGeometryError {
    NonPositiveDiameter,
    NonPositivePitch,
    ZeroStarts,
    MajorNotGreaterThanMinor,
    InvalidThreadAngle,
    PitchDiameterOutOfRange,
}

impl ThreadGeometry {
    pub fn validate(&self) -> Result<(), ThreadGeometryError> {
        if self.d <= 0.0 || self.d2 <= 0.0 || self.d3 <= 0.0 {
            return Err(ThreadGeometryError::NonPositiveDiameter);
        }
        if self.pitch <= 0.0 {
            return Err(ThreadGeometryError::NonPositivePitch);
        }
        if self.starts == 0 {
            return Err(ThreadGeometryError::ZeroStarts);
        }
        if self.d <= self.d3 {
            return Err(ThreadGeometryError::MajorNotGreaterThanMinor);
        }
        if !(self.d3..=self.d).contains(&self.d2) {
            return Err(ThreadGeometryError::PitchDiameterOutOfRange);
        }
        if !(self.thread_angle_deg.is_finite()) || self.thread_angle_deg <= 0.0 || self.thread_angle_deg >= 180.0 {
            return Err(ThreadGeometryError::InvalidThreadAngle);
        }
        Ok(())
    }

    /// `lead = n * p` (spec section 4) - the axial advance per full
    /// relative revolution, for an `n`-start thread.
    pub fn lead(&self) -> f64 {
        self.starts as f64 * self.pitch
    }

    /// `lambda = atan(lead / (pi * d2))` (spec section 4).
    pub fn lead_angle_rad(&self) -> f64 {
        (self.lead() / (std::f64::consts::PI * self.d2)).atan()
    }

    /// Minor/root cross-sectional area (`pi/4 * d3^2`).
    pub fn root_area(&self) -> f64 {
        std::f64::consts::FRAC_PI_4 * self.d3 * self.d3
    }

    /// Major-diameter cross-sectional area.
    pub fn major_area(&self) -> f64 {
        std::f64::consts::FRAC_PI_4 * self.d * self.d
    }

    /// Standard tensile-stress area approximation (ISO 898-1 / Machinery's
    /// Handbook form): `(pi/4) * ((d2 + d3) / 2)^2` - the average of the
    /// pitch and minor diameters, the widely used "tensile stress area"
    /// distinct from both the major- and minor-diameter areas (spec
    /// section 15: "at minimum calculate and expose major-diameter area,
    /// minor/root area, standard tensile-stress area").
    pub fn tensile_stress_area(&self) -> f64 {
        let d_avg = (self.d2 + self.d3) / 2.0;
        std::f64::consts::FRAC_PI_4 * d_avg * d_avg
    }

    /// Polar moment of inertia at the root section (`J = pi*d3^4/32`),
    /// used for torsional shear/twist at the most-stressed (thread-root)
    /// section (spec section 28).
    pub fn root_polar_moment(&self) -> f64 {
        std::f64::consts::PI * self.d3.powi(4) / 32.0
    }
}

/// Full V-thread power-screw torque relation (spec section 7, retained in
/// expanded closed form - never collapsed to `K*F*d`):
///
/// ```text
/// mu' = mu_thread / cos(alpha/2)
/// rho' = atan(mu')
/// T_thread = F * d2/2 * tan(lambda + rho')
/// ```
///
/// `alpha` here is the full thread included angle (`thread_angle_deg`);
/// the flank half-angle `alpha/2` is what appears in the friction
/// correction (e.g. 30 deg for a 60 deg metric/UN thread) - this is the
/// standard power-screw derivation (Shigley, *Mechanical Engineering
/// Design*, "Power Screws" chapter) applied to a thread's helix angle.
pub fn thread_torque(preload: f64, geometry: &ThreadGeometry, mu_thread: f64) -> f64 {
    let half_angle_rad = (geometry.thread_angle_deg / 2.0) * std::f64::consts::PI / 180.0;
    let mu_prime = mu_thread / half_angle_rad.cos();
    let rho_prime = mu_prime.atan();
    let lambda = geometry.lead_angle_rad();
    preload * geometry.d2 / 2.0 * (lambda + rho_prime).tan()
}

/// Derivative of [`thread_torque`] with respect to `preload` - `T_thread`
/// is exactly linear in `F` for fixed geometry/friction (everything else
/// in the formula is a constant coefficient), so this is exact, not a
/// finite-difference approximation. Exposed for an optional Newton-assisted
/// step (spec section 12: "optionally, Newton-Raphson... when a reliable
/// derivative exists") - never the sole solver, Brent remains primary.
pub fn thread_torque_slope(geometry: &ThreadGeometry, mu_thread: f64) -> f64 {
    thread_torque(1.0, geometry, mu_thread)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m10_geometry() -> ThreadGeometry {
        // M10x1.5 (coarse), approximate ISO metric dimensions.
        ThreadGeometry { d: 10.0, d2: 9.026, d3: 8.160, pitch: 1.5, starts: 1, thread_angle_deg: 60.0 }
    }

    #[test]
    fn lead_equals_pitch_for_a_single_start_thread() {
        let g = m10_geometry();
        assert!((g.lead() - 1.5).abs() < 1e-12);
    }

    #[test]
    fn lead_scales_with_start_count_for_a_multi_start_thread() {
        let mut g = m10_geometry();
        g.starts = 3;
        assert!((g.lead() - 4.5).abs() < 1e-12);
    }

    #[test]
    fn thread_torque_at_zero_friction_reduces_to_the_lead_angle_only_term() {
        let g = m10_geometry();
        // mu=0 -> rho'=0, T = F*d2/2*tan(lambda) - the friction-free case
        // required by spec section 71.
        let torque = thread_torque(10000.0, &g, 0.0);
        let expected = 10000.0 * g.d2 / 2.0 * g.lead_angle_rad().tan();
        assert!((torque - expected).abs() < 1e-6);
    }

    #[test]
    fn thread_torque_is_linear_in_preload() {
        let g = m10_geometry();
        let t1 = thread_torque(1000.0, &g, 0.15);
        let t2 = thread_torque(2000.0, &g, 0.15);
        assert!((t2 - 2.0 * t1).abs() < 1e-9);
    }

    #[test]
    fn thread_torque_slope_matches_the_torque_at_unit_preload() {
        let g = m10_geometry();
        let slope = thread_torque_slope(&g, 0.15);
        let torque_at_5000 = thread_torque(5000.0, &g, 0.15);
        assert!((slope * 5000.0 - torque_at_5000).abs() < 1e-9);
    }

    #[test]
    fn thread_torque_increases_with_friction() {
        let g = m10_geometry();
        let low = thread_torque(10000.0, &g, 0.10);
        let high = thread_torque(10000.0, &g, 0.20);
        assert!(high > low);
    }

    #[test]
    fn validate_rejects_major_not_greater_than_minor() {
        let mut g = m10_geometry();
        g.d = 8.0;
        assert_eq!(g.validate(), Err(ThreadGeometryError::MajorNotGreaterThanMinor));
    }

    #[test]
    fn validate_rejects_zero_pitch() {
        let mut g = m10_geometry();
        g.pitch = 0.0;
        assert_eq!(g.validate(), Err(ThreadGeometryError::NonPositivePitch));
    }

    #[test]
    fn validate_accepts_well_formed_geometry() {
        assert!(m10_geometry().validate().is_ok());
    }

    #[test]
    fn tensile_stress_area_is_between_root_and_major_area() {
        let g = m10_geometry();
        assert!(g.tensile_stress_area() > g.root_area());
        assert!(g.tensile_stress_area() < g.major_area());
    }
}
