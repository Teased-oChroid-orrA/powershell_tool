//! Measurements of a model's geometry that the checks and conversions need, from whatever describes it: a sketch, a
//! meshed surface (an imported mesh, a body mesh) or the tessellation of a STEP solid. Each named surface has an area and
//! an outward unit normal; the whole has the extent of its bounding box.
use crate::problem::{Analysis, Geometry, Problem, Shape};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct FaceFact {
    pub area: f64,
    /// Outward unit normal (zero for a curved or closed surface that has no single direction, such as a hole).
    pub normal: [f64; 3],
    pub centroid: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    /// Bounding-box extent; for a plane model the third is the thickness.
    pub extent: [f64; 3],
    pub faces: BTreeMap<String, FaceFact>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = norm(a);
    if n > 1e-300 { [a[0] / n, a[1] / n, a[2] / n] } else { [0.0; 3] }
}

/// Accumulates the triangles (or edges) of one surface.
#[derive(Default)]
struct Acc {
    area: f64,
    normal: [f64; 3],
    centroid: [f64; 3],
}
impl Acc {
    fn add_triangle(&mut self, a: [f64; 3], b: [f64; 3], c: [f64; 3]) {
        let n = cross(sub(b, a), sub(c, a));
        let area = 0.5 * norm(n);
        self.area += area;
        for k in 0..3 {
            self.normal[k] += 0.5 * n[k];
            self.centroid[k] += area * (a[k] + b[k] + c[k]) / 3.0;
        }
    }
    fn finish(self) -> FaceFact {
        let c = if self.area > 0.0 { [self.centroid[0] / self.area, self.centroid[1] / self.area, self.centroid[2] / self.area] } else { [0.0; 3] };
        FaceFact { area: self.area, normal: unit(self.normal), centroid: c }
    }
}

impl Facts {
    /// From a mesh: its node extent and its named boundary surfaces. A plane mesh has the problem's thickness for its third
    /// extent and its edges have the length times the thickness for area.
    pub fn from_mesh(mesh: &fea_core::Mesh, thickness: f64) -> Option<Facts> {
        if mesh.nodes.is_empty() {
            return None;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for n in &mesh.nodes {
            for k in 0..3 {
                lo[k] = lo[k].min(n[k]);
                hi[k] = hi[k].max(n[k]);
            }
        }
        let plane = mesh.dim() == 2;
        let extent = [hi[0] - lo[0], hi[1] - lo[1], if plane { thickness } else { hi[2] - lo[2] }];
        let mut faces = BTreeMap::new();
        for (name, list) in &mesh.surfaces {
            if plane {
                let (mut length, mut normal, mut centroid) = (0.0, [0.0; 3], [0.0; 3]);
                for f in list {
                    if f.len() < 2 {
                        continue;
                    }
                    let (a, b) = (mesh.nodes[f[0]], mesh.nodes[*f.last().unwrap_or(&f[0])]);
                    let d = sub(b, a);
                    let l = norm(d);
                    length += l;
                    // Edges run counter-clockwise seen from outside, so outward is to the right of the travel direction.
                    normal = [normal[0] + d[1], normal[1] - d[0], 0.0];
                    for k in 0..3 {
                        centroid[k] += l * 0.5 * (a[k] + b[k]);
                    }
                }
                if length > 0.0 {
                    faces.insert(name.clone(), FaceFact { area: length * thickness, normal: unit(normal), centroid: [centroid[0] / length, centroid[1] / length, centroid[2] / length] });
                }
            } else {
                let mut acc = Acc::default();
                for f in list {
                    for i in 1..f.len().saturating_sub(1) {
                        acc.add_triangle(mesh.nodes[f[0]], mesh.nodes[f[i]], mesh.nodes[f[i + 1]]);
                    }
                }
                if acc.area > 0.0 {
                    faces.insert(name.clone(), acc.finish());
                }
            }
        }
        Some(Facts { extent, faces })
    }

    /// From the tessellation of a STEP solid (faces `face1`.. in the cache's face order).
    pub fn from_tessellation(vertices: &[[f64; 3]], triangles: &[[usize; 3]], face: &[usize]) -> Option<Facts> {
        if vertices.is_empty() || triangles.is_empty() {
            return None;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let mut accs: BTreeMap<usize, Acc> = BTreeMap::new();
        for (t, f) in triangles.iter().zip(face) {
            accs.entry(*f).or_default().add_triangle(vertices[t[0]], vertices[t[1]], vertices[t[2]]);
        }
        let faces = accs.into_iter().map(|(f, a)| (format!("face{}", f + 1), a.finish())).collect();
        Some(Facts { extent: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]], faces })
    }

    /// From the sketch of a sketch-based problem: the extent and the faces whose shape the outline gives in closed form
    /// (the four sides of a rectangle, the edges of a polygon, the two end faces of an extrusion).
    pub fn from_sketch(p: &Problem) -> Option<Facts> {
        let Geometry::Sketch { outer, depth, .. } = &p.geometry else { return None };
        let (lo, hi) = outer.bounds();
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        let t = if p.analysis == Analysis::Solid { *depth } else { p.thickness };
        let mut faces = BTreeMap::new();
        let mut put = |name: &str, area: f64, normal: [f64; 3], centroid: [f64; 3]| {
            faces.insert(name.to_string(), FaceFact { area, normal, centroid });
        };
        let mid = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]), 0.5 * t];
        match outer {
            Shape::Rect { .. } => {
                put("left", h * t, [-1.0, 0.0, 0.0], [lo[0], mid[1], mid[2]]);
                put("right", h * t, [1.0, 0.0, 0.0], [hi[0], mid[1], mid[2]]);
                put("bottom", w * t, [0.0, -1.0, 0.0], [mid[0], lo[1], mid[2]]);
                put("top", w * t, [0.0, 1.0, 0.0], [mid[0], hi[1], mid[2]]);
            }
            Shape::Polygon { pts } => {
                // Counter-clockwise polygons have the outward normal to the right of an edge; clockwise ones to the left.
                let area2: f64 = (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[i][1] * pts[(i + 1) % pts.len()][0]).sum();
                let side = if area2 >= 0.0 { 1.0 } else { -1.0 };
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let l = d[0].hypot(d[1]);
                    if l > 0.0 {
                        put(&format!("edge{}", i + 1), l * t, [side * d[1] / l, -side * d[0] / l, 0.0], [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), mid[2]]);
                    }
                }
            }
            _ => {}
        }
        if p.analysis == Analysis::Solid {
            let area = match outer {
                Shape::Rect { .. } => w * h,
                Shape::Polygon { pts } => 0.5 * (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[i][1] * pts[(i + 1) % pts.len()][0]).sum::<f64>().abs(),
                _ => w * h,
            };
            put("start", area, [0.0, 0.0, -1.0], [mid[0], mid[1], 0.0]);
            put("end", area, [0.0, 0.0, 1.0], [mid[0], mid[1], t]);
        }
        Some(Facts { extent: [w, h, t], faces })
    }

    /// The facts the problem alone gives (a sketch or a STEP solid's tessellation); none for an imported mesh, which needs
    /// the mesh itself (`from_mesh`).
    pub fn of(p: &Problem) -> Option<Facts> {
        match &p.geometry {
            Geometry::Sketch { .. } => Self::from_sketch(p),
            Geometry::Solid { cache: Some(c), .. } => Self::from_tessellation(&c.vertices, &c.triangles, &c.face),
            _ => None,
        }
    }
}
