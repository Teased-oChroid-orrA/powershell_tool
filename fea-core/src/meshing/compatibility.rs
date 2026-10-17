//! Deterministic registry for the audited existing meshing slice. Rejections precede
//! ranking; a family in ElementKind is not proof of arbitrary mesher compatibility.
use crate::ElementKind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    Structured2d,
    Unstructured2d,
    SweptHybrid,
    LatticeTet,
    NativeConstrained,
    GeneralHybrid3d,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryClass {
    PlanarRegion,
    SweepableSection,
    GeneralSolid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formulation {
    LinearElastic2d,
    LinearElastic3d,
    Thermal,
    Plastic,
    Contact,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Compatible,
    StrategyUnavailable,
    FormulationNotAudited,
    ElementMismatch,
    GeometryMismatch,
    ResourceEstimateMissing,
    ResourceBudget,
    CurvedCadNotCertified,
    /// Native recovery needs the smallest facet/segment angle of the input shell;
    /// without it the request cannot be certified (no guessing).
    ShellAngleUnknown,
    /// The input shell has angles below the audited bound, where conforming
    /// Delaunay recovery was measured not to terminate.
    ShellAngleTooSmall,
}
#[derive(Debug, Clone, Copy)]
pub struct Request {
    pub strategy: Strategy,
    pub geometry: GeometryClass,
    pub element: ElementKind,
    pub formulation: Formulation,
    /// Upper bounds, not predicted typical costs. None cannot satisfy a hard cap.
    pub element_upper_bound: Option<usize>,
    pub dof_upper_bound: Option<usize>,
    pub require_exact_curved_cad: bool,
    /// Smallest angle (degrees) between any two segments or facets of the closed
    /// triangulated input shell. Only the native constrained strategy consumes it.
    pub shell_min_angle_deg: Option<f64>,
}
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_elements: usize,
    pub max_dofs: usize,
}
/// Registry version changes whenever supported combinations/semantics change.
/// 2: `NativeConstrained` became available for closed triangulated shells (Tet4,
/// linear elastic 3D) whose smallest angle is at least [`NATIVE_MIN_SHELL_ANGLE_DEG`].
pub const REGISTRY_VERSION: u32 = 2;
/// Smallest shell angle with measured evidence (`docs/issue-46-phase-58.md`, `-59`):
/// 18-degree wedge/diagonal inputs recover and refine; 7-degree diagonals and the
/// sampled NEMA17 shell do not terminate. An evidence bound, not a theorem.
pub const NATIVE_MIN_SHELL_ANGLE_DEG: f64 = 18.0;
pub fn check(r: &Request, b: Budget) -> Reason {
    use ElementKind::*;
    if r.strategy == Strategy::GeneralHybrid3d {
        return Reason::StrategyUnavailable;
    }
    if !matches!(
        r.formulation,
        Formulation::LinearElastic2d | Formulation::LinearElastic3d
    ) {
        return Reason::FormulationNotAudited;
    }
    let geometry_ok = match r.strategy {
        Strategy::Structured2d | Strategy::Unstructured2d => {
            r.geometry == GeometryClass::PlanarRegion
        }
        Strategy::SweptHybrid => r.geometry == GeometryClass::SweepableSection,
        Strategy::LatticeTet | Strategy::NativeConstrained => {
            r.geometry == GeometryClass::GeneralSolid
        }
        _ => false,
    };
    if !geometry_ok {
        return Reason::GeometryMismatch;
    }
    let two = r.formulation == Formulation::LinearElastic2d;
    let compatible = match r.strategy {
        Strategy::Structured2d => two && matches!(r.element, Quad4 | Quad8 | Quad9),
        Strategy::Unstructured2d => two && matches!(r.element, Tri3 | Tri6 | Quad4 | Quad8 | Quad9),
        Strategy::SweptHybrid => {
            !two && matches!(r.element, Hex8 | Hex20 | Hex27 | Wedge6 | Wedge15)
        }
        Strategy::LatticeTet => !two && matches!(r.element, Tet4 | Tet10),
        // The conforming recovery produces linear tetrahedra only.
        Strategy::NativeConstrained => !two && r.element == Tet4,
        _ => false,
    };
    if !compatible {
        return Reason::ElementMismatch;
    }
    // Curved nodes exist, but general exact trimmed-CAD mappings are not certified.
    if r.require_exact_curved_cad {
        return Reason::CurvedCadNotCertified;
    }
    if r.strategy == Strategy::NativeConstrained {
        match r.shell_min_angle_deg {
            None => return Reason::ShellAngleUnknown,
            Some(a) if a.is_nan() || a < NATIVE_MIN_SHELL_ANGLE_DEG => {
                return Reason::ShellAngleTooSmall
            }
            Some(_) => {}
        }
    }
    match (r.element_upper_bound, r.dof_upper_bound) {
        (Some(e), Some(d)) if e > 0 && d > 0 && e <= b.max_elements && d <= b.max_dofs => {
            Reason::Compatible
        }
        (None, _) | (_, None) => Reason::ResourceEstimateMissing,
        _ => Reason::ResourceBudget,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recommendation {
    pub index: usize,
    pub reason: Reason,
}
/// Conservative deterministic policy: preserve audited input order. No unsupported
/// speed heuristic or hidden candidate solves; explicit invalid choices are errors.
pub fn select(
    candidates: &[Request],
    budget: Budget,
    fixed: Option<usize>,
) -> Result<Recommendation, Reason> {
    if let Some(index) = fixed {
        let r = candidates.get(index).ok_or(Reason::StrategyUnavailable)?;
        return match check(r, budget) {
            Reason::Compatible => Ok(Recommendation {
                index,
                reason: Reason::Compatible,
            }),
            e => Err(e),
        };
    }
    candidates
        .iter()
        .position(|r| check(r, budget) == Reason::Compatible)
        .map(|index| Recommendation {
            index,
            reason: Reason::Compatible,
        })
        .ok_or(Reason::StrategyUnavailable)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn r() -> Request {
        Request {
            strategy: Strategy::LatticeTet,
            geometry: GeometryClass::GeneralSolid,
            element: ElementKind::Tet10,
            formulation: Formulation::LinearElastic3d,
            element_upper_bound: Some(100),
            dof_upper_bound: Some(1000),
            require_exact_curved_cad: false,
            shell_min_angle_deg: None,
        }
    }
    const B: Budget = Budget {
        max_elements: 100,
        max_dofs: 1000,
    };
    #[test]
    fn registry_rejects_unavailable_and_incompatible_combinations() {
        let mut q = r();
        assert_eq!(check(&q, B), Reason::Compatible);
        q.strategy = Strategy::GeneralHybrid3d;
        assert_eq!(check(&q, B), Reason::StrategyUnavailable);
        q = r();
        q.element = ElementKind::Hex20;
        assert_eq!(check(&q, B), Reason::ElementMismatch);
        q = r();
        q.formulation = Formulation::Contact;
        assert_eq!(check(&q, B), Reason::FormulationNotAudited);
        q = r();
        q.require_exact_curved_cad = true;
        assert_eq!(check(&q, B), Reason::CurvedCadNotCertified);
    }
    #[test]
    fn native_constrained_requires_tet4_and_a_measured_shell_angle() {
        let native = |element, angle| {
            let mut q = r();
            q.strategy = Strategy::NativeConstrained;
            q.element = element;
            q.shell_min_angle_deg = angle;
            q
        };
        assert_eq!(
            check(&native(ElementKind::Tet4, Some(45.)), B),
            Reason::Compatible
        );
        assert_eq!(
            check(
                &native(ElementKind::Tet4, Some(NATIVE_MIN_SHELL_ANGLE_DEG)),
                B
            ),
            Reason::Compatible
        );
        assert_eq!(
            check(&native(ElementKind::Tet4, Some(7.)), B),
            Reason::ShellAngleTooSmall
        );
        assert_eq!(
            check(&native(ElementKind::Tet4, Some(f64::NAN)), B),
            Reason::ShellAngleTooSmall
        );
        assert_eq!(
            check(&native(ElementKind::Tet4, None), B),
            Reason::ShellAngleUnknown
        );
        assert_eq!(
            check(&native(ElementKind::Tet10, Some(45.)), B),
            Reason::ElementMismatch
        );
        // A fixed native choice with a bad shell is an error, never a silent switch to the lattice.
        let candidates = [native(ElementKind::Tet4, Some(7.)), r()];
        assert_eq!(select(&candidates, B, None).unwrap().index, 1);
        assert_eq!(
            select(&candidates, B, Some(0)),
            Err(Reason::ShellAngleTooSmall)
        );
        let mut exact = native(ElementKind::Tet4, Some(45.));
        exact.require_exact_curved_cad = true;
        assert_eq!(check(&exact, B), Reason::CurvedCadNotCertified);
        assert_eq!(REGISTRY_VERSION, 2);
    }
    #[test]
    fn budgets_and_invalid_fixed_choices_never_fall_back_silently() {
        let mut bad = r();
        bad.element_upper_bound = None;
        assert_eq!(check(&bad, B), Reason::ResourceEstimateMissing);
        bad.element_upper_bound = Some(101);
        assert_eq!(check(&bad, B), Reason::ResourceBudget);
        let candidates = [bad, r()];
        assert_eq!(select(&candidates, B, None).unwrap().index, 1);
        assert_eq!(select(&candidates, B, Some(0)), Err(Reason::ResourceBudget));
        assert_eq!(
            select(&candidates, B, Some(99)),
            Err(Reason::StrategyUnavailable)
        );
        assert_eq!(select(&[], B, None), Err(Reason::StrategyUnavailable));
    }
}
