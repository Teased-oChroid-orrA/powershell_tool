//! Simulated training sessions for the two rankers (sketch constraint suggestions, boundary-condition kinds).
//!
//! This is a *prior*, not user data. Each session is scripted: a design is drawn (or a boundary-condition set is
//! built) through the same code paths a person uses, and the offers' outcomes are decided by an explicit
//! design-intent policy below, with per-session style and random slips so that sessions differ. The recorded offers
//! come out of `SketchEdit` / `advice::boundary_options` exactly as live recordings do, so situations, candidate
//! orders and the content-free format are the real ones. What is *not* real is who chooses: the policies encode
//! conventional drafting and structural-analysis practice as stated here, which is why `rank::train` weights the
//! corpus at `CORPUS_WEIGHT` of a real session and reports `Evidence::Simulated` until the user's own sessions are
//! enough to judge it. Deterministic in its seed.
use super::advising::{bc_situation_of, BcSituation};
use super::refining::{refine_situation, RefineLevel, RefineSituation};
use crate::gpu_viewer::editor::{
    rank::{Recorded, CORPUS_BIT},
    sketcher::{ConstraintSituation, SketchEdit, Tool},
};
use fea_problem::{
    advice::{self, BcKind},
    sketch::{infer::Rule, Sketch},
    templates::templates,
    Analysis, Geometry, Problem, Shape,
};

pub(super) struct Corpus {
    pub(super) constraints: Vec<Recorded<ConstraintSituation, Rule>>,
    pub(super) boundary: Vec<Recorded<BcSituation, BcKind>>,
    pub(super) refinement: Vec<Recorded<RefineSituation, RefineLevel>>,
}

/// Sessions of each ranker in the corpus the Advisor menu builds.
pub(super) const SESSIONS: usize = 600;
pub(super) const SEED: u64 = 0x5EED_C0DE;

pub(super) fn build(sessions: usize, seed: u64) -> Corpus {
    let mut rng = Rng(seed);
    let mut constraints = Vec::new();
    let mut boundary = Vec::new();
    let mut refinement = Vec::new();
    for n in 0..sessions as u64 {
        let id = CORPUS_BIT | n;
        constraints.extend(constraint_session(&mut rng).into_iter().map(|e| Recorded { session: id, situation: e.situation, offered: e.offered, taken: e.taken }));
        boundary.extend(boundary_session(&mut rng).into_iter().map(|(situation, offered, taken)| Recorded { session: id | 1 << 40, situation, offered, taken }));
        refinement.extend(refine_session(&mut rng).into_iter().map(|(situation, offered, taken)| Recorded { session: id | 2 << 40, situation, offered, taken }));
    }
    Corpus { constraints, boundary, refinement }
}

// ------------------------------------------------------------------------------------------------ randomness

/// SplitMix64: small, seedable, good enough to vary scripted sessions.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

// ------------------------------------------------------------------------------------------------ constraints

#[derive(Clone, Copy, PartialEq)]
enum Family {
    /// Rectangle with round holes.
    Plate,
    /// Axis-aligned L / T outline drawn as a polyline.
    Bracket,
    /// Two parallel lines closed by two arcs.
    Slot,
    /// Angled outlines: parallelogram, right triangle, hexagon.
    Freeform,
    /// Equal holes on a bolt circle around a larger bore.
    BoltCircle,
    /// A line, a tangent fillet arc, a line.
    Fillet,
}
const FAMILIES: [Family; 6] = [Family::Plate, Family::Bracket, Family::Slot, Family::Freeform, Family::BoltCircle, Family::Fillet];

/// The order a designer applies suggestions in: alignment first, then joins and tangency, then relations.
const PRIORITY: [Rule; 9] = [Rule::Horizontal, Rule::Vertical, Rule::Tangent, Rule::Coincident, Rule::Equal, Rule::Parallel, Rule::Perpendicular, Rule::PointOn, Rule::Midpoint];

/// How likely a designer drawing `family` takes a suggestion of `rule` (design intent, stated).
fn intent(family: Family, rule: Rule) -> f64 {
    use Family::*;
    use Rule::*;
    match rule {
        // Axis-aligned designs are meant to be square to the axes; angled ones only sometimes (right angles).
        Horizontal | Vertical => match family {
            Plate | Bracket | Slot | Fillet => 0.93,
            Freeform => 0.5,
            BoltCircle => 0.3,
        },
        // Tangency is the point of slots and fillets; elsewhere a near-tangent is usually a coincidence.
        Tangent => match family {
            Slot | Fillet => 0.94,
            _ => 0.12,
        },
        Coincident => match family {
            Slot | Fillet => 0.85,
            _ => 0.5,
        },
        Equal => match family {
            Freeform | BoltCircle => 0.85,
            Plate => 0.65,
            Slot => 0.5,
            Bracket => 0.35,
            Fillet => 0.3,
        },
        Parallel => match family {
            Freeform => 0.85,
            Slot => 0.6,
            Bracket => 0.3,
            Plate | Fillet | BoltCircle => 0.15,
        },
        // Redundant after horizontal / vertical in axis-aligned designs.
        Perpendicular => match family {
            Freeform => 0.6,
            Fillet => 0.3,
            Bracket => 0.25,
            _ => 0.1,
        },
        PointOn => match family {
            BoltCircle => 0.6,
            _ => 0.3,
        },
        Midpoint => 0.15,
    }
}

struct Draw<'a> {
    sk: Sketch,
    ed: SketchEdit,
    rng: &'a mut Rng,
    family: Family,
    /// This session's designer: 0.5 accepts about half of what they would, 1.0 everything they would.
    thoroughness: f64,
    /// Click error as a fraction of the design's size (beyond the snap distance a suggestion appears).
    jitter: f64,
    scale: f64,
    /// The first point has been pinned.
    pinned: bool,
}

impl Draw<'_> {
    fn tool(&mut self, t: Tool) {
        self.ed.set_tool(t);
    }
    fn click(&mut self, p: [f64; 2]) {
        let j = self.jitter * self.scale;
        let at = [p[0] + self.rng.range(-j, j), p[1] + self.rng.range(-j, j)];
        self.place(at);
    }
    /// Close a polyline / loop exactly on its first point (the snap does the rest).
    fn click_exact(&mut self, p: [f64; 2]) {
        self.place(p);
    }
    fn place(&mut self, at: [f64; 2]) {
        if let Ok(created) = self.ed.click(&mut self.sk, at) {
            self.decide();
            self.dimension(&created);
        }
    }
    /// Designers pin the sketch and size what they drew, which is what moves the degrees of freedom (and so the
    /// situations later offers are made in) from "many" towards "none".
    fn dimension(&mut self, created: &[fea_problem::sketch::Id]) {
        use fea_problem::sketch::{Constraint, Entity};
        for &id in created {
            let mut touched = false;
            match self.sk.entity(id).cloned() {
                Some(Entity::Line { a, .. }) => {
                    if !self.pinned && self.rng.chance(0.7) {
                        if let Some(p) = self.sk.point(a) {
                            touched |= self.sk.add_constraint(Constraint::Fixed { p: a, x: p[0], y: p[1] }).is_some();
                        }
                        self.pinned = true;
                    }
                    if self.rng.chance(0.45 * self.thoroughness) {
                        if let Some((p, q)) = self.sk.line_pts(id) {
                            let d = ((p[0] - q[0]).hypot(p[1] - q[1]) * 10.0).round() / 10.0;
                            if d > 0.0 {
                                touched |= self.sk.add_constraint(Constraint::Length { line: id, d }).is_some();
                            }
                        }
                    }
                }
                Some(Entity::Circle { .. } | Entity::Arc { .. }) if self.rng.chance(0.45 * self.thoroughness) => {
                    if let Some((_, r)) = self.sk.round(id) {
                        let d = (r * 10.0).round() / 10.0;
                        if d > 0.0 {
                            touched |= self.sk.add_constraint(Constraint::Radius { e: id, d }).is_some();
                        }
                    }
                }
                _ => {}
            }
            if touched {
                let report = self.sk.solve();
                if report.status == fea_problem::sketch::solve::Status::Conflicting {
                    // A dimension that fights an accepted constraint is dropped, as the editor does.
                    if let Some(last) = self.sk.constraints.last().map(|c| c.id) {
                        self.sk.remove_constraint(last);
                    }
                }
                self.ed.last_report = Some((&self.sk.solve()).into());
            }
        }
    }
    fn decide(&mut self) {
        for rule in PRIORITY {
            if let Some(i) = self.ed.suggestions.iter().position(|s| s.rule == rule) {
                if self.rng.chance(intent(self.family, rule) * self.thoroughness) {
                    let _ = self.ed.accept(&mut self.sk, i);
                }
            }
        }
    }
    fn polyline(&mut self, pts: &[[f64; 2]]) {
        self.tool(Tool::Polyline);
        for p in pts {
            self.click(*p);
        }
        self.click_exact(pts[0]);
        self.tool(Tool::Select);
    }
}

fn constraint_session(rng: &mut Rng) -> Vec<crate::gpu_viewer::editor::sketcher::ConstraintEvent> {
    let family = FAMILIES[rng.below(FAMILIES.len())];
    let scale = rng.range(2.0, 40.0);
    let (thoroughness, jitter) = (rng.range(0.45, 1.0), rng.range(0.004, 0.02));
    let mut ed = SketchEdit::new();
    ed.pick = 0.012 * scale;
    let mut d = Draw { sk: Sketch::default(), ed, rng, family, thoroughness, jitter, scale, pinned: false };
    let (w, h) = (scale, scale * d.rng.range(0.3, 0.9));
    match family {
        Family::Plate => {
            d.tool(Tool::Rectangle);
            d.click([0.0, 0.0]);
            d.click([w, h]);
            d.tool(Tool::Circle);
            let r = h * d.rng.range(0.08, 0.2);
            for k in 0..1 + d.rng.below(3) {
                let cx = w * (0.2 + 0.3 * k as f64 + d.rng.range(-0.03, 0.03));
                d.click([cx, h * 0.5]);
                d.click([cx + r, h * 0.5]);
            }
        }
        Family::Bracket => {
            let (a, b, c) = (w * d.rng.range(0.3, 0.6), h * d.rng.range(0.3, 0.6), w * d.rng.range(0.2, 0.4));
            if d.rng.chance(0.5) {
                // L
                d.polyline(&[[0.0, 0.0], [w, 0.0], [w, b], [a, b], [a, h], [0.0, h]]);
            } else {
                // T
                d.polyline(&[[0.0, h], [0.0, h - b], [(w - c) / 2.0, h - b], [(w - c) / 2.0, 0.0], [(w + c) / 2.0, 0.0], [(w + c) / 2.0, h - b], [w, h - b], [w, h]]);
            }
        }
        Family::Slot => {
            let r = h * d.rng.range(0.2, 0.5);
            d.tool(Tool::Line);
            d.click([0.0, r]);
            d.click([w, r]);
            d.click([0.0, -r]);
            d.click([w, -r]);
            d.tool(Tool::Arc);
            d.click([w, 0.0]);
            d.click([w, -r]);
            d.click([w, r]);
            d.click([0.0, 0.0]);
            d.click([0.0, r]);
            d.click([0.0, -r]);
        }
        Family::Freeform => match d.rng.below(3) {
            0 => {
                let (b, t) = (h * d.rng.range(0.6, 1.2), d.rng.range(0.9, 1.25));
                d.polyline(&[[0.0, 0.0], [w, 0.0], [w + b * t.cos(), b * t.sin()], [b * t.cos(), b * t.sin()]]);
            }
            1 => d.polyline(&[[0.0, 0.0], [w, 0.0], [0.0, h]]),
            _ => {
                let r = w * 0.5;
                let pts: Vec<[f64; 2]> = (0..6).map(|k| [r * (k as f64 * std::f64::consts::FRAC_PI_3).cos(), r * (k as f64 * std::f64::consts::FRAC_PI_3).sin()]).collect();
                d.polyline(&pts);
            }
        },
        Family::BoltCircle => {
            d.tool(Tool::Circle);
            let rb = w * 0.4;
            d.click([0.0, 0.0]);
            d.click([w * 0.12, 0.0]);
            let n = 3 + d.rng.below(5);
            let r = w * d.rng.range(0.03, 0.06);
            for k in 0..n {
                let t = std::f64::consts::TAU * k as f64 / n as f64;
                let c = [rb * t.cos(), rb * t.sin()];
                d.click(c);
                d.click([c[0] + r, c[1]]);
            }
        }
        Family::Fillet => {
            let r = h * d.rng.range(0.15, 0.4);
            d.tool(Tool::Line);
            d.click([0.0, 0.0]);
            d.click([w, 0.0]);
            d.tool(Tool::Arc);
            d.click([w, r]);
            d.click([w, 0.0]);
            d.click([w + r, r]);
            d.tool(Tool::Line);
            d.click([w + r, r]);
            d.click([w + r, h + r]);
        }
    }
    d.tool(Tool::Select);
    d.ed.take_events()
}

// ------------------------------------------------------------------------------------------------ boundary conditions

/// A random starting problem: its geometry decides which surfaces exist.
fn scenario(rng: &mut Rng) -> Problem {
    let mut p = templates().remove(0).1;
    p.supports.clear();
    p.loads.clear();
    p.bushings.clear();
    p.history = None;
    p.analysis = match rng.below(20) {
        0..=6 => Analysis::PlaneStress,
        7..=9 => Analysis::PlaneStrain,
        10..=13 => Analysis::Axisymmetric,
        _ => Analysis::Solid,
    };
    let (w, h) = (rng.range(4.0, 30.0), rng.range(2.0, 15.0));
    let outer = if p.analysis != Analysis::Axisymmetric && rng.chance(0.2) {
        Shape::Polygon { pts: vec![[0.0, 0.0], [w, 0.0], [w, h * 0.5], [w * 0.4, h * 0.5], [w * 0.4, h], [0.0, h]] }
    } else {
        Shape::Rect { x0: 0.0, y0: 0.0, x1: w, y1: h }
    };
    let holes: Vec<Shape> = if p.analysis == Analysis::Axisymmetric {
        Vec::new()
    } else {
        (0..rng.below(4)).map(|k| Shape::Circle { cx: w * (0.15 + 0.25 * k as f64), cy: h * 0.25, r: h * rng.range(0.04, 0.1) }).collect()
    };
    p.geometry = Geometry::Sketch { outer, holes, depth: 1.0, layers: 4, regions: Vec::new() };
    p
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// What a competent analyst would do next on `p` (a surface and a kind), by structural-analysis practice: restrain the
/// body first (a clamped end, a bolted hole), then load the far side (pin load on a hole, pressure on an edge), then
/// refine with symmetry rollers or further loads; axisymmetric sections are restrained by their symmetry planes and
/// loaded by pressure on the bore and the outside.
fn next_step(p: &Problem, used: &[(String, BcKind)], rng: &mut Rng) -> Option<(String, BcKind)> {
    let names = p.edge_names();
    let has = |n: &str| names.iter().any(|x| x == n);
    let fresh = |s: &str, k: BcKind| !used.iter().any(|(u, uk)| u == s && *uk == k);
    let plane = matches!(p.analysis, Analysis::PlaneStress | Analysis::PlaneStrain);
    let solid = p.analysis == Analysis::Solid;
    let holes: Vec<&String> = names.iter().filter(|n| n.starts_with("hole")).collect();
    let (n_sup, n_load) = (p.supports.len(), p.loads.len());
    let pick = if p.analysis == Analysis::Axisymmetric {
        match (n_sup, n_load) {
            (0, _) => ("bottom".to_string(), BcKind::Roller),
            (1, _) => ("top".to_string(), BcKind::Roller),
            (_, 0) => ("left".to_string(), BcKind::Pressure),
            _ => ("right".to_string(), BcKind::Pressure),
        }
    } else if n_sup == 0 {
        let first_edge = names.iter().find(|n| !n.starts_with("hole") && *n != "start" && *n != "end").cloned().unwrap_or_default();
        if solid && has("start") && rng.chance(0.5) {
            ("start".to_string(), BcKind::Fixed)
        } else if !holes.is_empty() && rng.chance(0.3) {
            (holes[rng.below(holes.len())].clone(), BcKind::Fixed)
        } else if has("left") {
            ("left".to_string(), BcKind::Fixed)
        } else {
            (first_edge, BcKind::Fixed)
        }
    } else if n_load == 0 {
        // The surface farthest from what is restrained.
        let cents = advice::surface_centroids(p);
        let held: Vec<[f64; 2]> = cents.iter().filter(|(n, _)| used.iter().any(|(u, _)| u == n)).map(|(_, c)| *c).collect();
        let far = cents
            .iter()
            .filter(|(n, _)| !used.iter().any(|(u, _)| u == n))
            .max_by(|a, b| held.iter().map(|h| dist(*h, a.1)).fold(f64::MAX, f64::min).total_cmp(&held.iter().map(|h| dist(*h, b.1)).fold(f64::MAX, f64::min)))
            .map(|(n, _)| n.clone());
        match far {
            Some(n) if n.starts_with("hole") => (n, if plane { BcKind::Bearing } else { BcKind::Pressure }),
            _ if solid && has("end") => ("end".to_string(), BcKind::Pressure),
            Some(n) => (n, BcKind::Pressure),
            None => return None,
        }
    } else {
        match rng.below(3) {
            0 if solid && has("end") => ("end".to_string(), BcKind::Roller),
            0 if has("bottom") => ("bottom".to_string(), BcKind::Roller),
            1 if has("top") => ("top".to_string(), BcKind::Pressure),
            _ if !holes.is_empty() && plane => (holes[rng.below(holes.len())].clone(), BcKind::Bearing),
            _ => (names.get(rng.below(names.len().max(1)))?.clone(), BcKind::Pressure),
        }
    };
    fresh(&pick.0, pick.1).then_some(pick)
}

fn boundary_session(rng: &mut Rng) -> Vec<(BcSituation, Vec<BcKind>, Option<BcKind>)> {
    let mut p = scenario(rng);
    let mut used: Vec<(String, BcKind)> = Vec::new();
    let mut out = Vec::new();
    // Some sessions start from a model that already has a support or a load (a loaded template, an earlier session).
    if rng.chance(0.25) {
        if let Some(n) = p.edge_names().first().cloned() {
            let _ = advice::add_boundary(&mut p, &n, BcKind::Fixed);
            used.push((n, BcKind::Fixed));
        }
    }
    for _ in 0..3 + rng.below(5) {
        let Some((surface, wanted)) = next_step(&p, &used, rng) else { break };
        let options = advice::boundary_options(&p, &surface);
        let situation = bc_situation_of(&p, &surface);
        // The analyst wants `wanted`; they sometimes slip to another offered kind, or close the popup unanswered.
        let taken = if rng.chance(0.08) {
            None
        } else if rng.chance(0.08) {
            Some(options[rng.below(options.len())])
        } else if options.contains(&wanted) {
            Some(wanted)
        } else {
            None
        };
        out.push((situation, options, taken));
        if let Some(kind) = taken {
            if advice::add_boundary(&mut p, &surface, kind).is_ok() {
                used.push((surface, kind));
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ mesh density

/// Mesh-density sessions: a scenario with a few boundary conditions and a mesh size, then the analyst applies some of
/// the recommended zones. The level they choose follows practice: a pin- or bushing-loaded hole and a barely resolved
/// feature get the finest level; singular re-entrant corners are refined but not chased (adaptive passes do the rest),
/// so level 2 as often as 3; supports and loads (St Venant: the effect dies out) get the lightest level unless the
/// mesh is coarse there. Regions an analyst does not touch leave no event, as in the editor.
fn refine_session(rng: &mut Rng) -> Vec<(RefineSituation, Vec<RefineLevel>, Option<RefineLevel>)> {
    let mut p = scenario(rng);
    if p.analysis == Analysis::Axisymmetric {
        p.mesh.size = rng.range(0.1, 0.6);
    } else {
        p.mesh.size = rng.range(0.1, 1.5);
    }
    p.history = None;
    for _ in 0..rng.below(4) {
        if let Some((surface, kind)) = next_step(&p, &[], rng) {
            let _ = advice::add_boundary(&mut p, &surface, kind);
        }
    }
    let thoroughness = rng.range(0.4, 1.0);
    let mut out = Vec::new();
    for r in advice::refinement_regions(&p) {
        if !rng.chance(thoroughness) {
            continue;
        }
        let coarse = r.resolution == advice::Resolution::Coarse;
        let weights: [f64; 3] = match (r.kind, r.loaded, coarse) {
            (advice::RegionKind::Hole, true, _) => [0.02, 0.18, 0.8],
            (advice::RegionKind::Hole, false, true) => [0.02, 0.28, 0.7],
            (advice::RegionKind::Hole, false, false) => [0.1, 0.75, 0.15],
            (advice::RegionKind::ReentrantCorner, _, _) => [0.1, 0.45, 0.45],
            (advice::RegionKind::Support | advice::RegionKind::Load, _, true) => [0.35, 0.5, 0.15],
            (advice::RegionKind::Support | advice::RegionKind::Load, _, false) => [0.7, 0.25, 0.05],
        };
        let mut pick = rng.unit();
        let level = RefineLevel::ALL.into_iter().zip(weights).find_map(|(l, w)| {
            pick -= w;
            (pick <= 0.0).then_some(l)
        });
        let offered: Vec<RefineLevel> = advice::level_order(r.kind, r.loaded).into_iter().filter_map(RefineLevel::of).collect();
        out.push((refine_situation(&p, &r), offered, level.or(Some(RefineLevel::Two))));
    }
    out
}
