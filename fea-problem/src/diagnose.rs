//! Why a model will not solve, and what to do about it (ADR-020). `diagnose` classifies a failure from the **state of the
//! problem** (and the solver's error only to tell that a failure of a kind happened), never by parsing a message for
//! numbers, and returns for each finding its evidence, ranked remedies and the questions whose answer would settle it.
//! With no error it is the prevention pass: the same rules on a model that has not been solved yet.
//!
//! A remedy is data (`Edit`), not a closure: it can be shown, applied to a clone for a preview, applied for real, recorded
//! and ranked. Everything here is rule-based and deterministic; the rules decide what is offered and in which order, and a
//! learned ranker (app-tui, behind the measured gate) may only reorder. Every remedy says why it is offered.
use crate::advice::{self, Advice, BcKind, SurfaceKind};
use crate::problem::{Analysis, Geometry, Load, Problem, Support};
use serde::{Deserialize, Serialize};

/// What kind of failure. `Copy + Ord`: the situation label a ranker learns from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FailureClass {
    /// No support at all: the model is free to move.
    NoSupports,
    /// Supports exist but a rigid-body motion remains.
    UnderConstrained,
    /// A support or load names a surface the geometry does not have.
    UnknownSurface,
    /// More elements than the solver accepts at this mesh size.
    MeshTooFine,
    /// Elements so large that the thinnest section has fewer than two across it: the answer would be meaningless.
    MeshTooCoarse,
    /// A hole outside the outline, crossing it, or overlapping another hole.
    HoleOutsideOutline,
    /// A load or support value is NaN or infinite.
    NonFiniteInput,
    /// Elements of an imported mesh with a non-positive Jacobian (inverted, folded or collapsed).
    InvertedElements,
    /// A solved model whose displacements are a large share of the part's size: linear elasticity does not hold, and the usual
    /// cause is a unit slip between the modulus and the loads or lengths.
    ScaleSuspect,
    /// A hole so thin that the mesher cannot resolve it within its budget.
    ThinFeature,
    /// The contact solve of a bushed plate stalled instead of converging.
    ContactNotConverged,
    /// A steady thermal study with no fixed temperature and no convection: the temperature level is undetermined.
    NoThermalAnchor,
    /// A failure none of the rules recognise: the message is shown, with no remedy.
    Other,
}

impl FailureClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoSupports => "No support",
            Self::UnderConstrained => "Not fully supported",
            Self::UnknownSurface => "Unknown surface",
            Self::MeshTooFine => "Mesh too fine",
            Self::MeshTooCoarse => "Mesh too coarse",
            Self::HoleOutsideOutline => "Hole does not fit",
            Self::NonFiniteInput => "Invalid value",
            Self::NoThermalAnchor => "No fixed temperature",
            Self::InvertedElements => "Inverted elements",
            Self::ContactNotConverged => "Contact did not converge",
            Self::ThinFeature => "Feature too thin to mesh",
            Self::ScaleSuspect => "Implausibly large deformation",
            Self::Other => "Solve failed",
        }
    }
}

/// The kind of fix, independent of its arguments: what a ranker learns to prefer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RemedyKind {
    AddFixedSupport,
    AddRollerSupport,
    RetargetSurface,
    DropReference,
    CoarsenMesh,
    RefineMesh,
    RemoveHole,
    AddFixedTemperature,
    AddConvection,
    /// Turn imported elements of the opposite node orientation right side out.
    ReorientMesh,
    /// Give a bushing the plate's modulus (a unit mix-up is the usual reason for a bushing orders of magnitude off).
    MatchBushingModulus,
    /// Widen a hole that is thinner than the mesh can resolve.
    WidenHole,
    /// Not a fix: answering the finding's question instead of taking a fix. It is offered to the guidance ranker next to the
    /// remedies so it can learn when asking is what users do, and the card then shows the question first.
    AnswerQuestion,
}

/// A support or load by position in `Problem::supports` / `Problem::loads`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Support(usize),
    Load(usize),
    HeatSupport(usize),
    HeatLoad(usize),
}

/// One edit of the problem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Edit {
    /// An edit the recommender already knows (a support, the mesh size).
    Advice(AdviceEdit),
    /// Point a support or load at another surface.
    Retarget { target: Target, to: String },
    /// Remove a support or load.
    Drop(Target),
    /// Remove hole `n` (1-based, as its `holeN` name) from a sketch.
    RemoveHole(usize),
    /// Turn the elements of an imported mesh right side out when it is read (`MeshSpec::reorient`).
    Reorient,
    /// Set the Young's modulus of bushing `bushing` (0-based).
    SetBushingModulus { bushing: usize, e: f64 },
    /// Multiply the plate's Young's modulus by `factor`.
    ScaleModulus { factor: f64 },
    /// Make hole `n` (1-based) at least `to` thick: a circle's diameter, a slot's width, a rectangle's shorter side.
    WidenHole { hole: usize, to: f64 },
}

/// `Advice` is not serialisable (it carries the reason text); the edit part is what a remedy stores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AdviceEdit {
    Boundary { surface: String, kind: BcKind },
    MeshSize { size: f64 },
}

impl Edit {
    /// The surfaces of `p` this edit acts on, for a viewer to highlight while the user considers it (presentation only: the
    /// edit is not applied). A mesh-size edit touches the whole model and names none.
    pub fn surfaces(&self, p: &Problem) -> Vec<String> {
        let of_target = |t: &Target| -> Option<String> {
            match t {
                Target::Support(i) => p.supports.get(*i).and_then(|s| s.edge()).map(str::to_string),
                Target::Load(i) => p.loads.get(*i).and_then(advice::edge_of_load),
                Target::HeatSupport(i) => p.thermal.supports.get(*i).and_then(|s| s.edge()).map(str::to_string),
                Target::HeatLoad(i) => p.thermal.loads.get(*i).and_then(|l| l.edge()).map(str::to_string),
            }
        };
        match self {
            Edit::Advice(AdviceEdit::Boundary { surface, .. }) => vec![surface.clone()],
            Edit::Advice(AdviceEdit::MeshSize { .. }) => Vec::new(),
            Edit::Retarget { target, to } => of_target(target).into_iter().chain([to.clone()]).collect(),
            Edit::Drop(target) => of_target(target).into_iter().collect(),
            Edit::RemoveHole(n) => vec![format!("hole{n}")],
            Edit::Reorient => Vec::new(),
            Edit::WidenHole { hole, .. } => vec![format!("hole{hole}")],
            Edit::ScaleModulus { .. } => Vec::new(),
            Edit::SetBushingModulus { bushing, .. } => p.bushings.get(*bushing).map(|b| format!("hole{}", b.hole)).into_iter().collect(),
        }
    }

    pub fn apply(&self, p: &mut Problem) -> Result<(), String> {
        match self {
            Edit::Advice(AdviceEdit::Boundary { surface, kind }) => Advice::Boundary { surface: surface.clone(), kind: *kind, reason: String::new() }.apply(p),
            Edit::Advice(AdviceEdit::MeshSize { size }) => Advice::MeshSize { size: *size, reason: String::new() }.apply(p),
            Edit::Retarget { target, to } => match target {
                Target::Support(i) => match p.supports.get_mut(*i) {
                    Some(Support::Edge { edge, .. }) => {
                        *edge = to.clone();
                        Ok(())
                    }
                    _ => Err("that support does not act on a surface".into()),
                },
                Target::Load(i) => match p.loads.get_mut(*i) {
                    Some(Load::Pressure { edge, .. } | Load::Traction { edge, .. } | Load::Force { edge, .. }) => {
                        *edge = to.clone();
                        Ok(())
                    }
                    _ => Err("that load does not act on a surface".into()),
                },
                Target::HeatSupport(i) => match p.thermal.supports.get_mut(*i) {
                    Some(t) if t.edge().is_some() => {
                        t.set_edge(to);
                        Ok(())
                    }
                    _ => Err("that fixed temperature does not act on a surface".into()),
                },
                Target::HeatLoad(i) => match p.thermal.loads.get_mut(*i) {
                    Some(l) if l.edge().is_some() => {
                        l.set_edge(to);
                        Ok(())
                    }
                    _ => Err("that heat load does not act on a surface".into()),
                },
            },
            Edit::Reorient => {
                p.mesh.reorient = true;
                Ok(())
            }
            Edit::ScaleModulus { factor } => {
                if !(factor.is_finite() && *factor > 0.0) {
                    return Err("the factor must be positive".into());
                }
                p.material.e *= factor;
                Ok(())
            }
            Edit::WidenHole { hole, to } => match &mut p.geometry {
                Geometry::Sketch { holes, .. } if *hole >= 1 && *hole <= holes.len() => {
                    match &mut holes[hole - 1] {
                        crate::Shape::Circle { r, .. } => *r = 0.5 * to.max(2.0 * *r),
                        crate::Shape::Slot { width, length, .. } => {
                            *width = width.max(*to);
                            *length = length.max(1.5 * *width);
                        }
                        crate::Shape::Rect { x0, y0, x1, y1 } => {
                            if (*x1 - *x0) < *to {
                                *x1 = *x0 + *to;
                            }
                            if (*y1 - *y0) < *to {
                                *y1 = *y0 + *to;
                            }
                        }
                        _ => return Err("that hole's shape cannot be widened automatically".into()),
                    }
                    Ok(())
                }
                _ => Err("there is no such hole: the model changed".into()),
            },
            Edit::SetBushingModulus { bushing, e } => match p.bushings.get_mut(*bushing) {
                Some(b) => {
                    b.material.e = *e;
                    Ok(())
                }
                None => Err("there is no such bushing: the model changed".into()),
            },
            Edit::RemoveHole(n) => match &mut p.geometry {
                Geometry::Sketch { holes, .. } if *n >= 1 && *n <= holes.len() => {
                    holes.remove(n - 1);
                    // Loads and bushings that named the removed hole (or a later one) would now point at the wrong hole.
                    p.bushings.retain(|b| b.hole != *n);
                    for b in &mut p.bushings {
                        if b.hole > *n {
                            b.hole -= 1;
                        }
                    }
                    p.loads.retain(|l| !matches!(l, Load::Bearing { hole, .. } if hole == n));
                    for l in &mut p.loads {
                        if let Load::Bearing { hole, .. } = l {
                            if *hole > *n {
                                *hole -= 1;
                            }
                        }
                    }
                    Ok(())
                }
                _ => Err("there is no such hole: the model changed".into()),
            },
            Edit::Drop(target) => match target {
                Target::Support(i) if *i < p.supports.len() => {
                    p.supports.remove(*i);
                    Ok(())
                }
                Target::Load(i) if *i < p.loads.len() => {
                    p.loads.remove(*i);
                    Ok(())
                }
                Target::HeatSupport(i) if *i < p.thermal.supports.len() => {
                    p.thermal.supports.remove(*i);
                    Ok(())
                }
                Target::HeatLoad(i) if *i < p.thermal.loads.len() => {
                    p.thermal.loads.remove(*i);
                    Ok(())
                }
                _ => Err("nothing to remove: the model changed".into()),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Remedy {
    pub kind: RemedyKind,
    pub label: String,
    /// Why this is offered for this model.
    pub why: String,
    pub edit: Edit,
}

/// A question about intent whose answer picks an edit. The rules offer the question when the answer changes which remedy
/// is right and the model alone cannot say.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub id: &'static str,
    pub text: String,
    pub options: Vec<(String, Edit)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnosis {
    pub class: FailureClass,
    pub summary: String,
    /// Facts about this model that led to the class (numbers and names, not guesses).
    pub evidence: Vec<String>,
    /// Best first.
    pub remedies: Vec<Remedy>,
    pub questions: Vec<Question>,
}

/// Findings that need a result. A displacement that is a large share of the part's size cannot come from a linear model of a
/// sound design; the common cause is a modulus entered in a different unit from the loads and lengths. Nothing fails (the
/// solve is exact for the numbers it was given), so this is the only place the slip can be seen. The question offers the two
/// usual slips (a factor 1000 either way); a model that really deforms this much just ignores it.
pub fn diagnose_result(s: &crate::Solved) -> Vec<Diagnosis> {
    let p = &s.problem;
    if !p.study.is_structural() {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for n in &s.model.mesh.nodes {
        for k in 0..3 {
            lo[k] = lo[k].min(n[k]);
            hi[k] = hi[k].max(n[k]);
        }
    }
    let size = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f64, f64::max);
    let u = s.summary.max_displacement.value;
    if !(size > 0.0 && u.is_finite() && u > 0.05 * size) {
        return Vec::new();
    }
    let q = Question {
        id: "modulus_units",
        text: "Is Young's modulus in a different unit from the loads and lengths?".into(),
        options: vec![("Yes: it is 1000 times too small".into(), Edit::ScaleModulus { factor: 1e3 }), ("Yes: it is 1000 times too large".into(), Edit::ScaleModulus { factor: 1e-3 })],
    };
    vec![Diagnosis {
        class: FailureClass::ScaleSuspect,
        summary: format!("The largest displacement is {:.0} % of the part's size: a linear model is not trustworthy this far from its undeformed shape.", 100.0 * u / size),
        evidence: vec![format!("displacement {u:.4e}, part size {size:.4e}, modulus {:.3e}, peak von Mises {:.3e}", p.material.e, s.summary.max_von_mises.value)],
        remedies: Vec::new(),
        questions: vec![q],
    }]
}

/// Findings for `p`. `error` is the solver's failure message when a solve failed (`None`: the prevention pass, which
/// reports only what the model state already shows). Findings are ordered most blocking first.
pub fn diagnose(p: &Problem, error: Option<&str>) -> Vec<Diagnosis> {
    let mut out = Vec::new();
    out.extend(hole_misfits(p));
    out.extend(unknown_surfaces(p));
    out.extend(non_finite(p));
    if p.study.is_structural() && p.supports.is_empty() {
        out.push(no_supports(p));
    }
    out.extend(no_thermal_anchor(p));
    out.extend(error.and_then(|e| inverted_elements(p, e)));
    out.extend(error.and_then(|e| contact_not_converged(p, e)));
    out.extend(error.and_then(|e| thin_feature(p, e)));
    if p.study.is_structural() && error.is_some_and(|e| e.contains("rigid-body") && e.contains("constrained")) && !p.supports.is_empty() {
        out.push(under_constrained(p));
    }
    out.extend(mesh_too_fine(p));
    out.extend(mesh_too_coarse(p));
    if out.is_empty() {
        if let Some(e) = error {
            out.push(Diagnosis { class: FailureClass::Other, summary: e.to_string(), evidence: Vec::new(), remedies: Vec::new(), questions: Vec::new() });
        }
    }
    out
}

fn surface_of_support(s: &Support) -> Option<&str> {
    advice::edge_of_support(s)
}

fn lev(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut prev = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let cur = row[j];
            row[j] = (prev + usize::from(a[i - 1] != b[j - 1])).min(row[j] + 1).min(row[j - 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// The existing surface name closest to `name`, when it is a plausible slip (a few characters off).
fn closest(name: &str, names: &[String]) -> Option<String> {
    let best = names.iter().min_by_key(|n| lev(&name.to_lowercase(), &n.to_lowercase()))?;
    (lev(&name.to_lowercase(), &best.to_lowercase()) <= (name.chars().count() / 3).max(2)).then(|| best.clone())
}

fn unknown_surfaces(p: &Problem) -> Vec<Diagnosis> {
    let names = p.edge_names();
    // An imported mesh's names are only known once the file is read.
    if names.is_empty() || matches!(p.geometry, Geometry::Imported { .. }) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut check = |target: Target, name: &str, what: &str| {
        if names.iter().any(|n| n == name) {
            return;
        }
        let mut remedies = Vec::new();
        if let Some(to) = closest(name, &names) {
            remedies.push(Remedy { kind: RemedyKind::RetargetSurface, label: format!("Use '{to}' instead of '{name}'"), why: format!("'{to}' is the surface name closest to '{name}'"), edit: Edit::Retarget { target, to } });
        }
        remedies.push(Remedy { kind: RemedyKind::DropReference, label: format!("Remove the {what} on '{name}'"), why: format!("the geometry has no surface '{name}'"), edit: Edit::Drop(target) });
        out.push(Diagnosis {
            class: FailureClass::UnknownSurface,
            summary: format!("The {what} acts on '{name}', which the geometry does not have."),
            evidence: vec![format!("surfaces that exist: {}", names.join(", "))],
            remedies,
            questions: Vec::new(),
        });
    };
    for (i, s) in p.supports.iter().enumerate() {
        if let Some(n) = surface_of_support(s) {
            check(Target::Support(i), n, "support");
        }
    }
    for (i, l) in p.loads.iter().enumerate() {
        if let Load::Pressure { edge, .. } | Load::Traction { edge, .. } | Load::Force { edge, .. } = l {
            check(Target::Load(i), edge, "load");
        }
    }
    if p.study.is_thermal() {
        for (i, t) in p.thermal.supports.iter().enumerate() {
            if let Some(n) = t.edge() {
                check(Target::HeatSupport(i), n, "fixed temperature");
            }
        }
        for (i, l) in p.thermal.loads.iter().enumerate() {
            if let Some(n) = l.edge() {
                check(Target::HeatLoad(i), n, "heat load");
            }
        }
    }
    out
}

fn non_finite(p: &Problem) -> Vec<Diagnosis> {
    let bad = |v: &[f64]| v.iter().any(|x| !x.is_finite());
    let mut out = Vec::new();
    for (i, l) in p.loads.iter().enumerate() {
        let vals: Vec<f64> = match l {
            Load::Pressure { p, .. } => vec![*p],
            Load::Traction { tx, ty, tz, .. } => vec![*tx, *ty, *tz],
            Load::Force { fx, fy, fz, .. } => vec![*fx, *fy, *fz],
            Load::Bearing { fx, fy, .. } => vec![*fx, *fy],
            Load::Point { fx, fy, fz, .. } => vec![*fx, *fy, *fz],
            Load::Body { bx, by, bz } => vec![*bx, *by, *bz],
        };
        if bad(&vals) {
            out.push(Diagnosis {
                class: FailureClass::NonFiniteInput,
                summary: format!("Load {} has a value that is not a finite number.", i + 1),
                evidence: vec![format!("values: {vals:?}")],
                remedies: vec![Remedy { kind: RemedyKind::DropReference, label: format!("Remove load {}", i + 1), why: "a NaN or infinite load makes every result meaningless".into(), edit: Edit::Drop(Target::Load(i)) }],
                questions: Vec::new(),
            });
        }
    }
    out
}

/// Outline surfaces a first support could go on, with where each is (when the geometry has centroids), not yet supported.
fn support_candidates(p: &Problem) -> Vec<String> {
    let cents = advice::surface_centroids(p);
    let taken: Vec<&str> = p.supports.iter().filter_map(surface_of_support).collect();
    let loaded: Vec<String> = p.loads.iter().filter_map(advice::edge_of_load).collect();
    let loaded_refs: Vec<&str> = loaded.iter().map(String::as_str).collect();
    let mut names: Vec<(String, f64)> = if cents.is_empty() {
        p.edge_names().into_iter().filter(|n| !taken.contains(&n.as_str()) && !loaded.contains(n)).map(|n| (n, 0.0)).collect()
    } else {
        let away = advice::centroid_of(&loaded_refs, &cents);
        cents.iter().filter(|(n, _)| matches!(advice::surface_kind(n), SurfaceKind::OuterEdge | SurfaceKind::EndFace) && !taken.contains(&n.as_str()) && !loaded.contains(n)).map(|(n, c)| (n.clone(), away.map_or(0.0, |a| -advice::dist(*c, a)))).collect()
    };
    // Farthest from the load first (the most negative key), ties by name.
    names.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    names.into_iter().map(|(n, _)| n).collect()
}

fn support_remedies(p: &Problem, why_first: &str) -> (Vec<Remedy>, Vec<Question>) {
    let cands = support_candidates(p);
    let mut remedies = Vec::new();
    if let Some(first) = cands.first() {
        remedies.push(Remedy { kind: RemedyKind::AddFixedSupport, label: format!("Fix '{first}'"), why: format!("{why_first}; '{first}' is the surface farthest from the load"), edit: Edit::Advice(AdviceEdit::Boundary { surface: first.clone(), kind: BcKind::Fixed }) });
        remedies.push(Remedy { kind: RemedyKind::AddRollerSupport, label: format!("Hold '{first}' with a roller (symmetry)"), why: "a roller removes only the motion across the surface: right when the part is symmetric about it".into(), edit: Edit::Advice(AdviceEdit::Boundary { surface: first.clone(), kind: BcKind::Roller }) });
    }
    let questions = if cands.len() > 1 {
        vec![Question {
            id: "clamped_surface",
            text: "Which surface is held in reality (bolted, clamped, welded)?".into(),
            options: cands.iter().map(|n| (n.clone(), Edit::Advice(AdviceEdit::Boundary { surface: n.clone(), kind: BcKind::Fixed }))).collect(),
        }]
    } else {
        Vec::new()
    };
    (remedies, questions)
}

fn no_supports(p: &Problem) -> Diagnosis {
    let (remedies, questions) = support_remedies(p, "the model has no support");
    Diagnosis {
        class: FailureClass::NoSupports,
        summary: "The model has no support, so it is free to move.".into(),
        evidence: vec![format!("{} load(s), 0 supports", p.loads.len())],
        remedies,
        questions,
    }
}

/// The kernel refused an imported mesh because of elements with a non-positive Jacobian. A mesh exported with the opposite
/// node orientation has them all mirrored, which turning them right side out repairs exactly; collapsed or folded elements
/// (still refused after that) are the source program's to remesh.
fn inverted_elements(p: &Problem, error: &str) -> Option<Diagnosis> {
    let listed = error.contains("inverted or degenerate");
    let unrepairable = error.contains("cannot be turned right side out");
    if !(listed || unrepairable) || !matches!(p.geometry, Geometry::Imported { .. }) {
        return None;
    }
    let (summary, remedies) = if p.mesh.reorient || unrepairable {
        ("Some elements of the imported mesh are collapsed or folded; reversing their node order does not help.".to_string(), Vec::new())
    } else {
        (
            "The imported mesh has elements with a reversed node order (negative volume or area), so the solver refuses it.".to_string(),
            vec![Remedy {
                kind: RemedyKind::ReorientMesh,
                label: "Turn the reversed elements right side out when the mesh is read".into(),
                why: "a file written with the opposite orientation has its elements mirrored, which a node permutation repairs exactly; the file itself is not changed".into(),
                edit: Edit::Reorient,
            }],
        )
    };
    Some(Diagnosis { class: FailureClass::InvertedElements, summary, evidence: vec![error.to_string()], remedies, questions: Vec::new() })
}

/// Thickness of a hole: a circle's diameter, a slot's width, a rectangle's shorter side (`None`: not a simple shape).
fn hole_thickness(h: &crate::Shape) -> Option<f64> {
    match h {
        crate::Shape::Circle { r, .. } => Some(2.0 * r),
        crate::Shape::Slot { width, .. } => Some(*width),
        crate::Shape::Rect { x0, y0, x1, y1 } => Some((x1 - x0).abs().min((y1 - y0).abs())),
        _ => None,
    }
}

/// The mesher stopped at its vertex budget. A hole much thinner than the element size is the cause the model can show: the
/// refinement has to follow it down to its own width. Reproduced with a slot 1e-5 wide (minutes of refinement before this
/// guard, an error in seconds after). The remedies are the two honest choices: widen it to what the mesh can resolve, or take it out.
fn thin_feature(p: &Problem, error: &str) -> Option<Diagnosis> {
    if !error.contains("mesh exceeds") {
        return None;
    }
    let Geometry::Sketch { holes, .. } = &p.geometry else { return None };
    let (hole, thick) = holes.iter().enumerate().filter_map(|(i, h)| hole_thickness(h).map(|t| (i + 1, t))).min_by(|a, b| a.1.total_cmp(&b.1))?;
    if !(thick.is_finite() && thick < 0.2 * p.mesh.size) {
        return None;
    }
    let to = advice::nice(0.5 * p.mesh.size);
    Some(Diagnosis {
        class: FailureClass::ThinFeature,
        summary: format!("Hole {hole} is far thinner than the elements, so the mesher keeps refining until it runs out of budget."),
        evidence: vec![format!("hole {hole} is {thick:.3e} thick, the element size is {}", p.mesh.size)],
        remedies: vec![
            Remedy { kind: RemedyKind::WidenHole, label: format!("Widen hole {hole} to {to}"), why: "a feature has to be at least a fraction of an element wide to be meshed; widen it to what the mesh can resolve, or decide it does not matter".into(), edit: Edit::WidenHole { hole, to } },
            Remedy { kind: RemedyKind::RemoveHole, label: format!("Remove hole {hole}"), why: "a hair-thin feature carries no load of its own and costs the whole mesh".into(), edit: Edit::RemoveHole(hole) },
        ],
        questions: Vec::new(),
    })
}

/// The contact solve of a bushed plate gave up. Reproduced: a bushing whose modulus is 1e-7 of the plate's (a units slip) makes
/// the loaded solve stagnate. The rules name what the model shows (the stiffness ratio, the interference, the friction) and
/// offer a fix only for the cause they can see: a bushing orders of magnitude off the plate's modulus. Other stalls get the
/// evidence and no invented remedy.
fn contact_not_converged(p: &Problem, error: &str) -> Option<Diagnosis> {
    let stalled = error.contains("could not be solved") || error.contains("did not converge") || error.contains("NoConvergence");
    if !stalled || p.bushings.is_empty() {
        return None;
    }
    let mut evidence = Vec::new();
    let mut remedies = Vec::new();
    for (i, b) in p.bushings.iter().enumerate() {
        let ratio = b.material.e / p.material.e;
        evidence.push(format!("bushing {}: modulus {:.3e} against the plate's {:.3e} (ratio {ratio:.1e}), diametral interference {}, friction {}", i + 1, b.material.e, p.material.e, b.interference, b.friction));
        if ratio.is_finite() && !(1e-3..=1e3).contains(&ratio) && !remedies.iter().any(|r: &Remedy| matches!(r.edit, Edit::SetBushingModulus { bushing, .. } if bushing == i)) {
            remedies.push(Remedy {
                kind: RemedyKind::MatchBushingModulus,
                label: format!("Give bushing {} the plate's modulus ({:.3e})", i + 1, p.material.e),
                why: format!("its modulus is {:.0e} of the plate's: a units slip (psi against ksi, MPa against Pa) is the usual cause, and a fit between bodies that stiff-apart cannot be resolved; set the real value afterwards", ratio),
                edit: Edit::SetBushingModulus { bushing: i, e: p.material.e },
            });
        }
    }
    Some(Diagnosis { class: FailureClass::ContactNotConverged, summary: "The contact solve of the bushed plate did not converge.".into(), evidence, remedies, questions: Vec::new() })
}

/// A steady thermal study needs something to pin the temperature level: a fixed temperature or a convection surface.
fn no_thermal_anchor(p: &Problem) -> Option<Diagnosis> {
    if !p.study.is_thermal() || !p.thermal.supports.is_empty() || p.thermal.loads.iter().any(|l| matches!(l, crate::thermal::HeatLoad::Convection { .. })) {
        return None;
    }
    let names = p.edge_names();
    let heated: Vec<String> = p.thermal.loads.iter().filter_map(|l| l.edge().map(str::to_string)).collect();
    // The surface away from the heat input first; with none, the first.
    let free: Vec<String> = names.iter().filter(|n| !heated.contains(n) && matches!(advice::surface_kind(n), SurfaceKind::OuterEdge | SurfaceKind::EndFace | SurfaceKind::Hole)).cloned().collect();
    let mut remedies = Vec::new();
    if let Some(first) = free.first().or(names.first()) {
        remedies.push(Remedy { kind: RemedyKind::AddFixedTemperature, label: format!("Hold '{first}' at a fixed temperature"), why: "a heat sink at a known temperature fixes the level of the whole field".into(), edit: Edit::Advice(AdviceEdit::Boundary { surface: first.clone(), kind: BcKind::FixedTemperature }) });
        remedies.push(Remedy { kind: RemedyKind::AddConvection, label: format!("Cool '{first}' by convection"), why: "heat lost to a surrounding fluid also fixes the level, with the ambient temperature as the reference".into(), edit: Edit::Advice(AdviceEdit::Boundary { surface: first.clone(), kind: BcKind::Convection }) });
    }
    Some(Diagnosis {
        class: FailureClass::NoThermalAnchor,
        summary: "Nothing fixes the temperature level: add a fixed temperature or a convection surface.".into(),
        evidence: vec![format!("{} heat load(s), 0 fixed temperatures, no convection", p.thermal.loads.len())],
        remedies,
        questions: Vec::new(),
    })
}

fn under_constrained(p: &Problem) -> Diagnosis {
    let (remedies, questions) = support_remedies(p, "a rigid-body motion is still free");
    let held: Vec<String> = p.supports.iter().map(|s| match s {
        Support::Edge { edge, ux, uy, uz } => format!("{edge}: {}{}{}", if ux.is_some() { "x " } else { "" }, if uy.is_some() { "y " } else { "" }, if uz.is_some() { "z" } else { "" }),
        Support::Point { ux, uy, uz, .. } => format!("point: {}{}{}", if ux.is_some() { "x " } else { "" }, if uy.is_some() { "y " } else { "" }, if uz.is_some() { "z" } else { "" }),
    }).collect();
    Diagnosis {
        class: FailureClass::UnderConstrained,
        summary: "The supports leave the model free to move or turn.".into(),
        evidence: held,
        remedies,
        questions,
    }
}

fn mesh_too_fine(p: &Problem) -> Option<Diagnosis> {
    let Geometry::Sketch { layers, .. } = &p.geometry else { return None };
    let est = crate::build::estimate_mesh(p).ok()?.elements * if p.analysis == Analysis::Solid { *layers as f64 } else { 1.0 };
    if est <= crate::build::MAX_ELEMENTS {
        return None;
    }
    // Elements scale with 1/h^2: the size that lands at 60 % of the limit (room for local refinement).
    let size = p.mesh.size * (est / (0.6 * crate::build::MAX_ELEMENTS)).sqrt();
    Some(Diagnosis {
        class: FailureClass::MeshTooFine,
        summary: format!("The mesh would have about {est:.0} elements; the limit is {:.0}.", crate::build::MAX_ELEMENTS),
        evidence: vec![format!("element size {}, hole factor {}, {} refinement zone(s)", p.mesh.size, p.mesh.hole_factor, p.mesh.refine.len())],
        remedies: vec![Remedy { kind: RemedyKind::CoarsenMesh, label: format!("Set the element size to {:.4}", size), why: "elements grow as 1/size^2: this size brings the count to about 60 % of the limit".into(), edit: Edit::Advice(AdviceEdit::MeshSize { size }) }],
        questions: Vec::new(),
    })
}

fn hole_misfits(p: &Problem) -> Vec<Diagnosis> {
    crate::build::hole_problems(p)
        .into_iter()
        .map(|(n, why)| Diagnosis {
            class: FailureClass::HoleOutsideOutline,
            summary: format!("Hole {n} does not fit in the outline."),
            evidence: vec![why],
            remedies: vec![Remedy { kind: RemedyKind::RemoveHole, label: format!("Remove hole {n}"), why: "a hole must lie wholly inside the outline and clear of the other holes; edit its position or size instead if the hole is wanted".into(), edit: Edit::RemoveHole(n) }],
            questions: Vec::new(),
        })
        .collect()
}

fn mesh_too_coarse(p: &Problem) -> Option<Diagnosis> {
    let Geometry::Sketch { outer, .. } = &p.geometry else { return None };
    let (lo, hi) = outer.bounds();
    let thin = (hi[0] - lo[0]).min(hi[1] - lo[1]);
    if !(thin.is_finite() && thin > 0.0 && p.mesh.size.is_finite() && p.mesh.size > thin / 2.0) {
        return None;
    }
    let size = advice::nice(thin / 6.0);
    Some(Diagnosis {
        class: FailureClass::MeshTooCoarse,
        summary: "The elements are larger than half the thinnest section, so the mesh cannot resolve the part.".into(),
        evidence: vec![format!("element size {}, thinnest section {}", p.mesh.size, thin)],
        remedies: vec![Remedy { kind: RemedyKind::RefineMesh, label: format!("Set the element size to {size}"), why: "about 6 elements across the thinnest section is the usual minimum for a trustworthy stress".into(), edit: Edit::Advice(AdviceEdit::MeshSize { size }) }],
        questions: Vec::new(),
    })
}
