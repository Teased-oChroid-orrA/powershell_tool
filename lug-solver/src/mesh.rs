//! Global-local mapped Q9 mesh of the lug.
//!
//! Every node lies on a ray from the hole centre: column `c` is the ray at
//! angle `theta_c`, lattice row `i` the position along it. That one
//! construction gives all three zones the design asks for:
//!
//! * **Local (critical) region** - the first layers are concentric washer
//!   rings around the bore. Their radial thickness is set per ray to the
//!   local tangential element width (`a * dtheta`), so elements next to the
//!   bore are ~1:1.
//! * **Transition** - along each ray the layers grow geometrically
//!   (`h_k = h_1 q^(k-1)`), `q` solved per ray so the stack reaches the outline
//!   in exactly `n_layers` layers with `q <= max_growth`.
//! * **Global** - the last layer sits exactly on the lug outline (straight
//!   sides, corner arcs, far end). Angular stations are placed on the
//!   outline's critical angles so element edges conform to it.
//!
//! Q9 lattice rule (see `edge-check`'s `fem.rs`): corner nodes at the layer
//! and station boundaries, midside nodes at the element midpoints.

use crate::geometry::LugGeometry;
use std::f64::consts::PI;

/// A bushing filling the lug hole, as seen by the mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BushingMesh {
    /// Radius of its inner (pin-contact) surface.
    pub inner_radius: f64,
    /// Element layers through its wall.
    pub layers: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshSpec {
    /// Elements around the full bore circle at the coarse (far) spacing.
    pub elements_around: usize,
    /// Largest layer-to-layer growth ratio allowed in the transition.
    pub max_growth: f64,
    /// Radial / tangential element size of the first layer at the bore.
    pub first_layer_aspect: f64,
    /// Finer angular stations around the loaded sector (the contact patch).
    pub refine: Option<Refinement>,
}

impl Default for MeshSpec {
    fn default() -> Self {
        Self { elements_around: 72, max_growth: 1.25, first_layer_aspect: 1.0, refine: None }
    }
}

/// Fine angular spacing inside `centre +- half_width`, growing smoothly (about
/// 25 % per element) to the coarse spacing outside it. All angles in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Refinement {
    pub centre: f64,
    pub half_width: f64,
    pub fine_step: f64,
}

impl Refinement {
    /// Angular element size at polar angle `theta`, given the coarse step.
    fn step_at(&self, theta: f64, coarse: f64) -> f64 {
        let mut d = (theta - self.centre).rem_euclid(2.0 * PI);
        if d > PI {
            d = 2.0 * PI - d;
        }
        if d <= self.half_width {
            self.fine_step.min(coarse)
        } else {
            (self.fine_step + 0.25 * (d - self.half_width)).min(coarse)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub nodes: Vec<[f64; 2]>,
    /// Q9 connectivity, local order `3*b + a` (`a` radial, `b` angular).
    pub elems: Vec<[usize; 9]>,
    /// Number of lattice columns (rays), including midside columns.
    pub n_cols: usize,
    /// Lattice points per column: the bushing stack (if any) then the lug stack.
    pub n_rows: usize,
    /// Layers of the lug stack.
    pub n_layers: usize,
    /// Rows of the bushing stack per column (`2 * layers + 1`), 0 without a bushing.
    pub bush_rows: usize,
    /// Material group of every element: 0 lug, 1 bushing.
    pub elem_group: Vec<u8>,
    /// Conforming interface edges `(bushing outer edge, lug bore edge)`, node ids `[corner, mid, corner]`.
    pub interface_edges: Vec<([usize; 3], [usize; 3])>,
    /// Radius of the lug hole (the bushing's outer radius).
    pub hole_radius: f64,
    /// Closed ring (full model) versus a half model on `y >= 0`.
    pub periodic: bool,
    /// Angle of every column.
    pub theta: Vec<f64>,
    /// Node ids of each pin-contact edge `[corner, mid, corner]`, in order of increasing angle
    /// (the bushing's inner surface when there is a bushing, else the lug bore).
    pub bore_edges: Vec<[usize; 3]>,
    /// Radius of the pin-contact surface.
    pub bore_radius: f64,
    pub stats: MeshStats,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MeshStats {
    pub nodes: usize,
    pub elements: usize,
    pub n_layers: usize,
    /// Largest growth ratio actually used on any ray.
    pub max_growth: f64,
    /// Largest radial/tangential aspect among the first-layer elements.
    pub bore_aspect_max: f64,
    /// Largest element aspect anywhere.
    pub aspect_max: f64,
}

impl Mesh {
    #[inline]
    pub fn node_id(&self, col: usize, row: usize) -> usize {
        col * self.n_rows + row
    }

    /// Column of the lattice, wrapping around a closed ring.
    #[inline]
    pub fn col(&self, c: usize) -> usize {
        if self.periodic {
            c % self.n_cols
        } else {
            c
        }
    }

    pub fn bore_node(&self, col: usize) -> usize {
        self.node_id(col, 0)
    }

    /// Build the mesh. `half` meshes `y >= 0` only (axial load, symmetric lug).
    pub fn build(geom: &LugGeometry, spec: MeshSpec, half: bool) -> Result<Mesh, String> {
        Self::build_with(geom, spec, half, None)
    }

    /// As [`build`](Self::build), optionally with a bushing of inner radius `bushing.inner_radius`
    /// filling the hole: `bushing.layers` rings of the same mapped grid between that radius and
    /// the hole, on their own nodes so the interface can be a contact.
    pub fn build_with(geom: &LugGeometry, spec: MeshSpec, half: bool, bushing: Option<BushingMesh>) -> Result<Mesh, String> {
        geom.validate()?;
        if let Some(b) = bushing {
            if !(b.inner_radius > 0.0 && b.inner_radius < geom.bore_radius() * (1.0 - 1e-6)) {
                return Err("the bushing's inner radius must be positive and smaller than the hole radius".into());
            }
            if b.layers == 0 {
                return Err("the bushing needs at least one layer".into());
            }
        }
        if spec.elements_around < 8 {
            return Err("elements_around must be at least 8".into());
        }
        if spec.max_growth <= 1.0 || !spec.max_growth.is_finite() {
            return Err("max_growth must exceed 1".into());
        }
        let a = geom.bore_radius();
        let dtheta_t = 2.0 * PI / spec.elements_around as f64;

        // Angular element boundaries on the critical angles.
        let (lo, hi) = (0.0, if half { PI } else { 2.0 * PI });
        let mut crit: Vec<f64> = geom.critical_angles().into_iter().filter(|t| *t >= lo - 1e-12 && *t <= hi + 1e-12).collect();
        if crit.first().is_none_or(|t| *t > lo + 1e-12) {
            crit.insert(0, lo);
        }
        if crit.last().is_none_or(|t| *t < hi - 1e-12) {
            crit.push(hi);
        }
        let mut stations: Vec<f64> = vec![crit[0]];
        for w in crit.windows(2) {
            let (alpha, beta) = (w[0], w[1]);
            let span = beta - alpha;
            let pts: Vec<f64> = match spec.refine {
                None => {
                    let n = ((span / dtheta_t).round() as usize).max(1);
                    (1..=n).map(|k| alpha + span * k as f64 / n as f64).collect()
                }
                Some(r) => {
                    // March from alpha with the local step, then stretch to land exactly on beta.
                    let mut raw = vec![alpha];
                    let mut t = alpha;
                    while t < beta - 1e-12 {
                        t += r.step_at(t, dtheta_t);
                        raw.push(t);
                    }
                    if raw.len() >= 3 && beta - raw[raw.len() - 2] < 0.5 * (raw[raw.len() - 2] - raw[raw.len() - 3]) {
                        raw.pop(); // fold a sliver into its neighbour
                    }
                    let last = *raw.last().unwrap();
                    let scale = span / (last - alpha);
                    raw.iter().skip(1).map(|x| alpha + (x - alpha) * scale).collect()
                }
            };
            stations.extend(pts);
        }
        let n_ang = stations.len() - 1;
        // Lattice columns: stations plus element midpoints.
        let mut theta: Vec<f64> = Vec::with_capacity(2 * n_ang + 1);
        for k in 0..n_ang {
            theta.push(stations[k]);
            theta.push(0.5 * (stations[k] + stations[k + 1]));
        }
        if half {
            theta.push(stations[n_ang]);
        }
        let n_cols = theta.len();

        // First-layer thickness per ray: ~1:1 with the local tangential element size.
        let col_h1: Vec<f64> = (0..n_cols)
            .map(|c| {
                let local = if c % 2 == 1 {
                    // midside column: the span of its own element
                    let next = if c + 1 < n_cols { theta[c + 1] } else { theta[0] + 2.0 * PI };
                    next - theta[c - 1]
                } else {
                    let lo = if c == 0 { if half { 0.0 } else { theta[n_cols - 1] - 2.0 * PI } } else { theta[c - 1] };
                    let hi = if c == n_cols - 1 { if half { 0.0 } else { theta[0] + 2.0 * PI } } else { theta[c + 1] };
                    let (lo_span, hi_span) = (theta[c] - lo, hi - theta[c]);
                    // end columns of a half model see one side only
                    match (c == 0 && half, c == n_cols - 1 && half) {
                        (true, _) => hi_span * 2.0,
                        (_, true) => lo_span * 2.0,
                        _ => lo_span + hi_span,
                    }
                };
                (spec.first_layer_aspect.max(0.1) * a * local.abs()).max(1e-9)
            })
            .collect();

        // Layer count from the ray needing the most layers at the allowed growth.
        let n_layers = theta
            .iter()
            .zip(&col_h1)
            .map(|(t, h1c)| layers_for((geom.ray_distance(*t) - a) / h1c, spec.max_growth))
            .max()
            .unwrap_or(4)
            .max(4);

        let bush_layers = bushing.map_or(0, |b| b.layers);
        let bush_rows = if bushing.is_some() { 2 * bush_layers + 1 } else { 0 };
        let lug_rows = 2 * n_layers + 1;
        let n_rows = bush_rows + lug_rows;
        let mut nodes = vec![[0.0f64; 2]; n_cols * n_rows];
        let mut max_q = 1.0f64;
        for (c, &t) in theta.iter().enumerate() {
            let h = geom.ray_distance(t) - a;
            let (q, g) = layer_fractions(col_h1[c] / h, n_layers);
            max_q = max_q.max(q);
            let (ct, st) = (t.cos(), t.sin());
            // Bushing stack: uniform layers from its inner radius to the hole radius.
            if let Some(b) = bushing {
                for i in 0..bush_rows {
                    let r = b.inner_radius + (a - b.inner_radius) * i as f64 / (bush_rows - 1) as f64;
                    nodes[c * n_rows + i] = [r * ct, r * st];
                }
            }
            for i in 0..lug_rows {
                let frac = if i % 2 == 0 { g[i / 2] } else { 0.5 * (g[i / 2] + g[i / 2 + 1]) };
                let r = a + h * frac;
                nodes[c * n_rows + bush_rows + i] = [r * ct, r * st];
            }
            // Exact outline point on the last row (avoids round-off off the boundary).
            nodes[c * n_rows + n_rows - 1] = [(a + h) * ct, (a + h) * st];
        }

        let pin_radius = bushing.map_or(a, |b| b.inner_radius);
        let mut mesh = Mesh {
            nodes,
            elems: Vec::new(),
            n_cols,
            n_rows,
            n_layers,
            bush_rows,
            elem_group: Vec::new(),
            interface_edges: Vec::new(),
            hole_radius: a,
            periodic: !half,
            theta,
            bore_edges: Vec::new(),
            bore_radius: pin_radius,
            stats: MeshStats::default(),
        };

        for jj in 0..n_ang {
            let cols = [mesh.col(2 * jj), mesh.col(2 * jj + 1), mesh.col(2 * jj + 2)];
            mesh.bore_edges.push([mesh.node_id(cols[0], 0), mesh.node_id(cols[1], 0), mesh.node_id(cols[2], 0)]);
            if bushing.is_some() {
                let (rb, rl) = (bush_rows - 1, bush_rows);
                mesh.interface_edges.push((
                    [mesh.node_id(cols[0], rb), mesh.node_id(cols[1], rb), mesh.node_id(cols[2], rb)],
                    [mesh.node_id(cols[0], rl), mesh.node_id(cols[1], rl), mesh.node_id(cols[2], rl)],
                ));
            }
            // Bushing layers (group 1), then lug layers (group 0).
            for k in 0..bush_layers + n_layers {
                let (row0, group) = if k < bush_layers { (2 * k, 1u8) } else { (bush_rows + 2 * (k - bush_layers), 0u8) };
                let mut e = [0usize; 9];
                for (b, &col) in cols.iter().enumerate() {
                    for ia in 0..3 {
                        e[3 * b + ia] = mesh.node_id(col, row0 + ia);
                    }
                }
                mesh.elems.push(e);
                mesh.elem_group.push(group);
            }
        }

        mesh.stats = mesh.measure(max_q)?;
        Ok(mesh)
    }

    /// Quality metrics; also proves every element is a valid (positive-Jacobian) quad.
    fn measure(&self, max_growth: f64) -> Result<MeshStats, String> {
        let mut aspect_max: f64 = 1.0;
        let mut bore_aspect_max: f64 = 1.0;
        for (idx, e) in self.elems.iter().enumerate() {
            let xy: Vec<[f64; 2]> = e.iter().map(|&n| self.nodes[n]).collect();
            // Jacobian columns at the element centre (node 4 is the centre of the lattice).
            let (_, dxi, deta) = edge_check::fem::shape_q9(0.0, 0.0);
            let (mut a11, mut a12, mut a21, mut a22) = (0.0, 0.0, 0.0, 0.0);
            for n in 0..9 {
                a11 += dxi[n] * xy[n][0];
                a12 += dxi[n] * xy[n][1];
                a21 += deta[n] * xy[n][0];
                a22 += deta[n] * xy[n][1];
            }
            let (lx, ly) = (a11.hypot(a12), a21.hypot(a22));
            if lx <= 0.0 || ly <= 0.0 || a11 * a22 - a12 * a21 <= 0.0 {
                return Err(format!("element {idx} is degenerate or inverted"));
            }
            let asp = if lx > ly { lx / ly } else { ly / lx };
            aspect_max = aspect_max.max(asp);
            if e[0] % self.n_rows == 0 {
                bore_aspect_max = bore_aspect_max.max(asp);
            }
        }
        Ok(MeshStats { nodes: self.nodes.len(), elements: self.elems.len(), n_layers: self.n_layers, max_growth, bore_aspect_max, aspect_max })
    }
}

/// Layers needed so a ray `h_over_h1` first-layer thicknesses long can be
/// covered with growth no larger than `q_max`.
fn layers_for(h_over_h1: f64, q_max: f64) -> usize {
    if h_over_h1 <= 1.0 {
        return 1;
    }
    let n = (1.0 + h_over_h1 * (q_max - 1.0)).ln() / q_max.ln();
    n.ceil().max(1.0) as usize
}

/// Cumulative layer boundaries `g_0 = 0 .. g_n = 1` of a geometric stack whose
/// first layer is `target` of the whole (`target = h1 / H`). Returns the growth
/// ratio and the boundaries; uniform layers if the target cannot be reached.
fn layer_fractions(target: f64, n: usize) -> (f64, Vec<f64>) {
    let nf = n as f64;
    if target >= 1.0 / nf {
        return (1.0, (0..=n).map(|k| k as f64 / nf).collect());
    }
    // (q - 1) / (q^n - 1) decreases from 1/n (q -> 1) as q grows.
    let f = |q: f64| (q - 1.0) / (q.powf(nf) - 1.0);
    let (mut lo, mut hi) = (1.0 + 1e-9, 8.0);
    if f(hi) > target {
        return (hi, geometric(hi, n));
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let q = 0.5 * (lo + hi);
    (q, geometric(q, n))
}

fn geometric(q: f64, n: usize) -> Vec<f64> {
    let total = q.powi(n as i32) - 1.0;
    (0..=n).map(|k| (q.powi(k as i32) - 1.0) / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lug() -> LugGeometry {
        LugGeometry::round_head(0.5, 1.5, 0.25, 3.0)
    }

    #[test]
    fn layer_fractions_hit_the_requested_first_layer_and_end_at_one() {
        let (q, g) = layer_fractions(0.02, 14);
        assert!(q > 1.0 && (g[0]).abs() < 1e-15 && (g[14] - 1.0).abs() < 1e-12);
        assert!((g[1] - 0.02).abs() < 1e-6, "first layer {}", g[1]);
        for w in g.windows(3) {
            assert!(((w[2] - w[1]) / (w[1] - w[0]) - q).abs() < 1e-6, "constant growth ratio");
        }
    }

    #[test]
    fn short_rays_fall_back_to_uniform_layers() {
        let (q, g) = layer_fractions(0.5, 4);
        assert_eq!(q, 1.0);
        assert!((g[1] - 0.25).abs() < 1e-15);
    }

    #[test]
    fn full_mesh_area_equals_outline_minus_hole() {
        let g = lug();
        let mesh = Mesh::build(&g, MeshSpec::default(), false).unwrap();
        let area = mesh_area(&mesh);
        let exact = g.outline_area() - PI * g.bore_radius().powi(2);
        assert!((area - exact).abs() / exact < 2e-4, "{area} vs {exact}");
    }

    #[test]
    fn half_mesh_area_is_half_of_the_full_mesh() {
        let g = lug();
        let full = mesh_area(&Mesh::build(&g, MeshSpec::default(), false).unwrap());
        let half = mesh_area(&Mesh::build(&g, MeshSpec::default(), true).unwrap());
        assert!((2.0 * half - full).abs() / full < 1e-9, "{half} {full}");
    }

    #[test]
    fn bore_elements_are_about_square_and_growth_is_bounded() {
        let mesh = Mesh::build(&lug(), MeshSpec::default(), false).unwrap();
        let s = mesh.stats;
        assert!(s.bore_aspect_max < 1.6, "bore aspect {}", s.bore_aspect_max);
        assert!(s.max_growth <= 1.25 + 1e-6, "growth {}", s.max_growth);
        assert!(s.aspect_max < 12.0, "global aspect {}", s.aspect_max);
    }

    #[test]
    fn outline_nodes_lie_on_the_outline_and_bore_nodes_on_the_bore() {
        let g = lug();
        let mesh = Mesh::build(&g, MeshSpec::default(), false).unwrap();
        for c in 0..mesh.n_cols {
            let b = mesh.nodes[mesh.node_id(c, 0)];
            assert!((b[0].hypot(b[1]) - g.bore_radius()).abs() < 1e-12);
            let o = mesh.nodes[mesh.node_id(c, mesh.n_rows - 1)];
            let th = o[1].atan2(o[0]);
            assert!((o[0].hypot(o[1]) - g.ray_distance(th)).abs() < 1e-9);
        }
    }

    #[test]
    fn elements_and_bore_edges_are_consistent() {
        let mesh = Mesh::build(&lug(), MeshSpec::default(), false).unwrap();
        assert_eq!(mesh.elems.len(), mesh.bore_edges.len() * mesh.n_layers);
        assert_eq!(mesh.bore_edges.len() * 2, mesh.n_cols, "closed ring: two columns per element");
        let half = Mesh::build(&lug(), MeshSpec::default(), true).unwrap();
        assert_eq!(half.bore_edges.len() * 2 + 1, half.n_cols);
    }

    /// Area by 3x3 Gauss over every element.
    pub fn mesh_area(mesh: &Mesh) -> f64 {
        let mut area = 0.0;
        for e in &mesh.elems {
            for &(gx, wx) in &edge_check::fem::GAUSS3 {
                for &(gy, wy) in &edge_check::fem::GAUSS3 {
                    let (_, dxi, deta) = edge_check::fem::shape_q9(gx, gy);
                    let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                    for n in 0..9 {
                        let p = mesh.nodes[e[n]];
                        j11 += dxi[n] * p[0];
                        j12 += dxi[n] * p[1];
                        j21 += deta[n] * p[0];
                        j22 += deta[n] * p[1];
                    }
                    area += (j11 * j22 - j12 * j21) * wx * wy;
                }
            }
        }
        area
    }
}
