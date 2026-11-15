//! Bulk (Dorfler) marking with mandatory elements and resource budgets.
//!
//! **Convention.** For non-negative indicators `q_e` the marked set `M` is the smallest set, taken in order of decreasing `q_e`
//! (ties by element index, so the choice is deterministic), with `sum_{e in M} q_e >= theta * sum_e q_e`; mandatory elements are
//! marked first and count towards the sum. With `squared` the indicators are first squared (`q_e = score_e^2`), the usual
//! convention for energy-norm estimators whose squares add up to the global error; without it the scores are summed as given
//! (they are ranking scores, not error contributions).

use crate::meshing::MeshError;

#[derive(Debug, Clone, PartialEq)]
pub struct Marked {
    pub marked: Vec<bool>,
    pub n_marked: usize,
    /// `sum_M q / sum q` reached (1 when nothing is left to refine).
    pub covered: f64,
}

/// Dorfler marking (see the module docs). `theta` in `(0, 1]`.
pub fn dorfler(score: &[f64], theta: f64, mandatory: &[bool], squared: bool) -> Result<Marked, MeshError> {
    if !(theta > 0.0 && theta <= 1.0) {
        return Err(MeshError::Failed(format!("the Dorfler fraction {theta} is outside (0, 1]")));
    }
    if mandatory.len() != score.len() {
        return Err(MeshError::Failed(format!("{} mandatory flags for {} scores", mandatory.len(), score.len())));
    }
    if let Some(i) = score.iter().position(|s| !s.is_finite() || *s < 0.0) {
        return Err(MeshError::Failed(format!("score {i} is {} (scores must be finite and non-negative)", score[i])));
    }
    let q: Vec<f64> = score.iter().map(|s| if squared { s * s } else { *s }).collect();
    let total: f64 = q.iter().sum();
    let mut marked = vec![false; score.len()];
    let mut acc = 0.0;
    for e in 0..score.len() {
        if mandatory[e] {
            marked[e] = true;
            acc += q[e];
        }
    }
    let mut order: Vec<usize> = (0..score.len()).filter(|&e| !marked[e] && q[e] > 0.0).collect();
    order.sort_by(|&a, &b| q[b].total_cmp(&q[a]).then(a.cmp(&b)));
    for e in order {
        if acc >= theta * total {
            break;
        }
        marked[e] = true;
        acc += q[e];
    }
    let n_marked = marked.iter().filter(|&&m| m).count();
    Ok(Marked { marked, n_marked, covered: if total > 0.0 { (acc / total).min(1.0) } else { 1.0 } })
}

/// Resource limits of an adaptive run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Budget {
    pub max_dofs: Option<usize>,
    pub max_elements: Option<usize>,
    /// Bytes of the matrix and its factor (8 per stored entry).
    pub max_bytes: Option<usize>,
}

impl Budget {
    /// Whether a mesh of this size is within the limits.
    pub fn allows(&self, dofs: usize, elements: usize, bytes: usize) -> bool {
        self.max_dofs.is_none_or(|m| dofs <= m) && self.max_elements.is_none_or(|m| elements <= m) && self.max_bytes.is_none_or(|m| bytes <= m)
    }

    /// Most elements a mesh of `dofs` unknowns on `elements` elements may grow to under the dof and element limits (the bytes
    /// limit is checked on the real mesh: it depends on the fill-in).
    pub fn element_cap(&self, dofs: usize, elements: usize) -> usize {
        let by_dofs = self.max_dofs.map_or(usize::MAX, |m| ((m as f64) / dofs.max(1) as f64 * elements as f64).floor() as usize);
        by_dofs.min(self.max_elements.unwrap_or(usize::MAX))
    }
}

/// Reduce `marked` so that the predicted element count stays within `cap`: every marked element is replaced by
/// `children = (1 / ratio)^dim` elements. Mandatory elements stay; the rest are dropped from the lowest score up. Returns
/// `false` when the mandatory elements alone exceed the cap (the budget is exhausted).
pub fn trim_to_budget(marked: &mut [bool], score: &[f64], mandatory: &[bool], elements: usize, cap: usize, children: f64) -> bool {
    // The marked count is kept as elements are dropped (recounting the flags per drop was quadratic in the mesh size).
    let predicted = |count: usize| elements as f64 + count as f64 * (children - 1.0);
    let mut count = marked.iter().filter(|&&x| x).count();
    if predicted(count) <= cap as f64 {
        return true;
    }
    let mut droppable: Vec<usize> = (0..marked.len()).filter(|&e| marked[e] && !mandatory[e]).collect();
    droppable.sort_by(|&a, &b| score[a].total_cmp(&score[b]).then(b.cmp(&a)));
    for e in droppable {
        if predicted(count) <= cap as f64 {
            break;
        }
        marked[e] = false;
        count -= 1;
    }
    predicted(count) <= cap as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dorfler_marks_the_smallest_set_reaching_the_fraction() {
        let q = [5.0, 1.0, 3.0, 1.0];
        let m = dorfler(&q, 0.5, &[false; 4], false).unwrap();
        assert_eq!(m.marked, vec![true, false, false, false]); // 5 >= 0.5 * 10
        let m = dorfler(&q, 0.8, &[false; 4], false).unwrap();
        assert_eq!(m.marked, vec![true, false, true, false]); // 8 >= 8
        assert!((m.covered - 0.8).abs() < 1e-12);
        let m = dorfler(&q, 1.0, &[false; 4], false).unwrap();
        assert_eq!(m.n_marked, 4);
        // squared indicators: 25 of 36 reaches 0.6
        let m = dorfler(&q, 0.6, &[false; 4], true).unwrap();
        assert_eq!(m.marked, vec![true, false, false, false]);
        // the marked sum always reaches theta (property over a pseudo-random field)
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let r: Vec<f64> = (0..200).map(|_| { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s % 1000) as f64 / 100.0 }).collect();
        for theta in [0.1, 0.33, 0.5, 0.9] {
            let m = dorfler(&r, theta, &vec![false; r.len()], false).unwrap();
            let sum: f64 = r.iter().zip(&m.marked).filter(|p| *p.1).map(|p| p.0).sum();
            assert!(sum >= theta * r.iter().sum::<f64>() - 1e-12 && m.covered >= theta - 1e-12);
            // minimality: dropping the smallest marked element breaks the bound
            let min_marked = r.iter().zip(&m.marked).filter(|p| *p.1).map(|p| *p.0).fold(f64::INFINITY, f64::min);
            assert!(sum - min_marked < theta * r.iter().sum::<f64>() + 1e-12);
        }
    }

    #[test]
    fn mandatory_elements_are_marked_first_ties_are_deterministic_and_bad_input_is_refused() {
        let q = [0.0, 4.0, 4.0, 2.0];
        let m = dorfler(&q, 0.4, &[true, false, false, false], false).unwrap();
        assert!(m.marked[0]); // zero score but mandatory
        assert_eq!(m.marked, vec![true, true, false, false]); // the tie goes to the lower index
        assert!(dorfler(&q, 0.0, &[false; 4], false).is_err());
        assert!(dorfler(&q, 1.5, &[false; 4], false).is_err());
        assert!(dorfler(&[1.0, f64::NAN], 0.5, &[false; 2], false).is_err());
        assert!(dorfler(&[1.0, -1.0], 0.5, &[false; 2], false).is_err());
        assert!(dorfler(&q, 0.5, &[false; 3], false).is_err());
        let zero = dorfler(&[0.0, 0.0], 0.5, &[false; 2], false).unwrap();
        assert_eq!((zero.n_marked, zero.covered), (0, 1.0));
    }

    #[test]
    fn the_budget_trims_the_lowest_ranked_unmandated_elements() {
        let b = Budget { max_dofs: Some(2000), max_elements: None, max_bytes: Some(1_000_000) };
        assert!(b.allows(2000, 100, 1_000_000) && !b.allows(2001, 100, 0) && !b.allows(10, 10, 1_000_001));
        assert_eq!(b.element_cap(1000, 500), 1000);
        assert_eq!(Budget::default().element_cap(1000, 500), usize::MAX);
        // 100 elements, 4 children each: 10 marked would give 130; cap 118 allows 6 marked
        let mut marked = vec![false; 100];
        let mut score = vec![0.0; 100];
        for e in 0..10 {
            marked[e] = true;
            score[e] = e as f64 + 1.0;
        }
        let mut mandatory = vec![false; 100];
        mandatory[0] = true; // the lowest score, but protected
        assert!(trim_to_budget(&mut marked, &score, &mandatory, 100, 118, 4.0));
        assert_eq!(marked.iter().filter(|&&m| m).count(), 6);
        assert!(marked[0] && marked[9] && !marked[1] && !marked[2] && !marked[3] && !marked[4]);
        // mandatory alone over the cap: exhausted
        let mut m2 = vec![true; 5];
        assert!(!trim_to_budget(&mut m2, &[1.0; 5], &[true; 5], 5, 6, 4.0));
    }
}
