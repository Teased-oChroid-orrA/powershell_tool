//! Parametric modelling in the solid editor: the feature history (sketch on a plane -> extrude / revolve) and the
//! Sketch mode that edits one sketch of it. The interaction rules live in `editor::sketcher` (pure); this file maps
//! pixels to the sketch plane, records undo, builds the sketch scene and supplies the tree, property, ribbon and menu
//! content. Design: `docs/adr/ADR-015-parametric-sketch-history.md`.
use super::*;
use crate::gpu_viewer::{
    controller::{StandardView, UiAction},
    editor::{
        authoring::{Overlay, OverlayKind},
        camera::pick_radius,
        scene::{Builder, CODE_FRAME, CODE_LOAD, CODE_SELECTED, CODE_SUPPORT, CODE_TRUSS},
        session::{EditorAction, Tone},
        sketcher::{self, DimKind, SketchEdit, Tool},
    },
    menu::{Entry, Item},
    ui::{DockItem, RibbonTab, TreeNode},
};
use fea_problem::{
    history::{Feature as HFeature, FeatureKind, History, Plane},
    sketch::{infer::Rule, solve::Status, Constraint, Id, Sketch},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneChoice {
    Front,
    Top,
    Right,
}
impl PlaneChoice {
    pub const ALL: [PlaneChoice; 3] = [PlaneChoice::Front, PlaneChoice::Top, PlaneChoice::Right];
    fn plane(self) -> Plane {
        match self {
            Self::Front => Plane::Front { offset: 0.0 },
            Self::Top => Plane::Top { offset: 0.0 },
            Self::Right => Plane::Right { offset: 0.0 },
        }
    }
    fn label(self) -> &'static str {
        self.plane().label()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureParam {
    Depth,
    Layers,
    Angle,
    Segments,
    PlaneOffset,
}
impl FeatureParam {
    fn label(self) -> &'static str {
        match self {
            Self::Depth => "depth",
            Self::Layers => "layers",
            Self::Angle => "angle (degrees)",
            Self::Segments => "segments",
            Self::PlaneOffset => "plane offset",
        }
    }
}

/// What the Sketch / feature commands of the ribbon, menus, tree and property panel ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SketchAction {
    NewSketch(PlaneChoice),
    /// A sketch on a flat face of the built body: the index into `History::planar_faces`.
    NewSketchOnFace(usize),
    /// Open the sketch feature for editing.
    Edit(Id),
    Finish,
    /// Finish a sketch that is an open chain of lines and arcs, to use as a sweep path.
    FinishPath,
    /// Leave the sketch and undo everything done since the command that opened it.
    Cancel,
    Tool(Tool),
    Constrain(Rule),
    Accept(usize),
    DeleteSelected,
    RemoveConstraint(Id),
    /// Start typing the dimension of the selection.
    Dimension,
    /// Pin the selected points where they are.
    Fix,
    ToggleGrid,
    Convert,
    AddExtrude,
    AddRevolve,
    /// Sweep a profile sketch along a path sketch (the selected sketch is the profile, else the first).
    AddSweep,
    SelectFeature(Id),
    Suppress(Id),
    /// Roll the model back to just after this feature, or forward to the end (`None`).
    RollbackTo(Option<Id>),
    RemoveFeature(Id),
    EditParam(Id, FeatureParam),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SketchView {
    pub(super) center: [f64; 2],
    pub(super) span: f64,
}

pub(super) struct Sketching {
    pub(super) feature: Id,
    pub(super) edit: SketchEdit,
    pub(super) view: SketchView,
    pub(super) grid_on: bool,
    /// Problem before a point drag started (one undo step per drag).
    drag_before: Option<Problem>,
    /// The problem as it was before the command that opened the sketch (`Cancel` returns to it).
    entry: Problem,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TypeTarget {
    Dimension,
    Param(Id, FeatureParam),
}
pub(super) struct Typing {
    target: TypeTarget,
    text: String,
    /// The text is a suggestion: the first key typed replaces it (as in a selected entry box).
    fresh: bool,
}

fn ui(a: SketchAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(SolidAction::Sketch(a)))
}
fn entry(label: &str, shortcut: &str, a: SketchAction) -> Entry {
    Entry::new(label, shortcut, ui(a))
}

impl SolidSession {
    // ------------------------------------------------------------------ queries

    pub fn sketch_mode(&self) -> bool {
        self.sketching.is_some()
    }
    pub(super) fn typing_active(&self) -> bool {
        self.typing.is_some()
    }
    pub(super) fn history(&self) -> Option<&History> {
        self.problem.history.as_ref()
    }
    fn feature(&self, id: Id) -> Option<&HFeature> {
        self.history()?.feature(id)
    }
    fn active_sketch(&self) -> Option<(&Plane, &Sketch)> {
        let s = self.sketching.as_ref()?;
        match &self.feature(s.feature)?.kind {
            FeatureKind::Sketch { plane, sketch } => Some((plane, sketch)),
            _ => None,
        }
    }
    /// Sketch feature the next Extrude / Revolve would use: the selected one, else the last in the history.
    fn profile_sketch(&self) -> Option<Id> {
        let h = self.history()?;
        let is_sketch = |id: Id| matches!(h.feature(id).map(|f| &f.kind), Some(FeatureKind::Sketch { .. }));
        self.selected_feature.filter(|id| is_sketch(*id)).or_else(|| h.features[..h.active_len()].iter().rev().find(|f| matches!(f.kind, FeatureKind::Sketch { .. })).map(|f| f.id))
    }
    pub(super) fn body_feature(&self) -> Option<&HFeature> {
        self.history()?.features.iter().find(|f| !matches!(f.kind, FeatureKind::Sketch { .. }))
    }
    pub fn convert_availability(&self) -> Result<(), String> {
        match (&self.problem.history, &self.problem.geometry) {
            (Some(_), _) => Err("the model already has a feature history".into()),
            (None, Geometry::Imported { .. }) => Err("an imported mesh has no sketch to turn into a history".into()),
            (None, Geometry::Solid { .. }) => Err("a stored STEP solid has no sketch to turn into a history".into()),
            _ => Ok(()),
        }
    }
    pub fn feature_availability(&self, revolve: bool) -> Result<(), String> {
        if self.sketching.is_some() {
            return Err("finish the sketch first".into());
        }
        if matches!(self.problem.geometry, Geometry::Imported { .. }) {
            return Err("an imported mesh has no sketch".into());
        }
        if self.body_feature().is_some() {
            return Err("the model already has its solid feature; edit its parameters, or remove it first".into());
        }
        if self.problem.history.is_some() && self.profile_sketch().is_none() {
            return Err("draw a sketch first".into());
        }
        let _ = revolve;
        Ok(())
    }

    // ------------------------------------------------------------------ editing helpers

    /// Apply `f` to a copy of the problem; keep it (as one undoable edit) only when it and `apply_history` succeed.
    fn try_edit(&mut self, f: impl FnOnce(&mut Problem) -> Result<(), String>) -> Result<(), String> {
        let mut next = self.problem.clone();
        f(&mut next)?;
        next.apply_history()?;
        self.edit(|p| *p = next);
        Ok(())
    }

    /// Run `f` on the open sketch and its editor state; the change is one undo step.
    fn with_sketch<R>(&mut self, f: impl FnOnce(&mut Sketch, &mut SketchEdit) -> R) -> Option<R> {
        let mut st = self.sketching.take()?;
        let mut out = None;
        self.edit(|p| {
            if let Some((_, sk)) = p.history.as_mut().and_then(|h| h.sketch_mut(st.feature)) {
                out = Some(f(sk, &mut st.edit));
            }
        });
        self.sketching = Some(st);
        self.sync_surface_selection();
        self.settle_constraint_advice();
        self.touch();
        out
    }

    /// Selection and hover only: no undo step, no dirty flag.
    fn view_sketch<R>(&mut self, f: impl FnOnce(&Sketch, &mut SketchEdit) -> R) -> Option<R> {
        let st = self.sketching.as_mut()?;
        let sk = match &self.problem.history.as_ref()?.feature(st.feature)?.kind {
            FeatureKind::Sketch { sketch, .. } => sketch,
            _ => return None,
        };
        let r = f(sk, &mut st.edit);
        self.sync_surface_selection();
        self.touch();
        Some(r)
    }

    /// Mesh-surface names (`left`, `hole1`, `edge3`, ...) the selected sketch entities will have once the part is
    /// built, as the selected surfaces: supports and loads applied now act on those entities.
    fn sync_surface_selection(&mut self) {
        let names: Vec<String> = match (self.sketching.as_ref(), self.active_sketch()) {
            (Some(st), Some((_, sk))) => {
                let mut v: Vec<String> = sk.surface_names().unwrap_or_default().into_iter().filter(|(id, _)| st.edit.selection.contains(id)).map(|(_, n)| n).collect();
                v.dedup();
                v
            }
            _ => Vec::new(),
        };
        self.selected = None;
        self.multi.clear();
        for n in names {
            if let Some(prev) = self.selected.take() {
                self.multi.insert(prev);
            }
            self.selected = Some(n);
        }
    }

    /// Model-space point of the sketch plane under a viewport pixel.
    fn plane_point(&self, cam: &Camera, px: [f32; 2], size: [f32; 2]) -> Option<[f64; 2]> {
        let v = self.sketching.as_ref()?.view;
        let ndc = Camera::ndc(px[0] as f64, px[1] as f64, size[0], size[1]);
        let q = cam.unproject_to_plane(ndc, 2, 0.0)?;
        Some([v.center[0] + q[0] as f64 * v.span, v.center[1] + q[1] as f64 * v.span])
    }

    /// Dimension values and constraint glyphs placed on the sketch geometry, as viewport pixels for `cam`.
    pub(super) fn sketch_overlay(&self, cam: &Camera, size: [f32; 2]) -> Vec<Overlay> {
        use fea_problem::sketch::Entity;
        let (Some(st), Some((_, sk))) = (self.sketching.as_ref(), self.active_sketch()) else { return Vec::new() };
        let v = st.view;
        let px = |p: [f64; 2]| {
            let scene = [((p[0] - v.center[0]) / v.span) as f32, ((p[1] - v.center[1]) / v.span) as f32, 0.0];
            Camera::pixel(cam.project(scene).0, size[0], size[1])
        };
        // Where an entity is for a label: a point itself, a line's middle, a circle or arc's centre.
        let anchor = |id: Id| -> Option<[f64; 2]> {
            match sk.entity(id)? {
                Entity::Point { x, y } => Some([*x, *y]),
                Entity::Line { a, b } => {
                    let (a, b) = (sk.point(*a)?, sk.point(*b)?);
                    Some([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])
                }
                Entity::Circle { c, .. } | Entity::Arc { c, .. } => sk.point(*c),
            }
        };
        let mut out: Vec<Overlay> = Vec::new();
        let mut placed: Vec<[f32; 2]> = Vec::new();
        for c in &sk.constraints {
            let c = &c.v;
            let refs = c.refs();
            let pts: Vec<[f64; 2]> = refs.iter().filter_map(|id| anchor(*id)).collect();
            if pts.is_empty() {
                continue;
            }
            let mid = [pts.iter().map(|p| p[0]).sum::<f64>() / pts.len() as f64, pts.iter().map(|p| p[1]).sum::<f64>() / pts.len() as f64];
            let (text, kind, at) = match *c {
                Constraint::Length { d, .. } | Constraint::Distance { d, .. } | Constraint::HDist { d, .. } | Constraint::VDist { d, .. } => (format_number(d), OverlayKind::Dimension, mid),
                Constraint::Radius { e, d } => {
                    let centre = anchor(e).unwrap_or(mid);
                    (format!("R{}", format_number(d)), OverlayKind::Dimension, [centre[0] + d * 0.7, centre[1] + d * 0.7])
                }
                Constraint::Angle { deg, .. } => (format!("{} deg", format_number(deg)), OverlayKind::Dimension, mid),
                Constraint::Horizontal { .. } => ("H".into(), OverlayKind::Constraint, mid),
                Constraint::Vertical { .. } => ("V".into(), OverlayKind::Constraint, mid),
                Constraint::Parallel { .. } => ("//".into(), OverlayKind::Constraint, mid),
                Constraint::Perpendicular { .. } => ("_|_".into(), OverlayKind::Constraint, mid),
                Constraint::Equal { .. } => ("=".into(), OverlayKind::Constraint, mid),
                Constraint::Tangent { .. } => ("T".into(), OverlayKind::Constraint, mid),
                Constraint::Coincident { .. } => ("o".into(), OverlayKind::Constraint, pts[0]),
                Constraint::PointOn { .. } => ("o|".into(), OverlayKind::Constraint, pts[0]),
                Constraint::Midpoint { .. } => ("M".into(), OverlayKind::Constraint, pts[0]),
                Constraint::Fixed { .. } => ("F".into(), OverlayKind::Constraint, pts[0]),
            };
            let mut p = px(at);
            // Labels at the same place stack instead of overprinting; dimensions sit a little off their geometry.
            if kind == OverlayKind::Dimension {
                p[1] -= 14.0;
            } else {
                p[0] += 10.0;
                p[1] += 10.0;
            }
            while placed.iter().any(|q| (q[0] - p[0]).abs() < 28.0 && (q[1] - p[1]).abs() < 14.0) {
                p[1] += 15.0;
            }
            placed.push(p);
            if p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= size[0] && p[1] <= size[1] {
                out.push(Overlay { px: p, text, kind });
            }
        }
        out
    }

    /// Refresh the pick distance and grid from the current view before the editor uses them.
    fn sync_scale(&mut self, cam: &Camera, px: [f32; 2], size: [f32; 2]) {
        let Some(a) = self.plane_point(cam, px, size) else { return };
        let r = pick_radius(size);
        let Some(b) = self.plane_point(cam, [px[0] + r, px[1]], size) else { return };
        let pick = (b[0] - a[0]).hypot(b[1] - a[1]);
        if let Some(st) = &mut self.sketching {
            if pick.is_finite() && pick > 0.0 {
                st.edit.pick = pick;
            }
            st.edit.grid = if st.grid_on { sketch::nice_step(st.view.span * 4.0) } else { 0.0 };
        }
    }

    // ------------------------------------------------------------------ modes

    pub(super) fn enter_sketch(&mut self, feature: Id, entry: Problem) {
        let Some(FeatureKind::Sketch { sketch, .. }) = self.feature(feature).map(|f| f.kind.clone()) else { return };
        let ext = sketch.extent();
        let center = {
            let pts: Vec<[f64; 2]> = sketch.entities.iter().filter_map(|e| sketch.point(e.id)).collect();
            if pts.is_empty() {
                [0.0, 0.0]
            } else {
                let lo = pts.iter().fold([f64::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
                let hi = pts.iter().fold([f64::NEG_INFINITY; 2], |a, p| [a[0].max(p[0]), a[1].max(p[1])]);
                [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])]
            }
        };
        let mut edit = SketchEdit::new();
        edit.last_report = Some((&sketch.clone().solve()).into());
        self.sketching = Some(Sketching { feature, edit, view: SketchView { center, span: if ext > 0.0 { ext * 1.5 } else { 10.0 } }, grid_on: true, drag_before: None, entry });
        self.selected_feature = Some(feature);
        self.want_view = Some(StandardView::Front);
        self.status = "Sketch: pick a tool in the ribbon (L line, R rectangle, C circle, A arc, P polyline, D dimension).".into();
        self.touch();
    }

    fn finish_sketch(&mut self) {
        if self.sketching.is_none() {
            return;
        }
        match self.try_edit(|_| Ok(())) {
            Ok(()) => {
                self.sketching = None;
                self.typing = None;
                self.want_view = (self.problem.analysis == fea_problem::Analysis::Solid).then_some(StandardView::Isometric);
                self.status = "Sketch finished; the model is rebuilt from the history.".into();
                self.touch();
            }
            Err(e) => self.status = format!("Cannot finish yet: {e}"),
        }
    }

    /// Leave a sketch that is an open chain of lines and arcs without rebuilding the model from it: it is no profile,
    /// but it is what a sweep follows. The sweep, once added, is what rebuilds the model.
    fn finish_path(&mut self) {
        let Some((_, sk)) = self.active_sketch() else { return };
        match sk.open_path() {
            Ok(_) => {
                self.sketching = None;
                self.typing = None;
                self.status = "Path sketch finished. Select the profile sketch, then Sweep.".into();
                self.touch();
            }
            Err(e) => self.status = format!("This sketch is not a sweep path: {e}."),
        }
    }

    fn cancel_sketch(&mut self) {
        let Some(st) = self.sketching.take() else { return };
        self.typing = None;
        let entry = st.entry;
        self.edit(|p| *p = entry);
        self.want_view = (self.problem.analysis == fea_problem::Analysis::Solid).then_some(StandardView::Isometric);
        self.status = "Sketch cancelled: what it changed was undone (Ctrl/Cmd+Z brings it back).".into();
        self.touch();
    }

    /// Leave the sketch without rebuilding (undo moved past it, or the feature vanished).
    pub(super) fn drop_sketch_mode_if_orphaned(&mut self) {
        if let Some(st) = &self.sketching {
            if self.feature(st.feature).is_none() {
                self.sketching = None;
                self.typing = None;
            }
        }
    }

    fn convert_to_history(&mut self) {
        match self.convert_availability().and_then(|()| History::from_problem(&self.problem)) {
            Ok(h) => {
                let result = self.try_edit(|p| {
                    p.history = Some(h);
                    Ok(())
                });
                self.status = match result {
                    Ok(()) => "The model now has a feature history (Sketch1" .to_string() + if self.problem.analysis == fea_problem::Analysis::Solid { ", Extrude1)." } else { ")." },
                    Err(e) => format!("Cannot convert: {e}"),
                };
            }
            Err(e) => self.status = e,
        }
    }

    /// The flat faces of the body the history builds, as planes to sketch on (none while it has no extruded body).
    pub(super) fn face_choices(&self) -> Vec<fea_problem::history::PlanarFace> {
        self.history().and_then(|h| h.planar_faces().ok()).unwrap_or_default()
    }

    fn new_sketch(&mut self, plane: Plane, label: String) {
        if self.sketching.is_some() {
            self.status = "Finish the current sketch first.".into();
            return;
        }
        let entry = self.problem.clone();
        if self.problem.history.is_none() {
            // Never silently rewrite a loaded model: it is the user's choice to turn it into a history, or to start over.
            self.status = "This model has no feature history. Convert it (Features > Convert to history) to sketch on top of it, or File > New part to start empty.".into();
            return;
        }
        let mut id = 0;
        let n = self.history().map_or(0, |h| h.features.iter().filter(|f| matches!(f.kind, FeatureKind::Sketch { .. })).count());
        self.edit(|p| {
            if let Some(h) = &mut p.history {
                id = h.add(&format!("Sketch{}", n + 1), FeatureKind::Sketch { plane, sketch: Sketch::default() });
            }
        });
        self.enter_sketch(id, entry);
        self.status = format!("New sketch on the {label}.");
    }

    /// Delete a feature and the ones that depend on it. Always allowed: when what is left has no valid body the model
    /// is "unbuilt" (the viewport draws the sketches, meshing and solving say what is missing); deleting the last
    /// feature clears the model, including the geometry the history had produced and its boundary conditions.
    fn remove_feature(&mut self, id: Id) {
        if self.feature(id).is_none() {
            self.status = "That feature no longer exists.".into();
            return;
        }
        let mut next = self.problem.clone();
        if let Some(h) = next.history.as_mut() {
            h.remove(id);
        }
        if next.history.as_ref().is_some_and(|h| h.features.is_empty()) {
            next.clear_geometry();
        } else {
            let _ = next.apply_history();
        }
        self.edit(|p| *p = next);
        self.selected_feature = None;
        self.status = match self.problem.validate_geometry() {
            Ok(()) => "Feature deleted (with the features that depend on it).".to_string(),
            Err(e) => format!("Feature deleted. The model is empty or incomplete now: {e}."),
        };
    }

    /// Which sketches a sweep would use as `(profile, path)`: the selected sketch is the profile, else the first sketch;
    /// the path is the last other sketch.
    pub(super) fn sweep_sketches(&self) -> Result<(Id, Id), String> {
        self.feature_availability(false)?;
        let h = self.history().ok_or("the model has no feature history")?;
        let sketches: Vec<Id> = h.features[..h.active_len()].iter().filter(|f| matches!(f.kind, FeatureKind::Sketch { .. })).map(|f| f.id).collect();
        if sketches.len() < 2 {
            return Err("a sweep needs two sketches: draw the profile and the path (on a plane the profile leaves along its normal)".into());
        }
        let profile = self.selected_feature.filter(|id| sketches.contains(id)).unwrap_or(sketches[0]);
        let path = sketches.iter().rev().copied().find(|id| *id != profile).ok_or("a sweep needs two sketches")?;
        Ok((profile, path))
    }

    fn add_sweep(&mut self) {
        let (profile, path) = match self.sweep_sketches() {
            Ok(v) => v,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let result = self.try_edit(|p| {
            let h = p.history.as_mut().ok_or("no history")?;
            h.add("Sweep1", FeatureKind::Sweep { profile, path, layers: 12 });
            Ok(())
        });
        match result {
            Ok(()) => {
                self.selected_feature = self.body_feature().map(|f| f.id);
                self.want_view = Some(StandardView::Isometric);
                self.status = "Swept; the first sketch is the profile and the last the path. Edit the layers in the tree.".into();
            }
            Err(e) => self.status = format!("Cannot sweep: {e}"),
        }
    }

    fn add_body(&mut self, revolve: bool) {
        if let Err(e) = self.feature_availability(revolve) {
            self.status = e;
            return;
        }
        if self.problem.history.is_none() {
            self.convert_to_history();
            if self.problem.history.is_none() {
                return;
            }
            // A converted solid already has its Extrude; a converted plane problem gets the feature asked for.
            if self.body_feature().is_some() {
                self.status = "The model's sketch is extruded (Extrude1); edit its parameters in the tree.".into();
                return;
            }
        }
        let Some(profile) = self.profile_sketch() else { return };
        let depth = self.history().and_then(|h| h.feature(profile)).and_then(|f| match &f.kind {
            FeatureKind::Sketch { sketch, .. } => Some(sketch.extent() * 0.25),
            _ => None,
        });
        let depth = depth.filter(|d| *d > 0.0).unwrap_or(1.0);
        let result = self.try_edit(|p| {
            let h = p.history.as_mut().ok_or("no history")?;
            h.add(if revolve { "Revolve1" } else { "Extrude1" }, if revolve { FeatureKind::Revolve { profile, angle_deg: 360.0, segments: 12 } } else { FeatureKind::Extrude { profile, depth, layers: 4 } });
            Ok(())
        });
        match result {
            Ok(()) => {
                self.selected_feature = self.body_feature().map(|f| f.id);
                self.want_view = Some(StandardView::Isometric);
                self.status = if revolve { "Revolved 360 degrees about the sketch's y axis; edit the angle in the tree." } else { "Extruded; edit depth and layers in the tree." }.into();
            }
            Err(e) => self.status = format!("Cannot add the feature: {e}"),
        }
    }

    // ------------------------------------------------------------------ actions

    pub(super) fn do_sketch_action(&mut self, a: SketchAction) {
        match a {
            SketchAction::NewSketch(p) => self.new_sketch(p.plane(), p.label().to_string()),
            SketchAction::NewSketchOnFace(i) => match self.face_choices().into_iter().nth(i) {
                Some(f) => self.new_sketch(f.plane, format!("face '{}'", f.name)),
                None => self.status = "That face no longer exists; the body changed.".into(),
            },
            SketchAction::Edit(id) => {
                if self.sketching.is_some() {
                    self.status = "Finish the current sketch first.".into();
                } else {
                    let entry = self.problem.clone();
                    self.enter_sketch(id, entry)
                }
            }
            SketchAction::Finish => self.finish_sketch(),
            SketchAction::FinishPath => self.finish_path(),
            SketchAction::Cancel => self.cancel_sketch(),
            SketchAction::Tool(t) => {
                if let Some(st) = &mut self.sketching {
                    st.edit.set_tool(t);
                    self.status = st.edit.status.clone();
                }
                self.typing = None;
                // Choosing Dimension with something already selected dimensions it at once.
                if t == Tool::Dimension {
                    self.begin_dimension(None);
                }
                self.touch();
            }
            SketchAction::Constrain(rule) => {
                let r = self.with_sketch(|sk, ed| ed.constrain_selected(sk, rule));
                self.report(r.unwrap_or(Err("not in a sketch".into())));
            }
            SketchAction::Accept(i) => {
                let r = self.with_sketch(|sk, ed| ed.accept(sk, i));
                self.report(r.unwrap_or(Err("not in a sketch".into())));
            }
            SketchAction::DeleteSelected => {
                let n = self.with_sketch(|sk, ed| ed.delete_selected(sk)).unwrap_or(0);
                self.status = if n == 0 { "Nothing selected to delete.".into() } else { format!("Deleted {n} item(s).") };
            }
            SketchAction::RemoveConstraint(id) => {
                self.with_sketch(|sk, ed| {
                    sk.remove_constraint(id);
                    let r = sk.solve();
                    ed.last_report = Some((&r).into());
                });
                self.status = "Constraint removed.".into();
            }
            SketchAction::Dimension => self.begin_dimension(None),
            SketchAction::Fix => {
                let r = self.with_sketch(|sk, ed| ed.fix_selected(sk));
                if let Some(Err(e)) = r {
                    self.status = e;
                } else if let Some(st) = &self.sketching {
                    self.status = st.edit.status.clone();
                }
            }
            SketchAction::ToggleGrid => {
                if let Some(st) = &mut self.sketching {
                    st.grid_on = !st.grid_on;
                    self.status = if st.grid_on { "Grid snap on." } else { "Grid snap off." }.into();
                }
            }
            SketchAction::Convert => self.convert_to_history(),
            SketchAction::AddExtrude => self.add_body(false),
            SketchAction::AddSweep => self.add_sweep(),
            SketchAction::AddRevolve => self.add_body(true),
            SketchAction::SelectFeature(id) => {
                self.selected_feature = Some(id);
                self.status = self.feature(id).map(|f| format!("{} selected.", f.name)).unwrap_or_default();
                self.touch();
            }
            SketchAction::Suppress(id) => {
                let now = self.feature(id).map(|f| !f.suppressed).unwrap_or(false);
                let r = self.try_edit(|p| {
                    let h = p.history.as_mut().ok_or("no history")?;
                    h.features.iter_mut().find(|f| f.id == id).ok_or("no such feature")?.suppressed = now;
                    Ok(())
                });
                self.status = match r {
                    Ok(()) => if now { "Feature suppressed." } else { "Feature restored." }.to_string(),
                    Err(e) => format!("Cannot change that: {e}"),
                };
            }
            SketchAction::RollbackTo(to) => {
                let r = self.try_edit(|p| {
                    let h = p.history.as_mut().ok_or("no history")?;
                    h.rollback = match to {
                        Some(id) => Some(h.features.iter().position(|f| f.id == id).ok_or("no such feature")? + 1),
                        None => None,
                    };
                    Ok(())
                });
                self.status = match r {
                    Ok(()) => if to.is_some() { "Rolled back: later features are inactive." } else { "Rolled forward to the end of the history." }.to_string(),
                    Err(e) => format!("Cannot roll back there: {e}"),
                };
            }
            SketchAction::RemoveFeature(id) => self.remove_feature(id),
            SketchAction::EditParam(id, param) => {
                let current = self.param_value(id, param);
                self.typing = Some(Typing { target: TypeTarget::Param(id, param), text: current.map(format_number).unwrap_or_default(), fresh: true });
                self.status = format!("Type the {} and press Enter (Esc cancels).", param.label());
            }
        }
        self.touch();
    }

    fn report(&mut self, r: Result<(), String>) {
        if let Err(e) = r {
            self.status = e;
        }
        if let Some(st) = &self.sketching {
            if !st.edit.status.is_empty() && self.status.is_empty() {
                self.status = st.edit.status.clone();
            }
        }
    }

    fn param_value(&self, id: Id, p: FeatureParam) -> Option<f64> {
        Some(match (&self.feature(id)?.kind, p) {
            (FeatureKind::Extrude { depth, .. }, FeatureParam::Depth) => *depth,
            (FeatureKind::Extrude { layers, .. }, FeatureParam::Layers) => *layers as f64,
            (FeatureKind::Revolve { angle_deg, .. }, FeatureParam::Angle) => *angle_deg,
            (FeatureKind::Revolve { segments, .. }, FeatureParam::Segments) => *segments as f64,
            (FeatureKind::Sweep { layers, .. }, FeatureParam::Layers) => *layers as f64,
            (FeatureKind::Sketch { plane, .. }, FeatureParam::PlaneOffset) => match plane {
                Plane::Front { offset } | Plane::Top { offset } | Plane::Right { offset } => *offset,
                Plane::Datum { .. } => return None,
            },
            _ => return None,
        })
    }

    fn begin_dimension(&mut self, first: Option<char>) {
        let Some(st) = &self.sketching else { return };
        let Some((_, sk)) = self.active_sketch() else { return };
        if st.edit.dimension_kind(sk).is_none() {
            self.status = "Select a line, a circle / arc, two points or two lines to dimension.".into();
            return;
        }
        let current = st.edit.dimension_value(sk);
        let (text, fresh) = match first {
            Some(c) => (c.to_string(), false),
            None => (current.map(format_number).unwrap_or_default(), true),
        };
        self.typing = Some(Typing { target: TypeTarget::Dimension, text, fresh });
    }

    /// Commit the typed number.
    fn commit_typing(&mut self) {
        let Some(t) = self.typing.take() else { return };
        let Ok(v) = t.text.trim().parse::<f64>() else {
            self.status = format!("'{}' is not a number.", t.text);
            self.typing = Some(t);
            return;
        };
        match t.target {
            TypeTarget::Dimension => {
                let r = self.with_sketch(|sk, ed| ed.set_dimension(sk, v));
                match r {
                    Some(Ok(())) => {
                        if let Some(st) = &mut self.sketching {
                            st.edit.selection.clear();
                            self.status = st.edit.status.clone();
                        }
                    }
                    Some(Err(e)) => self.status = e,
                    None => {}
                }
            }
            TypeTarget::Param(id, param) => {
                let r = self.try_edit(|p| {
                    let h = p.history.as_mut().ok_or("no history")?;
                    let f = h.features.iter_mut().find(|f| f.id == id).ok_or("no such feature")?;
                    let whole = v.fract() == 0.0 && (1.0..=10_000.0).contains(&v);
                    match (&mut f.kind, param) {
                        (FeatureKind::Extrude { depth, .. }, FeatureParam::Depth) => *depth = v,
                        (FeatureKind::Extrude { layers, .. }, FeatureParam::Layers) if whole => *layers = v as usize,
                        (FeatureKind::Revolve { angle_deg, .. }, FeatureParam::Angle) => *angle_deg = v,
                        (FeatureKind::Revolve { segments, .. }, FeatureParam::Segments) if whole => *segments = v as usize,
                        (FeatureKind::Sweep { layers, .. }, FeatureParam::Layers) if whole => *layers = v as usize,
                        (FeatureKind::Sketch { plane, .. }, FeatureParam::PlaneOffset) => match plane {
                            Plane::Front { offset } | Plane::Top { offset } | Plane::Right { offset } => *offset = v,
                            Plane::Datum { .. } => return Err("a datum plane has no offset".into()),
                        },
                        _ => return Err(format!("{} must be a whole number of at least 1", param.label())),
                    }
                    Ok(())
                });
                self.status = match r {
                    Ok(()) => format!("{} set to {}.", param.label(), format_number(v)),
                    Err(e) => e,
                };
            }
        }
        self.touch();
    }

    // ------------------------------------------------------------------ keys

    /// Keys while a number is typed or a sketch is open; `None` leaves the key to the normal handler.
    pub(super) fn sketch_key(&mut self, key: Key, mods: Mods) -> Option<KeyResult> {
        if self.typing.is_some() {
            match key {
                Key::Escape => self.typing = None,
                Key::Enter => self.commit_typing(),
                Key::Backspace => {
                    if let Some(t) = &mut self.typing {
                        if std::mem::take(&mut t.fresh) {
                            t.text.clear();
                        } else {
                            t.text.pop();
                        }
                    }
                }
                Key::Char(c) if is_edit_char(c) => {
                    if let Some(t) = &mut self.typing {
                        if std::mem::take(&mut t.fresh) {
                            t.text.clear();
                        }
                        t.text.push(c);
                    }
                }
                _ => {}
            }
            self.touch();
            return Some(KeyResult::Handled);
        }
        let (tool, busy) = {
            let st = self.sketching.as_ref()?;
            (st.edit.tool, st.edit.busy())
        };
        if mods.command {
            return None;
        }
        let handled = match key {
            Key::Escape => {
                if busy {
                    self.view_sketch(|_, ed| ed.cancel());
                } else if tool != Tool::Select {
                    self.do_sketch_action(SketchAction::Tool(Tool::Select));
                } else {
                    self.view_sketch(|_, ed| ed.selection.clear());
                }
                true
            }
            Key::Enter => {
                if tool == Tool::Dimension {
                    self.begin_dimension(None);
                } else if busy {
                    self.view_sketch(|_, ed| ed.cancel());
                } else {
                    self.finish_sketch();
                }
                true
            }
            Key::Delete | Key::Backspace => {
                self.do_sketch_action(SketchAction::DeleteSelected);
                true
            }
            Key::Char(c) if c.is_ascii_digit() && c != '0' && self.sketching.as_ref().is_none_or(|s| s.edit.tool != Tool::Dimension) => {
                self.do_sketch_action(SketchAction::Accept(c as usize - '1' as usize));
                true
            }
            Key::Char(c) if tool == Tool::Dimension && is_edit_char(c) && !c.eq_ignore_ascii_case(&'e') => {
                self.begin_dimension(Some(c));
                true
            }
            Key::Char(c) => {
                let t = match c.to_ascii_lowercase() {
                    's' => Some(Tool::Select),
                    'l' => Some(Tool::Line),
                    'p' => Some(Tool::Polyline),
                    'r' => Some(Tool::Rectangle),
                    'c' => Some(Tool::Circle),
                    'a' => Some(Tool::Arc),
                    'd' => Some(Tool::Dimension),
                    _ => None,
                };
                match (t, c.to_ascii_lowercase()) {
                    (Some(t), _) => {
                        self.do_sketch_action(SketchAction::Tool(t));
                        true
                    }
                    (None, 'h') => {
                        self.do_sketch_action(SketchAction::Constrain(Rule::Horizontal));
                        true
                    }
                    (None, 'v') => {
                        self.do_sketch_action(SketchAction::Constrain(Rule::Vertical));
                        true
                    }
                    (None, 'f') => {
                        self.do_sketch_action(SketchAction::Fix);
                        true
                    }
                    (None, 'g') => {
                        self.do_sketch_action(SketchAction::ToggleGrid);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        handled.then_some(KeyResult::Handled)
    }

    // ------------------------------------------------------------------ pointer

    pub(super) fn sketch_click(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) {
        self.sync_scale(cam, px, size);
        let Some(pt) = self.plane_point(cam, px, size) else {
            self.status = "Turn the view to face the sketch plane (Front view) to sketch.".into();
            return;
        };
        let tool = self.sketching.as_ref().map(|s| s.edit.tool).unwrap_or_default();
        match tool {
            Tool::Select | Tool::Dimension => {
                if tool == Tool::Dimension {
                    self.view_sketch(|sk, ed| ed.dimension_select(sk, pt, additive));
                } else {
                    self.view_sketch(|sk, ed| ed.select_at(sk, pt, additive));
                }
                if tool == Tool::Dimension {
                    let ready = self.sketching.as_ref().zip(self.active_sketch()).is_some_and(|(st, (_, sk))| st.edit.dimension_kind(sk).is_some());
                    if ready {
                        self.begin_dimension(None);
                    }
                }
            }
            _ => {
                let r = self.with_sketch(|sk, ed| ed.click(sk, pt));
                match r {
                    Some(Ok(_)) => self.status = self.sketching.as_ref().map(|s| s.edit.status.clone()).unwrap_or_default(),
                    Some(Err(e)) => self.status = e,
                    None => {}
                }
            }
        }
    }

    pub(super) fn sketch_hover(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        self.sync_scale(cam, px, size);
        let Some(pt) = self.plane_point(cam, px, size) else { return false };
        let before = self.sketching.as_ref().and_then(|s| s.edit.hover);
        self.view_sketch(|sk, ed| ed.hover_at(sk, pt));
        self.sketching.as_ref().and_then(|s| s.edit.hover) != before
    }

    pub(super) fn sketch_grab(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        if self.sketching.as_ref().is_none_or(|s| s.edit.tool != Tool::Select) || self.typing.is_some() {
            return false;
        }
        self.sync_scale(cam, px, size);
        let Some(pt) = self.plane_point(cam, px, size) else { return false };
        let hit = self.active_sketch().and_then(|(_, sk)| self.sketching.as_ref().and_then(|s| s.edit.pick_entity(sk, pt)));
        let Some(id) = hit else { return false };
        let is_draggable = self.active_sketch().is_some_and(|(_, sk)| matches!(sk.entity(id), Some(fea_problem::sketch::Entity::Point { .. } | fea_problem::sketch::Entity::Circle { .. })));
        if !is_draggable {
            return false;
        }
        let before = self.problem.clone();
        if let Some(st) = &mut self.sketching {
            st.drag_before = Some(before);
            st.edit.selection = [id].into();
        }
        self.sync_surface_selection();
        self.touch();
        true
    }

    pub(super) fn sketch_drag(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        let Some(pt) = self.plane_point(cam, px, size) else { return };
        let Some(st) = &mut self.sketching else { return };
        let Some(id) = st.edit.selection.iter().next().copied() else { return };
        let feature = st.feature;
        if let Some((_, sk)) = self.problem.history.as_mut().and_then(|h| h.sketch_mut(feature)) {
            st.edit.drag(sk, id, pt);
        }
        self.touch();
    }

    pub(super) fn sketch_release(&mut self) {
        let Some(before) = self.sketching.as_mut().and_then(|s| s.drag_before.take()) else { return };
        if self.problem != before {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
        }
        self.touch();
    }

    /// Right press in a sketch: select what is under the cursor (the context menu then offers constraints).
    pub(super) fn sketch_select_at(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        self.sync_scale(cam, px, size);
        let Some(pt) = self.plane_point(cam, px, size) else { return };
        let keep = self.sketching.as_ref().is_some_and(|s| s.edit.pick_entity(self.active_sketch().map(|a| a.1).expect("sketch"), pt).is_some_and(|id| s.edit.selection.contains(&id)));
        if !keep {
            self.view_sketch(|sk, ed| ed.select_at(sk, pt, false));
        }
    }

    pub(super) fn sketch_box(&mut self, a: [f32; 2], b: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) {
        let (Some(p), Some(q)) = (self.plane_point(cam, a, size), self.plane_point(cam, b, size)) else { return };
        self.view_sketch(|sk, ed| ed.select_box(sk, p, q, additive));
    }

    // ------------------------------------------------------------------ scene

    pub(super) fn sketch_scene(&self) -> Result<Scene, String> {
        let st = self.sketching.as_ref().ok_or("not sketching")?;
        let (plane, sk) = self.active_sketch().ok_or("the sketch feature is missing")?;
        let v = st.view;
        let to = |p: [f64; 2]| [((p[0] - v.center[0]) / v.span) as f32, ((p[1] - v.center[1]) / v.span) as f32, 0.0];
        let mut b = Builder::new();
        // Axes through the sketch origin.
        let o = to([0.0, 0.0]);
        b.tube([o[0] - 0.6, o[1], 0.0], [o[0] + 0.6, o[1], 0.0], 0.0012, [None, None], CODE_FRAME);
        b.tube([o[0], o[1] - 0.6, 0.0], [o[0], o[1] + 0.6, 0.0], 0.0012, [None, None], CODE_FRAME);
        let defined = st.edit.last_report.is_some_and(|r| r.status == Status::FullyConstrained);
        let base = if defined { CODE_SUPPORT } else { CODE_TRUSS };
        for (id, p, q) in sketcher::sketch_segments(sk) {
            let sel = st.edit.selection.contains(&id);
            b.tube(to(p), to(q), if sel { 0.004 } else { 0.0028 }, [None, None], if sel { CODE_SELECTED } else { base });
        }
        for e in &sk.entities {
            if let Some(p) = sk.point(e.id) {
                let sel = st.edit.selection.contains(&e.id);
                b.cube(to(p), if sel { 0.0085 } else { 0.0055 }, None, if sel { CODE_SELECTED } else { CODE_FRAME });
            }
        }
        for (p, q) in st.edit.preview() {
            b.tube(to(p), to(q), 0.0022, [None, None], CODE_LOAD);
        }
        if let Some(h) = st.edit.hover {
            b.cube(to(h.at), 0.009, None, CODE_LOAD);
        }
        b.into_scene(format!("{} | sketch on the {} (blue: under-defined, teal: fully defined)", self.problem.name, plane.label()))
    }

    /// The model's sketches (or its outline and holes) as curves in the plane they live on, for a model that has no
    /// current mesh: nothing meshes on its own, so without it the viewport would be an empty placeholder until the
    /// user generates the mesh. `None`: nothing to draw.
    pub(super) fn outline_scene(&self, why: &str) -> Option<Scene> {
        let mut segs: Vec<([f64; 3], [f64; 3], bool)> = Vec::new();
        let mut add = |frame: fea_problem::history::Frame, sketch: &Sketch| {
            let closed = sketch.profile().is_ok();
            for (_, p, q) in sketcher::sketch_segments(sketch) {
                segs.push((frame.apply([p[0], p[1], 0.0]), frame.apply([q[0], q[1], 0.0]), closed));
            }
        };
        match (&self.problem.history, &self.problem.geometry) {
            (Some(h), _) => {
                for f in h.features[..h.active_len()].iter().filter(|f| !f.suppressed) {
                    if let FeatureKind::Sketch { plane, sketch } = &f.kind {
                        if let Ok(frame) = plane.frame() {
                            add(frame, sketch);
                        }
                    }
                }
            }
            (None, Geometry::Sketch { outer, holes, .. }) => {
                if let Ok(frame) = (Plane::Front { offset: 0.0 }).frame() {
                    add(frame, &Sketch::from_shapes(outer, holes));
                }
            }
            (None, Geometry::Imported { .. } | Geometry::Solid { .. }) => {}
        }
        if segs.is_empty() {
            return None;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for (a, b, _) in &segs {
            for p in [a, b] {
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
        }
        let half = (0..3).map(|i| 0.5 * (hi[i] - lo[i])).fold(0.0_f64, f64::max).max(1e-9);
        let to = |p: [f64; 3]| -> [f32; 3] { std::array::from_fn(|i| (0.5 * (p[i] - 0.5 * (lo[i] + hi[i])) / half) as f32) };
        let mut b = Builder::new();
        for (p, q, closed) in &segs {
            b.tube(to(*p), to(*q), 0.004, [None, None], if *closed { CODE_SUPPORT } else { CODE_TRUSS });
        }
        for (p, q, level) in self.refine_rings() {
            b.tube(to(p), to(q), 0.002 + 0.0018 * f32::from(level), [None, None], super::refining::ring_code(level));
        }
        b.into_scene(format!("{} | sketch outline ({why})", self.problem.name)).ok()
    }

    // ------------------------------------------------------------------ panels

    fn constraint_text(&self, sk: &Sketch, c: &Constraint) -> String {
        let name = |id: Id| format!("#{id}");
        let refs: Vec<String> = c.refs().into_iter().map(name).collect();
        let value = match c {
            Constraint::Length { d, .. } | Constraint::Distance { d, .. } | Constraint::HDist { d, .. } | Constraint::VDist { d, .. } | Constraint::Radius { d, .. } => format!(" = {}", format_number(*d)),
            Constraint::Angle { deg, .. } => format!(" = {} deg", format_number(*deg)),
            Constraint::Fixed { x, y, .. } => format!(" at ({}, {})", format_number(*x), format_number(*y)),
            _ => String::new(),
        };
        let _ = sk;
        format!("{} {}{}", c.label(), refs.join(" "), value)
    }

    pub(super) fn sketch_properties(&self) -> Vec<DockItem> {
        let mut out = Vec::new();
        let (Some(st), Some((plane, sk))) = (self.sketching.as_ref(), self.active_sketch()) else { return out };
        let name = self.feature(st.feature).map_or("Sketch", |f| f.name.as_str());
        out.push(DockItem::Heading(format!("{name} on the {}", plane.label())));
        let (text, tone) = match st.edit.last_report {
            Some(r) => match r.status {
                Status::UnderConstrained => (format!("Under-defined: {} degrees of freedom left", r.dof), Tone::Warn),
                Status::FullyConstrained => ("Fully defined".to_string(), Tone::On),
                Status::OverConstrained => ("Over-defined: some constraints are redundant".to_string(), Tone::Warn),
                Status::Conflicting => ("Conflicting constraints".to_string(), Tone::Warn),
            },
            None => ("Empty sketch".to_string(), Tone::Dim),
        };
        out.push(DockItem::text(text, tone));
        out.push(DockItem::text(format!("tool: {}   grid snap: {}", st.edit.tool.label(), if st.grid_on { "on (G)" } else { "off (G)" }), Tone::Dim));
        if let Some(t) = &self.typing {
            if t.target == TypeTarget::Dimension {
                let label = st.edit.dimension_kind(sk).map_or("value", DimKind::label);
                out.push(DockItem::row(format!("{label} = [{}_]", t.text), Tone::Warn, None, true));
            }
        }
        out.push(DockItem::Gap);
        out.push(DockItem::Heading("Selection".into()));
        match st.edit.selection.len() {
            0 => out.push(DockItem::text("click an entity; Shift adds; drag a point", Tone::Dim)),
            n => {
                out.push(DockItem::text(format!("{n} selected"), Tone::Normal));
                let names = self.all_selected();
                if !names.is_empty() {
                    out.push(DockItem::text(format!("surfaces: {}", names.join(", ")), Tone::Dim));
                }
                if let Some(k) = st.edit.dimension_kind(sk) {
                    let v = st.edit.dimension_value(sk).map_or(String::new(), format_number);
                    out.push(DockItem::row(format!("Dimension {} (now {v}) - click or type", k.label()), Tone::On, Some(ui(SketchAction::Dimension)), false));
                }
            }
        }
        if !st.edit.suggestions.is_empty() {
            out.push(DockItem::Gap);
            out.push(DockItem::Heading(if st.edit.learned_order { "Suggested constraints (learned order)".into() } else { "Suggested constraints".to_string() }));
            for (i, s) in st.edit.suggestions.iter().take(9).enumerate() {
                out.push(DockItem::row(format!("{} {}  ({:.0}%)", i + 1, self.constraint_text(sk, &s.constraint), 100.0 * s.score), Tone::On, Some(ui(SketchAction::Accept(i))), false));
            }
            out.push(DockItem::text("press the number or click to apply", Tone::Dim));
        }
        out.push(DockItem::Gap);
        out.push(DockItem::Heading(format!("Constraints ({})", sk.constraints.len())));
        for c in &sk.constraints {
            out.push(DockItem::row(self.constraint_text(sk, &c.v), if c.v.is_dimension() { Tone::Normal } else { Tone::Dim }, Some(ui(SketchAction::RemoveConstraint(c.id))), false));
        }
        if !sk.constraints.is_empty() {
            out.push(DockItem::text("click a constraint to remove it (Ctrl/Cmd+Z undoes)", Tone::Dim));
        }
        out
    }

    /// Right-click menu of a history row: what can be done to the feature (its values are in the inspector).
    pub(super) fn history_node_menu(&self, node: u32) -> Vec<Item> {
        let id = node - T_HISTORY_BASE;
        let Some(f) = self.feature(id) else { return Vec::new() };
        let mut v = Vec::new();
        if matches!(f.kind, FeatureKind::Sketch { .. }) && self.sketching.is_none() {
            v.push(Item::Entry(entry("Edit sketch", "", SketchAction::Edit(id))));
        }
        v.push(Item::Entry(entry(if f.suppressed { "Unsuppress" } else { "Suppress" }, "", SketchAction::Suppress(id))));
        v.push(Item::Entry(entry("Roll back to here", "", SketchAction::RollbackTo(Some(id)))));
        if self.history().is_some_and(|h| h.rollback.is_some()) {
            v.push(Item::Entry(entry("Roll forward to the end", "", SketchAction::RollbackTo(None))));
        }
        v.push(Item::Separator);
        v.push(Item::Entry(entry("Delete feature", "", SketchAction::RemoveFeature(id))));
        v
    }

    /// Parameters of the selected history feature (the commands are its tree row's right-click menu).
    pub(super) fn feature_panel(&self) -> Vec<DockItem> {
        let mut out = Vec::new();
        let Some(f) = self.selected_feature.and_then(|id| self.feature(id)) else { return out };
        out.push(DockItem::Gap);
        out.push(DockItem::Heading(format!("Feature: {} ({})", f.name, f.kind.label())));
        let row = |label: &str, param: FeatureParam| {
            let v = self.param_value(f.id, param).map_or_else(|| "-".to_string(), format_number);
            DockItem::field(label, v, Tone::Normal, Some(ui(SketchAction::EditParam(f.id, param))), false)
        };
        match &f.kind {
            FeatureKind::Sketch { plane, sketch } => {
                out.push(DockItem::text(format!("{}, {} entities, {} constraints", plane.label(), sketch.entities.len(), sketch.constraints.len()), Tone::Dim));
                if !matches!(plane, Plane::Datum { .. }) {
                    out.push(row("Plane offset", FeatureParam::PlaneOffset));
                }
            }
            FeatureKind::Extrude { .. } => {
                out.push(row("Depth", FeatureParam::Depth));
                out.push(row("Layers", FeatureParam::Layers));
            }
            FeatureKind::Revolve { .. } => {
                out.push(row("Angle (deg)", FeatureParam::Angle));
                out.push(row("Segments", FeatureParam::Segments));
                out.push(DockItem::text("axis: the sketch's y axis; x is the radius", Tone::Dim));
            }
            FeatureKind::Sweep { profile, path, .. } => {
                let name = |id: Id| self.feature(id).map_or("?".to_string(), |f| f.name.clone());
                out.push(DockItem::text(format!("profile {} along path {}", name(*profile), name(*path)), Tone::Dim));
                out.push(row("Layers", FeatureParam::Layers));
            }
        }
        if let Some(t) = &self.typing {
            if let TypeTarget::Param(_, p) = t.target {
                out.push(DockItem::row(format!("{} = [{}_]", p.label(), t.text), Tone::Warn, None, true));
            }
        }
        out
    }

    pub(super) fn history_nodes(&self) -> Vec<TreeNode> {
        let mut out = Vec::new();
        let Some(h) = self.history() else { return out };
        out.push(TreeNode::new(T_HISTORY, 1, '+', format!("History ({})", h.features.len())).folder());
        let active = h.active_len();
        for (i, f) in h.features.iter().enumerate() {
            let glyph = match f.kind {
                FeatureKind::Sketch { .. } => 'S',
                FeatureKind::Extrude { .. } => 'E',
                FeatureKind::Revolve { .. } => 'R',
                FeatureKind::Sweep { .. } => 'W',
            };
            let mark = if i >= active { " (rolled back)" } else if f.suppressed { " (suppressed)" } else { "" };
            let editing = self.sketching.as_ref().is_some_and(|s| s.feature == f.id);
            out.push(
                TreeNode::new(T_HISTORY_BASE + f.id, 2, glyph, format!("{}{}{}", f.name, mark, if editing { " [editing]" } else { "" }))
                    .selected(self.selected_feature == Some(f.id))
                    .on_click(ui(SketchAction::SelectFeature(f.id))),
            );
        }
        out
    }

    fn tool_entry(&self, t: Tool) -> Entry {
        let key = match t {
            Tool::Select => "S",
            Tool::Line => "L",
            Tool::Polyline => "P",
            Tool::Rectangle => "R",
            Tool::Circle => "C",
            Tool::Arc => "A",
            Tool::Dimension => "D",
        };
        entry(t.label(), key, SketchAction::Tool(t)).checked(self.sketching.as_ref().is_some_and(|s| s.edit.tool == t))
    }

    fn constraint_availability(&self, rule: Rule) -> Result<(), String> {
        let st = self.sketching.as_ref().ok_or("open a sketch first")?;
        let (_, sk) = self.active_sketch().ok_or("open a sketch first")?;
        st.edit.selection_constraint(sk, rule).map(|_| ())
    }

    fn fix_availability(&self) -> Result<(), String> {
        let st = self.sketching.as_ref().ok_or("open a sketch first")?;
        let (_, sk) = self.active_sketch().ok_or("open a sketch first")?;
        if st.edit.fixable(sk).is_empty() { Err("select a point that is not fixed yet".into()) } else { Ok(()) }
    }

    fn constrain_entries(&self) -> Vec<Entry> {
        [
            (Rule::Horizontal, "Horizontal", "H"),
            (Rule::Vertical, "Vertical", "V"),
            (Rule::Parallel, "Parallel", ""),
            (Rule::Perpendicular, "Perpendicular", ""),
            (Rule::Equal, "Equal", ""),
            (Rule::Tangent, "Tangent", ""),
            (Rule::Coincident, "Coincident", ""),
            (Rule::PointOn, "Point on", ""),
            (Rule::Midpoint, "Midpoint", ""),
        ]
        .into_iter()
        .map(|(r, label, key)| entry(label, key, SketchAction::Constrain(r)).when(self.constraint_availability(r)))
        .collect()
    }

    pub(super) fn sketch_ribbon(&self) -> Vec<RibbonTab> {
        let on_edges = if self.all_selected().is_empty() { Err("select an edge or a hole of the sketch first".to_string()) } else { Ok(()) };
        vec![RibbonTab::new("Sketch")
            .group("Sketch", vec![entry("Finish sketch", "Enter", SketchAction::Finish), entry("Finish as sweep path", "", SketchAction::FinishPath), entry("Cancel sketch", "", SketchAction::Cancel), entry("Grid snap", "G", SketchAction::ToggleGrid).checked(self.sketching.as_ref().is_some_and(|s| s.grid_on))])
            .group("Draw", [Tool::Line, Tool::Polyline, Tool::Rectangle, Tool::Circle, Tool::Arc].into_iter().map(|t| self.tool_entry(t)).collect())
            .group("Select", vec![self.tool_entry(Tool::Select)])
            .group("Constrain", self.constrain_entries())
            .group("Dimension", vec![self.tool_entry(Tool::Dimension), entry("Fix point", "F", SketchAction::Fix).when(self.fix_availability())])
            .group(
                "Boundary",
                vec![
                    Entry::new("Support on selection", "", UiAction::Editor(EditorAction::Solid(SolidAction::SupportOnSelected))).when(on_edges.clone()),
                    Entry::new("Load on selection", "", UiAction::Editor(EditorAction::Solid(SolidAction::LoadOnSelected))).when(on_edges),
                ],
            )]
    }

    pub(super) fn features_ribbon(&self) -> RibbonTab {
        let mut sketch: Vec<Entry> = PlaneChoice::ALL.iter().map(|p| entry(&format!("Sketch on {}", p.label()), "", SketchAction::NewSketch(*p))).collect();
        sketch.extend(self.face_entries());
        sketch.push(entry("Edit selected sketch", "", SketchAction::Edit(self.selected_feature.unwrap_or(0))).when(self.edit_availability()));
        RibbonTab::new("Features").group("Sketch", sketch).group(
            "Solid",
            vec![
                entry("Extrude", "", SketchAction::AddExtrude).when(self.feature_availability(false)),
                entry("Revolve", "", SketchAction::AddRevolve).when(self.feature_availability(true)),
                entry("Sweep", "", SketchAction::AddSweep).when(self.sweep_sketches().map(|_| ())),
                entry("Convert to history", "", SketchAction::Convert).when(self.convert_availability()),
            ],
        )
    }

    /// "Sketch on face ..." for each flat face of the built body.
    fn face_entries(&self) -> Vec<Entry> {
        self.face_choices().iter().enumerate().map(|(i, f)| entry(&format!("Sketch on face '{}'", f.name), "", SketchAction::NewSketchOnFace(i))).collect()
    }

    fn edit_availability(&self) -> Result<(), String> {
        match self.selected_feature.and_then(|id| self.feature(id)) {
            Some(f) if matches!(f.kind, FeatureKind::Sketch { .. }) => Ok(()),
            Some(_) => Err("the selected feature is not a sketch".into()),
            None => Err("select a sketch in the tree first".into()),
        }
    }

    /// The Insert menu: sketch tools while sketching, else the model inserts plus the history commands.
    pub(super) fn insert_items(&self, base: Vec<Item>) -> Vec<Item> {
        if self.sketching.is_some() {
            let mut v: Vec<Item> = [Tool::Line, Tool::Polyline, Tool::Rectangle, Tool::Circle, Tool::Arc, Tool::Dimension, Tool::Select].into_iter().map(|t| Item::Entry(self.tool_entry(t))).collect();
            v.push(Item::Separator);
            v.extend(self.constrain_entries().into_iter().map(Item::Entry));
            v.push(Item::Separator);
            v.push(Item::Entry(entry("Finish sketch", "Enter", SketchAction::Finish)));
            v.push(Item::Entry(entry("Finish as sweep path", "", SketchAction::FinishPath)));
            v.push(Item::Entry(entry("Cancel sketch", "", SketchAction::Cancel)));
            return v;
        }
        let mut v = base;
        v.push(Item::Separator);
        v.extend(PlaneChoice::ALL.iter().map(|p| Item::Entry(entry(&format!("Sketch on {}", p.label()), "", SketchAction::NewSketch(*p)))));
        v.extend(self.face_entries().into_iter().map(Item::Entry));
        v.push(Item::Entry(entry("Edit selected sketch", "", SketchAction::Edit(self.selected_feature.unwrap_or(0))).when(self.edit_availability())));
        v.push(Item::Entry(entry("Extrude", "", SketchAction::AddExtrude).when(self.feature_availability(false))));
        v.push(Item::Entry(entry("Revolve", "", SketchAction::AddRevolve).when(self.feature_availability(true))));
        v.push(Item::Entry(entry("Sweep", "", SketchAction::AddSweep).when(self.sweep_sketches().map(|_| ()))));
        v.push(Item::Entry(entry("Convert to feature history", "", SketchAction::Convert).when(self.convert_availability())));
        v
    }

    pub(super) fn sketch_context(&self) -> Vec<Item> {
        let mut v: Vec<Item> = self.constrain_entries().into_iter().map(Item::Entry).collect();
        v.push(Item::Separator);
        v.push(Item::Entry(entry("Dimension selection", "", SketchAction::Dimension)));
        let boundary = if self.all_selected().is_empty() { Err("select an edge or a hole of the sketch first".to_string()) } else { Ok(()) };
        let solid = |label: &str, a: SolidAction| Item::Entry(Entry::new(label, "", UiAction::Editor(EditorAction::Solid(a))).when(boundary.clone()));
        v.push(Item::Separator);
        v.push(solid("Fixed support on selection", SolidAction::SupportOnSelected));
        v.push(solid("Load on selection", SolidAction::LoadOnSelected));
        v.push(Item::Entry(entry("Delete", "Del", SketchAction::DeleteSelected)));
        v.push(Item::Separator);
        v.push(Item::Entry(entry("Finish sketch", "Enter", SketchAction::Finish)));
        v.push(Item::Entry(entry("Cancel sketch", "", SketchAction::Cancel)));
        v
    }
}

pub(super) const T_HISTORY: u32 = 6;
pub(super) const T_HISTORY_BASE: u32 = 20_000;

/// Trim trailing zeros: 12 -> "12", 0.25 -> "0.25".
pub(super) fn format_number(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
