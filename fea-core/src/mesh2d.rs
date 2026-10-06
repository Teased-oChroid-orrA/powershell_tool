//! From a [`Region`] to a finite-element [`Mesh`]: unstructured triangulation (`delaunay.rs`) as
//! Tri3 or Tri6, or split into three quadrilaterals per triangle (all-quad Quad4 / Quad8 / Quad9,
//! the plasticity-friendly choice and the input of `sweep.rs`).
//!
//! Boundary midside nodes (and, for the quad split, the quarter points of the original boundary
//! edges) are evaluated on the geometry's curves, so arcs are represented by quadratic elements to
//! `O(h^3)` instead of by chords. Segment names become node sets and surfaces; surface edges are
//! listed in load orientation (body on the left) with the midside node last.

use crate::delaunay::{triangulate, BoundaryEdge, MeshOptions, TriMesh};
use crate::element::ElementKind;
use crate::geometry::Region;
use crate::mesh::{Mesh, Physics};
use std::collections::HashMap;

/// Mesh `region` with target edge length `size(x)`.
pub fn mesh_region(region: &Region, physics: Physics, kind: ElementKind, size: &dyn Fn([f64; 2]) -> f64, opt: MeshOptions) -> Result<Mesh, String> {
    let tm = triangulate(region, size, opt)?;
    convert(&tm, region, physics, kind)
}

fn key(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

struct Ctx<'a> {
    tm: &'a TriMesh,
    /// Boundary edge info by sorted vertex pair.
    boundary: HashMap<(usize, usize), &'a BoundaryEdge>,
}

impl Ctx<'_> {
    /// Point at fraction `f` (0 at `x`, 1 at `y`) along the original edge `x -> y`: on the curve for
    /// a boundary edge, straight otherwise.
    fn along(&self, x: usize, y: usize, f: f64) -> [f64; 3] {
        if let Some(e) = self.boundary.get(&key(x, y)) {
            let (t_x, t_y) = if e.a == x { (e.t_a, e.t_b) } else { (e.t_b, e.t_a) };
            let p = self.tm.segments[e.seg].0.point(t_x + f * (t_y - t_x));
            return [p[0], p[1], 0.0];
        }
        let (a, b) = (self.tm.pts[x], self.tm.pts[y]);
        [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1]), 0.0]
    }
}

/// Convert a triangulation to a mesh of `kind` (Tri3, Tri6, Quad4, Quad8 or Quad9).
pub fn convert(tm: &TriMesh, region: &Region, physics: Physics, kind: ElementKind) -> Result<Mesh, String> {
    if !matches!(kind, ElementKind::Tri3 | ElementKind::Tri6 | ElementKind::Quad4 | ElementKind::Quad8 | ElementKind::Quad9) || physics.dim() != 2 {
        return Err(format!("{kind:?} is not a 2D element for this analysis type"));
    }
    let ctx = Ctx { tm, boundary: tm.boundary.iter().map(|e| (key(e.a, e.b), e)).collect() };
    let mut mesh = Mesh::new(physics);
    for p in &tm.pts {
        mesh.add_node([p[0], p[1], 0.0]);
    }
    let mut mid: HashMap<(usize, usize), usize> = HashMap::new(); // midside node of an original edge
    let mut quarter: HashMap<(usize, usize), usize> = HashMap::new(); // node of the half edge of (x, y) next to x
    let mut conn: Vec<usize> = Vec::new();
    let mut edge_mid = |mesh: &mut Mesh, x: usize, y: usize| -> usize { *mid.entry(key(x, y)).or_insert_with(|| mesh.add_node(ctx.along(x, y, 0.5))) };
    match kind {
        ElementKind::Tri3 => {
            for t in &tm.tris {
                conn.extend(t);
            }
        }
        ElementKind::Tri6 => {
            for t in &tm.tris {
                conn.extend(t);
                for k in 0..3 {
                    conn.push(edge_mid(&mut mesh, t[k], t[(k + 1) % 3]));
                }
            }
        }
        _ => {
            for t in &tm.tris {
                let (a, b, c) = (t[0], t[1], t[2]);
                let (mab, mbc, mca) = (edge_mid(&mut mesh, a, b), edge_mid(&mut mesh, b, c), edge_mid(&mut mesh, c, a));
                let cen = [(tm.pts[a][0] + tm.pts[b][0] + tm.pts[c][0]) / 3.0, (tm.pts[a][1] + tm.pts[b][1] + tm.pts[c][1]) / 3.0, 0.0];
                let g = mesh.add_node(cen);
                // Quads (A, Mab, G, Mca), (B, Mbc, G, Mab), (C, Mca, G, Mbc), counter-clockwise.
                let corners = [[a, mab, g, mca], [b, mbc, g, mab], [c, mca, g, mbc]];
                if kind == ElementKind::Quad4 {
                    for q in &corners {
                        conn.extend(q);
                    }
                    continue;
                }
                // Quad8 / Quad9 midsides, in the order (0,1), (1,2), (2,3), (3,0). The quarter points of
                // the original edges sit on the geometry; the three spokes (Mxy-G) are straight.
                let mut near = |mesh: &mut Mesh, x: usize, y: usize| -> usize { *quarter.entry((x, y)).or_insert_with(|| mesh.add_node(ctx.along(x, y, 0.25))) };
                let (spab, spbc, spca) = (avg(&mesh.nodes[mab], &mesh.nodes[g]), avg(&mesh.nodes[mbc], &mesh.nodes[g]), avg(&mesh.nodes[mca], &mesh.nodes[g]));
                let (spab, spbc, spca) = (mesh.add_node(spab), mesh.add_node(spbc), mesh.add_node(spca));
                let sides = [
                    [near(&mut mesh, a, b), spab, spca, near(&mut mesh, a, c)],
                    [near(&mut mesh, b, c), spbc, spab, near(&mut mesh, b, a)],
                    [near(&mut mesh, c, a), spca, spbc, near(&mut mesh, c, b)],
                ];
                for (q, m) in corners.iter().zip(&sides) {
                    conn.extend(q);
                    conn.extend(m);
                    if kind == ElementKind::Quad9 {
                        let n = &mesh.nodes;
                        // Bilinear-blend centre of the serendipity geometry.
                        let cq: [f64; 3] = std::array::from_fn(|i| 0.5 * m.iter().map(|&k| n[k][i]).sum::<f64>() - 0.25 * q.iter().map(|&k| n[k][i]).sum::<f64>());
                        let cn = mesh.add_node(cq);
                        conn.push(cn);
                    }
                }
            }
        }
    }
    mesh.add_block(kind, conn, region.material, "domain")?;
    // Node sets and surfaces by segment name.
    for e in &tm.boundary {
        let name = tm.segments[e.seg].1.clone();
        let m = mid.get(&key(e.a, e.b)).copied();
        let (qa, qb) = (quarter.get(&(e.a, e.b)).copied(), quarter.get(&(e.b, e.a)).copied());
        let set = mesh.node_sets.entry(name.clone()).or_default();
        set.extend([e.a, e.b]);
        set.extend(m);
        set.extend(qa);
        set.extend(qb);
        let faces = mesh.surfaces.entry(name).or_default();
        match kind {
            ElementKind::Tri3 => faces.push(vec![e.a, e.b]),
            ElementKind::Tri6 => faces.push(vec![e.a, e.b, m.expect("boundary midside")]),
            ElementKind::Quad4 => {
                let m = m.expect("boundary midside");
                faces.push(vec![e.a, m]);
                faces.push(vec![m, e.b]);
            }
            _ => {
                let m = m.expect("boundary midside");
                faces.push(vec![e.a, m, qa.expect("quarter node")]);
                faces.push(vec![m, e.b, qb.expect("quarter node")]);
            }
        }
    }
    for set in mesh.node_sets.values_mut() {
        set.sort_unstable();
        set.dedup();
    }
    Ok(mesh)
}

fn avg(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), 0.0]
}
