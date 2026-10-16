//! The unit system of a problem. The solvers are unit-blind: lengths, forces, stresses and densities only have to be
//! consistent with each other. A problem therefore *declares* which consistent system it is written in, so cards, the
//! material library and the checks can speak in units (and a slip such as a modulus in ksi beside loads in psi is named),
//! and it can be *converted* to another system with every number rescaled.
//!
//! | system | length | force | stress | mass density | temperature |
//! |---|---|---|---|---|---|
//! | `InchPsi` (the default, and every file written before units existed) | in | lbf | psi | lbf s^2/in^4 | deg F |
//! | `MmMpa` | mm | N | MPa | N s^2/mm^4 (tonne/mm^3) | deg C |
//! | `MeterPa` | m | N | Pa | kg/m^3 | deg C |
//!
//! Thermal power uses force * length / second; absolute temperatures include the Fahrenheit/Celsius offset.
use crate::{
    history::{FeatureKind, Plane},
    problem::{Geometry, Load, MaterialSpec, Problem, Seg, Shape, Support},
    sketch::{Constraint, Entity, Sketch},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UnitSystem {
    #[default]
    InchPsi,
    MmMpa,
    MeterPa,
}

/// Standard gravity in inch / s^2, which turns a weight density (lbf/in^3) into a mass density in lbf s^2/in^4.
pub const G_INCH: f64 = 386.089;

impl UnitSystem {
    pub const ALL: [UnitSystem; 3] = [UnitSystem::InchPsi, UnitSystem::MmMpa, UnitSystem::MeterPa];

    pub fn label(self) -> &'static str {
        match self {
            Self::InchPsi => "inch, lbf, psi",
            Self::MmMpa => "mm, N, MPa",
            Self::MeterPa => "m, N, Pa",
        }
    }
    pub fn length(self) -> &'static str {
        match self {
            Self::InchPsi => "in",
            Self::MmMpa => "mm",
            Self::MeterPa => "m",
        }
    }
    pub fn force(self) -> &'static str {
        match self {
            Self::InchPsi => "lbf",
            Self::MmMpa | Self::MeterPa => "N",
        }
    }
    pub fn stress(self) -> &'static str {
        match self {
            Self::InchPsi => "psi",
            Self::MmMpa => "MPa",
            Self::MeterPa => "Pa",
        }
    }
    pub fn density(self) -> &'static str {
        match self {
            Self::InchPsi => "lbf s^2/in^4",
            Self::MmMpa => "N s^2/mm^4",
            Self::MeterPa => "kg/m^3",
        }
    }
    pub fn temperature(self) -> &'static str {
        match self {
            Self::InchPsi => "deg F",
            Self::MmMpa | Self::MeterPa => "deg C",
        }
    }
    /// Metres in one length unit.
    pub fn length_m(self) -> f64 {
        match self {
            Self::InchPsi => 0.0254,
            Self::MmMpa => 1e-3,
            Self::MeterPa => 1.0,
        }
    }
    /// Newtons in one force unit.
    pub fn force_n(self) -> f64 {
        match self {
            Self::InchPsi => 4.448_221_615_260_5,
            Self::MmMpa | Self::MeterPa => 1.0,
        }
    }
    /// Pascals in one stress unit.
    pub fn stress_pa(self) -> f64 {
        self.force_n() / self.length_m().powi(2)
    }
    /// Kelvin in one degree of temperature difference.
    pub fn kelvin_per_degree(self) -> f64 {
        match self {
            Self::InchPsi => 5.0 / 9.0,
            Self::MmMpa | Self::MeterPa => 1.0,
        }
    }
    /// Mass density unit in kg/m^3 (force s^2 / length^4).
    pub fn density_kg_m3(self) -> f64 {
        self.force_n() / self.length_m().powi(4)
    }
    /// The factors that take a number written in `self` to `to`.
    fn factors(self, to: UnitSystem) -> Factors {
        Factors {
            length: self.length_m() / to.length_m(),
            force: self.force_n() / to.force_n(),
            stress: self.stress_pa() / to.stress_pa(),
            density: self.density_kg_m3() / to.density_kg_m3(),
            // A temperature difference, and a coefficient per degree (its inverse).
            temperature: self.kelvin_per_degree() / to.kelvin_per_degree(),
        }
    }
    /// Stress in this system from ksi (the material library's unit).
    pub fn from_ksi(self, ksi: f64) -> f64 {
        if self == UnitSystem::InchPsi {
            // The library's own system: exact, not a round trip through pascals.
            return ksi * 1000.0;
        }
        ksi * 1000.0 * UnitSystem::InchPsi.stress_pa() / self.stress_pa()
    }
    /// Mass density in this system from a weight density in lb/in^3 (the library's unit).
    pub fn from_weight_density(self, lb_in3: f64) -> f64 {
        if self == UnitSystem::InchPsi {
            return lb_in3 / G_INCH;
        }
        lb_in3 / G_INCH * UnitSystem::InchPsi.density_kg_m3() / self.density_kg_m3()
    }
    /// Thermal expansion per degree in this system from microstrain per deg F (the library's unit).
    pub fn from_microstrain_per_f(self, u: f64) -> f64 {
        u * 1e-6 / (UnitSystem::InchPsi.kelvin_per_degree() / self.kelvin_per_degree())
    }
    /// A modulus in this system as pascals.
    pub fn stress_to_pa(self, v: f64) -> f64 {
        v * self.stress_pa()
    }
}

#[derive(Clone, Copy)]
struct Factors {
    length: f64,
    force: f64,
    stress: f64,
    density: f64,
    temperature: f64,
}

fn scale_shape(s: &mut Shape, f: f64) {
    match s {
        Shape::Rect { x0, y0, x1, y1 } => (*x0, *y0, *x1, *y1) = (*x0 * f, *y0 * f, *x1 * f, *y1 * f),
        Shape::Circle { cx, cy, r } => (*cx, *cy, *r) = (*cx * f, *cy * f, *r * f),
        Shape::Slot { cx, cy, length, width, .. } => (*cx, *cy, *length, *width) = (*cx * f, *cy * f, *length * f, *width * f),
        Shape::Polygon { pts } => pts.iter_mut().for_each(|p| *p = [p[0] * f, p[1] * f]),
        Shape::Outline { segs } => segs.iter_mut().for_each(|sg| match sg {
            Seg::Line { a, b } => (*a, *b) = ([a[0] * f, a[1] * f], [b[0] * f, b[1] * f]),
            Seg::Arc { c, r, .. } => (*c, *r) = ([c[0] * f, c[1] * f], *r * f),
            Seg::Spline { pts } => pts.iter_mut().for_each(|p| *p = [p[0] * f, p[1] * f]),
        }),
    }
}

fn scale_sketch(sk: &mut Sketch, f: f64) {
    for e in &mut sk.entities {
        match &mut e.v {
            Entity::Point { x, y } => (*x, *y) = (*x * f, *y * f),
            Entity::Circle { r, .. } => *r *= f,
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => {}
        }
    }
    for c in &mut sk.constraints {
        match &mut c.v {
            Constraint::Fixed { x, y, .. } => (*x, *y) = (*x * f, *y * f),
            Constraint::Length { d, .. } | Constraint::Distance { d, .. } | Constraint::HDist { d, .. } | Constraint::VDist { d, .. } | Constraint::Radius { d, .. } => *d *= f,
            _ => {}
        }
    }
}

fn scale_material(m: &mut MaterialSpec, k: Factors) {
    m.e *= k.stress;
    m.yield_stress = m.yield_stress.map(|y| y * k.stress);
    m.hardening *= k.stress;
    m.density *= k.density;
    // Expansion is a strain per degree: a degree of the new scale is `1 / temperature` of the old one.
    m.alpha /= k.temperature;
}

impl Problem {
    /// Whether geometry can be rescaled; external sources retain a cumulative coordinate scale.
    pub fn can_convert_units(&self) -> Result<(), String> {
        match self.geometry {
            Geometry::Sketch { .. } | Geometry::Imported { .. } | Geometry::Solid { cache: Some(_), .. } => Ok(()),
            Geometry::Solid { .. } => Err("a STEP solid needs a current cached surface before conversion: re-import it with the OpenCascade build".into()),
        }
    }

    /// Rescale every length, force, stress, density and expansion so the same physical problem is written in `to`.
    /// Thermal quantities follow the same consistent force/length/time system. Fails before mutation when geometry cannot be rescaled.
    pub fn convert_units(&mut self, to: UnitSystem) -> Result<(), String> {
        if to == self.units {
            return Ok(());
        }
        self.can_convert_units()?;
        let k = self.units.factors(to);
        let f = k.length;
        if let Geometry::Imported { scale, .. } = &mut self.geometry {
            scale.length *= f;
            scale.stress *= k.stress;
            scale.expansion /= k.temperature;
        }
        if let Geometry::Solid { cache: Some(c), .. } = &mut self.geometry {
            c.coordinate_scale *= f;
            c.deflection *= f;
            for v in &mut c.vertices {
                for x in v {
                    *x *= f;
                }
            }
        }
        // Energy = force * length; temperatures are absolute, expansion uses differences.
        let offset = match (self.units, to) {
            (UnitSystem::InchPsi, _) => -32.0 * k.temperature,
            (_, UnitSystem::InchPsi) => 32.0,
            _ => 0.0,
        };
        let absolute = |t: &mut f64| *t = *t * k.temperature + offset;
        self.thermal.conductivity *= k.force / k.temperature;
        self.thermal.source *= k.force / f.powi(2);
        absolute(&mut self.thermal.reference);
        for support in &mut self.thermal.supports {
            match support {
                crate::thermal::HeatSupport::Temperature { t, .. } => absolute(t),
                crate::thermal::HeatSupport::Point { x, y, z, t } => {
                    *x *= f;
                    *y *= f;
                    *z *= f;
                    absolute(t);
                }
            }
        }
        for load in &mut self.thermal.loads {
            match load {
                crate::thermal::HeatLoad::Flux { q, .. } => *q *= k.force / f,
                crate::thermal::HeatLoad::Convection { h, ambient, .. } => {
                    *h *= k.force / (f * k.temperature);
                    absolute(ambient);
                }
                crate::thermal::HeatLoad::Point { x, y, z, q } => {
                    *x *= f;
                    *y *= f;
                    *z *= f;
                    *q *= k.force * f;
                }
            }
        }
        self.thickness *= f;
        scale_material(&mut self.material, k);
        self.region_materials.iter_mut().flatten().for_each(|m| scale_material(m, k));
        self.delta_t *= k.temperature;
        if let Geometry::Sketch { outer, holes, depth, regions, .. } = &mut self.geometry {
            scale_shape(outer, f);
            holes.iter_mut().for_each(|h| scale_shape(h, f));
            *depth *= f;
            for r in regions.iter_mut() {
                scale_shape(&mut r.outer, f);
                r.holes.iter_mut().for_each(|h| scale_shape(h, f));
            }
        }
        self.mesh.size *= f;
        for z in &mut self.mesh.refine {
            z.centre = [z.centre[0] * f, z.centre[1] * f];
            z.radius *= f;
            z.to = z.to.map(|t| [t[0] * f, t[1] * f]);
            z.blend = z.blend.map(|b| b * f);
        }
        for s in &mut self.supports {
            match s {
                Support::Edge { ux, uy, uz, .. } => {
                    [ux, uy, uz].into_iter().for_each(|u| *u = u.map(|v| v * f))
                }
                Support::Point { x, y, z, ux, uy, uz,
                } => {
                    (*x, *y, *z) = (*x * f, *y * f, *z * f);
                    [ux, uy, uz].into_iter().for_each(|u| *u = u.map(|v| v * f));
                }
            }
        }
        for l in &mut self.loads {
            match l {
                Load::Pressure { p, .. } => *p *= k.stress,
                Load::Traction { tx, ty, tz, .. } => {
                    (*tx, *ty, *tz) = (*tx * k.stress, *ty * k.stress, *tz * k.stress)
                }
                Load::Force { fx, fy, fz, .. } => {
                    (*fx, *fy, *fz) = (*fx * k.force, *fy * k.force, *fz * k.force)
                }
                Load::Bearing { fx, fy, .. } => (*fx, *fy) = (*fx * k.force, *fy * k.force),
                Load::Point { x, y, z, fx, fy, fz,
                } => {
                    (*x, *y, *z) = (*x * f, *y * f, *z * f);
                    (*fx, *fy, *fz) = (*fx * k.force, *fy * k.force, *fz * k.force);
                }
                // Force per volume: force over length cubed.
                Load::Body { bx, by, bz } => {
                    let g = k.force / f.powi(3);
                    (*bx, *by, *bz) = (*bx * g, *by * g, *bz * g);
                }
            }
        }
        for b in &mut self.bushings {
            b.inner_diameter *= f;
            b.offset = [b.offset[0] * f, b.offset[1] * f];
            b.interference *= f;
            scale_material(&mut b.material, k);
        }
        if let Some(h) = &mut self.history {
            for feat in &mut h.features {
                match &mut feat.kind {
                    FeatureKind::Sketch { plane, sketch } => {
                        scale_sketch(sketch, f);
                        match plane {
                            Plane::Front { offset } | Plane::Top { offset } | Plane::Right { offset } => *offset *= f,
                            Plane::Datum { origin, .. } => {
                                *origin = [origin[0] * f, origin[1] * f, origin[2] * f]
                            }
                        }
                    }
                    FeatureKind::Extrude { depth, .. } => *depth *= f,
                    FeatureKind::Revolve { .. } | FeatureKind::Sweep { .. } => {}
                }
            }
        }
        self.units = to;
        Ok(())
    }

    /// The same numbers, now said to be in `to`: the repair when a file or an entry was written in another system than
    /// the one declared. Nothing is rescaled.
    pub fn declare_units(&mut self, to: UnitSystem) {
        self.units = to;
    }

    /// The modulus of the material in pascals, for sanity checks that do not depend on the declared system.
    pub fn modulus_pa(&self) -> f64 {
        self.units.stress_to_pa(self.material.e)
    }
}
