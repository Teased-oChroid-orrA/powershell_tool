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
    quality::tet_scaled_jacobian,
    tet_flip::{improve, improve_by, FlipOptions, FlipReport},
    tet_smooth::{optimize, optimize_by, SmoothOptions},
    tet_topology::{
        in_circle_2d, orientation, orientation_2d, sphere_side, SphereSide, TetTopology,
    },
    MeshError,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
/// Multiplicative hash for the small integer keys of the per-facet tables (vertex numbers, directed edges): the
/// default SipHash dominated the insertion into large facet meshes. Iteration order is never observed.
#[derive(Default)]
struct FastHasher(u64);
impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn write_u8(&mut self, n: u8) {
        self.write_u64(n as u64);
    }
    fn write_i64(&mut self, n: i64) {
        self.write_u64(n as u64);
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(26) ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}
type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;
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
    /// extraction (boundary, sub-facets and vertices are preserved exactly). With
    /// `min_scaled_jacobian` set, cells are judged by the scaled Jacobian, so
    /// `FlipOptions::target` is a scaled-Jacobian threshold.
    pub reconnect: Option<FlipOptions>,
    /// With `reconnect`, alternate that pass with smoothing of the interior vertices (`fea_core::meshing::tet_smooth`) for up
    /// to this many rounds (`0`: reconnection only). Boundary vertices and the boundary never move.
    pub smooth_rounds: usize,
    /// Also insert the circumcentre of every interior tetrahedron whose scaled Jacobian (the solver gate's measure,
    /// `fea_core::meshing::quality`) is below this bound. A radius-edge bound does not see slivers, and a Delaunay
    /// recovery of a sampled CAD shell is full of them: tetrahedra whose four vertices all lie on the boundary, flat to the
    /// rounding of the coordinates where a planar or cylindrical face carries four co-circular samples. Their circumcentre
    /// is an interior point (or, on the face, splits the sub-facets it encroaches), which removes the sliver; the budget
    /// `max_steiner` bounds the effort and cells that stay below the bound are counted in `unresolved_bad_cells`.
    pub min_scaled_jacobian: Option<f64>,
    /// Most points the sliver refinement (`min_scaled_jacobian`) may add; it also stops at fifteen times the vertices it starts
    /// with (at least 1500). It stops there, quietly: the cells still below the bound are counted in `unresolved_bad_cells`.
    pub max_quality_steiner: usize,
    /// Wall-clock seconds the sliver refinement may run (per recovery); it then returns to the last conforming state it kept,
    /// exactly as when the point budget is spent. A recovery that cannot finish its cascades in time keeps the mesh it had.
    pub max_quality_seconds: f64,
    /// A sub-segment shorter than this fraction of the input's bounding-box diagonal is never split: refinement that has
    /// descended that far (a cascade of concentric shells around a vertex) produces elements no solver can use, and the
    /// recovery stops with an error instead.
    pub min_segment_relative: f64,
    /// Merge adjacent unfrozen input facets that are coplanar (every vertex within 1e-9 of the other triangle's plane,
    /// relative to its edge length, and the patch flat to 1e-8 of its size) into one polygonal facet. The edges between
    /// merged facets stop being segments, so a diagonal of a planar face (a CAD face sampled into triangles) no longer
    /// has to be recovered and cannot start a cascade of ever smaller splits. The output sub-facets then carry as parent
    /// the input facet that contains their centroid (they may straddle a removed diagonal); a merged patch is not
    /// reproduced triangle by triangle. Off by default: the result is then triangle for triangle the input.
    pub merge_coplanar: bool,
    /// When the recovery with `protect_encroached` fails because refinement does not terminate (the Steiner budget is
    /// exhausted or a sub-segment reaches `min_segment_relative`), run it again once with `protect_encroached: false`.
    /// The input angles of a sampled CAD surface are far below what Shewchuk's protect-everything rule terminates on
    /// (needles of a curved face, dihedrals of a few degrees): there every encroached item is split and the splits
    /// cascade. Splitting only what the exact presence check finds missing needs a fraction of the points and ends on
    /// these shells; its result is verified exactly like the first. A recovery that succeeds at the first attempt is
    /// unchanged. Has no effect when `protect_encroached` is already false.
    pub unprotected_fallback: bool,
    /// With the fallback enabled, the recovery that protects every encroached item gives up after this many Steiner
    /// points (counted like `max_steiner`) and the unprotected recovery takes over with its own full budget. The protected
    /// recovery either ends within a few thousand points or cascades without a scale to stop it (the ABC corpus, 377
    /// shells that finished: at most 25 895 points, 3 above 20 000; 18 that did not were still adding points after 8 000
    /// to 62 000 when they ran out of time), so waiting for it to reach `max_steiner` only spends the time the fallback
    /// needs. Ignored when the fallback is off (then `max_steiner` alone limits the run).
    pub protected_max_steiner: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_steiner: 4000,
            max_cells: 400_000,
            protect_encroached: true,
            max_radius_edge: None,
            reconnect: None,
            smooth_rounds: 0,
            min_scaled_jacobian: None,
            max_quality_steiner: 50_000,
            max_quality_seconds: 30.,
            min_segment_relative: 1e-6,
            merge_coplanar: false,
            unprotected_fallback: true,
            protected_max_steiner: 30_000,
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
    /// Result of the optional reconnection pass over the interior cells. With
    /// `Options::min_scaled_jacobian` set, its `min_radius_ratio_*` and
    /// `cells_below_target_*` fields are in scaled-Jacobian units.
    pub reconnection: Option<FlipReport>,
    pub hull_cells: usize,
    pub interior_cells: usize,
    /// |sum of cell volumes - PLC enclosed volume| / PLC enclosed volume.
    pub volume_error_relative: f64,
    /// The recovery that protects every encroached item failed to terminate and this result is the unprotected fallback's.
    pub unprotected_fallback_used: bool,
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
    let lookup = FacetLookup::new(facets, points.len());
    // `blocked[i][k]`: face `k` of cell `i` is a sub-facet
    let mut blocked = vec![[false; 4]; cells.len()];
    let mut inside = vec![false; cells.len()];
    let mut stack = Vec::new();
    for (i, t) in cells.iter().enumerate() {
        let (b, seeded) = lookup.faces(*t, points);
        blocked[i] = b;
        if seeded {
            inside[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        for k in 0..4 {
            if let Some(j) = neighbors[i][k] {
                if !inside[j] && !blocked[i][k] {
                    inside[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    inside.into_iter().map(|x| !x).collect()
}
/// The faces of a tetrahedron, `k` opposite vertex `k`, each oriented outward of a positive cell.
fn cell_faces(t: [usize; 4]) -> [[usize; 3]; 4] {
    let [a, b, c, d] = t;
    [[b, c, d], [a, d, c], [a, b, d], [a, c, b]]
}
/// The sub-facets by their keys, for telling which faces of a cell are sub-facets and whether one has the cell on its
/// inner side.
struct FacetLookup {
    /// each sub-facet by its key, with its outward orientation
    keys: FastMap<[usize; 3], [usize; 3]>,
    /// the other orientations of a key that two meshes hold differently
    other_sides: FastMap<[usize; 3], Vec<[usize; 3]>>,
    /// Only a face with all three vertices on sub-facets can be one: the others are not looked up.
    on_facet: Vec<bool>,
}
impl FacetLookup {
    fn new(facets: &[[usize; 3]], points: usize) -> Self {
        let mut keys = FastMap::<[usize; 3], [usize; 3]>::default();
        let mut other_sides = FastMap::<[usize; 3], Vec<[usize; 3]>>::default();
        let mut on_facet = vec![false; points];
        for f in facets {
            let first = *keys.entry(face_key(*f)).or_insert(*f);
            if first != *f {
                other_sides.entry(face_key(*f)).or_default().push(*f);
            }
            for &v in f {
                on_facet[v] = true;
            }
        }
        Self { keys, other_sides, on_facet }
    }
    /// Which faces of the cell `t` are sub-facets, and whether one of them has the cell on its inner side.
    fn faces(&self, t: [usize; 4], points: &[V3]) -> ([bool; 4], bool) {
        let (mut blocked, mut seeded) = ([false; 4], false);
        for (k, f) in cell_faces(t).into_iter().enumerate() {
            if !f.iter().all(|&v| self.on_facet[v]) {
                continue;
            }
            let key = face_key(f);
            if let Some(first) = self.keys.get(&key) {
                blocked[k] = true;
                let apex = points[t[k]];
                for f in std::iter::once(first).chain(self.other_sides.get(&key).into_iter().flatten()) {
                    let [a, b, c] = f.map(|v| points[v]);
                    // The interior side of an outward face is the Less side.
                    if !seeded && orientation(a, b, c, apex) == Ok(Ordering::Less) {
                        seeded = true;
                    }
                }
            }
        }
        (blocked, seeded)
    }
}
/// Inside flags and sliver keys of the Delaunay cells by id, kept from one round of quality refinement to the next: only
/// the cells created since are classified again. A cell keeps its side of the shell while it lives (points never move,
/// and the sub-facets only ever subdivide or re-triangulate the same surface). A facet flip in `drive` drops the cache all
/// the same, since it changes which faces block the flood without creating cells.
#[derive(Clone)]
struct CellCache {
    bound: f64,
    min_sj: f64,
    inside: Vec<bool>,
    /// The badness key of an inside cell the refinement should remove ([`State::bad_cells`]), by id.
    key: Vec<Option<f64>>,
}
/// Planar Delaunay triangulation of one input facet and the Steiner points on it.
#[derive(Clone)]
struct FacetMesh {
    parent: usize,
    origin: V3,
    e1: V3,
    e2: V3,
    /// The input triangles of the facet (one, or a coplanar patch), counter-clockwise in the facet frame.
    region: Vec<[usize; 3]>,
    uv: FastMap<usize, [f64; 2]>,
    tris: Vec<[usize; 3]>,
    /// Circumcentre and squared circumradius of each sub-facet, parallel to `tris`; empty when stale (every change of
    /// `tris` clears it). Points are never moved, so a sub-facet's sphere never changes.
    spheres: Vec<Option<(V3, f64)>>,
    /// Box around all the balls of `spheres` (valid with `spheres`): a vertex outside it encroaches no sub-facet.
    ball_box: ([f64; 3], [f64; 3]),
    /// The balls of `spheres` by index, rebuilt with them, for a mesh of at least [`FacetMesh::GRID_MIN`] sub-facets: a
    /// scan of a large merged patch's balls took about a third of the sliver stage on ABC `00006600` (Phase 160).
    ball_grid: Option<BallGrid<usize>>,
}
impl FacetMesh {
    /// Fewest sub-facets for which [`FacetMesh::ball_grid`] is kept; smaller meshes are scanned.
    const GRID_MIN: usize = 32;
    fn new(parent: usize, f: [usize; 3], points: &[V3]) -> Result<Self, Error> {
        Self::patch(parent, &[f], points)
    }
    /// A facet made of coplanar triangles `members` (outward oriented, all counter-clockwise in the common frame). The
    /// frame is that of the largest member. Interior edges are free: the planar triangulation is made Delaunay by flips.
    fn patch(parent: usize, members: &[[usize; 3]], points: &[V3]) -> Result<Self, Error> {
        let normal = |f: &[usize; 3]| {
            let [a, b, c] = f.map(|v| points[v]);
            cross(sub(b, a), sub(c, a))
        };
        let f = *members
            .iter()
            .max_by(|x, y| norm2(normal(x)).total_cmp(&norm2(normal(y))))
            .ok_or_else(|| fail(ErrorKind::Geometry, "empty PLC facet"))?;
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
            region: members.to_vec(),
            uv: FastMap::default(),
            tris: members.to_vec(),
            spheres: Vec::new(),
            ball_box: ([0.; 3], [0.; 3]),
            ball_grid: None,
        };
        for v in members.iter().flatten() {
            let q = mesh.project(points[*v]);
            mesh.uv.insert(*v, q);
        }
        if members.len() > 1 {
            let uv = |v: usize| mesh.uv[&v];
            if members.iter().any(|t| orientation_2d(uv(t[0]), uv(t[1]), uv(t[2])) != Ordering::Greater) {
                return Err(fail(ErrorKind::Geometry, "coplanar PLC patch is not consistently oriented"));
            }
            mesh.make_delaunay();
        }
        Ok(mesh)
    }
    /// Lawson flips of interior edges until every one is locally Delaunay in the facet plane (boundary edges have one
    /// triangle and never flip), so that Bowyer–Watson cavities of later insertions are star-shaped.
    fn make_delaunay(&mut self) {
        let mut budget = self.tris.len() * self.tris.len() + 64;
        'again: while budget > 0 {
            budget -= 1;
            let mut across = BTreeMap::<[usize; 2], usize>::new();
            for (i, t) in self.tris.iter().enumerate() {
                for k in 0..3 {
                    across.insert([t[k], t[(k + 1) % 3]], i);
                }
            }
            for i in 0..self.tris.len() {
                let t = self.tris[i];
                for k in 0..3 {
                    let (x, y, p) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let Some(&j) = across.get(&[y, x]) else {
                        continue;
                    };
                    let q = *self.tris[j].iter().find(|v| **v != x && **v != y).unwrap();
                    let uv = |v: usize| self.uv[&v];
                    if in_circle_2d(uv(x), uv(y), uv(p), uv(q)) != Ordering::Greater {
                        continue;
                    }
                    let (na, nb) = ([x, q, p], [q, y, p]);
                    let convex = orientation_2d(uv(na[0]), uv(na[1]), uv(na[2])) == Ordering::Greater
                        && orientation_2d(uv(nb[0]), uv(nb[1]), uv(nb[2])) == Ordering::Greater;
                    if convex {
                        self.tris[i] = na;
                        self.tris[j] = nb;
                        self.spheres.clear();
                        continue 'again;
                    }
                }
            }
            return;
        }
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
        let mut across = FastMap::<[usize; 2], usize>::with_capacity_and_hasher(3 * self.tris.len(), Default::default());
        for (i, t) in self.tris.iter().enumerate() {
            for k in 0..3 {
                across.insert([t[k], t[(k + 1) % 3]], i);
            }
        }
        let mut in_cavity = vec![false; self.tris.len()];
        let mut cavity: Vec<usize> = seed.into_iter().collect();
        for &i in &cavity {
            in_cavity[i] = true;
        }
        let mut next_to_visit = 0;
        while next_to_visit < cavity.len() {
            let t = self.tris[cavity[next_to_visit]];
            next_to_visit += 1;
            for k in 0..3 {
                let Some(&j) = across.get(&[t[(k + 1) % 3], t[k]]) else {
                    continue;
                };
                if in_cavity[j] {
                    continue;
                }
                let [a, b, c] = self.tris[j].map(uv);
                if in_circle_2d(a, b, c, q) == Ordering::Greater {
                    in_cavity[j] = true;
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
            .filter(|&i| !in_cavity[i])
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
        // The survivors keep their circumspheres (points never move) and keep their order; the new cone triangles are
        // appended, so `spheres` stays a valid prefix that `ensure_spheres` completes.
        if self.spheres.len() == self.tris.len() {
            let mut kept = Vec::with_capacity(next.len());
            for (i, sphere) in self.spheres.iter().enumerate() {
                if !in_cavity[i] {
                    kept.push(*sphere);
                }
            }
            self.spheres = kept;
        } else {
            self.spheres.clear();
        }
        self.tris = next;
        Ok(())
    }
    /// Flip interior edges so absent sub-facets are replaced by pairs that are
    /// faces of the tetrahedralization. This resolves cospherical ties (grids,
    /// right angles) that a refinement would otherwise chase to zero size.
    /// Returns the number of flips.
    fn flip_toward(&mut self, faces: &FastMap<[usize; 3], u32>) -> usize {
        let mut flips = 0;
        'again: loop {
            let mut across = BTreeMap::<[usize; 2], usize>::new();
            for (i, t) in self.tris.iter().enumerate() {
                for k in 0..3 {
                    across.insert([t[k], t[(k + 1) % 3]], i);
                }
            }
            for i in 0..self.tris.len() {
                if faces.contains_key(&face_key(self.tris[i])) {
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
                    if convex && faces.contains_key(&face_key(a)) && faces.contains_key(&face_key(b)) {
                        let (lo, hi) = (i.min(j), i.max(j));
                        self.tris[hi] = b;
                        self.tris[lo] = a;
                        self.spheres.clear();
                        flips += 1;
                        continue 'again;
                    }
                }
            }
            return flips;
        }
    }
    /// Fill `spheres` (`None`: degenerate sub-facet) if stale.
    fn ensure_spheres(&mut self, points: &[V3]) {
        if self.spheres.len() != self.tris.len() {
            // A prefix of `spheres` is valid for the first triangles (an insertion keeps its survivors' spheres); a
            // shorter or empty list is completed. The box only grows: removed balls stay in it, which is merely coarser.
            let done = self.spheres.len().min(self.tris.len());
            self.spheres.truncate(done);
            let (mut lo, mut hi) = if done == 0 { ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]) } else { self.ball_box };
            let fresh: Vec<Option<(V3, f64)>> = self.tris[done..]
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|x| points[x]);
                    circumcentre(a, b, c).map(|cc| (cc, norm2(sub(a, cc))))
                })
                .collect();
            self.spheres.extend(fresh.iter().copied());
            for (cc, r2) in fresh.iter().flatten() {
                // slack for the rounding of the radius and of the box test
                let r = r2.sqrt() * (1. + 1e-6) + 1e-12 * (1. + norm2(*cc).sqrt());
                for k in 0..3 {
                    lo[k] = lo[k].min(cc[k] - r);
                    hi[k] = hi[k].max(cc[k] + r);
                }
            }
            self.ball_box = (lo, hi);
            // every change of `tris` leaves `spheres` shorter than `tris`, so the grid is rebuilt after each one
            self.ball_grid = (self.tris.len() >= Self::GRID_MIN).then(|| {
                let mut grid = BallGrid::new(&[lo, hi]);
                for (i, sphere) in self.spheres.iter().enumerate() {
                    if let Some((cc, r2)) = sphere {
                        grid.place(i, *cc, r2.sqrt(), true);
                    }
                }
                grid
            });
        }
        debug_assert!(
            self.tris.iter().zip(&self.spheres).all(|(t, s)| {
                let [a, b, c] = t.map(|x| points[x]);
                *s == circumcentre(a, b, c).map(|cc| (cc, norm2(sub(a, cc))))
            }),
            "stale sub-facet circumsphere cache"
        );
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
        // an edge of the mesh has both ends among its vertices
        if !self.uv.contains_key(&s[0]) || !self.uv.contains_key(&s[1]) {
            return false;
        }
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
    /// Is `p` inside the original facet (closed): in one of its input triangles?
    fn contains(&self, p: V3, points: &[V3]) -> bool {
        let q = self.project(p);
        self.region.iter().any(|t| {
            (0..3).all(|k| {
                let ua = self.uv_of(t[k], points);
                let ub = self.uv_of(t[(k + 1) % 3], points);
                orientation_2d(ua, ub, q) != Ordering::Less
            })
        })
    }
    fn uv_of(&self, v: usize, points: &[V3]) -> [f64; 2] {
        self.uv
            .get(&v)
            .copied()
            .unwrap_or_else(|| self.project(points[v]))
    }
}
/// Uniform grid over the points for "is any point inside this ball" queries: the encroachment tests would otherwise scan
/// every vertex. It only prunes candidates; the caller applies its own exact predicate, so answers equal the full scan.
/// Points outside the grid box (the far enclosing points, a Steiner point rounded off the shell) are always candidates.
#[derive(Clone)]
struct PointGrid {
    lo: V3,
    cell: f64,
    cells: Vec<Vec<u32>>,
    outliers: Vec<u32>,
    indexed: usize,
}
impl PointGrid {
    const N: usize = 40;
    /// Most cells a query may visit before the caller's linear scan is cheaper.
    const MAX_CELLS: usize = 4096;
    fn new(vertices: &[V3]) -> Self {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0., f64::max);
        let cell = if extent.is_finite() && extent > 0. { extent / Self::N as f64 * (1. + 1e-9) } else { 1. };
        Self { lo, cell, cells: vec![Vec::new(); Self::N.pow(3)], outliers: Vec::new(), indexed: 0 }
    }
    fn coord(&self, x: f64, k: usize) -> Option<usize> {
        let f = ((x - self.lo[k]) / self.cell).floor();
        (f >= 0. && f < Self::N as f64).then_some(f as usize)
    }
    fn sync(&mut self, pts: &[V3]) {
        for (i, p) in pts.iter().enumerate().skip(self.indexed) {
            match (self.coord(p[0], 0), self.coord(p[1], 1), self.coord(p[2], 2)) {
                (Some(x), Some(y), Some(z)) => self.cells[(x * Self::N + y) * Self::N + z].push(i as u32),
                _ => self.outliers.push(i as u32),
            }
        }
        self.indexed = pts.len();
    }
    /// Whether `hit` holds for some point within `radius` of `centre`; `None` when the ball is too large for the grid.
    fn any_within(&mut self, pts: &[V3], centre: V3, radius: f64, hit: &dyn Fn(usize) -> bool) -> Option<bool> {
        if !(radius.is_finite() && centre.iter().all(|x| x.is_finite())) {
            return None;
        }
        self.sync(pts);
        // slack for the rounding of the radius and of the cell index
        let r = radius * (1. + 1e-6) + 1e-12 * (1. + norm2(centre).sqrt());
        let mut range = [(0usize, 0usize); 3];
        let mut count = 1usize;
        for k in 0..3 {
            let f = |x: f64| ((x - self.lo[k]) / self.cell).floor().clamp(-1., Self::N as f64);
            let (a, b) = (f(centre[k] - r), f(centre[k] + r));
            if b < 0. || a >= Self::N as f64 {
                // the ball misses the grid box on this axis: only outliers can be inside
                range[k] = (1, 0);
                count = 0;
            } else {
                range[k] = (a.max(0.) as usize, b.min(Self::N as f64 - 1.) as usize);
                count = count.saturating_mul(range[k].1 - range[k].0 + 1);
            }
        }
        if count > Self::MAX_CELLS {
            return None;
        }
        if self.outliers.iter().any(|&i| hit(i as usize)) {
            return Some(true);
        }
        if count == 0 {
            return Some(false);
        }
        for x in range[0].0..=range[0].1 {
            for y in range[1].0..=range[1].1 {
                for z in range[2].0..=range[2].1 {
                    if self.cells[(x * Self::N + y) * Self::N + z].iter().any(|&i| hit(i as usize)) {
                        return Some(true);
                    }
                }
            }
        }
        Some(false)
    }
}
/// Multi-level grid over balls, answering "which balls hold this point" without scanning all of them. A ball of diameter `d`
/// lives on the level whose cell size is at least `d` (cell = root / 2^level), in every cell its bounding box touches (at
/// most 3 per axis), so a point inside the ball is in one of those cells; a ball wider than the root cell is kept in a list
/// every query returns. It only prunes candidates: the caller applies the exact test, so the answers equal the full scan.
/// Segments use it for their diametral balls ([`SegmentGrid`]), facet meshes for the boxes around their sub-facet balls.
#[derive(Clone)]
struct BallGrid<K> {
    lo: V3,
    root: f64,
    buckets: FastMap<(u8, i64, i64, i64), Vec<K>>,
    oversize: Vec<K>,
    lowest: u8,
    highest: u8,
}
type SegmentGrid = BallGrid<[usize; 2]>;
impl<K: Copy + PartialEq> BallGrid<K> {
    const MAX_LEVEL: u8 = 60;
    fn new(vertices: &[V3]) -> Self {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0., f64::max);
        let root = if extent.is_finite() && extent > 0. { extent * 2. } else { 1. };
        Self { lo, root, buckets: FastMap::default(), oversize: Vec::new(), lowest: Self::MAX_LEVEL, highest: 0 }
    }
    fn level(&self, length: f64) -> u8 {
        let l = (self.root / length).log2().floor();
        if l.is_finite() { l.clamp(0., Self::MAX_LEVEL as f64) as u8 } else { Self::MAX_LEVEL }
    }
    fn cell(&self, level: u8, x: f64, k: usize) -> i64 {
        ((x - self.lo[k]) * 2f64.powi(level as i32) / self.root).floor() as i64
    }
    /// The cell ranges of a ball, with slack for the rounding of the radius and of the cell index.
    fn ranges(&self, level: u8, centre: V3, radius: f64) -> [(i64, i64); 3] {
        let r = radius * (1. + 1e-6) + 1e-12 * (1. + norm2(centre).sqrt());
        std::array::from_fn(|k| (self.cell(level, centre[k] - r, k), self.cell(level, centre[k] + r, k)))
    }
    /// Add or remove `key` with the ball (`centre`, `radius`); a removal must repeat the ball of the addition.
    fn place(&mut self, key: K, centre: V3, radius: f64, add: bool) {
        // a ball wider than the root cell (or not finite) would touch too many cells: it is a candidate of every query
        if !(2. * radius <= self.root && centre.iter().all(|x| x.is_finite())) {
            if add {
                self.oversize.push(key);
            } else if let Some(at) = self.oversize.iter().position(|e| *e == key) {
                self.oversize.swap_remove(at);
            }
            return;
        }
        let level = self.level(2. * radius);
        let range = self.ranges(level, centre, radius);
        for x in range[0].0..=range[0].1 {
            for y in range[1].0..=range[1].1 {
                for z in range[2].0..=range[2].1 {
                    let key3 = (level, x, y, z);
                    if add {
                        self.buckets.entry(key3).or_default().push(key);
                    } else if let Some(b) = self.buckets.get_mut(&key3) {
                        if let Some(at) = b.iter().position(|e| *e == key) {
                            b.swap_remove(at);
                        }
                        if b.is_empty() {
                            self.buckets.remove(&key3);
                        }
                    }
                }
            }
        }
        if add {
            self.lowest = self.lowest.min(level);
            self.highest = self.highest.max(level);
        }
    }
    /// Every key whose ball may hold `p` (a superset of those that do), each once per level it lives on: a key sits in
    /// several cells of its level, but a point lies in exactly one cell per level.
    fn candidates(&self, p: V3) -> impl Iterator<Item = K> + '_ {
        let (mut lo, mut hi) = (self.lowest, self.highest);
        if lo > hi {
            (lo, hi) = (1, 0);
        }
        (lo..=hi)
            .filter_map(move |level| {
                self.buckets.get(&(level, self.cell(level, p[0], 0), self.cell(level, p[1], 1), self.cell(level, p[2], 2)))
            })
            .flatten()
            .chain(&self.oversize)
            .copied()
    }
}
impl SegmentGrid {
    fn place_segment(&mut self, s: [usize; 2], pts: &[V3], add: bool) {
        let (pa, pb) = (pts[s[0]], pts[s[1]]);
        let centre: V3 = std::array::from_fn(|i| (pa[i] + pb[i]) * 0.5);
        self.place(s, centre, norm2(sub(pa, pb)).sqrt() * 0.5, add);
    }
    fn add(&mut self, s: [usize; 2], pts: &[V3]) {
        self.place_segment(s, pts, true);
    }
    fn remove(&mut self, s: [usize; 2], pts: &[V3]) {
        self.place_segment(s, pts, false);
    }
    /// The segments (other than those with an end in `skip`) whose diametral ball holds `p`, in no particular order.
    fn encroached_by(&self, pts: &[V3], p: V3, skip: Option<usize>) -> Vec<[usize; 2]> {
        // a segment is a candidate once: it lives on one level, and only a segment longer than the root cell (none: the
        // root is twice the input extent) is in the oversize list
        self.candidates(p)
            .filter(|s| skip.map_or(true, |v| s[0] != v && s[1] != v) && encroaches(pts[s[0]], pts[s[1]], p))
            .collect()
    }
}
/// The facet meshes filed by the box around their sub-facet balls, so an encroachment query visits only the meshes near the
/// point: scanning every mesh for every candidate point took about a quarter of the sliver stage on ABC `00006600` (Phase
/// 160). Every change of a mesh's triangles must mark it `stale`; [`FacetIndex::near`] files it again before answering.
#[derive(Clone)]
struct FacetIndex {
    grid: BallGrid<usize>,
    /// The ball (around the mesh's ball box) each mesh is filed under; `None`: not filed, it has no ball.
    balls: Vec<Option<(V3, f64)>>,
    stale: Vec<usize>,
}
impl FacetIndex {
    fn new(vertices: &[V3], meshes: usize) -> Self {
        Self { grid: BallGrid::new(vertices), balls: vec![None; meshes], stale: (0..meshes).collect() }
    }
    /// The meshes whose ball box may hold `p`, in index order, with their sub-facet balls current.
    fn near(&mut self, meshes: &mut [FacetMesh], pts: &[V3], p: V3) -> Vec<usize> {
        for k in std::mem::take(&mut self.stale) {
            meshes[k].ensure_spheres(pts);
            let (lo, hi) = meshes[k].ball_box;
            let ball = (0..3).all(|i| lo[i] <= hi[i]).then(|| {
                let c: V3 = std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5);
                (c, norm2(sub(hi, c)).sqrt())
            });
            if ball != self.balls[k] {
                if let Some((c, r)) = self.balls[k] {
                    self.grid.place(k, c, r, false);
                }
                if let Some((c, r)) = ball {
                    self.grid.place(k, c, r, true);
                }
                self.balls[k] = ball;
            }
        }
        let mut near: Vec<usize> = self.grid.candidates(p).collect();
        near.sort_unstable();
        near.dedup();
        debug_assert!(near.iter().all(|&k| meshes[k].spheres.len() == meshes[k].tris.len()), "a changed facet mesh was not marked stale");
        near
    }
}
/// Every edge and face of the triangulation with the number of tetrahedra that have it (absent at zero).
#[derive(Clone, Default, Debug, PartialEq)]
struct Presence {
    edges: FastMap<[usize; 2], u32>,
    faces: FastMap<[usize; 3], u32>,
}
impl Presence {
    /// Count the edges and faces of the cell in (`add`) or out; an edge or face that appears or disappears goes to the
    /// logs of `missing`, when kept.
    fn count(&mut self, [a, b, c, d]: [usize; 4], add: bool, mut missing: Option<&mut Missing>) {
        /// Whether the key appeared or disappeared.
        fn step<K: std::hash::Hash + Eq>(map: &mut FastMap<K, u32>, key: K, add: bool) -> bool {
            if add {
                let n = map.entry(key).or_default();
                *n += 1;
                *n == 1
            } else if let std::collections::hash_map::Entry::Occupied(mut e) = map.entry(key) {
                *e.get_mut() -= 1;
                if *e.get() == 0 {
                    e.remove();
                    return true;
                }
                false
            } else {
                false
            }
        }
        for (x, y) in [(a, b), (a, c), (a, d), (b, c), (b, d), (c, d)] {
            let e = edge(x, y);
            if step(&mut self.edges, e, add) {
                if let Some(m) = missing.as_deref_mut() {
                    m.edge_log.push(e);
                }
            }
        }
        for f in [[a, b, c], [a, b, d], [a, c, d], [b, c, d]] {
            let f = face_key(f);
            if step(&mut self.faces, f, add) {
                if let Some(m) = missing.as_deref_mut() {
                    m.face_log.push(f);
                }
            }
        }
    }
}
/// The sub-segments and sub-facets missing from the triangulation, kept from one exact presence check of `drive` to the
/// next once the first has run: a check looks again only at what changed since (`State::missing_items`).
#[derive(Clone, Default)]
struct Missing {
    segments: BTreeSet<[usize; 2]>,
    /// Missing sub-facets of each mesh, in the order of its `tris`.
    tris: Vec<Vec<[usize; 3]>>,
    /// Since the last check: edges that appeared or disappeared, and segments added or removed.
    edge_log: Vec<[usize; 2]>,
    /// Since the last check: faces that appeared or disappeared.
    face_log: Vec<[usize; 3]>,
    /// Since the last check: meshes whose sub-facets changed.
    mesh_log: Vec<usize>,
}
#[derive(Clone)]
struct State {
    delaunay: Delaunay3,
    grid: std::cell::RefCell<PointGrid>,
    segments: BTreeSet<[usize; 2]>,
    seg_grid: SegmentGrid,
    meshes: Vec<FacetMesh>,
    /// The meshes holding each vertex (in their `uv`), kept current by [`State::own_vertex`]: scanning every mesh for the
    /// facets a split segment bounds took about 70% of the sliver stage on ABC `00007062` (Phase 164).
    vertex_meshes: Vec<Vec<usize>>,
    facet_index: FacetIndex,
    /// Edge and face counts of the triangulation once the first exact presence check has built them (`State::ensure_presence`).
    presence: Option<Presence>,
    /// Sub-segments / sub-facets known to be encroached (or forced) and not yet split.
    enc_segments: BTreeSet<[usize; 2]>,
    enc_tris: BTreeSet<(usize, [usize; 3])>,
    /// Entries of `enc_tris` queued because the exact presence check found the sub-facet missing, not because a vertex
    /// encroaches it: a refresh of the facet mesh drops them unless they are encroached too.
    forced_tris: BTreeSet<(usize, [usize; 3])>,
    /// Sub-segments and sub-facets the Delaunay triangulation lacked (their recovery is required), as opposed to those split
    /// only because a sliver's centre encroached them (optional).
    required_segments: BTreeSet<[usize; 2]>,
    required_tris: BTreeSet<(usize, [usize; 3])>,
    /// Optional splits refused at the feature-size floor: no sliver centre that encroaches one is tried again.
    refused_segments: BTreeSet<[usize; 2]>,
    /// Sub-facets a sliver's centre would have split but could not: left as they are (they are present).
    refused_tris: BTreeSet<(usize, [usize; 3])>,
    /// Shortest sub-segment a sliver may ask to have split: the shortest input segment, so that the splits forced by slivers
    /// stay a level or two below the input resolution instead of cascading around an acute vertex. `0` without slivers.
    optional_floor: f64,
    steiner: usize,
    /// Steiner points this attempt may add.
    steiner_cap: usize,
    segment_steiner: usize,
    facet_steiner: usize,
    quality_steiner: usize,
    /// Points (including the splits it forces) the sliver refinement may add; a quarter more ends it by a rollback.
    quality_cap: usize,
    /// Point count when the sliver refinement began.
    quality_start: Option<usize>,
    quality_clock: Option<std::time::Instant>,
    /// Rounds of the sliver refinement so far (it ends at 5000).
    quality_rounds: usize,
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
    /// Cell classification carried between quality rounds.
    cell_cache: Option<CellCache>,
    /// Missing sub-segments and sub-facets carried between presence checks.
    missing: Option<Missing>,
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
    /// The edges and faces of the triangulation, each with the number of tetrahedra that have it, built from every cell.
    fn tet_edges_and_faces(&self) -> Presence {
        let mut p = Presence::default();
        for t in self.delaunay.cells() {
            p.count(t, true, None);
        }
        p
    }
    /// A copy to return to after a failed sliver stage, without the caches that are rebuilt from scratch when absent (the
    /// presence table, the kept missing items and the kept cell classification): cloning and dropping them took about a
    /// tenth of the stage on ABC `00006600` (Phase 160).
    fn rollback_copy(&mut self) -> State {
        let caches = (self.presence.take(), self.missing.take(), self.cell_cache.take());
        let copy = self.clone();
        (self.presence, self.missing, self.cell_cache) = caches;
        copy
    }
    /// The edge and face counts of the triangulation, built once and then kept current by `insert`: rebuilding them for every
    /// exact presence check took about three quarters of the sliver stage on ABC `00006600` (Phase 159).
    fn ensure_presence(&mut self) {
        if self.presence.is_none() {
            self.presence = Some(self.tet_edges_and_faces());
        }
    }
    /// Mark the sub-facets of mesh `k` changed (for the facet index and the kept missing items).
    fn mesh_changed(&mut self, k: usize) {
        self.facet_index.stale.push(k);
        if let Some(m) = self.missing.as_mut() {
            m.mesh_log.push(k);
        }
    }
    /// Note that mesh `k` holds vertex `v` (call it after every insertion into a mesh; a mesh never loses a vertex).
    fn own_vertex(&mut self, k: usize, v: usize) {
        if self.vertex_meshes.len() <= v {
            self.vertex_meshes.resize(v + 1, Vec::new());
        }
        if !self.vertex_meshes[v].contains(&k) {
            self.vertex_meshes[v].push(k);
        }
    }
    /// The sub-segments (in order) and sub-facets (by mesh, in `tris` order) missing from the triangulation, from every one.
    fn missing_items_from_scratch(&mut self) -> (Vec<[usize; 2]>, Vec<(usize, [usize; 3])>) {
        self.ensure_presence();
        let Some(Presence { edges, faces }) = &self.presence else { unreachable!() };
        let segments = self.segments.iter().copied().filter(|s| !edges.contains_key(s)).collect();
        let tris = self
            .meshes
            .iter()
            .enumerate()
            .flat_map(|(k, m)| m.tris.iter().map(move |t| (k, *t)))
            .filter(|(_, t)| !faces.contains_key(&face_key(*t)))
            .collect();
        (segments, tris)
    }
    /// [`State::missing_items_from_scratch`], looking again only at the edges and faces that appeared or disappeared, the
    /// segments added or removed and the meshes changed since the previous call (`PLC_VERIFY_KEPT` checks it is the same).
    fn missing_items(&mut self) -> (Vec<[usize; 2]>, Vec<(usize, [usize; 3])>) {
        self.ensure_presence();
        let Some(Presence { edges, faces }) = &self.presence else { unreachable!() };
        let absent = |t: &[usize; 3]| !faces.contains_key(&face_key(*t));
        let m = match self.missing.as_mut() {
            None => {
                let m = Missing {
                    segments: self.segments.iter().copied().filter(|s| !edges.contains_key(s)).collect(),
                    tris: self.meshes.iter().map(|mesh| mesh.tris.iter().copied().filter(absent).collect()).collect(),
                    ..Missing::default()
                };
                self.missing.insert(m)
            }
            Some(m) => {
                for e in std::mem::take(&mut m.edge_log) {
                    if self.segments.contains(&e) && !edges.contains_key(&e) {
                        m.segments.insert(e);
                    } else {
                        m.segments.remove(&e);
                    }
                }
                let mut dirty = std::mem::take(&mut m.mesh_log);
                // a face can only be a sub-facet of a mesh holding all three of its vertices
                for f in std::mem::take(&mut m.face_log) {
                    for &k in self.vertex_meshes.get(f[0]).map_or(&[][..], |v| v) {
                        let uv = &self.meshes[k].uv;
                        if uv.contains_key(&f[1]) && uv.contains_key(&f[2]) {
                            dirty.push(k);
                        }
                    }
                }
                dirty.sort_unstable();
                dirty.dedup();
                m.tris.resize(self.meshes.len(), Vec::new());
                for k in dirty {
                    m.tris[k] = self.meshes[k].tris.iter().copied().filter(absent).collect();
                }
                m
            }
        };
        let tris = m.tris.iter().enumerate().flat_map(|(k, ts)| ts.iter().map(move |t| (k, *t))).collect();
        (m.segments.iter().copied().collect(), tris)
    }
    fn segment_encroached(&self, s: [usize; 2]) -> bool {
        let pts = self.points();
        let (pa, pb) = (pts[s[0]], pts[s[1]]);
        let hit = |p: usize| p != s[0] && p != s[1] && encroaches(pa, pb, pts[p]);
        let centre: V3 = std::array::from_fn(|i| (pa[i] + pb[i]) * 0.5);
        let radius = norm2(sub(pa, pb)).sqrt() * 0.5;
        self.grid
            .borrow_mut()
            .any_within(pts, centre, radius, &hit)
            .unwrap_or_else(|| (0..pts.len()).any(hit))
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
        let hit = |p: usize| !t.contains(&p) && norm2(sub(pts[p], cc)) < r2 * (1. - 1e-9);
        Ok(self
            .grid
            .borrow_mut()
            .any_within(pts, cc, r2.sqrt(), &hit)
            .unwrap_or_else(|| (0..pts.len()).any(hit)))
    }
    /// Re-derive the encroachment bookkeeping of one facet mesh against all vertices.
    fn refresh_mesh(&mut self, k: usize) -> Result<(), Error> {
        if !self.options.protect_encroached {
            return Ok(());
        }
        self.enc_tris.retain(|(m, _)| *m != k);
        self.forced_tris.retain(|(m, _)| *m != k);
        if self.frozen_facets.contains(&k) {
            return Ok(());
        }
        for i in 0..self.meshes[k].tris.len() {
            let t = self.meshes[k].tris[i];
            if !self.refused_tris.contains(&(k, t))
                && !self.unrefinable.contains(&(k, t))
                && self.tri_encroached(t)?
            {
                self.enc_tris.insert((k, t));
            }
        }
        Ok(())
    }
    /// [`Self::refresh_mesh`] after the vertex `v` was inserted into facet mesh `k` and [`Self::note_vertex`] ran for it.
    /// Vertices are only ever added, so an existing sub-facet stays encroached once it is, and a sub-facet that was not
    /// encroached can only become so through the new vertex, which `note_vertex` has already tested against it. The only
    /// sub-facets whose status is unknown are the new ones, which are exactly those with `v` as a corner (the insertion
    /// replaces a cavity by a cone from `v`): they are checked against every vertex, and entries of sub-facets that left the
    /// mesh are dropped. Same bookkeeping as the full refresh at O(new x V) instead of O(sub-facets x V).
    fn refresh_mesh_around(&mut self, k: usize, v: usize) -> Result<(), Error> {
        if !self.options.protect_encroached {
            return Ok(());
        }
        if self.frozen_facets.contains(&k) {
            self.enc_tris.retain(|(m, _)| *m != k);
            self.forced_tris.retain(|(m, _)| *m != k);
            return Ok(());
        }
        let live: BTreeSet<[usize; 3]> = self.meshes[k].tris.iter().copied().collect();
        // forced entries survive a refresh only when a vertex encroaches them as well
        let forced: Vec<(usize, [usize; 3])> =
            self.forced_tris.range((k, [0; 3])..(k + 1, [0; 3])).copied().collect();
        for entry in forced {
            self.forced_tris.remove(&entry);
            if !(live.contains(&entry.1) && self.tri_encroached(entry.1)?) {
                self.enc_tris.remove(&entry);
            }
        }
        let stale: Vec<(usize, [usize; 3])> = self
            .enc_tris
            .range((k, [0; 3])..(k + 1, [0; 3]))
            .filter(|(_, t)| !live.contains(t))
            .copied()
            .collect();
        for entry in stale {
            self.enc_tris.remove(&entry);
        }
        for t in live {
            if t.contains(&v)
                && !self.refused_tris.contains(&(k, t))
                && !self.unrefinable.contains(&(k, t))
                && self.tri_encroached(t)?
            {
                self.enc_tris.insert((k, t));
            }
        }
        self.enc_tris.retain(|entry| !self.refused_tris.contains(entry) && !self.unrefinable.contains(entry));
        // The incremental bookkeeping must equal the full recomputation (checked in debug builds, including every test).
        #[cfg(debug_assertions)]
        {
            let mut full = BTreeSet::new();
            for &t in &self.meshes[k].tris {
                // Optional quality splits can be refused at the feature-size floor. They remain
                // geometrically encroached, but are deliberately absent from the work queue.
                if !self.refused_tris.contains(&(k, t))
                    && !self.unrefinable.contains(&(k, t))
                    && self.tri_encroached(t)?
                {
                    full.insert((k, t));
                }
            }
            let kept: BTreeSet<(usize, [usize; 3])> =
                self.enc_tris.range((k, [0; 3])..(k + 1, [0; 3])).copied().collect();
            debug_assert_eq!(kept, full, "incremental sub-facet encroachment bookkeeping diverged");
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
            .seg_grid
            .encroached_by(pts, p, Some(v))
            .into_iter()
            .filter(|s| !self.frozen_segments.contains(s))
            .collect();
        #[cfg(debug_assertions)]
        {
            // the grid only prunes: it must return exactly what the full scan does
            let full: BTreeSet<[usize; 2]> = self
                .segments
                .iter()
                .copied()
                .filter(|s| s[0] != v && s[1] != v && !self.frozen_segments.contains(s) && encroaches(pts[s[0]], pts[s[1]], p))
                .collect();
            let got: BTreeSet<[usize; 2]> = hit.iter().copied().collect();
            debug_assert_eq!(got, full, "segment grid disagrees with the full scan");
        }
        self.enc_segments.extend(hit);
        let mut hit_tris = Vec::new();
        for k in self.facet_index.near(&mut self.meshes, pts, p) {
            let m = &self.meshes[k];
            if self.frozen_facets.contains(&k) {
                continue;
            }
            if (0..3).any(|i| p[i] < m.ball_box.0[i] || p[i] > m.ball_box.1[i]) {
                continue;
            }
            for (t, sphere) in m.tris.iter().zip(&m.spheres) {
                if t.contains(&v) {
                    continue;
                }
                if let Some((cc, r2)) = sphere {
                    if norm2(sub(p, *cc)) < r2 * (1. - 1e-9) {
                        if !self.refused_tris.contains(&(k, *t)) && !self.unrefinable.contains(&(k, *t)) {
                            hit_tris.push((k, *t));
                        }
                    }
                }
            }
        }
        self.enc_tris.extend(hit_tris);
        Ok(())
    }
    /// One batch of quality refinement. Returns whether anything was queued or
    /// inserted (false: nothing left to do or nothing more can be done).
    fn refine_quality(&mut self, bound: f64, min_sj: f64) -> Result<bool, Error> {
        let pts = self.points().to_vec();
        let bad = self.bad_cells(bound, min_sj, &pts);
        if std::env::var_os("PLC_VERIFY_KEPT").is_some() {
            let (full, inside) = self.bad_cells_from_scratch(bound, min_sj, &pts);
            let kept = &self.cell_cache.as_ref().expect("set by bad_cells").inside;
            assert!(
                full.len() == bad.len() && full.iter().zip(&bad).all(|(a, b)| a.0.to_bits() == b.0.to_bits() && a.1 == b.1 && a.2 == b.2),
                "kept cell classification differs from a full one: {} against {} bad cells",
                bad.len(),
                full.len()
            );
            assert!(self.delaunay.ids().into_iter().all(|id| inside[id] == kept[id]), "kept inside flags differ from a full classification");
        }
        self.quality_rounds += 1;
        if min_sj > 0. && self.quality_rounds > 5000 {
            self.unresolved_bad = bad.len();
            return Ok(false);
        }
        // Sliver refinement is best effort: stop with headroom left rather than run the budget out in the middle of the
        // splits a centre forces.
        if min_sj > 0.
            && (self.steiner.saturating_sub(self.quality_start.unwrap_or(0)) >= self.quality_cap || self.steiner + 64 + self.steiner_cap / 16 >= self.steiner_cap)
        {
            self.unresolved_bad = bad.len();
            return Ok(false);
        }
        if std::env::var_os("STAGEDBG").is_some() {
            eprintln!("refine round: bad {} steiner {} cap {} quality {}/{}", bad.len(), self.steiner, self.steiner_cap, self.quality_steiner, self.quality_cap);
        }
        let batch = if min_sj > 0. { (bad.len() / 4).clamp(1, 2048) } else { (bad.len() / 8).clamp(1, 256) };
        let mut current = pts.clone();
        let mut killed = BTreeSet::new();
        let mut progressed = false;
        let mut unresolved = 0;
        // STAGEDBG: why cells stay (no centre, frozen/refused segment, segment at the floor, frozen/refused sub-facet, insertion)
        let mut why = [0usize; 5];
        let (mut placed, mut created_bad, mut queued) = (0usize, 0usize, 0usize);
        // STAGEDBG: segments the queued points encroach (input, sub-segment; length over the floor < 2, < 4, < 16, more) and
        // queued slivers with a non-encroaching point in the picking region
        let (mut seg_kind, mut seg_len, mut free_alt) = ([0usize; 2], [0usize; 4], 0usize);
        let stagedbg = std::env::var_os("STAGEDBG").is_some();
        for (_, t, id) in bad.into_iter().take(batch) {
            let mut key = t;
            key.sort_unstable();
            if killed.contains(&key) {
                continue;
            }
            let corners = t.map(|v| pts[v]);
            let centroid: V3 = std::array::from_fn(|k| corners.iter().map(|p| p[k]).sum::<f64>() / 4.);
            let longest2 = (0..4)
                .flat_map(|i| ((i + 1)..4).map(move |j| (i, j)))
                .map(|(i, j)| norm2(sub(corners[i], corners[j])))
                .fold(0., f64::max);
            // The circumcentre of a near-flat cell is ill-conditioned: rounded, it can lie outside the exact circumsphere, so
            // inserting it destroys nothing and the same cell (and the same centre, now a duplicate vertex) comes back every
            // round, or far away from the cell it should remove. Only a centre strictly inside the cell's own circumsphere
            // and within a few edge lengths of it is used; a sliver whose centre is not takes its centroid (always inside
            // the circumsphere), which a flat cell on a boundary face places in the face and so splits the face.
            let centre = tet_circumsphere(corners)
                .map(|(c, _)| c)
                .filter(|c| (min_sj <= 0. || norm2(sub(*c, centroid)) <= 9. * longest2) && sphere_side(corners, *c) == Ok(SphereSide::Inside));
            let cc = match centre {
                Some(c) => c,
                None if min_sj > 0. && tet_scaled_jacobian(&corners) < min_sj => centroid,
                None => {
                    unresolved += 1;
                    why[0] += 1;
                    continue;
                }
            };
            let centre = cc;
            // not killed: the cell is still alive under `id`
            let cc = if min_sj > 0. { self.better_point(corners, cc, &current, id) } else { cc };
            let hit: Vec<[usize; 2]> = self.seg_grid.encroached_by(&current, cc, None);
            // A centre that encroaches a frozen segment or facet cannot be placed (the item may not be split): the cell stays.
            // Neither can one that encroaches a sub-segment already at the floor when the cell is a sliver: a sliver is not
            // worth a refusal of the whole recovery.
            let frozen_hit = hit.iter().any(|s| self.frozen_segments.contains(s) || self.refused_segments.contains(s));
            if frozen_hit
                || (min_sj > 0.
                    && hit
                        .iter()
                        .any(|s| norm2(sub(current[s[0]], current[s[1]])).sqrt() < self.optional_floor))
            {
                unresolved += 1;
                why[if frozen_hit { 1 } else { 2 }] += 1;
                continue;
            }
            let hit_tris = self.facets_encroached_by(&current, cc);
            let hits_frozen = hit_tris.iter().any(|(k, _)| self.frozen_facets.contains(k));
            if hits_frozen || hit_tris.iter().any(|x| self.unrefinable.contains(x) || self.refused_tris.contains(x)) {
                unresolved += 1;
                why[3] += 1;
                continue;
            }
            if !hit.is_empty() || !hit_tris.is_empty() {
                if stagedbg && min_sj > 0. {
                    for s in &hit {
                        seg_kind[usize::from(s.iter().any(|&v| v >= self.input_vertices))] += 1;
                        let r = norm2(sub(current[s[0]], current[s[1]])).sqrt() / self.optional_floor;
                        seg_len[[2., 4., 16.].iter().take_while(|&&b| r >= b).count()] += 1;
                    }
                    if Self::picking_points(corners, centre).into_iter().any(|q| {
                        self.seg_grid.encroached_by(&current, q, None).is_empty() && self.facets_encroached_by(&current, q).is_empty()
                    }) {
                        free_alt += 1;
                    }
                }
                self.enc_segments.extend(hit);
                // encroached by the rejected centre, not by a vertex
                self.forced_tris.extend(hit_tris.iter().copied());
                self.enc_tris.extend(hit_tris);
                progressed = true;
                queued += 1;
                continue;
            }
            match self.insert_near(cc, Some(id)) {
                Ok(i) => {
                    debug_assert_eq!(i, current.len());
                    current.push(cc);
                    self.quality_steiner += 1;
                    progressed = true;
                    placed += 1;
                    if min_sj > 0. && std::env::var_os("STAGEDBG").is_some() {
                        let pts = self.points();
                        created_bad += self
                            .delaunay
                            .last_added()
                            .iter()
                            .filter(|t| tet_scaled_jacobian(&t.map(|v| pts[v])) < min_sj)
                            .count();
                    }
                    for r in self.delaunay.last_removed() {
                        let mut k = *r;
                        k.sort_unstable();
                        killed.insert(k);
                    }
                }
                // A centre outside the hull or a degenerate cavity: leave this cell.
                Err(e) if e.kind == ErrorKind::Geometry => {
                    unresolved += 1;
                    why[4] += 1;
                }
                Err(e) => return Err(e),
            }
        }
        if std::env::var_os("STAGEDBG").is_some() {
            eprintln!("refine why: no-centre {} frozen-segment {} floor-segment {} frozen-facet {} insertion {} | placed {placed} created-bad {created_bad} queued-splits {queued} | segments input/sub {seg_kind:?} by length/floor <2,<4,<16,more {seg_len:?} free-alternative {free_alt}", why[0], why[1], why[2], why[3], why[4]);
        }
        self.unresolved_bad = unresolved;
        Ok(progressed)
    }
    /// The badness key of the inside cell `t` when the refinement should remove it: its radius-edge ratio over `bound`
    /// (`f64::MAX` without a circumsphere), else, for a sliver under `min_sj`, a key below `bound` that grows as the scaled
    /// Jacobian falls.
    fn badness(t: [usize; 4], pts: &[V3], bound: f64, min_sj: f64) -> Option<f64> {
        let corners = t.map(|v| pts[v]);
        match tet_circumsphere(corners) {
            Some((_, ratio)) if ratio <= bound => {
                if min_sj > 0. {
                    let sj = tet_scaled_jacobian(&corners);
                    if sj < min_sj {
                        // after every cell the radius-edge bound rejects, the worst sliver first
                        return Some(bound.min(1e6) * (1. - sj / min_sj).max(0.));
                    }
                }
                None
            }
            found => Some(found.map_or(f64::MAX, |x| x.1)),
        }
    }
    /// The inside cells to remove, worst first (ties by cell id), with their keys and Delaunay ids, from a full
    /// classification of every cell; also the inside flags by id.
    fn bad_cells_from_scratch(&self, bound: f64, min_sj: f64, pts: &[V3]) -> (Vec<(f64, [usize; 4], usize)>, Vec<bool>) {
        let (cells, neighbors) = self.delaunay.snapshot();
        let ids = self.delaunay.ids();
        let facet_list: Vec<[usize; 3]> = self.meshes.iter().flat_map(|m| m.tris.iter().copied()).collect();
        let outside = outside_cells(&cells, &neighbors, &facet_list, pts);
        let mut inside = vec![false; self.delaunay.id_bound()];
        let mut bad = Vec::new();
        for ((t, out), &id) in cells.iter().zip(&outside).zip(&ids) {
            if !*out {
                inside[id] = true;
                if let Some(key) = Self::badness(*t, pts, bound, min_sj) {
                    bad.push((key, *t, id));
                }
            }
        }
        bad.sort_by(|a, b| b.0.total_cmp(&a.0));
        (bad, inside)
    }
    /// [`State::bad_cells_from_scratch`], classifying only the cells created since the previous call: the result is the
    /// same (`PLC_VERIFY_KEPT` checks it every round).
    fn bad_cells(&mut self, bound: f64, min_sj: f64, pts: &[V3]) -> Vec<(f64, [usize; 4], usize)> {
        let created = self.delaunay.take_created();
        let kept = self.cell_cache.as_ref().is_some_and(|c| c.bound.to_bits() == bound.to_bits() && c.min_sj.to_bits() == min_sj.to_bits());
        if !kept {
            let (bad, inside) = self.bad_cells_from_scratch(bound, min_sj, pts);
            let mut key = vec![None; inside.len()];
            for &(k, _, id) in &bad {
                key[id] = Some(k);
            }
            self.cell_cache = Some(CellCache { bound, min_sj, inside, key });
            return bad;
        }
        let n = self.delaunay.id_bound();
        let facet_list: Vec<[usize; 3]> = self.meshes.iter().flat_map(|m| m.tris.iter().copied()).collect();
        let lookup = FacetLookup::new(&facet_list, pts.len());
        let delaunay = &self.delaunay;
        let cache = self.cell_cache.as_mut().expect("kept above");
        cache.inside.resize(n, false);
        cache.key.resize(n, None);
        let mut fresh: Vec<usize> = created.into_iter().filter(|&id| delaunay.cell(id).is_some()).collect();
        fresh.sort_unstable();
        fresh.dedup();
        // position + 1 in `fresh`, by id
        let mut slot = vec![0usize; n];
        for (i, &id) in fresh.iter().enumerate() {
            slot[id] = i + 1;
            cache.inside[id] = false;
        }
        // The same flood as `outside_cells`, over the new cells: seeded by a sub-facet they lie inside of, or by a kept
        // inside neighbour across a face that is no sub-facet.
        let mut blocked = vec![[false; 4]; fresh.len()];
        let mut stack = Vec::new();
        for (i, &id) in fresh.iter().enumerate() {
            let (v, nb) = delaunay.cell(id).expect("live");
            let (b, seeded) = lookup.faces(v, pts);
            blocked[i] = b;
            let joined = seeded || (0..4).any(|k| !b[k] && nb[k].is_some_and(|j| slot[j] == 0 && cache.inside[j]));
            if joined {
                cache.inside[id] = true;
                stack.push(i);
            }
        }
        while let Some(i) = stack.pop() {
            let (_, nb) = delaunay.cell(fresh[i]).expect("live");
            for k in 0..4 {
                if let Some(j) = nb[k] {
                    if slot[j] != 0 && !cache.inside[j] && !blocked[i][k] {
                        cache.inside[j] = true;
                        stack.push(slot[j] - 1);
                    }
                }
            }
        }
        for &id in &fresh {
            let (v, _) = delaunay.cell(id).expect("live");
            cache.key[id] = if cache.inside[id] { Self::badness(v, pts, bound, min_sj) } else { None };
        }
        let mut bad: Vec<(f64, [usize; 4], usize)> = (0..n)
            .filter_map(|id| {
                let k = cache.key[id]?;
                let (v, _) = delaunay.cell(id)?;
                Some((k, v, id))
            })
            .collect();
        bad.sort_by(|a, b| b.0.total_cmp(&a.0));
        bad
    }
    /// The sub-facets whose circumsphere (the smallest ball through their corners) has `p` strictly inside.
    fn facets_encroached_by(&mut self, current: &[V3], p: V3) -> Vec<(usize, [usize; 3])> {
        // the sub-facets of mesh `k`, in index order, by its ball grid when it has one and `by_grid`
        let scan = |k: usize, m: &FacetMesh, hit: &mut Vec<(usize, [usize; 3])>, by_grid: bool| {
            if (0..3).any(|i| p[i] < m.ball_box.0[i] || p[i] > m.ball_box.1[i]) {
                return;
            }
            let test = |i: usize| {
                if let Some((fc, r2)) = m.spheres[i] {
                    if norm2(sub(p, fc)) < r2 * (1. - 1e-9) {
                        hit.push((k, m.tris[i]));
                    }
                }
            };
            match m.ball_grid.as_ref().filter(|_| by_grid) {
                Some(grid) => {
                    let mut near: Vec<usize> = grid.candidates(p).collect();
                    near.sort_unstable();
                    near.dedup();
                    near.into_iter().for_each(test);
                }
                None => (0..m.tris.len()).for_each(test),
            }
        };
        let mut hit = Vec::new();
        for k in self.facet_index.near(&mut self.meshes, current, p) {
            scan(k, &self.meshes[k], &mut hit, true);
        }
        #[cfg(debug_assertions)]
        {
            // the index only prunes: it must return exactly what the scan of every mesh does
            let mut full = Vec::new();
            for (k, m) in self.meshes.iter_mut().enumerate() {
                m.ensure_spheres(current);
                scan(k, m, &mut full, false);
            }
            debug_assert_eq!(hit, full, "facet index disagrees with the full scan");
        }
        hit
    }
    /// A point for the sliver `corners` instead of its centre `cc`, when one is better. Inserting the centre often makes new
    /// slivers (on ABC `00006600` about 100 cells under the bound per point in the first rounds, about 2 in the last). Points a
    /// little away from the centre that are still strictly inside the cell's circumsphere remove the cell too (the picking
    /// region of Li's sliver removal); this takes the one whose new cells have the best worst scaled Jacobian, if that is better
    /// than the centre's and the point encroaches no segment or sub-facet. Otherwise the centre stays, with its usual handling.
    fn better_point(&mut self, corners: [V3; 4], cc: V3, current: &[V3], id: usize) -> V3 {
        // the worst scaled Jacobian of the cells inserting `q` would make; the scan stops at the first at most `floor`
        let worst_new = |q: V3, d: &mut Delaunay3, floor: f64| -> f64 {
            let Ok(cones) = d.preview(q, Some(id)) else { return f64::NEG_INFINITY };
            let at = |v: usize| if v == current.len() { q } else { current[v] };
            let mut worst = f64::INFINITY;
            for t in &cones {
                worst = worst.min(tet_scaled_jacobian(&t.map(at)));
                if worst <= floor {
                    break;
                }
            }
            worst
        };
        let base = worst_new(cc, &mut self.delaunay, f64::NEG_INFINITY);
        if !base.is_finite() || base >= self.options.min_scaled_jacobian.unwrap_or(0.) {
            return cc;
        }
        let free: Vec<V3> = Self::picking_points(corners, cc)
            .into_iter()
            .filter(|&q| self.seg_grid.encroached_by(current, q, None).is_empty() && self.facets_encroached_by(current, q).is_empty())
            .collect();
        // the first point with the strictly best value wins, so a later one only has to beat the best so far
        let mut best = (base, cc);
        for q in free {
            let w = worst_new(q, &mut self.delaunay, best.0);
            if w > best.0 {
                best = (w, q);
            }
        }
        best.1
    }
    /// The points around the centre `cc` of the cell `corners` that [`State::better_point`] tries: 14 directions at 0.3 of
    /// the smaller of the circumradius and the shortest edge, those strictly inside the circumsphere.
    fn picking_points(corners: [V3; 4], cc: V3) -> Vec<V3> {
        let shortest = (0..4)
            .flat_map(|i| ((i + 1)..4).map(move |j| (i, j)))
            .map(|(i, j)| norm2(sub(corners[i], corners[j])))
            .fold(f64::INFINITY, f64::min)
            .sqrt();
        let radius = 0.3 * norm2(sub(cc, corners[0])).sqrt().min(shortest);
        let s = 1. / 3f64.sqrt();
        [
            [1., 0., 0.], [-1., 0., 0.], [0., 1., 0.], [0., -1., 0.], [0., 0., 1.], [0., 0., -1.],
            [s, s, s], [s, s, -s], [s, -s, s], [s, -s, -s], [-s, s, s], [-s, s, -s], [-s, -s, s], [-s, -s, -s],
        ]
        .iter()
        .map(|u| std::array::from_fn(|k| cc[k] + radius * u[k]))
        .filter(|q| sphere_side(corners, *q) == Ok(SphereSide::Inside))
        .collect()
    }
    fn insert(&mut self, p: V3) -> Result<usize, Error> {
        self.insert_near(p, None)
    }
    /// [`State::insert`], locating `p` from the live Delaunay cell `near`.
    fn insert_near(&mut self, p: V3, near: Option<usize>) -> Result<usize, Error> {
        if self.steiner >= self.steiner_cap {
            return Err(fail(
                ErrorKind::Limit,
                BUDGET_MESSAGE,
            ));
        }
        let i = self.delaunay.insert_near(p, near).map_err(mesh_error)?;
        if let Some(presence) = self.presence.as_mut() {
            for &t in self.delaunay.last_removed() {
                presence.count(t, false, self.missing.as_mut());
            }
            for &t in self.delaunay.last_added() {
                presence.count(t, true, self.missing.as_mut());
            }
        }
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
                FLOOR_MESSAGE,
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
        if let Some(m) = self.missing.as_mut() {
            m.edge_log.extend([s, halves[0], halves[1]]);
        }
        {
            let pts = self.delaunay.points();
            self.seg_grid.remove(s, pts);
            for h in halves {
                self.seg_grid.add(h, pts);
            }
        }
        // Both facets bounded by the segment gain the new point on their edge; such a mesh holds both ends.
        let mut holding = self.vertex_meshes.get(a).cloned().unwrap_or_default();
        holding.sort_unstable();
        debug_assert_eq!(
            holding.iter().copied().filter(|&k| self.meshes[k].has_boundary_edge(s)).collect::<Vec<_>>(),
            (0..self.meshes.len()).filter(|&k| self.meshes[k].has_boundary_edge(s)).collect::<Vec<_>>(),
            "the vertex index disagrees with the scan of every mesh"
        );
        let mut touched = Vec::new();
        for k in holding {
            if self.meshes[k].has_boundary_edge(s) {
                self.mesh_changed(k);
                self.meshes[k].insert(mi, m, Some(s))?;
                self.own_vertex(k, mi);
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
            self.refresh_mesh_around(k, mi)?;
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
/// Groups of coplanar adjacent unfrozen input facets (see [`Options::merge_coplanar`]), ordered by their smallest facet.
/// A group is kept only if it is flat as a whole, so a chain of facets that each bend a hair cannot add up to a curved patch.
fn coplanar_groups(vertices: &[V3], facets: &[[usize; 3]], frozen: Option<&[bool]>) -> Vec<Vec<usize>> {
    const PAIR: f64 = 1e-9;
    const PATCH: f64 = 1e-8;
    let normal = |f: &[usize; 3]| {
        let [a, b, c] = f.map(|v| vertices[v]);
        cross(sub(b, a), sub(c, a))
    };
    let is_frozen = |i: usize| frozen.is_some_and(|fr| fr[i]);
    let mut by_edge = BTreeMap::<[usize; 2], Vec<usize>>::new();
    for (i, f) in facets.iter().enumerate() {
        for k in 0..3 {
            by_edge.entry(edge(f[k], f[(k + 1) % 3])).or_default().push(i);
        }
    }
    let mut parent: Vec<usize> = (0..facets.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    // the distance of `p` from the plane of facet `f`, relative to the facets' longest edge
    let off_plane = |f: usize, p: usize, scale: f64| {
        let n = normal(&facets[f]);
        let a = vertices[facets[f][0]];
        dot(n, sub(vertices[p], a)).abs() / (norm2(n).sqrt() * scale)
    };
    for pair in by_edge.values() {
        let [f, g] = pair[..] else { continue };
        if is_frozen(f) || is_frozen(g) {
            continue;
        }
        let (nf, ng) = (normal(&facets[f]), normal(&facets[g]));
        if !(dot(nf, ng) > 0. && norm2(nf) > 0. && norm2(ng) > 0.) {
            continue;
        }
        let scale = facets[f]
            .iter()
            .chain(&facets[g])
            .flat_map(|&u| facets[f].iter().chain(&facets[g]).map(move |&v| (u, v)))
            .map(|(u, v)| norm2(sub(vertices[u], vertices[v])).sqrt())
            .fold(0., f64::max);
        let flat = facets[g].iter().all(|&v| off_plane(f, v, scale) <= PAIR)
            && facets[f].iter().all(|&v| off_plane(g, v, scale) <= PAIR);
        if flat {
            let (a, b) = (find(&mut parent, f), find(&mut parent, g));
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for i in 0..facets.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let mut out = Vec::new();
    for members in groups.into_values() {
        let flat = members.len() == 1 || {
            let big = *members
                .iter()
                .max_by(|&&x, &&y| norm2(normal(&facets[x])).total_cmp(&norm2(normal(&facets[y]))))
                .unwrap();
            let verts: BTreeSet<usize> = members.iter().flat_map(|&m| facets[m]).collect();
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for &v in &verts {
                for k in 0..3 {
                    lo[k] = lo[k].min(vertices[v][k]);
                    hi[k] = hi[k].max(vertices[v][k]);
                }
            }
            let diagonal = norm2(sub(hi, lo)).sqrt();
            diagonal > 0. && verts.iter().all(|&v| off_plane(big, v, diagonal) <= PATCH)
        };
        if flat {
            out.push(members);
        } else {
            out.extend(members.into_iter().map(|m| vec![m]));
        }
    }
    out.sort_by_key(|m| m[0]);
    out
}
const FLOOR_MESSAGE: &str = "refinement descends below the feature-size floor around a vertex (input angles too small for the requested quality)";
const BUDGET_MESSAGE: &str = "Steiner point budget exceeded; the input may have acute segment or facet angles";
fn conform_impl(
    vertices: &[V3],
    facets: &[[usize; 3]],
    frozen: Option<&[bool]>,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Conforming, Error> {
    // The exact predicates refuse coordinates below 1e-30 in magnitude that are not zero. A coordinate that small is the
    // residue of a rotation that should have produced 0 (a STEP placement: -5.9e-31 instead of 0), never a feature, so it
    // is flushed to 0 here; the output points then differ from the input by less than 1e-30.
    const TINY: f64 = 1e-30;
    let flushed: Vec<V3>;
    let vertices = if vertices.iter().flatten().any(|x| *x != 0. && x.abs() < TINY) {
        flushed = vertices.iter().map(|v| v.map(|x| if x.abs() < TINY { 0. } else { x })).collect();
        &flushed[..]
    } else {
        vertices
    };
    let first = conform_attempt(vertices, facets, frozen, options, &mut progress);
    match first {
        Err(e)
            if options.protect_encroached
                && options.unprotected_fallback
                && (e.detail == FLOOR_MESSAGE || e.detail == BUDGET_MESSAGE) =>
        {
            let relaxed = Options { protect_encroached: false, ..options };
            // a refusal of the second attempt is the more informative one only when it is not the same kind of cascade
            conform_attempt(vertices, facets, frozen, relaxed, &mut progress)
                .map(|mut c| {
                    c.report.unprotected_fallback_used = true;
                    c
                })
                .map_err(|second| {
                    if second.detail == FLOOR_MESSAGE || second.detail == BUDGET_MESSAGE {
                        e
                    } else {
                        second
                    }
                })
        }
        other => other,
    }
}
/// The sliver refinement ends (by a rollback of the caller) when it has spent its points or its time.
fn quality_overdue(state: &State) -> Result<(), Error> {
    if let Some(start) = state.quality_start {
        let late = state.quality_clock.is_some_and(|c| c.elapsed().as_secs_f64() > state.options.max_quality_seconds);
        if late || state.steiner > start + state.quality_cap + state.quality_cap / 4 {
            return Err(fail(ErrorKind::Limit, BUDGET_MESSAGE));
        }
    }
    Ok(())
}
/// The recovery loop: splits what is encroached or missing until the shell conforms, then refines quality.
fn drive(
    state: &mut State,
    progress: &mut dyn FnMut(Progress) -> bool,
    rounds: &mut usize,
    snapshot: &mut Option<State>,
    next_snapshot: &mut usize,
) -> Result<(), Error> {
    let mut pts_cache: Vec<V3> = Vec::new();
    loop {
        quality_overdue(state)?;
        // Encroached segments before facets; each insertion updates both lists.
        while let Some(s) = state.enc_segments.pop_first() {
            if !state.segments.contains(&s) {
                continue;
            }
            quality_overdue(state)?;
            publish(
                &*state,
                Phase::Segments,
                (state.enc_segments.len() + 1, state.enc_tris.len()),
                progress,
            )?;
            // a split only a sliver asked for may descend no further than the input resolution: the sliver stays
            if state.quality_start.is_some() && !state.required_segments.contains(&s) {
                let [a, b] = s;
                if norm2(sub(state.points()[a], state.points()[b])).sqrt() < state.optional_floor {
                    state.refused_segments.insert(s);
                    continue;
                }
            }
            match state.split_segment(s) {
                Err(e) if e.detail == FLOOR_MESSAGE && state.quality_start.is_some() && !state.required_segments.contains(&s) => {
                    state.refused_segments.insert(s);
                }
                other => other?,
            }
        }
        if let Some((k, t)) = state.enc_tris.pop_first() {
            let was_forced = state.forced_tris.remove(&(k, t));
            if !state.meshes[k].tris.contains(&t) {
                continue;
            }
            publish(
                &*state,
                Phase::Facets,
                (0, state.enc_tris.len() + 1),
                progress,
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
            if state.unrefinable.contains(&(k, t)) || (state.refused_tris.contains(&(k, t)) && !state.required_tris.contains(&(k, t))) {
                continue;
            }
            let (cc, _) = state.tri_circumsphere(t)?;
            // points are only ever appended: the copy is extended, not rebuilt, for every sub-facet
            let have = pts_cache.len();
            pts_cache.extend_from_slice(&state.points()[have..]);
            let pts = &pts_cache;
            let hit: Vec<[usize; 2]> = state.seg_grid.encroached_by(&pts, cc, None);
            // The centre of a sub-facet that encroaches a frozen segment cannot be placed; a present sub-facet is left as it is.
            if hit.iter().any(|s| state.frozen_segments.contains(s)) {
                state.unrefinable.insert((k, t));
                continue;
            }
            if !hit.is_empty() {
                if !state.required_tris.contains(&(k, t)) && hit.iter().any(|s| state.refused_segments.contains(s)) {
                    state.refused_tris.insert((k, t));
                    continue;
                }
                // The centre is rejected; its encroached segments are split first. Recovering a required sub-facet makes
                // those splits required as well: the sliver floor refuses only optional splits, and a refused split here would
                // queue the same sub-facet and segment again forever (ABC `00006600`, Phase 158).
                if state.required_tris.contains(&(k, t)) {
                    state.required_segments.extend(hit.iter().copied());
                }
                state.enc_segments.extend(hit);
                state.enc_tris.insert((k, t));
                if was_forced {
                    state.forced_tris.insert((k, t));
                }
                continue;
            }
            if !state.meshes[k].contains(cc, &pts) {
                // In exact arithmetic a centre outside the facet encroaches a segment; here it did not (rounding on a skinny
                // sub-facet), so the boundary sub-segment the centre is nearest to, the one it lies beyond, is split instead and
                // the sub-facet revisited, as when the centre sits on the boundary.
                let nearest = state.meshes[k].nearest_boundary_edge(cc, &pts);
                if let Some(s) = nearest.filter(|s| state.segments.contains(s) && !state.frozen_segments.contains(s)) {
                    if !state.required_tris.contains(&(k, t)) && state.refused_segments.contains(&s) {
                        state.refused_tris.insert((k, t));
                        continue;
                    }
                    if state.required_tris.contains(&(k, t)) {
                        state.required_segments.insert(s);
                    }
                    state.enc_segments.insert(s);
                    state.enc_tris.insert((k, t));
                    if was_forced {
                        state.forced_tris.insert((k, t));
                    }
                    continue;
                }
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
            state.mesh_changed(k);
            if let Err(refusal) = state.meshes[k].insert(id, cc, None) {
                // The centre sits on the facet boundary within rounding: split the
                // closest boundary sub-segment instead and revisit the sub-facet.
                let nearest = state.meshes[k].nearest_boundary_edge(cc, &pts);
                if let Some(s) = nearest.filter(|s| state.segments.contains(s)) {
                    if !state.required_tris.contains(&(k, t)) && state.refused_segments.contains(&s) {
                        state.refused_tris.insert((k, t));
                        continue;
                    }
                    if state.required_tris.contains(&(k, t)) {
                        state.required_segments.insert(s);
                    }
                    state.enc_segments.insert(s);
                    state.enc_tris.insert((k, t));
                    if was_forced {
                        state.forced_tris.insert((k, t));
                    }
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
            state.own_vertex(k, id);
            let inserted = state.insert(cc)?;
            debug_assert_eq!(inserted, id);
            state.facet_steiner += 1;
            state.max_offset = state.max_offset.max(off / longest.sqrt());
            state.note_vertex(id)?;
            state.refresh_mesh_around(k, id)?;
            continue;
        }
        // Encroachment is only a sufficient condition; verify presence exactly and
        // force whatever a cospherical tie left out.
        *rounds += 1;
        let (missing_segments, missing_tris) = state.missing_items();
        if std::env::var_os("PLC_VERIFY_KEPT").is_some() {
            assert!(
                (missing_segments.clone(), missing_tris.clone()) == state.missing_items_from_scratch(),
                "kept missing items differ from a full presence check"
            );
        }
        let Some(Presence { faces, .. }) = &state.presence else { unreachable!() };
        // A cospherical tie can make the facet's planar triangulation pick the other
        // diagonal than the tetrahedralization: align it before refining anything.
        let mut flipped = 0;
        if missing_segments.is_empty() && !missing_tris.is_empty() {
            for (k, mesh) in state.meshes.iter_mut().enumerate() {
                let n = mesh.flip_toward(&faces);
                if n > 0 {
                    state.facet_index.stale.push(k);
                    if let Some(m) = state.missing.as_mut() {
                        m.mesh_log.push(k);
                    }
                }
                flipped += n;
            }
        }
        if flipped > 0 {
            state.cell_cache = None;
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
            let (bound, min_sj) = (state.options.max_radius_edge, state.options.min_scaled_jacobian);
            if bound.is_some() || min_sj.is_some() {
                // Sliver refinement can run into an input angle and fail late; the state before it is conforming and
                // good, so keep a copy, renewed whenever the point count has grown by a quarter, and return to it on a failure.
                state.quality_start.get_or_insert(state.steiner);
                state.quality_clock.get_or_insert_with(std::time::Instant::now);
                if min_sj.is_some() && state.steiner >= *next_snapshot {
                    *snapshot = Some(state.rollback_copy());
                    *next_snapshot = state.steiner + state.steiner / 4 + 8;
                }
                publish(&*state, Phase::Facets, (0, 0), progress)?;
                if state.refine_quality(bound.unwrap_or(f64::INFINITY), min_sj.unwrap_or(0.))? {
                    continue;
                }
            }
            return Ok(());
        }
        publish(
            &*state,
            Phase::Facets,
            (missing_segments.len(), missing_tris.len()),
            progress,
        )?;
        if !missing_segments.is_empty() {
            state.required_segments.extend(missing_segments.iter().copied());
            state.enc_segments.extend(missing_segments);
        } else {
            state.required_tris.extend(missing_tris.iter().copied());
            state.forced_tris.extend(missing_tris.iter().copied());
            state.enc_tris.extend(missing_tris);
        }
    }
}
fn publish(
    state: &State,
    phase: Phase,
    missing: (usize, usize),
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(), Error> {
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
}
/// The recovery's starting state: the Delaunay tetrahedralization of the input vertices, the facet meshes (and the patches
/// of input facets each one covers) and the segments, with the initially encroached items queued.
fn initial_state(
    vertices: &[V3],
    facets: &[[usize; 3]],
    frozen: Option<&[bool]>,
    options: Options,
) -> Result<(State, Vec<Vec<usize>>), Error> {
    let limits = DelaunayLimits {
        max_points: vertices.len() + options.max_steiner,
        max_cells: options.max_cells,
    };
    // One facet mesh per group of input facets (a single triangle unless coplanar facets are merged).
    let groups: Vec<Vec<usize>> = if options.merge_coplanar {
        coplanar_groups(vertices, facets, frozen)
    } else {
        (0..facets.len()).map(|i| vec![i]).collect()
    };
    let mut meshes = Vec::with_capacity(groups.len());
    let mut patches: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
    for members in groups {
        let tris: Vec<[usize; 3]> = members.iter().map(|&m| facets[m]).collect();
        match FacetMesh::patch(meshes.len(), &tris, vertices) {
            Ok(mesh) => {
                meshes.push(mesh);
                patches.push(members);
            }
            // a patch that does not hold together as one planar facet is kept as its triangles
            Err(_) if members.len() > 1 => {
                for m in members {
                    meshes.push(FacetMesh::new(meshes.len(), facets[m], vertices)?);
                    patches.push(vec![m]);
                }
            }
            Err(e) => return Err(e),
        }
    }
    let mut patch_of = vec![0; facets.len()];
    for (k, members) in patches.iter().enumerate() {
        for &m in members {
            patch_of[m] = k;
        }
    }
    // Segments are the edges between different facets: the edges inside a merged patch are free.
    let mut segments = BTreeSet::new();
    let mut edge_patch = BTreeMap::<[usize; 2], usize>::new();
    for (i, f) in facets.iter().enumerate() {
        for k in 0..3 {
            let e = edge(f[k], f[(k + 1) % 3]);
            match edge_patch.get(&e) {
                Some(&other) if other == patch_of[i] => {
                    segments.remove(&e);
                }
                Some(_) => {}
                None => {
                    edge_patch.insert(e, patch_of[i]);
                    segments.insert(e);
                }
            }
        }
    }
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
    let seg_grid = {
        let mut grid = SegmentGrid::new(vertices);
        for &e in &segments {
            grid.add(e, vertices);
        }
        grid
    };
    let segments_input_shortest = {
        let l = segments
            .iter()
            .map(|e| norm2(sub(vertices[e[0]], vertices[e[1]])).sqrt())
            .fold(f64::INFINITY, f64::min);
        if l.is_finite() {
            l
        } else {
            0.
        }
    };
    let mut state = State {
        delaunay: Delaunay3::new(&seed, options.max_cells),
        grid: std::cell::RefCell::new(PointGrid::new(vertices)),
        segments,
        seg_grid,
        facet_index: FacetIndex::new(vertices, meshes.len()),
        vertex_meshes: {
            let mut held = vec![Vec::new(); vertices.len()];
            for (k, mesh) in meshes.iter().enumerate() {
                for &v in mesh.uv.keys() {
                    held[v].push(k);
                }
            }
            held
        },
        meshes,
        presence: None,
        enc_segments: BTreeSet::new(),
        enc_tris: BTreeSet::new(),
        forced_tris: BTreeSet::new(),
        required_segments: BTreeSet::new(),
        required_tris: BTreeSet::new(),
        refused_segments: BTreeSet::new(),
        refused_tris: BTreeSet::new(),
        optional_floor: if options.min_scaled_jacobian.is_some() {
            segments_input_shortest
        } else {
            0.
        },
        steiner: 0,
        steiner_cap: if options.protect_encroached && options.unprotected_fallback {
            options.max_steiner.min(options.protected_max_steiner)
        } else {
            options.max_steiner
        },
        segment_steiner: 0,
        facet_steiner: 0,
        quality_steiner: 0,
        quality_start: None,
        quality_clock: None,
        quality_rounds: 0,
        quality_cap: options.max_quality_steiner.min((15 * vertices.len()).max(1500)),
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
            .map(|(i, _)| patch_of[i])
            .collect(),
        unrefinable: BTreeSet::new(),
        cell_cache: None,
        missing: None,
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
    let all: Vec<[usize; 2]> = state.segments.iter().copied().collect();
    for s in all {
        if options.protect_encroached && !state.frozen_segments.contains(&s) && state.segment_encroached(s) {
            state.enc_segments.insert(s);
        }
    }
    for k in 0..state.meshes.len() {
        state.refresh_mesh(k)?;
    }
    Ok((state, patches))
}
fn conform_attempt(
    vertices: &[V3],
    facets: &[[usize; 3]],
    frozen: Option<&[bool]>,
    options: Options,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<Conforming, Error> {
    if facets.is_empty() || facets.iter().flatten().any(|&v| v >= vertices.len()) {
        return Err(fail(ErrorKind::Topology, "invalid PLC facet list"));
    }
    let mut incidence = BTreeMap::<[usize; 2], usize>::new();
    for f in facets {
        for k in 0..3 {
            *incidence.entry(edge(f[k], f[(k + 1) % 3])).or_default() += 1;
        }
    }
    if incidence.values().any(|&n| n != 2) {
        return Err(fail(
            ErrorKind::Topology,
            "PLC must be a closed manifold shell (every edge in exactly two facets)",
        ));
    }
    if !progress(Progress {
        phase: Phase::Delaunay,
        steiner: 0,
        missing_segments: 0,
        missing_facets: 0,
    }) {
        return Err(fail(ErrorKind::Cancelled, "PLC recovery cancelled"));
    }
    let (mut state, patches) = initial_state(vertices, facets, frozen, options)?;
    let mut rounds = 0;
    let (mut snapshot, mut next_snapshot) = (None, 0);
    let mut cancelled = false;
    match drive(&mut state, progress, &mut rounds, &mut snapshot, &mut next_snapshot) {
        Ok(()) => {}
        // A refusal after the sliver stage began, or a cancellation during it, returns to the last conforming state it kept:
        // the sliver stage is an improvement of a result that already conforms and ends early without losing it.
        Err(e) if snapshot.is_some() => {
            if std::env::var_os("STAGEDBG").is_some() {
                eprintln!("rollback after {} quality points: {e}", state.quality_steiner);
            }
            cancelled = e.kind == ErrorKind::Cancelled;
            state = snapshot.take().unwrap();
        }
        Err(e) => return Err(e),
    }
    if !cancelled {
        publish(&state, Phase::Extraction, (0, 0), progress)?;
    }
    let mut result = extract(state, vertices.len(), rounds)?;
    // facet numbers of the result are input facet numbers: a merged patch hands each sub-facet to the member it lies in
    for (f, parent) in &mut result.facets {
        *parent = match patches[*parent][..] {
            [only] => only,
            _ => containing_facet(&result.points, facets, &patches[*parent], *f),
        };
    }
    Ok(result)
}
/// The member of `members` (input facets) that contains the centroid of the sub-facet `f` (the one it is deepest in).
fn containing_facet(points: &[V3], facets: &[[usize; 3]], members: &[usize], f: [usize; 3]) -> usize {
    let c: V3 = std::array::from_fn(|i| (points[f[0]][i] + points[f[1]][i] + points[f[2]][i]) / 3.);
    let depth = |m: usize| {
        let [a, b, d] = facets[m].map(|v| points[v]);
        let n = cross(sub(b, a), sub(d, a));
        let area2 = norm2(n);
        let w = [
            dot(cross(sub(b, c), sub(d, c)), n),
            dot(cross(sub(d, c), sub(a, c)), n),
            dot(cross(sub(a, c), sub(b, c)), n),
        ];
        w.iter().fold(f64::INFINITY, |x, y| x.min(*y)) / area2
    };
    *members
        .iter()
        .max_by(|&&x, &&y| depth(x).total_cmp(&depth(y)))
        .expect("a patch has members")
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
    let smooth_rounds = state.options.smooth_rounds;
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
    let mut points = topology.points().to_vec();
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
            // With the sliver recipe the flips (and the smoothing) judge cells by the scaled Jacobian the solver gate uses: by
            // radius ratio they turned the stage's cells into slivers (ABC `00006600`: 388 cells under 0.1 against 19, Phase 160).
            let by_sj = |p: &[fea_core::meshing::tet_topology::Point], t: [usize; 4]| {
                let sj = tet_scaled_jacobian(&t.map(|v| p[v]));
                if sj.is_finite() { sj.max(0.) } else { 0. }
            };
            let sliver_recipe = state.options.min_scaled_jacobian.is_some();
            if smooth_rounds > 0 {
                let smoothing = SmoothOptions::default();
                let (better, report) = if sliver_recipe {
                    optimize_by(&seed, &BTreeSet::new(), flip, &smoothing, smooth_rounds, &by_sj)
                } else {
                    optimize(&seed, &BTreeSet::new(), flip, &smoothing, smooth_rounds)
                }
                .map_err(mesh_error)?;
                // smoothing moves interior vertices: the points of the result are the improved ones
                points = better.points().to_vec();
                (better.cells().to_vec(), Some(report.flips))
            } else {
                let (better, report) = if sliver_recipe {
                    improve_by(&seed, &BTreeSet::new(), flip, &by_sj)
                } else {
                    improve(&seed, &BTreeSet::new(), flip)
                }
                .map_err(mesh_error)?;
                (better.cells().to_vec(), Some(report))
            }
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
            unprotected_fallback_used: false,
        },
        points,
        cells: interior.into_iter().map(|t| t.map(|v| renumber[v])).collect(),
        facets: facets.into_iter().map(|(f, k)| (f.map(|v| renumber[v]), k)).collect(),
        segments: stats.3.into_iter().map(|e| e.map(|v| renumber[v])).collect(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    /// The edge and face counts kept current by every insertion equal the ones rebuilt from all cells (Phase 159).
    #[test]
    fn presence_counts_follow_insertions() {
        let vertices = [
            [0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.],
            [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.],
        ];
        let facets = [
            [0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
            [3, 7, 6], [3, 6, 2], [0, 4, 7], [0, 7, 3], [1, 2, 6], [1, 6, 5],
        ];
        let (mut state, _) = initial_state(&vertices, &facets, None, Options::default()).unwrap();
        state.ensure_presence();
        for i in 0..200 {
            let x = |k: u64| ((i as u64 * 2654435761 + k * 40503) % 1000) as f64 / 1000.;
            state.insert([x(1), x(2), x(3)]).unwrap();
        }
        let rebuilt = state.tet_edges_and_faces();
        assert!(rebuilt.edges.len() > 1000);
        assert_eq!(state.presence.as_ref().unwrap(), &rebuilt);
    }
    /// A required sub-facet (one the triangulation lacked) whose circumcentre encroaches a segment shorter than the sliver
    /// floor: the split it needs is required, not optional. Refusing it at the floor queued the same sub-facet and segment again
    /// until the sliver stage's clock ran out and its work was rolled back (ABC `00006600`, Phase 158).
    #[test]
    fn a_required_sub_facet_gets_its_split_below_the_sliver_floor() {
        let vertices = [
            [0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.],
            [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.],
        ];
        let facets = [
            [0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
            [3, 7, 6], [3, 6, 2], [0, 4, 7], [0, 7, 3], [1, 2, 6], [1, 6, 5],
        ];
        let options = Options { min_scaled_jacobian: Some(0.2), max_quality_seconds: 5., ..Options::default() };
        let (mut state, _) = initial_state(&vertices, &facets, None, options).unwrap();
        // sliver stage under way, every segment below its floor; the right triangle's circumcentre is the midpoint of the
        // face diagonal 0-2, a segment
        let t = state.meshes[0].tris[0];
        assert!(state.segments.contains(&[0, 2]));
        state.optional_floor = 10.;
        state.quality_start = Some(0);
        state.quality_clock = Some(std::time::Instant::now());
        state.required_tris.insert((0, t));
        state.enc_tris.insert((0, t));
        let start = std::time::Instant::now();
        let (mut rounds, mut snapshot, mut next) = (0, None, usize::MAX);
        drive(&mut state, &mut |_| true, &mut rounds, &mut snapshot, &mut next).unwrap();
        assert!(state.segment_steiner >= 1, "the diagonal was not split");
        assert!(!state.segments.contains(&[0, 2]));
        assert!(start.elapsed().as_secs_f64() < 5., "{:?}", start.elapsed());
    }
    /// Inserting a sliver's circumcentre can make new slivers; the search returns a point inside the circumsphere whose new
    /// cells are better and that encroaches no segment or sub-facet (Phase 160).
    #[test]
    fn the_sliver_point_search_takes_a_better_point_than_the_centre() {
        // a unit box whose faces are 6 x 6 grids, so the sub-facet and segment balls stay near the faces
        let n: i64 = 6;
        let faces: [([i64; 3], [i64; 3], [i64; 3]); 6] = [
            ([0, 0, 0], [0, 1, 0], [1, 0, 0]),
            ([0, 0, 1], [1, 0, 0], [0, 1, 0]),
            ([0, 0, 0], [1, 0, 0], [0, 0, 1]),
            ([0, 1, 0], [0, 0, 1], [1, 0, 0]),
            ([0, 0, 0], [0, 0, 1], [0, 1, 0]),
            ([1, 0, 0], [0, 1, 0], [0, 0, 1]),
        ];
        let mut index = std::collections::HashMap::new();
        let (mut vertices, mut facets): (Vec<V3>, Vec<[usize; 3]>) = (Vec::new(), Vec::new());
        for (a, u, v) in faces {
            for i in 0..n {
                for j in 0..n {
                    let k = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)].map(|(s, t)| {
                        let p: [i64; 3] = std::array::from_fn(|c| a[c] * n + u[c] * s + v[c] * t);
                        *index.entry(p).or_insert_with(|| {
                            vertices.push(p.map(|x| x as f64 / n as f64));
                            vertices.len() - 1
                        })
                    });
                    facets.push([k[0], k[1], k[2]]);
                    facets.push([k[0], k[2], k[3]]);
                }
            }
        }
        let options = Options { min_scaled_jacobian: Some(0.2), ..Options::default() };
        let (mut state, _) = initial_state(&vertices, &facets, None, options).unwrap();
        let mut seed = 11u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..250 {
            let p: V3 = std::array::from_fn(|_| 0.05 + 0.9 * next());
            state.insert(p).unwrap();
        }
        let (cells, neighbors) = state.delaunay.snapshot();
        let current = state.points().to_vec();
        let facet_list: Vec<[usize; 3]> = state.meshes.iter().flat_map(|m| m.tris.iter().copied()).collect();
        let outside = outside_cells(&cells, &neighbors, &facet_list, &current);
        let worst = |cones: Vec<[usize; 4]>, q: V3| -> f64 {
            cones
                .iter()
                .map(|t| {
                    let corners = t.map(|v| if v == current.len() { q } else { current[v] });
                    tet_scaled_jacobian(&corners)
                })
                .fold(f64::INFINITY, f64::min)
        };
        let ids = state.delaunay.ids();
        let mut moved = 0;
        for ((t, out), &id) in cells.iter().zip(&outside).zip(&ids) {
            if *out {
                continue;
            }
            let corners = t.map(|v| current[v]);
            if tet_scaled_jacobian(&corners) >= 0.2 {
                continue;
            }
            let Some((c, _)) = tet_circumsphere(corners) else { continue };
            if sphere_side(corners, c) != Ok(SphereSide::Inside) {
                continue;
            }
            let r = state.better_point(corners, c, &current, id);
            if r == c {
                continue;
            }
            moved += 1;
            assert!(sphere_side(corners, r) == Ok(SphereSide::Inside), "{r:?} is not inside the circumsphere of {corners:?}");
            assert!(state.seg_grid.encroached_by(&current, r, None).is_empty(), "{r:?} encroaches a segment");
            assert!(state.facets_encroached_by(&current, r).is_empty(), "{r:?} encroaches a sub-facet");
            let (at_r, at_c) = (worst(state.delaunay.preview(r, None).unwrap(), r), worst(state.delaunay.preview(c, None).unwrap(), c));
            assert!(at_r > at_c, "{r:?}: {at_r} is not better than the centre's {at_c}");
        }
        assert!(moved >= 1, "the search never left the centre");
    }
    #[test]
    fn cancellation_precedes_delaunay_initialization() {
        let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let facets = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        let mut calls = 0;
        let error = conform(&vertices, &facets, Options { max_cells: 0, ..Options::default() }, |p| {
            calls += 1;
            assert_eq!(p.phase, Phase::Delaunay);
            assert_eq!(p.steiner, 0);
            false
        })
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
        assert_eq!(calls, 1);
    }
    /// The grid only prunes: every ball query equals the linear scan, including balls that reach outside the grid box,
    /// balls too large for the grid, points outside the box (outliers) and points added between queries.
    #[test]
    fn point_grid_ball_queries_equal_the_linear_scan() {
        let mut state = 7u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let base: Vec<V3> = (0..64).map(|_| std::array::from_fn(|_| next() * 10.)).collect();
        let mut grid = PointGrid::new(&base);
        let mut pts = base.clone();
        let mut grid_answers = 0;
        for round in 0..400 {
            if round % 5 == 0 {
                // new points: inside the box, just outside it, far away
                let scale = [10., 10.5, 1e3][round / 5 % 3];
                pts.push(std::array::from_fn(|_| next() * scale - 0.2));
            }
            let centre: V3 = std::array::from_fn(|_| next() * 14. - 2.);
            let radius = next().powi(3) * [0.5, 3., 40., 5e3][round % 4];
            let hit = |i: usize| norm2(sub(pts[i], centre)) < radius * radius * (1. - 1e-9);
            let expected = (0..pts.len()).any(hit);
            if let Some(answer) = grid.any_within(&pts, centre, radius, &hit) {
                grid_answers += 1;
                assert_eq!(answer, expected, "round {round}: ball {centre:?} r {radius}");
            }
        }
        assert!(grid_answers > 200, "the grid should answer most queries, answered {grid_answers}");
    }
    /// The ball grid only prunes: its candidates hold every ball that contains the point, after additions and removals, for
    /// balls of every size including those wider than the root cell and those outside the grid box.
    #[test]
    fn ball_grid_candidates_hold_every_ball_with_the_point() {
        let mut state = 5u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let corners: Vec<V3> = vec![[0.; 3], [10.; 3]];
        let mut grid = BallGrid::<usize>::new(&corners);
        let mut balls: Vec<Option<(V3, f64)>> = Vec::new();
        let mut pruned = 0;
        for round in 0..600 {
            if round % 3 != 2 || balls.iter().all(Option::is_none) {
                let centre: V3 = std::array::from_fn(|_| next() * 14. - 2.);
                let radius = next().powi(3) * [0.2, 2., 15., 1e4][round % 4];
                grid.place(balls.len(), centre, radius, true);
                balls.push(Some((centre, radius)));
            } else {
                let k = (next() * balls.len() as f64) as usize % balls.len();
                if let Some((c, r)) = balls[k].take() {
                    grid.place(k, c, r, false);
                }
            }
            let p: V3 = std::array::from_fn(|_| next() * 14. - 2.);
            let got: BTreeSet<usize> = grid.candidates(p).collect();
            for (k, b) in balls.iter().enumerate() {
                match b {
                    Some((c, r)) if norm2(sub(p, *c)) < r * r => assert!(got.contains(&k), "round {round}: ball {k} missed"),
                    None => assert!(!got.contains(&k), "round {round}: removed ball {k} returned"),
                    _ => {}
                }
            }
            pruned += balls.iter().flatten().count() - got.len().min(balls.iter().flatten().count());
        }
        assert!(pruned > 0, "the grid never pruned a ball");
    }

    fn quad(v: &mut Vec<V3>, f: &mut Vec<[usize; 3]>, a: V3, b: V3, c: V3, d: V3) {
        let n = v.len();
        v.extend([a, b, c, d]);
        f.push([n, n + 1, n + 2]);
        f.push([n, n + 2, n + 3]);
    }
    /// A cube as 12 triangles: every face is one group of two, nothing merges across an edge of the cube.
    #[test]
    fn coplanar_groups_pair_the_triangles_of_each_cube_face_only() {
        let (v, f) = {
            let (mut v, mut f) = (Vec::new(), Vec::new());
            for (a, b, c, d) in [
                ([0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]),
                ([0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]),
                ([0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]),
                ([0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]),
                ([0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]),
                ([1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]),
            ] {
                quad(&mut v, &mut f, a, b, c, d);
            }
            // weld the duplicated corners
            let mut map = Vec::new();
            let mut unique: Vec<V3> = Vec::new();
            for p in &v {
                let k = unique.iter().position(|q| q == p).unwrap_or_else(|| {
                    unique.push(*p);
                    unique.len() - 1
                });
                map.push(k);
            }
            (unique, f.iter().map(|t| t.map(|i| map[i])).collect::<Vec<_>>())
        };
        let groups = coplanar_groups(&v, &f, None);
        assert_eq!(groups.len(), 6);
        assert!(groups.iter().all(|g| g.len() == 2 && g[1] == g[0] + 1), "{groups:?}");
        // a frozen triangle stays alone
        let mut frozen = vec![false; f.len()];
        frozen[0] = true;
        let groups = coplanar_groups(&v, &f, Some(&frozen));
        assert_eq!(groups.len(), 7);
        assert!(groups.contains(&vec![0]) && groups.contains(&vec![1]));
    }
    /// A tilted plane whose vertices carry rounding noise merges; a bend of 1e-6 does not; a fin (opposite normal) does not;
    /// a chain of facets each bent by less than the pair tolerance but flat to nothing as a whole is split back.
    #[test]
    fn coplanar_groups_tolerate_rounding_but_not_bends_fins_or_accumulation() {
        let tilt = |x: f64, y: f64, noise: f64| -> V3 { [x, y, 0.3 * x + 0.7 * y + noise] };
        let (mut v, mut f) = (Vec::new(), Vec::new());
        quad(&mut v, &mut f, tilt(0., 0., 0.), tilt(2., 0., 3e-16), tilt(2., 1., -2e-16), tilt(0., 1., 1e-16));
        assert_eq!(coplanar_groups(&v, &f, None), vec![vec![0, 1]], "rounding-level noise merges");
        let (mut v, mut f) = (Vec::new(), Vec::new());
        quad(&mut v, &mut f, tilt(0., 0., 0.), tilt(2., 0., 0.), tilt(2., 1., 0.), tilt(0., 1., 2e-6));
        assert_eq!(coplanar_groups(&v, &f, None), vec![vec![0], vec![1]], "a 1e-6 bend does not merge");
        // a fin: the second triangle folds back over the first (normals opposite)
        let (v, f) = (vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]], vec![[0, 1, 2], [1, 0, 3]]);
        assert_eq!(coplanar_groups(&v, &f, None).len(), 2, "opposite normals never merge");
        // accumulation: 100 quads, each bent 4e-10 rad against the previous (pair test passes), 2e-8 of the size in total
        let (mut v, mut f) = (Vec::new(), Vec::new());
        let (mut x, mut z, mut phi) = (0., 0., 0.);
        for _ in 0..100 {
            let (nx, nz) = (x + f64::cos(phi), z + f64::sin(phi));
            quad(&mut v, &mut f, [x, 0., z], [nx, 0., nz], [nx, 1., nz], [x, 1., z]);
            (x, z, phi) = (nx, nz, phi + 4e-10);
        }
        // weld consecutive quads along their shared edge
        let mut weld = vec![0usize; v.len()];
        let (mut next, mut ids) = (0, Vec::new());
        for i in 0..v.len() {
            let same = ids.iter().position(|&(j, _): &(usize, usize)| v[j] == v[i]);
            weld[i] = match same {
                Some(p) => ids[p].1,
                None => {
                    ids.push((i, next));
                    next += 1;
                    next - 1
                }
            };
        }
        let mut pts = vec![[0.; 3]; next];
        for i in 0..v.len() {
            pts[weld[i]] = v[i];
        }
        let tris: Vec<[usize; 3]> = f.iter().map(|t| t.map(|i| weld[i])).collect();
        let groups = coplanar_groups(&pts, &tris, None);
        let largest = groups.iter().map(Vec::len).max().unwrap();
        assert!(largest <= 2, "a chain that is not flat as a whole must not merge: largest group {largest}");
    }
    /// A non-convex (L-shaped) merged patch keeps star-shaped insertion cavities around its reflex corner, and the
    /// triangulation keeps its area and orientation after every insertion.
    #[test]
    fn an_l_shaped_patch_accepts_points_around_its_reflex_corner() {
        let v: Vec<V3> = vec![[0., 0., 0.], [2., 0., 0.], [2., 1., 0.], [1., 1., 0.], [1., 2., 0.], [0., 2., 0.]];
        // a deliberately non-Delaunay fan from the corner opposite the reflex vertex
        let tris = vec![[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]];
        let mut mesh = FacetMesh::patch(0, &tris, &v).unwrap();
        let area = |m: &FacetMesh| -> f64 {
            m.tris
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|x| m.uv[&x]);
                    assert_eq!(orientation_2d(a, b, c), Ordering::Greater, "inverted sub-facet");
                    ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) / 2.
                })
                .sum()
        };
        assert!((area(&mesh) - 3.).abs() < 1e-12);
        let mut state = 5u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut points = v.clone();
        let mut inserted = 0;
        while inserted < 150 {
            // concentrate on the reflex corner (1, 1)
            let r = next().powi(3) * 1.2;
            let a = next() * std::f64::consts::TAU;
            let p: V3 = [1. + r * a.cos(), 1. + r * a.sin(), 0.];
            if !mesh.contains(p, &points) {
                continue;
            }
            let id = points.len();
            points.push(p);
            mesh.insert(id, p, None).unwrap_or_else(|e| panic!("insertion {inserted} at {p:?}: {e:?}"));
            inserted += 1;
            assert!((area(&mesh) - 3.).abs() < 1e-9, "area after insertion {inserted}");
        }
    }
    /// A thin U-shaped patch given as a poor (long, non-Delaunay) triangulation: insertions near its two reflex corners
    /// would build cavities that wrap around the notch unless the patch is made Delaunay and cavities stay visible.
    #[test]
    fn a_u_shaped_patch_with_a_poor_triangulation_takes_many_insertions() {
        let v: Vec<V3> = [(0., 0.), (10., 0.), (10., 10.), (9., 10.), (9., 1.), (1., 1.), (1., 10.), (0., 10.)]
            .iter()
            .map(|&(x, y)| [x, y, 0.])
            .collect();
        let tris = vec![[0, 1, 4], [0, 4, 5], [0, 5, 6], [0, 6, 7], [1, 2, 4], [2, 3, 4]];
        let mut mesh = FacetMesh::patch(0, &tris, &v).unwrap();
        let area = |m: &FacetMesh| -> f64 {
            m.tris
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|x| m.uv[&x]);
                    assert_eq!(orientation_2d(a, b, c), Ordering::Greater, "inverted sub-facet");
                    ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) / 2.
                })
                .sum()
        };
        let total = 10. + 9. + 9.;
        assert!((area(&mesh) - total).abs() < 1e-12);
        let mut state = 11u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut points = v.clone();
        let mut inserted = 0;
        while inserted < 400 {
            let corner = [[9., 1.], [1., 1.], [5., 0.5]][inserted % 3];
            let r = next().powi(2) * 3.;
            let a = next() * std::f64::consts::TAU;
            let p: V3 = [corner[0] + r * a.cos(), corner[1] + r * a.sin(), 0.];
            if !mesh.contains(p, &points) {
                continue;
            }
            let id = points.len();
            points.push(p);
            mesh.insert(id, p, None).unwrap_or_else(|e| panic!("insertion {inserted} at {p:?}: {e:?}"));
            inserted += 1;
        }
        assert!((area(&mesh) - total).abs() < 1e-9);
    }
    /// Random valid flips of interior edges turn a patch into a poor, non-Delaunay triangulation of the same region.
    fn scramble(mesh: &mut FacetMesh, next: &mut impl FnMut() -> f64, flips: usize) {
        for _ in 0..flips {
            let mut across = BTreeMap::<[usize; 2], usize>::new();
            for (i, t) in mesh.tris.iter().enumerate() {
                for k in 0..3 {
                    across.insert([t[k], t[(k + 1) % 3]], i);
                }
            }
            let i = (next() * mesh.tris.len() as f64) as usize % mesh.tris.len();
            let t = mesh.tris[i];
            let k = (next() * 3.) as usize % 3;
            let (x, y, p) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let Some(&j) = across.get(&[y, x]) else { continue };
            let q = *mesh.tris[j].iter().find(|v| **v != x && **v != y).unwrap();
            let uv = |v: usize| mesh.uv[&v];
            let (a, b) = ([x, q, p], [q, y, p]);
            if orientation_2d(uv(a[0]), uv(a[1]), uv(a[2])) == Ordering::Greater
                && orientation_2d(uv(b[0]), uv(b[1]), uv(b[2])) == Ordering::Greater
            {
                mesh.tris[i] = a;
                mesh.tris[j] = b;
            }
        }
        mesh.spheres.clear();
    }
    fn locally_delaunay(mesh: &FacetMesh) -> bool {
        let mut across = BTreeMap::<[usize; 2], usize>::new();
        for (i, t) in mesh.tris.iter().enumerate() {
            for k in 0..3 {
                across.insert([t[k], t[(k + 1) % 3]], i);
            }
        }
        mesh.tris.iter().all(|t| {
            (0..3).all(|k| {
                let (x, y, p) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                let Some(&j) = across.get(&[y, x]) else { return true };
                let q = *mesh.tris[j].iter().find(|v| **v != x && **v != y).unwrap();
                let uv = |v: usize| mesh.uv[&v];
                in_circle_2d(uv(x), uv(y), uv(p), uv(q)) != Ordering::Greater
            })
        })
    }
    #[test]
    fn a_merged_patch_starts_locally_delaunay_and_insertions_stay_valid_on_poor_triangulations() {
        let v: Vec<V3> = [(0., 0.), (10., 0.), (10., 10.), (9., 10.), (9., 1.), (1., 1.), (1., 10.), (0., 10.)]
            .iter()
            .map(|&(x, y)| [x, y, 0.])
            .collect();
        let tris = vec![[0, 1, 4], [0, 4, 5], [0, 5, 6], [0, 6, 7], [1, 2, 4], [2, 3, 4]];
        let patch = FacetMesh::patch(0, &tris, &v).unwrap();
        assert!(locally_delaunay(&patch), "Lawson flips leave every interior edge locally Delaunay");
        let mut raw = FacetMesh::patch(0, &tris[..1], &v).unwrap();
        raw.tris = tris.clone();
        raw.uv = patch.uv.clone();
        assert!(!locally_delaunay(&raw), "the given triangulation is not Delaunay (the flips did something)");
        let mut state = 3u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        for round in 0..300 {
            let mut mesh = FacetMesh::patch(0, &tris, &v).unwrap();
            scramble(&mut mesh, &mut next, 40);
            let mut points = v.clone();
            for k in 0..25 {
                let corner = [[9., 1.], [1., 1.], [5., 0.5]][k % 3];
                let (r, a) = (next().powi(2) * 4., next() * std::f64::consts::TAU);
                let p: V3 = [corner[0] + r * a.cos(), corner[1] + r * a.sin(), 0.];
                if !mesh.contains(p, &points) {
                    continue;
                }
                points.push(p);
                mesh.insert(points.len() - 1, p, None).unwrap_or_else(|e| panic!("round {round}: {p:?}: {e:?}"));
                for t in &mesh.tris {
                    let [a, b, c] = t.map(|x| mesh.uv[&x]);
                    assert_eq!(orientation_2d(a, b, c), Ordering::Greater, "round {round}: inverted sub-facet");
                }
            }
        }
    }
}
