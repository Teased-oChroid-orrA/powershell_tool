//! Non-dimensional formulation. The finite-element problem is built and solved with every length divided by the bore
//! radius `l`, every stress and modulus by the housing modulus `e` and every force by `f = e l^2` (the thickness is a
//! length), so the stiffness, penalty and load entries are O(1) whatever the units or the size of the part, and the solver's
//! tolerances mean the same thing for a 0.25 in and a 2 in bore. The results are mapped back to inch / lbf / psi
//! here. Scaling an already scaled input is the identity (`l = e = 1`), so a search may call the public entry points on
//! scaled inputs.

use crate::model::{Analysis, Elasticity, Inputs, Orthotropy, ProfileBin};
use crate::offset::OffsetLimit;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Scale {
    /// Length unit, in.
    pub l: f64,
    /// Stress unit, psi.
    pub e: f64,
}

impl Scale {
    pub fn of(inp: &Inputs) -> Self {
        Self { l: inp.bore_radius(), e: inp.housing.e_psi }
    }

    /// Force unit, lbf.
    pub fn f(&self) -> f64 {
        self.e * self.l * self.l
    }

    /// Torque unit, lbf in.
    pub fn t(&self) -> f64 {
        self.f() * self.l
    }

    fn elasticity(&self, m: &Elasticity) -> Elasticity {
        Elasticity { e_psi: m.e_psi / self.e, nu: m.nu, ortho: m.ortho.map(|o| Orthotropy { e2_psi: o.e2_psi / self.e, g12_psi: o.g12_psi / self.e, ..o }) }
    }

    /// The inputs in units of `l`, `e` and `f`.
    pub fn inputs(&self, i: &Inputs) -> Inputs {
        Inputs {
            bore_dia: i.bore_dia / self.l,
            housing_od: i.housing_od / self.l,
            edge_distance: i.edge_distance.map(|e| e / self.l),
            bushing_id: i.bushing_id / self.l,
            offset: i.offset / self.l,
            interference_dia: i.interference_dia / self.l,
            thickness: i.thickness / self.l,
            housing: self.elasticity(&i.housing),
            bushing: self.elasticity(&i.bushing),
            pin: self.elasticity(&i.pin),
            pin_clearance_dia: i.pin_clearance_dia / self.l,
            load_lbf: i.load_lbf / self.f(),
            min_wall: i.min_wall / self.l,
            mesh_size: i.mesh_size.map(|m| m / self.l),
            ..*i
        }
    }

    /// A result of the scaled problem in inch / lbf / psi.
    pub fn analysis(&self, mut a: Analysis) -> Analysis {
        let (l, e, f, t) = (self.l, self.e, self.f(), self.t());
        a.offset *= l;
        a.torque_required *= t;
        a.torque_capacity_fit *= t;
        a.torque_capacity *= t;
        a.design_capacity *= t;
        a.pin_peak_pressure *= e;
        a.onset_torque = a.onset_torque.map(|v| v * t);
        a.ground_leak *= f;
        a.friction_torque *= t;
        a.net_force = [a.net_force[0] * f, a.net_force[1] * f];
        a.fit_pressure_mean *= e;
        a.fit_pressure_min *= e;
        a.fit_pressure_max *= e;
        a.wall_thin *= l;
        a.wall_thick *= l;
        a.profile = a.profile.iter().map(|b| ProfileBin { fit: b.fit * e, loaded: b.loaded * e, ..*b }).collect();
        a
    }

    /// A search result over offsets (`lengths`) or over loads.
    pub fn limit(&self, mut v: OffsetLimit, lengths: bool) -> OffsetLimit {
        let k = if lengths { self.l } else { self.f() };
        v.value *= k;
        v.upper = v.upper.map(|u| u * k);
        v.wall_limit *= self.l;
        v
    }

    /// Torque.
    pub fn torque(&self, v: f64) -> f64 {
        v * self.t()
    }
}
