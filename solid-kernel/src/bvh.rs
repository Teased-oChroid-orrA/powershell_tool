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
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
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
                    stack.push(r);
                    stack.push(l);
                } else {
                    stack.push(l);
                    stack.push(r);
                }
            }
        }
        best
    }

    /// Call `f(triangle, squared distance)` for every triangle within `radius` of `p`.
    pub fn within(&self, vertices: &[V3], triangles: &[[usize; 3]], p: V3, radius: f64, f: &mut dyn FnMut(usize, f64)) {
        let r2 = radius * radius;
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let node = self.nodes[n];
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
                        f(t, d2);
                    }
                }
            } else {
                stack.push(node.a);
                stack.push(node.b);
            }
        }
    }
}
