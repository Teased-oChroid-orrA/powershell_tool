//! Input validation - spec section 69: reject non-physical geometry/
//! material inputs before they reach the solver pipeline, rather than
//! producing a plausible-looking but meaningless result.

use crate::compliance::{FastenerSegment, MemberStack};
use crate::thread::{ThreadGeometry, ThreadGeometryError};

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    Thread(ThreadGeometryError),
    NonPositiveModulus,
    InvalidPoissonRatio,
    NegativeFriction,
    ZeroMemberThickness,
    NonPositiveMemberModulus,
    NonFiniteValue(&'static str),
    ZeroContactArea,
    NegativeFastenerSegmentLength,
    EmptyMemberStack,
}

pub fn validate_thread(thread: &ThreadGeometry) -> Result<(), ValidationError> {
    thread.validate().map_err(ValidationError::Thread)
}

pub fn validate_material(e: f64, nu: f64) -> Result<(), ValidationError> {
    if !e.is_finite() || e <= 0.0 {
        return Err(ValidationError::NonPositiveModulus);
    }
    if !nu.is_finite() || !(0.0..0.5).contains(&nu) {
        return Err(ValidationError::InvalidPoissonRatio);
    }
    Ok(())
}

pub fn validate_friction(mu: f64) -> Result<(), ValidationError> {
    if !mu.is_finite() || mu < 0.0 {
        return Err(ValidationError::NegativeFriction);
    }
    Ok(())
}

pub fn validate_fastener_segments(segments: &[FastenerSegment]) -> Result<(), ValidationError> {
    for s in segments {
        if !s.length.is_finite() || s.length < 0.0 {
            return Err(ValidationError::NegativeFastenerSegmentLength);
        }
        if !s.diameter.is_finite() || s.diameter <= 0.0 {
            return Err(ValidationError::NonFiniteValue("fastener segment diameter"));
        }
    }
    Ok(())
}

pub fn validate_member_stack(stack: &MemberStack) -> Result<(), ValidationError> {
    if stack.members.is_empty() {
        return Err(ValidationError::EmptyMemberStack);
    }
    for m in &stack.members {
        if !m.thickness.is_finite() || m.thickness <= 0.0 {
            return Err(ValidationError::ZeroMemberThickness);
        }
        if !m.e.is_finite() || m.e <= 0.0 {
            return Err(ValidationError::NonPositiveMemberModulus);
        }
        if !m.hole_diameter.is_finite() || m.hole_diameter < 0.0 {
            return Err(ValidationError::NonFiniteValue("member hole diameter"));
        }
    }
    Ok(())
}

pub fn validate_contact_annulus(inner_radius: f64, outer_radius: f64) -> Result<(), ValidationError> {
    if !inner_radius.is_finite() || !outer_radius.is_finite() || outer_radius <= inner_radius || inner_radius < 0.0 {
        return Err(ValidationError::ZeroContactArea);
    }
    Ok(())
}

/// Non-fatal advisory warnings (spec section 70) - the analysis still
/// proceeds, but the result should visibly flag these to the user.
pub fn warnings_for(preload: f64, proof_load: Option<f64>, von_mises: f64, yield_strength: Option<f64>, separation_margin: f64, slip_margin: f64) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(proof) = proof_load {
        if preload > proof {
            warnings.push("Preload exceeds proof load".to_string());
        }
    }
    if let Some(sy) = yield_strength {
        if von_mises > sy {
            warnings.push("Combined stress exceeds yield".to_string());
        }
    }
    if separation_margin.is_finite() && separation_margin < 0.15 {
        warnings.push("Joint near separation".to_string());
    }
    if slip_margin.is_finite() && slip_margin < 0.15 {
        warnings.push("Low slip margin".to_string());
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_material_rejects_negative_modulus() {
        assert_eq!(validate_material(-1.0, 0.3), Err(ValidationError::NonPositiveModulus));
    }

    #[test]
    fn validate_material_rejects_out_of_range_poisson_ratio() {
        assert_eq!(validate_material(200_000.0, 0.9), Err(ValidationError::InvalidPoissonRatio));
    }

    #[test]
    fn validate_friction_rejects_negative_values() {
        assert_eq!(validate_friction(-0.1), Err(ValidationError::NegativeFriction));
    }

    #[test]
    fn validate_contact_annulus_rejects_zero_or_inverted_area() {
        assert_eq!(validate_contact_annulus(5.0, 5.0), Err(ValidationError::ZeroContactArea));
        assert_eq!(validate_contact_annulus(6.0, 5.0), Err(ValidationError::ZeroContactArea));
    }

    #[test]
    fn warnings_for_flags_preload_exceeding_proof_load() {
        let warnings = warnings_for(10000.0, Some(9000.0), 0.0, None, f64::INFINITY, f64::INFINITY);
        assert!(warnings.iter().any(|w| w.contains("proof load")));
    }

    #[test]
    fn warnings_for_is_empty_when_everything_is_healthy() {
        let warnings = warnings_for(5000.0, Some(9000.0), 30000.0, Some(50000.0), 1.0, 1.0);
        assert!(warnings.is_empty());
    }
}
