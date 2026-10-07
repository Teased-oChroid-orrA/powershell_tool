//! A result field (or just the mesh) drawn into a pixel grid, for a terminal view. Pixels are
//! assumed square; the caller maps two pixel rows to one text row (half blocks).

use fea_core::{Mesh, ElementKind};

#[derive(Debug, Clone)]
pub struct Raster {
    pub w: usize,
    pub h: usize,
    /// Row-major from the top; `NaN` = outside the body.
    pub value: Vec<f64>,
    /// Element edge pixels (the mesh lines).
    pub edge: Vec<bool>,
    /// Data range over the drawn pixels (`0, 0` when nothing is drawn).
    pub min: f64,
    pub max: f64,
    /// Model coordinates of the pixel grid: `x = origin[0] + (px + 0.5) / scale`, `y = origin[1] - (py + 0.5) / scale`.
    pub origin: [f64; 2],
    pub scale: f64,
}

impl Raster {
    pub fn at(&self, x: usize, y: usize) -> f64 {
        self.value[y * self.w + x]
    }
}

/// Triangles `(corner nodes)` to paint: every 2D element fanned, or the front (`z = max`) boundary
/// faces of a 3D mesh. Also the polygon outlines to stroke.
fn faces(mesh: &Mesh) -> (Vec<[usize; 3]>, Vec<Vec<usize>>) {
    let (mut tris, mut outlines) = (Vec::new(), Vec::new());
    if mesh.dim() == 2 {
        for blk in &mesh.blocks {
            let nc = blk.kind.n_corners();
            for c in blk.conn.chunks_exact(blk.kind.n_nodes()) {
                tris.push([c[0], c[1], c[2]]);
                if nc == 4 {
                    tris.push([c[0], c[2], c[3]]);
                }
                outlines.push(c[..nc].to_vec());
            }
        }
        return (tris, outlines);
    }
    let zmax = mesh.nodes.iter().map(|x| x[2]).fold(f64::NEG_INFINITY, f64::max);
    let zmin = mesh.nodes.iter().map(|x| x[2]).fold(f64::INFINITY, f64::min);
    let tol = 1e-9 * (zmax - zmin).max(1e-300) + 1e-12;
    for f in mesh.boundary_faces() {
        let nc = if matches!(f.nodes.len(), 3 | 6) { 3 } else { 4 };
        let c = &f.nodes[..nc];
        if c.iter().all(|&n| (mesh.nodes[n][2] - zmax).abs() <= tol) {
            tris.push([c[0], c[1], c[2]]);
            if nc == 4 {
                tris.push([c[0], c[2], c[3]]);
            }
            outlines.push(c.to_vec());
        }
    }
    (tris, outlines)
}

/// Paint `values` (per node; `None` = flat fill) of `mesh` into a `w x h` grid. `deform = (u, scale)`
/// draws the displaced shape (`u` has `dim` components per node).
pub fn rasterize(mesh: &Mesh, values: Option<&[f64]>, deform: Option<(&[f64], f64)>, w: usize, h: usize) -> Raster {
    let d = mesh.dim();
    let pos: Vec<[f64; 2]> = mesh
        .nodes
        .iter()
        .enumerate()
        .map(|(i, x)| match deform {
            Some((u, s)) => [x[0] + s * u[i * d], x[1] + s * u[i * d + 1]],
            None => [x[0], x[1]],
        })
        .collect();
    let (tris, outlines) = faces(mesh);
    let mut r = Raster { w, h, value: vec![f64::NAN; w * h], edge: vec![false; w * h], min: 0.0, max: 0.0, origin: [0.0; 2], scale: 1.0 };
    if tris.is_empty() || w == 0 || h == 0 {
        return r;
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for t in &tris {
        for &n in t {
            for i in 0..2 {
                lo[i] = lo[i].min(pos[n][i]);
                hi[i] = hi[i].max(pos[n][i]);
            }
        }
    }
    let (dx, dy) = ((hi[0] - lo[0]).max(1e-300), (hi[1] - lo[1]).max(1e-300));
    let scale = (w as f64 / dx).min(h as f64 / dy) * 0.999;
    // Centre the drawing in the grid.
    let (ox, oy) = (lo[0] - 0.5 * (w as f64 / scale - dx), hi[1] + 0.5 * (h as f64 / scale - dy));
    r.origin = [ox, oy];
    r.scale = scale;
    let px = |p: [f64; 2]| ((p[0] - ox) * scale, (oy - p[1]) * scale);
    let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for t in &tris {
        let q: Vec<(f64, f64)> = t.iter().map(|&n| px(pos[n])).collect();
        let val: Vec<f64> = t.iter().map(|&n| values.map_or(0.0, |v| v[n])).collect();
        let (x0, x1) = (q.iter().map(|p| p.0).fold(f64::INFINITY, f64::min), q.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max));
        let (y0, y1) = (q.iter().map(|p| p.1).fold(f64::INFINITY, f64::min), q.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max));
        let area = (q[1].0 - q[0].0) * (q[2].1 - q[0].1) - (q[2].0 - q[0].0) * (q[1].1 - q[0].1);
        if area.abs() < 1e-18 {
            continue;
        }
        let (ia, ib) = ((x0 - 0.5).floor().max(0.0) as usize, ((x1 - 0.5).ceil().max(0.0) as usize).min(w - 1));
        let (ja, jb) = ((y0 - 0.5).floor().max(0.0) as usize, ((y1 - 0.5).ceil().max(0.0) as usize).min(h - 1));
        for j in ja..=jb {
            for i in ia..=ib {
                let (cx, cy) = (i as f64 + 0.5, j as f64 + 0.5);
                let l1 = ((cx - q[0].0) * (q[2].1 - q[0].1) - (q[2].0 - q[0].0) * (cy - q[0].1)) / area;
                let l2 = ((q[1].0 - q[0].0) * (cy - q[0].1) - (cx - q[0].0) * (q[1].1 - q[0].1)) / area;
                let l0 = 1.0 - l1 - l2;
                let eps = -1e-9;
                if l0 >= eps && l1 >= eps && l2 >= eps {
                    let v = l0 * val[0] + l1 * val[1] + l2 * val[2];
                    r.value[j * w + i] = v;
                    vmin = vmin.min(v);
                    vmax = vmax.max(v);
                }
            }
        }
    }
    for o in &outlines {
        for k in 0..o.len() {
            let (a, b) = (px(pos[o[k]]), px(pos[o[(k + 1) % o.len()]]));
            let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
            for s in 0..=steps {
                let t = s as f64 / steps as f64;
                let (x, y) = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                if x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h {
                    r.edge[y as usize * w + x as usize] = true;
                }
            }
        }
    }
    if vmin.is_finite() {
        r.min = vmin;
        r.max = vmax;
    }
    r
}

/// Kind of element a mesh holds first, for labels.
pub fn first_kind(mesh: &Mesh) -> Option<ElementKind> {
    mesh.blocks.first().map(|b| b.kind)
}
