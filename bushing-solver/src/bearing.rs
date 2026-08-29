//! Ported from engineering.toolbox's `src/lib/core/shared/bearing.ts`
//! (`calculateUniversalBearing`) - converts a bearing surface profile
//! (a sequence of cylindrical/frustum segments) into an effective
//! sequencing thickness for the edge-distance-strength check.
//!
//! Only `t_eff_sequence` is exposed: it's the sole field `solve.rs`
//! consumes today (edge-distance strength). The TS source's `t_eff`/
//! `is_knife_edge`/per-segment results have no caller in this scoped
//! port, so they're left unported rather than carried as unused surface
//! area.

/// One segment of the bearing profile along the housing axis - a
/// cylinder when `d_top == d_bottom`, a frustum (conical transition)
/// otherwise. `eta` overrides the auto efficiency (1.0 for cylinders,
/// 0.35 for frustums - `bearing.ts:46`) when set. `is_parent` mirrors the
/// TS source's `role` field: only `role !== 'doubler'` segments
/// ("parent") count toward `t_eff_sequence` - this crate never builds a
/// doubler segment, so it's a plain bool rather than a string enum.
#[derive(Debug, Clone, Copy)]
pub struct BearingSegment {
    pub d_top: f64,
    pub d_bottom: f64,
    pub height: f64,
    pub eta: Option<f64>,
    pub is_parent: bool,
}

pub struct BearingProfileResult {
    pub t_eff_sequence: f64,
}

/// Ported from `bearing.ts:30-82`.
pub fn calculate_universal_bearing(profile: &[BearingSegment]) -> BearingProfileResult {
    if profile.is_empty() {
        return BearingProfileResult { t_eff_sequence: 0.0 };
    }
    let t_eff_sequence = profile
        .iter()
        .filter(|s| s.is_parent)
        .map(|s| {
            let is_cyl = (s.d_top - s.d_bottom).abs() < 1e-6;
            let auto_eta = if is_cyl { 1.0 } else { 0.35 };
            let eta = s.eta.unwrap_or(auto_eta);
            s.height * eta
        })
        .sum();
    BearingProfileResult { t_eff_sequence }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_cylindrical_parent_segment_reduces_to_its_own_height() {
        let profile = [BearingSegment { d_top: 0.5, d_bottom: 0.5, height: 0.5, eta: None, is_parent: true }];
        let result = calculate_universal_bearing(&profile);
        assert!((result.t_eff_sequence - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_frustum_segment_gets_the_auto_035_efficiency() {
        let profile = [BearingSegment { d_top: 0.5, d_bottom: 0.625, height: 0.1, eta: None, is_parent: true }];
        let result = calculate_universal_bearing(&profile);
        assert!((result.t_eff_sequence - 0.1 * 0.35).abs() < 1e-9);
    }

    #[test]
    fn non_parent_segments_are_excluded_from_t_eff_sequence() {
        let profile = [
            BearingSegment { d_top: 0.5, d_bottom: 0.5, height: 0.5, eta: None, is_parent: true },
            BearingSegment { d_top: 0.5, d_bottom: 0.5, height: 100.0, eta: None, is_parent: false },
        ];
        let result = calculate_universal_bearing(&profile);
        assert!((result.t_eff_sequence - 0.5).abs() < 1e-9);
    }

    #[test]
    fn empty_profile_yields_zero() {
        let result = calculate_universal_bearing(&[]);
        assert_eq!(result.t_eff_sequence, 0.0);
    }
}
