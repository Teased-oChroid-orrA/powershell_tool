//! Multicriteria error fusion: turn the goal-oriented and the physics indicators into one non-negative refinement-ranking score
//! per element, plus the set of elements that must be refined whatever the ranking says.
//!
//! The score is a dimensionless *ranking*, not an error bound. With normalised indicators `a` (adjoint) and `p` (physics) and the
//! spatial physics weight `beta_e` of the zone:
//!
//! - **weighted sum** `alpha a + (1 - alpha) beta p`,
//! - **weighted maximum** `max(alpha a, (1 - alpha) beta p)`,
//! - **constraint-preserving** (the intended production strategy): the ranking is the goal-oriented one, `a`, but an element
//!   whose physics indicator exceeds the stated per-element allowance is *mandatory* and ranks above every other element, so a
//!   region whose physics error is unresolved is refined even where the adjoint says it does not matter for the goal.
//!
//! Dimensioned indicators are never modified: normalisation produces separate fields. A failed or unsupported estimator is
//! never silently dropped: the fusion degrades to the other criterion with an explicit `Degraded` result (or errors in strict
//! mode / when nothing is left).

use super::estimator::{ErrorField, Reliability};
use super::zones::{PhysicsEstimator, Zone, ZoneMap};
use crate::meshing::MeshError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Normalization {
    /// Divide by the largest value.
    Max,
    /// Divide by the root mean square.
    Rms,
    /// Divide by the given percentile (nearest rank, `0 < p <= 100`): robust to a single singular peak.
    Percentile(f64),
}

/// Result of normalising one field.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalised {
    pub scale: f64,
    pub values: Vec<f64>,
}

/// Normalise non-negative indicators. An identically zero field has scale `0` and normalises to zeros; a non-finite value is an
/// error (callers pass fields already checked for reliability).
pub fn normalise(values: &[f64], how: Normalization) -> Result<Normalised, MeshError> {
    if let Some(i) = values.iter().position(|v| !v.is_finite() || *v < 0.0) {
        return Err(MeshError::Failed(format!("indicator {i} is {} (indicators must be finite and non-negative)", values[i])));
    }
    let scale = match how {
        Normalization::Max => values.iter().cloned().fold(0.0f64, f64::max),
        Normalization::Rms => {
            if values.is_empty() {
                0.0
            } else {
                (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt()
            }
        }
        Normalization::Percentile(p) => {
            if !(p > 0.0 && p <= 100.0) {
                return Err(MeshError::Failed(format!("percentile {p} is outside (0, 100]")));
            }
            if values.is_empty() {
                0.0
            } else {
                let mut s = values.to_vec();
                s.sort_by(f64::total_cmp);
                let rank = ((p / 100.0) * s.len() as f64).ceil().max(1.0) as usize;
                s[rank - 1]
            }
        }
    };
    if scale <= 0.0 {
        return Ok(Normalised { scale: 0.0, values: vec![0.0; values.len()] });
    }
    Ok(Normalised { scale, values: values.iter().map(|v| v / scale).collect() })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FusionStrategy {
    WeightedSum,
    WeightedMax,
    ConstraintPreserving,
}

/// How several goals combine into the adjoint criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalCombine {
    /// Weighted sum of the normalised goal indicators.
    Sum,
    /// Weighted maximum: an element matters if it matters for any goal.
    Max,
}

#[derive(Debug, Clone)]
pub struct FusionConfig {
    pub strategy: FusionStrategy,
    /// Global adjoint priority `alpha` in `[0, 1]`.
    pub alpha: f64,
    pub normalisation: Normalization,
    pub goal_combine: GoalCombine,
    /// Largest physics indicator an element may carry (after the zone weight) before it is mandatory; `None`: no requirement.
    pub physics_allowance: Option<f64>,
    /// Error out instead of degrading when an estimator is unusable.
    pub strict: bool,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self { strategy: FusionStrategy::ConstraintPreserving, alpha: 0.7, normalisation: Normalization::Max, goal_combine: GoalCombine::Max, physics_allowance: None, strict: false }
    }
}

pub struct FusionInput<'a> {
    /// Goal-oriented fields with their weights (signed fields are taken in absolute value).
    pub adjoint: &'a [(f64, &'a ErrorField)],
    /// Physics fields by the estimator that produced them.
    pub physics: &'a [(PhysicsEstimator, &'a ErrorField)],
    pub zones: Option<&'a ZoneMap>,
    /// Elements that may not be refined further (size or level cap): they leave the ranking and are never mandatory, so an
    /// isolated singularity cannot demand refinement without end.
    pub capped: Option<&'a [bool]>,
    /// Elements that must be refined whatever the estimators say (a contact patch below its required resolution, a region the
    /// user protected). Applied by every strategy; a capped element still wins over a protected one.
    pub protected: Option<&'a [bool]>,
}

#[derive(Debug, Clone)]
pub struct Fused {
    /// Ranking score `>= 0`; mandatory elements are above every non-mandatory one.
    pub score: Vec<f64>,
    pub mandatory: Vec<bool>,
    /// Normalised criteria actually used (zeros where a criterion was unavailable).
    pub adjoint_norm: Vec<f64>,
    pub physics_norm: Vec<f64>,
    pub adjoint_scale: f64,
    pub physics_scale: f64,
    pub used_adjoint: bool,
    pub used_physics: bool,
    pub reliability: Reliability,
    pub notes: Vec<String>,
}

fn usable(f: &ErrorField, n: usize) -> Result<(), String> {
    if f.values.len() != n {
        return Err(format!("{} has {} values for {} elements", f.name, f.values.len(), n));
    }
    match &f.reliability {
        Reliability::Reliable | Reliability::Degraded(_) => Ok(()),
        Reliability::Failed(w) => Err(format!("{} failed: {w}", f.name)),
        Reliability::Unsupported(w) => Err(format!("{} unsupported: {w}", f.name)),
    }
}

/// Fuse the criteria into a ranking.
pub fn fuse(input: &FusionInput, cfg: &FusionConfig) -> Result<Fused, MeshError> {
    if !(0.0..=1.0).contains(&cfg.alpha) {
        return Err(MeshError::Failed(format!("alpha {} is outside [0, 1]", cfg.alpha)));
    }
    let n = input.adjoint.first().map(|f| f.1.values.len()).or_else(|| input.physics.first().map(|f| f.1.values.len())).ok_or_else(|| MeshError::Failed("no indicator fields to fuse".into()))?;
    let mut notes = Vec::new();
    let mut degraded: Vec<String> = Vec::new();

    // adjoint criterion
    let mut adj_parts: Vec<(f64, Normalised)> = Vec::new();
    for (w, f) in input.adjoint {
        match usable(f, n) {
            Ok(()) => {
                if let Reliability::Degraded(why) = &f.reliability {
                    degraded.push(format!("{}: {why}", f.name));
                }
                adj_parts.push((*w, normalise(&f.marking(), cfg.normalisation)?));
            }
            Err(why) => {
                if cfg.strict {
                    return Err(MeshError::Failed(why));
                }
                degraded.push(why);
            }
        }
    }
    let used_adjoint = !adj_parts.is_empty();
    let adjoint_scale = adj_parts.iter().map(|p| p.1.scale).fold(0.0f64, f64::max);
    let mut adjoint_norm = vec![0.0; n];
    if used_adjoint {
        let wsum: f64 = adj_parts.iter().map(|p| p.0).sum();
        let wmax = adj_parts.iter().map(|p| p.0).fold(0.0f64, f64::max);
        if wsum <= 0.0 || adj_parts.iter().any(|p| p.0 < 0.0) {
            return Err(MeshError::Failed("the goal weights must be non-negative and not all zero".into()));
        }
        for (e, slot) in adjoint_norm.iter_mut().enumerate() {
            *slot = match cfg.goal_combine {
                GoalCombine::Sum => adj_parts.iter().map(|(w, p)| w * p.values[e]).sum::<f64>() / wsum,
                // weights relative to the largest, so equal weights give the plain maximum
                GoalCombine::Max => adj_parts.iter().map(|(w, p)| w / wmax * p.values[e]).fold(0.0f64, f64::max),
            };
        }
    }

    // physics criterion: per element, the field of its zone's estimator, else the best available one
    let mut phys_norms: Vec<(PhysicsEstimator, Normalised, Vec<f64>)> = Vec::new();
    for (kind, f) in input.physics {
        match usable(f, n) {
            Ok(()) => {
                if let Reliability::Degraded(why) = &f.reliability {
                    degraded.push(format!("{}: {why}", f.name));
                }
                phys_norms.push((*kind, normalise(&f.marking(), cfg.normalisation)?, f.marking()));
            }
            Err(why) => {
                if cfg.strict {
                    return Err(MeshError::Failed(why));
                }
                degraded.push(why);
            }
        }
    }
    let used_physics = !phys_norms.is_empty();
    let physics_scale = phys_norms.iter().map(|p| p.1.scale).fold(0.0f64, f64::max);
    let mut physics_norm = vec![0.0; n];
    let mut physics_raw = vec![0.0; n];
    if used_physics {
        for e in 0..n {
            let wanted = input.zones.map(|z| z.zones[e].physics());
            let pick = phys_norms.iter().find(|p| Some(p.0) == wanted).or_else(|| phys_norms.iter().find(|p| p.0 == PhysicsEstimator::Equilibrated)).unwrap_or(&phys_norms[0]);
            let beta = input.zones.map_or(1.0, |z| z.beta[e]);
            physics_norm[e] = beta * pick.1.values[e];
            physics_raw[e] = beta * pick.2[e];
        }
        if let Some(z) = input.zones {
            let unmet = z.unmet();
            if !unmet.is_empty() {
                notes.push(format!("zones call for estimators that do not exist ({unmet:?}); their elements use the available physics estimator"));
            }
        }
    }
    if !used_adjoint && !used_physics {
        return Err(MeshError::Failed(format!("no usable estimator: {}", degraded.join("; "))));
    }

    // fuse
    let alpha = if !used_adjoint { 0.0 } else if !used_physics { 1.0 } else { cfg.alpha };
    let mut score: Vec<f64> = (0..n)
        .map(|e| match cfg.strategy {
            FusionStrategy::WeightedSum => alpha * adjoint_norm[e] + (1.0 - alpha) * physics_norm[e],
            FusionStrategy::WeightedMax => (alpha * adjoint_norm[e]).max((1.0 - alpha) * physics_norm[e]),
            FusionStrategy::ConstraintPreserving => {
                if used_adjoint {
                    adjoint_norm[e]
                } else {
                    physics_norm[e]
                }
            }
        })
        .collect();
    let mut mandatory = vec![false; n];
    if cfg.strategy == FusionStrategy::ConstraintPreserving {
        match (cfg.physics_allowance, used_physics) {
            (Some(allow), true) => {
                for e in 0..n {
                    mandatory[e] = physics_raw[e] > allow;
                }
            }
            (Some(_), false) => degraded.push("a physics requirement was given but no physics estimator is usable: it cannot be enforced".into()),
            (None, _) => notes.push("no physics allowance given: the constraint-preserving fusion ranks by the goal only".into()),
        }
    }
    if let Some(prot) = input.protected {
        if prot.len() != n {
            return Err(MeshError::Failed(format!("{} protection flags for {} elements", prot.len(), n)));
        }
        for e in 0..n {
            mandatory[e] |= prot[e];
        }
    }
    if let Some(cap) = input.capped {
        if cap.len() != n {
            return Err(MeshError::Failed(format!("{} cap flags for {} elements", cap.len(), n)));
        }
        for e in 0..n {
            if cap[e] {
                score[e] = 0.0;
                mandatory[e] = false;
            }
        }
    }
    // mandatory elements above everything else, ordered among themselves by their physics indicator
    if mandatory.iter().any(|&m| m) {
        let top = score.iter().cloned().fold(0.0f64, f64::max);
        let pmax = (0..n).filter(|&e| mandatory[e]).map(|e| physics_norm[e]).fold(0.0f64, f64::max).max(1e-300);
        for e in 0..n {
            if mandatory[e] {
                score[e] = top + 1.0 + physics_norm[e] / pmax;
            }
        }
    }
    let reliability = if degraded.is_empty() { Reliability::Reliable } else { Reliability::Degraded(degraded.join("; ")) };
    Ok(Fused { score, mandatory, adjoint_norm, physics_norm, adjoint_scale, physics_scale, used_adjoint, used_physics, reliability, notes })
}

impl Zone {
    /// Whether the zone is capped by the singular-region policy when its elements reach the size limit.
    pub fn is_singular(self) -> bool {
        self == Zone::Singular
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptivity::estimator::Indicator;

    fn field(name: &str, v: &[f64]) -> ErrorField {
        ErrorField::new(name, Indicator::Marking, v.to_vec(), v.iter().map(|x| x * x).sum::<f64>().sqrt(), Reliability::Reliable, 0.0)
    }

    #[test]
    fn normalisations_handle_zero_outliers_and_invalid_values() {
        let v = [1.0, 2.0, 3.0, 4.0, 100.0];
        assert_eq!(normalise(&v, Normalization::Max).unwrap().scale, 100.0);
        let rms = normalise(&v, Normalization::Rms).unwrap();
        assert!((rms.scale - (10030.0f64 / 5.0).sqrt()).abs() < 1e-12);
        // the 80th percentile ignores the single peak
        let pct = normalise(&v, Normalization::Percentile(80.0)).unwrap();
        assert_eq!(pct.scale, 4.0);
        assert_eq!(pct.values[4], 25.0);
        assert_eq!(normalise(&[0.0, 0.0], Normalization::Max).unwrap(), Normalised { scale: 0.0, values: vec![0.0, 0.0] });
        assert!(normalise(&[1.0, f64::NAN], Normalization::Max).is_err());
        assert!(normalise(&[1.0, -1.0], Normalization::Max).is_err());
        assert!(normalise(&[1.0], Normalization::Percentile(0.0)).is_err());
        assert_eq!(normalise(&[], Normalization::Max).unwrap().values.len(), 0);
    }

    #[test]
    fn weighted_sum_and_max_follow_their_formulas() {
        let adj = field("dwr", &[4.0, 2.0, 0.0, 1.0]);
        let phy = field("zz", &[0.0, 5.0, 10.0, 5.0]);
        let input = FusionInput { adjoint: &[(1.0, &adj)], physics: &[(PhysicsEstimator::SprPlus, &phy)], zones: None, capped: None, protected: None };
        let sum = fuse(&input, &FusionConfig { strategy: FusionStrategy::WeightedSum, alpha: 0.7, ..Default::default() }).unwrap();
        let (a, p) = ([1.0, 0.5, 0.0, 0.25], [0.0, 0.5, 1.0, 0.5]);
        for e in 0..4 {
            assert!((sum.score[e] - (0.7 * a[e] + 0.3 * p[e])).abs() < 1e-12);
        }
        let mx = fuse(&input, &FusionConfig { strategy: FusionStrategy::WeightedMax, alpha: 0.7, ..Default::default() }).unwrap();
        for e in 0..4 {
            assert!((mx.score[e] - (0.7 * a[e]).max(0.3 * p[e])).abs() < 1e-12);
        }
        // the dimensioned inputs are untouched
        assert_eq!(adj.values, vec![4.0, 2.0, 0.0, 1.0]);
        assert!(sum.reliability == Reliability::Reliable && sum.used_adjoint && sum.used_physics);
    }

    #[test]
    fn constraint_preserving_fusion_refines_unresolved_physics_even_where_the_goal_does_not_care() {
        let adj = field("dwr", &[10.0, 5.0, 0.0, 0.0]);
        let phy = field("eq", &[1.0, 1.0, 9.0, 3.0]);
        let input = FusionInput { adjoint: &[(1.0, &adj)], physics: &[(PhysicsEstimator::Equilibrated, &phy)], zones: None, capped: None, protected: None };
        // alpha = 1: a pure goal ranking would put element 2 (adjoint 0) last; the physics allowance of 5 makes it mandatory
        let cfg = FusionConfig { alpha: 1.0, physics_allowance: Some(5.0), ..Default::default() };
        let f = fuse(&input, &cfg).unwrap();
        assert_eq!(f.mandatory, vec![false, false, true, false]);
        let order: Vec<usize> = {
            let mut o: Vec<usize> = (0..4).collect();
            o.sort_by(|&a, &b| f.score[b].total_cmp(&f.score[a]));
            o
        };
        assert_eq!(order[0], 2, "{:?}", f.score);
        assert_eq!(order[1], 0);
        // without the requirement the goal alone ranks and nothing is mandatory
        let g = fuse(&input, &FusionConfig { alpha: 1.0, physics_allowance: None, ..Default::default() }).unwrap();
        assert!(g.mandatory.iter().all(|&m| !m));
        assert!(g.notes.iter().any(|n| n.contains("no physics allowance")));
        assert_eq!(g.score[2], 0.0);
    }

    #[test]
    fn a_failed_estimator_is_never_silently_dropped() {
        let adj = field("dwr", &[4.0, 2.0, 1.0]);
        let bad = ErrorField::new("eq", Indicator::Marking, vec![1.0, f64::NAN, 1.0], 0.0, Reliability::Reliable, 0.0);
        assert!(matches!(bad.reliability, Reliability::Failed(_)));
        let input = FusionInput { adjoint: &[(1.0, &adj)], physics: &[(PhysicsEstimator::Equilibrated, &bad)], zones: None, capped: None, protected: None };
        let f = fuse(&input, &FusionConfig::default()).unwrap();
        assert!(!f.used_physics && f.used_adjoint);
        assert!(matches!(&f.reliability, Reliability::Degraded(w) if w.contains("eq failed")), "{:?}", f.reliability);
        assert!(fuse(&input, &FusionConfig { strict: true, ..Default::default() }).is_err());
        // an unmet physics requirement is reported, not ignored
        let f = fuse(&input, &FusionConfig { physics_allowance: Some(1.0), ..Default::default() }).unwrap();
        assert!(matches!(&f.reliability, Reliability::Degraded(w) if w.contains("cannot be enforced")));
        // nothing usable at all
        let unsupported = ErrorField::unsupported("dwr", "plastic", 3);
        let none = FusionInput { adjoint: &[(1.0, &unsupported)], physics: &[(PhysicsEstimator::Equilibrated, &bad)], zones: None, capped: None, protected: None };
        assert!(fuse(&none, &FusionConfig::default()).is_err());
        // a field of the wrong length is refused
        let short = field("zz", &[1.0, 2.0]);
        let mismatched = FusionInput { adjoint: &[(1.0, &adj)], physics: &[(PhysicsEstimator::SprPlus, &short)], zones: None, capped: None, protected: None };
        assert!(matches!(fuse(&mismatched, &FusionConfig { strict: true, ..Default::default() }), Err(_)));
    }

    #[test]
    fn capped_elements_leave_the_ranking_so_a_singularity_cannot_demand_endless_refinement() {
        let adj = field("dwr", &[1.0, 9.0, 3.0]);
        let phy = field("eq", &[1.0, 100.0, 1.0]);
        let capped = [false, true, false];
        let input = FusionInput { adjoint: &[(1.0, &adj)], physics: &[(PhysicsEstimator::Equilibrated, &phy)], zones: None, capped: Some(&capped), protected: None };
        let f = fuse(&input, &FusionConfig { physics_allowance: Some(2.0), ..Default::default() }).unwrap();
        assert_eq!(f.score[1], 0.0);
        assert!(!f.mandatory[1]);
        assert!(f.score[2] > f.score[0]);
        assert!(fuse(&FusionInput { capped: Some(&[true]), ..input }, &FusionConfig::default()).is_err());
    }

    #[test]
    fn several_goals_combine_by_weighted_sum_or_maximum_and_the_zone_weight_scales_the_physics() {
        let g1 = field("g1", &[1.0, 0.0, 0.0]);
        let g2 = field("g2", &[0.0, 4.0, 0.0]);
        let phy = field("eq", &[0.0, 0.0, 1.0]);
        let adj = [(1.0, &g1), (1.0, &g2)];
        let sum = fuse(&FusionInput { adjoint: &adj, physics: &[], zones: None, capped: None, protected: None }, &FusionConfig { strategy: FusionStrategy::WeightedSum, goal_combine: GoalCombine::Sum, ..Default::default() }).unwrap();
        // each goal normalised by its own maximum, then averaged
        assert_eq!(sum.adjoint_norm, vec![0.5, 0.5, 0.0]);
        let mx = fuse(&FusionInput { adjoint: &adj, physics: &[], zones: None, capped: None, protected: None }, &FusionConfig { strategy: FusionStrategy::WeightedSum, goal_combine: GoalCombine::Max, ..Default::default() }).unwrap();
        assert_eq!(mx.adjoint_norm, vec![1.0, 1.0, 0.0]);
        // zone weights: a contact zone doubles the physics weight at element 2
        let zones = ZoneMap { zones: vec![Zone::Smooth, Zone::Smooth, Zone::Contact], beta: vec![1.0, 1.0, 2.0], singular_nodes: vec![] };
        let f = fuse(&FusionInput { adjoint: &[], physics: &[(PhysicsEstimator::Equilibrated, &phy)], zones: Some(&zones), capped: None, protected: None }, &FusionConfig { strategy: FusionStrategy::WeightedSum, ..Default::default() }).unwrap();
        assert_eq!(f.physics_norm[2], 2.0);
        assert!(!f.used_adjoint);
        assert!(Zone::Singular.is_singular());
    }

    #[test]
    fn protected_elements_are_mandatory_under_every_strategy_unless_capped() {
        let adj = field("dwr", &[5.0, 1.0, 0.0]);
        let prot = [false, false, true];
        for strategy in [FusionStrategy::WeightedSum, FusionStrategy::WeightedMax, FusionStrategy::ConstraintPreserving] {
            let input = FusionInput { adjoint: &[(1.0, &adj)], physics: &[], zones: None, capped: None, protected: Some(&prot) };
            let f = fuse(&input, &FusionConfig { strategy, ..Default::default() }).unwrap();
            assert!(f.mandatory[2] && f.score[2] > f.score[0], "{strategy:?}: {:?}", f.score);
        }
        let capped = [false, false, true];
        let f = fuse(&FusionInput { adjoint: &[(1.0, &adj)], physics: &[], zones: None, capped: Some(&capped), protected: Some(&prot) }, &FusionConfig::default()).unwrap();
        assert!(!f.mandatory[2] && f.score[2] == 0.0);
    }
}
