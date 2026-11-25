//! Advice over the assembly features `solid_kernel::features::detect` finds (epic `docs/epic-assembly-feature-advice.md`, Phase B).
//!
//! Read-only: each item names the faces it concerns by `FaceKey` (body index, STEP face id; never a persisted kernel id) and carries
//! a reason string with the measured numbers. Nothing here edits a `Problem`: a `Problem` still holds one body, so there is nothing
//! to apply a tie or a fastener to until the multi-body representation (and the solver exposure of epic Phase C) exists. Body numbers
//! in the text are 1-based (the order of the STEP product structure); `FaceKey::body` is the 0-based index.
use solid_kernel::features::{size_matches, FaceKey, FastenerSite, FitClass, Features, Hole, Interface, NearClosedBore, SizeMatch};
use solid_kernel::free_edge::{EdgeKind, FreeEdge, FreeEdgeError};
/// Re-exported so a front end names a hole's closure without a direct `solid-kernel` dependency.
pub use solid_kernel::features::Closure;

#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyAdvice {
    /// Two planar faces of different bodies that touch: bond (tie) them, or model contact between them.
    Tie { a: FaceKey, b: FaceKey, gap: f64, overlap_area: f64, reason: String },
    /// Coaxial bores of different bodies: one fastener passes through them.
    Fastener { bodies: Vec<usize>, faces: Vec<FaceKey>, size: Option<&'static str>, mismatch: bool, diameters: Vec<f64>, span: f64, reason: String },
    /// A bolt-sized hole (its diameter is a standard fastener size) with no mating bore on another body: a place for a support or a
    /// load. `size` names the standard sizes the diameter matches, "M5 or M6" when more than one does (5.0 mm is the M5 nominal and the M6 tap drill; the reason gives each match with its fit); `closure` is never
    /// `Enclosed` (an internal cavity is not a hole to bolt through).
    Unmated { body: usize, faces: Vec<FaceKey>, diameter: f64, closure: Closure, size: String, reason: String },
    /// A bore that covers nearly, not quite, a full turn: not a hole (the full-turn rule stands) but a candidate for a modelling gap.
    /// `covered` is the fraction of a turn; `gap_point` is on the bore wall in the middle of the widest gap (millimetres, in the
    /// assembly's frame) and `gap_width` the arc length of that gap at the wall. The advice repairs nothing.
    NearClosed { body: usize, faces: Vec<FaceKey>, diameter: f64, covered: f64, gap_width: f64, gap_point: [f64; 3], reason: String },
    /// Faces that looked like an interface but whose common area could not be computed: check them by hand.
    Unresolved { a: FaceKey, b: FaceKey, reason: String },
}

impl AssemblyAdvice {
    pub fn label(&self) -> String {
        match self {
            AssemblyAdvice::Tie { a, b, .. } => format!("Tie body {} face {} to body {} face {}", a.body + 1, a.face, b.body + 1, b.face),
            AssemblyAdvice::Fastener { bodies, size, .. } => {
                let list = bodies.iter().map(|b| (b + 1).to_string()).collect::<Vec<_>>().join(", ");
                match size {
                    Some(s) => format!("Treat the coaxial holes of bodies {list} as one {s} fastener"),
                    None => format!("Treat the coaxial holes of bodies {list} as one fastener"),
                }
            }
            AssemblyAdvice::Unmated { body, diameter, size, .. } => format!("Body {} hole \u{d8}{diameter:.3} mm ({size}) has no mate", body + 1),
            AssemblyAdvice::NearClosed { body, diameter, covered, .. } => format!("Body {} bore \u{d8}{diameter:.3} mm is {:.1} % closed: possible modelling gap", body + 1, covered * 100.0),
            AssemblyAdvice::Unresolved { a, b, .. } => format!("Check body {} face {} against body {} face {}", a.body + 1, a.face, b.body + 1, b.face),
        }
    }
    /// The measured numbers in one short line (the advice list shows it under the label).
    pub fn detail(&self) -> String {
        match self {
            AssemblyAdvice::Tie { gap, overlap_area, .. } => {
                let gap = if gap.abs() < 5e-7 { "no gap".to_string() } else if *gap > 0.0 { format!("{gap:.3} mm gap") } else { format!("{:.3} mm interpenetration", -gap) };
                format!("{gap} \u{b7} {overlap_area:.0} mm\u{b2}")
            }
            AssemblyAdvice::Fastener { diameters, span, .. } => {
                let d = diameters.iter().map(|d| format!("\u{d8}{d:.3}")).collect::<Vec<_>>().join(", ");
                format!("{d} mm \u{b7} {span:.2} mm")
            }
            AssemblyAdvice::Unmated { diameter, closure, .. } => format!("\u{d8}{diameter:.3} mm \u{b7} {}", closure_word(*closure)),
            AssemblyAdvice::NearClosed { diameter, covered, gap_width, .. } => format!("\u{d8}{diameter:.3} mm \u{b7} {:.1} % of a turn \u{b7} {gap_width:.3} mm gap", covered * 100.0),
            AssemblyAdvice::Unresolved { .. } => "common area not computed".to_string(),
        }
    }
    pub fn reason(&self) -> &str {
        match self {
            AssemblyAdvice::Tie { reason, .. } | AssemblyAdvice::Fastener { reason, .. } | AssemblyAdvice::Unmated { reason, .. } | AssemblyAdvice::NearClosed { reason, .. } | AssemblyAdvice::Unresolved { reason, .. } => reason,
        }
    }
    /// The faces to highlight when the item is selected.
    pub fn faces(&self) -> Vec<FaceKey> {
        match self {
            AssemblyAdvice::Tie { a, b, .. } | AssemblyAdvice::Unresolved { a, b, .. } => vec![*a, *b],
            AssemblyAdvice::Fastener { faces, .. } | AssemblyAdvice::Unmated { faces, .. } | AssemblyAdvice::NearClosed { faces, .. } => faces.clone(),
        }
    }
}

fn fit_word(f: FitClass) -> &'static str {
    match f {
        FitClass::Tap => "tap drill",
        FitClass::Nominal => "nominal (press / locating)",
        FitClass::Close => "close clearance",
        FitClass::Normal => "normal clearance",
        FitClass::Loose => "loose clearance",
    }
}

fn closure_word(c: Closure) -> &'static str {
    match c {
        Closure::Through => "through",
        Closure::Blind => "blind",
        Closure::Enclosed => "enclosed",
    }
}

fn matches_text(m: &[SizeMatch]) -> String {
    if m.is_empty() {
        "no standard size".to_string()
    } else {
        m.iter().map(|x| format!("{} {}", x.size, fit_word(x.fit))).collect::<Vec<_>>().join(" or ")
    }
}

/// The geometric nearest-free-edge reading of one hole as a clause (no strength statement: Assembly Joints has no material or load,
/// `docs/edge-distance-crosscheck.md`). The distance is from the hole axis in the plane of the plate face, so the ligament is the
/// distance less the radius; `e/D` is the figure the edge-distance rules quote.
fn edge_clause(h: &Hole, e: Option<&Result<FreeEdge, FreeEdgeError>>) -> String {
    match e {
        Some(Ok(e)) => {
            let kind = match e.kind {
                EdgeKind::Line => "a straight edge".to_string(),
                EdgeKind::Arc { radius } => format!("a circular edge of radius {radius:.2} mm"),
            };
            format!("nearest free edge {:.2} mm from the axis (e/D {:.2}), {kind} of a {:.2} mm plate", e.distance, e.distance / h.diameter(), e.thickness)
        }
        Some(Err(why)) => format!("no free edge distance read: {why}"),
        None => "no free edge distance read".to_string(),
    }
}

fn tie(i: &Interface) -> AssemblyAdvice {
    let gap = if i.gap.abs() < 5e-7 { "no gap".to_string() } else if i.gap > 0.0 { format!("a {:.4} mm gap", i.gap) } else { format!("{:.4} mm of interpenetration", -i.gap) };
    AssemblyAdvice::Tie {
        a: i.a,
        b: i.b,
        gap: i.gap,
        overlap_area: i.overlap_area,
        reason: format!(
            "Body {} face {} and body {} face {} face each other with {gap} and share {:.2} mm\u{b2}: a bonded (tie) interface is the cheap model of a bolted or welded joint there; contact is the one that lets them separate.",
            i.a.body + 1,
            i.a.face,
            i.b.body + 1,
            i.b.face,
            i.overlap_area
        ),
    }
}

fn fastener(f: &Features, s: &FastenerSite) -> AssemblyAdvice {
    let mut faces = Vec::new();
    let mut bores = Vec::new();
    for b in &s.bores {
        let h = &f.holes[b.hole];
        faces.extend(h.faces.iter().map(|&face| FaceKey { body: h.body, face }));
        bores.push(format!("body {} \u{d8}{:.3} mm ({})", h.body + 1, b.diameter, matches_text(&b.matches)));
    }
    let mut reason = format!("{} coaxial bores over {:.2} mm of axis: {}.", s.bores.len(), s.span[1] - s.span[0], bores.join("; "));
    let edges: Vec<String> = s.bores.iter().map(|b| format!("body {}: {}", f.holes[b.hole].body + 1, edge_clause(&f.holes[b.hole], f.free_edges.get(b.hole)))).collect();
    reason.push_str(&format!(" Edge distance, geometry only ({}).", edges.join("; ")));
    match s.fastener {
        Some(size) => reason.push_str(&format!(" Every bore fits {size}, so one {size} fastener can pass through them.")),
        None => reason.push_str(" No single standard size fits every bore."),
    }
    if !s.bridged_by.is_empty() {
        let pins = s.bridged_by.iter().map(|b| (b + 1).to_string()).collect::<Vec<_>>().join(", ");
        reason.push_str(&format!(" The bores do not touch (a spacer, washer or air gap lies between them); a pin or bolt of body {pins} runs through that gap and joins them."));
    }
    if s.diameter_mismatch {
        reason.push_str(" The bore diameters differ (a clearance hole in one part and a tapped or smaller hole in another is normal for a bolted joint; a mismatch with no common size is not).");
    }
    let diameters = s.bores.iter().map(|b| b.diameter).collect();
    AssemblyAdvice::Fastener { bodies: s.bodies.clone(), faces, size: s.fastener, mismatch: s.diameter_mismatch, diameters, span: s.span[1] - s.span[0], reason }
}

/// Two holes of one body are one stepped bore (a counterbore) when they share an axis and their axial ranges touch; the distances
/// are the strict read's geometry tolerance (`DEFAULT_CHORD / 10`, `Options::default`), since `Features` does not carry the models'.
const LINK_TOL_MM: f64 = crate::assembly_mesh::DEFAULT_CHORD / 10.0;

fn stepped(a: &Hole, b: &Hole) -> bool {
    let dot = |p: [f64; 3], q: [f64; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let cross = |p: [f64; 3], q: [f64; 3]| [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]];
    let sub = |p: [f64; 3], q: [f64; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let length = a.depth().max(b.depth()).max(a.radius).max(b.radius);
    let c = cross(a.axis, b.axis);
    if dot(c, c).sqrt() > LINK_TOL_MM / length.max(LINK_TOL_MM) {
        return false;
    }
    let d = sub(b.axis_point, a.axis_point);
    let along = dot(d, a.axis);
    let perp = sub(d, [a.axis[0] * along, a.axis[1] * along, a.axis[2] * along]);
    if dot(perp, perp).sqrt() > LINK_TOL_MM {
        return false;
    }
    let k = dot(b.axis, a.axis);
    let (e0, e1) = (along + b.range[0] * k, along + b.range[1] * k);
    a.range[0].max(e0.min(e1)) - a.range[1].min(e0.max(e1)) <= LINK_TOL_MM
}

/// The holes that belong to no fastener site and match a standard size, on bodies that have another body to mate with: a hole in a
/// site, or a section of a stepped bore (counterbore) one of whose sections is in a site, is mated. Enclosed cavities are skipped.
fn unmated(f: &Features) -> Vec<AssemblyAdvice> {
    let bodies = f.faces.iter().map(|c| c.key.body + 1).max().unwrap_or(0);
    if bodies < 2 {
        return Vec::new();
    }
    let mut mated = vec![false; f.holes.len()];
    for b in f.sites.iter().flat_map(|s| &s.bores) {
        mated[b.hole] = true;
    }
    // a stepped bore is mated as a whole: spread along touching coaxial sections of the same body until nothing changes
    loop {
        let mut changed = false;
        for i in 0..f.holes.len() {
            if !mated[i] && (0..f.holes.len()).any(|j| mated[j] && f.holes[j].body == f.holes[i].body && stepped(&f.holes[i], &f.holes[j])) {
                mated[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    f.holes
        .iter()
        .enumerate()
        .zip(&mated)
        .filter(|((_, h), m)| !**m && h.closure() != Closure::Enclosed)
        .filter_map(|((i, h), _)| {
            let matches = size_matches(h.diameter());
            let mut sizes: Vec<&str> = matches.iter().map(|m| m.size).collect();
            sizes.dedup();
            if sizes.is_empty() {
                return None;
            }
            let size = sizes.join(" or ");
            let closure = h.closure();
            let reason = format!(
                "Body {} has a \u{d8}{:.3} mm {} hole {:.2} mm deep ({}) with no coaxial bore on another body: it is not a fastener site, so it is a place for a support or a load. Edge distance, geometry only: {}.",
                h.body + 1,
                h.diameter(),
                closure_word(closure),
                h.depth(),
                matches_text(&matches),
                edge_clause(h, f.free_edges.get(i))
            );
            Some(AssemblyAdvice::Unmated { body: h.body, faces: h.faces.iter().map(|&face| FaceKey { body: h.body, face }).collect(), diameter: h.diameter(), closure, size, reason })
        })
        .collect()
}

/// A near-closed bore as advice: where the gap is, how wide, and that it is a candidate to look at, not a hole.
fn near_closed(n: &NearClosedBore) -> AssemblyAdvice {
    let h = &n.hole;
    let p = n.gap_point;
    let reason = format!(
        "Body {} has a \u{d8}{:.3} mm bore {:.2} mm deep whose faces cover {:.1} % of a turn: it is not a hole, because only a bore that wraps the full turn is one. The missing {:.3} mm of wall (arc length) is at ({:.2}, {:.2}, {:.2}) mm, halfway along the bore. It may be a designed slit (a split collar or clamp) or a gap left in the model: if it is a gap, close it in the CAD model and open the file again, and the bore counts as a hole and can pair into a fastener site.",
        h.body + 1,
        h.diameter(),
        h.depth(),
        n.covered * 100.0,
        n.gap_width,
        p[0],
        p[1],
        p[2]
    );
    AssemblyAdvice::NearClosed { body: h.body, faces: h.faces.iter().map(|&face| FaceKey { body: h.body, face }).collect(), diameter: h.diameter(), covered: n.covered, gap_width: n.gap_width, gap_point: p, reason }
}

/// The advice for detected features, in the detector's order: interfaces, fastener sites, holes without a mate, near-closed bores,
/// then unresolved pairs.
pub fn recommend(f: &Features) -> Vec<AssemblyAdvice> {
    let unresolved = f.unresolved.iter().map(|(a, b)| AssemblyAdvice::Unresolved {
        a: *a,
        b: *b,
        reason: format!("Body {} face {} and body {} face {} look like a contact (anti-parallel, within the gap tolerance) but the area they share could not be computed.", a.body + 1, a.face, b.body + 1, b.face),
    });
    f.interfaces.iter().map(tie).chain(f.sites.iter().map(|s| fastener(f, s))).chain(unmated(f)).chain(f.near_closed.iter().map(near_closed)).chain(unresolved).collect()
}

/// Read every body of a STEP assembly the way the solid editor does (strict, then the tolerant foreign-CAD read) and detect its
/// features.
pub fn detect_step(step: &str) -> Result<Features, String> {
    crate::assembly_mesh::with_bodies(step, crate::assembly_mesh::DEFAULT_CHORD, |models, _| solid_kernel::features::detect(models).map_err(|e| e.to_string()))
}

/// [`detect_step`] and [`recommend`].
pub fn recommend_step(step: &str) -> Result<Vec<AssemblyAdvice>, String> {
    Ok(recommend(&detect_step(step)?))
}
