//! A bounding-volume tree over triangles with nearest-triangle and range queries.
use crate::{dot, sub, V3};

/// Which part of a triangle a point is closest to (decides which pseudo-normal gives the sign).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Face,
    /// The edge between local vertices `i` and `j`.
    Edge(usize, usize),
    Vertex(usize),
}

/// Closest point of triangle `abc` to `p`, and the region it is in (Ericson, Real-Time Collision Detection).
pub fn closest_on_triangle(p: V3, a: V3, b: V3, c: V3) -> (V3, Region) {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, Region::Vertex(0));
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, Region::Vertex(1));
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return ([a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]], Region::Edge(0, 1));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, Region::Vertex(2));
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return ([a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]], Region::Edge(2, 0));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return ([b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]], Region::Edge(1, 2));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    ([a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w], Region::Face)
}

/// A tree node, one cache line. Leaf: `a` = first index into the leaf-ordered triangle arrays, `b` = count; inner: `a`/`b` =
/// left/right node indices. `leaf` is 1 for a leaf.
#[derive(Clone, Copy)]
#[repr(align(64))]
struct Node {
    lo: V3,
    hi: V3,
    a: u32,
    b: u32,
    leaf: u32,
}

pub struct Bvh {
    nodes: Vec<Node>,
    /// The triangles in leaf order (a leaf's triangles are contiguous), so a leaf is read sequentially instead of through
    /// `order` -> `triangles` -> three `vertices` loads.
    corners: Vec<[V3; 3]>,
    /// Original triangle index of each entry of `corners`.
    ids: Vec<u32>,
    /// Bounding box `[lo, hi]` of each entry of `corners`: a leaf triangle whose box is clearly farther than the best so far
    /// is skipped without the closest-point test.
    boxes: Vec<[V3; 2]>,
    /// Largest absolute coordinate of the mesh (scales the rounding margin of that skip).
    extent: f64,
}

/// Relative size of the rounding margin of the triangle-box skip: the closest point of a triangle is computed to a few ulps
/// of the coordinates' magnitude, so a box that is farther than `best + TOL * magnitude` cannot hold a triangle whose
/// computed squared distance is below `best`. Skipped triangles are provably ones the exact test would reject.
const TOL: f64 = 1e-12;

/// Squared distance bound above which the closest-point test of a triangle cannot give a result `< best2`.
fn skip_above(best2: f64, tol: f64) -> f64 {
    let r = best2.sqrt() + tol;
    r * r * (1.0 + TOL)
}

const LEAF: usize = 4;

fn box_dist2(lo: V3, hi: V3, p: V3) -> f64 {
    let mut d = 0.0;
    for k in 0..3 {
        let e = (lo[k] - p[k]).max(0.0).max(p[k] - hi[k]);
        d += e * e;
    }
    d
}

impl Bvh {
    pub fn build(vertices: &[V3], triangles: &[[usize; 3]]) -> Self {
        // node, triangle and leaf indices are `u32`: a surface this size would need more than 300 GB for its triangles alone
        assert!(triangles.len() < (1 << 31), "surface mesh too large for the BVH");
        let centre: Vec<V3> = triangles.iter().map(|t| [(vertices[t[0]][0] + vertices[t[1]][0] + vertices[t[2]][0]) / 3.0, (vertices[t[0]][1] + vertices[t[1]][1] + vertices[t[2]][1]) / 3.0, (vertices[t[0]][2] + vertices[t[1]][2] + vertices[t[2]][2]) / 3.0]).collect();
        let mut order: Vec<usize> = (0..triangles.len()).collect();
        let mut nodes: Vec<Node> = Vec::new();
        let mut stack = vec![(0usize, triangles.len(), usize::MAX, false)];
        // Iterative build; `(first, end, parent, is_right)`.
        while let Some((first, end, parent, right)) = stack.pop() {
            let id = nodes.len();
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for &t in &order[first..end] {
                for &v in &triangles[t] {
                    for k in 0..3 {
                        lo[k] = lo[k].min(vertices[v][k]);
                        hi[k] = hi[k].max(vertices[v][k]);
                    }
                }
            }
            nodes.push(Node { lo, hi, a: first as u32, b: (end - first) as u32, leaf: 1 });
            if parent != usize::MAX {
                if right {
                    nodes[parent].b = id as u32;
                } else {
                    nodes[parent].a = id as u32;
                }
            }
            if end - first > LEAF {
                let axis = (0..3).max_by(|&i, &j| (hi[i] - lo[i]).total_cmp(&(hi[j] - lo[j]))).unwrap_or(0);
                order[first..end].sort_by(|&x, &y| centre[x][axis].total_cmp(&centre[y][axis]));
                let mid = first + (end - first) / 2;
                nodes[id].leaf = 0;
                stack.push((mid, end, id, true));
                stack.push((first, mid, id, false));
            }
        }
        let corners: Vec<[V3; 3]> = order.iter().map(|&t| [vertices[triangles[t][0]], vertices[triangles[t][1]], vertices[triangles[t][2]]]).collect();
        let ids: Vec<u32> = order.iter().map(|&t| t as u32).collect();
        let boxes: Vec<[V3; 2]> = corners
            .iter()
            .map(|c: &[V3; 3]| {
                let (mut lo, mut hi) = (c[0], c[0]);
                for v in c {
                    for k in 0..3 {
                        lo[k] = lo[k].min(v[k]);
                        hi[k] = hi[k].max(v[k]);
                    }
                }
                [lo, hi]
            })
            .collect();
        let extent = vertices.iter().flatten().fold(0.0f64, |m: f64, x: &f64| m.max(x.abs()));
        Self { nodes, corners, ids, boxes, extent }
    }

    /// `(squared distance, triangle, region, closest point)` of the nearest triangle. Ties go to the triangle met first in
    /// the traversal (nearer child first), as before the leaf-ordered layout.
    pub fn nearest(&self, p: V3) -> (f64, usize, Region, V3) {
        let mut best = (f64::INFINITY, 0usize, Region::Face, p);
        // every level pops one node and pushes two, so the stack holds at most depth + 1 entries (depth is log2 of the triangles)
        let mut stack = [(0u32, 0.0f64); 128];
        let tol = TOL * self.extent.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
        let mut skip = f64::INFINITY;
        let root = &self.nodes[0];
        stack[0] = (0, box_dist2(root.lo, root.hi, p));
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let (n, d) = stack[top];
            // the box distance was computed when the node was pushed; `best` may have shrunk since
            if d >= best.0 {
                continue;
            }
            let node = &self.nodes[n as usize];
            if node.leaf != 0 {
                let (first, count) = (node.a as usize, node.b as usize);
                for (k, tri) in self.corners[first..first + count].iter().enumerate() {
                    let bx = &self.boxes[first + k];
                    if box_dist2(bx[0], bx[1], p) >= skip {
                        continue;
                    }
                    let (c, region) = closest_on_triangle(p, tri[0], tri[1], tri[2]);
                    let e = sub(p, c);
                    let d2 = dot(e, e);
                    if d2 < best.0 {
                        best = (d2, self.ids[first + k] as usize, region, c);
                        skip = skip_above(d2, tol);
                    }
                }
            } else {
                // Visit the nearer child last so it is popped first.
                let (l, r) = (&self.nodes[node.a as usize], &self.nodes[node.b as usize]);
                let (dl, dr) = (box_dist2(l.lo, l.hi, p), box_dist2(r.lo, r.hi, p));
                if dl < dr {
                    stack[top] = (node.b, dr);
                    stack[top + 1] = (node.a, dl);
                } else {
                    stack[top] = (node.a, dl);
                    stack[top + 1] = (node.b, dr);
                }
                top += 2;
            }
        }
        best
    }

    /// Call `f(triangle, squared distance, closest point)` for every triangle within `radius` of `p`.
    pub fn within(&self, p: V3, radius: f64, f: &mut dyn FnMut(usize, f64, V3)) {
        let r2 = radius * radius;
        let skip = skip_above(r2, TOL * self.extent.max(p[0].abs()).max(p[1].abs()).max(p[2].abs()));
        // the tree is balanced by a median split, so its depth is log2(triangles) and one child is pushed per level
        let mut stack = [0u32; 128];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if box_dist2(node.lo, node.hi, p) > r2 {
                continue;
            }
            if node.leaf != 0 {
                let (first, count) = (node.a as usize, node.b as usize);
                for (k, tri) in self.corners[first..first + count].iter().enumerate() {
                    let bx = &self.boxes[first + k];
                    if box_dist2(bx[0], bx[1], p) > skip {
                        continue;
                    }
                    let (c, _) = closest_on_triangle(p, tri[0], tri[1], tri[2]);
                    let e = sub(p, c);
                    let d2 = dot(e, e);
                    if d2 <= r2 {
                        f(self.ids[first + k] as usize, d2, c);
                    }
                }
            } else {
                stack[top] = node.a;
                stack[top + 1] = node.b;
                top += 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree, traversal and triangle test exactly as they were before the leaf-ordered layout, kept verbatim so the new
    /// code is compared with the original on bits: closest point, squared distance, triangle id (which of several tied
    /// triangles comes back), region, and the order in which `within` reports.
    mod reference {
        use super::super::{Region};
        use crate::{dot, sub, V3};

    /// Closest point of triangle `abc` to `p`, and the region it is in (Ericson, Real-Time Collision Detection).
    pub fn closest_on_triangle(p: V3, a: V3, b: V3, c: V3) -> (V3, Region) {
        let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
        let (d1, d2) = (dot(ab, ap), dot(ac, ap));
        if d1 <= 0.0 && d2 <= 0.0 {
            return (a, Region::Vertex(0));
        }
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        if d3 >= 0.0 && d4 <= d3 {
            return (b, Region::Vertex(1));
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            return ([a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]], Region::Edge(0, 1));
        }
        let cp = sub(p, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        if d6 >= 0.0 && d5 <= d6 {
            return (c, Region::Vertex(2));
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            return ([a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]], Region::Edge(2, 0));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            let bc = sub(c, b);
            return ([b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]], Region::Edge(1, 2));
        }
        let denom = 1.0 / (va + vb + vc);
        let (v, w) = (vb * denom, vc * denom);
        ([a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w], Region::Face)
    }

    #[derive(Clone, Copy)]
    struct Node {
        lo: V3,
        hi: V3,
        /// Leaf: `[first, count]` into `order`; inner: `[left, right]` node indices.
        a: usize,
        b: usize,
        leaf: bool,
    }

    pub struct Bvh {
        nodes: Vec<Node>,
        order: Vec<usize>,
    }

    const LEAF: usize = 4;

    fn box_dist2(lo: V3, hi: V3, p: V3) -> f64 {
        let mut d = 0.0;
        for k in 0..3 {
            let e = (lo[k] - p[k]).max(0.0).max(p[k] - hi[k]);
            d += e * e;
        }
        d
    }

    impl Bvh {
        pub fn build(vertices: &[V3], triangles: &[[usize; 3]]) -> Self {
            let centre: Vec<V3> = triangles.iter().map(|t| [(vertices[t[0]][0] + vertices[t[1]][0] + vertices[t[2]][0]) / 3.0, (vertices[t[0]][1] + vertices[t[1]][1] + vertices[t[2]][1]) / 3.0, (vertices[t[0]][2] + vertices[t[1]][2] + vertices[t[2]][2]) / 3.0]).collect();
            let mut order: Vec<usize> = (0..triangles.len()).collect();
            let mut nodes = Vec::new();
            let mut stack = vec![(0usize, triangles.len(), usize::MAX, false)];
            // Iterative build; `(first, end, parent, is_right)`.
            while let Some((first, end, parent, right)) = stack.pop() {
                let id = nodes.len();
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for &t in &order[first..end] {
                    for &v in &triangles[t] {
                        for k in 0..3 {
                            lo[k] = lo[k].min(vertices[v][k]);
                            hi[k] = hi[k].max(vertices[v][k]);
                        }
                    }
                }
                nodes.push(Node { lo, hi, a: first, b: end - first, leaf: true });
                if parent != usize::MAX {
                    if right {
                        nodes[parent].b = id;
                    } else {
                        nodes[parent].a = id;
                    }
                }
                if end - first > LEAF {
                    let axis = (0..3).max_by(|&i, &j| (hi[i] - lo[i]).total_cmp(&(hi[j] - lo[j]))).unwrap_or(0);
                    order[first..end].sort_by(|&x, &y| centre[x][axis].total_cmp(&centre[y][axis]));
                    let mid = first + (end - first) / 2;
                    nodes[id].leaf = false;
                    stack.push((mid, end, id, true));
                    stack.push((first, mid, id, false));
                }
            }
            Self { nodes, order }
        }

        /// `(squared distance, triangle, region, closest point)` of the nearest triangle.
        pub fn nearest(&self, vertices: &[V3], triangles: &[[usize; 3]], p: V3) -> (f64, usize, Region, V3) {
            let mut best = (f64::INFINITY, 0usize, Region::Face, p);
            // every level pops one node and pushes two, so the stack holds at most depth + 1 entries (depth is log2 of the triangles)
            let mut stack = [0usize; 128];
            let mut top = 1;
            while top > 0 {
                top -= 1;
                let n = stack[top];
                let node = self.nodes[n];
                if box_dist2(node.lo, node.hi, p) >= best.0 {
                    continue;
                }
                if node.leaf {
                    for &t in &self.order[node.a..node.a + node.b] {
                        let tri = triangles[t];
                        let (c, region) = closest_on_triangle(p, vertices[tri[0]], vertices[tri[1]], vertices[tri[2]]);
                        let d = sub(p, c);
                        let d2 = dot(d, d);
                        if d2 < best.0 {
                            best = (d2, t, region, c);
                        }
                    }
                } else {
                    // Visit the nearer child last so it is popped first.
                    let (l, r) = (node.a, node.b);
                    let (dl, dr) = (box_dist2(self.nodes[l].lo, self.nodes[l].hi, p), box_dist2(self.nodes[r].lo, self.nodes[r].hi, p));
                    if dl < dr {
                        stack[top] = r;
                        stack[top + 1] = l;
                    } else {
                        stack[top] = l;
                        stack[top + 1] = r;
                    }
                    top += 2;
                }
            }
            best
        }

        /// Call `f(triangle, squared distance, closest point)` for every triangle within `radius` of `p`.
        pub fn within(&self, vertices: &[V3], triangles: &[[usize; 3]], p: V3, radius: f64, f: &mut dyn FnMut(usize, f64, V3)) {
            let r2 = radius * radius;
            // the tree is balanced by a median split, so its depth is log2(triangles) and one child is pushed per level
            let mut stack = [0usize; 128];
            let mut top = 1;
            while top > 0 {
                top -= 1;
                let node = &self.nodes[stack[top]];
                if box_dist2(node.lo, node.hi, p) > r2 {
                    continue;
                }
                if node.leaf {
                    for &t in &self.order[node.a..node.a + node.b] {
                        let tri = triangles[t];
                        let (c, _) = closest_on_triangle(p, vertices[tri[0]], vertices[tri[1]], vertices[tri[2]]);
                        let d = sub(p, c);
                        let d2 = dot(d, d);
                        if d2 <= r2 {
                            f(t, d2, c);
                        }
                    }
                } else {
                    stack[top] = node.a;
                    stack[top + 1] = node.b;
                    top += 2;
                }
            }
        }
    }
    }

    // ---- the tests -------------------------------------------------------------------------------------------------------

    type Mesh = (Vec<V3>, Vec<[usize; 3]>);

    /// The 350-vertex, 740-triangle surface the NEMA17 bracket meshes against (dumped from `SurfaceSolid` during a real
    /// meshing run, `{:e}` text so every coordinate round-trips exactly).
    fn nema17() -> Mesh {
        let text = include_str!("../tests/data/bvh-nema17-surface.txt");
        let mut it = text.split_whitespace();
        let mut next = || it.next().expect("fixture ends early");
        let (nv, nt): (usize, usize) = (next().parse().unwrap(), next().parse().unwrap());
        let vertices = (0..nv).map(|_| [next().parse().unwrap(), next().parse().unwrap(), next().parse().unwrap()]).collect();
        let triangles = (0..nt).map(|_| [next().parse().unwrap(), next().parse().unwrap(), next().parse().unwrap()]).collect();
        (vertices, triangles)
    }

    /// A box cut into `n` x `n` squares per face: axis-aligned, so a query on a plane, edge or corner has many triangles at
    /// exactly the same distance (the tie-breaking case).
    fn grid_box(n: usize, size: V3) -> Mesh {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for axis in 0..3 {
            for side in [0.0, 1.0] {
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let base = vertices.len();
                for i in 0..=n {
                    for j in 0..=n {
                        let mut p = [0.0; 3];
                        p[axis] = side * size[axis];
                        p[u] = i as f64 / n as f64 * size[u];
                        p[v] = j as f64 / n as f64 * size[v];
                        vertices.push(p);
                    }
                }
                for i in 0..n {
                    for j in 0..n {
                        let a = base + i * (n + 1) + j;
                        let (b, c, d) = (a + 1, a + n + 1, a + n + 2);
                        triangles.push([a, b, d]);
                        triangles.push([a, d, c]);
                    }
                }
            }
        }
        (vertices, triangles)
    }

    fn icosphere(levels: usize) -> Mesh {
        let t = (1.0 + 5f64.sqrt()) / 2.0;
        let mut vertices: Vec<V3> = [[-1., t, 0.], [1., t, 0.], [-1., -t, 0.], [1., -t, 0.], [0., -1., t], [0., 1., t], [0., -1., -t], [0., 1., -t], [t, 0., -1.], [t, 0., 1.], [-t, 0., -1.], [-t, 0., 1.]]
            .iter()
            .map(|&v| unit(v))
            .collect();
        let mut triangles: Vec<[usize; 3]> = vec![
            [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
            [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
        ];
        for _ in 0..levels {
            let mut cache = std::collections::HashMap::new();
            let mut mid = |vertices: &mut Vec<V3>, a: usize, b: usize| {
                *cache.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    let m = unit([vertices[a][0] + vertices[b][0], vertices[a][1] + vertices[b][1], vertices[a][2] + vertices[b][2]]);
                    vertices.push(m);
                    vertices.len() - 1
                })
            };
            let mut next = Vec::new();
            for &[a, b, c] in &triangles {
                let (ab, bc, ca) = (mid(&mut vertices, a, b), mid(&mut vertices, b, c), mid(&mut vertices, c, a));
                next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
            }
            triangles = next;
        }
        // off-centre and stretched, so nothing is symmetric by accident
        (vertices.iter().map(|v| [3.0 * v[0] + 1.0, 2.0 * v[1] - 0.5, v[2] + 0.25]).collect(), triangles)
    }

    fn unit(a: V3) -> V3 {
        let n = dot(a, a).sqrt();
        [a[0] / n, a[1] / n, a[2] / n]
    }

    /// Deterministic generator (no `rand` dependency in this crate).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        fn point_in(&mut self, lo: V3, hi: V3) -> V3 {
            [lo[0] + (hi[0] - lo[0]) * self.next(), lo[1] + (hi[1] - lo[1]) * self.next(), lo[2] + (hi[2] - lo[2]) * self.next()]
        }
    }

    /// Query points of every kind the meshers produce: scattered through and around the part, on or beside the surface,
    /// exactly on vertices / edge midpoints / centroids (ties between triangles), and on the lattice planes of the grid box.
    fn query_points(mesh: &Mesh, seed: u64, per_kind: usize) -> Vec<V3> {
        let (vertices, triangles) = mesh;
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let size = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
        let mut rng = Lcg(seed);
        let mut points = Vec::new();
        // inside the box and well beyond it
        for scale in [0.0, 0.1, 1.0] {
            let (l, h) = ([lo[0] - scale * size, lo[1] - scale * size, lo[2] - scale * size], [hi[0] + scale * size, hi[1] + scale * size, hi[2] + scale * size]);
            for _ in 0..per_kind {
                points.push(rng.point_in(l, h));
            }
        }
        for _ in 0..per_kind {
            let tri = triangles[(rng.next() * triangles.len() as f64) as usize % triangles.len()];
            let (a, b, c) = (vertices[tri[0]], vertices[tri[1]], vertices[tri[2]]);
            let (mut u, mut v) = (rng.next(), rng.next());
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            let on = [a[0] + u * (b[0] - a[0]) + v * (c[0] - a[0]), a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]), a[2] + u * (b[2] - a[2]) + v * (c[2] - a[2])];
            let centroid = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
            let mid_ab = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0];
            let corner = [a, b, c][(rng.next() * 3.0) as usize % 3];
            for base in [on, centroid, mid_ab, corner] {
                points.push(base);
                // the mesher's own offsets: a fraction of an element off the surface, either side, and a few ulps
                for off in [1e-12, 1e-6, 1e-3, 0.02, 0.3] {
                    let d = [rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5];
                    let s = off * size;
                    points.push([base[0] + s * d[0], base[1] + s * d[1], base[2] + s * d[2]]);
                }
                points.push([f64::from_bits(base[0].to_bits() + 1), base[1], f64::from_bits(base[2].to_bits().wrapping_sub(1))]);
            }
        }
        // lattice points (multiples of size / 8 from the lower corner), on and off the faces of a box
        for i in 0..=10 {
            for j in 0..=10 {
                for k in 0..=10 {
                    let f = |x: usize, l: f64| l + (x as f64 - 1.0) * size / 8.0;
                    points.push([f(i, lo[0]), f(j, lo[1]), f(k, lo[2])]);
                }
            }
        }
        points
    }

    fn bits(v: V3) -> [u64; 3] {
        v.map(f64::to_bits)
    }

    fn check_mesh(name: &str, mesh: &Mesh, per_kind: usize) -> usize {
        let (vertices, triangles) = mesh;
        let old = reference::Bvh::build(vertices, triangles);
        let new = Bvh::build(vertices, triangles);
        let points = query_points(mesh, 0x9E37_79B9_7F4A_7C15 ^ triangles.len() as u64, per_kind);
        let size = {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for v in vertices {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max)
        };
        let (mut nearest_ties, mut within_hits) = (0usize, 0usize);
        for (n, &p) in points.iter().enumerate() {
            let (od, ot, or, oc) = old.nearest(vertices, triangles, p);
            let (nd, nt, nr, nc) = new.nearest(p);
            assert_eq!((od.to_bits(), ot, or, bits(oc)), (nd.to_bits(), nt, nr, bits(nc)), "{name}: nearest differs at query {n}, p = {p:?}");
            // another triangle at exactly the same distance: the tie-break (first met in the traversal) was exercised
            if n % 4 == 0 {
                nearest_ties += triangles.iter().enumerate().filter(|&(t, tri)| t != ot && {
                let (c, _) = reference::closest_on_triangle(p, vertices[tri[0]], vertices[tri[1]], vertices[tri[2]]);
                let e = sub(p, c);
                dot(e, e) == od
                }).count().min(1);
            }
            if n % 3 == 0 {
                for radius in [0.0, 1e-9 * size, 0.01 * size, 0.07 * size, 0.4 * size, 5.0 * size].into_iter().take(if n % 15 == 0 { 6 } else { 4 }) {
                    let (mut a, mut b) = (Vec::new(), Vec::new());
                    old.within(vertices, triangles, p, radius, &mut |t, d2, c| a.push((t, d2.to_bits(), bits(c))));
                    new.within(p, radius, &mut |t, d2, c| b.push((t, d2.to_bits(), bits(c))));
                    assert_eq!(a, b, "{name}: within differs at query {n}, p = {p:?}, radius {radius}");
                    within_hits += a.len();
                }
            }
        }
        eprintln!("{name}: {} triangles, {} queries identical ({nearest_ties} with a tie, {within_hits} within hits)", triangles.len(), points.len());
        assert!(within_hits > 0, "{name}: no within query found a triangle");
        nearest_ties
    }

    #[test]
    fn nearest_and_within_match_the_original_on_the_bracket_surface() {
        check_mesh("nema17", &nema17(), 1500);
    }

    /// The triangle-box skip has a rounding margin proportional to the coordinates' magnitude: a part far from the origin
    /// (rounded coordinates, distances tiny against them) and a part at a tiny scale must still agree on every bit.
    #[test]
    fn nearest_and_within_match_the_original_far_from_the_origin_and_at_tiny_scale() {
        let (vertices, triangles) = nema17();
        for (name, shift, scale) in [("nema17 + 1e6", 1.0e6, 1.0), ("nema17 + 1e9", 1.0e9, 1.0), ("nema17 x 1e-6", 0.0, 1.0e-6), ("nema17 x 1e-6 + 3", 3.0, 1.0e-6), ("nema17 x 1e4", -2.0e4, 1.0e4)] {
            let moved: Vec<V3> = vertices.iter().map(|v| v.map(|x| x * scale + shift)).collect();
            check_mesh(name, &(moved, triangles.clone()), 300);
        }
        let (vertices, triangles) = grid_box(4, [8.0, 8.0, 8.0]);
        let moved: Vec<V3> = vertices.iter().map(|v| v.map(|x| x + 1.0e7)).collect();
        check_mesh("grid box + 1e7", &(moved, triangles), 300);
    }

    #[test]
    fn nearest_and_within_match_the_original_where_triangles_tie() {
        // a lattice box: points on planes, edges and corners are equidistant from many triangles
        let ties = check_mesh("grid box", &grid_box(5, [8.0, 8.0, 8.0]), 1500) + check_mesh("grid box (rectangular)", &grid_box(4, [3.0, 7.0, 2.0]), 1500);
        assert!(ties > 100, "only {ties} queries had a tied triangle: the tie-break is not exercised");
    }

    #[test]
    fn nearest_and_within_match_the_original_on_a_sphere_and_tiny_meshes() {
        check_mesh("icosphere", &icosphere(3), 1500);
        check_mesh("icosahedron", &icosphere(0), 1500);
        check_mesh("box (12 triangles)", &grid_box(1, [1.0, 2.0, 3.0]), 1500);
    }

    /// `closest_on_triangle` is unchanged, and the triangle tests of a leaf are done on the leaf-ordered copies of the corners:
    /// every triangle id comes back through `ids` (a permutation).
    #[test]
    fn leaf_order_is_a_permutation_of_the_triangles() {
        let (vertices, triangles) = nema17();
        let bvh = Bvh::build(&vertices, &triangles);
        let mut seen = vec![false; triangles.len()];
        for (k, &id) in bvh.ids.iter().enumerate() {
            assert!(!std::mem::replace(&mut seen[id as usize], true));
            assert_eq!(bvh.corners[k], [vertices[triangles[id as usize][0]], vertices[triangles[id as usize][1]], vertices[triangles[id as usize][2]]]);
        }
        assert!(seen.iter().all(|&s| s));
    }

    /// Old against new on the queries of a real NEMA17 meshing run (`BVH_QUERIES`: lines `kind x y z radius`, kind 0 = signed
    /// distance / nearest, 1 = `faces_near`, 2 = nearest face; recorded by a temporary hook in `SurfaceSolid`, 2.4 M of the 19 M
    /// queries of the run). Alternating rounds in one process, so load hits both alike. Ignored: needs the recording.
    #[test]
    #[ignore = "micro-benchmark; needs BVH_QUERIES=<file>"]
    fn replay_recorded_queries() {
        let path = std::env::var("BVH_QUERIES").expect("BVH_QUERIES");
        let queries: Vec<(u8, V3, f64)> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split(' ').collect();
                (f[0].parse().unwrap(), [f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap()], f[4].parse().unwrap())
            })
            .collect();
        let (vertices, triangles) = nema17();
        let old = reference::Bvh::build(&vertices, &triangles);
        let new = Bvh::build(&vertices, &triangles);
        let run_old = || {
            let mut acc = 0.0;
            for &(kind, p, r) in &queries {
                if kind == 1 {
                    old.within(&vertices, &triangles, p, r, &mut |t, d2, _| acc += d2 + t as f64);
                } else {
                    acc += old.nearest(&vertices, &triangles, p).0;
                }
            }
            acc
        };
        let run_new = || {
            let mut acc = 0.0;
            for &(kind, p, r) in &queries {
                if kind == 1 {
                    new.within(p, r, &mut |t, d2, _| acc += d2 + t as f64);
                } else {
                    acc += new.nearest(p).0;
                }
            }
            acc
        };
        let (mut to, mut tn) = (Vec::new(), Vec::new());
        for _ in 0..7 {
            let t = std::time::Instant::now();
            let a = std::hint::black_box(run_old());
            to.push(t.elapsed().as_secs_f64());
            let t = std::time::Instant::now();
            let b = std::hint::black_box(run_new());
            tn.push(t.elapsed().as_secs_f64());
            assert_eq!(a.to_bits(), b.to_bits());
        }
        to.sort_by(f64::total_cmp);
        tn.sort_by(f64::total_cmp);
        eprintln!("{} queries: old median {:.3} s (min {:.3}), new median {:.3} s (min {:.3}), ratio {:.2}", queries.len(), to[3], to[0], tn[3], tn[0], tn[3] / to[3]);
    }
}
