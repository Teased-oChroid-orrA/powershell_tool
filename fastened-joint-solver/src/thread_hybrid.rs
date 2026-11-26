//! Elastic finite-element load sharing for directly tapped Unified threads.
//! This is a 1D bolt/parent bar-and-spring model, not a flank-contact or
//! rupture analysis. It never changes `thread_capability` strength rows.

use crate::{
    thread::ThreadGeometry,
    thread_capability::{Case, Family},
    thread_load_distribution::{self, ThreadLoadDistributionInputs},
};

#[derive(Clone, Copy, Debug)]
pub struct ElasticInputs {
    pub bolt_e_psi: f64,
    pub bolt_nu: f64,
    pub parent_e_psi: f64,
    pub parent_nu: f64,
    /// Outer diameter of the idealized cylindrical parent boss, in.
    pub parent_outer_diameter_in: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ElasticResult {
    pub per_turn_lbf: Vec<f64>,
    pub engaged_turns: f64,
    pub last_turn_fraction: f64,
    pub peak_fraction: f64,
}

pub fn tapped_load_share(case: &Case, input: ElasticInputs) -> Result<ElasticResult, &'static str> {
    if case.family != Family::Tapped {
        return Err("elastic FE load sharing currently supports directly tapped threads only");
    }
    if !input.parent_outer_diameter_in.is_finite() || input.parent_outer_diameter_in <= case.nominal_major {
        return Err("parent boss outer diameter must exceed nominal thread major diameter");
    }
    if !case.axial_load.is_finite() || case.axial_load <= 0.0 {
        return Err("positive axial load is required for elastic FE");
    }
    if !case.tpi.is_finite() || case.tpi <= 0.0 || !case.inner.engagement.is_finite() {
        return Err("valid engagement and TPI are required for elastic FE");
    }
    let turns = case.inner.engagement * case.tpi;
    if !turns.is_finite() || !(0.0 < turns && turns <= 40.0) {
        return Err("elastic FE supports more than zero and at most 40 engaged turns");
    }
    let whole = turns.floor();
    let remainder = turns - whole;
    let (count, last_fraction) = if whole >= 1.0 && remainder < 1e-9 {
        (whole as u32, 1.0)
    } else {
        (whole as u32 + 1, remainder)
    };
    let thread = ThreadGeometry {
        d: case.nominal_major,
        d2: case.inner.pitch,
        d3: case.inner.minor,
        pitch: 1.0 / case.tpi,
        starts: 1,
        thread_angle_deg: 60.0,
    };
    let parent = ThreadLoadDistributionInputs {
        engaged_threads: count,
        nut_modulus: input.parent_e_psi,
        nut_poisson_ratio: input.parent_nu,
        nut_outer_diameter: input.parent_outer_diameter_in,
    };
    let solved = thread_load_distribution::compute_with_last_fraction(
        case.axial_load,
        &thread,
        input.bolt_e_psi,
        input.bolt_nu,
        &parent,
        last_fraction,
    ).ok_or("elastic FE could not solve this geometry or material")?;
    if solved.per_thread_loads.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("elastic FE returned invalid thread loads");
    }
    let sum: f64 = solved.per_thread_loads.iter().sum();
    if (sum / case.axial_load - 1.0).abs() > 1e-8 {
        return Err("elastic FE thread loads do not balance axial load");
    }
    Ok(ElasticResult {
        peak_fraction: solved.max_thread_load / case.axial_load,
        per_turn_lbf: solved.per_thread_loads,
        engaged_turns: turns,
        last_turn_fraction: last_fraction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_capability::Interface;

    fn case() -> Case {
        Case {
            family: Family::Tapped,
            nominal_major: 0.25,
            tpi: 28.0,
            inner: Interface::basic(0.25, 28.0, 0.375),
            outer: None,
            sti: None,
            axial_load: 3000.0,
            bolt_tensile_strength: 200_000.0,
            bolt_shear_strength: 100_000.0,
            parent_shear_strength: 40_000.0,
            insert_shear_strength: None,
            assembly_rating: None,
        }
    }

    fn input() -> ElasticInputs {
        ElasticInputs { bolt_e_psi: 29e6, bolt_nu: 0.29, parent_e_psi: 10.3e6, parent_nu: 0.33, parent_outer_diameter_in: 0.5 }
    }

    #[test]
    fn tapped_partial_turns_balance_and_damage_only_changes_matching_inputs() {
        let base = case();
        let a = tapped_load_share(&base, input()).unwrap();
        assert_eq!(a.per_turn_lbf.len(), 11);
        assert!((a.last_turn_fraction - 0.5).abs() < 1e-12);
        assert!((a.per_turn_lbf.iter().sum::<f64>() - 3000.0).abs() < 1e-6);
        let mut damaged = base.clone();
        damaged.inner.engagement = 0.5 / damaged.tpi;
        let short = tapped_load_share(&damaged, input()).unwrap();
        assert_eq!(short.per_turn_lbf.len(), 1);
        assert!((short.per_turn_lbf[0] - 3000.0).abs() < 1e-6);
        damaged = base.clone();
        damaged.inner.minor -= 0.005;
        let b = tapped_load_share(&damaged, input()).unwrap();
        assert_ne!(a.per_turn_lbf, b.per_turn_lbf);
        damaged = base.clone();
        damaged.inner.female_minor += 0.005;
        let c = tapped_load_share(&damaged, input()).unwrap();
        assert_eq!(a.per_turn_lbf, c.per_turn_lbf);
        damaged.family = Family::HelicalCoil;
        assert!(tapped_load_share(&damaged, input()).is_err());
    }
}
