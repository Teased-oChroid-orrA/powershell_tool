//! Supports and loads drawn on the model: an arrow for every load (pointing the way it acts, the magnitude beside it) and a
//! symbol on every supported face (the held directions beside it), computed for the camera each frame and painted by the
//! chrome over the viewport. They follow the active load case and are there in every display style, with a result shown too.
use super::*;
use crate::gpu_viewer::editor::authoring::{Overlay, OverlayKind};
use fea_problem::Load;

/// Arrow length in viewport pixels.
const ARROW_PX: f32 = 56.0;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn unit(a: [f32; 3]) -> [f32; 3] {
    let l = len(a);
    if l > 1e-12 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0; 3]
    }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A named surface as the glyphs need it: where it is and which way is out of the body (scene units).
struct Place {
    at: [f32; 3],
    out: [f32; 3],
}

impl SolidSession {
    /// The mesh the glyphs sit on (the one shown, else the result's) with its nodes in scene units.
    fn glyph_mesh(&self) -> Option<(Arc<Mesh>, Vec<[f32; 3]>)> {
        let mesh = self.shown_mesh().cloned().or_else(|| self.current_solution().map(|s| Arc::new(s.model.mesh.clone())))?;
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in &mesh.nodes {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        // The same normalisation as `Scene::from_frames`.
        let span = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        if !(span.is_finite() && span > 0.0) {
            return None;
        }
        let pos = mesh.nodes.iter().map(|p| std::array::from_fn(|a| ((p[a] - lo[a] - (hi[a] - lo[a]) / 2.0) / span) as f32)).collect();
        Some((mesh, pos))
    }

    /// The node nearest a point (model units).
    fn nearest(mesh: &Mesh, x: f64, y: f64, z: f64) -> Option<usize> {
        let d = |n: &[f64; 3]| (n[0] - x).powi(2) + (n[1] - y).powi(2) + (n[2] - z).powi(2);
        mesh.nodes.iter().enumerate().min_by(|a, b| d(a.1).total_cmp(&d(b.1))).map(|(i, _)| i)
    }

    fn place(mesh: &Mesh, pos: &[[f32; 3]], name: &str) -> Option<Place> {
        let nodes = Self::nodes_of(mesh, name);
        if nodes.is_empty() {
            return None;
        }
        let n = nodes.len() as f32;
        let at = nodes.iter().fold([0.0; 3], |s, i| [s[0] + pos[*i][0] / n, s[1] + pos[*i][1] / n, s[2] + pos[*i][2] / n]);
        let centre = pos.iter().fold([0.0; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]);
        let centre = [centre[0] / pos.len() as f32, centre[1] / pos.len() as f32, centre[2] / pos.len() as f32];
        // Faces are listed counter-clockwise seen from outside, so their normals point out of the body.
        let mut normal = [0.0f32; 3];
        for face in mesh.surfaces.get(name).into_iter().flatten() {
            if mesh.dim() == 2 && face.len() >= 2 {
                let (a, b) = (pos[face[0]], pos[*face.last().unwrap_or(&face[0])]);
                let d = sub(b, a);
                normal = [normal[0] + d[1], normal[1] - d[0], 0.0];
            } else if face.len() >= 3 {
                let (a, b, c) = (pos[face[0]], pos[face[1]], pos[face[2]]);
                let (u, v) = (sub(b, a), sub(c, a));
                normal = [normal[0] + u[1] * v[2] - u[2] * v[1], normal[1] + u[2] * v[0] - u[0] * v[2], normal[2] + u[0] * v[1] - u[1] * v[0]];
            }
        }
        let mut out = unit(normal);
        if len(out) == 0.0 {
            out = unit(sub(at, centre));
        }
        Some(Place { at, out })
    }

    /// Glyphs of the supports and loads for `cam` in a viewport of `size` pixels.
    pub(super) fn bc_overlay(&self, cam: &Camera, size: [f32; 2]) -> Vec<Overlay> {
        if self.sketching.is_some() || self.show_study || !self.problem.study.is_structural() {
            return Vec::new();
        }
        let Some((mesh, pos)) = self.glyph_mesh() else { return Vec::new() };
        let px = |p: [f32; 3]| {
            let (ndc, _) = cam.project(p);
            Camera::pixel(ndc, size[0], size[1])
        };
        // A scene-space direction as a screen direction (unit vector in pixels).
        let screen = |at: [f32; 3], d: [f32; 3]| {
            let (a, b) = (px(at), px([at[0] + d[0] * 0.05, at[1] + d[1] * 0.05, at[2] + d[2] * 0.05]));
            let v = [b[0] - a[0], b[1] - a[1]];
            let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
            if l > 1e-6 {
                [v[0] / l, v[1] / l]
            } else {
                [0.0, -1.0]
            }
        };
        let case = self.problem.case();
        let dim = self.problem.analysis_dim();
        let mut out = Vec::new();
        let centre = pos.iter().fold([0.0; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]);
        let centre = [centre[0] / pos.len() as f32, centre[1] / pos.len() as f32, centre[2] / pos.len() as f32];
        for (i, sup) in self.problem.supports.iter().enumerate() {
            if !case.is_none_or(|c| c.support_on(i)) {
                continue;
            }
            let place = match sup {
                fea_problem::Support::Point { x, y, z, .. } => Self::nearest(&mesh, *x, *y, *z).map(|n| Place { at: pos[n], out: unit(sub(pos[n], centre)) }),
                _ => sup.edge().and_then(|e| Self::place(&mesh, &pos, e)),
            };
            let Some(place) = place else { continue };
            let held: String = sup.comps().iter().take(dim).enumerate().filter(|(_, c)| c.is_some()).map(|(a, _)| ["x", "y", "z"][a]).collect();
            out.push(Overlay { px: px(place.at), text: held, kind: OverlayKind::Support, tip: Some(screen(place.at, place.out)) });
        }
        let format = crate::toolboxes::fea_workbench::model::format_number;
        for (i, load) in self.problem.loads.iter().enumerate() {
            let factor = case.map_or(1.0, |c| c.factor(i));
            if factor == 0.0 {
                continue;
            }
            // (tail, direction in scene units, label); the arrow ends on the surface when the load pushes into it.
            let (anchor, dir, label, pushes_in): ([f32; 3], [f32; 3], String, Option<bool>) = match load {
                Load::Pressure { edge, p } => {
                    let Some(pl) = Self::place(&mesh, &pos, edge) else { continue };
                    let sign = if *p >= 0.0 { -1.0 } else { 1.0 };
                    (pl.at, [pl.out[0] * sign, pl.out[1] * sign, pl.out[2] * sign], format(p * factor), Some(*p >= 0.0))
                }
                Load::Traction { edge, tx, ty, tz } => {
                    let Some(pl) = Self::place(&mesh, &pos, edge) else { continue };
                    let d = unit([*tx as f32, *ty as f32, if dim == 3 { *tz as f32 } else { 0.0 }]);
                    let mag = (tx * tx + ty * ty + if dim == 3 { tz * tz } else { 0.0 }).sqrt() * factor;
                    (pl.at, d, format(mag), Some(dot(d, pl.out) < 0.0))
                }
                Load::Force { edge, fx, fy, fz } => {
                    let Some(pl) = Self::place(&mesh, &pos, edge) else { continue };
                    let d = unit([*fx as f32, *fy as f32, if dim == 3 { *fz as f32 } else { 0.0 }]);
                    let mag = (fx * fx + fy * fy + if dim == 3 { fz * fz } else { 0.0 }).sqrt() * factor;
                    (pl.at, d, format(mag), Some(dot(d, pl.out) < 0.0))
                }
                Load::Bearing { hole, fx, fy } => {
                    let Some(pl) = Self::place(&mesh, &pos, &format!("hole{hole}")) else { continue };
                    let d = unit([*fx as f32, *fy as f32, 0.0]);
                    (pl.at, d, format((fx * fx + fy * fy).sqrt() * factor), None)
                }
                Load::Point { x, y, z, fx, fy, fz } => {
                    let Some(n) = Self::nearest(&mesh, *x, *y, *z) else { continue };
                    let d = unit([*fx as f32, *fy as f32, if dim == 3 { *fz as f32 } else { 0.0 }]);
                    (pos[n], d, format((fx * fx + fy * fy + fz * fz).sqrt() * factor), None)
                }
                Load::Body { bx, by, bz } => {
                    let d = unit([*bx as f32, *by as f32, if dim == 3 { *bz as f32 } else { 0.0 }]);
                    (centre, d, format!("body {}", format((bx * bx + by * by + bz * bz).sqrt() * factor)), None)
                }
            };
            if len(dir) == 0.0 {
                continue;
            }
            let s = screen(anchor, dir);
            let a = px(anchor);
            // Tip on the surface for a load pushing in; otherwise the arrow leaves from the surface.
            let (tail, tip) = match pushes_in {
                Some(true) => ([a[0] - s[0] * ARROW_PX, a[1] - s[1] * ARROW_PX], a),
                _ => (a, [a[0] + s[0] * ARROW_PX, a[1] + s[1] * ARROW_PX]),
            };
            out.push(Overlay { px: tail, text: label, kind: OverlayKind::Load, tip: Some(tip) });
        }
        Self::declutter(out)
    }

    /// Glyphs that land on the same spot would draw over each other (a roller on a face and a point support at its
    /// centre, two loads on one face): supports there merge into one glyph naming every held direction, and loads fan out
    /// sideways so every arrow and label stays readable.
    fn declutter(glyphs: Vec<Overlay>) -> Vec<Overlay> {
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 10.0 && (a[1] - b[1]).abs() < 10.0;
        let mut out: Vec<Overlay> = Vec::new();
        for g in glyphs {
            match g.kind {
                OverlayKind::Support => {
                    if let Some(prev) = out.iter_mut().find(|o| o.kind == OverlayKind::Support && near(o.px, g.px)) {
                        let mut dirs: Vec<char> = prev.text.chars().chain(g.text.chars()).collect();
                        dirs.sort_unstable();
                        dirs.dedup();
                        prev.text = dirs.into_iter().collect();
                    } else {
                        out.push(g);
                    }
                }
                OverlayKind::Load => {
                    let crowd = out.iter().filter(|o| o.kind == OverlayKind::Load && near(o.px, g.px)).count();
                    let mut g = g;
                    if crowd > 0 {
                        // Sideways of the arrow, a little further each time.
                        let (tail, tip) = (g.px, g.tip.unwrap_or(g.px));
                        let d = [tip[0] - tail[0], tip[1] - tail[1]];
                        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
                        let side = [-d[1] / l * 22.0 * crowd as f32, d[0] / l * 22.0 * crowd as f32];
                        g.px = [tail[0] + side[0], tail[1] + side[1]];
                        g.tip = Some([tip[0] + side[0], tip[1] + side[1]]);
                    }
                    out.push(g);
                }
                _ => out.push(g),
            }
        }
        out
    }
}
