//! Rule-based recommendations for a structural problem: where a support or load most plausibly goes, and mesh
//! settings that the geometry calls for. Deterministic and explained: every item carries the reason it is made. It only
//! *proposes* edits (`Advice::apply`); nothing is applied silently and the solver's own checks stay authoritative.
//! A learned ranker (app-tui) may reorder `boundary_options`, never add to them.
//!
//! Mesh density: `refinement_regions` proposes where a finer mesh is worth having (round holes, re-entrant corners, where
//! loads and supports act) and at which level; `apply_region` turns one into a `MeshSpec::refine` zone.
//!
//! Scope: the `Problem` schema is structural (plane stress / strain, axisymmetric, solid) with a uniform temperature
//! change; there is no thermal-conduction problem type here yet, so no thermal advice exists.
use crate::problem::{Analysis, Geometry, Load, Problem, Seg, Shape, Support};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BcKind {
    Fixed,
    Roller,
    Pressure,
    Bearing,
}
impl BcKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "fixed support",
            Self::Roller => "roller support",
            Self::Pressure => "pressure load",
            Self::Bearing => "pin (bearing) load",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SurfaceKind {
    Hole,
    OuterEdge,
    EndFace,
    Other,
}

pub fn surface_kind(name: &str) -> SurfaceKind {
    match name {
        "start" | "end" => SurfaceKind::EndFace,
        n if n.starts_with("hole") || n.starts_with("plate_hole") => SurfaceKind::Hole,
        "bottom" | "right" | "top" | "left" | "outer" => SurfaceKind::OuterEdge,
        n if n.starts_with("edge") => SurfaceKind::OuterEdge,
        _ => SurfaceKind::Other,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Advice {
    Boundary { surface: String, kind: BcKind, reason: String },
    MeshSize { size: f64, reason: String },
    HoleFactor { factor: f64, reason: String },
    Layers { layers: usize, reason: String },
}

impl Advice {
    pub fn label(&self) -> String {
        match self {
            Advice::Boundary { surface, kind, .. } => format!("Add a {} on {surface}", kind.label()),
            Advice::MeshSize { size, .. } => format!("Set the element size to {}", short(*size)),
            Advice::HoleFactor { factor, .. } => format!("Set the hole size factor to {}", short(*factor)),
            Advice::Layers { layers, .. } => format!("Use {layers} layers through the thickness"),
        }
    }
    pub fn reason(&self) -> &str {
        match self {
            Advice::Boundary { reason, .. } | Advice::MeshSize { reason, .. } | Advice::HoleFactor { reason, .. } | Advice::Layers { reason, .. } => reason,
        }
    }
    /// Make the edit on `p`.
    pub fn apply(&self, p: &mut Problem) -> Result<(), String> {
        match self {
            Advice::Boundary { surface, kind, .. } => add_boundary(p, surface, *kind),
            Advice::MeshSize { size, .. } => {
                p.mesh.size = *size;
                Ok(())
            }
            Advice::HoleFactor { factor, .. } => {
                p.mesh.hole_factor = *factor;
                Ok(())
            }
            Advice::Layers { layers, .. } => match &mut p.geometry {
                Geometry::Sketch { layers: l, .. } => {
                    *l = *layers;
                    Ok(())
                }
                Geometry::Imported { .. } | Geometry::Solid { .. } => Err("an imported mesh or a solid has no layers".into()),
            },
        }
    }
}

fn short(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Add the support / load of `kind` on `surface`.
pub fn add_boundary(p: &mut Problem, surface: &str, kind: BcKind) -> Result<(), String> {
    match kind {
        BcKind::Fixed => p.supports.push(Support::fixed(surface)),
        BcKind::Roller => {
            // Hold the component normal to the surface.
            let comp = match surface {
                "bottom" | "top" => 1,
                "start" | "end" => 2,
                _ => 0,
            };
            p.supports.push(Support::roller(surface, comp));
        }
        BcKind::Pressure => p.loads.push(Load::Pressure { edge: surface.to_string(), p: 1000.0 }),
        BcKind::Bearing => {
            let n: usize = surface.strip_prefix("hole").and_then(|d| d.parse().ok()).ok_or("a bearing load acts on a hole")?;
            if !matches!(p.analysis, Analysis::PlaneStress | Analysis::PlaneStrain) {
                return Err("a bearing load is for plane problems".into());
            }
            p.loads.push(Load::Bearing { hole: n, fx: 1000.0, fy: 0.0 });
        }
    }
    Ok(())
}

/// Centroid of every named surface of a sketch problem (hole and outline edges), in the sketch plane.
pub fn surface_centroids(p: &Problem) -> Vec<(String, [f64; 2])> {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Vec::new() };
    let mut out = Vec::new();
    match outer {
        Shape::Rect { x0, y0, x1, y1 } => {
            let (mx, my) = (0.5 * (x0 + x1), 0.5 * (y0 + y1));
            out.extend([("bottom".to_string(), [mx, *y0]), ("right".into(), [*x1, my]), ("top".into(), [mx, *y1]), ("left".into(), [*x0, my])]);
        }
        Shape::Polygon { pts } => {
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                out.push((format!("edge{}", i + 1), [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]));
            }
        }
        Shape::Outline { segs } => out.extend(segs.iter().enumerate().map(|(i, sg)| (format!("edge{}", i + 1), sg.point(0.5)))),
        Shape::Circle { .. } | Shape::Slot { .. } => out.push(("outer".into(), outer.centre())),
    }
    for (i, h) in holes.iter().enumerate() {
        out.push((format!("hole{}", i + 1), h.centre()));
    }
    out
}

/// End points of every straight outline edge of a sketch problem, by surface name (rectangle and polygon outlines).
pub fn surface_edges(p: &Problem) -> Vec<(String, [[f64; 2]; 2])> {
    let Geometry::Sketch { outer, .. } = &p.geometry else { return Vec::new() };
    match outer {
        Shape::Rect { x0, y0, x1, y1 } => vec![
            ("bottom".to_string(), [[*x0, *y0], [*x1, *y0]]),
            ("right".into(), [[*x1, *y0], [*x1, *y1]]),
            ("top".into(), [[*x1, *y1], [*x0, *y1]]),
            ("left".into(), [[*x0, *y1], [*x0, *y0]]),
        ],
        Shape::Polygon { pts } => (0..pts.len()).map(|i| (format!("edge{}", i + 1), [pts[i], pts[(i + 1) % pts.len()]])).collect(),
        Shape::Outline { segs } => segs.iter().enumerate().filter_map(|(i, sg)| matches!(sg, Seg::Line { .. }).then(|| (format!("edge{}", i + 1), [sg.start(), sg.end()]))).collect(),
        Shape::Circle { .. } | Shape::Slot { .. } => Vec::new(),
    }
}

pub(crate) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub(crate) fn centroid_of(names: &[&str], all: &[(String, [f64; 2])]) -> Option<[f64; 2]> {
    let pts: Vec<[f64; 2]> = all.iter().filter(|(n, _)| names.contains(&n.as_str())).map(|(_, c)| *c).collect();
    (!pts.is_empty()).then(|| [pts.iter().map(|c| c[0]).sum::<f64>() / pts.len() as f64, pts.iter().map(|c| c[1]).sum::<f64>() / pts.len() as f64])
}

pub(crate) fn edge_of_support(s: &Support) -> Option<&str> {
    match s {
        Support::Edge { edge, .. } => Some(edge),
        Support::Point { .. } => None,
    }
}
pub(crate) fn edge_of_load(l: &Load) -> Option<String> {
    match l {
        Load::Pressure { edge, .. } | Load::Traction { edge, .. } | Load::Force { edge, .. } => Some(edge.clone()),
        Load::Bearing { hole, .. } => Some(format!("hole{hole}")),
        Load::Point { .. } | Load::Body { .. } => None,
    }
}

/// Supports and loads that the problem could reasonably still use, with reasons.
pub fn recommend(p: &Problem) -> Vec<Advice> {
    let mut out = Vec::new();
    let cents = surface_centroids(p);
    let support_names: Vec<&str> = p.supports.iter().filter_map(edge_of_support).collect();
    let load_names_owned: Vec<String> = p.loads.iter().filter_map(edge_of_load).collect();
    let load_names: Vec<&str> = load_names_owned.iter().map(String::as_str).collect();
    let outer_edges: Vec<&(String, [f64; 2])> = cents.iter().filter(|(n, _)| surface_kind(n) == SurfaceKind::OuterEdge).collect();
    let plane = matches!(p.analysis, Analysis::PlaneStress | Analysis::PlaneStrain);

    // Mesh (geometry-driven, independent of the problem type).
    if let Geometry::Sketch { outer, holes, .. } = &p.geometry {
        let (lo, hi) = outer.bounds();
        let thin = (hi[0] - lo[0]).min(hi[1] - lo[1]);
        let by_section = thin / 6.0;
        let min_hole_r = holes.iter().map(|h| h.reach()).fold(f64::INFINITY, f64::min);
        if by_section.is_finite() && by_section > 0.0 && p.mesh.size > by_section * 1.5 {
            let size = nice(by_section);
            let est = (hi[0] - lo[0]) * (hi[1] - lo[1]) / (size * size) * crate::build::elements_per_area(p.mesh.element.is_quad());
            if est < crate::build::MAX_ELEMENTS * 0.5 {
                out.push(Advice::MeshSize { size, reason: format!("the element size {} puts fewer than 6 elements across the thinnest section ({}); {} gives about 6", short(p.mesh.size), short(thin), short(size)) });
            }
        }
        if min_hole_r.is_finite() && min_hole_r > 0.0 && p.mesh.size * p.mesh.hole_factor > min_hole_r / 3.0 {
            let factor = ((min_hole_r / 3.0) / p.mesh.size).clamp(0.05, 1.0);
            let factor = (factor * 100.0).floor() / 100.0;
            out.push(Advice::HoleFactor { factor: factor.max(0.05), reason: format!("elements at the smallest hole ({}) are about {} long: under 3 along its radius; stress concentrations need finer elements there", short(min_hole_r), short(p.mesh.size * p.mesh.hole_factor)) });
        }
        if p.analysis == Analysis::Solid {
            if let Geometry::Sketch { depth, layers, .. } = &p.geometry {
                if *depth < 0.5 * thin && *layers < 3 {
                    out.push(Advice::Layers { layers: 3, reason: format!("a thin extrusion ({} deep) loaded in bending needs at least 3 elements through the thickness, it has {layers}", short(*depth)) });
                }
            }
        }
    }

    // Boundary conditions.
    if p.supports.is_empty() {
        let away_from = centroid_of(&load_names, &cents);
        let pick = match away_from {
            Some(l) => outer_edges.iter().max_by(|a, b| dist(a.1, l).total_cmp(&dist(b.1, l))),
            None => outer_edges.iter().min_by(|a, b| a.1[0].total_cmp(&b.1[0])),
        };
        if let Some((name, _)) = pick {
            let why = if away_from.is_some() { format!("the model is free to move; '{name}' is the outline edge farthest from the load") } else { format!("the model is free to move; '{name}' is the outline edge on the low-x side") };
            out.push(Advice::Boundary { surface: name.clone(), kind: BcKind::Fixed, reason: why });
        }
    } else if p.loads.is_empty() {
        let hole = cents.iter().find(|(n, _)| surface_kind(n) == SurfaceKind::Hole && !support_names.contains(&n.as_str()));
        if let (Some((h, _)), true) = (hole, plane) {
            out.push(Advice::Boundary { surface: h.clone(), kind: BcKind::Bearing, reason: format!("there is a support but no load; a pin usually loads a hole, and '{h}' is unused") });
        }
        if let Some(from) = centroid_of(&support_names, &cents) {
            let far = outer_edges.iter().filter(|(n, _)| !support_names.contains(&n.as_str())).max_by(|a, b| dist(a.1, from).total_cmp(&dist(b.1, from)));
            if let Some((name, _)) = far {
                out.push(Advice::Boundary { surface: name.clone(), kind: BcKind::Pressure, reason: format!("there is a support but no load; '{name}' is the outline edge farthest from the support") });
            }
        }
    }
    out
}

/// Support / load kinds that make sense on `surface` now, in the order the rules prefer (the intent: a first
/// support, then a load, then refinements).
pub fn boundary_options(p: &Problem, surface: &str) -> Vec<BcKind> {
    let plane = matches!(p.analysis, Analysis::PlaneStress | Analysis::PlaneStrain);
    match surface_kind(surface) {
        SurfaceKind::Hole => {
            if plane && p.supports.is_empty() {
                vec![BcKind::Fixed, BcKind::Bearing, BcKind::Pressure]
            } else if plane {
                vec![BcKind::Bearing, BcKind::Fixed, BcKind::Pressure]
            } else if p.supports.is_empty() {
                vec![BcKind::Fixed, BcKind::Pressure]
            } else {
                vec![BcKind::Pressure, BcKind::Fixed]
            }
        }
        _ => {
            if p.supports.is_empty() {
                vec![BcKind::Fixed, BcKind::Roller, BcKind::Pressure]
            } else {
                vec![BcKind::Pressure, BcKind::Fixed, BcKind::Roller]
            }
        }
    }
}

/// Round to 1 or 2 significant figures downward-ish (a "nice" size).
pub(crate) fn nice(v: f64) -> f64 {
    let mag = 10f64.powf(v.log10().floor());
    let m = v / mag;
    let step = if m >= 5.0 { 5.0 } else if m >= 2.0 { 2.0 } else { 1.0 };
    step * mag
}

// ------------------------------------------------------------------------------------------------ mesh density

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RegionKind {
    /// A round hole: a stress concentration.
    Hole,
    /// A re-entrant (concave) outline corner: the stress rises steeply towards it.
    ReentrantCorner,
    /// Where a support acts.
    Support,
    /// Where a load acts.
    Load,
}

impl RegionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hole => "hole",
            Self::ReentrantCorner => "re-entrant corner",
            Self::Support => "support",
            Self::Load => "load",
        }
    }
}

/// How coarse the mesh is against the feature (feature size over element size).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Resolution {
    /// Fewer than 2 elements per feature size: the feature is barely resolved.
    Coarse,
    Medium,
    /// 6 or more.
    Fine,
}

/// A place where a finer mesh is recommended, at `level` (`problem::level_factor`).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub centre: [f64; 2],
    /// Other end of a band along an edge (supports and loads); `None` for a round zone.
    pub to: Option<[f64; 2]>,
    /// Full-level radius (half the band width for a band).
    pub radius: f64,
    /// Width of the blend back to the base size.
    pub blend: f64,
    /// The level the rules recommend first.
    pub level: u8,
    pub kind: RegionKind,
    /// A load or a bushing acts here.
    pub loaded: bool,
    pub resolution: Resolution,
    /// A zone of at least this level already exists here.
    pub applied: bool,
    pub reason: String,
}

/// Levels in the order the rules prefer them for a region of this kind: every level is allowed, the order is advice.
pub fn level_order(kind: RegionKind, loaded: bool) -> [u8; 3] {
    match (kind, loaded) {
        (RegionKind::Hole, true) | (RegionKind::ReentrantCorner, _) => [3, 2, 1],
        (RegionKind::Hole, false) => [2, 3, 1],
        (RegionKind::Support | RegionKind::Load, _) => [1, 2, 3],
    }
}

fn outline_corners(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = pts.len();
    if n < 4 {
        return Vec::new();
    }
    let area2: f64 = (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum();
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let turn = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            turn * area2 < -1e-12 * area2.abs().max(1.0)
        })
        .map(|i| pts[i])
        .collect()
}

/// Where a finer mesh is worth having, with the level the rules recommend. Deterministic and explained; sketch geometry
/// only (an imported mesh brings its own density, and the zones live in the sketch plane).
pub fn refinement_regions(p: &Problem) -> Vec<Region> {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Vec::new() };
    // An empty or unbuilt feature history leaves a placeholder outline: nothing real to refine.
    if p.history.as_ref().is_some_and(|h| h.features.is_empty() || h.rebuild().is_err()) {
        return Vec::new();
    }
    let (lo, hi) = outer.bounds();
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    if !(diag.is_finite() && diag > 0.0 && p.mesh.size > 0.0) {
        return Vec::new();
    }
    let resolution = |feature: f64| match feature / p.mesh.size {
        r if r < 2.0 => Resolution::Coarse,
        r if r < 6.0 => Resolution::Medium,
        _ => Resolution::Fine,
    };
    let applied = |centre: [f64; 2], level: u8| p.mesh.refine.iter().any(|z| dist(z.centre, centre) <= 0.02 * diag && z.level >= level);
    let mut out = Vec::new();
    // Zones are sized to the feature they serve, not to the part: a hole is refined out to 1.25 radii and blends to
    // 2 radii; a corner, a support and a load get a band about 3% of the diagonal wide.
    let mut push = |centre: [f64; 2], to: Option<[f64; 2]>, radius: f64, blend: f64, kind: RegionKind, loaded: bool, feature: f64, reason: String| {
        let level = level_order(kind, loaded)[0];
        out.push(Region { centre, to, radius, blend, level, kind, loaded, resolution: resolution(feature), applied: applied(centre, level), reason });
    };
    for (i, h) in holes.iter().enumerate() {
        let r = h.reach();
        let n = i + 1;
        let loaded = p.loads.iter().any(|l| matches!(l, Load::Bearing { hole, .. } if *hole == n)) || p.bushings.iter().any(|b| b.hole == n);
        let why = if loaded { format!("hole {n} carries a pin or bushing load: the peak stress is at its edge and falls off within about two radii") } else { format!("hole {n} concentrates stress (about 3x the nominal at its edge) over about two radii") };
        push(h.centre(), None, 1.25 * r, 0.75 * r, RegionKind::Hole, loaded, r, why);
    }
    if let Shape::Polygon { pts } = outer {
        for (k, c) in outline_corners(pts).into_iter().enumerate() {
            push(c, None, 0.03 * diag, 0.03 * diag, RegionKind::ReentrantCorner, false, 0.04 * diag, format!("re-entrant corner {}: the stress rises steeply towards a concave corner", k + 1));
        }
    }
    let cents = surface_centroids(p);
    let edges = surface_edges(p);
    let band = 0.03 * diag;
    // A band along the edge it acts on; a surface that is not a straight edge keeps a small round zone.
    let place = |name: &str| -> Option<([f64; 2], Option<[f64; 2]>)> {
        edges.iter().find(|(n, _)| n == name).map(|(_, e)| (e[0], Some(e[1]))).or_else(|| cents.iter().find(|(n, _)| n == name).map(|(_, c)| (*c, None)))
    };
    for s in &p.supports {
        if let Some((c, to)) = edge_of_support(s).and_then(place) {
            push(c, to, band, band, RegionKind::Support, false, 0.06 * diag, "the support reaction enters here; the local stress field needs a few elements".into());
        }
    }
    for l in &p.loads {
        if matches!(l, Load::Bearing { .. }) {
            continue;
        }
        if let Some((c, to)) = edge_of_load(l).as_deref().and_then(place) {
            push(c, to, band, band, RegionKind::Load, true, 0.06 * diag, "the load enters here; the local stress field needs a few elements".into());
        }
    }
    out
}

/// Add (or raise) the refinement zone of `region` at `level`.
pub fn apply_region(p: &mut Problem, region: &Region, level: u8) -> Result<(), String> {
    if !(1..=crate::problem::MAX_LEVEL).contains(&level) {
        return Err(format!("the level is 1 to {}", crate::problem::MAX_LEVEL));
    }
    let near = |z: &crate::problem::RefineRegion| dist(z.centre, region.centre) <= 0.25 * region.radius;
    match p.mesh.refine.iter_mut().find(|z| near(z)) {
        Some(z) => {
            z.level = level;
            z.radius = region.radius;
            z.to = region.to;
            z.blend = Some(region.blend);
        }
        None => {
            if p.mesh.refine.len() >= crate::problem::MAX_REFINE {
                return Err(format!("at most {} refinement zones: remove one first", crate::problem::MAX_REFINE));
            }
            p.mesh.refine.push(crate::problem::RefineRegion { centre: region.centre, radius: region.radius, level, to: region.to, blend: Some(region.blend) });
        }
    }
    Ok(())
}

/// Most zones one solve may add.
const MAX_ERROR_ZONES: usize = 3;

/// Refinement zones the ZZ error estimate of `solved` asks for: clusters of the elements that carry half of the error,
/// where the relative error is above the problem's target. A cluster that an existing zone already covers is skipped,
/// so applying the result twice adds nothing. Sketch profiles only (the zones live in the sketch plane); the level is 1
/// while the error is within twice the target and 2 beyond.
pub fn error_zones(solved: &crate::solve::Solved) -> Vec<crate::problem::RefineRegion> {
    let p = &solved.problem;
    let rel = solved.history.last().map_or(f64::NAN, |h| h.zz_error);
    if !matches!(p.geometry, Geometry::Sketch { .. }) || p.analysis == crate::problem::Analysis::Solid || !(rel.is_finite() && rel > p.mesh.target_error) {
        return Vec::new();
    }
    let mesh = &solved.model.mesh;
    // Centroid, size (twice the farthest node from the centroid) and error of every element.
    let mut els: Vec<([f64; 2], f64, f64)> = Vec::with_capacity(solved.zz_eta.len());
    for b in &mesh.blocks {
        for e in 0..b.n_elems() {
            let nodes = b.elem(e);
            let c = nodes.iter().fold([0.0; 2], |a, &n| [a[0] + mesh.nodes[n][0], a[1] + mesh.nodes[n][1]]);
            let c = [c[0] / nodes.len() as f64, c[1] / nodes.len() as f64];
            let h = 2.0 * nodes.iter().map(|&n| dist(c, [mesh.nodes[n][0], mesh.nodes[n][1]])).fold(0.0, f64::max);
            els.push((c, h, 0.0));
        }
    }
    if els.len() != solved.zz_eta.len() {
        return Vec::new();
    }
    for (e, eta) in els.iter_mut().zip(&solved.zz_eta) {
        e.2 = *eta;
    }
    let total2: f64 = els.iter().map(|e| e.2 * e.2).sum();
    if !(total2.is_finite() && total2 > 0.0) {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..els.len()).collect();
    order.sort_by(|&a, &b| els[b].2.total_cmp(&els[a].2));
    let (mut acc, mut hot) = (0.0, Vec::new());
    for i in order {
        hot.push(i);
        acc += els[i].2 * els[i].2;
        if acc >= 0.5 * total2 {
            break;
        }
    }
    let level = if rel <= 2.0 * p.mesh.target_error { 1 } else { 2 };
    let mut zones: Vec<crate::problem::RefineRegion> = Vec::new();
    let mut taken = vec![false; els.len()];
    for &seed in &hot {
        if taken[seed] || zones.len() >= MAX_ERROR_ZONES {
            continue;
        }
        let reach = 3.0 * els[seed].1;
        let members: Vec<usize> = hot.iter().copied().filter(|&i| !taken[i] && dist(els[i].0, els[seed].0) <= reach).collect();
        let w: f64 = members.iter().map(|&i| els[i].2 * els[i].2).sum();
        let c = members.iter().fold([0.0; 2], |a, &i| [a[0] + els[i].0[0] * els[i].2 * els[i].2 / w, a[1] + els[i].0[1] * els[i].2 * els[i].2 / w]);
        let radius = members.iter().map(|&i| dist(c, els[i].0) + els[i].1).fold(2.0 * els[seed].1, f64::max);
        for &i in &members {
            taken[i] = true;
        }
        let covered = p.mesh.refine.iter().chain(zones.iter()).any(|z| z.level >= level && z.distance(c) <= z.radius);
        if !covered {
            zones.push(crate::problem::RefineRegion { centre: c, radius, level, to: None, blend: None });
        }
    }
    zones
}

/// Add `zones` to the problem's refinement list (up to the limit); the number actually added.
pub fn add_zones(p: &mut Problem, zones: &[crate::problem::RefineRegion]) -> usize {
    let room = crate::problem::MAX_REFINE.saturating_sub(p.mesh.refine.len());
    let n = zones.len().min(room);
    p.mesh.refine.extend_from_slice(&zones[..n]);
    n
}
