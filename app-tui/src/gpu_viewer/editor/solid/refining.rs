//! Mesh-density hints: where `fea_problem::advice::refinement_regions` recommends a finer mesh, drawn as rings in the
//! viewport (thicker = finer level) and listed in the property panel with the recommended level. Clicking applies a
//! level as a `MeshSpec::refine` zone (one undo step). The rules decide where and what is allowed; a ranker
//! (`editor::rank`, gated like the others) may only reorder the three levels offered for a region, from what the user
//! chose in the same kind of situation. Recording is content-free: the situation is the analysis class, the kind of
//! region, whether it is loaded and how coarse the mesh is against the feature.
use super::advising::AnalysisClass;
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        scene::Builder,
        session::{EditorAction, Tone},
    },
    text::wrap,
    ui::DockItem,
};
use fea_problem::{
    advice::{self, Region, RegionKind, Resolution},
    problem::level_factor,
    Analysis, RefineRegion,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RefineLevel {
    One,
    Two,
    Three,
}
impl RefineLevel {
    pub const ALL: [RefineLevel; 3] = [Self::One, Self::Two, Self::Three];
    pub fn number(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }
    pub fn of(n: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.number() == n)
    }
}

/// Coarse, content-free state the level ranker learns from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RefineSituation {
    pub analysis: AnalysisClass,
    pub kind: RegionKind,
    pub loaded: bool,
    pub resolution: Resolution,
}

pub(super) fn refine_situation(p: &Problem, r: &Region) -> RefineSituation {
    RefineSituation {
        analysis: match p.analysis {
            Analysis::PlaneStress | Analysis::PlaneStrain => AnalysisClass::Plane,
            Analysis::Axisymmetric => AnalysisClass::Axisymmetric,
            Analysis::Solid => AnalysisClass::Solid,
        },
        kind: r.kind,
        loaded: r.loaded,
        resolution: r.resolution,
    }
}

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

impl SolidSession {
    /// The levels for `region` in the order to show them (the rules', or the learned one when enabled, gated and sure).
    fn level_choices(&self, region: &Region) -> Vec<RefineLevel> {
        let rules: Vec<RefineLevel> = advice::level_order(region.kind, region.loaded).into_iter().filter_map(RefineLevel::of).collect();
        self.advisors.refinement.order(&refine_situation(&self.problem, region), &rules).0
    }

    /// Apply `level` (0 removes the zone) to region `index` of the current recommendations.
    pub(super) fn do_refine(&mut self, index: usize, level: u8) {
        let regions = advice::refinement_regions(&self.problem);
        let Some(region) = regions.get(index) else {
            self.status = "That region no longer applies.".into();
            return;
        };
        let mut next = self.problem.clone();
        let result = if level == 0 {
            let before = next.mesh.refine.len();
            next.mesh.refine.retain(|z| (z.centre[0] - region.centre[0]).hypot(z.centre[1] - region.centre[1]) > 0.25 * region.radius);
            if next.mesh.refine.len() == before { Err("there is no refinement zone here".to_string()) } else { Ok(()) }
        } else {
            advice::apply_region(&mut next, region, level)
        };
        match result {
            Ok(()) => {
                let (situation, offered) = (refine_situation(&self.problem, region), self.level_choices(region));
                self.edit(|p| *p = next);
                if let (true, Some(taken)) = (level > 0, RefineLevel::of(level)) {
                    if let Ok(Some(fit)) = self.advisors.refinement.record(situation, offered, Some(taken)) {
                        self.advisors.last_fit = Some(fit);
                    }
                }
                self.status = if level == 0 {
                    format!("Removed the refinement near the {} (undoable).", region.kind.label())
                } else {
                    format!("Refined the mesh near the {} at level {level} (elements x{}); generate the mesh again (undoable).", region.kind.label(), level_factor(level))
                };
            }
            Err(e) => self.status = e,
        }
    }

    /// Why "refine where recommended" cannot act now.
    pub fn refine_availability(&self) -> Result<(), String> {
        if self.sketching.is_some() {
            return Err("finish the sketch first".into());
        }
        self.problem.validate_geometry()?;
        if advice::refinement_regions(&self.problem).iter().all(|r| r.applied) {
            return Err("every recommended region is already refined".into());
        }
        Ok(())
    }

    pub(super) fn refine_all(&mut self) {
        let regions = advice::refinement_regions(&self.problem);
        let mut next = self.problem.clone();
        let mut n = 0;
        for r in regions.iter().filter(|r| !r.applied) {
            let level = self.level_choices(r).first().map_or(r.level, |l| l.number());
            if advice::apply_region(&mut next, r, level).is_ok() {
                n += 1;
            }
        }
        if n == 0 {
            self.status = "Nothing left to refine.".into();
            return;
        }
        self.edit(|p| *p = next);
        self.status = format!("Added {n} refinement zone(s) at the recommended levels; generate the mesh again (undoable).");
    }

    pub(super) fn clear_refine(&mut self) {
        if self.problem.mesh.refine.is_empty() {
            self.status = "There are no refinement zones.".into();
            return;
        }
        self.edit(|p| p.mesh.refine.clear());
        self.status = "Removed every refinement zone (undoable).".into();
    }

    /// The panel section: one block per region, then the legend.
    pub(super) fn refine_panel(&self) -> Vec<DockItem> {
        if !self.advisors.show || self.sketching.is_some() || self.problem.validate_geometry().is_err() {
            return Vec::new();
        }
        let regions = advice::refinement_regions(&self.problem);
        if regions.is_empty() && self.problem.mesh.refine.is_empty() {
            return Vec::new();
        }
        let mut out = vec![DockItem::Gap, DockItem::Heading("Mesh density".into())];
        let factor = |l: u8| format!("x{}", level_factor(l));
        let estimate = fea_problem::build::estimate_mesh(&self.problem).ok();
        let count_of = |zone: usize| estimate.as_ref().and_then(|e| e.per_zone.get(zone)).map_or(String::new(), |n| format!(", about {} elements", thousands(*n)));
        for (i, r) in regions.iter().enumerate() {
            let z = self.problem.mesh.refine.iter().position(|z| (z.centre[0] - r.centre[0]).hypot(z.centre[1] - r.centre[1]) <= 0.25 * r.radius);
            match z.map(|k| (k, &self.problem.mesh.refine[k])) {
                Some((k, z)) => out.push(DockItem::row(format!("{}: level {} ({}){}, click to remove", r.kind.label(), z.level, factor(z.level), count_of(k)), Tone::On, Some(ui(SolidAction::Refine(i, 0))), false)),
                None => {
                    let choices = self.level_choices(r);
                    let first = choices[0].number();
                    out.push(DockItem::row(format!("Refine {} at level {first} ({})", r.kind.label(), factor(first)), Tone::On, Some(ui(SolidAction::Refine(i, first))), false));
                    for l in &choices[1..] {
                        out.push(DockItem::row(format!("    or level {} ({})", l.number(), factor(l.number())), Tone::Dim, Some(ui(SolidAction::Refine(i, l.number()))), false));
                    }
                    for l in wrap(&r.reason, 38) {
                        out.push(DockItem::text(format!("  {l}"), Tone::Dim));
                    }
                }
            }
        }
        if regions.iter().any(|r| !r.applied) {
            out.push(DockItem::row("Refine everywhere recommended", Tone::On, Some(ui(SolidAction::RefineAll)), false));
        }
        if !self.problem.mesh.refine.is_empty() {
            out.push(DockItem::row(format!("Remove all {} zone(s)", self.problem.mesh.refine.len()), Tone::Warn, Some(ui(SolidAction::ClearRefine)), false));
        }
        if !self.error_zones.is_empty() {
            out.push(DockItem::text(format!("{} zone(s) from the last solve's error estimate are added at the next mesh or solve", self.error_zones.len()), Tone::Warn));
        }
        if let Some(e) = &estimate {
            out.push(DockItem::text(format!("budget: about {} elements; {:.0}% of the part at full refinement, {:.0}% touched", thousands(e.elements), 100.0 * e.core_share, 100.0 * e.touched_share), Tone::Normal));
        }
        out.push(DockItem::text("outlines in the viewport: thicker = finer", Tone::Dim));
        out.push(DockItem::text(format!("levels 1/2/3 = element size x{}/{}/{}", level_factor(1), level_factor(2), level_factor(3)), Tone::Dim));
        out
    }

    /// Outlines of every recommended, applied and error-estimate zone in model coordinates: `(from, to, level)`
    /// segments. A round zone is a ring; a band along an edge is a capsule.
    pub(super) fn refine_rings(&self) -> Vec<([f64; 3], [f64; 3], u8)> {
        if !self.show_refine || self.problem.validate_geometry().is_err() {
            return Vec::new();
        }
        let frame = self.problem.history.as_ref().and_then(|h| h.rebuild().ok()).map(|b| b.frame).unwrap_or(fea_problem::history::Frame { origin: [0.0; 3], u: [1.0, 0.0, 0.0], v: [0.0, 1.0, 0.0] });
        let mut zones: Vec<RefineRegion> = self.problem.mesh.refine.iter().chain(self.error_zones.iter()).cloned().collect();
        zones.extend(advice::refinement_regions(&self.problem).into_iter().filter(|r| !r.applied).map(|r| RefineRegion { centre: r.centre, radius: r.radius, level: r.level, to: r.to, blend: Some(r.blend) }));
        let mut out = Vec::new();
        for z in zones {
            let pts = zone_outline(&z);
            let n = pts.len();
            out.extend((0..n).map(|k| (frame.apply([pts[k][0], pts[k][1], 0.0]), frame.apply([pts[(k + 1) % n][0], pts[(k + 1) % n][1], 0.0]), z.level)));
        }
        out
    }

    /// Add the rings to a scene already normalised by `to_scene` (model point -> scene point); `code` maps a level to
    /// the scene's scalar code.
    pub(super) fn add_rings(&self, scene: &mut Scene, to_scene: impl Fn([f64; 3]) -> [f32; 3], code: impl Fn(u8) -> f32) {
        let rings = self.refine_rings();
        if rings.is_empty() {
            return;
        }
        let mut b = Builder::new();
        for (p, q, level) in rings {
            b.tube(to_scene(p), to_scene(q), 0.002 + 0.0018 * f32::from(level), [None, None], code(level));
        }
        let base = scene.positions.len() as u32;
        scene.positions.extend(b.positions.iter().copied());
        scene.triangles.extend(b.triangles.iter().map(|i| i + base));
        scene.samples.extend(b.value.iter().map(|v| [0.0, 0.0, 0.0, *v]));
    }
}

const RING_SEGMENTS: usize = 48;

/// Closed polyline of a zone's full-level boundary: a circle, or a capsule around the segment of a band.
pub(super) fn zone_outline(z: &RefineRegion) -> Vec<[f64; 2]> {
    let Some(b) = z.to else {
        return (0..RING_SEGMENTS).map(|k| {
            let t = std::f64::consts::TAU * k as f64 / RING_SEGMENTS as f64;
            [z.centre[0] + z.radius * t.cos(), z.centre[1] + z.radius * t.sin()]
        }).collect();
    };
    let a = z.centre;
    let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
    let half = RING_SEGMENTS / 2;
    let arc = |c: [f64; 2], from: f64| (0..=half).map(move |k| {
        let t = from + std::f64::consts::PI * k as f64 / half as f64;
        [c[0] + z.radius * t.cos(), c[1] + z.radius * t.sin()]
    });
    // A half circle round the end `b` (from -90 to +90 degrees of the band direction), then one round `a`.
    arc(b, ang - std::f64::consts::FRAC_PI_2).chain(arc(a, ang + std::f64::consts::FRAC_PI_2)).collect()
}

/// Scalar code of a ring: between the state codes (0 body, 1 support, 2 load, 3 selected), so no ring is ever
/// mistaken for a support, a load or a selection; the level shows in the ring's thickness.
pub(super) fn ring_code(level: u8) -> f32 {
    f32::from(level.clamp(1, 3)) - 0.5
}

/// The map `Scene::from_frames` applies to a mesh's nodes (centre the bounding box, divide by its largest extent).
pub(super) fn mesh_to_scene(mesh: &fea_core::Mesh) -> impl Fn([f64; 3]) -> [f32; 3] {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &mesh.nodes {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let span = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max).max(1e-300);
    move |p| std::array::from_fn(|a| ((p[a] - lo[a] - (hi[a] - lo[a]) / 2.0) / span) as f32)
}

/// `12345.6` as `12,346`.
pub(super) fn thousands(n: f64) -> String {
    let digits = format!("{:.0}", n.max(0.0));
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
