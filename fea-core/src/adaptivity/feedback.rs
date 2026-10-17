//! AMR feedback for the strategy autotuner: what finished adaptive runs say about where the initial mesh of the next, similar
//! analysis should already be finer.
//!
//! Boundaries kept by construction (autotuning addendum, section 60):
//! * AMR stays the authority on resolution. Advice is a set of *initial size multipliers*, each in `[min_scale, 1]`: it can
//!   only ask for a finer start, never suppress a refinement the controller requests, and `run_amr` is not touched.
//! * Only validated feedback counts: a run is used when it stopped `Converged` and kept its element-wise fields.
//! * A class is identified by an exact [`WorkloadKey`] (geometry, material, boundary conditions, contact, quantities of
//!   interest): histories of a different key are never pooled, so a mesh is never inferred accurate for a merely similar
//!   problem.
//! * Without enough agreeing evidence there is no advice (`None`), and the deterministic default start applies.
//!
//! The demand of a zone in one run is the sum over its cycles of the fraction of that zone's elements that were marked: about
//! the number of refinements an element of the zone went through. The advice for a zone pre-applies part of the typical demand
//! (`depth_fraction`), leaving the rest to AMR.

use super::{
    controller::{AmrResult, CycleFields, Stop},
    zones::Zone,
};
use std::collections::BTreeMap;

/// The inputs that must all match for two runs to be comparable. The caller supplies canonical descriptors (for example a
/// hash of the geometry, the material name and parameters, a digest of the supports and loads, the contact pairs, the goal
/// names).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkloadKey {
    pub geometry: String,
    pub material: String,
    pub boundary_conditions: String,
    pub contact: String,
    pub qois: Vec<String>,
}

/// Why a run ended, reduced to what feedback needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    Converged,
    BudgetLimited,
    Stagnated,
    Other,
}

/// What one adaptive run says.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub key: WorkloadKey,
    pub cycles: usize,
    pub end: EndKind,
    pub initial_elements: usize,
    pub final_elements: usize,
    pub initial_dofs: usize,
    pub final_dofs: usize,
    /// `final_dofs / initial_dofs`.
    pub dof_growth: f64,
    /// Physics (relative energy-norm) error estimate of the first and last cycle.
    pub initial_error: f64,
    pub final_error: f64,
    /// Geometric mean error reduction per cycle (`< 1` is progress); `None` with a single cycle or a zero first error.
    pub error_reduction_per_cycle: Option<f64>,
    pub total_seconds: f64,
    /// Refinement demand per zone name (`Debug` of [`Zone`]); empty when the element-wise fields were not kept for every
    /// cycle.
    pub zone_demand: BTreeMap<String, f64>,
    /// Elements per zone in the first kept cycle (to weigh how much of the mesh a zone is).
    pub zone_elements: BTreeMap<String, usize>,
    pub has_regional_detail: bool,
}

impl RunSummary {
    /// A run whose feedback may be used: converged, with regional detail.
    pub fn validated(&self) -> bool {
        self.end == EndKind::Converged && self.has_regional_detail
    }
}

fn demand_of(fields: &[CycleFields]) -> (BTreeMap<String, f64>, BTreeMap<String, usize>) {
    let mut demand: BTreeMap<String, f64> = BTreeMap::new();
    let mut first_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (i, f) in fields.iter().enumerate() {
        let mut count: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for (e, z) in f.zones.iter().enumerate() {
            let c = count.entry(format!("{z:?}")).or_default();
            c.0 += 1;
            if f.marked.get(e).copied().unwrap_or(false) {
                c.1 += 1;
            }
        }
        for (name, (n, marked)) in count {
            *demand.entry(name.clone()).or_default() += marked as f64 / n as f64;
            if i == 0 {
                first_counts.insert(name, n);
            }
        }
    }
    (demand, first_counts)
}

/// Summarise a finished run for the class `key`.
pub fn summarize(key: WorkloadKey, result: &AmrResult) -> RunSummary {
    let h = &result.history;
    let (first, last) = (h.first(), h.last());
    let (initial_dofs, final_dofs) = (first.map_or(0, |c| c.dofs), last.map_or(0, |c| c.dofs));
    let (initial_error, final_error) = (
        first.map_or(f64::NAN, |c| c.physics_error),
        last.map_or(f64::NAN, |c| c.physics_error),
    );
    let reduction =
        (h.len() > 1 && initial_error > 0. && final_error.is_finite() && final_error > 0.)
            .then(|| (final_error / initial_error).powf(1. / (h.len() - 1) as f64));
    let end = match &result.stop {
        Stop::Converged => EndKind::Converged,
        Stop::BudgetExhausted => EndKind::BudgetLimited,
        Stop::Stagnated(_) => EndKind::Stagnated,
        _ => EndKind::Other,
    };
    // Regional detail needs every cycle but the last (the last marks nothing) to have kept its fields.
    let has_regional_detail = !result.fields.is_empty()
        && result.fields.len() + 1 >= h.len()
        && result.fields.iter().all(|f| !f.zones.is_empty());
    let (zone_demand, zone_elements) = if has_regional_detail {
        demand_of(&result.fields)
    } else {
        Default::default()
    };
    RunSummary {
        key,
        cycles: h.len(),
        end,
        initial_elements: first.map_or(0, |c| c.elements),
        final_elements: last.map_or(0, |c| c.elements),
        initial_dofs,
        final_dofs,
        dof_growth: if initial_dofs > 0 {
            final_dofs as f64 / initial_dofs as f64
        } else {
            f64::NAN
        },
        initial_error,
        final_error,
        error_reduction_per_cycle: reduction,
        total_seconds: h
            .iter()
            .map(|c| {
                c.seconds.solve
                    + c.seconds.goals
                    + c.seconds.physics
                    + c.seconds.fusion
                    + c.seconds.marking_and_sizing
                    + c.seconds.meshing
                    + c.seconds.checks
            })
            .sum(),
        zone_demand,
        zone_elements,
        has_regional_detail,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AdviceOptions {
    /// Validated runs of the same key needed before any advice is given.
    pub min_runs: usize,
    /// Largest coefficient of variation of a zone's demand across runs for the zone to get advice.
    pub max_cv: f64,
    /// The controller's `refine_ratio`: one refinement scales the size by this.
    pub refine_ratio: f64,
    /// Share of the typical demand pre-applied to the initial mesh (the rest is left to AMR).
    pub depth_fraction: f64,
    /// Finest initial multiplier ever advised.
    pub min_scale: f64,
}

impl Default for AdviceOptions {
    fn default() -> Self {
        Self {
            min_runs: 3,
            max_cv: 0.5,
            refine_ratio: 0.5,
            depth_fraction: 0.5,
            min_scale: 0.25,
        }
    }
}

/// Advice for the initial mesh of one class of analyses.
#[derive(Debug, Clone, PartialEq)]
pub struct Advice {
    pub key: WorkloadKey,
    pub runs_used: usize,
    /// Initial size multiplier per zone name, each in `[min_scale, 1]`; zones without consistent evidence are absent.
    pub zone_scale: BTreeMap<String, f64>,
    pub expected_cycles: f64,
    pub expected_dof_growth: f64,
    pub expected_seconds: f64,
}

impl Advice {
    /// The multiplier for a zone (`1`: no advice, start at the default size).
    pub fn scale_for(&self, zone: Zone) -> f64 {
        self.zone_scale
            .get(&format!("{zone:?}"))
            .copied()
            .unwrap_or(1.0)
    }
}

/// Advice for `key` from the validated runs of exactly that key, or `None` when there are too few (the deterministic default
/// start then applies).
pub fn advise(
    history: &[RunSummary],
    key: &WorkloadKey,
    options: &AdviceOptions,
) -> Option<Advice> {
    let runs: Vec<&RunSummary> = history
        .iter()
        .filter(|r| &r.key == key && r.validated())
        .collect();
    if runs.len() < options.min_runs.max(1) {
        return None;
    }
    let mut zones: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for r in &runs {
        for (z, d) in &r.zone_demand {
            zones.entry(z.as_str()).or_default().push(*d);
        }
    }
    let mut zone_scale = BTreeMap::new();
    for (z, demand) in zones {
        if demand.len() < options.min_runs.max(1) {
            continue;
        }
        let mean = demand.iter().sum::<f64>() / demand.len() as f64;
        if mean.is_nan() || mean <= 0. {
            continue;
        }
        let var = demand.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / demand.len() as f64;
        if var.sqrt() / mean > options.max_cv {
            continue;
        }
        let scale = options
            .refine_ratio
            .powf(options.depth_fraction * mean)
            .clamp(options.min_scale, 1.0);
        if scale < 1.0 {
            zone_scale.insert(z.to_string(), scale);
        }
    }
    let n = runs.len() as f64;
    Some(Advice {
        key: key.clone(),
        runs_used: runs.len(),
        zone_scale,
        expected_cycles: runs.iter().map(|r| r.cycles as f64).sum::<f64>() / n,
        expected_dof_growth: runs.iter().map(|r| r.dof_growth).sum::<f64>() / n,
        expected_seconds: runs.iter().map(|r| r.total_seconds).sum::<f64>() / n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(geometry: &str) -> WorkloadKey {
        WorkloadKey {
            geometry: geometry.into(),
            material: "steel".into(),
            boundary_conditions: "bc1".into(),
            contact: "none".into(),
            qois: vec!["compliance".into()],
        }
    }

    fn run(k: &WorkloadKey, bearing: f64, smooth: f64, end: EndKind) -> RunSummary {
        RunSummary {
            key: k.clone(),
            cycles: 5,
            end,
            initial_elements: 100,
            final_elements: 800,
            initial_dofs: 200,
            final_dofs: 1600,
            dof_growth: 8.0,
            initial_error: 0.2,
            final_error: 0.02,
            error_reduction_per_cycle: Some(0.56),
            total_seconds: 4.0,
            zone_demand: BTreeMap::from([
                ("FastenerBearing".to_string(), bearing),
                ("Smooth".to_string(), smooth),
            ]),
            zone_elements: BTreeMap::new(),
            has_regional_detail: true,
        }
    }

    #[test]
    fn consistent_validated_runs_of_one_key_advise_a_finer_start_only_where_demand_is_high() {
        let k = key("joint-A");
        let history = vec![
            run(&k, 2.0, 0.1, EndKind::Converged),
            run(&k, 2.2, 0.12, EndKind::Converged),
            run(&k, 1.8, 0.09, EndKind::Converged),
        ];
        let a = advise(&history, &k, &AdviceOptions::default()).expect("three agreeing runs");
        assert_eq!(a.runs_used, 3);
        // bearing demand ~2 -> 0.5^(0.5*2) = 0.5; smooth demand 0.1 -> scale 0.5^0.05 ~ 0.966
        let bearing = a.scale_for(Zone::FastenerBearing);
        assert!((bearing - 0.5).abs() < 0.03, "{bearing}");
        assert!(a.scale_for(Zone::Smooth) > 0.95 && a.scale_for(Zone::Smooth) < 1.0);
        assert_eq!(
            a.scale_for(Zone::Contact),
            1.0,
            "a zone nobody saw gets no advice"
        );
        // every multiplier is a refinement request, never a coarsening and never beyond the floor
        assert!(a.zone_scale.values().all(|s| *s >= 0.25 && *s <= 1.0));
        assert_eq!(
            (a.expected_cycles, a.expected_dof_growth, a.expected_seconds),
            (5.0, 8.0, 4.0)
        );
    }

    #[test]
    fn only_validated_runs_of_exactly_the_same_key_count() {
        let (k, other) = (key("joint-A"), key("joint-B"));
        let mut unconverged = run(&k, 2.0, 0.1, EndKind::BudgetLimited);
        let mut no_detail = run(&k, 2.0, 0.1, EndKind::Converged);
        no_detail.has_regional_detail = false;
        unconverged.end = EndKind::Stagnated;
        let history = vec![
            run(&k, 2.0, 0.1, EndKind::Converged),
            run(&k, 2.0, 0.1, EndKind::Converged),
            unconverged,
            no_detail,
            run(&other, 2.0, 0.1, EndKind::Converged),
            run(&other, 2.0, 0.1, EndKind::Converged),
            run(&other, 2.0, 0.1, EndKind::Converged),
        ];
        // only two validated runs of `k` (the others are unconverged, detail-less or of another key): not enough
        assert!(advise(&history, &k, &AdviceOptions::default()).is_none());
        // the other key has three
        assert_eq!(
            advise(&history, &other, &AdviceOptions::default())
                .unwrap()
                .runs_used,
            3
        );
        // a similar but different key (one descriptor changed) is a different class
        let mut similar = k.clone();
        similar.material = "aluminium".into();
        assert!(advise(&history, &similar, &AdviceOptions::default()).is_none());
        assert!(advise(&[], &k, &AdviceOptions::default()).is_none());
    }

    #[test]
    fn disagreeing_runs_give_no_advice_for_that_zone() {
        let k = key("joint-A");
        let history = vec![
            run(&k, 0.2, 0.1, EndKind::Converged),
            run(&k, 3.0, 0.1, EndKind::Converged),
            run(&k, 0.1, 0.1, EndKind::Converged),
        ];
        let a = advise(&history, &k, &AdviceOptions::default()).unwrap();
        assert!(
            !a.zone_scale.contains_key("FastenerBearing"),
            "{:?}",
            a.zone_scale
        );
        assert_eq!(a.scale_for(Zone::FastenerBearing), 1.0);
    }

    #[test]
    fn the_floor_bounds_even_extreme_demand() {
        let k = key("joint-A");
        let history = vec![run(&k, 40.0, 0.1, EndKind::Converged); 3];
        let a = advise(&history, &k, &AdviceOptions::default()).unwrap();
        assert_eq!(a.scale_for(Zone::FastenerBearing), 0.25);
    }
}
