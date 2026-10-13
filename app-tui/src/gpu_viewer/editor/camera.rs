//! CPU mirror of the shader's camera (`shader.wgsl::vs`) plus picking: projecting model points to
//! the screen, finding the entity under the cursor and unprojecting the cursor onto a working plane.
use fea_problem::members::MemberProblem;

/// Same parameters the shader takes: yaw about y, pitch about x, zoom and window aspect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub aspect: f32,
    /// Screen-space shift in NDC (middle or right drag); applied after the projection.
    pub pan: [f32; 2],
}

/// Normalisation from model coordinates to the unit-ish scene space: `(p - center) / span`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub center: [f64; 3],
    pub span: f64,
}
impl Frame {
    pub const UNIT: Frame = Frame { center: [0.0; 3], span: 1.0 };
    /// Frame fitting a problem's nodes (a lone node or an empty model gets a unit-sized frame).
    pub fn fit(problem: &MemberProblem) -> Self {
        if problem.nodes.is_empty() {
            return Self { center: [0.0; 3], span: 100.0 };
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for n in &problem.nodes {
            for k in 0..3 {
                lo[k] = lo[k].min(n.pos[k]);
                hi[k] = hi[k].max(n.pos[k]);
            }
        }
        let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
        Self { center: std::array::from_fn(|k| 0.5 * (lo[k] + hi[k])), span: if span > 0.0 { span * 1.25 } else { 100.0 } }
    }
    pub fn to_scene(&self, p: [f64; 3]) -> [f32; 3] {
        std::array::from_fn(|k| ((p[k] - self.center[k]) / self.span) as f32)
    }
    pub fn to_model(&self, s: [f32; 3]) -> [f64; 3] {
        std::array::from_fn(|k| self.center[k] + s[k] as f64 * self.span)
    }
}

impl Camera {
    fn rotate(&self, p: [f32; 3]) -> [f32; 3] {
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let q = [cy * p[0] + sy * p[2], p[1], -sy * p[0] + cy * p[2]];
        [q[0], cp * q[1] - sp * q[2], sp * q[1] + cp * q[2]]
    }
    fn unrotate(&self, r: [f32; 3]) -> [f32; 3] {
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let q = [r[0], cp * r[1] + sp * r[2], -sp * r[1] + cp * r[2]];
        [cy * q[0] - sy * q[2], q[1], sy * q[0] + cy * q[2]]
    }
    /// Scene point to NDC `(x, y)` and shader depth.
    pub fn project(&self, p: [f32; 3]) -> ([f32; 2], f32) {
        let r = self.rotate(p);
        ([r[0] * self.zoom / self.aspect + self.pan[0], r[1] * self.zoom + self.pan[1]], 0.5 + r[2] * 0.25)
    }
    /// Window pixel (origin top-left) to NDC.
    pub fn ndc(px: f64, py: f64, width: f32, height: f32) -> [f32; 2] {
        [(2.0 * px as f32 / width) - 1.0, 1.0 - 2.0 * py as f32 / height]
    }
    pub fn pixel(ndc: [f32; 2], width: f32, height: f32) -> [f32; 2] {
        [(ndc[0] + 1.0) * 0.5 * width, (1.0 - ndc[1]) * 0.5 * height]
    }
    /// Intersection of the view ray through `ndc` with the plane `axis = value` (scene space).
    pub fn unproject_to_plane(&self, ndc: [f32; 2], axis: usize, value: f32) -> Option<[f32; 3]> {
        let rx = (ndc[0] - self.pan[0]) * self.aspect / self.zoom;
        let ry = (ndc[1] - self.pan[1]) / self.zoom;
        let origin = self.unrotate([rx, ry, 0.0]);
        let dir = self.unrotate([0.0, 0.0, 1.0]);
        if dir[axis].abs() < 1e-4 {
            return None;
        }
        let t = (value - origin[axis]) / dir[axis];
        Some(std::array::from_fn(|k| origin[k] + t * dir[k]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Node(u32),
    Member(u32),
}

/// Entity under a pixel: nodes within `radius` pixels win over members within the same radius;
/// among candidates the nearest on screen (ties: nearer to the viewer) is chosen.
pub fn pick(problem: &MemberProblem, frame: &Frame, cam: &Camera, px: [f32; 2], size: [f32; 2], radius: f32) -> Option<Hit> {
    let screen = |p: [f64; 3]| {
        let (n, d) = cam.project(frame.to_scene(p));
        (Camera::pixel(n, size[0], size[1]), d)
    };
    let mut best: Option<(f32, f32, u32)> = None;
    for n in &problem.nodes {
        let (s, d) = screen(n.pos);
        let dist = ((s[0] - px[0]).powi(2) + (s[1] - px[1]).powi(2)).sqrt();
        if dist <= radius && best.is_none_or(|(bd, bz, _)| (dist, d) < (bd, bz)) {
            best = Some((dist, d, n.id));
        }
    }
    if let Some((_, _, id)) = best {
        return Some(Hit::Node(id));
    }
    let mut best: Option<(f32, f32, u32)> = None;
    for m in &problem.members {
        let (Some(a), Some(b)) = (problem.node(m.nodes[0]), problem.node(m.nodes[1])) else { continue };
        let ((sa, da), (sb, db)) = (screen(a.pos), screen(b.pos));
        let ab = [sb[0] - sa[0], sb[1] - sa[1]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if len2 > 0.0 { (((px[0] - sa[0]) * ab[0] + (px[1] - sa[1]) * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let c = [sa[0] + t * ab[0], sa[1] + t * ab[1]];
        let dist = ((c[0] - px[0]).powi(2) + (c[1] - px[1]).powi(2)).sqrt();
        let depth = da + t * (db - da);
        if dist <= radius && best.is_none_or(|(bd, bz, _)| (dist, depth) < (bd, bz)) {
            best = Some((dist, depth, m.id));
        }
    }
    best.map(|(_, _, id)| Hit::Member(id))
}

/// Nodes whose projection lies in the pixel box `a`..`b`, plus members with both ends inside.
pub fn pick_box(problem: &MemberProblem, frame: &Frame, cam: &Camera, a: [f32; 2], b: [f32; 2], size: [f32; 2]) -> Vec<Hit> {
    let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
    let inside = |p: [f64; 3]| {
        let (n, _) = cam.project(frame.to_scene(p));
        let s = Camera::pixel(n, size[0], size[1]);
        s[0] >= lo[0] && s[0] <= hi[0] && s[1] >= lo[1] && s[1] <= hi[1]
    };
    let nodes: Vec<u32> = problem.nodes.iter().filter(|n| inside(n.pos)).map(|n| n.id).collect();
    let mut out: Vec<Hit> = nodes.iter().map(|id| Hit::Node(*id)).collect();
    out.extend(problem.members.iter().filter(|m| m.nodes.iter().all(|n| nodes.contains(n))).map(|m| Hit::Member(m.id)));
    out
}

/// Pick tolerance in window pixels for a viewport of `size`: grows with the display (a Retina window
/// is twice as many physical pixels, so a fixed 10 px target was half the visual size) but stays usable.
pub fn pick_radius(size: [f32; 2]) -> f32 {
    (size[0].min(size[1]) / 50.0).clamp(12.0, 40.0)
}

/// Snap a value to the nearest multiple of `step` (no-op for a non-positive step).
pub fn snap(v: f64, step: f64) -> f64 {
    if step > 0.0 && step.is_finite() {
        // `+ 0.0` normalises -0.0 so coordinates compare and serialise as plain zero.
        (v / step).round() * step + 0.0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_problem::members::{Edit, MemberKind};

    fn cam(yaw: f32, pitch: f32) -> Camera {
        Camera { yaw, pitch, zoom: 1.6, aspect: 1.5, pan: [0.0, 0.0] }
    }

    #[test]
    fn rotate_and_unrotate_are_inverse() {
        for (y, p) in [(0.0, 0.0), (0.7, -0.4), (-2.0, 1.2)] {
            let c = cam(y, p);
            for pt in [[0.3, -0.2, 0.1], [1.0, 2.0, -3.0]] {
                let back = c.unrotate(c.rotate(pt));
                for k in 0..3 {
                    assert!((back[k] - pt[k]).abs() < 1e-5);
                }
            }
        }
    }
    #[test]
    fn projection_matches_the_shader_formula() {
        // yaw = pitch = 0: x scaled by zoom/aspect, y by zoom, depth 0.5 + 0.25 z (shader.wgsl::vs).
        let (ndc, depth) = cam(0.0, 0.0).project([0.3, -0.2, 0.4]);
        assert!((ndc[0] - 0.3 * 1.6 / 1.5).abs() < 1e-6 && (ndc[1] + 0.2 * 1.6).abs() < 1e-6);
        assert!((depth - 0.6).abs() < 1e-6);
        // a quarter-turn about y maps +x onto -z (depth) and +z onto +x
        let (ndc, _) = cam(std::f32::consts::FRAC_PI_2, 0.0).project([0.0, 0.0, 1.0]);
        assert!((ndc[0] - 1.6 / 1.5).abs() < 1e-5);
    }
    #[test]
    fn pixel_and_ndc_round_trip() {
        let n = Camera::ndc(300.0, 150.0, 600.0, 300.0);
        assert!(n[0].abs() < 1e-6 && n[1].abs() < 1e-6);
        let p = Camera::pixel(Camera::ndc(123.0, 45.0, 800.0, 600.0), 800.0, 600.0);
        assert!((p[0] - 123.0).abs() < 1e-3 && (p[1] - 45.0).abs() < 1e-3);
    }
    #[test]
    fn unprojection_inverts_projection_on_each_working_plane() {
        for (yaw, pitch) in [(0.0, 0.0), (0.5, 0.3), (-0.8, -0.6)] {
            let c = cam(yaw, pitch);
            for axis in 0..3 {
                let mut p = [0.21, -0.13, 0.34];
                p[axis] = 0.05;
                let (ndc, _) = c.project(p);
                let Some(q) = c.unproject_to_plane(ndc, axis, 0.05) else { continue };
                for k in 0..3 {
                    assert!((q[k] - p[k]).abs() < 1e-4, "axis {axis} yaw {yaw}: {q:?} vs {p:?}");
                }
            }
        }
        // A plane seen edge-on cannot be unprojected onto.
        assert!(cam(0.0, 0.0).unproject_to_plane([0.0, 0.0], 0, 0.0).is_none());
    }
    #[test]
    fn frame_round_trip_and_fit() {
        let mut p = MemberProblem::new("f");
        assert_eq!(Frame::fit(&p).span, 100.0);
        p.apply(Edit::AddNode { pos: [10.0, 0.0, 0.0] }).unwrap();
        p.apply(Edit::AddNode { pos: [30.0, 5.0, 0.0] }).unwrap();
        let f = Frame::fit(&p);
        assert_eq!(f.center, [20.0, 2.5, 0.0]);
        assert!((f.span - 25.0).abs() < 1e-12);
        let s = f.to_scene([30.0, 5.0, 0.0]);
        let m = f.to_model(s);
        assert!((m[0] - 30.0).abs() < 1e-4 && (m[1] - 5.0).abs() < 1e-4);
    }
    #[test]
    fn snap_rounds_to_grid_and_ignores_bad_steps() {
        assert_eq!(snap(7.4, 5.0), 5.0);
        assert_eq!(snap(7.6, 5.0), 10.0);
        assert_eq!(snap(-0.2, 5.0).to_bits(), 0.0_f64.to_bits(), "no negative zero");
        assert_eq!(snap(7.4, 0.0), 7.4);
        assert_eq!(snap(7.4, f64::NAN), 7.4);
    }
    #[test]
    fn picking_prefers_nodes_then_members_and_misses_empty_space() {
        let mut p = MemberProblem::new("pick");
        let a = p.apply(Edit::AddNode { pos: [0.0, 0.0, 0.0] }).unwrap().unwrap();
        let b = p.apply(Edit::AddNode { pos: [100.0, 0.0, 0.0] }).unwrap().unwrap();
        let m = p.apply(Edit::AddMember { a, b, kind: MemberKind::Truss { e: 1.0, area: 1.0 } }).unwrap().unwrap();
        let frame = Frame { center: [50.0, 0.0, 0.0], span: 125.0 };
        let c = cam(0.0, 0.0);
        let size = [900.0, 600.0];
        let at = |pt: [f64; 3]| {
            let (n, _) = c.project(frame.to_scene(pt));
            Camera::pixel(n, size[0], size[1])
        };
        let near_a = at([0.0, 0.0, 0.0]);
        assert_eq!(pick(&p, &frame, &c, [near_a[0] + 2.0, near_a[1] - 2.0], size, 8.0), Some(Hit::Node(a)));
        let mid = at([50.0, 0.0, 0.0]);
        assert_eq!(pick(&p, &frame, &c, [mid[0], mid[1] + 3.0], size, 8.0), Some(Hit::Member(m)));
        assert_eq!(pick(&p, &frame, &c, [mid[0], mid[1] + 60.0], size, 8.0), None);
        assert_eq!(pick(&MemberProblem::new("e"), &frame, &c, mid, size, 8.0), None);
    }
}
