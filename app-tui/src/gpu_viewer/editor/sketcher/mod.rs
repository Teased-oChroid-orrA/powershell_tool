//! Interaction core of the native Sketch mode: tools, snapping (points, entities, horizontal / vertical
//! alignment, grid), rubber-band preview, auto-constraints and dimension entry over a
//! `fea_problem::sketch::Sketch`. Pure: positions are in the sketch plane, the window glue maps pixels to
//! the plane (`solid/sketching.rs`). Every mutation re-solves and rolls back when the result would be
//! conflicting, so the sketch never holds contradictory constraints.
use fea_problem::sketch::{
    infer::{Rule, Suggestion, Tolerance},
    solve::{Report, Status},
    Constraint, Entity, Id, Sketch,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Select,
    Line,
    Polyline,
    Rectangle,
    Circle,
    Arc,
    Spline,
    Dimension,
}

impl Tool {
    pub const ALL: [Tool; 8] = [Tool::Select, Tool::Line, Tool::Polyline, Tool::Rectangle, Tool::Circle, Tool::Arc, Tool::Spline, Tool::Dimension];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Polyline => "Polyline",
            Tool::Rectangle => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Arc => "Arc",
            Tool::Spline => "Spline",
            Tool::Dimension => "Dimension",
        }
    }
    /// The hint shown while the tool waits for its first click.
    pub fn first_hint(self) -> &'static str {
        match self {
            Tool::Select => "Click to select; Shift adds; Delete removes; drag a point to move it.",
            Tool::Line => "Line: click the start point.",
            Tool::Polyline => "Polyline: click the start point; click the first point again, or press Esc, to finish.",
            Tool::Rectangle => "Rectangle: click the first corner.",
            Tool::Circle => "Circle: click the centre.",
            Tool::Arc => "Arc: click the centre, then the start point, then the end point (counter-clockwise).",
            Tool::Spline => "Spline: click the points it passes through; click the last point again, or press Enter, to finish.",
            Tool::Dimension => "Dimension: select a line, a circle / arc, two points or two lines.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapKind {
    /// An existing point (the new geometry shares it).
    Point(Id),
    /// On a line (extended) or circle: adds a `PointOn`.
    OnEntity(Id),
    /// Same y as the anchor / same x as the anchor.
    Horizontal,
    Vertical,
    Grid,
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub at: [f64; 2],
    pub kind: SnapKind,
}

/// A click already made for the active tool.
#[derive(Clone, Copy, Debug)]
struct Placed {
    at: [f64; 2],
    point: Option<Id>,
}

/// What the user can dimension with the current selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DimKind {
    Length(Id),
    Radius(Id),
    Distance(Id, Id),
    Angle(Id, Id),
}

impl DimKind {
    pub fn label(self) -> &'static str {
        match self {
            DimKind::Length(_) => "length",
            DimKind::Radius(_) => "radius",
            DimKind::Distance(..) => "distance",
            DimKind::Angle(..) => "angle (degrees)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EntityKind {
    Line,
    Round,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dof {
    Zero,
    Few,
    Many,
}
/// Coarse, content-free state a constraint ranker learns from (no coordinates, ids or values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConstraintSituation {
    pub tool: Tool,
    pub entity: EntityKind,
    pub dof: Dof,
    /// The sketch already has constraints (not the first thing drawn).
    pub constrained: bool,
}
/// One offer of constraint suggestions and what became of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintEvent {
    pub situation: ConstraintSituation,
    /// Distinct rules in the order they were shown.
    pub offered: Vec<Rule>,
    pub taken: Option<Rule>,
}

/// Line segments to draw as the rubber band of the active tool.
pub type Preview = Vec<([f64; 2], [f64; 2])>;

#[derive(Clone, Debug, Default)]
pub struct SketchEdit {
    pub tool: Tool,
    /// Grid step in model units (0 = none) and pick distance, both set from the view by the glue.
    pub grid: f64,
    pub pick: f64,
    placed: Vec<Placed>,
    pub hover: Option<Snap>,
    pub selection: BTreeSet<Id>,
    pub status: String,
    /// Constraints the last drawn entity already satisfies, best first (apply with `accept`).
    pub suggestions: Vec<Suggestion>,
    pub last_report: Option<ReportSummary>,
    /// The suggestions are in a learned order (else the rules' order).
    pub learned_order: bool,
    /// Situation of the offer currently shown, for recording what happens to it.
    offer: Option<ConstraintSituation>,
    events: Vec<ConstraintEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportSummary {
    pub status: Status,
    pub dof: usize,
}

impl From<&Report> for ReportSummary {
    fn from(r: &Report) -> Self {
        Self { status: r.status, dof: r.dof }
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl SketchEdit {
    pub fn new() -> Self {
        Self { pick: 0.1, grid: 0.0, status: Tool::Select.first_hint().into(), ..Self::default() }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.close_offer(None);
        self.suggestions.clear();
        self.tool = tool;
        self.placed.clear();
        self.hover = None;
        self.suggestions.clear();
        // Select and Dimension work on the current selection; drawing tools start clean.
        if !matches!(tool, Tool::Select | Tool::Dimension) {
            self.selection.clear();
        }
        self.status = tool.first_hint().into();
    }

    /// A tool is mid-gesture (Esc cancels it instead of leaving the sketch).
    pub fn busy(&self) -> bool {
        !self.placed.is_empty()
    }

    pub fn cancel(&mut self) {
        self.placed.clear();
        self.status = self.tool.first_hint().into();
    }

    /// Where the pointer lands after snapping. `anchor` (the last placed point) enables alignment snaps.
    pub fn snap(&self, sk: &Sketch, raw: [f64; 2]) -> Snap {
        let pick = self.pick.max(1e-12);
        // 1. Existing point.
        let near_point = sk
            .entities
            .iter()
            .filter_map(|e| if let Entity::Point { x, y } = e.v { Some((e.id, [x, y])) } else { None })
            .map(|(id, p)| (id, p, dist(p, raw)))
            .filter(|c| c.2 <= pick)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((id, p, _)) = near_point {
            return Snap { at: p, kind: SnapKind::Point(id) };
        }
        // 2. On a line / circle / arc: the closest foot point.
        let mut best: Option<(f64, Snap)> = None;
        for e in &sk.entities {
            let foot = match e.v {
                Entity::Line { .. } => sk.line_foot(e.id, raw),
                Entity::Circle { .. } | Entity::Arc { .. } => sk.round(e.id).filter(|(_, r)| *r > 1e-12).map(|(c, r)| {
                    let d = dist(raw, c).max(1e-300);
                    [c[0] + (raw[0] - c[0]) * r / d, c[1] + (raw[1] - c[1]) * r / d]
                }),
                _ => None,
            };
            if let Some(f) = foot {
                let d = dist(f, raw);
                if d <= pick && best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, Snap { at: f, kind: SnapKind::OnEntity(e.id) }));
                }
            }
        }
        if let Some((_, s)) = best {
            return s;
        }
        // 3. Alignment with the last placed point.
        if let Some(a) = self.placed.last() {
            let (dx, dy) = ((raw[0] - a.at[0]).abs(), (raw[1] - a.at[1]).abs());
            if dy <= pick && dy <= dx {
                return Snap { at: [raw[0], a.at[1]], kind: SnapKind::Horizontal };
            }
            if dx <= pick && dx < dy {
                return Snap { at: [a.at[0], raw[1]], kind: SnapKind::Vertical };
            }
        }
        // 4. Grid.
        if self.grid > 0.0 {
            let g = |v: f64| (v / self.grid).round() * self.grid;
            return Snap { at: [g(raw[0]), g(raw[1])], kind: SnapKind::Grid };
        }
        Snap { at: raw, kind: SnapKind::Free }
    }

    pub fn hover_at(&mut self, sk: &Sketch, raw: [f64; 2]) {
        self.hover = Some(self.snap(sk, raw));
    }

    /// Rubber band of the tool in progress, ending at the hovered (snapped) position.
    pub fn preview(&self) -> Preview {
        let (Some(h), Some(a)) = (self.hover, self.placed.first()) else { return Vec::new() };
        let seg = |p: [f64; 2], q: [f64; 2]| (p, q);
        match self.tool {
            Tool::Line | Tool::Polyline => vec![seg(self.placed.last().expect("non-empty").at, h.at)],
            Tool::Rectangle => {
                let (p, q) = (a.at, h.at);
                vec![seg(p, [q[0], p[1]]), seg([q[0], p[1]], q), seg(q, [p[0], q[1]]), seg([p[0], q[1]], p)]
            }
            Tool::Circle => circle_segments(a.at, dist(a.at, h.at)),
            Tool::Arc if self.placed.len() == 1 => vec![seg(a.at, h.at)],
            Tool::Arc => {
                let (c, s) = (self.placed[0].at, self.placed[1].at);
                let r = dist(c, s);
                let (a0, a1) = (angle(c, s), angle(c, h.at));
                arc_segments(c, r, a0, a1)
            }
            Tool::Spline => {
                let mut pts: Vec<[f64; 2]> = self.placed.iter().map(|p| p.at).collect();
                if pts.last().is_none_or(|l| dist(*l, h.at) > 1e-9) {
                    pts.push(h.at);
                }
                let line = fea_problem::Seg::spline_polyline(&pts, 12);
                line.windows(2).map(|w| (w[0], w[1])).collect()
            }
            Tool::Select | Tool::Dimension => Vec::new(),
        }
    }

    // ----------------------------------------------------------------------------- drawing

    /// A click in the plane. Returns the ids of the entities it created.
    pub fn click(&mut self, sk: &mut Sketch, raw: [f64; 2]) -> Result<Vec<Id>, String> {
        let snap = self.snap(sk, raw);
        match self.tool {
            Tool::Select | Tool::Dimension => {
                self.select_at(sk, raw, false);
                Ok(Vec::new())
            }
            Tool::Line | Tool::Polyline => self.click_line(sk, snap),
            Tool::Rectangle => self.click_rectangle(sk, snap),
            Tool::Circle => self.click_circle(sk, snap),
            Tool::Arc => self.click_arc(sk, snap),
            Tool::Spline => self.click_spline(sk, snap),
        }
    }

    fn placed_of(snap: &Snap) -> Placed {
        Placed { at: snap.at, point: if let SnapKind::Point(id) = snap.kind { Some(id) } else { None } }
    }

    /// The point entity for a placed click: the shared one, or a new one (with `PointOn` when snapped to an entity).
    fn realize(sk: &mut Sketch, p: &Placed, snap_kind: Option<SnapKind>) -> Id {
        if let Some(id) = p.point {
            return id;
        }
        let id = sk.add_point(p.at[0], p.at[1]);
        if let Some(SnapKind::OnEntity(on)) = snap_kind {
            let _ = sk.add_constraint(Constraint::PointOn { p: id, on });
        }
        id
    }

    fn click_line(&mut self, sk: &mut Sketch, snap: Snap) -> Result<Vec<Id>, String> {
        if self.placed.is_empty() {
            self.placed.push(Self::placed_of(&snap));
            self.status = "Click the end point (Esc finishes).".into();
            return Ok(Vec::new());
        }
        let start = *self.placed.last().expect("non-empty");
        if dist(start.at, snap.at) < 1e-9 {
            return Err("a line needs two different points".into());
        }
        let backup = sk.clone();
        let a = Self::realize(sk, &start, None);
        // The start point of a chained line was realized by the previous click; remember it.
        let b = Self::realize(sk, &Self::placed_of(&snap), Some(snap.kind));
        let line = sk.add_line(a, b);
        match snap.kind {
            SnapKind::Horizontal => {
                let _ = sk.add_constraint(Constraint::Horizontal { line });
            }
            SnapKind::Vertical => {
                let _ = sk.add_constraint(Constraint::Vertical { line });
            }
            _ => {}
        }
        if self.settle(sk, backup, line).is_err() {
            return Err(self.status.clone());
        }
        let first = self.placed.first().and_then(|p| p.point).unwrap_or(a);
        let closed = matches!(snap.kind, SnapKind::Point(id) if id == first);
        if self.tool == Tool::Polyline && !closed {
            self.placed = vec![Placed { at: self.placed[0].at, point: Some(first) }, Placed { at: snap.at, point: Some(b) }];
            self.status = "Click the next point; click the first point or press Esc to finish.".into();
        } else {
            self.placed.clear();
            self.status = format!("Line added. {}", self.tool.first_hint());
        }
        self.suggest_for(sk, line);
        Ok(vec![line])
    }

    fn click_rectangle(&mut self, sk: &mut Sketch, snap: Snap) -> Result<Vec<Id>, String> {
        if self.placed.is_empty() {
            self.placed.push(Self::placed_of(&snap));
            self.status = "Click the opposite corner.".into();
            return Ok(Vec::new());
        }
        let (p, q) = (self.placed[0], snap.at);
        if (p.at[0] - q[0]).abs() < 1e-9 || (p.at[1] - q[1]).abs() < 1e-9 {
            return Err("a rectangle needs a width and a height".into());
        }
        let backup = sk.clone();
        let c0 = Self::realize(sk, &p, None);
        let c2 = Self::realize(sk, &Self::placed_of(&snap), Some(snap.kind));
        let c1 = sk.add_point(q[0], p.at[1]);
        let c3 = sk.add_point(p.at[0], q[1]);
        let lines = [sk.add_line(c0, c1), sk.add_line(c1, c2), sk.add_line(c2, c3), sk.add_line(c3, c0)];
        for (i, l) in lines.iter().enumerate() {
            let c = if i % 2 == 0 { Constraint::Horizontal { line: *l } } else { Constraint::Vertical { line: *l } };
            let _ = sk.add_constraint(c);
        }
        self.settle(sk, backup, lines[0])?;
        self.placed.clear();
        self.close_offer(None);
        self.suggestions.clear();
        self.status = "Rectangle added (horizontal / vertical constraints applied). Dimension it with the Dimension tool.".into();
        Ok(lines.to_vec())
    }

    fn click_circle(&mut self, sk: &mut Sketch, snap: Snap) -> Result<Vec<Id>, String> {
        if self.placed.is_empty() {
            self.placed.push(Self::placed_of(&snap));
            self.status = "Click a point on the circle (or its radius).".into();
            return Ok(Vec::new());
        }
        let c = self.placed[0];
        let r = dist(c.at, snap.at);
        if r < 1e-9 {
            return Err("a circle needs a radius".into());
        }
        let backup = sk.clone();
        let cp = Self::realize(sk, &c, None);
        let id = sk.add_circle_about(cp, r);
        self.settle(sk, backup, id)?;
        self.placed.clear();
        self.status = "Circle added.".into();
        self.suggest_for(sk, id);
        Ok(vec![id])
    }

    fn click_arc(&mut self, sk: &mut Sketch, snap: Snap) -> Result<Vec<Id>, String> {
        match self.placed.len() {
            0 => {
                self.placed.push(Self::placed_of(&snap));
                self.status = "Click the start point (sets the radius).".into();
                Ok(Vec::new())
            }
            1 => {
                if dist(self.placed[0].at, snap.at) < 1e-9 {
                    return Err("an arc needs a radius".into());
                }
                self.placed.push(Self::placed_of(&snap));
                self.status = "Click the end point (counter-clockwise from the start).".into();
                Ok(Vec::new())
            }
            _ => {
                let (c, s) = (self.placed[0], self.placed[1]);
                let r = dist(c.at, s.at);
                let a_end = angle(c.at, snap.at);
                if (a_end - angle(c.at, s.at)).rem_euclid(std::f64::consts::TAU) < 1e-6 {
                    return Err("the end point must differ from the start point".into());
                }
                let backup = sk.clone();
                let cp = Self::realize(sk, &c, None);
                let sp = Self::realize(sk, &s, None);
                let e = sk.add_point(c.at[0] + r * a_end.cos(), c.at[1] + r * a_end.sin());
                let arc = sk.add_arc(cp, sp, e);
                self.settle(sk, backup, arc)?;
                self.placed.clear();
                self.status = "Arc added.".into();
                self.suggest_for(sk, arc);
                Ok(vec![arc])
            }
        }
    }

    fn click_spline(&mut self, sk: &mut Sketch, snap: Snap) -> Result<Vec<Id>, String> {
        // Clicking the last point again ends the spline (a double click).
        if self.placed.last().is_some_and(|l| dist(l.at, snap.at) <= 0.5 * self.pick.max(1e-9)) {
            return self.finish_spline(sk);
        }
        self.placed.push(Self::placed_of(&snap));
        self.status = format!("{} points; click the next, or the last again / press Enter to finish.", self.placed.len());
        Ok(Vec::new())
    }

    /// Make the spline through the points clicked so far (Enter, or a second click on the last point).
    pub fn finish_spline(&mut self, sk: &mut Sketch) -> Result<Vec<Id>, String> {
        if self.placed.len() < 2 {
            self.status = "A spline needs at least two points.".into();
            return Err(self.status.clone());
        }
        let backup = sk.clone();
        let placed = std::mem::take(&mut self.placed);
        let ids: Vec<Id> = placed.iter().map(|p| Self::realize(sk, p, None)).collect();
        let Some(spline) = sk.add_spline(ids) else {
            *sk = backup;
            self.status = "That is not a spline.".into();
            return Err(self.status.clone());
        };
        self.settle(sk, backup, spline)?;
        self.status = "Spline added.".into();
        self.suggest_for(sk, spline);
        Ok(vec![spline])
    }

    /// Re-solve after an edit; restore `backup` and explain when the result is conflicting or fails.
    fn settle(&mut self, sk: &mut Sketch, backup: Sketch, touched: Id) -> Result<(), String> {
        let r = sk.solve();
        if r.status == Status::Conflicting {
            *sk = backup;
            self.status = "That would contradict existing constraints; nothing was added.".into();
            return Err(self.status.clone());
        }
        let _ = touched;
        self.last_report = Some((&r).into());
        Ok(())
    }

    fn suggest_for(&mut self, sk: &Sketch, id: Id) {
        self.close_offer(None);
        let extent = sk.extent().max(self.pick * 10.0);
        self.suggestions = sk.suggest(id, &Tolerance::for_extent(extent));
        self.learned_order = false;
        self.offer = (!self.suggestions.is_empty()).then(|| ConstraintSituation {
            tool: self.tool,
            entity: if matches!(sk.entity(id), Some(Entity::Line { .. })) { EntityKind::Line } else { EntityKind::Round },
            dof: match self.last_report.map(|r| r.dof) {
                Some(0) => Dof::Zero,
                Some(1 | 2) => Dof::Few,
                _ => Dof::Many,
            },
            constrained: !sk.constraints.is_empty(),
        });
    }

    /// Distinct rules of the current suggestions, in the order shown.
    pub fn offered_rules(&self) -> Vec<Rule> {
        let mut v: Vec<Rule> = Vec::new();
        for s in &self.suggestions {
            if !v.contains(&s.rule) {
                v.push(s.rule);
            }
        }
        v
    }

    /// Situation of the shown offer, for a ranker.
    pub fn situation(&self) -> Option<ConstraintSituation> {
        self.offer
    }

    /// Show the suggestions in `order` (rules not listed keep their place at the end).
    pub fn reorder(&mut self, order: &[Rule]) {
        let pos = |r: Rule| order.iter().position(|o| *o == r).unwrap_or(usize::MAX);
        self.suggestions.sort_by_key(|s| pos(s.rule));
        self.learned_order = true;
    }

    /// The offer ends: record it (`taken` the rule applied, `None` when it was ignored).
    fn close_offer(&mut self, taken: Option<Rule>) {
        if let Some(situation) = self.offer.take() {
            let offered = self.offered_rules();
            if !offered.is_empty() {
                self.events.push(ConstraintEvent { situation, offered, taken });
            }
        }
    }

    /// Events since the last call (the glue records them when recording is on).
    pub fn take_events(&mut self) -> Vec<ConstraintEvent> {
        std::mem::take(&mut self.events)
    }

    /// Apply suggestion `index` of the current list.
    pub fn accept(&mut self, sk: &mut Sketch, index: usize) -> Result<(), String> {
        let Some(s) = self.suggestions.get(index).cloned() else { return Err("no such suggestion".into()) };
        let backup = sk.clone();
        if sk.add_constraint(s.constraint.clone()).is_none() {
            return Err("that constraint does not apply".into());
        }
        self.settle(sk, backup, 0)?;
        // The offer (as shown) is closed with this choice; what remains is shown as a fresh offer.
        let before = self.offered_rules();
        if let Some(situation) = self.offer {
            self.events.push(ConstraintEvent { situation, offered: before, taken: Some(s.rule) });
        }
        self.suggestions.remove(index);
        self.status = format!("{} applied.", s.constraint.label());
        Ok(())
    }

    /// The constraint `kind` the current selection defines, or why it cannot (the single source for the menu's
    /// enabled state and for applying it).
    pub fn selection_constraint(&self, sk: &Sketch, kind: Rule) -> Result<Constraint, String> {
        let sel: Vec<Id> = self.selection.iter().copied().collect();
        let c = match (kind, sel.as_slice()) {
            (Rule::Horizontal, [l]) => Constraint::Horizontal { line: *l },
            (Rule::Vertical, [l]) => Constraint::Vertical { line: *l },
            (Rule::Coincident, [a, b]) => Constraint::Coincident { a: *a, b: *b },
            (Rule::Parallel, [a, b]) => Constraint::Parallel { a: *a, b: *b },
            (Rule::Perpendicular, [a, b]) => Constraint::Perpendicular { a: *a, b: *b },
            (Rule::Equal, [a, b]) => Constraint::Equal { a: *a, b: *b },
            (Rule::Tangent, [a, b]) => Constraint::Tangent { a: *a, b: *b },
            // Either order works: the point is whichever entity is a point.
            (Rule::PointOn, [p, on]) => {
                if sk.point(*p).is_some() { Constraint::PointOn { p: *p, on: *on } } else { Constraint::PointOn { p: *on, on: *p } }
            }
            (Rule::Midpoint, [p, l]) => {
                if sk.point(*p).is_some() { Constraint::Midpoint { p: *p, line: *l } } else { Constraint::Midpoint { p: *l, line: *p } }
            }
            _ => return Err(format!("select {} first", needs(kind))),
        };
        if sk.accepts(&c) { Ok(c) } else { Err(format!("{} does not apply to this selection (needs {})", c.label(), needs(kind))) }
    }

    /// Add a constraint on the current selection, e.g. Horizontal on a selected line.
    pub fn constrain_selected(&mut self, sk: &mut Sketch, kind: Rule) -> Result<(), String> {
        let c = self.selection_constraint(sk, kind)?;
        let backup = sk.clone();
        sk.add_constraint(c.clone());
        self.settle(sk, backup, 0)?;
        self.status = format!("{} applied.", c.label());
        Ok(())
    }

    /// Points of the selection that are not yet fixed in place.
    pub fn fixable(&self, sk: &Sketch) -> Vec<Id> {
        self.selection
            .iter()
            .copied()
            .filter(|id| sk.point(*id).is_some() && !sk.constraints.iter().any(|c| matches!(c.v, Constraint::Fixed { p, .. } if p == *id)))
            .collect()
    }

    /// Pin the selected points where they are.
    pub fn fix_selected(&mut self, sk: &mut Sketch) -> Result<usize, String> {
        let pts = self.fixable(sk);
        if pts.is_empty() {
            return Err("select a point that is not fixed yet".into());
        }
        let backup = sk.clone();
        for p in &pts {
            let [x, y] = sk.point(*p).expect("point");
            sk.add_constraint(Constraint::Fixed { p: *p, x, y });
        }
        self.settle(sk, backup, 0)?;
        self.status = format!("{} point(s) fixed.", pts.len());
        Ok(pts.len())
    }

    // ----------------------------------------------------------------------------- selection

    /// The entity nearest to `at` within the pick distance: points first, then lines, then circles / arcs.
    pub fn pick_entity(&self, sk: &Sketch, at: [f64; 2]) -> Option<Id> {
        let pick = self.pick;
        let pt = sk
            .entities
            .iter()
            .filter_map(|e| if let Entity::Point { x, y } = e.v { Some((e.id, dist([x, y], at))) } else { None })
            .filter(|c| c.1 <= pick)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = pt {
            return Some(id);
        }
        sk.entities
            .iter()
            .filter_map(|e| {
                let d = match e.v {
                    Entity::Line { .. } => sk.line_foot(e.id, at).map(|f| dist(f, at)),
                    Entity::Circle { .. } => sk.round(e.id).map(|(c, r)| (dist(c, at) - r).abs()),
                    Entity::Arc { .. } => sk.round(e.id).filter(|(c, _)| arc_contains(sk, e.id, angle(*c, at))).map(|(c, r)| (dist(c, at) - r).abs()),
                    Entity::Spline { .. } => sk.spline_polyline(e.id, 12).map(|l| l.windows(2).map(|w| segment_distance(w[0], w[1], at)).fold(f64::INFINITY, f64::min)),
                    Entity::Point { .. } => None,
                };
                d.filter(|d| *d <= pick).map(|d| (e.id, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|c| c.0)
    }

    pub fn select_at(&mut self, sk: &Sketch, at: [f64; 2], additive: bool) {
        let hit = self.pick_entity(sk, at);
        match (hit, additive) {
            (Some(id), true) => {
                if !self.selection.remove(&id) {
                    self.selection.insert(id);
                }
            }
            (Some(id), false) => {
                self.selection.clear();
                self.selection.insert(id);
            }
            (None, false) => self.selection.clear(),
            (None, true) => {}
        }
        self.status = match self.dimension_kind(sk) {
            Some(k) if self.tool == Tool::Dimension => format!("Type the {} and press Enter.", k.label()),
            _ => self.describe_selection(sk),
        };
    }

    /// A click of the Dimension tool: it replaces the selection, except that a second point joins a first one
    /// (distance) and Shift-click adds (e.g. a second line for the angle).
    pub fn dimension_select(&mut self, sk: &Sketch, at: [f64; 2], additive: bool) {
        let hit = self.pick_entity(sk, at);
        let lone_point = self.selection.len() == 1 && self.selection.iter().all(|id| sk.point(*id).is_some());
        let join = additive || (lone_point && hit.is_some_and(|h| sk.point(h).is_some()));
        self.select_at(sk, at, join);
    }

    pub fn select_box(&mut self, sk: &Sketch, a: [f64; 2], b: [f64; 2], additive: bool) {
        let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
        if !additive {
            self.selection.clear();
        }
        for e in &sk.entities {
            let inside = |p: [f64; 2]| p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1];
            let hit = match e.v {
                Entity::Point { x, y } => inside([x, y]),
                Entity::Line { .. } => sk.line_pts(e.id).is_some_and(|(p, q)| inside(p) && inside(q)),
                Entity::Circle { .. } => sk.round(e.id).is_some_and(|(c, r)| inside([c[0] - r, c[1] - r]) && inside([c[0] + r, c[1] + r])),
                Entity::Arc { c, s, e: en } => [c, s, en].iter().all(|p| sk.point(*p).is_some_and(inside)),
                Entity::Spline { ref pts } => pts.iter().all(|p| sk.point(*p).is_some_and(inside)),
            };
            if hit {
                self.selection.insert(e.id);
            }
        }
        self.status = self.describe_selection(sk);
    }

    fn describe_selection(&self, sk: &Sketch) -> String {
        match self.selection.len() {
            0 => self.tool.first_hint().into(),
            1 => {
                let id = *self.selection.iter().next().expect("one");
                match sk.entity(id) {
                    Some(Entity::Point { x, y }) => format!("Point ({x:.4}, {y:.4})"),
                    Some(Entity::Line { .. }) => format!("Line, length {:.4}", sk.measure_line(id).unwrap_or(0.0)),
                    Some(Entity::Circle { .. }) | Some(Entity::Arc { .. }) => format!("Radius {:.4}", sk.round(id).map_or(0.0, |r| r.1)),
                    Some(Entity::Spline { pts }) => format!("Spline through {} points (dimension its points)", pts.len()),
                    None => String::new(),
                }
            }
            n => format!("{n} selected"),
        }
    }

    /// Delete the selection (and what depends on it).
    pub fn delete_selected(&mut self, sk: &mut Sketch) -> usize {
        let n = self.selection.len();
        for id in std::mem::take(&mut self.selection) {
            sk.remove_entity(id);
        }
        // Points no entity uses any more are removed too (they are invisible clutter otherwise).
        sk.remove_orphan_points();
        let r = sk.solve();
        self.last_report = Some((&r).into());
        self.close_offer(None);
        self.suggestions.clear();
        n
    }

    // ----------------------------------------------------------------------------- dimensions

    /// What the selection can be dimensioned as.
    pub fn dimension_kind(&self, sk: &Sketch) -> Option<DimKind> {
        let sel: Vec<Id> = self.selection.iter().copied().collect();
        let kind = |id: Id| sk.entity(id).cloned();
        match sel.as_slice() {
            [a] => match kind(*a)? {
                Entity::Line { .. } => Some(DimKind::Length(*a)),
                Entity::Circle { .. } | Entity::Arc { .. } => Some(DimKind::Radius(*a)),
                Entity::Point { .. } | Entity::Spline { .. } => None,
            },
            [a, b] => match (kind(*a)?, kind(*b)?) {
                (Entity::Point { .. }, Entity::Point { .. }) => Some(DimKind::Distance(*a, *b)),
                (Entity::Line { .. }, Entity::Line { .. }) => Some(DimKind::Angle(*a, *b)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Current value of the dimension the selection defines (the default shown in the entry box).
    pub fn dimension_value(&self, sk: &Sketch) -> Option<f64> {
        Some(match self.dimension_kind(sk)? {
            DimKind::Length(l) => sk.measure_line(l)?,
            DimKind::Radius(c) => sk.round(c)?.1,
            DimKind::Distance(a, b) => dist(sk.point(a)?, sk.point(b)?),
            DimKind::Angle(a, b) => sk.measure_angle(a, b)?,
        })
    }

    /// Add the driving dimension for the selection with the typed value and re-solve (rolled back on conflict).
    pub fn set_dimension(&mut self, sk: &mut Sketch, value: f64) -> Result<(), String> {
        let kind = self.dimension_kind(sk).ok_or("select what to dimension first")?;
        if !value.is_finite() {
            return Err("the value must be a number".into());
        }
        if !matches!(kind, DimKind::Angle(..)) && value <= 0.0 {
            return Err("a dimension must be positive".into());
        }
        let backup = sk.clone();
        // Replace an existing dimension of the same kind on the same entities (editing it).
        let c = match kind {
            DimKind::Length(line) => Constraint::Length { line, d: value },
            DimKind::Radius(e) => Constraint::Radius { e, d: value },
            DimKind::Distance(a, b) => Constraint::Distance { a, b, d: value },
            DimKind::Angle(a, b) => Constraint::Angle { a, b, deg: value },
        };
        let same: Vec<Id> = sk.constraints.iter().filter(|k| k.v.is_dimension() && std::mem::discriminant(&k.v) == std::mem::discriminant(&c) && k.v.refs() == c.refs()).map(|k| k.id).collect();
        for id in same {
            sk.remove_constraint(id);
        }
        if sk.add_constraint(c.clone()).is_none() {
            return Err("that dimension does not apply".into());
        }
        let r = sk.solve();
        if r.status == Status::Conflicting {
            *sk = backup;
            return Err("that value contradicts the existing constraints (or over-defines the sketch); nothing changed".into());
        }
        self.last_report = Some((&r).into());
        self.status = format!("{} = {value} set.", c.label());
        Ok(())
    }

    // ----------------------------------------------------------------------------- dragging

    /// Move the entity's points toward `to` (a drag): constraints stay satisfied.
    pub fn drag(&mut self, sk: &mut Sketch, id: Id, to: [f64; 2]) -> Report {
        let pulls: Vec<(Id, [f64; 2])> = match sk.entity(id) {
            Some(Entity::Point { .. }) => vec![(id, to)],
            Some(Entity::Circle { c, .. }) => vec![(*c, to)],
            _ => Vec::new(),
        };
        let r = sk.drag(&pulls);
        self.last_report = Some((&r).into());
        r
    }
}

fn needs(kind: Rule) -> &'static str {
    match kind {
        Rule::Horizontal | Rule::Vertical => "one line",
        Rule::Coincident => "two points",
        Rule::Parallel | Rule::Perpendicular | Rule::Equal => "two lines (Equal: two lines or two circles)",
        Rule::Tangent => "a line and a circle, or two circles",
        Rule::PointOn => "a point and a line or circle",
        Rule::Midpoint => "a point and a line",
    }
}

fn angle(c: [f64; 2], p: [f64; 2]) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0])
}

fn circle_segments(c: [f64; 2], r: f64) -> Preview {
    arc_segments(c, r, 0.0, std::f64::consts::TAU - 1e-9)
}

/// Counter-clockwise polyline from angle `a0` to `a1` (a full turn when equal).
pub fn arc_segments(c: [f64; 2], r: f64, a0: f64, a1: f64) -> Preview {
    let mut sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
    if sweep < 1e-9 {
        sweep = std::f64::consts::TAU;
    }
    let n = ((sweep / std::f64::consts::TAU * 48.0).ceil() as usize).max(2);
    let pt = |i: usize| {
        let t = a0 + sweep * i as f64 / n as f64;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    };
    (0..n).map(|i| (pt(i), pt(i + 1))).collect()
}

/// Does the arc `id` (ccw start -> end about its centre) include the direction `a`?
fn arc_contains(sk: &Sketch, id: Id, a: f64) -> bool {
    let Some(Entity::Arc { c, s, e }) = sk.entity(id) else { return false };
    let (Some(c), Some(s), Some(e)) = (sk.point(*c), sk.point(*s), sk.point(*e)) else { return false };
    let tau = std::f64::consts::TAU;
    let span = (angle(c, e) - angle(c, s)).rem_euclid(tau);
    (a - angle(c, s)).rem_euclid(tau) <= span + 1e-9
}

/// Tessellation of every sketch entity as segments, for drawing: `(entity id, a, b)`.
pub fn sketch_segments(sk: &Sketch) -> Vec<(Id, [f64; 2], [f64; 2])> {
    let mut out = Vec::new();
    for e in &sk.entities {
        match e.v {
            Entity::Line { .. } => {
                if let Some((a, b)) = sk.line_pts(e.id) {
                    out.push((e.id, a, b));
                }
            }
            Entity::Circle { .. } => {
                if let Some((c, r)) = sk.round(e.id) {
                    out.extend(circle_segments(c, r).into_iter().map(|(a, b)| (e.id, a, b)));
                }
            }
            Entity::Arc { c, s, e: en } => {
                if let (Some(c), Some(s), Some(en)) = (sk.point(c), sk.point(s), sk.point(en)) {
                    out.extend(arc_segments(c, dist(c, s), angle(c, s), angle(c, en)).into_iter().map(|(a, b)| (e.id, a, b)));
                }
            }
            Entity::Spline { .. } => {
                if let Some(l) = sk.spline_polyline(e.id, 16) {
                    out.extend(l.windows(2).map(|w| (e.id, w[0], w[1])));
                }
            }
            Entity::Point { .. } => {}
        }
    }
    out
}

/// Distance from `q` to the segment `a`-`b`.
fn segment_distance(a: [f64; 2], b: [f64; 2], q: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 { (((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(q, [a[0] + t * d[0], a[1] + t * d[1]])
}

#[cfg(test)]
mod tests;
