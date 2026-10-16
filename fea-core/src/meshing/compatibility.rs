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
}
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_elements: usize,
    pub max_dofs: usize,
}
/// Registry version changes whenever supported combinations/semantics change.
pub const REGISTRY_VERSION: u32 = 1;
pub fn check(r: &Request, b: Budget) -> Reason {
    use ElementKind::*;
    if matches!(
        r.strategy,
        Strategy::NativeConstrained | Strategy::GeneralHybrid3d
    ) {
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
        Strategy::LatticeTet => r.geometry == GeometryClass::GeneralSolid,
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
        _ => false,
    };
    if !compatible {
        return Reason::ElementMismatch;
    }
    // Curved nodes exist, but general exact trimmed-CAD mappings are not certified.
    if r.require_exact_curved_cad {
        return Reason::CurvedCadNotCertified;
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
        q.strategy = Strategy::NativeConstrained;
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
