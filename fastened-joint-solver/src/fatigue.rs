//! Bolt fatigue screen after VDI 2230: the alternating stress the external axial load range puts in the bolt against the
//! thread's endurance amplitude. Elastic and single-bolt: no thermal load, no settlement, no eccentric clamping.
//!
//! Sources (formulas as given by the FVA knowledge base page "root bolt connection", doc.fva-service.de, and
//! the TU Magdeburg paper journals.ub.ovgu.de techmech 2206; VDI 2230 itself was not available here, check the edition):
//! - alternating stress `sigma_a = Phi_en (F_A,max - F_A,min) / (2 A_S)`, with the load-introduction factor `Phi_en = n Phi`
//!   (`Phi` the joint's load fraction, `n` how far from the head / nut the load enters);
//! - endurance amplitude of a thread rolled before heat treatment `sigma_ASV = 0.85 (150 / d + 45)` (N/mm^2, `d` the nominal
//!   diameter in mm), for the endurance cycle count of VDI 2230 (2 x 10^6);
//! - rolled after heat treatment `sigma_ASG = (2 - F_Sm / F_0.2min) sigma_ASV`, with the mean bolt force
//!   `F_Sm = F_V + Phi_en (F_A,max + F_A,min) / 2` and `F_0.2min = R_p0.2 A_S` (no thermal part here).
//!
//! The formulas are metric; this module takes the crate's in / lbf / psi and converts only the endurance amplitude.
//! The margin of safety is `sigma_AS / sigma_a - 1` (the safety factor `S_D = sigma_AS / sigma_a` minus one).
//! Measured lives are often shorter than this prediction (the TU Magdeburg paper cites a factor 2 to 4): a screen, not a proof.

const MM_PER_IN: f64 = 25.4;
const PSI_PER_MPA: f64 = 145.037_737_730_209_2;

/// How the thread was made, which sets the endurance amplitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRolling {
    /// Rolled before heat treatment (`sigma_ASV`): independent of the preload.
    BeforeHeatTreatment,
    /// Rolled after heat treatment (`sigma_ASG`): the endurance falls as the mean bolt force nears the yield force.
    AfterHeatTreatment,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FatigueInputs {
    /// Lowest and highest external axial load on the joint (lbf), `force_max >= force_min`.
    pub force_min: f64,
    pub force_max: f64,
    /// Load-introduction factor `n` in (0, 1]: 1 loads the joint at the bolt head and nut faces, smaller values introduce
    /// the load nearer the joint's interior (VDI 2230 gives a table; 0.5 is a mid-range assumption, not a measured value).
    pub load_introduction_factor: f64,
    pub thread_rolling: ThreadRolling,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FatigueResult {
    /// `Phi_en = n Phi`, the share of the external load range the bolt sees.
    pub bolt_load_fraction: f64,
    /// `sigma_a` (psi).
    pub alternating_stress: f64,
    /// Mean bolt axial stress over the tensile stress area, preload included (psi).
    pub mean_stress: f64,
    /// `sigma_AS` (psi): `sigma_ASV` or `sigma_ASG`.
    pub endurance_amplitude: f64,
    /// `S_D = sigma_AS / sigma_a` (infinite when the load does not vary).
    pub safety_factor: f64,
    /// `S_D - 1`.
    pub margin: f64,
}

/// `sigma_ASV` in psi for a nominal diameter in inches.
pub fn endurance_rolled_before(nominal_diameter: f64) -> f64 {
    0.85 * (150.0 / (nominal_diameter * MM_PER_IN) + 45.0) * PSI_PER_MPA
}

/// `sigma_ASG` in psi: `sigma_ASV` scaled by `2 - F_Sm / F_0.2min` (not below zero).
pub fn endurance_rolled_after(nominal_diameter: f64, mean_force: f64, yield_force: f64) -> f64 {
    (2.0 - mean_force / yield_force).max(0.0) * endurance_rolled_before(nominal_diameter)
}

/// The screen for a preload `preload`, joint load fraction `phi`, tensile stress area `area` and nominal diameter `d`.
/// `yield_strength` (`R_p0.2`, psi) is needed only for [`ThreadRolling::AfterHeatTreatment`]; without it that case is `None`.
pub fn evaluate(inputs: &FatigueInputs, preload: f64, phi: f64, area: f64, nominal_diameter: f64, yield_strength: Option<f64>) -> Option<FatigueResult> {
    let phi_en = inputs.load_introduction_factor * phi;
    let alternating_stress = phi_en * (inputs.force_max - inputs.force_min) / (2.0 * area);
    let mean_force = preload + phi_en * 0.5 * (inputs.force_max + inputs.force_min);
    let endurance_amplitude = match inputs.thread_rolling {
        ThreadRolling::BeforeHeatTreatment => endurance_rolled_before(nominal_diameter),
        ThreadRolling::AfterHeatTreatment => endurance_rolled_after(nominal_diameter, mean_force, yield_strength? * area),
    };
    let safety_factor = if alternating_stress > 0.0 { endurance_amplitude / alternating_stress } else { f64::INFINITY };
    Some(FatigueResult { bolt_load_fraction: phi_en, alternating_stress, mean_stress: mean_force / area, endurance_amplitude, safety_factor, margin: safety_factor - 1.0 })
}

/// Why a [`FatigueInputs`] is refused.
pub fn check(inputs: &FatigueInputs) -> Result<(), &'static str> {
    let finite = inputs.force_min.is_finite() && inputs.force_max.is_finite() && inputs.load_introduction_factor.is_finite();
    if !finite {
        return Err("fatigue input is not finite");
    }
    if inputs.force_max < inputs.force_min {
        return Err("fatigue maximum load is below the minimum");
    }
    if !(inputs.load_introduction_factor > 0.0 && inputs.load_introduction_factor <= 1.0) {
        return Err("load-introduction factor must be in (0, 1]");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(rolling: ThreadRolling) -> FatigueInputs {
        FatigueInputs { force_min: 0.0, force_max: 4000.0, load_introduction_factor: 0.5, thread_rolling: rolling }
    }

    #[test]
    fn the_endurance_amplitude_of_an_m10_bolt_matches_the_formula_in_mpa() {
        // d = 10 mm: 0.85 (15 + 45) = 51 N/mm^2.
        let psi = endurance_rolled_before(10.0 / 25.4);
        assert!((psi / PSI_PER_MPA - 51.0).abs() < 1e-9, "{}", psi / PSI_PER_MPA);
    }

    #[test]
    fn alternating_stress_is_half_the_bolt_share_of_the_load_range_over_the_area() {
        // Phi 0.2, n 0.5: the bolt sees 10 % of 4000 lbf = 400 lbf; half of it over 0.05 in^2 = 4000 psi.
        let r = evaluate(&inputs(ThreadRolling::BeforeHeatTreatment), 6000.0, 0.2, 0.05, 0.25, None).unwrap();
        assert!((r.bolt_load_fraction - 0.1).abs() < 1e-12);
        assert!((r.alternating_stress - 4000.0).abs() < 1e-9, "{}", r.alternating_stress);
        // mean force 6000 + 0.1 x 2000 = 6200 lbf.
        assert!((r.mean_stress - 6200.0 / 0.05).abs() < 1e-9);
        assert!((r.margin - (r.endurance_amplitude / 4000.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn a_constant_load_has_no_alternating_stress_and_an_infinite_margin() {
        let mut i = inputs(ThreadRolling::BeforeHeatTreatment);
        i.force_min = 4000.0;
        let r = evaluate(&i, 6000.0, 0.2, 0.05, 0.25, None).unwrap();
        assert_eq!(r.alternating_stress, 0.0);
        assert!(r.margin.is_infinite());
    }

    #[test]
    fn rolling_after_heat_treatment_loses_endurance_as_the_preload_nears_yield() {
        let yield_strength = 120_000.0;
        let area = 0.05; // yield force 6000 lbf
        let low = evaluate(&inputs(ThreadRolling::AfterHeatTreatment), 1000.0, 0.2, area, 0.25, Some(yield_strength)).unwrap();
        let high = evaluate(&inputs(ThreadRolling::AfterHeatTreatment), 5000.0, 0.2, area, 0.25, Some(yield_strength)).unwrap();
        assert!(low.endurance_amplitude > high.endurance_amplitude);
        // At F_Sm = 0.5 F_0.2 the factor is 1.5.
        let r = evaluate(&FatigueInputs { force_min: 0.0, force_max: 0.0, ..inputs(ThreadRolling::AfterHeatTreatment) }, 3000.0, 0.2, area, 0.25, Some(yield_strength)).unwrap();
        assert!((r.endurance_amplitude / endurance_rolled_before(0.25) - 1.5).abs() < 1e-12);
        // Same preload at a larger yield strength endures more; at the yield force the factor is 1.
        let at_yield = evaluate(&FatigueInputs { force_min: 0.0, force_max: 0.0, ..inputs(ThreadRolling::AfterHeatTreatment) }, 6000.0, 0.2, area, 0.25, Some(yield_strength)).unwrap();
        assert!((at_yield.endurance_amplitude / endurance_rolled_before(0.25) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rolled_after_without_a_yield_strength_is_none_and_rolled_before_does_not_need_one() {
        assert!(evaluate(&inputs(ThreadRolling::AfterHeatTreatment), 1000.0, 0.2, 0.05, 0.25, None).is_none());
        assert!(evaluate(&inputs(ThreadRolling::BeforeHeatTreatment), 1000.0, 0.2, 0.05, 0.25, None).is_some());
    }

    #[test]
    fn a_bigger_bolt_has_a_lower_endurance_amplitude() {
        assert!(endurance_rolled_before(0.5) < endurance_rolled_before(0.25));
    }

    #[test]
    fn bad_inputs_are_refused() {
        let ok = inputs(ThreadRolling::BeforeHeatTreatment);
        assert!(check(&ok).is_ok());
        assert!(check(&FatigueInputs { force_max: -1.0, ..ok }).is_err());
        assert!(check(&FatigueInputs { load_introduction_factor: 0.0, ..ok }).is_err());
        assert!(check(&FatigueInputs { load_introduction_factor: 1.5, ..ok }).is_err());
        assert!(check(&FatigueInputs { force_max: f64::NAN, ..ok }).is_err());
    }
}
