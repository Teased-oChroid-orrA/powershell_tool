//! Shared value types. Units are imperial throughout (in, lbf, psi) - the
//! same convention as `bushing-solver`.

use mechanics_core::materials::Material;

/// Plane-stress tensor components (psi), x toward the far side of the
/// plate, y across it (the load acts along -x, toward the free edge at
/// x = 0).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stress {
    pub xx: f64,
    pub yy: f64,
    pub xy: f64,
}

impl Stress {
    pub const ZERO: Stress = Stress { xx: 0.0, yy: 0.0, xy: 0.0 };

    pub fn new(xx: f64, yy: f64, xy: f64) -> Self {
        Self { xx, yy, xy }
    }

    pub fn scaled(self, k: f64) -> Self {
        Self { xx: self.xx * k, yy: self.yy * k, xy: self.xy * k }
    }

    pub fn plus(self, o: Stress) -> Self {
        Self { xx: self.xx + o.xx, yy: self.yy + o.yy, xy: self.xy + o.xy }
    }

    /// Von Mises equivalent stress for plane stress.
    pub fn von_mises(self) -> f64 {
        (self.xx * self.xx - self.xx * self.yy + self.yy * self.yy + 3.0 * self.xy * self.xy).max(0.0).sqrt()
    }

    /// Shear traction along a line whose unit direction is `d = (dx, dy)`:
    /// `n·σ·d` with `n = (-dy, dx)` (the in-plane normal).
    pub fn shear_along(self, dx: f64, dy: f64) -> f64 {
        let (nx, ny) = (-dy, dx);
        nx * (self.xx * dx + self.xy * dy) + ny * (self.xy * dx + self.yy * dy)
    }
}

/// Material strengths and elastic constants in psi (the `Material` table
/// is in ksi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strengths {
    pub e: f64,
    pub nu: f64,
    pub sy: f64,
    pub fsu: f64,
    pub ftu: f64,
    pub fbru: f64,
    /// `Fbru` at e/D = 1.5 (psi); `0.0` = not tabulated.
    pub fbru_e15: f64,
}

impl Strengths {
    /// Same fall-through rule as `bushing-solver::solve::compute`: a zero
    /// (absent) `Fsu`/`Fbru`/`Ftu` falls back to `Sy`.
    pub fn from_material(m: &Material) -> Self {
        let ksi = 1000.0;
        let or_sy = |v: f64| if v != 0.0 { v } else { m.sy_ksi };
        Self { e: m.e_ksi * ksi, nu: m.nu, sy: m.sy_ksi * ksi, fsu: or_sy(m.fsu_ksi) * ksi, ftu: or_sy(m.ftu_ksi) * ksi, fbru: or_sy(m.fbru_ksi) * ksi, fbru_e15: m.fbru_e15_ksi * ksi }
    }
}

/// Plate geometry for one evaluation. The free edge is the line `x = 0`,
/// the bore centre is `(edge, 0)`. The load acts on the bore along -x
/// (worst case: straight at the nearest free edge).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub bore_radius: f64,
    pub edge: f64,
    pub thickness: f64,
    /// Distance from the bore centre to the far (load-reacting) face of
    /// the plate, in. Only the finite-element model uses it; the
    /// half-plane analytic model has no far face.
    pub plate_far: f64,
    /// Half-height of the plate across the load line, in. FE only.
    pub plate_half_height: f64,
    /// Inclination (deg) of the classical shear-out planes to the load
    /// line - the solver's `edge_load_angle_deg` (default 40).
    pub plane_angle_deg: f64,
}

/// Loads for one evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loads {
    /// Interference contact pressure on the bore, psi.
    pub fit_pressure: f64,
    /// Pin load through the bushing toward the edge, lbf.
    pub pin_load: f64,
}

/// Failure modes a model can report. `margin = allowable / applied - 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Peak elastic von Mises stress anywhere on the bore or the free edge
    /// versus yield: first local yielding.
    FirstYield,
    /// Mean shear stress on the two tangent shear-out planes versus `Fsu`:
    /// the limit load if the material redistributes shear fully (ductile).
    ShearOut,
    /// Mean hoop tension across the ligament between bore and free edge
    /// versus `Ftu` (interference splitting).
    Splitting,
    /// Elastic-plastic limit load of the plate (edge, plane strain) versus
    /// the pin load: no assumption about where or how the ligament fails.
    Collapse,
    /// Applied bearing stress `P / (D t)` versus the tabulated `Fbru`.
    Bearing,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::FirstYield => "first yield",
            Mode::ShearOut => "shear-out",
            Mode::Splitting => "splitting",
            Mode::Bearing => "bearing",
            Mode::Collapse => "collapse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeMargin {
    pub mode: Mode,
    pub margin: f64,
}

/// `allowable / applied - 1`, infinite when nothing is applied.
pub fn margin_of(allowable: f64, applied: f64) -> f64 {
    if applied > 0.0 && applied.is_finite() {
        allowable / applied - 1.0
    } else {
        f64::INFINITY
    }
}

/// The bushing pressed into the bore, for the contact FE model: it is the
/// only model that represents the bushing itself (the others take the fit
/// as a given pressure and the pin load as a given distribution).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BushingSpec {
    /// Bushing bore radius, in (the pin's radius).
    pub inner_radius: f64,
    /// Radial interference (bushing OD radius minus housing bore radius), in.
    pub interference: f64,
    pub e: f64,
    pub nu: f64,
    /// Coulomb friction coefficient on the bushing/housing interface.
    pub friction: f64,
}
