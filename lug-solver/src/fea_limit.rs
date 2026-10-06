//! Ultimate capacity on the general kernel: the displacement-driven pin on the plastic lug, for any
//! load direction and with a bushing, returning the condensed solver's [`LimitLoad`] and the finite-strain
//! solver's [`FsResult`] so the toolbox can run either.

use crate::fea::{Drive, FeaLug, RunSpec, Tuning};
use crate::finite::{FsPoint, FsResult};
use crate::plastic::Hardening;
use crate::solve::{CurvePoint, LimitLoad, LimitOptions, LoadCase, PinSpec};
use fea_core::material::J2;
use fea_core::nonlinear::Stop;
use fea_core::NlSolution;
use std::time::Instant;

/// Pin load (whole lug, lbf) after every converged step: the master's force along the (rotated) travel.
fn loads(fea: &FeaLug, nl: &NlSolution) -> Vec<f64> {
    let share = if fea.half { 2.0 } else { 1.0 };
    nl.steps.iter().map(|s| share * s.master_force.first().map_or(0.0, |f| f[0])).collect()
}

impl FeaLug {
    /// Number of load steps for a travel of `cap` (`Tuning::step_fraction` of the bore radius each).
    fn collapse_steps(&self, cap: f64, tune: Tuning) -> usize {
        ((cap / (tune.step_fraction * self.bore_radius)).ceil() as usize).clamp(6, 80)
    }

    /// Run the displacement-driven pin on a lug with plasticity `j2` for `cap` of travel along the load direction.
    fn drive_pin(&self, pin: PinSpec, case: LoadCase, j2: J2, cap: f64, tweak: impl Fn(&mut fea_core::NlOptions)) -> Result<NlSolution, String> {
        if self.half && !case.is_axial() {
            return Err("a symmetric (half) model only supports axial loads".into());
        }
        let tune = self.tuning.unwrap_or(Tuning::collapse());
        let fit = if self.bushing.is_some() { Some(Self::fit_start(&self.fit_only(pin, tune, Some(j2))?)) } else { None };
        self.run(RunSpec { pin, dir: case.direction(), drive: Drive::Travel { cap }, tune, steps: self.collapse_steps(cap, tune), start: fit.as_ref(), plastic: Some(j2) }, tweak)
    }

    /// Collapse load of the elastic-perfectly-plastic (or hardening) lug: the kernel counterpart of
    /// [`LugModel::limit_load_with`](crate::LugModel::limit_load_with), valid for any load direction (a full
    /// model; the sideways pin translation is free and force-free) and with a bushing. `self` should be the
    /// plane-strain model (`PlaneMode::Strain`).
    pub fn limit_load(&self, pin: PinSpec, case: LoadCase, flow_stress: f64, options: LimitOptions) -> Result<LimitLoad, String> {
        if !(flow_stress.is_finite() && flow_stress > 0.0) {
            return Err("the flow stress must be positive".into());
        }
        let clock = Instant::now();
        let a = self.bore_radius;
        let cap = options.travel_cap_over_a * a;
        let law = match options.hardening {
            Some(h) => h,
            None => Hardening::linear(flow_stress, 0.0, 1.0)?,
        };
        let plateau_rule = options.stop_on_plateau && options.hardening.is_none();
        let strain_limit = options.strain_limit;
        let nl = self.drive_pin(pin, case, J2::small(law), cap, |o| {
            o.stop_on_plateau = plateau_rule;
            o.failure_ep = strain_limit;
        })?;
        let load = loads(self, &nl);
        let mut curve: Vec<CurvePoint> = nl.steps.iter().zip(&load).map(|(s, &p)| CurvePoint { travel: s.lambda * cap, load_lbf: p, plastic_fraction: s.plastic_fraction, max_equivalent_strain: s.max_ep }).collect();
        let mut strain_limited = false;
        let mut plateau = nl.stop == Stop::Plateau;
        if let (Stop::FailureStrain, Some(limit)) = (&nl.stop, strain_limit) {
            // Load at the ductility limit, interpolated on the strain between the last two points.
            if let [.., prev, last] = curve.as_mut_slice() {
                let f = ((limit - prev.max_equivalent_strain) / (last.max_equivalent_strain - prev.max_equivalent_strain).max(1e-300)).clamp(0.0, 1.0);
                *last = CurvePoint { travel: prev.travel + f * (last.travel - prev.travel), load_lbf: prev.load_lbf + f * (last.load_lbf - prev.load_lbf), plastic_fraction: last.plastic_fraction, max_equivalent_strain: limit };
            }
            strain_limited = true;
            plateau = true;
        }
        let limit_load_lbf = curve.iter().map(|c| c.load_lbf).fold(0.0, f64::max);
        if limit_load_lbf <= 0.0 {
            return Err("no load was carried".into());
        }
        let note = (!plateau).then(|| format!("the pin travel cap ({:.2} of the bore radius) ended the run before the load flattened: a lower bound", options.travel_cap_over_a));
        Ok(LimitLoad { limit_load_lbf, plateau, strain_limited, curve, flow_stress, plastic_iterations: nl.steps.iter().map(|s| s.iterations).sum(), max_plastic_strain: nl.state.max_ep(), elapsed_ms: clock.elapsed().as_secs_f64() * 1e3, note })
    }

    /// Plastic collapse load for several load directions, in parallel (the capacity envelope); `self` is the
    /// plane-strain model and a full one unless every angle is axial.
    pub fn collapse_sweep(&self, pin: PinSpec, flow_stress: f64, angles_deg: &[f64]) -> Vec<Result<LimitLoad, String>> {
        crate::solve::parallel_map(angles_deg, |&angle_deg| self.limit_load(pin, LoadCase { load_lbf: 1000.0, angle_deg }, flow_stress, LimitOptions::default()))
    }

    /// Finite-strain collapse with a true stress - plastic strain law: the kernel counterpart of
    /// [`FiniteLug::collapse`](crate::FiniteLug::collapse) (total-Lagrangian logarithmic strain, any load
    /// direction, with a bushing). The result is the peak load, or the load at `strain_limit`.
    pub fn finite_collapse(&self, pin: PinSpec, case: LoadCase, law: Hardening, strain_limit: Option<f64>, travel_cap_over_a: f64) -> Result<FsResult, String> {
        let clock = Instant::now();
        let a = self.bore_radius;
        let cap = travel_cap_over_a * a;
        let nl = self.drive_pin(pin, case, J2::finite(law), cap, |o| {
            o.failure_ep = strain_limit;
            // The load peaks and falls: read the peak once the curve is clearly past it.
            o.stop_on_fall = 0.04;
        })?;
        let load = loads(self, &nl);
        let mut curve: Vec<FsPoint> = nl.steps.iter().zip(&load).map(|(s, &p)| FsPoint { travel: s.lambda * cap, load_lbf: p, max_ep: s.max_ep }).collect();
        let mut strain_limited = false;
        if let (Stop::FailureStrain, Some(limit)) = (&nl.stop, strain_limit) {
            if let [.., prev, last] = curve.as_mut_slice() {
                let f = ((limit - prev.max_ep) / (last.max_ep - prev.max_ep).max(1e-300)).clamp(0.0, 1.0);
                *last = FsPoint { travel: prev.travel + f * (last.travel - prev.travel), load_lbf: prev.load_lbf + f * (last.load_lbf - prev.load_lbf), max_ep: limit };
            }
            strain_limited = true;
        }
        let peak = curve.iter().map(|c| c.load_lbf).fold(0.0, f64::max);
        if peak <= 0.0 {
            return Err("no load was carried".into());
        }
        let last = curve.last().map_or(0.0, |c| c.load_lbf);
        let peak_reached = nl.stop == Stop::Plateau || nl.stop == Stop::Completed && last < 0.99 * peak || !strain_limited && last < 0.99 * peak;
        let collapse_lbf = if strain_limited { last } else { peak };
        let note = (!peak_reached && !strain_limited).then(|| "the pin travel cap ended the run before the load peaked: a lower bound".to_string());
        Ok(FsResult { curve, collapse_lbf, peak_reached, strain_limited, max_ep: nl.state.max_ep(), newton_iterations: nl.steps.iter().map(|s| s.iterations).sum(), factorisations: nl.factorisations, elapsed_ms: clock.elapsed().as_secs_f64() * 1e3, note })
    }
}
