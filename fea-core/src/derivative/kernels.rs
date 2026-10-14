//! The derivative capabilities of the kernels this crate actually has, for the policy engine ([`DerivativePolicyEngine`]).
//!
//! Naming: `<module>.<kernel>`. The element and constitutive kernels are written against [`Scalar`](super::Scalar), so forward and
//! reverse AD, hyper-dual and complex-step verification and the difference reference all apply; the J2 return map also has the
//! closed-form consistent tangent (cheap, first order). Contact kernels are nonsmooth: the penalty problem is smooth on a fixed
//! active set, the unilateral problem has only generalized (directional) derivatives across a change of the active set.

use super::policy::*;

pub const ELEMENT_STIFFNESS: &str = "element.stiffness";
pub const ELEMENT_MASS: &str = "element.mass";
pub const ELEMENT_GEOMETRY: &str = "element.geometry";
pub const ELEMENT_THERMAL_LOAD: &str = "element.thermal_load";
pub const ELEMENT_GEOMETRIC_STIFFNESS: &str = "element.geometric_stiffness";
pub const CONSTITUTIVE_J2: &str = "constitutive.j2";
pub const CONSTITUTIVE_HYPERELASTIC: &str = "constitutive.hyperelastic";
pub const CONTACT_PENALTY: &str = "contact.penalty";
pub const CONTACT_UNILATERAL: &str = "contact.unilateral";

impl DerivativePolicyEngine {
    /// A policy engine with every kernel of `fea-core` registered. Specific (analytic) capabilities are registered first so that they
    /// win cost ties.
    pub fn with_fea_kernels() -> Self {
        let mut e = Self::default();
        // The consistent tangent of the J2 return map: the analytic derivative costs a fraction of one more return map.
        e.register_scalar_kernel(CONSTITUTIVE_J2, Some(0.6));
        e.register_scalar_kernel(CONSTITUTIVE_HYPERELASTIC, None);
        e.register_scalar_kernel(ELEMENT_STIFFNESS, None);
        e.register_scalar_kernel(ELEMENT_MASS, None);
        e.register_scalar_kernel(ELEMENT_GEOMETRY, None);
        e.register_scalar_kernel(ELEMENT_THERMAL_LOAD, None);
        e.register_scalar_kernel(ELEMENT_GEOMETRIC_STIFFNESS, None);
        // penalty contact: smooth on a fixed active set; across a transition only the generalized derivative is honest
        e.register_scalar_kernel(CONTACT_PENALTY, None);
        e.register_generalized(CONTACT_PENALTY);
        e.register_generalized(CONTACT_UNILATERAL);
        e
    }
}
