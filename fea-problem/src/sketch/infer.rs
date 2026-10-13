//! Deterministic constraint inference ("auto relations"): which geometric constraints the geometry of a
//! just-drawn entity already satisfies within a tolerance. This is the rule-based baseline of the constraint
//! advisor; a learned ranker may only reorder what this function allows (`docs/issue-12-phase-24.md`).
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub struct Tolerance {
    pub angle_deg: f64,
    /// Model-unit distance for coincidence / point-on / tangency.
    pub dist: f64,
    /// Relative difference for equal length / radius.
    pub ratio: f64,
}

impl Tolerance {
    /// Tolerances from the sketch's visible extent (a pick distance is a fraction of it).
    pub fn for_extent(extent: f64) -> Self {
        Self { angle_deg: 3.0, dist: 0.012 * extent.max(1e-9), ratio: 0.03 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Rule {
    Horizontal,
    Vertical,
    Coincident,
    Parallel,
    Perpendicular,
    Equal,
    Tangent,
    PointOn,
    Midpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub constraint: Constraint,
    pub rule: Rule,
    /// 1 = exactly satisfied now, 0 = at the tolerance edge.
    pub score: f64,
}

fn closeness(dev: f64, tol: f64) -> Option<f64> {
    (dev <= tol).then(|| 1.0 - dev / tol.max(1e-300))
}

fn unordered(a: &Constraint, b: &Constraint) -> bool {
    let (mut ra, mut rb) = (a.refs(), b.refs());
    ra.sort_unstable();
    rb.sort_unstable();
    std::mem::discriminant(a) == std::mem::discriminant(b) && ra == rb
}

/// Angle in degrees between two undirected directions, in [0, 90].
fn line_angle(d1: [f64; 2], d2: [f64; 2]) -> f64 {
    let a = (d1[0] * d2[1] - d1[1] * d2[0]).atan2(d1[0] * d2[0] + d1[1] * d2[1]).abs().to_degrees();
    if a > 90.0 { 180.0 - a } else { a }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl Sketch {
    /// Constraints entity `id` (and, for a line / arc, its end points) already satisfies relative to the rest of
    /// the sketch, best first. Constraints already present are not suggested again.
    pub fn suggest(&self, id: Id, tol: &Tolerance) -> Vec<Suggestion> {
        if self.check().is_err() {
            return Vec::new();
        }
        let mut out: Vec<Suggestion> = Vec::new();
        let mut push = |this: &Sketch, c: Constraint, rule: Rule, score: Option<f64>| {
            let Some(score) = score else { return };
            if this.constraints.iter().any(|k| unordered(&k.v, &c)) || out.iter().any(|s| unordered(&s.constraint, &c)) {
                return;
            }
            out.push(Suggestion { constraint: c, rule, score });
        };
        let Some(ent) = self.entity(id) else { return out };
        // Points that take part (the entity's own end points, or the point itself).
        let mut my_points: Vec<Id> = match ent {
            Entity::Point { .. } => vec![id],
            Entity::Line { a, b } => vec![*a, *b],
            Entity::Arc { s, e, .. } => vec![*s, *e],
            Entity::Spline { pts } => vec![pts[0], pts[pts.len() - 1]],
            Entity::Circle { .. } => Vec::new(),
        };
        if let Entity::Circle { c, .. } | Entity::Arc { c, .. } = ent.clone() {
            my_points.push(c);
        }
        let merged = |a: Id, b: Id| -> bool {
            a == b || self.constraints.iter().any(|k| matches!(k.v, Constraint::Coincident { a: x, b: y } if (x == a && y == b) || (x == b && y == a)))
        };
        // Point relations.
        for &p in &my_points {
            let Some(pp) = self.point(p) else { continue };
            for other in &self.entities {
                if other.id == id || my_points.contains(&other.id) && !matches!(ent, Entity::Point { .. }) && other.id == p {
                    continue;
                }
                match other.v.clone() {
                    Entity::Point { x, y } if other.id != p && !merged(p, other.id) => {
                        push(self, Constraint::Coincident { a: p, b: other.id }, Rule::Coincident, closeness(dist(pp, [x, y]), tol.dist));
                    }
                    Entity::Line { a, b } if !my_points.contains(&a) && !my_points.contains(&b) && !matches!(ent, Entity::Line { a: x, b: y } if (*x == a || *y == a || *x == b || *y == b)) => {
                        if let Some((p0, p1)) = self.line_pts(other.id) {
                            let d = [p1[0] - p0[0], p1[1] - p0[1]];
                            let len = d[0].hypot(d[1]);
                            if len > 1e-12 {
                                let off = ((d[0] * (pp[1] - p0[1]) - d[1] * (pp[0] - p0[0])) / len).abs();
                                let t = ((pp[0] - p0[0]) * d[0] + (pp[1] - p0[1]) * d[1]) / (len * len);
                                if (-0.0..=1.0).contains(&t) {
                                    let mid = dist(pp, [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1])]);
                                    if matches!(ent, Entity::Point { .. }) {
                                        push(self, Constraint::Midpoint { p, line: other.id }, Rule::Midpoint, closeness(mid, tol.dist));
                                    }
                                    push(self, Constraint::PointOn { p, on: other.id }, Rule::PointOn, closeness(off, tol.dist));
                                }
                            }
                        }
                    }
                    Entity::Circle { .. } | Entity::Arc { .. } if !matches!(ent, Entity::Circle { .. } | Entity::Arc { .. }) || !my_points.contains(&match other.v { Entity::Circle { c, .. } | Entity::Arc { c, .. } => c, _ => 0 }) => {
                        if let Some((c, r)) = self.round(other.id) {
                            let owns = matches!(other.v, Entity::Arc { s, e, .. } if s == p || e == p);
                            if !owns {
                                push(self, Constraint::PointOn { p, on: other.id }, Rule::PointOn, closeness((dist(pp, c) - r).abs(), tol.dist));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Whole-entity relations.
        match ent.clone() {
            Entity::Line { .. } => {
                let (p0, p1) = self.line_pts(id).expect("line");
                let d = [p1[0] - p0[0], p1[1] - p0[1]];
                let len = d[0].hypot(d[1]);
                if len > 1e-12 {
                    let to_h = line_angle(d, [1.0, 0.0]);
                    push(self, Constraint::Horizontal { line: id }, Rule::Horizontal, closeness(to_h, tol.angle_deg));
                    push(self, Constraint::Vertical { line: id }, Rule::Vertical, closeness(90.0 - to_h, tol.angle_deg));
                    for other in &self.entities {
                        if other.id == id {
                            continue;
                        }
                        match other.v.clone() {
                            Entity::Line { .. } => {
                                let (q0, q1) = self.line_pts(other.id).expect("line");
                                let e = [q1[0] - q0[0], q1[1] - q0[1]];
                                let lq = e[0].hypot(e[1]);
                                if lq <= 1e-12 {
                                    continue;
                                }
                                let ang = line_angle(d, e);
                                push(self, Constraint::Parallel { a: id, b: other.id }, Rule::Parallel, closeness(ang, tol.angle_deg));
                                push(self, Constraint::Perpendicular { a: id, b: other.id }, Rule::Perpendicular, closeness(90.0 - ang, tol.angle_deg));
                                push(self, Constraint::Equal { a: id, b: other.id }, Rule::Equal, closeness((len - lq).abs() / len.max(lq), tol.ratio));
                            }
                            Entity::Circle { .. } | Entity::Arc { .. } => {
                                if let Some((c, r)) = self.round(other.id) {
                                    let off = ((d[0] * (c[1] - p0[1]) - d[1] * (c[0] - p0[0])) / len).abs();
                                    push(self, Constraint::Tangent { a: id, b: other.id }, Rule::Tangent, closeness((off - r).abs(), tol.dist));
                                }
                            }
                            Entity::Point { .. } | Entity::Spline { .. } => {}
                        }
                    }
                }
            }
            Entity::Circle { .. } | Entity::Arc { .. } => {
                let (c0, r0) = self.round(id).expect("round");
                for other in &self.entities {
                    if other.id == id {
                        continue;
                    }
                    match other.v.clone() {
                        Entity::Circle { .. } | Entity::Arc { .. } => {
                            let (c1, r1) = self.round(other.id).expect("round");
                            push(self, Constraint::Equal { a: id, b: other.id }, Rule::Equal, closeness((r0 - r1).abs() / r0.max(r1).max(1e-300), tol.ratio));
                            let d = dist(c0, c1);
                            let dev = (d - (r0 + r1)).abs().min((d - (r0 - r1).abs()).abs());
                            push(self, Constraint::Tangent { a: id, b: other.id }, Rule::Tangent, closeness(dev, tol.dist));
                        }
                        Entity::Line { .. } => {
                            let (p0, p1) = self.line_pts(other.id).expect("line");
                            let d = [p1[0] - p0[0], p1[1] - p0[1]];
                            let len = d[0].hypot(d[1]);
                            if len > 1e-12 {
                                let off = ((d[0] * (c0[1] - p0[1]) - d[1] * (c0[0] - p0[0])) / len).abs();
                                push(self, Constraint::Tangent { a: other.id, b: id }, Rule::Tangent, closeness((off - r0).abs(), tol.dist));
                            }
                        }
                        Entity::Point { .. } | Entity::Spline { .. } => {}
                    }
                }
            }
            Entity::Point { .. } | Entity::Spline { .. } => {}
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.rule.cmp(&b.rule)));
        out
    }

    /// Apply the suggestion (a constraint is added and the sketch re-solved); returns the new constraint id.
    pub fn apply(&mut self, s: &Suggestion) -> Option<Id> {
        let id = self.add_constraint(s.constraint.clone())?;
        self.solve();
        Some(id)
    }
}
