//! Conforming Delaunay recovery of a closed triangulated PLC by Steiner points.
//!
//! Every PLC segment is split (concentric-shell points of encroached segments) and
//! every input facet keeps its own planar Delaunay triangulation (the sub-facets);
//! encroached or missing sub-facets receive their circumcentre, until each
//! sub-segment is an edge and each sub-facet a face of the Delaunay
//! tetrahedralization; the tetrahedra inside the shell are then extracted. The result is verified by exact set membership and
//! by the divergence-theorem volume, never by the heuristics that drove it.
//!
//! Termination is guaranteed only for inputs without acute segment/facet angles;
//! otherwise the Steiner budget refuses explicitly. Steiner points are computed in
//! binary64, so they lie on their parent segment/facet up to rounding; the largest
//! relative deviation is reported. No quality bound (slivers) is established.
use crate::delaunay3::Delaunay3;
use crate::{
    step::{Error, ErrorKind},
    V3,
};
use fea_core::meshing::{
    tet_delaunay::{tetrahedralize, Limits as DelaunayLimits},
    tet_flip::{improve, FlipOptions, FlipReport},
    tet_topology::{
        in_circle_2d, orientation, orientation_2d, sphere_side, SphereSide, TetTopology,
    },
    MeshError,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub max_steiner: usize,
    pub max_cells: usize,
    /// Split every sub-segment/sub-facet whose diametral sphere holds a vertex
    /// (Shewchuk's rule, guaranteed Delaunay-conforming for well-conditioned
    /// inputs). When false only those absent from the triangulation are split;
    /// this needs far fewer Steiner points on surface-mesh inputs but is only a
    /// heuristic, ended by the exact final presence check either way.
    pub protect_encroached: bool,
    /// Also insert circumcentres of interior tetrahedra whose circumradius to
    /// shortest-edge ratio exceeds this bound (Delaunay refinement; centres that
    /// would encroach a sub-segment or sub-facet split those instead). `None`
    /// leaves the recovered cells as they are. Slivers are not addressed.
    pub max_radius_edge: Option<f64>,
    /// Reconnect low-quality interior cells by 2-3 flips and edge removals after
    /// extraction (boundary, sub-facets and vertices are preserved exactly).
    pub reconnect: Option<FlipOptions>,
    /// A sub-segment shorter than this fraction of the input's bounding-box diagonal is never split: refinement that has
    /// descended that far (a cascade of concentric shells around a vertex) produces elements no solver can use, and the
    /// recovery stops with an error instead.
    pub min_segment_relative: f64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_steiner: 4000,
            max_cells: 400_000,
            protect_encroached: true,
            max_radius_edge: None,
            reconnect: None,
            min_segment_relative: 1e-6,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Delaunay,
    Segments,
    Facets,
    Extraction,
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub phase: Phase,
    pub steiner: usize,
    pub missing_segments: usize,
    pub missing_facets: usize,
}
#[derive(Debug, Clone)]
pub struct Report {
    pub input_vertices: usize,
    pub segment_steiner: usize,
    pub facet_steiner: usize,
    pub rounds: usize,
    /// Largest Steiner offset from its parent segment line / facet plane,
    /// relative to the parent's longest edge.
    pub max_steiner_offset_relative: f64,
    /// Steiner points added by quality refinement (included in neither count above).
    pub quality_steiner: usize,
    /// Largest circumradius / shortest-edge ratio over the interior cells.
    pub max_radius_edge_ratio: f64,
    /// Interior cells above `max_radius_edge` whose centre could not be inserted.
    pub unresolved_bad_cells: usize,
    /// Result of the optional reconnection pass over the interior cells.
    pub reconnection: Option<FlipReport>,
    pub hull_cells: usize,
    pub interior_cells: usize,
    /// |sum of cell volumes - PLC enclosed volume| / PLC enclosed volume.
    pub volume_error_relative: f64,
}
/// Conforming tetrahedralization of the solid enclosed by the input shell.
#[derive(Debug)]
pub struct Conforming {
    pub points: Vec<V3>,
    /// Positively oriented tetrahedra inside the shell.
    pub cells: Vec<[usize; 4]>,
    /// Sub-facets (outward orientation as given) with the index of the input facet.
    pub facets: Vec<([usize; 3], usize)>,
    pub segments: Vec<[usize; 2]>,
    pub report: Report,
}
/// Smallest angle, in degrees, between any two segments (triangle corners) or
/// facets (dihedral angles measured on either side) of a closed outward-oriented
/// triangulated shell. This is the input-angle measure Delaunay refinement
/// depends on; `fea_core::meshing::compatibility` gates the native strategy on it.
pub fn min_shell_angle_deg(vertices: &[V3], facets: &[[usize; 3]]) -> f64 {
    let unit = |v: V3| {
        let n = norm2(v).sqrt();
        v.map(|x| x / n)
    };
    let mut smallest = std::f64::consts::PI;
    let mut normals = Vec::with_capacity(facets.len());
    for f in facets {
        let [a, b, c] = f.map(|v| vertices[v]);
        for (p, q, r) in [(a, b, c), (b, c, a), (c, a, b)] {
            let (u, w) = (unit(sub(q, p)), unit(sub(r, p)));
            smallest = smallest.min(dot(u, w).clamp(-1., 1.).acos());
        }
        normals.push(unit(cross(sub(b, a), sub(c, a))));
    }
    let mut by_edge = BTreeMap::<[usize; 2], Vec<(usize, bool)>>::new();
    for (i, f) in facets.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            by_edge.entry(edge(a, b)).or_default().push((i, a < b));
        }
    }
    for (e, owners) in by_edge {
        if let [(i, forward), (j, _)] = owners.as_slice() {
            let dir = unit(sub(vertices[e[1]], vertices[e[0]]));
            let (n1, n2) = (normals[*i], normals[*j]);
            // Signed angle from n1 to n2 about the edge as seen along facet i's direction.
            let along = if *forward { dir } else { dir.map(|x| -x) };
            let phi = dot(cross(n1, n2), along).atan2(dot(n1, n2));
            let interior = std::f64::consts::PI - phi;
            let tau = 2. * std::f64::consts::PI;
            let hazard = interior.rem_euclid(tau).min(tau - interior.rem_euclid(tau));
            smallest = smallest.min(hazard);
        }
    }
    smallest.to_degrees()
}
impl Conforming {
    /// Tet4 solver mesh of the extracted cells, through the existing conformity
    /// and quality gates (a sliver beyond them is refused, not repaired). The
    /// whole boundary is the surface set `boundary`.
    pub fn to_solver_mesh(&self, material: fea_core::Elastic) -> Result<fea_core::Mesh, MeshError> {
        TetTopology::new(
            self.points.clone(),
            self.cells.clone(),
            self.cells.len() + 1,
        )?
        .to_mesh(material)
    }
}
fn fail(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        offset: 0,
        kind,
        detail: detail.into(),
    }
}
fn mesh_error(e: MeshError) -> Error {
    match e {
        MeshError::LimitExceeded(m) => fail(ErrorKind::Limit, m),
        other => fail(ErrorKind::Geometry, other.to_string()),
    }
}
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: V3, b: V3) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}
fn norm2(a: V3) -> f64 {
    dot(a, a)
}
fn edge(a: usize, b: usize) -> [usize; 2] {
    [a.min(b), a.max(b)]
}
fn face_key(mut f: [usize; 3]) -> [usize; 3] {
    f.sort_unstable();
    f
}
/// Strictly-inside test with a small inward margin: a vertex on the diametral
/// sphere (right angles, cocircular grids) is not encroaching, which would
/// otherwise refine such inputs forever. A segment that is then missing from the
/// triangulation because of a tie is still split by the `missing` rule.
fn encroaches(pa: V3, pb: V3, p: V3) -> bool {
    let centre: V3 = std::array::from_fn(|i| (pa[i] + pb[i]) * 0.5);
    norm2(sub(p, centre)) < norm2(sub(pa, pb)) * 0.25 * (1. - 1e-9)
}
fn circumcentre(a: V3, b: V3, c: V3) -> Option<V3> {
    let (u, v) = (sub(b, a), sub(c, a));
    let n = cross(u, v);
    let d = 2. * norm2(n);
    if !(d.is_finite() && d > 0.) {
        return None;
    }
    let (uu, vv) = (norm2(u), norm2(v));
    let t = cross(n, u);
    let w = cross(v, n);
    Some(std::array::from_fn(|i| a[i] + (vv * t[i] + uu * w[i]) / d))
}
/// Circumcentre and circumradius / shortest-edge ratio of a tetrahedron.
fn tet_circumsphere(p: [V3; 4]) -> Option<(V3, f64)> {
    let (a, b, c) = (sub(p[1], p[0]), sub(p[2], p[0]), sub(p[3], p[0]));
    let det = 2. * dot(a, cross(b, c));
    if !(det.is_finite() && det != 0.) {
        return None;
    }
    let (ab, ca, bc) = (cross(b, c), cross(c, a), cross(a, b));
    let off: V3 =
        std::array::from_fn(|i| (norm2(a) * ab[i] + norm2(b) * ca[i] + norm2(c) * bc[i]) / det);
    let shortest = (0..4)
        .flat_map(|i| ((i + 1)..4).map(move |j| (i, j)))
        .map(|(i, j)| norm2(sub(p[i], p[j])))
        .fold(f64::MAX, f64::min)
        .sqrt();
    let centre = std::array::from_fn(|i| p[0][i] + off[i]);
    Some((centre, norm2(off).sqrt() / shortest))
}
/// Which tetrahedra lie outside the shell. The cells on the inner side of each
/// (outward-oriented) sub-facet are inside, and inside spreads across every face
/// that is not a sub-facet; a sliver hull face created by rounding cannot leak.
fn outside_cells(
    cells: &[[usize; 4]],
    neighbors: &[[Option<usize>; 4]],
    facets: &[[usize; 3]],
    points: &[V3],
) -> Vec<bool> {
    let faces_of = |t: [usize; 4]| {
        let [a, b, c, d] = t;
        [[b, c, d], [a, d, c], [a, b, d], [a, c, b]]
    };
    let facet_keys: BTreeSet<[usize; 3]> = facets.iter().map(|f| face_key(*f)).collect();
    let mut owners = BTreeMap::<[usize; 3], Vec<(usize, usize)>>::new();
    for (i, t) in cells.iter().enumerate() {
        for (k, f) in faces_of(*t).into_iter().enumerate() {
            let key = face_key(f);
            if facet_keys.contains(&key) {
                owners.entry(key).or_default().push((i, k));
            }
        }
    }
    let mut inside = vec![false; cells.len()];
    let mut stack = Vec::new();
    for f in facets {
        for &(i, k) in owners.get(&face_key(*f)).map_or(&[][..], |v| v) {
            let apex = points[cells[i][k]];
            let [a, b, c] = f.map(|v| points[v]);
            // The interior side of an outward face is the Less side.
            if orientation(a, b, c, apex) == Ok(Ordering::Less) && !inside[i] {
                inside[i] = true;
                stack.push(i);
            }
        }
    }
    while let Some(i) = stack.pop() {
        for (k, f) in faces_of(cells[i]).into_iter().enumerate() {
            if let Some(j) = neighbors[i][k] {
                if !inside[j] && !facet_keys.contains(&face_key(f)) {
                    inside[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    inside.into_iter().map(|x| !x).collect()
}
/// Planar Delaunay triangulation of one input facet and the Steiner points on it.
struct FacetMesh {
    parent: usize,
    origin: V3,
    e1: V3,
    e2: V3,
    /// The three input corners, counter-clockwise in the facet frame.
    corners: [usize; 3],
    uv: BTreeMap<usize, [f64; 2]>,
    tris: Vec<[usize; 3]>,
}
impl FacetMesh {
    fn new(parent: usize, f: [usize; 3], points: &[V3]) -> Result<Self, Error> {
        let [a, b, c] = f.map(|v| points[v]);
        let n = cross(sub(b, a), sub(c, a));
        let (nn, ll) = (norm2(n).sqrt(), norm2(sub(b, a)).sqrt());
        if !(nn.is_finite() && nn > 0. && ll > 0.) {
            return Err(fail(ErrorKind::Geometry, "degenerate PLC facet"));
        }
        let e1: V3 = sub(b, a).map(|x| x / ll);
        let e2 = cross(n.map(|x| x / nn), e1);
        let mut mesh = Self {
            parent,
            origin: a,
            e1,
            e2,
            corners: f,
            uv: BTreeMap::new(),
            tris: vec![f],
        };
        for v in f {
            mesh.uv.insert(v, mesh.project(points[v]));
        }
        Ok(mesh)
    }
    fn project(&self, p: V3) -> [f64; 2] {
        let d = sub(p, self.origin);
        [dot(d, self.e1), dot(d, self.e2)]
    }
    /// Bowyer–Watson insertion of a point on or inside the facet. A point on a
    /// facet edge replaces the two degenerate cone triangles by nothing, so the
    /// edge is subdivided.
    fn insert(&mut self, id: usize, p: V3, on_edge: Option<[usize; 2]>) -> Result<(), Error> {
        let q = self.project(p);
        let uv = |v: usize| self.uv[&v];
        // Seed: the triangle on the split edge (even when rounding puts the point a
        // hair outside its circle) or the triangle containing the point. The
        // cavity is the connected set of triangles whose circle strictly holds it.
        let seed = on_edge
            .and_then(|e| {
                (0..self.tris.len()).find(|&i| {
                    let t = self.tris[i];
                    (0..3).any(|k| edge(t[k], t[(k + 1) % 3]) == e)
                })
            })
            .or_else(|| {
                (0..self.tris.len()).find(|&i| {
                    let t = self.tris[i];
                    (0..3)
                        .all(|k| orientation_2d(uv(t[k]), uv(t[(k + 1) % 3]), q) != Ordering::Less)
                })
            });
        let mut across = BTreeMap::<[usize; 2], usize>::new();
        for (i, t) in self.tris.iter().enumerate() {
            for k in 0..3 {
                across.insert([t[k], t[(k + 1) % 3]], i);
            }
        }
        let mut cavity: Vec<usize> = seed.into_iter().collect();
        let mut next_to_visit = 0;
        while next_to_visit < cavity.len() {
            let t = self.tris[cavity[next_to_visit]];
            next_to_visit += 1;
            for k in 0..3 {
                let Some(&j) = across.get(&[t[(k + 1) % 3], t[k]]) else {
                    continue;
                };
                if cavity.contains(&j) {
                    continue;
                }
                let [a, b, c] = self.tris[j].map(uv);
                if in_circle_2d(a, b, c, q) == Ordering::Greater {
                    cavity.push(j);
                }
            }
        }
        if cavity.is_empty() {
            return Err(fail(
                ErrorKind::Geometry,
                format!(
                    "Steiner point lies outside its facet (facet {}, uv {:?}, on edge {:?}, {} sub-facets)",
                    self.parent,
                    q,
                    on_edge,
                    self.tris.len()
                ),
            ));
        }
        let mut boundary = BTreeMap::<[usize; 2], [usize; 2]>::new();
        for &i in &cavity {
            let t = self.tris[i];
            for k in 0..3 {
                let (x, y) = (t[k], t[(k + 1) % 3]);
                if boundary.remove(&[y, x]).is_none() {
                    boundary.insert([x, y], [x, y]);
                }
            }
        }
        let mut next: Vec<[usize; 3]> = (0..self.tris.len())
            .filter(|i| !cavity.contains(i))
            .map(|i| self.tris[i])
            .collect();
        for [x, y] in boundary.into_values() {
            // A split point is on its edge by construction; rounding must not
            // turn that edge's cone triangle into a sliver.
            if on_edge == Some(edge(x, y)) {
                continue;
            }
            match orientation_2d(uv(x), uv(y), q) {
                Ordering::Greater => next.push([x, y, id]),
                Ordering::Equal => {} // the point lies on this boundary edge
                Ordering::Less => {
                    return Err(fail(
                        ErrorKind::Geometry,
                        "facet insertion cavity is not star-shaped",
                    ))
                }
            }
        }
        self.uv.insert(id, q);
        self.tris = next;
        Ok(())
    }
    /// Flip interior edges so absent sub-facets are replaced by pairs that are
    /// faces of the tetrahedralization. This resolves cospherical ties (grids,
    /// right angles) that a refinement would otherwise chase to zero size.
    /// Returns the number of flips.
    fn flip_toward(&mut self, faces: &BTreeSet<[usize; 3]>) -> usize {
        let mut flips = 0;
        'again: loop {
            let mut across = BTreeMap::<[usize; 2], usize>::new();
            for (i, t) in self.tris.iter().enumerate() {
                for k in 0..3 {
                    across.insert([t[k], t[(k + 1) % 3]], i);
                }
            }
            for i in 0..self.tris.len() {
                if faces.contains(&face_key(self.tris[i])) {
                    continue;
                }
                let t = self.tris[i];
                for k in 0..3 {
                    let (x, y, p) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let Some(&j) = across.get(&[y, x]) else {
                        continue;
                    };
                    let q = *self.tris[j].iter().find(|v| **v != x && **v != y).unwrap();
                    let (a, b) = ([x, q, p], [q, y, p]);
                    let uv = |v: usize| self.uv[&v];
                    let convex = orientation_2d(uv(a[0]), uv(a[1]), uv(a[2])) == Ordering::Greater
                        && orientation_2d(uv(b[0]), uv(b[1]), uv(b[2])) == Ordering::Greater;
                    if convex && faces.contains(&face_key(a)) && faces.contains(&face_key(b)) {
                        let (lo, hi) = (i.min(j), i.max(j));
                        self.tris[hi] = b;
                        self.tris[lo] = a;
                        flips += 1;
                        continue 'again;
                    }
                }
            }
            return flips;
        }
    }
    /// The facet boundary sub-segment nearest to `p`.
    fn nearest_boundary_edge(&self, p: V3, points: &[V3]) -> Option<[usize; 2]> {
        let mut count = BTreeMap::<[usize; 2], usize>::new();
        for t in &self.tris {
            for k in 0..3 {
                *count.entry(edge(t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        count
            .into_iter()
            .filter(|(_, n)| *n == 1)
            .map(|(e, _)| {
                let (a, b) = (points[e[0]], points[e[1]]);
                let ab = sub(b, a);
                let t = (dot(sub(p, a), ab) / norm2(ab)).clamp(0., 1.);
                let q: V3 = std::array::from_fn(|i| a[i] + ab[i] * t);
                (norm2(sub(p, q)), e)
            })
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|x| x.1)
    }
    fn has_boundary_edge(&self, s: [usize; 2]) -> bool {
        let mut seen = 0;
        for t in &self.tris {
            for k in 0..3 {
                if edge(t[k], t[(k + 1) % 3]) == s {
                    seen += 1;
                }
            }
        }
        seen == 1
    }
    /// Is `q` inside the original facet triangle (closed)?
    fn contains(&self, p: V3, points: &[V3]) -> bool {
        let q = self.project(p);
        (0..3).all(|k| {
            let (a, b) = (self.corners[k], self.corners[(k + 1) % 3]);
            let ua = self.uv_of(a, points);
            let ub = self.uv_of(b, points);
            orientation_2d(ua, ub, q) != Ordering::Less
        })
    }
    fn uv_of(&self, v: usize, points: &[V3]) -> [f64; 2] {
        self.uv
            .get(&v)
            .copied()
            .unwrap_or_else(|| self.project(points[v]))
    }
}
struct State {
    delaunay: Delaunay3,
    segments: BTreeSet<[usize; 2]>,
    meshes: Vec<FacetMesh>,
    /// Sub-segments / sub-facets known to be encroached (or forced) and not yet split.
    enc_segments: BTreeSet<[usize; 2]>,
    enc_tris: BTreeSet<(usize, [usize; 3])>,
    steiner: usize,
    segment_steiner: usize,
    facet_steiner: usize,
    quality_steiner: usize,
    unresolved_bad: usize,
    max_offset: f64,
    options: Options,
    /// Vertices below this index are input vertices (segment apices).
    input_vertices: usize,
    /// Shortest sub-segment that may still be split (millimetres).
    min_segment: f64,
    /// Segments of frozen facets: never split, never counted as encroached.
    frozen_segments: BTreeSet<[usize; 2]>,
    /// Frozen facets (indices into `meshes`): their triangle is never subdivided.
    frozen_facets: BTreeSet<usize>,
    /// Sub-facets whose circumcentre could only be placed by splitting a frozen segment: left as they are.
    unrefinable: BTreeSet<(usize, [usize; 3])>,
}
impl State {
    fn points(&self) -> &[V3] {
        self.delaunay.points()
    }
    fn facets(&self) -> Vec<([usize; 3], usize)> {
        self.meshes
            .iter()
            .flat_map(|m| m.tris.iter().map(|t| (*t, m.parent)))
            .collect()
    }
    fn tet_edges_and_faces(&self) -> (BTreeSet<[usize; 2]>, BTreeSet<[usize; 3]>) {
        let (mut edges, mut faces) = (BTreeSet::new(), BTreeSet::new());
        for [a, b, c, d] in self.delaunay.cells() {
            for (x, y) in [(a, b), (a, c), (a, d), (b, c), (b, d), (c, d)] {
                edges.insert(edge(x, y));
            }
            for f in [[a, b, c], [a, b, d], [a, c, d], [b, c, d]] {
                faces.insert(face_key(f));
            }
        }
        (edges, faces)
    }
    fn segment_encroached(&self, s: [usize; 2]) -> bool {
        let pts = self.points();
        (0..pts.len()).any(|p| p != s[0] && p != s[1] && encroaches(pts[s[0]], pts[s[1]], pts[p]))
    }
    fn tri_circumsphere(&self, t: [usize; 3]) -> Result<(V3, f64), Error> {
        let [a, b, c] = t.map(|v| self.points()[v]);
        let cc = circumcentre(a, b, c)
            .ok_or_else(|| fail(ErrorKind::Geometry, "degenerate PLC sub-facet"))?;
        Ok((cc, norm2(sub(a, cc))))
    }
    fn tri_encroached(&self, t: [usize; 3]) -> Result<bool, Error> {
        let (cc, r2) = self.tri_circumsphere(t)?;
        let pts = self.points();
        Ok((0..pts.len()).any(|p| !t.contains(&p) && norm2(sub(pts[p], cc)) < r2 * (1. - 1e-9)))
    }
    /// Re-derive the encroachment bookkeeping of one facet mesh against all vertices.
    fn refresh_mesh(&mut self, k: usize) -> Result<(), Error> {
        if !self.options.protect_encroached {
            return Ok(());
        }
        self.enc_tris.retain(|(m, _)| *m != k);
        if self.frozen_facets.contains(&k) {
            return Ok(());
        }
        for i in 0..self.meshes[k].tris.len() {
            let t = self.meshes[k].tris[i];
            if self.tri_encroached(t)? {
                self.enc_tris.insert((k, t));
            }
        }
        Ok(())
    }
    /// A vertex was added: it may encroach any existing sub-segment or sub-facet.
    fn note_vertex(&mut self, v: usize) -> Result<(), Error> {
        if !self.options.protect_encroached {
            return Ok(());
        }
        let p = self.points()[v];
        let pts = self.delaunay.points();
        let hit: Vec<[usize; 2]> = self
            .segments
            .iter()
            .copied()
            .filter(|s| s[0] != v && s[1] != v && !self.frozen_segments.contains(s) && encroaches(pts[s[0]], pts[s[1]], p))
            .collect();
        self.enc_segments.extend(hit);
        let mut hit_tris = Vec::new();
        for (k, m) in self.meshes.iter().enumerate() {
            if self.frozen_facets.contains(&k) {
                continue;
            }
            for &t in &m.tris {
                if t.contains(&v) {
                    continue;
                }
                let [a, b, c] = t.map(|x| pts[x]);
                if let Some(cc) = circumcentre(a, b, c) {
                    if norm2(sub(p, cc)) < norm2(sub(a, cc)) * (1. - 1e-9) {
                        hit_tris.push((k, t));
                    }
                }
            }
        }
        self.enc_tris.extend(hit_tris);
        Ok(())
    }
    /// One batch of quality refinement. Returns whether anything was queued or
    /// inserted (false: nothing left to do or nothing more can be done).
    fn refine_quality(&mut self, bound: f64) -> Result<bool, Error> {
        let (cells, neighbors) = self.delaunay.snapshot();
        let pts = self.points().to_vec();
        let facet_list: Vec<[usize; 3]> = self
            .meshes
            .iter()
            .flat_map(|m| m.tris.iter().copied())
            .collect();
        let outside = outside_cells(&cells, &neighbors, &facet_list, &pts);
        let mut bad: Vec<(f64, [usize; 4])> = Vec::new();
        for (t, out) in cells.iter().zip(&outside) {
            if *out {
                continue;
            }
            match tet_circumsphere(t.map(|v| pts[v])) {
                Some((_, ratio)) if ratio <= bound => {}
                _ => bad.push((
                    tet_circumsphere(t.map(|v| pts[v])).map_or(f64::MAX, |x| x.1),
                    *t,
                )),
            }
        }
        bad.sort_by(|a, b| b.0.total_cmp(&a.0));
        let batch = (bad.len() / 8).clamp(1, 256);
        let mut killed = BTreeSet::new();
        let mut progressed = false;
        let mut unresolved = 0;
        for (_, t) in bad.into_iter().take(batch) {
            let mut key = t;
            key.sort_unstable();
            if killed.contains(&key) {
                continue;
            }
            let Some((cc, _)) = tet_circumsphere(t.map(|v| pts[v])) else {
                unresolved += 1;
                continue;
            };
            // The circumcentre of a near-flat cell is ill-conditioned: rounded, it can lie outside the exact circumsphere, so
            // inserting it destroys nothing and the same cell (and the same centre, now a duplicate vertex) comes back every
            // round. Only a centre strictly inside the cell's own circumsphere can remove it.
            if sphere_side(t.map(|v| pts[v]), cc) != Ok(SphereSide::Inside) {
                unresolved += 1;
                continue;
            }
            let current = self.points().to_vec();
            let hit: Vec<[usize; 2]> = self
                .segments
                .iter()
                .copied()
                .filter(|&[x, y]| encroaches(current[x], current[y], cc))
                .collect();
            // A centre that encroaches a frozen segment or facet cannot be placed (the item may not be split): the cell stays.
            if hit.iter().any(|s| self.frozen_segments.contains(s)) {
                unresolved += 1;
                continue;
            }
            let mut hit_tris = Vec::new();
            let mut hits_frozen = false;
            for (k, m) in self.meshes.iter().enumerate() {
                for &f in &m.tris {
                    let [a, b, c] = f.map(|v| current[v]);
                    if let Some(fc) = circumcentre(a, b, c) {
                        if norm2(sub(cc, fc)) < norm2(sub(a, fc)) * (1. - 1e-9) {
                            if self.frozen_facets.contains(&k) {
                                hits_frozen = true;
                            }
                            hit_tris.push((k, f));
                        }
                    }
                }
            }
            if hits_frozen {
                unresolved += 1;
                continue;
            }
            if !hit.is_empty() || !hit_tris.is_empty() {
                self.enc_segments.extend(hit);
                self.enc_tris.extend(hit_tris);
                progressed = true;
                continue;
            }
            match self.insert(cc) {
                Ok(_) => {
                    self.quality_steiner += 1;
                    progressed = true;
                    for r in self.delaunay.last_removed() {
                        let mut k = *r;
                        k.sort_unstable();
                        killed.insert(k);
                    }
                }
                // A centre outside the hull or a degenerate cavity: leave this cell.
                Err(e) if e.kind == ErrorKind::Geometry => unresolved += 1,
                Err(e) => return Err(e),
            }
        }
        self.unresolved_bad = unresolved;
        Ok(progressed)
    }
    fn insert(&mut self, p: V3) -> Result<usize, Error> {
        if self.steiner >= self.options.max_steiner {
            return Err(fail(
                ErrorKind::Limit,
                "Steiner point budget exceeded; the input may have acute segment or facet angles",
            ));
        }
        let i = self.delaunay.insert(p).map_err(mesh_error)?;
        self.steiner += 1;
        Ok(i)
    }
    fn split_segment(&mut self, s: [usize; 2]) -> Result<(), Error> {
        let [a, b] = s;
        let (pa, pb) = (self.points()[a], self.points()[b]);
        let len2 = norm2(sub(pa, pb));
        if self.frozen_segments.contains(&s) {
            return Err(fail(
                ErrorKind::Geometry,
                format!(
                    "a frozen segment {pa:?} - {pb:?} is missing from the Delaunay triangulation and may not be split"
                ),
            ));
        }
        if len2.sqrt() < self.min_segment {
            return Err(fail(
                ErrorKind::Geometry,
                "refinement descends below the feature-size floor around a vertex (input angles too small for the requested quality)",
            ));
        }
        // Concentric shells (Ruppert/Shewchuk): when exactly one end is an input
        // vertex its incident segments all split at powers-of-two distances from
        // it, so segments meeting at a small angle stop encroaching on each other.
        let (apex_a, apex_b) = (a < self.input_vertices, b < self.input_vertices);
        let fraction = if apex_a != apex_b {
            let l = len2.sqrt();
            let d = 2f64.powf((l * 0.5).log2().round());
            let f = (d / l).clamp(0.25, 0.75);
            if apex_a {
                f
            } else {
                1. - f
            }
        } else {
            0.5
        };
        let m: V3 = std::array::from_fn(|i| pa[i] + (pb[i] - pa[i]) * fraction);
        // Offset of the rounded split point from the exact segment line.
        let off = norm2(cross(sub(m, pa), sub(pb, pa))) / len2;
        let mi = self.insert(m)?;
        self.segment_steiner += 1;
        self.max_offset = self.max_offset.max(off.sqrt() / len2.sqrt());
        self.segments.remove(&s);
        self.enc_segments.remove(&s);
        let halves = [edge(a, mi), edge(mi, b)];
        self.segments.extend(halves);
        // Both facets bounded by the segment gain the new point on their edge.
        let mut touched = Vec::new();
        for k in 0..self.meshes.len() {
            if self.meshes[k].has_boundary_edge(s) {
                self.meshes[k].insert(mi, m, Some(s))?;
                touched.push(k);
            }
        }
        self.note_vertex(mi)?;
        for h in halves {
            if self.options.protect_encroached && self.segment_encroached(h) {
                self.enc_segments.insert(h);
            }
        }
        for k in touched {
            self.refresh_mesh(k)?;
        }
        Ok(())
    }
}
/// Recover `facets` (outward-oriented triangles over `vertices`, a closed
/// manifold shell) and all their edges in a Delaunay tetrahedralization, then
/// extract the enclosed cells.
pub fn conform(
    vertices: &[V3],
    facets: &[[usize; 3]],
    options: Options,
    progress: impl FnMut(Progress) -> bool,
) -> Result<Conforming, Error> {
    conform_impl(vertices, facets, None, options, progress)
}
/// [`conform`] with a set of **frozen** facets: `frozen[i]` freezes `facets[i]`. A frozen facet and every segment of it are
/// never split, so the result contains each frozen facet exactly as given (a triangle of the output with that parent and the
/// same three vertices); Steiner points go to the unfrozen facets, their segments and the interior. When a frozen facet or
/// segment is not in the Delaunay triangulation of the vertices and the points added elsewhere, the recovery fails with an
/// error naming it instead of splitting it. This is the tetrahedral mesher for the region behind a layer of pyramids (their
/// triangular sides are the frozen facets): `docs/issue-46-phase-102.md`.
pub fn conform_preserving(
    vertices: &[V3],
    facets: &[[usize; 3]],
    frozen: &[bool],
    options: Options,
    progress: impl FnMut(Progress) -> bool,
) -> Result<Conforming, Error> {
    if frozen.len() != facets.len() {
        return Err(fail(
            ErrorKind::Topology,
            "one frozen flag per facet is required",
        ));
    }
    conform_impl(vertices, facets, Some(frozen), options, progress)
}
fn conform_impl(
    vertices: &[V3],
    facets: &[[usize; 3]],
    frozen: Option<&[bool]>,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Conforming, Error> {
    let publish = |state: &State,
                   phase,
                   missing: (usize, usize),
                   progress: &mut dyn FnMut(Progress) -> bool| {
        if progress(Progress {
            phase,
            steiner: state.steiner,
            missing_segments: missing.0,
            missing_facets: missing.1,
        }) {
            Ok(())
        } else {
            Err(fail(ErrorKind::Cancelled, "PLC recovery cancelled"))
        }
    };
    if facets.is_empty() || facets.iter().flatten().any(|&v| v >= vertices.len()) {
        return Err(fail(ErrorKind::Topology, "invalid PLC facet list"));
    }
    let mut segments = BTreeSet::new();
    let mut incidence = BTreeMap::<[usize; 2], usize>::new();
    for f in facets {
        for k in 0..3 {
            let e = edge(f[k], f[(k + 1) % 3]);
            segments.insert(e);
            *incidence.entry(e).or_default() += 1;
        }
    }
    if incidence.values().any(|&n| n != 2) {
        return Err(fail(
            ErrorKind::Topology,
            "PLC must be a closed manifold shell (every edge in exactly two facets)",
        ));
    }
    let limits = DelaunayLimits {
        max_points: vertices.len() + options.max_steiner,
        max_cells: options.max_cells,
    };
    let meshes = facets
        .iter()
        .enumerate()
        .map(|(i, f)| FacetMesh::new(i, *f, vertices))
        .collect::<Result<Vec<_>, _>>()?;
    // Eight far points make every shell point (and every Steiner point on the shell) an interior point of the triangulation. The
    // incremental engine does not extend a hull exactly, and a convex shell (a cylinder, a block) has all its points on the hull
    // of its vertices: a Steiner point rounded a few ulps outside it used to dent the hull until a later point fell far outside.
    let enclosing = enclosing_points(vertices);
    let with_box: Vec<V3> = vertices.iter().copied().chain(enclosing).collect();
    let limits = DelaunayLimits {
        max_points: limits.max_points + ENCLOSING,
        ..limits
    };
    let seed = tetrahedralize(&with_box, limits).map_err(mesh_error)?;
    let mut state = State {
        delaunay: Delaunay3::new(&seed, options.max_cells),
        segments,
        meshes,
        enc_segments: BTreeSet::new(),
        enc_tris: BTreeSet::new(),
        steiner: 0,
        segment_steiner: 0,
        facet_steiner: 0,
        quality_steiner: 0,
        unresolved_bad: 0,
        max_offset: 0.,
        options,
        input_vertices: vertices.len(),
        frozen_segments: frozen
            .into_iter()
            .flat_map(|fr| facets.iter().zip(fr))
            .filter(|(_, f)| **f)
            .flat_map(|(t, _)| (0..3).map(move |k| edge(t[k], t[(k + 1) % 3])))
            .collect(),
        frozen_facets: frozen
            .into_iter()
            .flat_map(|fr| fr.iter().enumerate())
            .filter(|(_, f)| **f)
            .map(|(i, _)| i)
            .collect(),
        unrefinable: BTreeSet::new(),
        min_segment: {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for v in vertices {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            options.min_segment_relative * norm2(sub(hi, lo)).sqrt()
        },
    };
    publish(&state, Phase::Delaunay, (0, 0), &mut progress)?;
    let all: Vec<[usize; 2]> = state.segments.iter().copied().collect();
    for s in all {
        if options.protect_encroached && !state.frozen_segments.contains(&s) && state.segment_encroached(s) {
            state.enc_segments.insert(s);
        }
    }
    for k in 0..state.meshes.len() {
        state.refresh_mesh(k)?;
    }
    let mut rounds = 0;
    loop {
        // Encroached segments before facets; each insertion updates both lists.
        while let Some(s) = state.enc_segments.pop_first() {
            if !state.segments.contains(&s) {
                continue;
            }
            publish(
                &state,
                Phase::Segments,
                (state.enc_segments.len() + 1, state.enc_tris.len()),
                &mut progress,
            )?;
            state.split_segment(s)?;
        }
        if let Some((k, t)) = state.enc_tris.pop_first() {
            if !state.meshes[k].tris.contains(&t) {
                continue;
            }
            publish(
                &state,
                Phase::Facets,
                (0, state.enc_tris.len() + 1),
                &mut progress,
            )?;
            if state.frozen_facets.contains(&k) {
                let [a, b, c] = t.map(|v| state.points()[v]);
                return Err(fail(
                    ErrorKind::Geometry,
                    format!(
                        "a frozen facet {a:?} {b:?} {c:?} is missing from the Delaunay triangulation and may not be split"
                    ),
                ));
            }
            if state.unrefinable.contains(&(k, t)) {
                continue;
            }
            let (cc, _) = state.tri_circumsphere(t)?;
            let pts = state.points().to_vec();
            let hit: Vec<[usize; 2]> = state
                .segments
                .iter()
                .copied()
                .filter(|&[x, y]| encroaches(pts[x], pts[y], cc))
                .collect();
            // The centre of a sub-facet that encroaches a frozen segment cannot be placed; a present sub-facet is left as it is.
            if hit.iter().any(|s| state.frozen_segments.contains(s)) {
                state.unrefinable.insert((k, t));
                continue;
            }
            if !hit.is_empty() {
                // The centre is rejected; its encroached segments are split first.
                state.enc_segments.extend(hit);
                state.enc_tris.insert((k, t));
                continue;
            }
            if !state.meshes[k].contains(cc, &pts) {
                return Err(fail(
                    ErrorKind::Geometry,
                    "sub-facet circumcentre leaves its facet without encroaching a segment",
                ));
            }
            let (off, longest) = {
                let m = &state.meshes[k];
                let n = cross(m.e1, m.e2);
                let longest = m
                    .tris
                    .iter()
                    .flat_map(|t| (0..3).map(|i| norm2(sub(pts[t[i]], pts[t[(i + 1) % 3]]))))
                    .fold(0., f64::max);
                (dot(sub(cc, m.origin), n).abs(), longest)
            };
            // The facet mesh is updated first: it changes nothing when it refuses.
            let id = pts.len();
            if let Err(refusal) = state.meshes[k].insert(id, cc, None) {
                // The centre sits on the facet boundary within rounding: split the
                // closest boundary sub-segment instead and revisit the sub-facet.
                let nearest = state.meshes[k].nearest_boundary_edge(cc, &pts);
                if let Some(s) = nearest.filter(|s| state.segments.contains(s)) {
                    state.enc_segments.insert(s);
                    state.enc_tris.insert((k, t));
                    continue;
                }
                return Err(fail(
                    ErrorKind::Geometry,
                    format!(
                        "sub-facet circumcentre could not be inserted into its facet: {}",
                        refusal.detail
                    ),
                ));
            }
            let inserted = state.insert(cc)?;
            debug_assert_eq!(inserted, id);
            state.facet_steiner += 1;
            state.max_offset = state.max_offset.max(off / longest.sqrt());
            state.note_vertex(id)?;
            state.refresh_mesh(k)?;
            continue;
        }
        // Encroachment is only a sufficient condition; verify presence exactly and
        // force whatever a cospherical tie left out.
        rounds += 1;
        let (edges, faces) = state.tet_edges_and_faces();
        let missing_segments: Vec<_> = state
            .segments
            .iter()
            .copied()
            .filter(|s| !edges.contains(s))
            .collect();
        let missing_tris: Vec<(usize, [usize; 3])> = state
            .meshes
            .iter()
            .enumerate()
            .flat_map(|(k, m)| m.tris.iter().map(move |t| (k, *t)))
            .filter(|(_, t)| !faces.contains(&face_key(*t)))
            .collect();
        // A cospherical tie can make the facet's planar triangulation pick the other
        // diagonal than the tetrahedralization: align it before refining anything.
        let mut flipped = 0;
        if missing_segments.is_empty() && !missing_tris.is_empty() {
            for mesh in &mut state.meshes {
                flipped += mesh.flip_toward(&faces);
            }
        }
        if flipped > 0 {
            for k in 0..state.meshes.len() {
                state.refresh_mesh(k)?;
            }
            continue;
        }
        if let Some((k, t)) = missing_tris.iter().find(|(k, t)| state.unrefinable.contains(&(*k, *t))) {
            let [a, b, c] = t.map(|v| state.points()[v]);
            return Err(fail(
                ErrorKind::Geometry,
                format!(
                    "facet {k} ({a:?} {b:?} {c:?}) is missing and refining it would split a frozen segment"
                ),
            ));
        }
        if missing_segments.is_empty() && missing_tris.is_empty() {
            let bound = state.options.max_radius_edge;
            match bound {
                Some(bound) if state.refine_quality(bound)? => continue,
                _ => break,
            }
        }
        publish(
            &state,
            Phase::Facets,
            (missing_segments.len(), missing_tris.len()),
            &mut progress,
        )?;
        if !missing_segments.is_empty() {
            state.enc_segments.extend(missing_segments);
        } else {
            state.enc_tris.extend(missing_tris);
        }
    }
    publish(&state, Phase::Extraction, (0, 0), &mut progress)?;
    extract(state, vertices.len(), rounds)
}
/// Number of far points added around the input (a box).
const ENCLOSING: usize = 8;
fn enclosing_points(vertices: &[V3]) -> [V3; ENCLOSING] {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for v in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let centre: V3 = std::array::from_fn(|k| 0.5 * (lo[k] + hi[k]));
    let half = (0..3)
        .map(|k| 0.5 * (hi[k] - lo[k]))
        .fold(0., f64::max)
        .max(f64::MIN_POSITIVE);
    let reach = 64. * half;
    std::array::from_fn(|i| {
        std::array::from_fn(|k| centre[k] + if (i >> k) & 1 == 1 { reach } else { -reach })
    })
}
fn extract(state: State, input_vertices: usize, rounds: usize) -> Result<Conforming, Error> {
    let reconnect = state.options.reconnect;
    let max_cells = state.options.max_cells;
    let facets = state.facets();
    let stats = (
        state.segment_steiner,
        state.facet_steiner,
        state.max_offset,
        state.segments.clone(),
        state.quality_steiner,
        state.unresolved_bad,
    );
    let topology = state.delaunay.into_topology().map_err(mesh_error)?;
    let points = topology.points().to_vec();
    let cells = topology.cells();
    let facet_keys: BTreeSet<[usize; 3]> = facets.iter().map(|(f, _)| face_key(*f)).collect();
    // Cells reachable from the hull boundary without crossing a PLC facet lie outside.
    let neighbors = topology.neighbors();
    let faces_of = |t: [usize; 4]| {
        let [a, b, c, d] = t;
        [[b, c, d], [a, d, c], [a, b, d], [a, c, b]]
    };
    let facet_list: Vec<[usize; 3]> = facets.iter().map(|(f, _)| *f).collect();
    let outside = outside_cells(cells, neighbors, &facet_list, &points);
    let interior: Vec<[usize; 4]> = cells
        .iter()
        .zip(&outside)
        .filter(|(_, o)| !**o)
        .map(|(t, _)| *t)
        .collect();
    let (interior, reconnection) = match reconnect {
        None => (interior, None),
        Some(flip) => {
            let seed = TetTopology::new(points.clone(), interior, max_cells).map_err(mesh_error)?;
            let (better, report) = improve(&seed, &BTreeSet::new(), flip).map_err(mesh_error)?;
            (better.cells().to_vec(), Some(report))
        }
    };
    // Each sub-facet must separate one inside cell from one outside cell (or the void).
    let mut uses = BTreeMap::<[usize; 3], usize>::new();
    for t in &interior {
        for f in faces_of(*t) {
            let k = face_key(f);
            if facet_keys.contains(&k) {
                *uses.entry(k).or_default() += 1;
            }
        }
    }
    if facet_keys.iter().any(|k| uses.get(k) != Some(&1)) {
        let bad = facet_keys
            .iter()
            .filter(|k| uses.get(*k) != Some(&1))
            .count();
        return Err(fail(
            ErrorKind::Topology,
            format!(
                "{bad} of {} recovered sub-facets do not bound the extracted cells exactly once",
                facet_keys.len()
            ),
        ));
    }
    let tet_volume = |t: &[usize; 4]| {
        let [a, b, c, d] = t.map(|v| points[v]);
        dot(sub(a, d), cross(sub(b, d), sub(c, d))).abs() / 6.
    };
    let cell_volume: f64 = interior.iter().map(tet_volume).sum();
    // Divergence theorem: V = 1/6 sum over outward facets of p0 . (p1 x p2).
    let enclosed: f64 = facets
        .iter()
        .map(|(f, _)| {
            let [a, b, c] = f.map(|v| points[v]);
            dot(a, cross(b, c)) / 6.
        })
        .sum();
    if !(enclosed.is_finite() && enclosed > 0.) {
        return Err(fail(
            ErrorKind::Topology,
            "PLC shell is not outward oriented with positive enclosed volume",
        ));
    }
    let max_radius_edge_ratio = interior
        .iter()
        .map(|t| tet_circumsphere(t.map(|v| points[v])).map_or(f64::INFINITY, |x| x.1))
        .fold(0., f64::max);
    // The far points are in no inside cell, no facet and no segment; they are dropped below with any unused Steiner point.
    debug_assert!(interior
        .iter()
        .flatten()
        .all(|&v| !(input_vertices..input_vertices + ENCLOSING).contains(&v)));
    let volume_error_relative = (cell_volume - enclosed).abs() / enclosed;
    if volume_error_relative > 1e-9 {
        return Err(fail(
            ErrorKind::Geometry,
            format!("extracted cells do not fill the PLC volume (relative error {volume_error_relative:e})"),
        ));
    }
    // Steiner points that no cell, facet or segment uses (a quality point whose cell was then dropped, or one that fell outside
    // the body) are not part of the result: they would be nodes of no element. The input vertices keep their numbers.
    let kept = |i: usize| i < input_vertices + ENCLOSING;
    let mut used = vec![false; points.len()];
    for t in &interior {
        t.iter().for_each(|&v| used[v] = true);
    }
    for (f, _) in &facets {
        f.iter().for_each(|&v| used[v] = true);
    }
    for e in &stats.3 {
        e.iter().for_each(|&v| used[v] = true);
    }
    let mut renumber = vec![usize::MAX; points.len()];
    let mut next = 0;
    for (i, slot) in renumber.iter_mut().enumerate() {
        if i < input_vertices {
            *slot = next;
            next += 1;
        } else if !kept(i) && used[i] {
            *slot = next;
            next += 1;
        }
    }
    let points: Vec<V3> = points
        .iter()
        .enumerate()
        .filter(|(i, _)| renumber[*i] != usize::MAX)
        .map(|(_, p)| *p)
        .collect();
    Ok(Conforming {
        report: Report {
            input_vertices,
            segment_steiner: stats.0,
            facet_steiner: stats.1,
            rounds,
            max_steiner_offset_relative: stats.2,
            quality_steiner: stats.4,
            max_radius_edge_ratio,
            unresolved_bad_cells: stats.5,
            reconnection,
            hull_cells: cells.len(),
            interior_cells: interior.len(),
            volume_error_relative,
        },
        points,
        cells: interior.into_iter().map(|t| t.map(|v| renumber[v])).collect(),
        facets: facets.into_iter().map(|(f, k)| (f.map(|v| renumber[v]), k)).collect(),
        segments: stats.3.into_iter().map(|e| e.map(|v| renumber[v])).collect(),
    })
}
