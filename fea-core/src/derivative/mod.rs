//! Unified adaptive derivative and sensitivity engine (epic `docs/epic-derivative-engine.md`).
//!
//! Three levels, independently configurable:
//! 1. **Local derivative construction** — kernels written once against [`Scalar`], differentiated by analytic formulas, forward
//!    ([`dual`]), reverse ([`tape`]), implicit ([`constitutive`]), complex-step ([`complex`]) or hyper-dual ([`hyperdual`]) arithmetic.
//! 2. **Global sensitivity propagation** — direct and adjoint solves of the converged system ([`global`]), shared with the
//!    adaptivity code.
//! 3. **Nonlinear / nonsmooth solution** — Newton, active sets and semismooth methods ([`contact`]); their derivative semantics are
//!    reported, never assumed.
//!
//! The [`DerivativePolicyEngine`] selects among the registered methods from a [`DerivativeContext`]; incompatible selections are
//! rejected with the reason.

pub mod complementarity;
pub mod complex;
pub mod constitutive;
pub mod contact;
pub mod dual;
pub mod element;
pub mod hyperdual;
pub mod linalg;
pub mod linear_sens;
pub mod matrix_free;
pub mod ops;
pub mod path;
pub mod policy;
pub mod scalar;
pub mod second_order;
pub mod tape;
pub mod transient;
pub mod verify;

pub use policy::*;
pub use scalar::Scalar;
