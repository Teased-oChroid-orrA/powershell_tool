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
/// 3: strategy generation ([`candidates`]), the full verdict table ([`explain`]) and the
/// evidence matrix ([`EVIDENCE`]) were added; the verdicts of [`check`] did not change.
pub const REGISTRY_VERSION: u32 = 3;
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
/// Every strategy the registry knows, in the audited preference order used by [`candidates`].
pub const STRATEGIES: [Strategy; 6] = [
    Strategy::Structured2d,
    Strategy::Unstructured2d,
    Strategy::SweptHybrid,
    Strategy::LatticeTet,
    Strategy::NativeConstrained,
    Strategy::GeneralHybrid3d,
];
/// Strategy generation: one request per known strategy for the same problem, in the audited order, so that [`select`] and
/// [`explain`] see every option. The problem is described by `template` (its `strategy` field is replaced); nothing here
/// judges compatibility, [`check`] does.
pub fn candidates(template: &Request) -> Vec<Request> {
    STRATEGIES
        .iter()
        .map(|&strategy| Request {
            strategy,
            ..*template
        })
        .collect()
}
/// The verdict of every candidate, in order: the full table behind a [`select`] answer, including why each rejected one
/// was rejected.
pub fn explain(candidates: &[Request], budget: Budget) -> Vec<(Strategy, Reason)> {
    candidates
        .iter()
        .map(|r| (r.strategy, check(r, budget)))
        .collect()
}
/// One audited claim about a strategy and the committed document holding its measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    pub strategy: Strategy,
    pub claim: &'static str,
    /// Path relative to the repository root.
    pub doc: &'static str,
    /// First registry version in which the claim holds.
    pub since: u32,
}
/// The evidence matrix: what each available strategy is certified for, and where the numbers are. A strategy without an
/// entry here must not be available (`tests::every_available_strategy_has_evidence_in_a_committed_document`).
pub const EVIDENCE: &[Evidence] = &[
    Evidence {
        strategy: Strategy::Structured2d,
        claim: "structured quadrilateral meshes of planar regions, linear elastic 2D",
        doc: "docs/fea-core.md",
        since: 1,
    },
    Evidence {
        strategy: Strategy::Unstructured2d,
        claim: "constrained Delaunay triangles and quads of planar regions, linear elastic 2D",
        doc: "docs/fea-core.md",
        since: 1,
    },
    Evidence {
        strategy: Strategy::SweptHybrid,
        claim: "hex/wedge sweeps of sweepable sections, linear elastic 3D",
        doc: "docs/fea-core.md",
        since: 1,
    },
    Evidence {
        strategy: Strategy::LatticeTet,
        claim: "signed-distance lattice tetrahedra (Tet4/Tet10), linear elastic 3D",
        doc: "docs/issue-46-phase-64.md",
        since: 1,
    },
    Evidence {
        strategy: Strategy::NativeConstrained,
        claim: "conforming Delaunay recovery of closed shells whose smallest angle is at least the audited bound",
        doc: "docs/issue-46-phase-61.md",
        since: 2,
    },
];
/// Whether any request could ever be compatible with `strategy` (the registry does not offer it otherwise).
pub fn is_available(strategy: Strategy) -> bool {
    strategy != Strategy::GeneralHybrid3d
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
        assert_eq!(REGISTRY_VERSION, 3);
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
    #[test]
    fn candidates_cover_every_strategy_and_explain_gives_the_whole_verdict_table() {
        let all = candidates(&r());
        assert_eq!(all.len(), STRATEGIES.len());
        assert!(all.iter().zip(STRATEGIES).all(|(c, s)| c.strategy == s));
        let table = explain(&all, B);
        let verdict = |s| table.iter().find(|(t, _)| *t == s).unwrap().1;
        // A Tet10 general solid: only the lattice mesher qualifies; the 2D and sweep strategies mismatch on geometry.
        assert_eq!(verdict(Strategy::LatticeTet), Reason::Compatible);
        assert_eq!(verdict(Strategy::Structured2d), Reason::GeometryMismatch);
        assert_eq!(verdict(Strategy::SweptHybrid), Reason::GeometryMismatch);
        assert_eq!(
            verdict(Strategy::NativeConstrained),
            Reason::ElementMismatch
        );
        assert_eq!(
            verdict(Strategy::GeneralHybrid3d),
            Reason::StrategyUnavailable
        );
        // `select` over generated candidates agrees with the table.
        assert_eq!(select(&all, B, None).unwrap().index, 3);
        // With Tet4 and a healthy shell the native strategy also qualifies, after the lattice in the audited order.
        let mut tet4 = r();
        tet4.element = ElementKind::Tet4;
        tet4.shell_min_angle_deg = Some(30.);
        let table = explain(&candidates(&tet4), B);
        let ok: Vec<Strategy> = table
            .iter()
            .filter(|(_, v)| *v == Reason::Compatible)
            .map(|(s, _)| *s)
            .collect();
        assert_eq!(ok, [Strategy::LatticeTet, Strategy::NativeConstrained]);
    }
    #[test]
    fn every_available_strategy_has_evidence_in_a_committed_document() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for s in STRATEGIES {
            let entries: Vec<&Evidence> = EVIDENCE.iter().filter(|e| e.strategy == s).collect();
            if is_available(s) {
                assert!(!entries.is_empty(), "{s:?} is available without evidence");
            } else {
                assert!(entries.is_empty(), "{s:?} is unavailable but has evidence");
            }
            for e in entries {
                assert!(
                    root.join(e.doc).is_file(),
                    "{:?}: evidence document {} is missing",
                    e.strategy,
                    e.doc
                );
                assert!(e.since >= 1 && e.since <= REGISTRY_VERSION);
                assert!(!e.claim.is_empty());
            }
        }
        // `is_available` agrees with `check`: no request for an unavailable strategy is ever compatible.
        for s in STRATEGIES {
            if !is_available(s) {
                let mut q = r();
                q.strategy = s;
                assert_eq!(check(&q, B), Reason::StrategyUnavailable);
            }
        }
    }
}
