//! Parametric 2D sketch: entities with stable ids, geometric and dimensional constraints, a small
//! Levenberg-Marquardt solver with degrees-of-freedom reporting, and extraction of the closed profile
//! that features (extrude / revolve) turn into a meshable `Geometry`. Pure: no I/O, no UI.
//! Design: `docs/adr/ADR-015-parametric-sketch-history.md`.
use serde::{Deserialize, Serialize};

pub mod infer;
pub mod profile;
pub mod solve;

pub type Id = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Entity {
    Point { x: f64, y: f64 },
    /// Straight segment between two `Point`s.
    Line { a: Id, b: Id },
    /// Full circle: centre `Point`, radius.
    Circle { c: Id, r: f64 },
    /// Counter-clockwise arc about centre `c` from `s` to `e` (both `Point`s; `|cs| = |ce|` is implicit).
    Arc { c: Id, s: Id, e: Id },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Constraint {
    /// Two points coincide.
    Coincident { a: Id, b: Id },
    Horizontal { line: Id },
    Vertical { line: Id },
    Parallel { a: Id, b: Id },
    Perpendicular { a: Id, b: Id },
    /// Equal length (lines) or equal radius (circles / arcs).
    Equal { a: Id, b: Id },
    /// Line tangent to a circle / arc, or two circles / arcs tangent.
    Tangent { a: Id, b: Id },
    /// A point on a line (extended) or on a circle / arc.
    PointOn { p: Id, on: Id },
    Midpoint { p: Id, line: Id },
    /// Point pinned at a position.
    Fixed { p: Id, x: f64, y: f64 },
    // Dimensions (driving).
    Length { line: Id, d: f64 },
    Distance { a: Id, b: Id, d: f64 },
    HDist { a: Id, b: Id, d: f64 },
    VDist { a: Id, b: Id, d: f64 },
    Radius { e: Id, d: f64 },
    /// Angle in degrees from line `a`'s direction to line `b`'s direction.
    Angle { a: Id, b: Id, deg: f64 },
}

impl Constraint {
    pub fn is_dimension(&self) -> bool {
        matches!(self, Self::Length { .. } | Self::Distance { .. } | Self::HDist { .. } | Self::VDist { .. } | Self::Radius { .. } | Self::Angle { .. })
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Coincident { .. } => "Coincident",
            Self::Horizontal { .. } => "Horizontal",
            Self::Vertical { .. } => "Vertical",
            Self::Parallel { .. } => "Parallel",
            Self::Perpendicular { .. } => "Perpendicular",
            Self::Equal { .. } => "Equal",
            Self::Tangent { .. } => "Tangent",
            Self::PointOn { .. } => "Point on",
            Self::Midpoint { .. } => "Midpoint",
            Self::Fixed { .. } => "Fixed",
            Self::Length { .. } => "Length",
            Self::Distance { .. } => "Distance",
            Self::HDist { .. } => "Horizontal distance",
            Self::VDist { .. } => "Vertical distance",
            Self::Radius { .. } => "Radius",
            Self::Angle { .. } => "Angle",
        }
    }
    /// Ids of the entities the constraint refers to.
    pub fn refs(&self) -> Vec<Id> {
        match *self {
            Self::Coincident { a, b } | Self::Parallel { a, b } | Self::Perpendicular { a, b } | Self::Equal { a, b } | Self::Tangent { a, b } => vec![a, b],
            Self::Distance { a, b, .. } | Self::HDist { a, b, .. } | Self::VDist { a, b, .. } | Self::Angle { a, b, .. } => vec![a, b],
            Self::Horizontal { line } | Self::Vertical { line } | Self::Length { line, .. } => vec![line],
            Self::PointOn { p, on } => vec![p, on],
            Self::Midpoint { p, line } => vec![p, line],
            Self::Fixed { p, .. } => vec![p],
            Self::Radius { e, .. } => vec![e],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item<T> {
    pub id: Id,
    #[serde(flatten)]
    pub v: T,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Sketch {
    #[serde(default)]
    pub next_id: Id,
    pub entities: Vec<Item<Entity>>,
    #[serde(default)]
    pub constraints: Vec<Item<Constraint>>,
    /// Entities drawn only to hold dimensions and constraints (axes, centre lines, reference circles): they take part in
    /// the solve but never in the profile (`profile`, `surface_names`) or its branch / closure checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub construction: Vec<Id>,
}

impl Sketch {
    fn take_id(&mut self) -> Id {
        let id = self.next_id.max(self.entities.iter().map(|e| e.id + 1).chain(self.constraints.iter().map(|c| c.id + 1)).max().unwrap_or(0));
        self.next_id = id + 1;
        id
    }
    pub fn is_construction(&self, id: Id) -> bool {
        self.construction.contains(&id)
    }
    /// Mark a line, circle or arc as construction geometry (or back to real geometry). A point has no profile role, so it
    /// is refused, as is a missing entity.
    pub fn set_construction(&mut self, id: Id, on: bool) -> Result<(), String> {
        match self.entity(id) {
            None => return Err(format!("no entity {id}")),
            Some(Entity::Point { .. }) => return Err("a point is not part of a profile; only lines, circles and arcs can be construction geometry".into()),
            Some(_) => {}
        }
        self.construction.retain(|c| *c != id);
        if on {
            self.construction.push(id);
        }
        Ok(())
    }
    pub fn entity(&self, id: Id) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id).map(|e| &e.v)
    }
    fn entity_mut(&mut self, id: Id) -> Option<&mut Entity> {
        self.entities.iter_mut().find(|e| e.id == id).map(|e| &mut e.v)
    }
    pub fn point(&self, id: Id) -> Option<[f64; 2]> {
        match self.entity(id)? {
            Entity::Point { x, y } => Some([*x, *y]),
            _ => None,
        }
    }
    pub fn add_point(&mut self, x: f64, y: f64) -> Id {
        let id = self.take_id();
        self.entities.push(Item { id, v: Entity::Point { x, y } });
        id
    }
    pub fn add_line(&mut self, a: Id, b: Id) -> Id {
        let id = self.take_id();
        self.entities.push(Item { id, v: Entity::Line { a, b } });
        id
    }
    /// A circle of radius `r` about a new centre point; returns `(circle, centre)`.
    pub fn add_circle(&mut self, x: f64, y: f64, r: f64) -> (Id, Id) {
        let c = self.add_point(x, y);
        let id = self.take_id();
        self.entities.push(Item { id, v: Entity::Circle { c, r } });
        (id, c)
    }
    /// A circle of radius `r` about the existing point `c`.
    pub fn add_circle_about(&mut self, c: Id, r: f64) -> Id {
        let id = self.take_id();
        self.entities.push(Item { id, v: Entity::Circle { c, r } });
        id
    }
    pub fn add_arc(&mut self, c: Id, s: Id, e: Id) -> Id {
        let id = self.take_id();
        self.entities.push(Item { id, v: Entity::Arc { c, s, e } });
        id
    }
    /// Add a constraint; `None` (nothing added) when it names a missing entity or the wrong kind.
    pub fn add_constraint(&mut self, c: Constraint) -> Option<Id> {
        if !self.accepts(&c) {
            return None;
        }
        let id = self.take_id();
        self.constraints.push(Item { id, v: c });
        Some(id)
    }
    pub fn remove_constraint(&mut self, id: Id) -> bool {
        let n = self.constraints.len();
        self.constraints.retain(|c| c.id != id);
        self.constraints.len() != n
    }
    /// Delete an entity and everything that depends on it (lines / arcs through a deleted point,
    /// constraints naming a deleted entity). Points orphaned by the deletion that nothing else uses are kept.
    pub fn remove_entity(&mut self, id: Id) {
        let mut dead = vec![id];
        loop {
            let before = dead.len();
            for e in &self.entities {
                let uses = match e.v {
                    Entity::Line { a, b } => dead.contains(&a) || dead.contains(&b),
                    Entity::Circle { c, .. } => dead.contains(&c),
                    Entity::Arc { c, s, e } => dead.contains(&c) || dead.contains(&s) || dead.contains(&e),
                    Entity::Point { .. } => false,
                };
                if uses && !dead.contains(&e.id) {
                    dead.push(e.id);
                }
            }
            if dead.len() == before {
                break;
            }
        }
        self.entities.retain(|e| !dead.contains(&e.id));
        self.constraints.retain(|c| !c.v.refs().iter().any(|r| dead.contains(r)));
        self.construction.retain(|c| !dead.contains(c));
    }
    fn kind(&self, id: Id) -> Option<u8> {
        Some(match self.entity(id)? {
            Entity::Point { .. } => 0,
            Entity::Line { .. } => 1,
            Entity::Circle { .. } => 2,
            Entity::Arc { .. } => 3,
        })
    }
    /// Does `c` name existing entities of the kinds it needs?
    pub fn accepts(&self, c: &Constraint) -> bool {
        let k = |i: Id| self.kind(i);
        let round = |i: Id| matches!(k(i), Some(2 | 3));
        let line = |i: Id| k(i) == Some(1);
        let pt = |i: Id| k(i) == Some(0);
        match *c {
            Constraint::Coincident { a, b } => pt(a) && pt(b) && a != b,
            Constraint::Horizontal { line: l } | Constraint::Vertical { line: l } | Constraint::Length { line: l, .. } => line(l),
            Constraint::Parallel { a, b } | Constraint::Perpendicular { a, b } | Constraint::Angle { a, b, .. } => line(a) && line(b) && a != b,
            Constraint::Equal { a, b } => a != b && ((line(a) && line(b)) || (round(a) && round(b))),
            Constraint::Tangent { a, b } => a != b && ((line(a) && round(b)) || (round(a) && line(b)) || (round(a) && round(b))),
            Constraint::PointOn { p, on } => pt(p) && (line(on) || round(on)),
            Constraint::Midpoint { p, line: l } => pt(p) && line(l),
            Constraint::Fixed { p, .. } => pt(p),
            Constraint::Distance { a, b, .. } | Constraint::HDist { a, b, .. } | Constraint::VDist { a, b, .. } => pt(a) && pt(b) && a != b,
            Constraint::Radius { e, .. } => round(e),
        }
    }
    /// Endpoints `(a, b)` positions of a line.
    pub fn line_pts(&self, id: Id) -> Option<([f64; 2], [f64; 2])> {
        match self.entity(id)? {
            Entity::Line { a, b } => Some((self.point(*a)?, self.point(*b)?)),
            _ => None,
        }
    }
    /// `(centre, radius)` of a circle or arc.
    pub fn round(&self, id: Id) -> Option<([f64; 2], f64)> {
        match self.entity(id)? {
            Entity::Circle { c, r } => Some((self.point(*c)?, *r)),
            Entity::Arc { c, s, .. } => {
                let (c, s) = (self.point(*c)?, self.point(*s)?);
                Some((c, (s[0] - c[0]).hypot(s[1] - c[1])))
            }
            _ => None,
        }
    }
    /// Foot of the perpendicular from `q` on the line `id`, if it lies on the segment (with a small margin).
    pub fn line_foot(&self, id: Id, q: [f64; 2]) -> Option<[f64; 2]> {
        let (a, b) = self.line_pts(id)?;
        let d = [b[0] - a[0], b[1] - a[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        if l2 < 1e-24 {
            return None;
        }
        let t = ((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / l2;
        (-0.02..=1.02).contains(&t).then(|| [a[0] + t * d[0], a[1] + t * d[1]])
    }
    pub fn measure_line(&self, id: Id) -> Option<f64> {
        let (a, b) = self.line_pts(id)?;
        Some((b[0] - a[0]).hypot(b[1] - a[1]))
    }
    /// Angle in degrees from the direction of line `a` to line `b`, in (-180, 180].
    pub fn measure_angle(&self, a: Id, b: Id) -> Option<f64> {
        let ((a0, a1), (b0, b1)) = (self.line_pts(a)?, self.line_pts(b)?);
        let (d1, d2) = ([a1[0] - a0[0], a1[1] - a0[1]], [b1[0] - b0[0], b1[1] - b0[1]]);
        Some((d1[0] * d2[1] - d1[1] * d2[0]).atan2(d1[0] * d2[0] + d1[1] * d2[1]).to_degrees())
    }
    /// Largest side of the bounding box of the points (and circle extents); 0 for an empty sketch.
    pub fn extent(&self) -> f64 {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        let mut grow = |p: [f64; 2], r: f64| {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k] - r);
                hi[k] = hi[k].max(p[k] + r);
            }
        };
        for e in &self.entities {
            match e.v {
                Entity::Point { x, y } => grow([x, y], 0.0),
                Entity::Circle { .. } => {
                    if let Some((c, r)) = self.round(e.id) {
                        grow(c, r)
                    }
                }
                _ => {}
            }
        }
        if lo[0].is_finite() { (hi[0] - lo[0]).max(hi[1] - lo[1]) } else { 0.0 }
    }
    /// Remove points that no line, circle, arc or constraint refers to.
    pub fn remove_orphan_points(&mut self) {
        let used: Vec<Id> = self
            .entities
            .iter()
            .flat_map(|e| match e.v {
                Entity::Line { a, b } => vec![a, b],
                Entity::Circle { c, .. } => vec![c],
                Entity::Arc { c, s, e } => vec![c, s, e],
                Entity::Point { .. } => vec![],
            })
            .chain(self.constraints.iter().flat_map(|c| c.v.refs()))
            .collect();
        self.entities.retain(|e| !matches!(e.v, Entity::Point { .. }) || used.contains(&e.id));
    }
    /// A sketch equivalent to the legacy outline + holes. Rectangles and circles come fully dimensioned (position,
    /// size); a polygon and a slot are placed points / lines without constraints. Entity order follows the order of
    /// the shapes, so `edgeN` / `holeN` keep their meaning.
    pub fn from_shapes(outer: &crate::Shape, holes: &[crate::Shape]) -> Sketch {
        use crate::Shape;
        let mut s = Sketch::default();
        for shape in std::iter::once(outer).chain(holes) {
            match shape {
                Shape::Rect { x0, y0, x1, y1 } => {
                    let p = [s.add_point(*x0, *y0), s.add_point(*x1, *y0), s.add_point(*x1, *y1), s.add_point(*x0, *y1)];
                    let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[3]), s.add_line(p[3], p[0])];
                    for (i, id) in l.iter().enumerate() {
                        s.add_constraint(if i % 2 == 0 { Constraint::Horizontal { line: *id } } else { Constraint::Vertical { line: *id } });
                    }
                    s.add_constraint(Constraint::Fixed { p: p[0], x: *x0, y: *y0 });
                    s.add_constraint(Constraint::Length { line: l[0], d: x1 - x0 });
                    s.add_constraint(Constraint::Length { line: l[1], d: y1 - y0 });
                }
                Shape::Circle { cx, cy, r } => {
                    let (c, cp) = s.add_circle(*cx, *cy, *r);
                    s.add_constraint(Constraint::Fixed { p: cp, x: *cx, y: *cy });
                    s.add_constraint(Constraint::Radius { e: c, d: *r });
                }
                Shape::Polygon { pts } => {
                    let ids: Vec<Id> = pts.iter().map(|p| s.add_point(p[0], p[1])).collect();
                    for i in 0..ids.len() {
                        s.add_line(ids[i], ids[(i + 1) % ids.len()]);
                    }
                }
                Shape::Outline { segs } => {
                    // One point per edge start, shared with the previous edge's end; an arc also gets its centre.
                    let ids: Vec<Id> = segs.iter().map(|sg| s.add_point(sg.start()[0], sg.start()[1])).collect();
                    for (i, sg) in segs.iter().enumerate() {
                        let (from, to) = (ids[i], ids[(i + 1) % ids.len()]);
                        match *sg {
                            crate::Seg::Line { .. } => {
                                s.add_line(from, to);
                            }
                            crate::Seg::Arc { c, a0, a1, .. } => {
                                let centre = s.add_point(c[0], c[1]);
                                // Arcs are stored counter-clockwise from start to end.
                                if a1 >= a0 { s.add_arc(centre, from, to) } else { s.add_arc(centre, to, from) };
                            }
                        }
                    }
                }
                Shape::Slot { cx, cy, length, width, angle } => {
                    let a = angle.to_radians();
                    let (u, n) = ([a.cos(), a.sin()], [-a.sin(), a.cos()]);
                    let (r, h) = (0.5 * width, 0.5 * (length - width));
                    let (c1, c2) = ([cx - h * u[0], cy - h * u[1]], [cx + h * u[0], cy + h * u[1]]);
                    let off = |c: [f64; 2], k: f64| [c[0] + k * r * n[0], c[1] + k * r * n[1]];
                    let (pc1, pc2) = (s.add_point(c1[0], c1[1]), s.add_point(c2[0], c2[1]));
                    let p = [s.add_point(off(c1, -1.0)[0], off(c1, -1.0)[1]), s.add_point(off(c2, -1.0)[0], off(c2, -1.0)[1]), s.add_point(off(c2, 1.0)[0], off(c2, 1.0)[1]), s.add_point(off(c1, 1.0)[0], off(c1, 1.0)[1])];
                    s.add_line(p[0], p[1]);
                    s.add_arc(pc2, p[1], p[2]);
                    s.add_line(p[2], p[3]);
                    s.add_arc(pc1, p[3], p[0]);
                }
            }
        }
        s
    }
}
