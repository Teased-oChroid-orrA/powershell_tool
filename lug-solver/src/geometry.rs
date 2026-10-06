//! Lug outline: a rounded rectangle with the hole centre at the origin.
//!
//! The lug axis is `+x`: the shank runs to `x = +length` (where the model is
//! clamped) and the head ends at `x = -edge`. Head corners and far corners
//! carry their own radii; a full-round head is `edge = width / 2` with
//! `head_corner_radius = width / 2`. The outline is convex and contains the
//! origin, so every ray from the hole centre leaves it exactly once - which
//! is what the star-shaped mapped mesh in `mesh.rs` relies on.

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LugGeometry {
    /// Hole (bore) diameter.
    pub hole_dia: f64,
    /// Shank width (y extent is `+-width / 2`).
    pub width: f64,
    /// Distance from the hole centre to the end of the head.
    pub edge: f64,
    /// Distance from the hole centre to the clamped far end of the model.
    pub length: f64,
    pub thickness: f64,
    /// Radius of the two corners at the head end.
    pub head_corner_radius: f64,
    /// Radius of the two corners at the far end (0 for a plain shank).
    pub far_corner_radius: f64,
}

impl LugGeometry {
    /// A full-round lug: head radius = half the width, concentric with the hole.
    pub fn round_head(hole_dia: f64, width: f64, thickness: f64, shank_length: f64) -> Self {
        Self { hole_dia, width, edge: width / 2.0, length: shank_length, thickness, head_corner_radius: width / 2.0, far_corner_radius: 0.0 }
    }

    pub fn bore_radius(&self) -> f64 {
        self.hole_dia / 2.0
    }

    pub fn validate(&self) -> Result<(), String> {
        let finite = [self.hole_dia, self.width, self.edge, self.length, self.thickness, self.head_corner_radius, self.far_corner_radius].iter().all(|v| v.is_finite());
        if !finite {
            return Err("lug dimensions must be finite".into());
        }
        let a = self.bore_radius();
        if self.hole_dia <= 0.0 || self.thickness <= 0.0 {
            return Err("hole diameter and thickness must be positive".into());
        }
        if self.width <= self.hole_dia {
            return Err("width must exceed the hole diameter".into());
        }
        if self.edge <= a {
            return Err("edge distance must exceed the hole radius".into());
        }
        if self.length <= a {
            return Err("length must exceed the hole radius".into());
        }
        let half_w = self.width / 2.0;
        for (name, r, reach) in [("head", self.head_corner_radius, self.edge), ("far", self.far_corner_radius, self.length)] {
            if r < 0.0 || r > half_w + 1e-12 {
                return Err(format!("{name} corner radius must be between 0 and half the width"));
            }
            if r > reach + 1e-12 {
                return Err(format!("{name} corner radius exceeds the distance from the hole to that end"));
            }
        }
        if -self.edge + self.head_corner_radius > self.length - self.far_corner_radius + 1e-12 {
            return Err("corner radii overlap".into());
        }
        // The outline must stay clear of the bore everywhere.
        if self.ray_distance_all_min() <= a * (1.0 + 1e-9) {
            return Err("the outline touches the bore".into());
        }
        Ok(())
    }

    /// Distance from the origin to the outline along direction `theta`.
    pub fn ray_distance(&self, theta: f64) -> f64 {
        let (c, s) = (theta.cos(), theta.sin());
        let half_w = self.width / 2.0;
        let mut t = f64::INFINITY;
        if c > 1e-14 {
            t = t.min(self.length / c);
        }
        if c < -1e-14 {
            t = t.min(self.edge / -c);
        }
        if s > 1e-14 {
            t = t.min(half_w / s);
        }
        if s < -1e-14 {
            t = t.min(half_w / -s);
        }
        // Corner arcs: where the straight-sided exit falls in a corner square,
        // the true exit is the arc. Centres (cx, +-cy), outward direction (sx, sign).
        let corners = [
            (-self.edge + self.head_corner_radius, half_w - self.head_corner_radius, self.head_corner_radius, -1.0),
            (self.length - self.far_corner_radius, half_w - self.far_corner_radius, self.far_corner_radius, 1.0),
        ];
        let (px, py) = (t * c, t * s);
        for (cx, cy, r, sx) in corners {
            if r <= 0.0 {
                continue;
            }
            let sy = if py >= 0.0 { 1.0 } else { -1.0 };
            let (ccx, ccy) = (cx, sy * cy);
            let in_corner = (px - ccx) * sx > 0.0 && (py - ccy) * sy > 0.0;
            if in_corner {
                // |p0 + u d - cc| = r, larger root.
                let (dx, dy) = (-ccx, -ccy);
                let b = dx * c + dy * s;
                let cq = dx * dx + dy * dy - r * r;
                let disc = b * b - cq;
                if disc >= 0.0 {
                    t = -b + disc.sqrt();
                }
            }
        }
        t
    }

    fn ray_distance_all_min(&self) -> f64 {
        (0..720).map(|k| self.ray_distance(2.0 * PI * k as f64 / 720.0)).fold(f64::INFINITY, f64::min)
    }

    /// Angles (radians, `[0, 2*pi]`, sorted, de-duplicated) where the outline
    /// changes character: corner tangent points, sharp corners, and the two
    /// axis directions. Mesh stations must sit exactly on these so element
    /// edges conform to the outline.
    pub fn critical_angles(&self) -> Vec<f64> {
        let half_w = self.width / 2.0;
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for sy in [1.0, -1.0] {
            for (x_end, r, sx) in [(-self.edge, self.head_corner_radius, -1.0), (self.length, self.far_corner_radius, 1.0)] {
                if r <= 1e-12 {
                    pts.push((x_end, sy * half_w));
                } else {
                    pts.push((x_end - sx * r, sy * half_w));
                    pts.push((x_end, sy * (half_w - r)));
                }
            }
        }
        let mut ang: Vec<f64> = pts.iter().map(|&(x, y)| y.atan2(x).rem_euclid(2.0 * PI)).collect();
        ang.extend([0.0, PI, 2.0 * PI]);
        ang.sort_by(|a, b| a.total_cmp(b));
        ang.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        ang
    }

    /// Area of the outline (without the hole), by fine polar quadrature.
    pub fn outline_area(&self) -> f64 {
        let n = 20_000;
        let h = 2.0 * PI / n as f64;
        let mut area = 0.0;
        for k in 0..n {
            let th = (k as f64 + 0.5) * h;
            let r = self.ray_distance(th);
            area += 0.5 * r * r * h;
        }
        area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lug() -> LugGeometry {
        LugGeometry::round_head(0.5, 1.5, 0.25, 3.0)
    }

    #[test]
    fn round_head_ray_distance_is_the_head_radius_on_the_head_side() {
        let g = lug();
        for k in 0..=18 {
            let th = PI / 2.0 + PI * k as f64 / 18.0;
            assert!((g.ray_distance(th) - 0.75).abs() < 1e-9, "theta {th}");
        }
    }

    #[test]
    fn shank_side_ray_distance_follows_the_straight_sides_and_far_end() {
        let g = lug();
        assert!((g.ray_distance(0.0) - 3.0).abs() < 1e-12);
        assert!((g.ray_distance(PI / 2.0 - 1e-9) - 0.75).abs() < 1e-6);
        assert!((g.ray_distance(PI / 4.0) - 0.75 * 2f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn outline_area_matches_the_closed_form() {
        let g = lug();
        // Rectangle from the head centre to the far end plus a half disc.
        let exact = 1.5 * 3.0 + PI * 0.75 * 0.75 / 2.0;
        assert!((g.outline_area() - exact).abs() / exact < 1e-6, "{} vs {exact}", g.outline_area());
    }

    #[test]
    fn critical_angles_include_corners_tangents_and_axes() {
        let ang = lug().critical_angles();
        assert!(ang.first().unwrap().abs() < 1e-12 && (ang.last().unwrap() - 2.0 * PI).abs() < 1e-12);
        assert!(ang.iter().any(|a| (a - PI).abs() < 1e-12));
        assert!(ang.iter().any(|a| (a - PI / 2.0).abs() < 1e-12), "head tangent point at the top");
        let far = (0.75f64).atan2(3.0);
        assert!(ang.iter().any(|a| (a - far).abs() < 1e-12), "far corner");
    }

    #[test]
    fn rounded_rectangle_corner_arcs_are_followed() {
        let g = LugGeometry { hole_dia: 0.4, width: 2.0, edge: 1.5, length: 3.0, thickness: 0.2, head_corner_radius: 0.5, far_corner_radius: 0.0 };
        g.validate().unwrap();
        // On the 135-degree ray the exit is on the corner arc: centre (-1.0, 0.5), r 0.5.
        let th = 3.0 * PI / 4.0;
        let r = g.ray_distance(th);
        let (x, y) = (r * th.cos(), r * th.sin());
        assert!(((x + 1.0).hypot(y - 0.5) - 0.5).abs() < 1e-9, "({x}, {y})");
    }

    #[test]
    fn validation_rejects_impossible_lugs() {
        let mut g = lug();
        g.hole_dia = 1.6;
        assert!(g.validate().is_err());
        let mut g = lug();
        g.edge = 0.2;
        assert!(g.validate().is_err());
        let mut g = lug();
        g.head_corner_radius = 0.9;
        assert!(g.validate().is_err());
        assert!(lug().validate().is_ok());
    }
}
